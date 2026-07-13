// #![no_std]
// #![no_main]

// use embassy_executor::Spawner;
// use embassy_rp::uart::{Config, Uart};
// use embassy_time::{Duration, Timer};
// use {defmt_rtt as _, panic_probe as _};

// // const CERT_DER: &[u8] = include_bytes!("../../emqx.der");
// // const CERT_DER: &[u8] = include_bytes!("../../test_ecdsa.der");
// const CERT_DER: &[u8] = include_bytes!("../../ca.der");

// /// Provision the modem for Power Saving Mode.
// ///
// /// Set to `false` to leave the modem always-awake (the previous behaviour) while
// /// debugging the AT/MQTT path -- PSM makes an idle modem stop answering the UART,
// /// which is confusing if you are not expecting it.
// ///
// /// Note this only provisions PSM; it does not sleep the modem. At runtime that is
// /// `modem::psm::enter_psm()`. Nothing sleeps until `modem_task` asks.
// const PROVISION_PSM: bool = true;

// fn fmt_decimal(n: usize, buf: &mut [u8; 10]) -> &[u8] {
//     if n == 0 {
//         buf[9] = b'0';
//         return &buf[9..];
//     }
//     let mut n = n;
//     let mut i = 10usize;
//     while n > 0 {
//         i -= 1;
//         buf[i] = b'0' + (n % 10) as u8;
//         n /= 10;
//     }
//     &buf[i..]
// }

// fn byte_to_hex(b: u8) -> [u8; 2] {
//     const HEX: &[u8] = b"0123456789ABCDEF";
//     [HEX[(b >> 4) as usize], HEX[(b & 0xf) as usize]]
// }

// fn read_line(uart: &mut Uart<'_, embassy_rp::uart::Blocking>, buf: &mut [u8]) -> usize {
//     let mut len = 0;
//     loop {
//         let mut b = [0u8; 1];
//         uart.blocking_read(&mut b).unwrap();
//         if len < buf.len() {
//             buf[len] = b[0];
//             len += 1;
//         }
//         if len >= 4 && &buf[len - 4..len] == b"OK\r\n" {
//             break;
//         }
//         if len >= 7 && &buf[len - 7..len] == b"ERROR\r\n" {
//             break;
//         }
//         if len >= buf.len() {
//             break;
//         }
//     }
//     len
// }

// /// Log a response buffer as text when it's valid UTF-8, else as raw bytes.
// fn log_response(label: &str, buf: &[u8]) {
//     match core::str::from_utf8(buf) {
//         Ok(s) => defmt::info!("{}: {}", label, s),
//         Err(_) => defmt::info!("{} (raw): {:?}", label, buf),
//     }
// }

// #[embassy_executor::main]
// async fn main(_spawner: Spawner) {
//     let p = embassy_rp::init(Default::default());

//     defmt::info!(
//         "Provisioning: uploading TLS CA cert ({} bytes)",
//         CERT_DER.len()
//     );
//     defmt::info!("Waiting 12s for modem to boot...");
//     Timer::after(Duration::from_secs(12)).await;

//     let mut uart = Uart::new_blocking(p.UART1, p.PIN_4, p.PIN_5, Config::default());
//     // Larger buffer so the #TLSCERTLIST response (cert metadata) fits comfortably.
//     let mut buf = [0u8; 128];

//     // Tracks whether the cert was accepted by either upload mode.
//     let mut stored = false;

//     // Step 1: delete any existing cert at profile 0, type 1
//     defmt::info!("Deleting any existing cert...");
//     uart.blocking_write(b"AT#TLSCERTDEL=0,1\r").unwrap();
//     let n = read_line(&mut uart, &mut buf);
//     log_response("delete response", &buf[..n]);

