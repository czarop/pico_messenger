//! TEMPORARY diagnostic: watch the modem's RING line (RP2350 GPIO8).
//!
//! # What this is testing
//!
//! Option B's whole architecture rests on one unverified premise:
//!
//!   **A self-initiated PSM exit (the periodic TAU wake) produces a URC, and
//!   therefore a RING pulse.**
//!
//! We know RING fires "before sending any URC message" (AT manual, `AT#RINGPIN`),
//! and we have seen `#WAKEUP` on hardware -- but only ever after *we* pulsed
//! GPIO10 to wake the modem ourselves. Nobody has confirmed that the modem's own
//! T3412-driven wake announces itself the same way. If that wake is silent, there
//! is no URC, no RING pulse, no host wake, and Option B collapses.
//!
//! This matters because RING is now the ONLY viable dormant-wake source. The
//! POWMAN/AON alarm path is dead: `aon_timer`'s `DormantOnly` sets POWMAN's
//! `TIMER.PWRUP_ON_ALARM`, which requests a POWMAN *power-state* transition, but
//! `clocks::dormant_sleep()` does not enter a POWMAN power state -- it writes
//! "coma" to `ROSC.dormant()`. The only wake path embassy wires into ROSC
//! dormancy is GPIO dormant-wake. Confirmed on hardware: the AON timer counts and
//! its alarm fires correctly (`t+5000ms: now=5315 fired=true`), but it cannot
//! rouse a comatose ROSC. The board slept and never woke.
//!
//! # Pin
//!
//! RP2350 `GPIO8` <- ST87M01 `UART0_RTS` (pin 19) -- Challenger+ datasheet 3.2,
//! "Ring output to RP2350".
//!
//! Note the numbering trap: the *modem-side* pin is the modem's **GPIO_09**
//! (ST87M01 datasheet DS14679 Rev 4: pin 19 = GPIO_09, alt fn UART0_RTS), which is
//! what `AT#RINGPIN=1,9,0,100` refers to. The modem's GPIO_08 is a different pin
//! entirely (pin 18, UART0_CTS). Two numbering spaces, adjacent numbers.
//!
//! Configured active-low with a 100 ms pulse, so we watch for falling edges.
//!
//! # Reading the result
//!
//! Run a normal cycle, let `enter_psm()` succeed, then **do not call
//! `exit_psm()`** -- just wait. Within roughly one T3412 (set to 1 minute for this
//! test) you should see:
//!
//!   * `RING edge #1 (falling) at ...` from this task, AND
//!   * a `#WAKEUP` URC in the main log.
//!
//! Both present  -> Option B is real; build the dormant wake on GPIO8.
//! Silence       -> the TAU wake is silent; back to costing out POWMAN Pstate.
//!
//! A caveat on interpretation: RING fires before *any* URC, not just `#WAKEUP`.
//! So an edge is only meaningful if it lands during the PSM sleep window, well
//! after the last command. Edges during normal operation (GNSS fixes, `+CEREG`,
//! `#MQTTRECV`) are expected and prove only that RINGPIN is armed -- which is
//! itself worth knowing, but is not the thing under test.
//!
//! DELETE THIS FILE once the question is settled.

use embassy_rp::Peri;
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_rp::peripherals::PIN_8;
use embassy_time::Instant;

/// Log every edge on the modem's RING line.
///
/// Pull-up, because RING is configured active-low (`AT#RINGPIN=1,9,0,100`, where
/// `<level>=0`). It idles high and pulses low for ~100 ms.
#[embassy_executor::task]
pub async fn ring_watch_task(pin: Peri<'static, PIN_8>) -> ! {
    let mut ring = Input::new(pin, Pull::None); // let the modem drive it

    defmt::info!(
        "RING watch task spawned on GPIO8 (idle level = {})",
        match ring.get_level() {
            Level::High => "high",
            Level::Low => "low",
        }
    );

    let mut count: u32 = 0;

    loop {
        ring.wait_for_any_edge().await;
        count += 1;
        defmt::info!(
            "RING edge #{} -> level {} at {} ms",
            count,
            match ring.get_level() {
                Level::High => "high",
                Level::Low => "low",
            },
            Instant::now().as_millis()
        );
    }
}
