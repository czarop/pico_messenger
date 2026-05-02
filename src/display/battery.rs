use embedded_graphics::{
    Drawable,
    pixelcolor::BinaryColor,
    prelude::{DrawTarget, Point, Size},
    primitives::{PrimitiveStyleBuilder, Rectangle},
};

use embedded_graphics::prelude::Primitive;

pub struct BatteryIcon<'a> {
    pub level: &'a BatteryLevel,
    pub position: Point,
}

impl<'a> Drawable for BatteryIcon<'a> {
    type Color = BinaryColor;
    type Output = ();

    fn draw<D: DrawTarget<Color = BinaryColor>>(&self, display: &mut D) -> Result<(), D::Error> {
        let fill = PrimitiveStyleBuilder::new()
            .fill_color(BinaryColor::On)
            .build();
        let stroke = PrimitiveStyleBuilder::new()
            .stroke_color(BinaryColor::On)
            .stroke_width(1)
            .build();

        Rectangle::new(self.position, Size::new(20, 9))
            .into_styled(stroke)
            .draw(display)?;
        Rectangle::new(self.position + Point::new(20, 3), Size::new(2, 3))
            .into_styled(fill)
            .draw(display)?;

        let bars = match self.level {
            BatteryLevel::Empty => 0,
            BatteryLevel::Low => 1,
            BatteryLevel::Medium => 2,
            BatteryLevel::Full => 4,
        };
        for i in 0..bars {
            Rectangle::new(self.position + Point::new(2 + i * 4, 2), Size::new(3, 5))
                .into_styled(fill)
                .draw(display)?;
        }
        Ok(())
    }
}

pub enum BatteryLevel {
    Empty,
    Low,
    Medium,
    Full,
}
