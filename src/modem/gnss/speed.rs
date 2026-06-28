//! Speed-over-ground from two consecutive GNSS fixes.
//!
//! The device wakes on its ~15-minute cadence and takes a short burst of GNSS
//! fixes spaced by the `#GNSSFIX` `<period>` (recommended 3 s — set that period
//! explicitly on the GNSS `Start`, NOT the publish interval). Speed is the
//! ground distance between two consecutive fixes divided by the GPS-time gap
//! between them. Heading is sourced separately from the BNO085, so no bearing
//! maths live here.
//!
//! Δt is taken from the fixes' own `week_number`/`time_of_week` (not wall-clock),
//! making it immune to scheduler jitter and self-describing. Distance uses the
//! equirectangular (flat-earth) approximation, which is exact to <0.001 % over a
//! few-second baseline and cheaper than haversine.

use micromath::F32Ext;

use crate::modem::gnss::urc::fix::{GnssLocation, GnssPosition};

/// Mean Earth radius, metres.
const EARTH_RADIUS_M: f32 = 6_371_000.0;
/// Degrees -> radians (core has no no_std `f32::to_radians`).
const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;
/// Milliseconds per GPS week (for rollover-safe Δt).
const MS_PER_WEEK: i64 = 604_800_000;

/// Reject baselines shorter than this: a sub-second gap is a duplicate/glitch
/// URC, and dividing real jitter by a tiny Δt yields phantom speed.
const MIN_BASELINE_MS: i64 = 1_000;
/// Reject baselines longer than this: with a ~3 s nominal period, anything past
/// 15 s means a stale fix (e.g. one leaked from a previous wake) got paired in.
/// Must sit well above a few fix periods and well below the publish interval.
const MAX_BASELINE_MS: i64 = 15_000;
/// Horizontal displacement below this is treated as GNSS noise, not motion, and
/// reported as a confirmed-stationary 0 m/s rather than phantom drift.
///
/// NOTE — coupled to the fix baseline. At a 3 s baseline, walking (~1.4 m/s →
/// ~4.2 m) sits only just above this floor, so slow pedestrian motion is
/// marginal and may read as stationary; vehicle speeds are robust. Lengthen the
/// baseline if pedestrian detection matters. Field-tunable.
const JITTER_FLOOR_M: f32 = 4.0;

/// A pair of consecutive GNSS fixes: `start` then `end`.
pub struct FixPair {
    pub start: GnssLocation,
    pub end: GnssLocation,
}

impl FixPair {
    /// Speed over ground in m/s.
    ///
    /// * `None` — the pair is unusable (Δt non-positive, too short, or too long);
    ///   the caller should leave `FLAG_SPEED_VALID` clear.
    /// * `Some(0.0)` — a *valid* measurement of stationarity (displacement below
    ///   the jitter floor). This is data, distinct from `None`.
    /// * `Some(v)` — measured ground speed.
    pub fn speed_mps(&self) -> Option<f32> {
        let dt_ms = gps_delta_ms(&self.start.position, &self.end.position);
        if dt_ms < MIN_BASELINE_MS || dt_ms > MAX_BASELINE_MS {
            return None;
        }

        let dist = equirectangular_m(&self.start.position, &self.end.position);
        if dist < JITTER_FLOOR_M {
            return Some(0.0);
        }

        Some(dist / (dt_ms as f32 / 1000.0))
    }
}

/// GPS-time gap end-minus-start, in milliseconds, rollover-safe across the
/// week boundary. Negative if `end` precedes `start`.
fn gps_delta_ms(start: &GnssPosition, end: &GnssPosition) -> i64 {
    let s = start.time_of_week as i64 + start.week_number as i64 * MS_PER_WEEK;
    let e = end.time_of_week as i64 + end.week_number as i64 * MS_PER_WEEK;
    e - s
}

/// Ground distance between two positions, metres, via the equirectangular
/// approximation. Deltas are formed in f64 (they are tiny) before the cast to
/// f32, preserving precision.
pub fn equirectangular_m(a: &GnssPosition, b: &GnssPosition) -> f32 {
    let dlat = ((b.latitude - a.latitude) as f32) * DEG_TO_RAD;
    let dlon = ((b.longitude - a.longitude) as f32) * DEG_TO_RAD;
    let mean_lat = (((a.latitude + b.latitude) * 0.5) as f32) * DEG_TO_RAD;

    let x = dlon * mean_lat.cos();
    let y = dlat;
    (x * x + y * y).sqrt() * EARTH_RADIUS_M
}
