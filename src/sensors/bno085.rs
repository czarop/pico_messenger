use bno080::Error;
use bno080::interface::i2c_async::I2cInterfaceAsync;
use bno080::wrapper_async::{BNO085Async, WrapperError};
use defmt::info;
use embassy_rp::Peri;
use embassy_rp::gpio::Input;
use embassy_rp::peripherals::PIN_3;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Receiver, Sender};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::i2c::{I2c, Async};
use embassy_rp::peripherals::I2C1;
use embassy_time::Delay;

use crate::sensors::heading::{Heading, HeadingReading};

type ImuI2C = I2cDevice<
    'static,
    CriticalSectionRawMutex,
    I2c<'static, I2C1, Async>,
>;

pub struct Imu<I2C> {
    inner: BNO085Async<I2cInterfaceAsync<I2C>>,
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
        let mut inner = BNO085Async::new_with_interface(iface);
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

    pub async fn wait_for_motion_dormant(&mut self) {
        // enable significant motion as wake sensor
        self.inner.enable_significant_motion_wake().await
            .expect("failed to enable significant motion");
        
        // put BNO085 to sleep - significant motion will keep running
        self.inner.sleep().await
            .expect("failed to sleep BNO085");

        // dormant sleep RP2350 - wake on H_INTN going low
        let dormant = self.int_pin.dormant_wake(embassy_rp::gpio::DormantWakeConfig {
            edge_high: false,
            edge_low: true,
            level_high: false,
            level_low: false,
        });
        embassy_rp::clocks::dormant_sleep();
        drop(dormant);

        // wake BNO085 back up
        self.inner.wake().await
            .expect("failed to wake BNO085");
        
        // eat the significant motion report
        self.inner.eat_all_messages(&mut embassy_time::Delay).await;
    }

    // for testing - keeps clocks running so connected to probe
    pub async fn wait_for_motion(&mut self) {
        // enable significant motion as wake sensor
        self.inner.enable_significant_motion_wake().await
            .expect("failed to enable significant motion");
        
        // put BNO085 to sleep - significant motion will keep running
        self.inner.sleep().await
            .expect("failed to sleep BNO085");

        // wait for H_INTN to go low - clocks keep running
        self.int_pin.wait_for_low().await;

        // wake BNO085 back up
        self.inner.wake().await
            .expect("failed to wake BNO085");
        
        // eat the significant motion report
        self.inner.eat_all_messages(&mut embassy_time::Delay).await;
    }

    pub async fn enable_activity_recognition(&mut self) -> Result<(), ImuError<CommE>> {
        self.inner.enable_activity_classifier(60000).await.map_err(ImuError::from)
    }

    pub fn get_activity_type(self) -> bno080::activity::Activity {
        self.inner.activity()
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
    Activity(bno080::activity::Activity),
    Error,
}

#[embassy_executor::task]
pub async fn imu_task(
    mut imu: Imu<ImuI2C>,
    receiver: Receiver<'static, CriticalSectionRawMutex, ImuCommand, 4>,
    sender: Sender<'static, CriticalSectionRawMutex, ImuReport, 4>,
) {
    loop {
        // check for commands without blocking
        if let Ok(cmd) = receiver.try_receive() {
            match cmd {
                ImuCommand::WaitForMotion => {
                    imu.wait_for_motion().await;
                    sender.send(ImuReport::MotionDetected).await;
                }
                ImuCommand::GetHeading => {
                    match imu.heading().await {
                        Ok(h) => sender.send(ImuReport::Heading(h)).await,
                        Err(_) => sender.send(ImuReport::Error).await,
                    }
                }
                _ => {}
            }
        }

        // proactively push updates on every H_INTN pulse
        imu.int_pin.wait_for_low().await;
        imu.inner.handle_all_messages(&mut Delay, 150).await;

        // push whatever has changed
        if let Ok(h) = imu.inner.rotation_quaternion() {
            let _ = sender.try_send(ImuReport::Heading(
                HeadingReading {
                    heading: Heading::from_quaternion(h),
                    accuracy_deg: imu.inner.heading_accuracy(),
                }
            ));
        }
        
        let activity = imu.inner.activity();
        let _ = sender.try_send(ImuReport::Activity(activity));
        let _ = sender.try_send(ImuReport::StepCount(imu.inner.step_count()));
    }
}

#[embassy_executor::task]
pub async fn ui_task(
    reports: Receiver<'static, CriticalSectionRawMutex, ImuReport, 4>,
) {
    loop {
        // blocks until a report arrives
        let report = reports.receive().await;
        match report {
            ImuReport::Heading(h) => { info!("Heading: {}", h.heading) }
            ImuReport::Activity(a) => { info!("Activity: {:?}", a) }
            ImuReport::MotionDetected => { info!("Motion detected") }
            _ => {}
        }
    }
}