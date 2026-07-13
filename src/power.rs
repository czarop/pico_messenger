//! Host (RP2350) low-power sleep.
//!
//! # Where the power actually goes
//!
//! With the modem in PSM the *host* is the entire problem. Challenger+ datasheet
//! §2.2 puts the ST87M01 at **1.2 uA** asleep; the RP2350 in DORMANT is on the
//! order of **1-3 mA**. The host is three orders of magnitude larger than the
//! modem. Everything in this module is about the RP2350, not the radio.
//!
//! # Why DORMANT and not POWMAN Pstate
//!
//! `embassy-rp` (rev 24da56d) ships `aon_timer`, `gpio::dormant_wake` and
//! `clocks::dormant_sleep()` -- everything needed for DORMANT. It does **not**
//! ship a POWMAN power-state driver: `pac::POWMAN` exposes every register
//! (`state`, `pwrup[0..3]`, `boot[0..3]`, `vreg`, ...), but the sequencing logic
//! is absent -- no `0x5AFE` password protocol, no Pstate transition handling, no
//! BOOT-register save/restore, no CORESIGHT check.
//!
//! Pstate would roughly halve the host draw again, but it costs a reimplementation
//! of pico-sdk's `hardware/powman` plus a RAM-resident resume stub, because waking
//! from Pstate re-enters the program from the start with RAM clobbered by crt0.
//! That is a large architectural tax to pay before we know the board's floor.
//! DORMANT preserves all state and resumes as a normal function return.
//!
//! Swapping DORMANT for Pstate later means reimplementing exactly one function --
//! [`sleep_until`] -- which is why the seam is here.
//!
//! # Two things that WILL bite
//!
//! **1. `embassy_time` stops dead.** `dormant_sleep()` halts every clock,
//! including TIMER0, which drives `embassy_time`. `Instant::now()` freezes for the
//! whole sleep and resumes as if no time passed. Relative `Timer::after` still
//! works, but any wall-clock reasoning across a sleep is wrong. Long-horizon
//! timing -- the publish cadence, the stationary window -- must come from
//! [`AonTimer::now`], which runs off LPOSC and keeps counting through DORMANT.
//!
//! **2. The debug probe does not come back.** SWD/RTT die when the clocks stop and
//! probe-rs will not reattach. A real DORMANT build produces *no defmt output past
//! the sleep*. Debug the logic under `--features mock_host_sleep` (probe attached,
//! full logging, `Timer::after` instead of a real sleep); validate the real thing
//! end-to-end by whether the MQTT messages keep arriving.
//!
//! # Accuracy
//!
//! LPOSC is a low-power RC oscillator, ~6% off untrimmed -- roughly +/-54 s on a
//! 15-minute interval. Irrelevant for a tracker. If tight timing is ever needed,
//! the fix is an external 32.768 kHz crystal, not a software change.

use embassy_rp::Peri;
use embassy_rp::aon_timer::{AlarmWakeMode, AonTimer, ClockSource, Config};
use embassy_rp::peripherals::POWMAN;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Duration;

#[cfg(not(feature = "mock_host_sleep"))]
use embassy_rp::clocks;

/// LPOSC nominal frequency. Required for DORMANT -- XOSC is powered down.
const LPOSC_FREQ_KHZ: u32 = 32;

/// The AON timer, parked in a static so [`sleep_until`] can be a free function.
///
/// This mirrors `modem::psm`'s enter/exit: `modem_task` calls `power::sleep_until`
/// directly, with no peripheral threaded through `initiate_modem` and the two
/// `modem_task` spawn sites.
///
/// `AonTimer` is a thin token (`PhantomData` + `Config`) over `pac::POWMAN`, so
/// parking it costs essentially nothing.
static AON: Mutex<CriticalSectionRawMutex, Option<AonTimer<'static>>> = Mutex::new(None);

/// Build the AON timer and start it counting. Call once, from `startup`.
///
/// Must be called before the first [`sleep_until`], including under
/// `mock_host_sleep` -- the mock ignores the timer, but keeping the call
/// unconditional means the real and mock paths do not diverge at startup.
///
/// Caller supplies the interrupt binding:
///
/// ```ignore
/// embassy_rp::bind_interrupts!(struct PowerIrqs {
///     POWMAN_IRQ_TIMER => embassy_rp::aon_timer::InterruptHandler;
/// });
/// power::init(p.POWMAN, PowerIrqs);
/// ```
pub fn init(
    powman: Peri<'static, POWMAN>,
    irq: impl embassy_rp::interrupt::typelevel::Binding<
        embassy_rp::interrupt::typelevel::POWMAN_IRQ_TIMER,
        embassy_rp::aon_timer::InterruptHandler,
    > + 'static,
) {
    let mut timer = AonTimer::new(
        powman,
        irq,
        Config {
            // LPOSC is mandatory: XOSC is powered down in DORMANT, so an
            // XOSC-clocked alarm would never fire and the device would never wake.
            clock_source: ClockSource::Lposc,
            clock_freq_khz: LPOSC_FREQ_KHZ,
            // Sets PWRUP_ON_ALARM: a hardware power-up event rather than an
            // interrupt, because in DORMANT the CPU clock is stopped and there is
            // nothing left to service an IRQ.
            //
            // CAVEAT: embassy documents the TIMER register as Secure-only and warns
            // this "may fail silently in Non-secure contexts". We build
            // `imagedef-secure-exe`, so this should hold -- but "fails silently" is
            // exactly the failure that looks like a hung board, so the first real
            // DORMANT test needs a positive signal (a wake actually happening), not
            // merely an absence of errors.
            alarm_wake_mode: AlarmWakeMode::DormantOnly,
            // alarm_wake_mode: AlarmWakeMode::Both,
        },
    );

    timer.set_counter(0);
    timer.start();

    // Uncontended at startup.
    let mut slot = AON
        .try_lock()
        .expect("power::init: AON mutex already held at startup");
    *slot = Some(timer);

    defmt::info!("AON timer started (LPOSC, dormant wake)");
}

