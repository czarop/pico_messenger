use crate::display::screen;

use crate::modem::communication::{COMMAND_CHANNEL, GNSS_COMMAND};
use crate::modem::gnss::commands::init::GnssInit;
use crate::modem::setup::initiate_modem;
use crate::power;
use crate::sensors::bno085::bno085;
use crate::sensors::bno085::reports::ImuReport;
use crate::sensors::{battery_meter, telemetry, temp_sensor};
use crate::state::{EmbassyStorage, load_state};
use core::fmt::Write;
use core::str::FromStr;
use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c;
use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use heapless::String;

use embassy_rp::i2c::I2c;
use embassy_rp::peripherals::{DMA_CH0, I2C0, I2C1, PIO0};

use embassy_rp::pio::InterruptHandler;
use embassy_rp::{bind_interrupts, dma};
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex};
use embassy_sync::mutex::Mutex;

use littlefs2::fs::{Allocation, Filesystem};
use {defmt_rtt as _, panic_probe as _};

use static_cell::StaticCell;
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
    I2C0_IRQ => embassy_rp::i2c::InterruptHandler<I2C0>;
    I2C1_IRQ => embassy_rp::i2c::InterruptHandler<I2C1>;
    POWMAN_IRQ_TIMER => embassy_rp::aon_timer::InterruptHandler;
});

/// Shared-bus I2C0 handle, as handed to the telemetry task.
///
/// Embassy tasks cannot be generic, so the concrete device type has to be named
/// somewhere both this module and `sensors::telemetry` can see it.
pub type TelemetryI2c =
    i2c::I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, I2C0, embassy_rp::i2c::Async>>;

static STORAGE: StaticCell<EmbassyStorage> = StaticCell::new();
static ALLOC: StaticCell<Allocation<EmbassyStorage>> = StaticCell::new();
static FS: StaticCell<Mutex<ThreadModeRawMutex, Filesystem<'static, EmbassyStorage>>> =
    StaticCell::new();
static I2C0_BUS: StaticCell<
    Mutex<CriticalSectionRawMutex, I2c<'static, I2C0, embassy_rp::i2c::Async>>,
> = StaticCell::new();
static I2C1_BUS: StaticCell<
    Mutex<CriticalSectionRawMutex, I2c<'static, I2C1, embassy_rp::i2c::Async>>,
> = StaticCell::new();

static IMU_REPORTS: Channel<CriticalSectionRawMutex, ImuReport, 4> = Channel::new();

