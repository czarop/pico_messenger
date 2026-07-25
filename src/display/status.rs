//! Debug status panel: shared status slots + a button-driven OLED display task.
//!
//! Exists because the debug probe drops SWD during the modem's TLS-connect
//! current surge, taking the RTT log with it. This panel shows the *current*
//! device state on the SH1107 OLED with no probe attached at all.
//!
//! Design (deliberate, do not "improve" back the other way):
//! * NOT a log mirror -- `defmt` strings do not exist on-device, and an OLED
//!   holds ~5 lines. Tasks write their current state into cheap shared slots
//!   (same idea as `sensors::telemetry`) and this module renders a snapshot.
//! * The panel is OFF by default. FeatherWing button A (Feather D9 = GPIO25 on
//!   the Challenger+) turns it on for [`PANEL_TIMEOUT_SECS`]; pressing again
//!   turns it off early. While off, nothing is drawn and the I2C bus is idle.
//! * MQTT stack phase is read straight from the `MQTT_STATE` watch (it has one
//!   spare receiver slot) rather than duplicated through a setter.
//! * A display fault must never take the device down: every I2C result in the
//!   task is log-and-continue. This feature exists to AID debugging.
//!
//! Known limits (fine for a debug aid):
//! * While the host is in DORMANT the task is halted -- a button press does
//!   nothing until the next wake. (To-do: button as a DORMANT wake source.)
//! * `Instant`-based ages freeze across DORMANT, so "pub: OK 12s" undercounts
//!   if a sleep happened in between. The value is still useful as "since the
//!   last publish of awake time".

use core::cell::RefCell;
use core::fmt::Write;

use embassy_futures::select::{Either, select};
use embassy_rp::gpio::Input;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant, Timer};

use crate::display::screen::{Display, StatusScreen};
use crate::modem::mqtt::state::MqttStackState;
use crate::sensors::battery_meter::BatteryLevel;

/// How long the panel stays on after a button press before blanking again.
const PANEL_TIMEOUT_SECS: u32 = 10000;

/// Redraw period while the panel is on. 1 Hz is plenty and keeps contention on
/// the shared I2C0 bus (telemetry, RTC) negligible.
const REDRAW: Duration = Duration::from_secs(1);

/// Which sleep-regime phase `modem_task` is currently in, for the bottom line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SleepPhase {
    /// Doing cycle work (bring-up, fix, publish).
    Awake,
    /// Timed rest on the RTC cadence (moving, or probe saw motion).
    Resting,
    /// 5-minute stationary motion probe.
    Probe,
    /// Indefinite motion-only sleep (parked or blind).
    DeepRest,
}

struct StatusInner {
    registered: bool,
    rsrp_dbm: Option<i16>,
    /// Outcome and time of the last publish attempt.
    last_pub: Option<(bool, Instant)>,
    fix_count: u16,
    nofix_count: u16,
    sleep: SleepPhase,
}

static STATUS: Mutex<CriticalSectionRawMutex, RefCell<StatusInner>> =
    Mutex::new(RefCell::new(StatusInner {
        registered: false,
        rsrp_dbm: None,
        last_pub: None,
        fix_count: 0,
        nofix_count: 0,
        sleep: SleepPhase::Awake,
    }));

// ---- setters: cheap, non-async, safe from any task ----

pub fn set_registered(registered: bool) {
    STATUS.lock(|s| s.borrow_mut().registered = registered);
}

pub fn set_rsrp(dbm: Option<i16>) {
    STATUS.lock(|s| s.borrow_mut().rsrp_dbm = dbm);
}

/// Record the outcome of a publish attempt (true = believed delivered).
pub fn note_publish(delivered: bool) {
    STATUS.lock(|s| s.borrow_mut().last_pub = Some((delivered, Instant::now())));
}

/// A successful GNSS fix: bump the fix counter and clear the no-fix streak,
/// mirroring `NO_FIX_COUNT` semantics in `modem_task`.
pub fn note_fix() {
    STATUS.lock(|s| {
        let mut s = s.borrow_mut();
        s.fix_count = s.fix_count.saturating_add(1);
        s.nofix_count = 0;
    });
}

/// A no-fix cycle: display the current consecutive-strike count.
pub fn note_nofix(strikes: u8) {
    STATUS.lock(|s| s.borrow_mut().nofix_count = strikes as u16);
}

pub fn set_sleep(phase: SleepPhase) {
    STATUS.lock(|s| s.borrow_mut().sleep = phase);
}

// ---- rendering ----

