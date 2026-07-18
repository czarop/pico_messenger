// use super::bno08x::Error;
// use super::bno08x::interface::i2c_async::I2cInterfaceAsync;
// use super::bno08x::wrapper_async::{BNO085Async, ROTATION_VECTOR_REPORT_ID, WrapperError};
// use defmt::info;
// use embassy_rp::Peri;
// use embassy_rp::gpio::{Input, Output};
// use embassy_rp::peripherals::{PIN_2, PIN_3};
// use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// use embassy_sync::channel::{Receiver, Sender};

// use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
// use embassy_rp::i2c::{Async, I2c};
// use embassy_rp::peripherals::I2C1;
// use embassy_sync::signal::Signal;
// use embassy_time::Delay;

// use crate::sensors::bno085::reports::ImuReport;

// type ImuI2C = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, I2C1, Async>>;

// pub type ImuDevice = Imu<ImuI2C, Input<'static>>;

// pub struct Imu<I2C, HINT> {
//     inner: BNO085Async<I2cInterfaceAsync<I2C, HINT>>,
// }

// impl<I2C, CommE> Imu<I2C, Input<'static>>
// where
//     I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
//     CommE: core::fmt::Debug,
// {
//     pub async fn new(
//         i2c: I2C,
//         int_pin: Peri<'static, PIN_3>,
//         rst_pin: Peri<'static, PIN_2>,
//     ) -> Self {
//         let hint = Input::new(int_pin, embassy_rp::gpio::Pull::Up);
//         let mut rst_pin = Output::new(rst_pin, embassy_rp::gpio::Level::High);
//         let iface = I2cInterfaceAsync::default(i2c, hint);
//         let mut inner = BNO085Async::new_with_interface(iface);

//         rst_pin.set_low();
//         embassy_time::Timer::after_millis(10).await;
//         rst_pin.set_high();

//         inner.wait_for_hint().await;

//         inner
//             .init(&mut embassy_time::Delay)
//             .await
//             .expect("BNO085 init failed");
//         info!("BNO085 init completed");
//         Self { inner }
//     }

//     pub async fn wait_for_motion_dormant(&mut self) {
//         self.inner
//             .enable_significant_motion_wake()
//             .await
//             .expect("failed to enable significant motion wake");
//         self.inner.sleep().await.expect("failed to sleep BNO085");
//         embassy_time::Timer::after_millis(50).await;
//         self.inner.handle_all_messages(&mut Delay, 10).await;
//         self.inner.dormant_sleep_on_hint();
//         self.inner.wake().await.expect("failed to wake BNO085");
//     }

//     // for testing - keeps clocks running so connected to probe
//     pub async fn wait_for_motion(&mut self) {
//         self.inner
//             .enable_significant_motion_wake()
//             .await
//             .expect("failed to enable significant motion wake");
//         self.inner.sleep().await.expect("failed to sleep BNO085");
//         embassy_time::Timer::after_millis(50).await;
//         self.inner.handle_all_messages(&mut Delay, 10).await;
//         self.inner.wait_for_hint().await;
//         self.inner.wake().await.expect("failed to wake BNO085");
//     }
// }

// impl<I2C, HINT, CommE> Imu<I2C, HINT>
// where
//     I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
//     CommE: core::fmt::Debug,
//     HINT: embedded_hal::digital::InputPin + embedded_hal_async::digital::Wait,
// {
//     pub async fn enable_rotation_vector(&mut self, millis: u16) -> Result<(), ImuError<CommE>> {
//         self.inner
//             .enable_rotation_vector(millis)
//             .await
//             .map_err(ImuError::from)
//     }

//     // pub fn heading(&mut self) -> Result<HeadingReading, ImuError<CommE>> {

//     //     let quat = self.inner.rotation_quaternion().map_err(ImuError::from)?;

//     //     Ok(HeadingReading {
//     //         heading: Heading::from_quaternion(quat),
//     //         accuracy_deg: self.inner.heading_accuracy(),
//     //     })
//     // }

//     pub async fn enable_activity_recognition(&mut self) -> Result<(), ImuError<CommE>> {
//         self.inner
//             .enable_activity_classifier(60000)
//             .await
//             .map_err(ImuError::from)
//     }

//     pub fn get_activity_type(self) -> super::bno08x::activity::Activity {
//         self.inner.activity()
//     }

//     pub async fn enable_significant_motion_wake(&mut self) -> Result<(), ImuError<CommE>> {
//         self.inner
//             .enable_significant_motion_wake()
//             .await
//             .map_err(ImuError::from)
//     }

//     pub async fn enable_stability_detection_wake(
//         &mut self,
//         report_interval_micros: u32,
//     ) -> Result<(), ImuError<CommE>> {
//         self.inner
//             .enable_stability_detector_wake(report_interval_micros)
//             .await
//             .map_err(ImuError::from)
//     }