//     // Step 2: try hex mode (full inline hex string)
//     defmt::info!(
//         "Trying hex mode (AT#TLSCERTADD=0,1,{},<hex>)...",
//         CERT_DER.len()
//     );
//     uart.blocking_write(b"AT#TLSCERTADD=0,1,").unwrap();
//     let mut len_buf = [0u8; 10];
//     uart.blocking_write(fmt_decimal(CERT_DER.len(), &mut len_buf))
//         .unwrap();
//     uart.blocking_write(b",").unwrap();
//     for &byte in CERT_DER {
//         uart.blocking_write(&byte_to_hex(byte)).unwrap();
//     }
//     uart.blocking_write(b"\r").unwrap();

//     let n = read_line(&mut uart, &mut buf);
//     log_response("hex mode response", &buf[..n]);

//     if &buf[..n] == b"\r\nOK\r\n" {
//         defmt::info!("Hex mode SUCCESS.");
//         stored = true;
//     } else {
//         defmt::warn!("Hex mode failed. Trying binary mode...");

//         // Step 3: delete again before binary mode attempt
//         uart.blocking_write(b"AT#TLSCERTDEL=0,1\r").unwrap();
//         let n = read_line(&mut uart, &mut buf);
//         log_response("delete response", &buf[..n]);

//         // Binary mode: send command without data, get OK, send raw bytes
//         uart.blocking_write(b"AT#TLSCERTADD=0,1,").unwrap();
//         uart.blocking_write(fmt_decimal(CERT_DER.len(), &mut len_buf))
//             .unwrap();
//         uart.blocking_write(b"\r").unwrap();

//         let n = read_line(&mut uart, &mut buf);
//         log_response("binary mode command response", &buf[..n]);

//         if &buf[..n] == b"\r\nOK\r\n" {
//             defmt::info!("Sending {} raw DER bytes...", CERT_DER.len());
//             uart.blocking_write(CERT_DER).unwrap();

//             let n = read_line(&mut uart, &mut buf);
//             log_response("binary mode data response", &buf[..n]);

//             if &buf[..n] == b"\r\nOK\r\n" {
//                 defmt::info!("Binary mode SUCCESS.");
//                 stored = true;
//             } else {
//                 defmt::error!("Binary mode also failed. Both approaches exhausted.");
//             }
//         } else {
//             defmt::error!("Binary mode command rejected.");
//         }
//     }

//     // Step 4: integrity check BEFORE persisting.
//     // For type 0/1 the modem returns: #TLSCERTLIST: <sec_id>,<type>,<length>[,<YYMMDD>]
//     // A correct upload reads back as length 381 with a valid expiry date. A truncated
//     // binary upload (DER containing 0x0D) would show a wrong length or fail to parse.
//     if stored {
//         defmt::info!("Verifying stored cert (AT#TLSCERTLIST=0)...");
//         uart.blocking_write(b"AT#TLSCERTLIST=0\r").unwrap();
//         let n = read_line(&mut uart, &mut buf);
//         log_response("TLSCERTLIST response", &buf[..n]);
//         defmt::info!(
//             "EXPECT: length {} and a valid YYMMDD expiry date in the line above.",
//             CERT_DER.len()
//         );

//         // Step 5: Power Saving Mode.
//         //
//         // Set `PROVISION_PSM = false` to fall back to the old always-awake
//         // behaviour (AT#SLEEPMODE=0) while debugging the AT/MQTT path.
//         //
//         // Every one of these is NVM-saved and takes effect only after the
//         // AT#RESET=1 below -- you cannot enable sleep mode and use it in the same
//         // session. At runtime the modem is slept with the bare execution form
//         // `AT#SLEEPMODE` (no parameters) and woken by pulsing GPIO10 low.
//         if PROVISION_PSM {
//             // Sleep/wake URCs. 0x44 = b2 (PSM event) | b6 (energy report).
//             //
//             // NOT the manual's 0x7F: bit4 (verbosity) turns the URC into
//             // `#SLEEP PSM 3599.9s`, and atat's digester only recognises
//             // `\r\n{token}(:.*)?\r\n` -- a space after the token matches neither
//             // form, so the line would be folded into the next command's response
//             // buffer. Bit5 emits a bare `NBIOT SW version ...` line with no `#`
//             // prefix, same problem. See modem::psm::SLEEPIND_PSM_AND_ENERGY.
//             defmt::info!("Enabling sleep URCs (AT#SLEEPIND=0x44)...");
//             uart.blocking_write(b"AT#SLEEPIND=0x44\r").unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("SLEEPIND response", &buf[..n]);

