//! Battery and temperature telemetry for the MQTT payload.
//!
//! `modem_task` needs these values at publish time but owns neither sensor, and
//! must never block waiting for an I2C read. So a task owns the sensors, samples
//! them periodically, and publishes into shared slots that `modem_task` reads
//! with no coordination. Same pattern as the BNO085 heading slot.
//!
//! Sampling only happens while the host is awake: DORMANT halts this task along
//! with everything else, so there is no cost during a sleep and the readings are
//! at most `SAMPLE_INTERVAL` old when a cycle publishes.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::{Duration, Timer};

use crate::sensors::battery_meter::Max17048;
use crate::sensors::temp_sensor::{TempSensor, TempSensorPowerMode};

/// How often the sensors are read while the host is awake. A cycle is awake for
/// tens of seconds, so this samples a handful of times per cycle -- frequent
/// enough to be current at publish, rare enough to be negligible on the I2C bus.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Sentinel for "no reading".
const NONE: u32 = u32::MAX;

/// Battery: `soc << 16 | charging`. Sentinel [`NONE`] when unread.
static BATTERY: AtomicU32 = AtomicU32::new(NONE);

/// Temperature in units of 0.1 degC, stored as `i16 as u16 as u32`, sentinel
/// [`NONE`] when unread. Goes through u16 so negatives round-trip.
static TEMPERATURE: AtomicU32 = AtomicU32::new(NONE);

/// Battery state of charge (percent) and whether it is charging, or `None` if
/// the meter has not been read yet or the last read failed.
pub fn battery() -> Option<(u8, bool)> {
    let v = BATTERY.load(Ordering::Relaxed);
    if v == NONE {
        return None;
    }
    Some((((v >> 16) & 0xFF) as u8, (v & 1) != 0))
}

/// Temperature in units of 0.1 degC, or `None` if unread / last read failed.
pub fn temperature_c10() -> Option<i16> {
    let v = TEMPERATURE.load(Ordering::Relaxed);
    if v == NONE {
        return None;
    }
    Some((v & 0xFFFF) as u16 as i16)
}

/// Own the battery meter and temperature sensor, sampling both into the shared
/// slots above.
///
/// A failed read clears its slot rather than leaving a stale value: publishing a
/// stale battery level as if it were current is worse than publishing none, and
/// the payload has a validity flag for exactly this case.
#[embassy_executor::task]
pub async fn telemetry_task(
    mut battery_meter: Max17048<crate::startup::TelemetryI2c>,
    mut temp_sensor: TempSensor<crate::startup::TelemetryI2c>,
) {
    loop {
        match battery_meter.soc().await {
            Ok(soc) => {
                // The meter reports a u16 percentage; clamp so a nonsense reading
                // cannot wrap into a plausible-looking value.
                let soc = soc.min(100) as u32;
                let charging = match battery_meter.charge_rate().await {
                    Ok(rate) => rate > 0.0,
                    Err(_) => false,
                };
                BATTERY.store((soc << 16) | charging as u32, Ordering::Relaxed);
            }
            Err(_) => {
                // defmt::warn!("battery read failed");
                BATTERY.store(NONE, Ordering::Relaxed);
            }
        }

        match temp_sensor
            .read_temperature(TempSensorPowerMode::LPM3)
            .await
        {
            Ok(r) => {
                let c10 = (r.temperature * 10.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                TEMPERATURE.store(c10 as u16 as u32, Ordering::Relaxed);
            }
            Err(_) => {
                defmt::warn!("temperature read failed");
                TEMPERATURE.store(NONE, Ordering::Relaxed);
            }
        }

        Timer::after(SAMPLE_INTERVAL).await;
    }
}