/// Milliseconds on the AON timer's clock -- the only clock that survives DORMANT.
///
/// Use this, never `embassy_time::Instant`, for anything that must measure real
/// elapsed time across a sleep (publish cadence, stationary window, motion
/// timeouts). `Instant` loses the entire sleep duration; see the module docs.
///
/// Under `mock_host_sleep` the AON timer is still real and still running, so this
/// returns a true value in both builds. Only the *sleep* is faked.
pub async fn now_ms() -> u64 {
    AON.lock()
        .await
        .as_ref()
        .expect("power::now_ms before power::init")
        .now()
}

/// Sleep the RP2350 for `dur`, then return.
///
/// **Real build:** arms the AON alarm for `dur` and calls `clocks::dormant_sleep()`,
/// which stops every clock. This is a *blocking* call, not an await -- but that is
/// exactly right. Embassy's executor is cooperative and single-threaded, so the
/// moment this blocks, every other task freezes wherever it last awaited. No
/// quiescence protocol is needed; the tasks simply stop.
///
/// The one invariant that *does* matter: **no peripheral transaction may be in
/// flight when the clocks stop.** In practice this holds by construction --
/// `command_task` is the sole UART owner and parks on `COMMAND_CHANNEL.receive()`
/// when idle, and `modem_task` only reaches here after GNSS is stopped, the MQTT
/// session is torn down, and the modem has confirmed PSM entry with `#SLEEP`. By
/// then nothing is outstanding. Do not call this from anywhere else without
/// re-establishing that.
///
/// **Mock build (`mock_host_sleep`):** a plain `Timer::after`. Nothing sleeps, the
/// probe stays attached, defmt keeps logging, and every caller above this line --
/// cadence arithmetic, PSM ordering, state transitions -- still runs for real.
pub async fn sleep_until(dur: Duration) {
    #[cfg(feature = "mock_host_sleep")]
    {
        defmt::info!(
            "MOCK host sleep: {} ms (RP2350 stays awake)",
            dur.as_millis()
        );
        embassy_time::Timer::after(dur).await;
    }

    #[cfg(not(feature = "mock_host_sleep"))]
    {
        {
            let mut guard = AON.lock().await;
            let timer = guard
                .as_mut()
                .expect("power::sleep_until before power::init");

            if let Err(e) = timer.set_alarm_after(dur) {
                // Only failure mode is AlarmInPast, which for a positive duration
                // means the timer is not running. Refuse to sleep rather than
                // dormant with no wake source -- that is an unrecoverable hang.
                defmt::error!("AON set_alarm failed ({:?}) - NOT sleeping", e);
                return;
            }
        } // release before blocking: dormant_sleep() freezes the whole executor.

        // Last log line the probe will ever show in a real build.
        defmt::info!("entering DORMANT for {} ms", dur.as_millis());

        clocks::dormant_sleep();

        // Clocks are back. Everything else resumed exactly where it was parked --
        // except `embassy_time`, which believes no time passed at all.
        let mut guard = AON.lock().await;
        if let Some(timer) = guard.as_mut() {
            timer.clear_alarm();
        }
    }
}

pub async fn selftest() {
    let t0 = now_ms().await;
    defmt::info!("AON t0 = {} ms", t0);

    {
        let mut g = AON.lock().await;
        let timer = g.as_mut().unwrap();
        match timer.set_alarm_after(Duration::from_secs(5)) {
            Ok(()) => defmt::info!("alarm armed for +5s"),
            Err(e) => defmt::error!("set_alarm failed: {:?}", e),
        }
    }

    for i in 0..20 {
        embassy_time::Timer::after(Duration::from_millis(500)).await;
        let (now, fired) = {
            let g = AON.lock().await;
            let t = g.as_ref().unwrap();
            (t.now(), t.alarm_fired())
        };
        defmt::info!("t+{}ms: AON now={} fired={}", i * 500, now, fired);
        if fired {
            break;
        }
    }
}
