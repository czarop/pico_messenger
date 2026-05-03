use defmt::{error, info};
use embassy_rp::clocks::dormant_sleep;
use embassy_rp::gpio::{DormantWakeConfig, Input};
use embassy_rp::i2c::I2c;
use embassy_rp::peripherals::{I2C0, PIN_15};

use embassy_rp::Peri;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use lis2dw12_i2c::Register;

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
}
