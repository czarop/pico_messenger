//! Feature-gated GNSS mock for indoor/desk testing.
//!
//! Enabled with `--features mock_gnss` (or `mock_modem`). Streams fabricated
//! `GNSSState::Fix` values into the same `GNSS_STATE` watch the real `gnss_task`
//! writes to, so `next_fix`, the `join` bring-up, `speed_mps`, payload encoding
//! and publish all exercise the real code path — only the data is synthetic.
//!
//! Honours `GnssCommand::Stop`/`Start` like the real task: on `Stop` it emits
//! `GNSSState::Off` and pauses, on `Start` it resumes. This keeps the
//! stop-before-sleep path in `modem_task` testable on the desk (otherwise the
//! mock would never emit `Off` and `stop_gnss` would hit its timeout every cycle).
//!
//! Motion model: travels due east at `MOCK_SPEED_MPS`, latitude fixed. The
//! per-period longitude step is sized so the pipeline's equirectangular distance
//! recovers `MOCK_SPEED_MPS` exactly. Set `MOCK_SPEED_MPS = 0.0` to test the
//! stationary path (`speed_mps()` returns `Some(0.0)`).

use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};

use crate::modem::communication;
use crate::modem::gnss::state::GNSSState;
use crate::modem::gnss::urc::fix::{GnssAccuracy, GnssLocation, GnssPosition};
use crate::modem::gnss_task::GnssCommand;

/// Simulated ground speed in m/s. 0.0 = stationary; ~13.4 ≈ 30 mph.
const MOCK_SPEED_MPS: f32 = 13.4;
/// Metres per degree of longitude at the fixed mock latitude (53.4761°N),
/// derived as `(PI/180) * cos(lat) * R` so the recovered speed is exact.
const METERS_PER_DEG_LON: f64 = 66_178.556;

const MOCK_LAT: f64 = 53.4761;
const MOCK_START_LON: f64 = -2.2950;
const MOCK_ALTITUDE_M: f32 = 78.0;

/// Drive the mock at the same burst period the orchestrator uses.
#[embassy_executor::task]
pub async fn gnss_mock_task(period_secs: u32) -> ! {
    let sender = communication::GNSS_STATE.sender();

    let week_number: u16 = 2360;
    let mut time_of_week: u32 = 100_000_000;
    let mut lon = MOCK_START_LON;

    info!(
        "GNSS MOCK task spawned: {} m/s due east, period {}s",
        MOCK_SPEED_MPS, period_secs
    );

    loop {
        // --- streaming phase: emit fixes every `period_secs` until told to stop ---
        sender.send(GNSSState::Ready);
        loop {
            let loc = GnssLocation {
                position: GnssPosition {
                    week_number,
                    time_of_week,
                    latitude: MOCK_LAT,
                    longitude: lon,
                    altitude: MOCK_ALTITUDE_M,
                    accuracy: 3.0,
                },
                accuracy: GnssAccuracy {
                    std_dev_altitude: 2.0,
                    hdop: 1.0,
                    gdop: 2.0,
                    pdop: 1.7, // vdop derives to sqrt(1.7^2 - 1.0^2) ≈ 1.37
                },
            };
            sender.send(GNSSState::Fix(loc));

            // Advance one period of travel: due east, latitude unchanged.
            let step_m = MOCK_SPEED_MPS as f64 * period_secs as f64;
            lon += step_m / METERS_PER_DEG_LON;
            time_of_week = time_of_week.wrapping_add(period_secs * 1000);

            // Wait one period, but honour a Stop command if it arrives first.
            match select(
                Timer::after(Duration::from_secs(period_secs as u64)),
                communication::GNSS_COMMAND.wait(),
            )
            .await
            {
                Either::First(_) => {} // period elapsed -> next fix
                Either::Second(GnssCommand::Stop) => {
                    sender.send(GNSSState::Off);
                    info!("MOCK gnss stopped");
                    break; // leave streaming phase
                }
                Either::Second(_) => {} // Start/SetFrequency while running: ignore
            }
        }

        // --- stopped phase: stay Off until a Start command resumes streaming ---
        loop {
            if let GnssCommand::Start(..) = communication::GNSS_COMMAND.wait().await {
                info!("MOCK gnss restart");
                break;
            }
        }
    }
}
