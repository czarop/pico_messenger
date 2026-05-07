use defmt::{error, info};
use embassy_embedded_hal::shared_bus::I2cDeviceError;
use embassy_rp::Peri;
use embassy_rp::clocks::dormant_sleep;
use embassy_rp::gpio::{DormantWakeConfig, Input};
use embassy_rp::i2c::I2c;
use embassy_rp::peripherals::{I2C0, PIN_15};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use lis2dw12_i2c::Register;
use micromath::F32Ext;
use thiserror::Error;

type AccelerometerI2C = lis2dw12_i2c::Lis2dw12<
    embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    >,
    embassy_time::Delay,
>;

pub struct Accelerometer {
    inner: AccelerometerI2C,
    int_pin: Input<'static>,
}

impl Accelerometer {
    pub async fn new(
        int_pin: Peri<'static, PIN_15>,
        i2c_driver: embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, I2C0, embassy_rp::i2c::Async>,
        >,
    ) -> Self {
        let delay = embassy_time::Delay;
        let mut accel: lis2dw12_i2c::Lis2dw12<
            embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
                '_,
                CriticalSectionRawMutex,
                I2c<'static, I2C0, embassy_rp::i2c::Async>,
            >,
            embassy_time::Delay,
        > = lis2dw12_i2c::Lis2dw12::new_with_sa0_gnd(i2c_driver, delay);

        match accel
            .write_reg(
                lis2dw12_i2c::Register::Control1,
                lis2dw12_i2c::registers::ControlReg1::new(
                    lis2dw12_i2c::Control1LowPowerMode::LowPower2,
                    lis2dw12_i2c::Control1ModeSelect::LowPower,
                    lis2dw12_i2c::Control1DataRate::Hi12p5Lo1p6Hz,
                )
                .into(),
            )
            .await
        {
            Ok(_) => info!("Accelerometer initialized successfully!"),
            Err(e) => error!("Failed to initialize accelerometer: {:?}", e),
        };

        Self {
            inner: accel,
            int_pin: Input::new(int_pin, embassy_rp::gpio::Pull::Down),
        }
    }

    pub async fn configure_wake_on_movement(&mut self, threshold: u8, duration: u8) {
        self.inner
            .write_reg(Register::WakeUpThreshold, threshold)
            .await
            .expect("failed to set wake-up threshold");
        self.inner
            .write_reg(Register::WakeUpDuration, duration)
            .await
            .expect("failed to set wake-up duration");
        self.inner
            .write_reg(Register::Control4Interrupt1, 0x20) // route to INT1
            .await
            .expect("failed to route interrupt");
        self.inner
            .write_reg(Register::Control7, 0x20) // enable interrupts
            .await
            .expect("failed to enable interrupts");
    }

    pub async fn was_woken_by_motion(&mut self) -> bool {
        match self.inner.read_reg(Register::WakeUpSource).await {
            Ok(val) => val & 0x08 != 0, // WU_IA bit
            Err(_) => false,
        }
    }

    pub async fn clear_wake_source(&mut self) {
        // reading the register clears the latch
        let _ = self.inner.read_reg(Register::WakeUpSource).await;
    }

    pub async fn wait_for_motion(&mut self) {
        // testing
        self.int_pin.wait_for_rising_edge().await;
    }

    pub async fn wait_for_motion_dormant(&mut self) {
        let dormant = self.int_pin.dormant_wake(DormantWakeConfig {
            edge_high: true,
            edge_low: false,
            level_high: false,
            level_low: false,
        });
        dormant_sleep(); // halts all clocks - µA level power draw
        drop(dormant); // re-enables clocks, restores GPIO state
    }

    pub async fn acceleration_reading(
        &mut self,
    ) -> Result<AccelerometerReading, AccelerometerError<I2cDeviceError<embassy_rp::i2c::Error>>>
    {
        let res = self.inner.acc_gs().await?.into();

        Ok(res)
    }

    pub async fn read_orientation(&mut self) -> Result<Orientation, AccelerometerError<I2cDeviceError<embassy_rp::i2c::Error>>> {
        let val = self.inner.read_reg(Register::SixDimSource).await?;
        Ok(Orientation::from_reg(val))
    }

    
}

pub struct AccelerometerReading {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl AccelerometerReading {
    pub fn pitch(&self) -> f32 {
        f32::atan2(-self.x, f32::sqrt(self.y * self.y + self.z * self.z))
    }

    pub fn roll(&self) -> f32 {
        f32::atan2(self.y, self.z)
    }
}

impl From<(f32, f32, f32)> for AccelerometerReading {
    fn from(value: (f32, f32, f32)) -> Self {
        AccelerometerReading {
            x: value.0,
            y: value.1,
            z: value.2,
        }
    }
}

#[derive(Debug, Error)]
pub enum AccelerometerError<E: core::fmt::Debug> {
    #[error("I2C error: {0:?}")]
    I2cError(E),
    #[error("Invalid sensor data: {0}")]
    InvalidData(&'static str),
    #[error("Data not ready")]
    StaleData,
}

impl<E: core::fmt::Debug> From<E> for AccelerometerError<E> {
    fn from(e: E) -> Self {
        AccelerometerError::I2cError(e)
    }
}



#[derive(Debug, defmt::Format)]
pub enum Orientation {
    XDown,
    XUp,
    YDown,
    YUp,
    ZDown,
    ZUp,
    Unknown,
}

impl Orientation {
    pub fn from_reg(val: u8) -> Self {
        match val & 0x3F {
            v if v & 0x01 != 0 => Orientation::XDown,
            v if v & 0x02 != 0 => Orientation::XUp,
            v if v & 0x04 != 0 => Orientation::YDown,
            v if v & 0x08 != 0 => Orientation::YUp,
            v if v & 0x10 != 0 => Orientation::ZDown,
            v if v & 0x20 != 0 => Orientation::ZUp,
            _ => Orientation::Unknown,
        }
    }
}
