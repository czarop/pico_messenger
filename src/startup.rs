
use crate::display::screen::{self, StatusScreen};

use crate::sensors::bno085::bno085::{self, ENTER_SLEEP, ImuCommand, ImuReport};
use crate::sensors::{battery_meter, temp_sensor};
use crate::state::{EmbassyStorage, load_state};
use embassy_sync::channel::Channel;
use heapless::String;
use core::fmt::Write;
use core::str::FromStr;
use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c;
use embassy_executor::Spawner;

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
});

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

// static IMU_COMMANDS: Channel<CriticalSectionRawMutex, ImuCommand, 4> = Channel::new();
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

    info!("on startup message count was: {}", state.msg_count);

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
    let i2c_for_battery_monitor: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c0_bus);
    let i2c_for_temp_senor: i2c::I2cDevice<
        '_,
        CriticalSectionRawMutex,
        I2c<'static, I2C0, embassy_rp::i2c::Async>,
    > = i2c::I2cDevice::new(i2c0_bus);



    let mut display = screen::Display::new(i2c_for_display).await;
    let mut temp_senor = temp_sensor::TempSensor::new(i2c_for_temp_senor);
    let mut max17048 = battery_meter::Max17048::new(i2c_for_battery_monitor);
    let mut bno085 = bno085::Imu::new(i2c_for_bno085,  p.PIN_3, p.PIN_2).await;

    bno085.enable_rotation_vector(1000).await.expect("Failed to enable rotation vector");
    // bno085.enable_activity_recognition().await.expect("failed to initiate activity type");
    bno085.enable_significant_motion_wake().await.expect("failed to enable shake detection");


    spawner.spawn(bno085::imu_task(
        bno085,
        // IMU_COMMANDS.receiver(),
        IMU_REPORTS.sender(),
    ).expect("failed to spawn imu task"));

        spawner.spawn(bno085::ui_task(
        IMU_REPORTS.receiver(),
    ).expect("failed to spawn imu task"));

// Keep sender/receiver handles for main task
// let imu_cmd = IMU_COMMANDS.sender();
// let imu_report = IMU_REPORTS.receiver();



info!("entering loop");

    loop{
        info!("loop top");
        // imu_cmd.send(ImuCommand::GetHeading).await;
        // info!("2");
        // imu_cmd.send(ImuCommand::GetStepCount).await;
        // info!("3");
        // imu_cmd.send(ImuCommand::WaitForMotion).await;
        ENTER_SLEEP.signal(());
        info!("loop after sleep");

    let soc = match max17048.soc().await {
        Ok(soc) => soc,
        Err(e) => {
            // error!("Failed to read state of charge: {:?}", e);
            0
        }
    };
    // info!("State of charge: {}%", soc);
    let is_charging = match max17048.charge_rate().await {
        Ok(rate) => {
            info!("Charge rate: {}%/hr", rate);
            rate > 0.0
        },
        Err(e) => {
            // error!("Failed to read charge rate: {:?}", e);
            false
        }
    };

    let (temp_reading, humidity_reading) = match temp_senor.read_temperature(temp_sensor::TempSensorPowerMode::LPM3).await {
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
    let battery_level = battery_meter::BatteryLevel::from_soc(soc, is_charging);
   
    
    
    let display_info = StatusScreen{ 
        battery: battery_level, 
        message: [
            temp_reading.clone(),
            humidity_reading.clone(),
            Some(heapless::String::<24>::from_str("Updated!").expect("could not make heapless string")),
            None,
            None
    ]};
    let _ = display.show_message(display_info).await;


    embassy_time::Timer::after(embassy_time::Duration::from_secs(15)).await;
    


    let display_info = StatusScreen{ 
        battery: battery_level, 
        message: [
            temp_reading,
            humidity_reading,
            Some(heapless::String::<24>::from_str("Shake to update").expect("could not make heapless string")),
            None,
            None
    ]};
    let _ = display.show_message(display_info).await;
    

    }
}
