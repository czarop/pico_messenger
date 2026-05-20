use bno080::Error;
use bno080::interface::i2c_async::I2cInterfaceAsync;
use bno080::wrapper_async::{BNO080Async, WrapperError};
use embassy_rp::Peri;
use embassy_rp::gpio::Input;
use embassy_rp::peripherals::PIN_3;

use crate::sensors::heading::{Heading, HeadingReading};

pub struct Imu<I2C> {
    inner: BNO080Async<I2cInterfaceAsync<I2C>>,
    int_pin: Input<'static>, // is pulled low when the device has new data
}

impl<I2C, CommE> Imu<I2C>
where
    I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
    CommE: core::fmt::Debug,
{
    pub async fn new(i2c: I2C, int_pin: Peri<'static, PIN_3>) -> Self {
        let mut int_pin = Input::new(int_pin, embassy_rp::gpio::Pull::Up);
        let iface = I2cInterfaceAsync::default(i2c);
        let mut inner = BNO080Async::new_with_interface(iface);
        int_pin.wait_for_low().await;
        inner
            .init(&mut embassy_time::Delay)
            .await
            .expect("BNO085 init failed");
        Self { inner, int_pin }
    }

    pub async fn enable_rotation_vector(
        &mut self,
        millis: u16,
    ) -> Result<(), ImuError<CommE>> {
        self.inner.enable_rotation_vector(millis).await.map_err(ImuError::from)
    }

    pub async fn heading(&mut self) -> Result<HeadingReading, ImuError<CommE>> {
        self.int_pin.wait_for_low().await;
        self
            .inner
            .handle_all_messages(&mut embassy_time::Delay, 150)
            .await;
        let quat = self.inner.rotation_quaternion().map_err(ImuError::from)?;

        Ok(HeadingReading{
            heading: Heading::from_quaternion(quat),
            accuracy_deg: self.inner.heading_accuracy(),
        })
    }

    pub async fn wait_for_motion(&mut self) {
        
    }
}


#[derive(Debug, thiserror::Error)]
pub enum ImuError<E: core::fmt::Debug> {
    #[error("I2C communication error with IMU: {0:?}")]
    I2c(E),
    #[error("IMU sensor did not respond")]
    Unresponsive,
    #[error("Invalid IMU chip ID: {0}")]
    InvalidChipId(u8),
    #[error("Invalid IMU firmware version: {0}")]
    InvalidFirmware(u8),
    #[error("No IMU data available")]
    NoData,
}

impl<E: core::fmt::Debug> From<WrapperError<Error<E, ()>>> for ImuError<E> {
    fn from(e: WrapperError<Error<E, ()>>) -> Self {
        match e {
            WrapperError::CommError(Error::Comm(e)) => ImuError::I2c(e),
            WrapperError::InvalidChipId(id) => ImuError::InvalidChipId(id),
            WrapperError::InvalidFWVersion(v) => ImuError::InvalidFirmware(v),
            WrapperError::NoDataAvailable => ImuError::NoData,
            _ => ImuError::Unresponsive,
        }
    }
}