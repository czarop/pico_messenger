use crate::accelerometer::Accelerometer;
use crate::battery_meter::BatteryLevel;
use crate::display::screen;
use crate::state::{EmbassyStorage, load_state};
use heapless::String;
use core::fmt::Write;
use core::str::FromStr;
use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c;
use embassy_executor::Spawner;

use embassy_rp::i2c::I2c;
use embassy_rp::peripherals::{DMA_CH0, I2C0, PIO0};

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
});

static STORAGE: StaticCell<EmbassyStorage> = StaticCell::new();
static ALLOC: StaticCell<Allocation<EmbassyStorage>> = StaticCell::new();
static FS: StaticCell<Mutex<ThreadModeRawMutex, Filesystem<'static, EmbassyStorage>>> =
    StaticCell::new();
static I2C_BUS: StaticCell<
    Mutex<CriticalSectionRawMutex, I2c<'static, I2C0, embassy_rp::i2c::Async>>,
> = StaticCell::new();

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

    info!("on startup message count was: {}", state.msg_count);

    let i2c = I2c::new_async(
        p.I2C0,
        p.PIN_21,
        p.PIN_20,
        Irqs,
        embassy_rp::i2c::Config::default(),
    );
    let i2c_bus = I2C_BUS.init(Mutex::new(i2c));
    let i2c_for_accelerometer: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c_bus);
    let i2c_for_display: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c_bus);
    let i2c_for_battery_monitor: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c_bus);
    let i2c_for_temp_senor: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c_bus);

    let mut accelerometer = Accelerometer::new(p.PIN_15, i2c_for_accelerometer).await;
    let mut display = screen::Display::new(i2c_for_display).await;
    let mut temp_senor = crate::temp_sensor::TempSensor::new(i2c_for_temp_senor);

    

    let mut max17048 = crate::battery_meter::Max17048::new(i2c_for_battery_monitor);
    

    accelerometer.configure_wake_on_movement(0x02, 0x20).await;

    loop{
    accelerometer.clear_wake_source().await;



    let soc = match max17048.soc().await {
        Ok(soc) => soc,
        Err(e) => {
            error!("Failed to read state of charge: {:?}", e);
            0
        }
    };
    info!("State of charge: {}%", soc);
    let is_charging = match max17048.charge_rate().await {
        Ok(rate) => {
            info!("Charge rate: {}%/hr", rate);
            rate > 0.0
        },
        Err(e) => {
            error!("Failed to read charge rate: {:?}", e);
            false
        }
    };
    info!("Is charging: {}", is_charging);

    let (temp_reading, humidity_reading) = match temp_senor.read_temperature(crate::temp_sensor::TempSensorPowerMode::LPM3).await {
        Ok(r) => {
            // info!("Temperature: {}°C, Humidity: {}%", r.temperature, r.humidity);
            let mut temp: String<24> = String::new();
            core::write!(temp, "Temp: {:.1}C", r.temperature).unwrap();
            let mut humidity: String<24> = String::new();
            core::write!(humidity, "Humidity: {:.1}%", r.humidity).unwrap();
            (Some(temp), Some(humidity))
        },
        Err(e) => {
            error!("{:?}",defmt::Debug2Format(&e));
            let err_string:String<24>  = String::from_str("Temp Senor Error").expect("error making error string");
            (Some(err_string), None)
        },
    };
    let battery_level = BatteryLevel::from_soc(soc, is_charging);
    

    let _ = display.show_message(temp_reading.as_deref(), humidity_reading.as_deref(), Some("Updated!"), battery_level).await;
    
    embassy_time::Timer::after(embassy_time::Duration::from_secs(3)).await;
    

    // info!("Entering low power mode, waiting for motion...");
    let _ = display.show_message(temp_reading.as_deref(), humidity_reading.as_deref(), Some("Shake to update"), battery_level).await;
    // embassy_time::Timer::after(embassy_time::Duration::from_secs(3)).await;
    accelerometer.wait_for_motion().await;

    // let _ = display.show_message(Some("woken..."), Some("I woke up!"), None, battery_level).await;
    // embassy_time::Timer::after(embassy_time::Duration::from_secs(3)).await;
    // display.clear().await;
    // display.turn_display_off().await;

    // embassy_time::Timer::after(embassy_time::Duration::from_secs(3)).await;

    // embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;

    }
}
