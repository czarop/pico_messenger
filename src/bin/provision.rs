#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::uart::{Config, Uart};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// const CERT_DER: &[u8] = include_bytes!("../../emqx.der");
// const CERT_DER: &[u8] = include_bytes!("../../test_ecdsa.der");
const CERT_DER: &[u8] = include_bytes!("../../ca.der");

fn fmt_decimal(n: usize, buf: &mut [u8; 10]) -> &[u8] {
    if n == 0 {
        buf[9] = b'0';
        return &buf[9..];
    }
    let mut n = n;
    let mut i = 10usize;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    &buf[i..]
}

fn byte_to_hex(b: u8) -> [u8; 2] {
    const HEX: &[u8] = b"0123456789ABCDEF";
    [HEX[(b >> 4) as usize], HEX[(b & 0xf) as usize]]
}

fn read_line(uart: &mut Uart<'_, embassy_rp::uart::Blocking>, buf: &mut [u8]) -> usize {
    let mut len = 0;
    loop {
        let mut b = [0u8; 1];
        uart.blocking_read(&mut b).unwrap();
        if len < buf.len() {
            buf[len] = b[0];
            len += 1;
        }
        if len >= 4 && &buf[len - 4..len] == b"OK\r\n" {
            break;
        }
        if len >= 7 && &buf[len - 7..len] == b"ERROR\r\n" {
            break;
        }
        if len >= buf.len() {
            break;
        }
    }
    len
}

/// Log a response buffer as text when it's valid UTF-8, else as raw bytes.
fn log_response(label: &str, buf: &[u8]) {
    match core::str::from_utf8(buf) {
        Ok(s) => defmt::info!("{}: {}", label, s),
        Err(_) => defmt::info!("{} (raw): {:?}", label, buf),
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    defmt::info!(
        "Provisioning: uploading TLS CA cert ({} bytes)",
        CERT_DER.len()
    );
    defmt::info!("Waiting 12s for modem to boot...");
    Timer::after(Duration::from_secs(12)).await;

    let mut uart = Uart::new_blocking(p.UART1, p.PIN_4, p.PIN_5, Config::default());
    // Larger buffer so the #TLSCERTLIST response (cert metadata) fits comfortably.
    let mut buf = [0u8; 128];

    // Tracks whether the cert was accepted by either upload mode.
    let mut stored = false;

    // Step 1: delete any existing cert at profile 0, type 1
    defmt::info!("Deleting any existing cert...");
    uart.blocking_write(b"AT#TLSCERTDEL=0,1\r").unwrap();
    let n = read_line(&mut uart, &mut buf);
    log_response("delete response", &buf[..n]);

    // Step 2: try hex mode (full inline hex string)
    defmt::info!(
        "Trying hex mode (AT#TLSCERTADD=0,1,{},<hex>)...",
        CERT_DER.len()
    );
    uart.blocking_write(b"AT#TLSCERTADD=0,1,").unwrap();
    let mut len_buf = [0u8; 10];
    uart.blocking_write(fmt_decimal(CERT_DER.len(), &mut len_buf))
        .unwrap();
    uart.blocking_write(b",").unwrap();
    for &byte in CERT_DER {
        uart.blocking_write(&byte_to_hex(byte)).unwrap();
    }
    uart.blocking_write(b"\r").unwrap();

    let n = read_line(&mut uart, &mut buf);
    log_response("hex mode response", &buf[..n]);

    if &buf[..n] == b"\r\nOK\r\n" {
        defmt::info!("Hex mode SUCCESS.");
        stored = true;
    } else {
        defmt::warn!("Hex mode failed. Trying binary mode...");

        // Step 3: delete again before binary mode attempt
        uart.blocking_write(b"AT#TLSCERTDEL=0,1\r").unwrap();
        let n = read_line(&mut uart, &mut buf);
        log_response("delete response", &buf[..n]);

        // Binary mode: send command without data, get OK, send raw bytes
        uart.blocking_write(b"AT#TLSCERTADD=0,1,").unwrap();
        uart.blocking_write(fmt_decimal(CERT_DER.len(), &mut len_buf))
            .unwrap();
        uart.blocking_write(b"\r").unwrap();

        let n = read_line(&mut uart, &mut buf);
        log_response("binary mode command response", &buf[..n]);

        if &buf[..n] == b"\r\nOK\r\n" {
            defmt::info!("Sending {} raw DER bytes...", CERT_DER.len());
            uart.blocking_write(CERT_DER).unwrap();

            let n = read_line(&mut uart, &mut buf);
            log_response("binary mode data response", &buf[..n]);

            if &buf[..n] == b"\r\nOK\r\n" {
                defmt::info!("Binary mode SUCCESS.");
                stored = true;
            } else {
                defmt::error!("Binary mode also failed. Both approaches exhausted.");
            }
        } else {
            defmt::error!("Binary mode command rejected.");
        }
    }

    // Step 4: integrity check BEFORE persisting.
    // For type 0/1 the modem returns: #TLSCERTLIST: <sec_id>,<type>,<length>[,<YYMMDD>]
    // A correct upload reads back as length 381 with a valid expiry date. A truncated
    // binary upload (DER containing 0x0D) would show a wrong length or fail to parse.
    if stored {
        defmt::info!("Verifying stored cert (AT#TLSCERTLIST=0)...");
        uart.blocking_write(b"AT#TLSCERTLIST=0\r").unwrap();
        let n = read_line(&mut uart, &mut buf);
        log_response("TLSCERTLIST response", &buf[..n]);
        defmt::info!(
            "EXPECT: length {} and a valid YYMMDD expiry date in the line above.",
            CERT_DER.len()
        );

        // Step 5: disable PSM sleep so the modem stays awake for the AT/MQTT path.
        // #SLEEPMODE is saved to NVM and only takes effect after the reboot below.
        // (Re-enable later for battery operation: AT#SLEEPMODE=1,<hold>,<awake>.)
        defmt::info!("Disabling PSM sleep (AT#SLEEPMODE=0)...");
        uart.blocking_write(b"AT#SLEEPMODE=0\r").unwrap();
        let n = read_line(&mut uart, &mut buf);
        log_response("SLEEPMODE response", &buf[..n]);

        // Step 6: enable +CGEV packet-domain event reporting (mode 1 = forward URCs).
        // Takes effect immediately and persists across the reset below.
        defmt::info!("Enabling packet-domain event reporting (AT+CGEREP=1)...");
        uart.blocking_write(b"AT+CGEREP=1\r").unwrap();
        let n = read_line(&mut uart, &mut buf);
        log_response("CGEREP response", &buf[..n]);

        // Enable network time: with CTZR=3 the network delivers UTC on registration
        // (+CTZEU URC). TLS cert validation needs a valid clock; without this the
        // modem has no time and the handshake fails with "bad certificate".
        defmt::info!("Enabling network time reporting (AT+CTZR=3)...");
        uart.blocking_write(b"AT+CTZR=3\r").unwrap();
        let n = read_line(&mut uart, &mut buf);
        log_response("CTZR response", &buf[..n]);

        defmt::info!("Saving to NVM (AT#RESET=1)...");
        uart.blocking_write(b"AT#RESET=1\r").unwrap();
        defmt::info!("Done. Wait 15s then flash main firmware.");
    } else {
        defmt::error!("Cert NOT stored - skipping verify and reset. Fix upload first.");
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}