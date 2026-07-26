//! Host (RP2350) low-power sleep, woken by the PCF8523 RTC's INT line.
//!
//! # Why this shape
//!
//! DORMANT stops every clock on the RP2350 and can only be exited by a GPIO edge
//! (or a full reset). Two facts, both hardware-verified, forced this design:
//!
//! * The POWMAN/AON alarm cannot wake DORMANT. `aon_timer`'s `DormantOnly` sets
//!   POWMAN's `PWRUP_ON_ALARM`, a power-STATE request, but `clocks::dormant_sleep`
//!   puts ROSC into "coma"; the alarm fires but the chip never wakes. (The AON
//!   timer itself works fine -- counts and alarms -- it just cannot rouse a
//!   comatose ROSC.)
//! * The modem's own PSM self-wake is floored by the network at a 4-hour TAU, far
//!   too coarse for a 15-minute tracking cadence.
//!
//! So an external periodic GPIO edge is required. The PCF8523 countdown timer
//! provides exactly that on its INT/SQW pin. See `crate::rtc`.
//!
//! The AON timer that lived here previously is gone: it existed to wake (it can't)
//! and to measure elapsed time across sleep (the RTC's wall clock does that
//! better). Nothing here touches POWMAN any more.
//!
//! # The debug probe does not survive DORMANT
//!
//! SWD/RTT die when the clocks stop and probe-rs will not reattach, so a real
//! DORMANT build emits no defmt past the sleep. Two consequences:
//!
//! * `defmt-rtt` MUST be built with `disable-blocking-mode`, or the first log
//!   after wake blocks forever on a full RTT buffer and the board hangs. (This is
//!   what produced an earlier false "it never woke" result.)
//! * Validate real sleep by whether MQTT messages keep arriving, not by logs.
//!   Debug the surrounding logic under `--features mock_host_sleep`.
//!
//! # The mock keeps the real wake source
//!
//! Under `mock_host_sleep`, `sleep_dormant` waits on the *real* RTC INT edge
//! instead of entering DORMANT. So a mock build exercises the actual RTC
//! countdown config, the actual INT line, and the actual cadence -- everything
//! except the power state -- with the probe attached and logging live.

use embassy_rp::gpio::Input;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

/// The RTC INT line, parked in a static so [`sleep_dormant`] can be a free
/// function -- no `Input` threaded through `initiate_modem` and the two
/// `modem_task` spawn sites. Populated once by [`init`], from `startup`.
static RTC_INT: Mutex<CriticalSectionRawMutex, Option<Input<'static>>> = Mutex::new(None);

/// Hand the RTC INT pin to the power module. Call once, from `startup`, after
/// constructing the `Input` (which must be `Pull::Up` -- the PCF8523 INT is
/// open-drain active-low).
pub async fn init(rtc_int: Input<'static>) {
    *RTC_INT.lock().await = Some(rtc_int);
}

// ---------------------------------------------------------------------------
// Button peek support
// ---------------------------------------------------------------------------
//
// The display button (FeatherWing A, GPIO25) is also a DORMANT wake source: a
// press while asleep wakes the chip, the status panel runs for its on-period,
// and the SAME sleep is re-entered -- the tracking cycle never notices. The
// peek loops below own that behaviour so no caller of the sleep functions has
// to know button wakes exist.
//
// # The pin steal, and why it is sound
//
// `display_task` owns the button `Input` for awake-time presses, and embassy
// gives no way to arm a dormant wake on a pad without `&mut Input`. Rather
// than route the one Input through a mutex-and-handshake dance between two
// tasks, the sleep path constructs a SECOND `Input` on the same pin via
// `Peri::steal()` -- same config (input, Pull::Up), so nothing observable
// changes -- arms the dormant-wake guard on it, and after wake `mem::forget`s
// it. The forget is the load-bearing part: `Input`'s Drop deconfigures the
// pad, which would kill display_task's copy. Forgetting leaks nothing (the
// struct is trivially small and the pad is left exactly as display_task
// expects).
//
// # Wake discrimination
//
// All three wake lines are level-holding active-low: the RTC INT holds until
// its flag is cleared, the BNO085 HINT until the pending report is read, and
// the button for as long as the human holds it. So after `dormant_sleep()`
// returns, pad LEVELS identify the cause -- and a real source always wins over
// the button, so a race resolves in favour of the cycle.
//
// # Scheduled wake landing mid-peek
//
// The dormant pad triggers are edge-based, so an RTC/HINT edge that fires
// while the panel is up would be slept through on blind re-entry. Hence every
// re-entry is preceded by a level check of the real sources (in
// `sleep_dormant` directly; in the IMU paths via the closure's HINT check
// running before each dormant). Worst case a scheduled wake is delayed by the
// panel's remaining on-time (<=30s); the cadence self-corrects because all
// sleep timing is measured against the RTC wall clock.