//             // Host -> modem wake. pwrkey_evt = 0b1111: enable, active LOW, pull
//             // enabled, pull-up. Board maps RP2350 GPIO10 -> ST87M01 WAKE_UP (39).
//             // uart_evt = 0b0011: enable, active low -- a free fallback wake path.
//             defmt::info!("Arming wake pin (AT#WAKEUPEVENT=15,3)...");
//             uart.blocking_write(b"AT#WAKEUPEVENT=15,3\r").unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("WAKEUPEVENT response", &buf[..n]);

//             // <hold_time> = 300 s: seconds after the last AT command before the
//             // modem *auto*-sleeps. Deliberately long -- GNSS fixes arrive as
//             // #GNSSFIX URCs with no AT traffic to keep the hold timer alive, so a
//             // short value would drop the modem into PSM mid-fix. We never rely on
//             // auto-sleep; entry is always the explicit bare AT#SLEEPMODE.
//             // <awake_time> = 0: stuck-watchdog disabled (only arms above 600 s).
//             defmt::info!("Enabling sleep mode (AT#SLEEPMODE=1,300,0)...");
//             uart.blocking_write(b"AT#SLEEPMODE=1,300,0\r").unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("SLEEPMODE response", &buf[..n]);

//             // Network PSM contract. T3412 = "00100100" = 4 hours (GPRS Timer 3),
//             // T3324 = "00000001" = 2 seconds (GPRS Timer 2).
//             //
//             // T3412 is NOT the uplink cadence -- the RP2350 POWMAN timer owns
//             // that. It only bounds how long the modem may stay radio-silent
//             // before waking *itself* for a Tracking Area Update. Every uplink we
//             // send resets it, so long is strictly better for an uplink-only
//             // device: a short value would burn radio on pointless TAUs through a
//             // multi-hour DEEP_REST park.
//             //
//             // Requested, not granted. Read back <Periodic-TAU> from the CEREG
//             // URC enabled below. Positions 2 and 3 (RAU, GPRS-READY) are
//             // unsupported on NB-IoT and ignored, hence empty.
//             //
//             // Beware: "00100100" is 4 HOURS in position 4 and 4 MINUTES in
//             // position 5. Different unit tables. Do not swap them.
//             // defmt::info!("Requesting PSM (AT+CPSMS=1,,,\"00100100\",\"00000001\")...");
//             // uart.blocking_write(b"AT+CPSMS=1,,,\"00100100\",\"00000001\"\r")
//             //     .unwrap();
//             defmt::info!("Requesting PSM (AT+CPSMS=1,,,\"00000001\",\"00000001\") [TAU 10min, active 2s]...");
//             uart.blocking_write(b"AT+CPSMS=1,,,\"00000001\",\"00000001\"\r")
//                 .unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("CPSMS response", &buf[..n]);
            
            

//             // n=4 appends <Active-Time>,<Periodic-TAU> to the +CEREG URC -- the
//             // only way to learn what the network actually granted (AT manual 6.9).
//             // <stat> stays first, so network_task's cereg_registered() is
//             // unaffected; ModemUrc::Cereg already reserves String<96> for the
//             // longer body.
//             defmt::info!("Enabling PSM-form registration URC (AT+CEREG=4)...");
//             uart.blocking_write(b"AT+CEREG=4\r").unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("CEREG response", &buf[..n]);

//             // defmt::info!("Arming RING pin (AT#RINGPIN=1,9,0,100)...");
//             // uart.blocking_write(b"AT#RINGPIN=1,9,0,100\r").unwrap();
//             // let n = read_line(&mut uart, &mut buf);
//             // log_response("RINGPIN response", &buf[..n]);

