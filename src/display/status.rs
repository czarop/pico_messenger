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
//! # Button peeks during DORMANT
//!
//! The button is ALSO a DORMANT wake source (armed by `crate::power` on a
//! stolen second `Input` of the same pin -- see there). A press while asleep
//! wakes the chip, `power` signals [`SHOW_PANEL`], this task runs the panel,
//! signals [`PANEL_DONE`], and `power` re-enters the SAME sleep -- the cycle
//! never notices. Conversely, if a sleep is about to start while the panel is
//! up, `power` signals [`CLOSE_PANEL`] and waits for [`PANEL_DONE`] so the
//! clocks never stop with a live frame on the OLED.
//!
//! Sleep-duration bookkeeping: `modem_task` records the RTC seconds-of-day at
//! every AUTO-sleep entry via [`note_sleep_entry`]. A peek re-entry does NOT
//! update it, so the panel's "sleep: probe 4m32s" is true time asleep. The RTC
//! calendar is used (not `Instant`) because `embassy_time` freezes in DORMANT.
//!
//! Known limit (fine for a debug aid): `Instant`-based ages ("pub: OK 12s")
//! still freeze across DORMANT -- read them as "of awake time".

use core::cell::RefCell;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, Either3, select, select3};
use embassy_rp::gpio::Input;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};

use crate::display::screen::{Display, StatusScreen};
use crate::modem::mqtt::state::MqttStackState;
use crate::sensors::battery_meter::BatteryLevel;

/// How long the panel stays on after a button press before blanking again.
/// Also bounds how long a scheduled wake can be delayed by a mid-sleep peek
/// (the peek loop in `power` only re-checks its wake pads after PANEL_DONE).
const PANEL_TIMEOUT_SECS: u32 = 30;

/// Redraw period while the panel is on. 1 Hz is plenty and keeps contention on
/// the shared I2C0 bus (telemetry, RTC) negligible.
const REDRAW: Duration = Duration::from_secs(1);

// ---- panel <-> power handshake ----

/// power -> display: open the panel (button woke the chip out of DORMANT).
pub static SHOW_PANEL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// power -> display: blank NOW (a sleep is about to stop the clocks).
pub static CLOSE_PANEL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// display -> power: panel is blanked; safe to (re)enter DORMANT.
pub static PANEL_DONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// True while the OLED is actually lit. `power` consults this to decide
/// whether a CLOSE_PANEL round-trip is needed before dormanting.
pub static PANEL_VISIBLE: AtomicBool = AtomicBool::new(false);

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
    /// RTC seconds-of-day at the last AUTO-sleep entry. `None` while awake.
    /// Deliberately untouched by peek re-entries.
    sleep_entry_secs: Option<u32>,
}

