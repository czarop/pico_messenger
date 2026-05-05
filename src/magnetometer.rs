use embedded_hal_async::i2c::I2c;
use thiserror::Error;

const HDC302X_ADDR: u8 = 0x44;

pub struct Magnetometer<I> {
    i2c: I,
    recv_buffer: [u8; 6],
}

impl<I> Magnetometer<I>
where
    I: I2c,
{
    pub fn new(i2c: I) -> Self {
        Self {
            i2c,
            recv_buffer: [0u8; 6],
        }
    }

    pub async fn read_direction(
        &mut self,
        power_mode: TempSensorPowerMode,
    ) -> Result<TempSensorReading, TempSensorError<I::Error>> {
        self.i2c.write(HDC302X_ADDR, &power_mode.command()).await?;
        embassy_time::Timer::after(embassy_time::Duration::from_millis(power_mode.delay_ms()))
            .await;
        self.i2c.read(HDC302X_ADDR, &mut self.recv_buffer).await?;

        let raw_value = RawReading::from(self.recv_buffer);

        Ok(raw_value.into())
    }
}

#[derive(Debug, Error)]
pub enum TempSensorError<E: core::fmt::Debug> {
    #[error("I2C error: {0:?}")]
    I2cError(E),
    #[error("Invalid sensor data: {0}")]
    InvalidData(&'static str),
}

impl<E: core::fmt::Debug> From<E> for TempSensorError<E> {
    fn from(e: E) -> Self {
        TempSensorError::I2cError(e)
    }
}

pub enum TempSensorPowerMode {
    LPM0,
    LPM1,
    LPM2,
    LPM3,
}

impl TempSensorPowerMode {
    pub fn command(&self) -> [u8; 2] {
        match self {
            TempSensorPowerMode::LPM0 => [0x24, 0x00],
            TempSensorPowerMode::LPM1 => [0x24, 0x0B],
            TempSensorPowerMode::LPM2 => [0x24, 0x16],
            TempSensorPowerMode::LPM3 => [0x24, 0xFF],
        }
    }

    pub fn delay_ms(&self) -> u64 {
        match self {
            TempSensorPowerMode::LPM0 => 2,
            TempSensorPowerMode::LPM1 => 3,
            TempSensorPowerMode::LPM2 => 5,
            TempSensorPowerMode::LPM3 => 9,
        }
    }
}

#[allow(dead_code)]
struct RawReading {
    temp_msb: u8,
    temp_lsb: u8,
    temp_crc: u8,
    hum_msb: u8,
    hum_lsb: u8,
    hum_crc: u8,
}

impl From<[u8; 6]> for RawReading {
    fn from(buf: [u8; 6]) -> Self {
        Self {
            temp_msb: buf[0],
            temp_lsb: buf[1],
            temp_crc: buf[2],
            hum_msb: buf[3],
            hum_lsb: buf[4],
            hum_crc: buf[5],
        }
    }
}

pub struct TempSensorReading {
    pub temperature: f32,
    pub humidity: f32,
}

impl From<RawReading> for TempSensorReading {
    fn from(raw: RawReading) -> Self {
        let temp_raw = ((raw.temp_msb as u16) << 8) | (raw.temp_lsb as u16);
        let hum_raw = ((raw.hum_msb as u16) << 8) | (raw.hum_lsb as u16);
        Self {
            temperature: (temp_raw as f32 / 65535.0) * 175.0 - 45.0,
            humidity: (hum_raw as f32 / 65535.0) * 100.0,
        }
    }
}