//             // test ONLY
//             defmt::info!("Requesting PSM (TAU 1min TEST, active 2s)...");
//             uart.blocking_write(b"AT+CPSMS=1,,,\"10100001\",\"00000001\"\r")
//                 .unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("CPSMS response", &buf[..n]);
//         } else {
//             // Always-awake: keeps the modem responsive for AT/MQTT debugging.
//             defmt::info!("PSM disabled by PROVISION_PSM (AT#SLEEPMODE=0)...");
//             uart.blocking_write(b"AT#SLEEPMODE=0\r").unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("SLEEPMODE response", &buf[..n]);

//             uart.blocking_write(b"AT+CPSMS=0\r").unwrap();
//             let n = read_line(&mut uart, &mut buf);
//             log_response("CPSMS response", &buf[..n]);
//         }

//         // Step 6: enable +CGEV packet-domain event reporting (mode 1 = forward URCs).
//         // Takes effect immediately and persists across the reset below.
//         defmt::info!("Enabling packet-domain event reporting (AT+CGEREP=1)...");
//         uart.blocking_write(b"AT+CGEREP=1\r").unwrap();
//         let n = read_line(&mut uart, &mut buf);
//         log_response("CGEREP response", &buf[..n]);

//         // Enable network time: with CTZR=3 the network delivers UTC on registration
//         // (+CTZEU URC). TLS cert validation needs a valid clock; without this the
//         // modem has no time and the handshake fails with "bad certificate".
//         defmt::info!("Enabling network time reporting (AT+CTZR=3)...");
//         uart.blocking_write(b"AT+CTZR=3\r").unwrap();
//         let n = read_line(&mut uart, &mut buf);
//         log_response("CTZR response", &buf[..n]);

//         // AT#SLEEPMODE / AT#WAKEUPEVENT / AT#SLEEPIND are inert until this reboot.
//         defmt::info!("Saving to NVM (AT#RESET=1)...");
//         uart.blocking_write(b"AT#RESET=1\r").unwrap();
//         defmt::info!("Done. Wait 15s then flash main firmware.");
//         if PROVISION_PSM {
//             defmt::info!(
//                 "PSM provisioned. Modem will auto-sleep 300s after the last AT command."
//             );
//         }
//     } else {
//         defmt::error!("Cert NOT stored - skipping verify and reset. Fix upload first.");
//     }

//     loop {
//         Timer::after(Duration::from_secs(60)).await;
//     }
// }
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::uart::{Config, Uart};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// const CERT_DER: &[u8] = include_bytes!("../../emqx.der");
// const CERT_DER: &[u8] = include_bytes!("../../test_ecdsa.der");
const CERT_DER: &[u8] = include_bytes!("../../ca.der");

/// Provision the modem for Power Saving Mode.
///
/// Set to `false` to leave the modem always-awake (the previous behaviour) while
/// debugging the AT/MQTT path -- PSM makes an idle modem stop answering the UART,
/// which is confusing if you are not expecting it.
///
/// Note this only provisions PSM; it does not sleep the modem. At runtime that is
/// `modem::psm::enter_psm()`. Nothing sleeps until `modem_task` asks.
const PROVISION_PSM: bool = true;