static STATUS: Mutex<CriticalSectionRawMutex, RefCell<StatusInner>> =
    Mutex::new(RefCell::new(StatusInner {
        registered: false,
        rsrp_dbm: None,
        last_pub: None,
        fix_count: 0,
        nofix_count: 0,
        sleep: SleepPhase::Awake,
        sleep_entry_secs: None,
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

/// Mark the phase without a sleep-entry timestamp -- use for [`SleepPhase::Awake`].
pub fn set_sleep(phase: SleepPhase) {
    STATUS.lock(|s| {
        let mut s = s.borrow_mut();
        s.sleep = phase;
        s.sleep_entry_secs = None;
    });
}

/// Record an AUTO-sleep entry: phase + RTC seconds-of-day at entry. Called by
/// `modem_task` at every scheduled sleep site; NOT called on a peek re-entry,
/// so the displayed "asleep for" spans the whole sleep, peeks included.
pub fn note_sleep_entry(phase: SleepPhase, rtc_secs_of_day: Option<u32>) {
    STATUS.lock(|s| {
        let mut s = s.borrow_mut();
        s.sleep = phase;
        s.sleep_entry_secs = rtc_secs_of_day;
    });
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
/// `rtc_now_secs` is the current RTC seconds-of-day (for sleep age), read by
/// the display task each redraw; everything else comes from the slots above
/// plus the telemetry slots.
pub fn render(mqtt: Option<MqttStackState>, rtc_now_secs: Option<u32>) -> StatusScreen {
    let (registered, rsrp, last_pub, fixes, nofixes, sleep, sleep_entry) = STATUS.lock(|s| {
        let s = s.borrow();
        (
            s.registered,
            s.rsrp_dbm,
            s.last_pub,
            s.fix_count,
            s.nofix_count,
            s.sleep,
            s.sleep_entry_secs,
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
        // Age via the RTC wall clock (survives DORMANT; `Instant` does not).
        match (sleep, sleep_entry, rtc_now_secs) {
            (SleepPhase::Awake, _, _) | (_, None, _) | (_, _, None) => {
                line(format_args!("sleep: {}", phase))
            }
            (_, Some(entry), Some(now)) => {
                let secs = crate::rtc::elapsed_secs(entry, now);
                if secs >= 60 {
                    line(format_args!("sleep: {} {}m{:02}s", phase, secs / 60, secs % 60))
                } else {
                    line(format_args!("sleep: {} {}s", phase, secs))
                }
            }
        }
    };

    StatusScreen {
        battery,
        message: [net, mqtt_line, pub_line, fix_line, sleep_line],
    }
}

// ---- display task ----

/// Own the display and button. Panel off until either a local button press
/// (awake) or a [`SHOW_PANEL`] request from `power` (button woke the chip from
/// DORMANT); then redraw at 1 Hz for [`PANEL_TIMEOUT_SECS`], or until pressed
/// again, or until `power` demands [`CLOSE_PANEL`] because a sleep is due.
///
/// Every display I2C result is deliberately ignored after logging: transient
/// NAKs on the shared bus are expected and must not panic (same lesson as the
/// BNO085 init retries).
#[embassy_executor::task]
pub async fn display_task(
    mut display: Display,
    mut button: Input<'static>,
    mut mqtt_rx: embassy_sync::watch::Receiver<
        'static,
        CriticalSectionRawMutex,
        MqttStackState,
        3,
    >,
) {
    if display.turn_display_off().await.is_err() {
        defmt::warn!("display: initial blank failed");
    }

    loop {
        // Arm on a local press (active low) OR a peek request from power.
        let via_button = match select(button.wait_for_falling_edge(), SHOW_PANEL.wait()).await {
            Either::First(()) => true,
            Either::Second(()) => false,
        };
        if via_button {
            // Debounce, then wait for release so the same press cannot
            // immediately register as the "turn off" press below.
            Timer::after(Duration::from_millis(50)).await;
            button.wait_for_high().await;
            Timer::after(Duration::from_millis(50)).await;
        }

        PANEL_VISIBLE.store(true, Ordering::Relaxed);
        if display.turn_display_on().await.is_err() {
            defmt::warn!("display: turn on failed");
        }

        let mut shown_secs = 0u32;
        loop {
            let rtc_now = crate::rtc::now_secs_of_day().await;
            let screen = render(mqtt_rx.try_get(), rtc_now);
            if display.show_message(screen).await.is_err() {
                // Transient I2C NAK: skip this frame, never panic.
                defmt::warn!("display: redraw failed");
            }

            match select3(
                Timer::after(REDRAW),
                button.wait_for_falling_edge(),
                CLOSE_PANEL.wait(),
            )
            .await
            {
                Either3::First(()) => {
                    shown_secs += 1;
                    if shown_secs >= PANEL_TIMEOUT_SECS {
                        break;
                    }
                }
                // Second press: blank early. CLOSE_PANEL: a sleep is due.
                Either3::Second(()) | Either3::Third(()) => break,
            }
        }

        if display.turn_display_off().await.is_err() {
            defmt::warn!("display: blank failed");
        }
        PANEL_VISIBLE.store(false, Ordering::Relaxed);

        // Order matters: release power FIRST (it may be waiting to sleep and
        // must not be blocked on the user still holding the button), then eat
        // any duplicate open request (a DORMANT button wake can both satisfy
        // our local edge-wait AND leave SHOW_PANEL signalled), then wait for
        // release before re-arming so a held button can't retrigger.
        PANEL_DONE.signal(());
        SHOW_PANEL.reset();
        button.wait_for_high().await;
        Timer::after(Duration::from_millis(50)).await;
    }
}