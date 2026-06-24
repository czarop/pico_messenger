#![no_std]
#![no_main]

// Diagnostic binary: splits "egress blocked" from "TLS handshake failing".
//
// 1. Reads the modem clock (AT+CCLK?). A wrong/absent clock breaks TLS cert
//    validation, so we check it first.
// 2. Waits for PDP context 5 to come up (NB-IoT registration).
// 3. Creates a PLAIN (non-TLS) TCP socket and does AT#TCPCONNECT to the broker.
//
// Interpretation of the TCPCONNECT result:
//   OK / status 5  -> raw TCP reaches the broker. Egress works; the MQTT failure
//                     is in the TLS handshake (NB-IoT MTU, cert, or clock).
//   ERROR / timeout-> the modem cannot open a TCP connection to the broker at all.
//                     That is an egress/routing problem (Soracom / NB-IoT).
//
// Uses async BufferedUart with per-read timeouts so a silent modem can't hang us.
// Run with: cargo run --bin tcptest

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::UART1;
use embassy_rp::uart::{
    BufferedInterruptHandler, BufferedUart, BufferedUartRx, BufferedUartTx, Config,
};
use embassy_time::{Duration, Timer, with_timeout};
use embedded_io_async::{Read, Write};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// Broker target — must match your .env (MQTT_ADDRESS / MQTT_PORT).
const BROKER_IP: &[u8] = b"34.148.211.3";
const BROKER_PORT: &[u8] = b"8883";

bind_interrupts!(struct Irqs {
    UART1_IRQ => BufferedInterruptHandler<UART1>;
});

static TX_BUF: StaticCell<[u8; 64]> = StaticCell::new();
static RX_BUF: StaticCell<[u8; 256]> = StaticCell::new();