/// Requested extended periodic TAU (T3412), as an 8-bit string.
///
/// # This is the master wake clock
///
/// T3412 bounds how long the modem may stay radio-silent before it must wake
/// *itself* for a Tracking Area Update. That self-wake emits a URC, which pulses
/// the RING line (see `AT#RINGPIN` below), which is what wakes the RP2350 out of
/// DORMANT. So T3412 sets the tracking cadence.
///
/// It did not start out that way. The plan was for the RP2350's own POWMAN/AON
/// alarm to wake the host, with T3412 as a mere keepalive -- but that path is
/// dead. `aon_timer`'s `AlarmWakeMode::DormantOnly` sets POWMAN's
/// `TIMER.PWRUP_ON_ALARM`, which requests a POWMAN *power-state* transition; but
/// `clocks::dormant_sleep()` does not enter a POWMAN power state, it writes "coma"
/// to `ROSC.dormant()`. The only wake path embassy wires into ROSC dormancy is
/// GPIO dormant-wake. Verified on hardware: the AON timer counts and its alarm
/// fires correctly (`t+5000ms: now=5315 fired=true`), but it cannot rouse a
/// comatose ROSC -- the board slept and never woke. Hence: modem drives, RING
/// wakes.
///
/// # Encoding (3GPP TS 24.008 10.5.7.4a, GPRS Timer 3)
///
/// bits 7..5 = unit, bits 4..0 = multiplier.
///   000 = 10 min   001 = 1 hour   010 = 10 hours   011 = 2 s
///   100 = 30 s     101 = 1 min    110 = 320 hours  111 = deactivated
///
/// Sanity-checked against the AT manual's worked example: "01000111" =
/// 010 (10 h) x 00111 (7) = 70 hours. Correct.
///
/// Note the 1-minute unit (101) -- so a 15-minute TAU is exactly expressible, and
/// we are not forced onto the coarse 10-minute steps.
///
/// Requested, not granted. The network decides; read the granted value back from
/// `AT+CEREG?` or the `+CEREG` URC (`n=4` appends `<Active-Time>,<Periodic-TAU>`).
/// A 1-minute TAU is aggressive and carriers often floor it much higher.
const T3412_TEST: &[u8] = b"10100001"; // 101 (1 min)  x 00001 (1)  =  1 minute
#[allow(dead_code)]
const T3412_PRODUCTION: &[u8] = b"10101111"; // 101 (1 min)  x 01111 (15) = 15 minutes

/// Requested active time (T3324): how long the modem stays reachable after the
/// RRC connection releases, before dropping into PSM. Short = prompt PSM entry.
///
/// # Encoding (3GPP TS 24.008 10.5.7.3, GPRS Timer 2 -- a DIFFERENT table)
///
///   000 = 2 s   001 = 1 min   010 = 6 min (decihour)   111 = deactivated
///
/// Manual's worked example: "00100100" = 001 (1 min) x 00100 (4) = 4 min. Correct.
///
/// CAUTION: T3412 and T3324 index different unit tables, so the SAME byte means
/// different things depending on argument position. "00000001" is 10 MINUTES as
/// T3412 and 2 SECONDS as T3324. Swapping the two arguments is silent and would
/// be catastrophic for power. Order matters.
const T3324: &[u8] = b"00000001"; // 000 (2 s) x 00001 (1) = 2 seconds

