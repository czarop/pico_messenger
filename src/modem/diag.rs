//! Self-reported device health, published over MQTT so the device can be its
//! own logger.
//!
//! The debug probe drops SWD during the modem's connect-current surge, so at a
//! 15-minute production cadence a probe capture of a rare failure is
//! impractical. Instead the device keeps a handful of cumulative counters in
//! RAM and appends them to a `pico/diag` publish on every SUCCESSFUL cycle.
//!
//! The trick that makes this work with no probe: a cycle that fails to publish
//! is reported by the NEXT cycle that succeeds -- the counters are cumulative,
//! so `skip` rising from one good report to the next means a cycle was missed
//! in between. `skip / cyc` is the miss rate, which is the single number that
//! decides whether the tracker is reliable at production cadence.
//!
//! Counters are since-boot (RAM). A modem/host reset zeroes them, which is
//! itself visible in the Telegram stream as the totals dropping back down --
//! useful, not a bug. Lifetime persistence (PCF8523 battery RAM) is a later
//! addition if wanted.
//!
//! All counters are `Relaxed` atomics: they are statistics, not
//! synchronisation, and exact ordering between them does not matter.

use core::sync::atomic::{AtomicU32, Ordering};

/// Cycles begun (denominator for the miss rate).
static CYCLES: AtomicU32 = AtomicU32::new(0);
/// Cycles that gave up without publishing a location -- the headline number.
static SKIPPED: AtomicU32 = AtomicU32::new(0);
/// MQTT stack reached bring-up but CONNECT failed.
static CONNECT_FAILURES: AtomicU32 = AtomicU32::new(0);
/// AT#RESET=0 escalations (wedged socket / stuck searching / connect retries).
static RESETS: AtomicU32 = AtomicU32::new(0);
/// SLEEPMODE returned OK but no #SLEEP URC followed (PSM entry declined).
static PSM_SKIPS: AtomicU32 = AtomicU32::new(0);
/// Location resends after a teardown reset (duplicate sent because the first
/// publish's delivery was unverifiable). High `rsnd` relative to `cyc` means
/// disconnects are frequently wedging and we're leaning on the resend path.
static RESENDS: AtomicU32 = AtomicU32::new(0);

pub fn note_cycle_start() {
    CYCLES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_skipped() {
    SKIPPED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_connect_failure() {
    CONNECT_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_reset() {
    RESETS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_psm_skip() {
    PSM_SKIPS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_resend() {
    RESENDS.fetch_add(1, Ordering::Relaxed);
}

/// Build the `pico/diag` payload: plain CSV, human-readable in Telegram.
///
/// `rsrp_dbm` is this cycle's CESQ reading; `None` (no measurable signal, CESQ
/// index 255) renders as `na`, which is exactly the state that correlates with
/// skips, so it is worth surfacing rather than hiding.
///
/// Example: `cyc=42 skip=3 cfail=1 rst=0 psm=2 rsrp=-104`
/// Fits well inside the 50-char MQTT message field.
pub fn payload(rsrp_dbm: Option<i16>) -> heapless::String<50> {
    use core::fmt::Write;
    let mut s = heapless::String::new();
    let cyc = CYCLES.load(Ordering::Relaxed);
    let skip = SKIPPED.load(Ordering::Relaxed);
    let cfail = CONNECT_FAILURES.load(Ordering::Relaxed);
    let rst = RESETS.load(Ordering::Relaxed);
    let psm = PSM_SKIPS.load(Ordering::Relaxed);
    let rsnd = RESENDS.load(Ordering::Relaxed);
    // Compact field names: the MQTT message field is capped at 50 chars, and
    // the verbose form overflowed once counts reached 3+ digits. Legend:
    // c=cycles s=skipped f=connect-fails r=resets p=psm-skips d=resends
    // q=signal (RSRP dBm, or na when unmeasurable).
    match rsrp_dbm {
        Some(dbm) => {
            let _ = write!(s, "c={} s={} f={} r={} p={} d={} q={}", cyc, skip, cfail, rst, psm, rsnd, dbm);
        }
        None => {
            let _ = write!(s, "c={} s={} f={} r={} p={} d={} q=na", cyc, skip, cfail, rst, psm, rsnd);
        }
    }
    s
}