/// Build the second `Input` on the button pin. See module notes above; every
/// use MUST be paired with `core::mem::forget` after the guard is dropped.
fn steal_button() -> Input<'static> {
    // SAFETY: the only other handle is display_task's Input with identical
    // configuration; we never reconfigure and never let Drop run.
    let pin = unsafe { embassy_rp::peripherals::PIN_25::steal() };
    Input::new(pin, embassy_rp::gpio::Pull::Up)
}

/// Falling-edge dormant wake: all three sources are open-drain/active-low, and
/// an edge (not level) trigger avoids immediate re-wake off a held line.
fn edge_low() -> embassy_rp::gpio::DormantWakeConfig {
    embassy_rp::gpio::DormantWakeConfig {
        edge_high: false,
        edge_low: true,
        level_high: false,
        level_low: false,
    }
}

/// If the status panel is lit, ask it to blank and wait until it has -- the
/// clocks must never stop with a live frame on the OLED (it would display
/// stale data for the whole sleep and burn panel current doing it).
async fn close_panel_if_visible() {
    use crate::display::status;
    if status::PANEL_VISIBLE.load(core::sync::atomic::Ordering::Relaxed) {
        status::PANEL_DONE.reset();
        status::CLOSE_PANEL.signal(());
        status::PANEL_DONE.wait().await;
    }
}

/// Run one button peek: open the panel, wait until it closes (timeout or
/// second press). Residual race, accepted: a press landing between PANEL_DONE
/// and the pads re-arming is lost -- press again.
async fn run_peek() {
    use crate::display::status;
    defmt::info!("button peek: showing status panel, then re-entering sleep");
    status::PANEL_DONE.reset();
    status::SHOW_PANEL.signal(());
    status::PANEL_DONE.wait().await;
}

/// Sleep the RP2350 until the RTC's INT line pulses.
///
/// **Real:** arm GPIO dormant-wake on `rtc_int` (falling edge -- the PCF8523 INT
/// is open-drain active-low) and enter DORMANT. Blocks until the edge arrives.
/// This is a blocking call, not an await, and that is correct: embassy's executor
/// is cooperative and single-threaded, so the instant this blocks, every other
/// task freezes where it last awaited. No quiescence protocol is needed.
///
/// The only invariant that matters: no peripheral transaction may be in flight
/// when the clocks stop. `modem_task` only reaches here after GNSS is stopped, the
/// MQTT session is torn down, and the modem has confirmed PSM entry with `#SLEEP`
/// -- by which point `command_task` is parked on its channel and nothing is
/// outstanding. Do not call this from elsewhere without re-establishing that.
///
/// **Mock (`mock_host_sleep`):** await the real INT falling edge. Nothing sleeps;
/// probe and logging stay live.
///
/// `rtc_int` (configured `Pull::Up`) must have been handed over via [`init`]
/// before the first call. Panics if not -- sleeping with no wake source armed
/// would be an unrecoverable hang, so failing loudly is correct.
/// Sleep for `target_secs` of real time, to within about a second.
///
/// A single `arm` cannot be trusted for this: Timer B's source divider free-runs,
/// so a request of N ticks fires anywhere in `(N-1, N]` ticks -- up to a minute
/// early on the 1/60 Hz source. This closes the loop instead:
///
/// 1. read the RTC calendar (a 1 Hz counter that survives DORMANT),
/// 2. arm SHORT of what remains, so we can never overshoot,
/// 3. sleep,
/// 4. on wake, measure what actually elapsed and repeat with the remainder.
///
/// The coarse arm gets us to within a minute, the fine arm to within a second, so
/// this normally costs two DORMANT cycles rather than one. Errors do not
/// accumulate across cycles because every step is measured against real time.
///
/// Falls back to a single plain sleep if the RTC calendar is unreadable, so a
/// clock fault degrades the cadence rather than hanging the device.
/// Wait for the RTC INT falling edge WITHOUT dormanting. Mock/test use, and for
/// racing the RTC against another wake source while clocks are up.
pub async fn wait_rtc_int() {
    let mut guard = RTC_INT.lock().await;
    let rtc_int = guard
        .as_mut()
        .expect("power::wait_rtc_int before power::init");
    rtc_int.wait_for_falling_edge().await;
}