/// Which T3412 to provision.
///
/// `true`  -> [`T3412_TEST`] (1 min): the modem self-wakes quickly, so the
///            RING-wake premise can be verified in a minute rather than fifteen.
/// `false` -> [`T3412_PRODUCTION`] (15 min).
const USE_TEST_TAU: bool = true;

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

    // NOTE: this waits for a modem that may not actually be rebooting. Reflashing
    // the RP2350 does NOT reboot the modem, and the modem retains state across a
    // reflash -- a firmware run aborted mid-fix leaves the GNSS engine running,
    // spraying #GNSSFIX URCs. `read_line` below is a blocking read with no
    // digester, so those URCs land *inside* the command responses and corrupt
    // every one of them (seen: "delete response: +CME ERROR: 1121 #GNSSFIX: 5
    // #GNSSFIX: 5 ...", where 1121 = "AT command forward to Modem error").
    //
    // ALWAYS POWER-CYCLE THE BOARD BEFORE PROVISIONING. Unplug USB and battery,
    // wait ~10 s, plug back in. If any response below contains #GNSSFIX, the modem
    // did not reboot -- stop and power-cycle properly.
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

        // Step 5: Power Saving Mode.
        //
        // Set `PROVISION_PSM = false` to fall back to the old always-awake
        // behaviour (AT#SLEEPMODE=0) while debugging the AT/MQTT path.
        //
        // Every one of these is NVM-saved and takes effect only after the
        // AT#RESET=1 below -- you cannot enable sleep mode and use it in the same
        // session. At runtime the modem is slept with the bare execution form
        // `AT#SLEEPMODE` (no parameters) and woken either by pulsing GPIO10 low
        // (host-initiated) or by its own T3412 expiry (self-initiated).
        if PROVISION_PSM {
            // Sleep/wake URCs. 0x44 = b2 (PSM event) | b6 (energy report).
            //
            // NOT the manual's 0x7F: bit4 (verbosity) turns the URC into
            // `#SLEEP PSM 3599.9s`, and atat's digester only recognises
            // `\r\n{token}(:.*)?\r\n` -- a space after the token matches neither
            // form, so the line would be folded into the next command's response
            // buffer. Bit5 emits a bare `NBIOT SW version ...` line with no `#`
            // prefix, same problem. See modem::psm::SLEEPIND_PSM_AND_ENERGY.
            //
            // (Even at 0x44 the modem still appends a trailing space -- `#SLEEP \r\n`
            // -- and omits the leading CRLF when #ENERGY precedes it. Both are
            // handled by the custom parsers in modem::urc.)
            defmt::info!("Enabling sleep URCs (AT#SLEEPIND=0x44)...");
            uart.blocking_write(b"AT#SLEEPIND=0x44\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("SLEEPIND response", &buf[..n]);

            // Host -> modem wake. pwrkey_evt = 0b1111: enable, active LOW, pull
            // enabled, pull-up. Board maps RP2350 GPIO10 -> ST87M01 WAKE_UP (39).
            // uart_evt = 0b0011: enable, active low -- a free fallback wake path.
            //
            // Confirmed on hardware: one 50 ms pulse on GPIO10 wakes the modem,
            // which answers the very first AT ping ~150 ms later.
            defmt::info!("Arming wake pin (AT#WAKEUPEVENT=15,3)...");
            uart.blocking_write(b"AT#WAKEUPEVENT=15,3\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("WAKEUPEVENT response", &buf[..n]);

            // Modem -> host wake. RING fires before the modem sends ANY URC.
            //
            // <gpio> = 9 is the MODEM's GPIO_09, NOT the RP2350's GPIO8. Two
            // separate numbering spaces, and they are trivially confusable:
            //   ST87M01 datasheet DS14679 Rev 4, module pad table:
            //     pin 19 = GPIO_09, alt fn UART0_RTS   <-- this one
            //     pin 18 = GPIO_08, alt fn UART0_CTS
            //   Challenger+ datasheet 3.2:
            //     "NB-IoT INT | RP2350 GPIO8 | ST87M01 UART0_RTS (Pin 19)"
            // So: modem GPIO_09 (pin 19) is wired to RP2350 GPIO8.
            //
            // <level> = 0 (active low), <delay> = 100 ms pulse (valid range 10-300).
            //
            // This is the ONLY viable dormant-wake source for the RP2350 -- see
            // T3412_TEST's docs for why the POWMAN/AON alarm path is dead.
            defmt::info!("Arming RING pin (AT#RINGPIN=1,9,0,100)...");
            uart.blocking_write(b"AT#RINGPIN=1,9,0,100\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("RINGPIN response", &buf[..n]);

            // <hold_time> = 300 s: seconds after the last AT command before the
            // modem *auto*-sleeps. Deliberately long -- GNSS fixes arrive as
            // #GNSSFIX URCs with no AT traffic to keep the hold timer alive, so a
            // short value would drop the modem into PSM mid-fix. We never rely on
            // auto-sleep; entry is always the explicit bare AT#SLEEPMODE.
            // <awake_time> = 0: stuck-watchdog disabled (only arms above 600 s).
            defmt::info!("Enabling sleep mode (AT#SLEEPMODE=1,300,0)...");
            uart.blocking_write(b"AT#SLEEPMODE=1,300,0\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("SLEEPMODE response", &buf[..n]);

            // Network PSM contract. ONE CPSMS call -- issuing it twice would mean
            // the last one silently wins, which is exactly the sort of thing that
            // wastes an afternoon.
            //
            // Positions 2 and 3 (requested_periodic_RAU, requested_GPRS_READY_timer)
            // are "not supported in NB-IOT ... any input will be ignored" (AT manual
            // 6.9), hence the empty middle args. The two timer values are String
            // type and MUST be quoted -- the manual's own example writes "01000111".
            //
            // Argument ORDER is load-bearing and swapping it is silent: position 4
            // is T3412 (GPRS Timer 3), position 5 is T3324 (GPRS Timer 2), and they
            // index different unit tables. See the const docs above.
            let t3412: &[u8] = if USE_TEST_TAU {
                T3412_TEST
            } else {
                T3412_PRODUCTION
            };
            if USE_TEST_TAU {
                defmt::info!("Requesting PSM: TAU = 1 min (TEST), active time = 2 s");
            } else {
                defmt::info!("Requesting PSM: TAU = 15 min (production), active time = 2 s");
            }
            uart.blocking_write(b"AT+CPSMS=1,,,\"").unwrap();
            uart.blocking_write(t3412).unwrap();
            uart.blocking_write(b"\",\"").unwrap();
            uart.blocking_write(T3324).unwrap();
            uart.blocking_write(b"\"\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("CPSMS response", &buf[..n]);

            defmt::info!("Reading back stored PSM request (AT+CPSMS?)...");
            uart.blocking_write(b"AT+CPSMS?\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("CPSMS? response", &buf[..n]);

            // n=4 appends <Active-Time>,<Periodic-TAU> to the +CEREG URC -- the
            // only way to learn what the network actually GRANTED, as opposed to
            // what we asked for (AT manual 6.9).
            //
            // <stat> stays the first field, so network_task's cereg_registered()
            // parse is unaffected; ModemUrc::Cereg already reserves String<96>.
            //
            // Caveat: the URC only fires on a registration STATE CHANGE. If the
            // modem is already registered it stays silent, and you learn nothing.
            // Read it back explicitly with `AT+CEREG?` if you need the number.
            defmt::info!("Enabling PSM-form registration URC (AT+CEREG=4)...");
            uart.blocking_write(b"AT+CEREG=4\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("CEREG response", &buf[..n]);

            // Read back what the network actually granted. Requested != granted --
            // a 1-minute TAU in particular is aggressive and carriers often floor
            // it much higher. Worth knowing BEFORE interpreting a long silence as
            // "the modem never self-woke".
            defmt::info!("Reading granted PSM timers (AT+CEREG?)...");
            uart.blocking_write(b"AT+CEREG?\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("CEREG? response", &buf[..n]);
            defmt::info!(
                "EXPECT +CEREG: 4,<stat>,...,\"<Active-Time>\",\"<Periodic-TAU>\" -- the last two are GRANTED"
            );
        } else {
            // Always-awake: keeps the modem responsive for AT/MQTT debugging.
            defmt::info!("PSM disabled by PROVISION_PSM (AT#SLEEPMODE=0)...");
            uart.blocking_write(b"AT#SLEEPMODE=0\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("SLEEPMODE response", &buf[..n]);

            uart.blocking_write(b"AT+CPSMS=0\r").unwrap();
            let n = read_line(&mut uart, &mut buf);
            log_response("CPSMS response", &buf[..n]);
        }

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

        // AT#SLEEPMODE / AT#WAKEUPEVENT / AT#SLEEPIND / AT#RINGPIN are all inert
        // until this reboot.
        defmt::info!("Saving to NVM (AT#RESET=1)...");
        uart.blocking_write(b"AT#RESET=1\r").unwrap();
        defmt::info!("Done. Wait 15s then flash main firmware.");
        if PROVISION_PSM {
            defmt::info!("PSM provisioned. Modem will auto-sleep 300s after the last AT command.");
        }
    } else {
        defmt::error!("Cert NOT stored - skipping verify and reset. Fix upload first.");
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}