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

/// Run `f` with the RTC INT pad armed as a dormant wake source.
///
/// DORMANT halts all clocks and is woken by ANY armed pad, but every guard must
/// be alive across the single `dormant_sleep()` call — so a multi-source sleep
/// has to be assembled in one place. This lets the task that owns the other pad
/// (currently `imu_task`, which owns the BNO085 HINT line) add the RTC to its own
/// sleep: it arms its pad and calls `dormant_sleep()` inside `f`.
///
/// The guard disarms the pad on drop, after `f` returns.
pub async fn with_rtc_dormant_wake<R>(f: impl FnOnce() -> R) -> R {
    use embassy_rp::gpio::DormantWakeConfig;

    let mut guard = RTC_INT.lock().await;
    let rtc_int = guard
        .as_mut()
        .expect("power::with_rtc_dormant_wake before power::init");

    // INT is open-drain active-low: wake on the falling edge. A level trigger
    // would risk immediate re-wake while the flag is still asserted.
    let cfg = DormantWakeConfig {
        edge_high: false,
        edge_low: true,
        level_high: false,
        level_low: false,
    };
    let dormant = rtc_int.dormant_wake(cfg);
    let r = f();
    drop(dormant);
    r
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
        use embassy_rp::gpio::DormantWakeConfig;

        defmt::info!("entering DORMANT (RTC INT wake) -- last log until wake");

        // The guard configures the pad as a dormant-wake source and disarms it on
        // drop. INT is open-drain active-low, so wake on the falling edge; a level
        // trigger would risk immediate re-wake if the flag is still asserted.
        let cfg = DormantWakeConfig {
            edge_high: false,
            edge_low: true,
            level_high: false,
            level_low: false,
        };
        let dormant = rtc_int.dormant_wake(cfg);

        // Stops all clocks. Returns only after the RTC INT edge.
        embassy_rp::clocks::dormant_sleep();

        // Explicitly release the wake config. `embassy_time` is wrong now (its
        // driver clock stopped and it believes no time passed) -- rely on the RTC
        // wall clock for anything spanning the sleep.
        drop(dormant);
    }
}