pub async fn startup(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let flash = embassy_rp::flash::Flash::new_blocking(p.FLASH);

    // StaticCell::init() gives you &'static mut T
    let storage = STORAGE.init(EmbassyStorage { flash });
    let alloc = ALLOC.init(Allocation::new());

    if Filesystem::mount(alloc, storage).is_err() {
        Filesystem::format(storage).expect("Format failed");
    }
    let fs = Filesystem::mount(alloc, storage).expect("Mount failed");

    // Wrap in Mutex and store globally
    let fs: &'static mut Mutex<_, _> = FS.init(Mutex::new(fs));

    let mut state = {
        let mut file_system = fs.lock().await;
        load_state(&mut *file_system)
    };

    let i2c0 = I2c::new_async(
        p.I2C0,
        p.PIN_21,
        p.PIN_20,
        Irqs,
        embassy_rp::i2c::Config::default(),
    );
    let i2c0_bus = I2C0_BUS.init(Mutex::new(i2c0));

    let i2c1 = I2c::new_async(
        p.I2C1,
        p.PIN_7,
        p.PIN_6,
        Irqs,
        embassy_rp::i2c::Config::default(),
    );
    let i2c1_bus = I2C1_BUS.init(Mutex::new(i2c1));

    let i2c_for_bno085: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C1, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c1_bus);

    let i2c_for_display: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c0_bus);
    let i2c_for_battery_monitor: TelemetryI2c = i2c::I2cDevice::new(i2c0_bus);
    let i2c_for_temp_sensor: TelemetryI2c = i2c::I2cDevice::new(i2c0_bus);
    let mut i2c_for_pressure_sensor: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c0_bus);

    

    let my_altitude_known = 21.0;
    // let mut pressure_sensor =
    //     crate::sensors::altimeter::Altimeter::new(i2c_for_pressure_sensor, Some(my_altitude_known))
    //         .await
    //         .expect("could not initiate pressure sensor");

    // ---- IMU ----
    let mut bno085 = bno085::Imu::new(i2c_for_bno085, p.PIN_3, p.PIN_2).await;

    bno085
        .enable_rotation_vector(1000)
        .await
        .expect("Failed to enable rotation vector");

    bno085
        .enable_significant_motion_wake()
        .await
        .expect("failed to enable shake detection");

    // Persist calibration across restarts. Without it MotionEngine relearns from
    // scratch on every power-up, and heading stays above the accuracy threshold
    // (so unpublished) for a while after each boot.
    // bno085
    //     .enable_periodic_dcd_save()
    //     .await
    //     .expect("failed to enable periodic DCD save");

    spawner
        .spawn(bno085::imu_task(bno085, IMU_REPORTS.sender()).expect("failed to spawn imu task"));

    spawner.spawn(bno085::ui_task(IMU_REPORTS.receiver()).expect("failed to spawn ui task"));

    // ---- Battery / temperature telemetry ----
    // The task owns both sensors and publishes into shared slots that modem_task
    // reads at publish time. It only runs while the host is awake -- DORMANT halts
    // it along with everything else.
    let battery_monitor = battery_meter::Max17048::new(i2c_for_battery_monitor);
    let temperature_sensor = temp_sensor::TempSensor::new(i2c_for_temp_sensor);
    spawner.spawn(
        telemetry::telemetry_task(battery_monitor, temperature_sensor)
            .expect("failed to spawn telemetry task"),
    );

    // ---- RTC ----
    let i2c_for_rtc = i2c::I2cDevice::new(i2c0_bus);
    match crate::rtc::Pcf8523::new(i2c_for_rtc).await {
        Ok(rtc) => crate::rtc::init(rtc).await, // register; do NOT arm here
        Err(e) => defmt::error!("RTC init failed (I2C): {:?}", e),
    }
    let rtc_int = embassy_rp::gpio::Input::new(p.PIN_13, embassy_rp::gpio::Pull::Up);
    crate::power::init(rtc_int).await;

    

    i2c_scan(&mut i2c_for_pressure_sensor).await;

    // ---- Display (debug status panel) ----
    // Panel is OFF by default; FeatherWing button A (Feather D9 = GPIO25 on the
    // Challenger+) turns it on for 30s, a second press blanks it early. The
    // task takes MQTT_STATE's one spare watch receiver (3 slots total;
    // network_task and modem_task take the other two) so the stack phase is
    // read at the source with no setter plumbing. A failed display init only
    // disables the panel -- it must never take the device down.
    let display_button = embassy_rp::gpio::Input::new(p.PIN_25, embassy_rp::gpio::Pull::Up);
    match screen::Display::new(i2c_for_display).await {
        Ok(display) => match crate::modem::communication::MQTT_STATE.receiver() {
            Some(mqtt_rx) => spawner
                .spawn(crate::display::status::display_task(
                    display,
                    display_button,
                    mqtt_rx,
                ).expect("failed to spawn display task"))
                ,
            None => defmt::warn!("no MQTT_STATE receiver free - status panel disabled"),
        },
        Err(_) => defmt::warn!("display init failed - status panel disabled"),
    }

    // ---- Modem ----
    let gnss_bias = embassy_rp::gpio::Output::new(p.PIN_11, embassy_rp::gpio::Level::High);
    let wake_pin = embassy_rp::gpio::Output::new(p.PIN_10, embassy_rp::gpio::Level::High);
    let tx_pin: embassy_rp::Peri<'static, embassy_rp::peripherals::PIN_4> = p.PIN_4;
    let rx_pin: embassy_rp::Peri<'static, embassy_rp::peripherals::PIN_5> = p.PIN_5;
    let uart: embassy_rp::Peri<'static, embassy_rp::peripherals::UART1> = p.UART1;

    #[cfg(not(feature = "mock_modem"))]
    initiate_modem(spawner, tx_pin, rx_pin, gnss_bias, wake_pin, uart);

    #[cfg(feature = "mock_modem")]
    {
        // No modem hardware: the UART pins / bias are not used in this build.
        let _ = (tx_pin, rx_pin, gnss_bias, wake_pin, uart);
        crate::modem::setup::initiate_mock_modem(spawner);
    }

    // NOTE: do NOT signal ENTER_SLEEP from here. `modem_task` owns the sleep
    // regime -- it decides between the RTC cadence, the motion probe and
    // indefinite rest, and signals `imu_task` accordingly. A second signaller
    // races it: ENTER_SLEEP holds one value and one waiter consumes it, so the
    // two tasks end up fighting over who puts the sensor to sleep.
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(15)).await;
    }
}

async fn i2c_scan(i2c: &mut impl embedded_hal_async::i2c::I2c) {
    info!("Scanning I2C bus...");
    for addr in 0x08..=0x77u8 {
        let mut buf = [0u8; 1];
        match i2c.read(addr, &mut buf).await {
            Ok(_) => info!("Found device at 0x{:02X}", addr),
            Err(_) => {}
        }
    }
    info!("Scan complete");
}