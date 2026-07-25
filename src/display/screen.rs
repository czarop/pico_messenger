use embassy_embedded_hal::shared_bus::asynch::i2c;
use embassy_rp::i2c::I2c;
use embassy_rp::peripherals::I2C0;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, ascii::FONT_6X10},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};

use oled_async::{Builder, prelude::*};

use crate::{display::battery::BatteryIcon, sensors::battery_meter::BatteryLevel};

pub struct StatusScreen {
    pub battery: BatteryLevel,
    pub message: [Option<heapless::String<24>>; 5],
}

impl Drawable for StatusScreen {
    type Color = BinaryColor;
    type Output = ();

    fn draw<D: DrawTarget<Color = BinaryColor>>(&self, display: &mut D) -> Result<(), D::Error> {
        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        BatteryIcon {
            level: &self.battery,
            position: Point::new(105, 1),
        }
        .draw(display)?;

        for (i, line) in self.message.iter().enumerate() {
            if let Some(text) = line.as_deref() {
                Text::with_baseline(
                    text,
                    Point::new(0, 2 + i as i32 * 12),
                    text_style,
                    Baseline::Top,
                )
                .draw(display)?;
            }
        }

        Ok(())
    }
}

type DisplayType = oled_async::displays::sh1107::Sh1107_64_128;
type DisplayI2C = display_interface_i2c::I2CInterface<
    i2c::I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, I2C0, embassy_rp::i2c::Async>>,
>;

pub struct Display {
    inner: GraphicsMode<DisplayType, DisplayI2C, { 128 * 128 / 8 }>,
}

impl Display {
    pub async fn new(
        i2c: embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, I2C0, embassy_rp::i2c::Async>,
        >,
    ) -> Result<Self, DisplayError> {
        let di: display_interface_i2c::I2CInterface<
            embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
                '_,
                CriticalSectionRawMutex,
                I2c<'static, I2C0, embassy_rp::i2c::Async>,
            >,
        > = display_interface_i2c::I2CInterface::new(i2c, 0x3C, 0x40);
        let raw_disp = Builder::new(DisplayType {})
            .with_rotation(DisplayRotation::Rotate90)
            .connect(di);

        let mut display: GraphicsMode<_, _, { 128 * 128 / 8 }> = raw_disp.into();
        display.init().await.map_err(|_| DisplayError::Init)?;
        display.clear();
        display.flush().await.map_err(|_| DisplayError::Init)?;
        Ok(Self { inner: display })
    }

    pub async fn turn_display_off(&mut self) -> Result<(), DisplayError> {
        self.inner
            .display_on(false)
            .await
            .map_err(|_| DisplayError::Power)
    }

    pub async fn turn_display_on(&mut self) -> Result<(), DisplayError> {
        self.inner
            .display_on(true)
            .await
            .map_err(|_| DisplayError::Power)
    }

    // pub async fn show_status(&mut self, battery: u8, signal: i8) { ... }
    pub async fn show_message(&mut self, display_info: StatusScreen) -> Result<(), DisplayError> {
        let display = &mut self.inner;
        display.clear();

        display_info
            .draw(display)
            .map_err(|_| DisplayError::Flush)?;

        display.flush().await.map_err(|_| DisplayError::Flush)?;

        Ok(())
    }
    pub async fn clear(&mut self) -> Result<(), DisplayError> {
        self.inner.clear();
        self.inner.flush().await.map_err(|_| DisplayError::Flush)
    }
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DisplayError {
    #[error("Error updating display")]
    Flush,
    #[error("Error initialising display")]
    Init,
    #[error("Error switching display on/off")]
    Power,
}