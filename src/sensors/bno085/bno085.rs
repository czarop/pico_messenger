use super::bno08x::Error;
use super::bno08x::interface::i2c_async::I2cInterfaceAsync;
use super::bno08x::wrapper_async::{BNO085Async, WrapperError};
use defmt::info;
use embassy_rp::Peri;
use embassy_rp::gpio::{Input, Output};
use embassy_rp::peripherals::{PIN_2, PIN_3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Receiver, Sender};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::i2c::{Async, I2c};
use embassy_rp::peripherals::I2C1;
use embassy_sync::signal::Signal;
use embassy_time::Delay;

use crate::sensors::heading::{Heading, HeadingReading};

type ImuI2C = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, I2C1, Async>>;

pub type ImuDevice = Imu<ImuI2C, Input<'static>>;

pub struct Imu<I2C, HINT> {
    inner: BNO085Async<I2cInterfaceAsync<I2C, HINT>>,
    // int_pin: Input<'static>, // is pulled low when the device has new data
}

impl<I2C, CommE> Imu<I2C, Input<'static>>
where
    I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
    CommE: core::fmt::Debug,
{
    pub async fn new(
        i2c: I2C,
        int_pin: Peri<'static, PIN_3>,
        rst_pin: Peri<'static, PIN_2>,
    ) -> Self {
        let hint = Input::new(int_pin, embassy_rp::gpio::Pull::Up);
        let mut rst_pin = Output::new(rst_pin, embassy_rp::gpio::Level::High);
        let iface = I2cInterfaceAsync::default(i2c, hint);
        let mut inner = BNO085Async::new_with_interface(iface);

        rst_pin.set_low();
        embassy_time::Timer::after_millis(10).await;
        rst_pin.set_high();

        inner.sensor_interface.wait_for_hint().await.ok();

        inner
            .init(&mut embassy_time::Delay)
            .await
            .expect("BNO085 init failed");
        info!("BNO085 init completed");
        Self { inner }
    }

    pub async fn wait_for_motion_dormant(&mut self) {
        self.inner.enable_significant_motion_wake().await.expect("failed to enable significant motion wake");
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        self.inner.handle_all_messages(&mut Delay, 10).await;
        self.inner.sensor_interface.dormant_sleep_on_hint();
        self.inner.wake().await.expect("failed to wake BNO085");
    }

    // for testing - keeps clocks running so connected to probe
    pub async fn wait_for_motion(&mut self) {
        self.inner.enable_significant_motion_wake().await.expect("failed to enable significant motion wake");
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        self.inner.handle_all_messages(&mut Delay, 10).await;
        self.inner.sensor_interface.wait_for_hint().await.ok();
        self.inner.wake().await.expect("failed to wake BNO085");

    }
}

impl<I2C, HINT, CommE> Imu<I2C, HINT>
where
    I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
    CommE: core::fmt::Debug,
    HINT: embedded_hal::digital::InputPin + embedded_hal_async::digital::Wait,
{

    pub async fn enable_rotation_vector(&mut self, millis: u16) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_rotation_vector(millis)
            .await
            .map_err(ImuError::from)
    }

    pub fn heading(&mut self) -> Result<HeadingReading, ImuError<CommE>> {
        
        let quat = self.inner.rotation_quaternion().map_err(ImuError::from)?;

        Ok(HeadingReading {
            heading: Heading::from_quaternion(quat),
            accuracy_deg: self.inner.heading_accuracy(),
        })
    }

    pub async fn enable_activity_recognition(&mut self) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_activity_classifier(60000)
            .await
            .map_err(ImuError::from)
    }

    pub fn get_activity_type(self) -> super::bno08x::activity::Activity {
        self.inner.activity()
    }

    pub async fn enable_significant_motion_wake(&mut self) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_significant_motion_wake()
            .await
            .map_err(ImuError::from)
    }

    pub async fn enable_stability_detection_wake(&mut self) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_stability_detector_wake()
            .await
            .map_err(ImuError::from)
    }

    pub async fn enable_shake_detection_wake(&mut self) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_shake_detector_wake()
            .await
            .map_err(ImuError::from)
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

pub enum ImuCommand {
    EnableRotationVector,
    EnableStepCounter,
    WaitForMotion,
    GetHeading,
    GetStepCount,
}

pub enum ImuReport {
    Heading(HeadingReading),
    StepCount(u16),
    MotionDetected,
    Activity(super::bno08x::activity::Activity),
    Error,
}

pub static ENTER_SLEEP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
pub async fn imu_task(
    mut imu: ImuDevice,
    sender: Sender<'static, CriticalSectionRawMutex, ImuReport, 4>,
) {
    loop {

        // race HINT against sleep signal
        match embassy_futures::select::select(
            imu.inner.sensor_interface.wait_for_hint(),
            ENTER_SLEEP.wait(),
        ).await {
            embassy_futures::select::Either::First(_) => {
                // normal data flow
                imu.inner.handle_one_message(&mut Delay, 10).await;

                if let Ok(h) = imu.inner.rotation_quaternion() {
                    let _ = sender.try_send(ImuReport::Heading(HeadingReading {
                        heading: Heading::from_quaternion(h),
                        accuracy_deg: imu.inner.heading_accuracy(),
                    }));
                }
                let _ = sender.try_send(ImuReport::Activity(imu.inner.activity()));
                let _ = sender.try_send(ImuReport::StepCount(imu.inner.step_count()));
            }
            embassy_futures::select::Either::Second(_) => {
                // enter low power — whole board sleeps until motion
                
                imu.wait_for_motion().await;
                sender.send(ImuReport::MotionDetected).await;
            }
        }
    }
}

#[embassy_executor::task]
pub async fn ui_task(reports: Receiver<'static, CriticalSectionRawMutex, ImuReport, 4>) {

    loop {
        // blocks until a report arrives
        let report = reports.receive().await;
        match report {
            ImuReport::Heading(h) => {
                info!("Heading: {}", h.heading)
            }
            ImuReport::Activity(a) => {
                info!("Activity: {:?}", a)
            }
            ImuReport::MotionDetected => {
                info!("Motion detected")
            }
            _ => {}
        }
    }
}
