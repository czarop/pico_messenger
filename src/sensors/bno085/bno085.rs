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

use crate::sensors::bno085::reports::ImuReport;
use crate::sensors::heading::{Heading, HeadingReading};

type ImuI2C = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, I2C1, Async>>;

pub type ImuDevice = Imu<ImuI2C, Input<'static>>;

pub struct Imu<I2C, HINT> {
    inner: BNO085Async<I2cInterfaceAsync<I2C, HINT>>,
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

        inner.wait_for_hint().await;

        inner
            .init(&mut embassy_time::Delay)
            .await
            .expect("BNO085 init failed");
        info!("BNO085 init completed");
        Self { inner }
    }

    /// Sleep the BNO085 and DORMANT the whole board until significant motion.
    ///
    /// Do NOT add Set Feature traffic to this sequence. Executable SLEEP
    /// preserves the hub's sensor configuration and executable ON restores it,
    /// so reports resume on their own. Sending a Set Feature around the
    /// sleep/ON window zeroes the stored report interval and the stream never
    /// comes back.
    pub async fn wait_for_motion_dormant(&mut self) {
        self.inner
            .enable_significant_motion_wake()
            .await
            .expect("failed to enable significant motion wake");
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        self.inner.handle_all_messages(&mut Delay, 10).await;
        self.inner.dormant_sleep_on_hint();
        self.inner.wake().await.expect("failed to wake BNO085");
    }

    // for testing - keeps clocks running so connected to probe
    /// Sleep the sensor with significant-motion wake armed, then sleep the host
    /// until EITHER the RTC INT fires or the BNO085 reports motion. Returns which
    /// one woke us.
    ///
    /// The caller must have armed the RTC countdown first. Both pads are armed as
    /// dormant wake sources across a single `dormant_sleep()`, which is why this
    /// lives here: `imu_task` owns HINT, and borrows the RTC pad via
    /// `power::with_rtc_dormant_wake`.
    ///
    /// Same rule as the other sleeps — no Set Feature traffic in here.
    pub async fn wait_for_motion_or_rtc(&mut self) -> WakeSource {
        self.inner
            .enable_significant_motion_wake()
            .await
            .expect("failed to enable significant motion wake");
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        self.inner.handle_all_messages(&mut Delay, 10).await;

        #[cfg(feature = "mock_host_sleep")]
        let woke_on_motion = {
            // Clocks stay up: race the two lines instead of dormanting.
            match embassy_futures::select::select(
                self.inner.wait_for_hint(),
                crate::power::wait_rtc_int(),
            )
            .await
            {
                embassy_futures::select::Either::First(_) => true,
                embassy_futures::select::Either::Second(_) => false,
            }
        };

        #[cfg(not(feature = "mock_host_sleep"))]
        let woke_on_motion = {
            crate::power::with_rtc_dormant_wake(|| self.inner.dormant_sleep_on_hint()).await;
            // Discriminate after the fact: HINT is level, active-low, and stays
            // asserted until the pending report is read. If it is still low the
            // BNO085 has something for us, i.e. motion woke us; otherwise it was
            // the RTC.
            self.inner.hint_low()
        };

        self.inner.wake().await.expect("failed to wake BNO085");

        if woke_on_motion {
            WakeSource::Motion
        } else {
            WakeSource::Rtc
        }
    }

    /// Put ONLY the BNO085 into its low-power state; the host stays awake and
    /// keeps its own wake source (the RTC). Used on the moving/RTC-cadence path,
    /// where the sensor would otherwise keep running its fusion engine at full
    /// rate for the whole interval while the host is dormant — which dominates
    /// the power budget.
    ///
    /// Significant-motion wake is deliberately NOT armed here: nothing is
    /// listening on HINT in this mode, the RTC ends the sleep.
    ///
    /// Same rule as the motion sleeps — no Set Feature traffic. Executable SLEEP
    /// preserves the sensor configuration and [`Self::wake_sensor`] restores it.
    pub async fn sleep_sensor(&mut self) {
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        self.inner.handle_all_messages(&mut Delay, 10).await;
    }

    /// Bring the BNO085 back from [`Self::sleep_sensor`]. Executable ON restores
    /// the previously configured reports on its own.
    pub async fn wake_sensor(&mut self) {
        self.inner.wake().await.expect("failed to wake BNO085");
    }

    /// Test variant of [`Self::wait_for_motion_dormant`]: waits on the HINT
    /// edge instead of DORMANTing, so clocks stay up and the probe stays
    /// attached. Same rule — no Set Feature traffic in here.
    pub async fn wait_for_motion(&mut self) {
        self.inner
            .enable_significant_motion_wake()
            .await
            .expect("failed to enable significant motion wake");
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        self.inner.handle_all_messages(&mut Delay, 10).await;
        self.inner.wait_for_hint().await;
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

    /// Heading from the most recently PARSED rotation vector report.
    ///
    /// Cached, not an on-demand read: SH-2 has no "sample now" command, the hub
    /// pushes reports. Only as fresh as the last `handle_one_message`, so this
    /// is a convenience accessor, not a replacement for the report stream.
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


    pub async fn enable_stability_detection_wake(
        &mut self,
        report_interval_micros: u32,
    ) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_stability_detector_wake(report_interval_micros)
            .await
            .map_err(ImuError::from)
    }

    pub async fn enable_shake_detection_wake(
        &mut self,
        report_interval_micros: u32,
    ) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_shake_detector_wake(report_interval_micros)
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