//     pub async fn enable_shake_detection_wake(
//         &mut self,
//         report_interval_micros: u32,
//     ) -> Result<(), ImuError<CommE>> {
//         self.inner
//             .enable_shake_detector_wake(report_interval_micros)
//             .await
//             .map_err(ImuError::from)
//     }
// }

// #[derive(Debug, thiserror::Error)]
// pub enum ImuError<E: core::fmt::Debug> {
//     #[error("I2C communication error with IMU: {0:?}")]
//     I2c(E),
//     #[error("IMU sensor did not respond")]
//     Unresponsive,
//     #[error("Invalid IMU chip ID: {0}")]
//     InvalidChipId(u8),
//     #[error("Invalid IMU firmware version: {0}")]
//     InvalidFirmware(u8),
//     #[error("No IMU data available")]
//     NoData,
// }

// impl<E: core::fmt::Debug> From<WrapperError<Error<E, ()>>> for ImuError<E> {
//     fn from(e: WrapperError<Error<E, ()>>) -> Self {
//         match e {
//             WrapperError::CommError(Error::Comm(e)) => ImuError::I2c(e),
//             WrapperError::InvalidChipId(id) => ImuError::InvalidChipId(id),
//             WrapperError::InvalidFWVersion(v) => ImuError::InvalidFirmware(v),
//             WrapperError::NoDataAvailable => ImuError::NoData,
//             _ => ImuError::Unresponsive,
//         }
//     }
// }

// pub static ENTER_SLEEP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// #[embassy_executor::task]
// pub async fn imu_task(
//     mut imu: ImuDevice,
//     sender: Sender<'static, CriticalSectionRawMutex, ImuReport, 4>,
// ) {
//     loop {
//         // race HINT against sleep signal
//         match embassy_futures::select::select(imu.inner.wait_for_hint(), ENTER_SLEEP.wait()).await {
//             embassy_futures::select::Either::First(_) => {
//                 // normal data flow
//                 imu.inner.handle_one_message(&mut Delay, 10).await;

//                 let report = ImuReport::from(imu.inner.get_last_update());
//                 if !matches!(report, ImuReport::None) {
//                     let _ = sender.try_send(report);
//                 }
//             }
//             embassy_futures::select::Either::Second(_) => {
//                 // enter low power — whole board sleeps until motion

//                 imu.wait_for_motion().await;
//                 sender.send(ImuReport::MotionDetected).await;
//             }
//         }
//     }
// }

// #[embassy_executor::task]
// pub async fn ui_task(reports: Receiver<'static, CriticalSectionRawMutex, ImuReport, 4>) {
//     loop {
//         // blocks until a report arrives
//         let report = reports.receive().await;
//         match report {
//             ImuReport::Heading(h) => {
//                 info!("Heading: {}", h.heading)
//             }
//             ImuReport::Activity(a) => {
//                 info!("Activity: {:?}", a)
//             }
//             ImuReport::MotionDetected => {
//                 info!("Motion detected")
//             }
//             _ => {}
//         }
//     }
// }
use super::bno08x::Error;
use super::bno08x::interface::i2c_async::I2cInterfaceAsync;
use super::bno08x::wrapper_async::{BNO085Async, ROTATION_VECTOR_REPORT_ID, WrapperError};
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

type ImuI2C = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, I2C1, Async>>;

pub type ImuDevice = Imu<ImuI2C, Input<'static>>;

/// A steady-state BNO085 report plus the params needed to re-enable it. Wake
/// triggers (significant motion, stability, shake) are deliberately excluded:
/// they are the sleep *mechanism*, not reports to reinstate.
#[derive(Clone, Copy)]
pub enum ActiveReport {
    RotationVector { period_ms: u16 },
    ActivityRecognition,
}

pub struct Imu<I2C, HINT> {
    inner: BNO085Async<I2cInterfaceAsync<I2C, HINT>>,
    /// Currently-enabled steady-state reports — the single source of truth used
    /// to reinstate the stream after a motion sleep (which tears reports down).
    /// Kept in sync by the `enable_*` methods via `track`.
    active: heapless::Vec<ActiveReport, 8>,
}