/// One line of the panel. FONT_6X10 on a 128px-wide panel shows 21 chars;
/// the buffer is 24 so a slightly long line clips rather than errors.
fn line(args: core::fmt::Arguments) -> Option<heapless::String<24>> {
    let mut s = heapless::String::new();
    // Truncation on overflow is acceptable here; never fail the redraw over it.
    let _ = s.write_fmt(args);
    Some(s)
}

/// Format the current status snapshot as a [`StatusScreen`].
///
/// `mqtt` is the stack phase from the caller's `MQTT_STATE` watch receiver;
/// everything else comes from the slots above plus the telemetry slots.
pub fn render(mqtt: Option<MqttStackState>) -> StatusScreen {
    let (registered, rsrp, last_pub, fixes, nofixes, sleep) = STATUS.lock(|s| {
        let s = s.borrow();
        (
            s.registered,
            s.rsrp_dbm,
            s.last_pub,
            s.fix_count,
            s.nofix_count,
            s.sleep,
        )
    });

    // Battery icon straight from the telemetry slots; no setter needed. Shows
    // Empty until the first successful read (there is no "unknown" glyph).
    let battery = match crate::sensors::telemetry::battery() {
        Some((soc, charging)) => BatteryLevel::from_soc(soc as u16, charging),
        None => BatteryLevel::Empty,
    };

    let net = {
        let reg = if registered { "reg" } else { "---" };
        match rsrp {
            Some(dbm) => line(format_args!("NET {} {}dB", reg, dbm)),
            None => line(format_args!("NET {} ?dB", reg)),
        }
    };

    let mqtt_line = {
        let phase = match mqtt {
            Some(MqttStackState::Down) => "down",
            Some(MqttStackState::IpUp) => "ip up",
            Some(MqttStackState::SocketReady(_)) => "socket",
            Some(MqttStackState::MqttReady) => "ready",
            None => "?",
        };
        line(format_args!("MQTT: {}", phase))
    };

    let pub_line = match last_pub {
        Some((ok, at)) => {
            let outcome = if ok { "OK" } else { "FAIL" };
            let age = at.elapsed().as_secs().min(9999);
            line(format_args!("pub: {} {}s", outcome, age))
        }
        None => line(format_args!("pub: none")),
    };

    let fix_line = line(format_args!("fix {} nofix {}", fixes, nofixes));

    let sleep_line = {
        let phase = match sleep {
            SleepPhase::Awake => "awake",
            SleepPhase::Resting => "rest",
            SleepPhase::Probe => "probe",
            SleepPhase::DeepRest => "deep",
        };
        line(format_args!("sleep: {}", phase))
    };

    StatusScreen {
        battery,
        message: [net, mqtt_line, pub_line, fix_line, sleep_line],
    }
}

// ---- display task ----

/// Own the display and button. Panel off until the button is pressed; then
/// redraw at 1 Hz for [`PANEL_TIMEOUT_SECS`] or until pressed again.
///
/// Every display I2C result is deliberately ignored after logging: transient
/// NAKs on the shared bus are expected and must not panic (same lesson as the
/// BNO085 init retries).
#[embassy_executor::task]
pub async fn display_task(
    mut display: Display,
    mut button: Input<'static>,
    mut mqtt_rx: embassy_sync::watch::Receiver<'static, CriticalSectionRawMutex, MqttStackState, 3>,
) {
    if display.turn_display_off().await.is_err() {
        defmt::warn!("display: initial blank failed");
    }

    loop {
        // Arm on a press (active low). Debounce, then wait for release so the
        // same press cannot immediately register as the "turn off" press below.
        button.wait_for_falling_edge().await;
        Timer::after(Duration::from_millis(50)).await;
        button.wait_for_high().await;
        Timer::after(Duration::from_millis(50)).await;

        if display.turn_display_on().await.is_err() {
            defmt::warn!("display: turn on failed");
        }

        let mut shown_secs = 0u32;
        loop {
            let screen = render(mqtt_rx.try_get());
            if display.show_message(screen).await.is_err() {
                // Transient I2C NAK: skip this frame, never panic.
                defmt::warn!("display: redraw failed");
            }

            match select(Timer::after(REDRAW), button.wait_for_falling_edge()).await {
                Either::First(()) => {
                    shown_secs += 1;
                    if shown_secs >= PANEL_TIMEOUT_SECS {
                        break;
                    }
                }
                // Second press: blank early.
                Either::Second(()) => break,
            }
        }

        if display.turn_display_off().await.is_err() {
            defmt::warn!("display: blank failed");
        }

        // If the off-press is still held, wait it out before re-arming.
        button.wait_for_high().await;
        Timer::after(Duration::from_millis(50)).await;
    }
}
