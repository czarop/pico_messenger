use embedded_graphics::{
    Drawable,
    pixelcolor::BinaryColor,
    prelude::{DrawTarget, Point, Size},
    primitives::{Line, PrimitiveStyleBuilder, Rectangle},
};

use embedded_graphics::prelude::Primitive;

use crate::sensors::battery_meter::BatteryLevel;

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

        if BatteryLevel::Charging == *self.level {

        let cx = self.position.x + 10;
        let cy = self.position.y + 4;

        Line::new(Point::new(cx - 2, cy), Point::new(cx + 2, cy))
            .into_styled(stroke).draw(display)?;
        Line::new(Point::new(cx, cy - 2), Point::new(cx, cy + 2))
            .into_styled(stroke).draw(display)?;
                } 

        let bars = match self.level {
            BatteryLevel::Empty => 0,
            BatteryLevel::Critical => 1,
            BatteryLevel::Low => 2,
            BatteryLevel::Medium => 3,
            BatteryLevel::High => 4,
            BatteryLevel::Full => 5,
            BatteryLevel::Charging => 0,
        };
        for i in 0..bars {
            Rectangle::new(self.position + Point::new(2 + i * 4, 2), Size::new(3, 5))
                .into_styled(fill)
                .draw(display)?;
        }
        Ok(())
    }
}