impl<I2C, HINT> Imu<I2C, HINT> {
    /// Upsert a report into the active set: replace an entry of the same kind,
    /// else push. Push failure (capacity) is ignored — `active` is sized for
    /// every report kind, so overflow can't happen in practice.
    fn track(&mut self, report: ActiveReport) {
        let d = core::mem::discriminant(&report);
        if let Some(i) = self.active.iter().position(|e| core::mem::discriminant(e) == d) {
            self.active[i] = report;
        } else {
            let _ = self.active.push(report);
        }
    }
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
        Self {
            inner,
            active: heapless::Vec::new(),
        }
    }

    /// Re-send every currently-active steady-state report to the sensor. Used
    /// after a motion sleep, which tears the reports down. Iterates a snapshot
    /// so the `&mut self` enable calls don't alias `self.active`.
    async fn reinstate_active(&mut self) {
        let snapshot = self.active.clone();
        for report in snapshot {
            let r = match report {
                ActiveReport::RotationVector { period_ms } => {
                    defmt::info!("DIAG reinstate: RV period_ms={}", period_ms);
                    // Plain flags=0 enable. The hub rewrites intervals it does
                    // not accept (it stored 32000us for a 1000us significant
                    // motion request), and with flags=0x0C it stored interval 0
                    // for the rotation vector — i.e. it took the flags and
                    // rejected the rate, which is why nothing reported. Rotation
                    // vector may simply not be legal as a wake/always-on sensor.
                    self.enable_rotation_vector(period_ms).await
                }
                ActiveReport::ActivityRecognition => self.enable_activity_recognition().await,
            };
            if let Err(e) = r {
                defmt::warn!(
                    "failed to reinstate a BNO085 report after wake: {}",
                    defmt::Debug2Format(&e)
                );
            }
        }
    }

    /// Turn every active steady-state report OFF by re-sending Set Feature with
    /// a report interval of zero (SH-2 §5.4.1: interval 0 == sensor off). Leaves
    /// `self.active` untouched so `reinstate_active` can restore them.
    ///
    /// Interval 0 turns the sensor off regardless of its feature flags, so this
    /// is what stops an always-on report from running through the sleep and
    /// waking the host. It also replaces the executable SLEEP command, which is
    /// not needed: with only wake/always-on sensors left enabled the hub enters
    /// its low-power state on its own.
    async fn silence_active(&mut self) {
        let snapshot = self.active.clone();
        for report in snapshot {
            let r = match report {
                ActiveReport::RotationVector { .. } => self.inner.enable_rotation_vector(0).await,
                ActiveReport::ActivityRecognition => self.inner.enable_activity_classifier(0).await,
            };
            if r.is_err() {
                defmt::warn!("failed to silence a BNO085 report before sleep");
            }
        }
    }

    pub async fn wait_for_motion_dormant(&mut self) {
        self.inner
            .enable_significant_motion_wake()
            .await
            .expect("failed to enable significant motion wake");
        // Silence the streaming reports FIRST (interval 0 == off regardless of
        // feature flags). This is what stops the always-on rotation vector from
        // running through the executable sleep and waking the host itself.
        self.silence_active().await;
        // Then the executable SLEEP. Hardware finding: the matching executable
        // ON below is required — without it the hub stays in its low-power state
        // and no Set Feature will restart reporting.
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        // Drain any stale reports, then clear the sig-motion latch so
        // `took_significant_motion()` after wake reflects only THIS sleep window.
        self.inner.handle_all_messages(&mut Delay, 10).await;
        let _ = self.inner.significant_motion_detected();
        self.inner.dormant_sleep_on_hint();
        self.inner.wake().await.expect("failed to wake BNO085");
        // Settle + drain thoroughly before Set Feature. Evidence: with a minimal
        // drain the first reinstate silently fails and only a SECOND full
        // sleep/wake/reinstate pass restores reporting — i.e. the hub is not yet
        // ready to accept feature config immediately after executable ON.
        self.settle_after_wake().await;
        // Rebuild the steady-state report stream the sleep tore down.
        self.reinstate_active().await;
        self.query_rotation_vector_config().await;
    }

    /// DIAGNOSTIC: ask the hub what configuration it currently holds for the
    /// rotation vector. The reply is logged as `DIAG GetFeatureResp`. A non-zero
    /// interval means our Set Feature WAS accepted and the hub simply isn't
    /// producing reports; interval 0 (or no reply at all) means the Set Feature
    /// never landed. That distinction is what we still cannot infer from HINT.
    async fn query_rotation_vector_config(&mut self) {
        let id = ROTATION_VECTOR_REPORT_ID;
        if self.inner.request_feature(id).await.is_err() {
            defmt::warn!("DIAG: GetFeatureRequest send failed");
            return;
        }
        embassy_time::Timer::after_millis(100).await;
        let n = self.inner.handle_all_messages(&mut Delay, 40).await;
        defmt::info!("DIAG: drained {} after GetFeatureRequest", n);
    }

    /// Bring the hub back after executable ON. ON resets this device (it emits
    /// the 3-message reset sequence), so wake is really a re-init: resync the
    /// driver's SHTP state to match the freshly reset device, then run the same
    /// `init` the cold-boot path uses so Set Feature is honoured again.
    async fn settle_after_wake(&mut self) {
        embassy_time::Timer::after_millis(200).await;
        let drained = self.inner.handle_all_messages(&mut Delay, 40).await;
        defmt::info!("DIAG wake settle: drained {}", drained);
        // Critical: the device restarted its SHTP sequence numbers at zero; the
        // driver must do the same or every packet we send is out of step.
        self.inner.resync_after_device_reset();
        if let Err(_e) = self.inner.init(&mut Delay).await {
            defmt::warn!("BNO085 re-init after wake failed");
        }
        let _ = self.inner.significant_motion_detected();
    }

    // for testing - keeps clocks running so connected to probe
    pub async fn wait_for_motion(&mut self) {
        self.inner
            .enable_significant_motion_wake()
            .await
            .expect("failed to enable significant motion wake");
        // Silence the streaming reports FIRST (interval 0 == off regardless of
        // feature flags). This is what stops the always-on rotation vector from
        // running through the executable sleep and waking the host itself.
        self.silence_active().await;
        // Then the executable SLEEP. Hardware finding: the matching executable
        // ON below is required — without it the hub stays in its low-power state
        // and no Set Feature will restart reporting.
        self.inner.sleep().await.expect("failed to sleep BNO085");
        embassy_time::Timer::after_millis(50).await;
        // Drain any stale reports, then clear the sig-motion latch so
        // `took_significant_motion()` after wake reflects only THIS sleep window.
        self.inner.handle_all_messages(&mut Delay, 10).await;
        let _ = self.inner.significant_motion_detected();
        self.inner.wait_for_hint().await;
        self.inner.wake().await.expect("failed to wake BNO085");
        // Settle + drain thoroughly before Set Feature (see settle_after_wake).
        self.settle_after_wake().await;
        // Rebuild the steady-state report stream the sleep tore down.
        self.reinstate_active().await;
        self.query_rotation_vector_config().await;
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
            .map_err(ImuError::from)?;
        defmt::info!("DIAG track: RV period_ms={}", millis);
        self.track(ActiveReport::RotationVector { period_ms: millis });
        Ok(())
    }

    // pub fn heading(&mut self) -> Result<HeadingReading, ImuError<CommE>> {

    //     let quat = self.inner.rotation_quaternion().map_err(ImuError::from)?;

    //     Ok(HeadingReading {
    //         heading: Heading::from_quaternion(quat),
    //         accuracy_deg: self.inner.heading_accuracy(),
    //     })
    // }

    pub async fn enable_activity_recognition(&mut self) -> Result<(), ImuError<CommE>> {
        self.inner
            .enable_activity_classifier(60000)
            .await
            .map_err(ImuError::from)?;
        self.track(ActiveReport::ActivityRecognition);
        Ok(())
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

    /// Read-and-clear the significant-motion latch: `true` if the BNO085 has
    /// signalled a significant-motion event since this was last called.
    ///
    /// Intended as the post-wake discriminator once RTC-INT and BNO-HINT are
    /// armed together: `true` => the BNO woke the host, `false` => it was the
    /// RTC (or nothing). Only meaningful after the wake path has drained
    /// messages (`wait_for_motion*` do this); a bare poll without a preceding
    /// drain reflects only what has already been parsed.
    pub fn took_significant_motion(&mut self) -> bool {
        self.inner.significant_motion_detected()
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

pub static ENTER_SLEEP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

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
                // DIAG (temporary): proves the HINT wait released. If these stop
                // appearing after a motion wake, the task is parked in
                // wait_for_hint => the sensor is not asserting HINT (no reports).
                // If they keep appearing but no Heading is logged, HINT is
                // asserted but handle_one_message yields nothing new => stale
                // last_update being re-sent.
                defmt::info!("DIAG imu: hint released");

                // normal data flow
                imu.inner.handle_one_message(&mut Delay, 10).await;

                let report = ImuReport::from(imu.inner.get_last_update());
                defmt::info!("DIAG imu: report none={}", matches!(report, ImuReport::None));
                if !matches!(report, ImuReport::None) {
                    let _ = sender.try_send(report);
                }
            }
            embassy_futures::select::Either::Second(_) => {
                // Enter low power: sleep until the BNO085 flags significant
                // motion. Mock keeps clocks and the probe alive (HINT wait);
                // the real path DORMANTs the whole board on HINT. Mirrors the
                // same gating as `power::sleep_dormant`.
                #[cfg(feature = "mock_host_sleep")]
                imu.wait_for_motion().await;
                #[cfg(not(feature = "mock_host_sleep"))]
                imu.wait_for_motion_dormant().await;

                // The sleep reinstates the previously-active reports itself (see
                // `reinstate_active`), so the report stream resumes with no
                // restore logic needed here.

                // Woken by motion. Release modem_task to publish immediately,
                // and emit the UI report as before.
                MOTION_WOKE.signal(());
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