/// What ended a dual-source sleep — see [`Imu::wait_for_motion_or_rtc`].
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum WakeSource {
    /// The BNO085 flagged significant motion.
    Motion,
    /// The RTC countdown expired.
    Rtc,
}

/// Which kind of sleep `imu_task` should perform when [`ENTER_SLEEP`] fires.
///
/// The two differ in who owns the host's wake source, which is why they can't be
/// merged: DORMANT is a whole-chip halt, so exactly one task may drive it.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum SleepMode {
    /// Stationary: sleep the sensor with significant-motion wake armed, then
    /// DORMANT the host on HINT. `imu_task` owns the host sleep and reports back
    /// via [`MOTION_WOKE`].
    DeepRest,
    /// Stationary probe: sleep the sensor with motion wake armed AND the RTC
    /// running, waking on whichever comes first. Result is reported via
    /// [`PROBE_RESULT`]. The caller must arm the RTC before signalling this.
    Probe,
    /// Moving: sleep the sensor only. The host keeps the RTC as its wake source
    /// and sleeps itself. `imu_task` acks via [`SENSOR_ASLEEP`] and then waits
    /// for [`WAKE_SENSOR`].
    SensorOnly,
}

pub static ENTER_SLEEP: Signal<CriticalSectionRawMutex, SleepMode> = Signal::new();

/// Ack from `imu_task`: the BNO085 is now asleep. The host must wait for this
/// before dormanting on the RTC path — dormanting mid-sequence would cut the
/// sleep commands off part-way.
pub static SENSOR_ASLEEP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Outcome of a [`SleepMode::Probe`] sleep: which source woke the host.
pub static PROBE_RESULT: Signal<CriticalSectionRawMutex, WakeSource> = Signal::new();

/// Host -> `imu_task`: the RTC ended the sleep, bring the sensor back up.
pub static WAKE_SENSOR: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Wake-back from `imu_task` to `modem_task`: fired once the BNO085 has flagged
/// significant motion and the host is awake again. `modem_task` parks on this
/// after handing off via [`ENTER_SLEEP`], then publishes its immediate update.
pub static MOTION_WOKE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
pub async fn imu_task(
    mut imu: ImuDevice,
    sender: Sender<'static, CriticalSectionRawMutex, ImuReport, 4>,
) {
    loop {
        // race HINT against sleep signal
        match embassy_futures::select::select(imu.inner.wait_for_hint(), ENTER_SLEEP.wait()).await {
            embassy_futures::select::Either::First(_) => {
                // normal data flow
                imu.inner.handle_one_message(&mut Delay, 10).await;

                let report = ImuReport::from(imu.inner.get_last_update());
                if !matches!(report, ImuReport::None) {
                    let _ = sender.try_send(report);
                }
            }
            embassy_futures::select::Either::Second(mode) => match mode {
                SleepMode::DeepRest => {
                    // Sleep until the BNO085 flags significant motion. Mock keeps
                    // clocks and the probe alive (HINT wait); the real path
                    // DORMANTs the whole board.
                    #[cfg(feature = "mock_host_sleep")]
                    imu.wait_for_motion().await;
                    #[cfg(not(feature = "mock_host_sleep"))]
                    imu.wait_for_motion_dormant().await;

                    // Release modem_task to publish immediately, then emit the UI
                    // report as before.
                    MOTION_WOKE.signal(());
                    sender.send(ImuReport::MotionDetected).await;
                }
                SleepMode::Probe => {
                    let src = imu.wait_for_motion_or_rtc().await;
                    PROBE_RESULT.signal(src);
                    if src == WakeSource::Motion {
                        sender.send(ImuReport::MotionDetected).await;
                    }
                }
                SleepMode::SensorOnly => {
                    // Sensor down, host stays in charge of its own RTC wake.
                    imu.sleep_sensor().await;
                    SENSOR_ASLEEP.signal(());
                    WAKE_SENSOR.wait().await;
                    imu.wake_sensor().await;
                }
            },
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