/// Read a modem response until OK/ERROR or `timeout`. Returns (len, timed_out).
async fn read_resp(rx: &mut BufferedUartRx, buf: &mut [u8], timeout: Duration) -> (usize, bool) {
    let mut len = 0usize;
    let result = with_timeout(timeout, async {
        loop {
            let mut b = [0u8; 1];
            match rx.read(&mut b).await {
                Ok(0) => continue,
                Ok(_) => {
                    if len < buf.len() {
                        buf[len] = b[0];
                        len += 1;
                    }
                    if len >= 4 && &buf[len - 4..len] == b"OK\r\n" {
                        return;
                    }
                    if len >= 7 && &buf[len - 7..len] == b"ERROR\r\n" {
                        return;
                    }
                    if len >= buf.len() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    })
    .await;
    (len, result.is_err())
}

fn log_response(label: &str, buf: &[u8], timed_out: bool) {
    if timed_out {
        defmt::warn!("{}: <NO RESPONSE / timeout>", label);
        return;
    }
    match core::str::from_utf8(buf) {
        Ok(s) => defmt::info!("{}: {}", label, s),
        Err(_) => defmt::info!("{} (raw): {:?}", label, buf),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    defmt::info!("TCP diagnostic starting. Waiting 12s for modem boot...");
    Timer::after(Duration::from_secs(12)).await;

    let uart = BufferedUart::new(
        p.UART1,
        p.PIN_4,
        p.PIN_5,
        Irqs,
        TX_BUF.init([0; 64]),
        RX_BUF.init([0; 256]),
        Config::default(),
    );
    let (mut tx, mut rx) = uart.split();
    let mut buf = [0u8; 256];

    // --- Step 1: set the modem clock ---------------------------------------
    // The network isn't providing time (CCLK was ERROR), so set it manually to
    // a date after the cert's notBefore (2026-06-23). Format: "YY/MM/DD,hh:mm:ss+zz"
    // (zz = quarter-hours from GMT). Takes effect immediately.
    defmt::info!("Setting modem clock (AT+CCLK=...) to 2026-07-01 UTC...");
    let _ = tx.write_all(b"AT+CCLK=\"26/07/01,12:00:00+00\"\r").await;
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(5)).await;
    log_response("CCLK set", &buf[..n], to);

    // Enable network time-zone/universal-time reporting (works only if the
    // network provides it; harmless otherwise).
    let _ = tx.write_all(b"AT+CTZR=3\r").await;
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(5)).await;
    log_response("CTZR=3", &buf[..n], to);

    // Confirm the clock took.
    let _ = tx.write_all(b"AT+CCLK?\r").await;
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(5)).await;
    log_response("CCLK now", &buf[..n], to);

    // --- Step 2: wait for PDP context 5 (registration) ----------------------
    defmt::info!("Waiting for PDP context 5 (AT+CGPADDR=5)...");
    let mut active = false;
    for _ in 0..40 {
        let _ = tx.write_all(b"AT+CGPADDR=5\r").await;
        let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(5)).await;
        if !to && contains(&buf[..n], b"CGPADDR") && contains(&buf[..n], b"\"") {
            log_response("PDP active", &buf[..n], false);
            active = true;
            break;
        }
        defmt::info!("not registered yet, polling...");
        Timer::after(Duration::from_secs(3)).await;
    }
    if !active {
        defmt::error!("PDP context never came up - check signal/registration.");
        loop {
            Timer::after(Duration::from_secs(60)).await;
        }
    }

    // --- Step 3: create a PLAIN (non-TLS) TCP socket ------------------------
    // No <security_profile_id> => no TLS. 30s timeouts give the connect room.
    // --- Step 2b: read the negotiated path MTU -----------------------------
    // NB-IoT often gives a small IPv4 MTU; if so, large TLS handshake records
    // get fragmented and dropped. The IPv4_MTU is near the end of the response.
    defmt::info!("Reading PDP MTU (AT+CGCONTRDP=5)...");
    let _ = tx.write_all(b"AT+CGCONTRDP=5\r").await;
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(5)).await;
    log_response("CGCONTRDP", &buf[..n], to);

    // --- Step 3a: close any socket left open by a previous run --------------
    // The prior run's TCPCONNECT left the single TCP socket open; the modem holds
    // it across a reflash, so SOCKETCREATE can stall/fail. Close it first.
    // Guarded by the timeout reader so a silent close can't hang us.
    defmt::info!("Closing any stale socket (AT#SOCKETCLOSE=5,0)...");
    let _ = tx.write_all(b"AT#SOCKETCLOSE=5,0\r").await;
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(8)).await;
    log_response("SOCKETCLOSE", &buf[..n], to);

    defmt::info!("Creating TLS socket (security profile 0)...");
    let _ = tx
        .write_all(b"AT#SOCKETCREATE=5,0,\"TCP\",0,90,90,0,0\r")
        .await;
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(10)).await;
    log_response("SOCKETCREATE", &buf[..n], to);
    if to || !contains(&buf[..n], b"SOCKETCREATE") {
        defmt::error!("Socket create failed/silent - cannot run TCP test. See above.");
        loop {
            Timer::after(Duration::from_secs(60)).await;
        }
    }
    // Single TCP socket => new socket is id 0.

    // --- Step 4: raw TCP connect to the broker ------------------------------
    // App note format: AT#TCPCONNECT=<ctx>,<socket>,<ip>,<port>  (ip unquoted).
    defmt::info!("Attempting TLS connect to broker (handshake happens here)...");
    let _ = tx.write_all(b"AT#TCPCONNECT=5,0,").await;
    let _ = tx.write_all(BROKER_IP).await;
    let _ = tx.write_all(b",").await;
    let _ = tx.write_all(BROKER_PORT).await;
    let _ = tx.write_all(b"\r").await;
    // NB-IoT is slow; a TLS handshake with retransmits can take a while. Allow 90s.
    defmt::info!("(waiting up to 90s - NB-IoT TLS handshakes are slow)");
    let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(90)).await;
    log_response("TCPCONNECT", &buf[..n], to);

    if !to && contains(&buf[..n], b"OK") && !contains(&buf[..n], b"ERROR") {
        defmt::info!(
            "RESULT: TLS CONNECTED. The handshake succeeded -> the clock was the problem. MQTT should now work."
        );
        let _ = tx.write_all(b"AT#TCPCONNECT?\r").await;
        let (n, to) = read_resp(&mut rx, &mut buf, Duration::from_secs(5)).await;
        log_response("TCPCONNECT? status (5 = connected)", &buf[..n], to);
    } else if to {
        defmt::warn!(
            "RESULT: TLS TIMED OUT (no response). Handshake stalled -> likely NB-IoT MTU/fragmentation, not the clock."
        );
    } else {
        defmt::warn!(
            "RESULT: TLS FAILED (error). Handshake rejected -> cert/clock still wrong, or cipher/cert issue. Check CCLK now line above."
        );
    }

    defmt::info!("Diagnostic complete.");
    let _ = tx; // keep tx alive
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