/// Run `f` -- which must enter DORMANT (e.g. `dormant_sleep_on_hint`) and
/// return whether ITS OWN wake line (BNO085 HINT) is asserted -- with the RTC
/// INT pad and the peek button armed as additional dormant wake sources.
/// Returns `f`'s verdict from the final iteration: `true` = the caller's
/// source fired, `false` = the RTC (or a spurious wake) ended the sleep.
///
/// DORMANT halts all clocks and is woken by ANY armed pad, but every guard
/// must be alive across the single `dormant_sleep()` call -- so a multi-source
/// sleep has to be assembled in one place. The task that owns the other pad
/// (currently `imu_task`, which owns the BNO085 HINT line) arms its pad and
/// dormants inside `f`.
///
/// `f` returning its own line's LEVEL is what makes peeks and mid-peek wakes
/// safe here: it is re-evaluated before every re-entry, so a HINT that
/// asserted while the panel was up is seen, not slept through.
pub async fn with_rtc_dormant_wake(mut f: impl FnMut() -> bool) -> bool {
    let mut guard = RTC_INT.lock().await;
    let rtc_int = guard
        .as_mut()
        .expect("power::with_rtc_dormant_wake before power::init");

    close_panel_if_visible().await;

    loop {
        let mut btn = steal_button();
        let own_source = {
            let _btn_wake = btn.dormant_wake(edge_low());
            let _rtc_wake = rtc_int.dormant_wake(edge_low());
            f()
        };
        let rtc_fired = rtc_int.is_low();
        let btn_pressed = btn.is_low();
        core::mem::forget(btn); // Drop would deconfigure display_task's pad

        if own_source || rtc_fired {
            return own_source;
        }
        if btn_pressed {
            run_peek().await;
            continue; // same sleep, same pads; f re-checks HINT on entry
        }
        // Spurious: neither source nor button. Report as an RTC-side wake --
        // identical to the pre-peek behaviour (any non-HINT wake ended the
        // sleep), and `sleep_for_secs`-style callers re-measure anyway.
        return false;
    }
}

/// DeepRest variant of [`with_rtc_dormant_wake`]: same peek behaviour, but
/// loops until `f`'s OWN source fired -- there is no RTC alarm in this mode
/// (motion is the only legitimate end), so an RTC-attributed or spurious wake
/// just re-enters. The RTC pad is deliberately NOT armed.
pub async fn with_button_peek_dormant(mut f: impl FnMut() -> bool) {
    close_panel_if_visible().await;

    loop {
        let mut btn = steal_button();
        let own_source = {
            let _btn_wake = btn.dormant_wake(edge_low());
            f()
        };
        let btn_pressed = btn.is_low();
        core::mem::forget(btn);

        if own_source {
            return;
        }
        if btn_pressed {
            run_peek().await;
        }
        // Not our source: re-enter (f re-checks its line on entry).
    }
}

pub async fn sleep_for_secs(target_secs: u32) {
    let Some(start) = crate::rtc::now_secs_of_day().await else {
        defmt::error!("sleep_for_secs: RTC calendar unreadable — single coarse sleep");
        let _ = crate::rtc::arm_wake_secs(target_secs).await;
        sleep_dormant().await;
        return;
    };

    loop {
        let elapsed = match crate::rtc::now_secs_of_day().await {
            Some(now) => crate::rtc::elapsed_secs(start, now),
            None => {
                defmt::error!("sleep_for_secs: RTC read failed mid-sleep — giving up early");
                return;
            }
        };

        if elapsed >= target_secs {
            return;
        }
        let remaining = target_secs - elapsed;

        // Arm short of the remainder; 0 means nothing could be armed.
        if crate::rtc::arm_wake_secs(remaining).await == 0 {
            defmt::error!("sleep_for_secs: could not arm wake — aborting sleep");
            return;
        }

        sleep_dormant().await;
    }
}

pub async fn sleep_dormant() {
    let mut guard = RTC_INT.lock().await;
    let rtc_int = guard
        .as_mut()
        .expect("power::sleep_dormant before power::init");

    #[cfg(feature = "mock_host_sleep")]
    {
        defmt::info!("MOCK host sleep: waiting on real RTC INT (RP2350 stays awake)");
        rtc_int.wait_for_falling_edge().await;
        defmt::info!("MOCK host sleep: RTC INT fired, waking");
    }

    #[cfg(not(feature = "mock_host_sleep"))]
    {
        close_panel_if_visible().await;

        defmt::info!("entering DORMANT (RTC INT + button wake) -- last log until wake");

        loop {
            // Both guards must be alive across the single dormant_sleep().
            // They disarm on drop; the button Input must then be forgotten,
            // never dropped (see the pin-steal notes above).
            let mut btn = steal_button();
            {
                let _btn_wake = btn.dormant_wake(edge_low());
                let _rtc_wake = rtc_int.dormant_wake(edge_low());

                // Stops all clocks. Returns only after an armed pad's edge.
                embassy_rp::clocks::dormant_sleep();
            }

            // `embassy_time` is wrong now (its driver clock stopped and it
            // believes no time passed) -- rely on the RTC wall clock for
            // anything spanning the sleep.
            //
            // Discriminate by LEVEL: RTC INT holds low until its flag is
            // cleared, so if it fired -- even during a peek's panel window --
            // this check sees it and the real wake wins over the button.
            let rtc_fired = rtc_int.is_low();
            let btn_pressed = btn.is_low();
            core::mem::forget(btn);

            if rtc_fired {
                break;
            }
            if btn_pressed {
                run_peek().await;
                continue; // same sleep: re-arm both pads, back to DORMANT
            }
            // Spurious wake: neither line low. Return; sleep_for_secs
            // re-measures against the RTC and re-arms the remainder.
            break;
        }
    }
}