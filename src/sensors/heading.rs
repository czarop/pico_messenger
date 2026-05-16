use crate::sensors::{accelerometer::AccelerometerReading, magnetometer::{MagnetometerReading, RawReading}};
use micromath::F32Ext;

// determin after device construction or set up in a calibration procedure
// and log to file state
const MAG_OFFSET_X: f32 = 165.0;
const MAG_OFFSET_Y: f32 = 7.0;
const MAG_OFFSET_Z: f32 = -122.0;

const MAG_SCALE_X: f32 = 104.0;
const MAG_SCALE_Y: f32 = 111.0;
const MAG_SCALE_Z: f32 = 112.0;

#[derive(Debug, defmt::Format, PartialEq, Eq)]
pub enum Heading {
    North(u16),
    NorthEast(u16),
    East(u16),
    SouthEast(u16),
    South(u16),
    SouthWest(u16),
    West(u16),
    NorthWest(u16),
}

impl Heading {
    pub fn new_from_raw(accel: &AccelerometerReading, raw: &RawReading) -> Self {
        // when sensor is positioned 90 degrees on long edge (upright)
        // let mx = (raw.x_unscaled() as f32 - MAG_OFFSET_X) / MAG_SCALE_X;
        // let my = (raw.y_unscaled() as f32 - MAG_OFFSET_Y) / MAG_SCALE_Y;
        // let mz = (raw.z_unscaled() as f32 - MAG_OFFSET_Z) / MAG_SCALE_Z;
        
        // let heading_rad = (my).atan2(mz);
        

        // when sensor is flat
        let mx = (raw.x_unscaled() as f32 - MAG_OFFSET_X) / MAG_SCALE_X;
        let my = -((raw.y_unscaled() as f32 - MAG_OFFSET_Y) / MAG_SCALE_Y);
        let mz = ((raw.z_unscaled() as f32 - MAG_OFFSET_Z) / MAG_SCALE_Z);

        let pitch = accel.pitch();
        let roll = accel.roll();

        let x_comp = mx * pitch.cos() + mz * pitch.sin();
        let y_comp = mx * roll.sin() * pitch.sin() + my * roll.cos() - mz * roll.sin() * pitch.cos();

        let heading_rad = y_comp.atan2(x_comp);


        let mut heading_deg = heading_rad.to_degrees();
        if heading_deg < 0.0 {
            heading_deg += 360.0;
        }

        defmt::info!("pitch: {}, roll: {}", pitch, roll);
defmt::info!("mx: {}, my: {}, mz: {}", mx, my, mz);
defmt::info!("x_comp: {}, y_comp: {}", x_comp, y_comp);

        match heading_deg as u16 {
            338..=360 | 0..=22 => Heading::North(heading_deg as u16),
            23..=67 => Heading::NorthEast(heading_deg as u16),
            68..=112 => Heading::East(heading_deg as u16),
            113..=157 => Heading::SouthEast(heading_deg as u16),
            158..=202 => Heading::South(heading_deg as u16),
            203..=247 => Heading::SouthWest(heading_deg as u16),
            248..=292 => Heading::West(heading_deg as u16),
            293..=337 => Heading::NorthWest(heading_deg as u16),
            _ => Heading::North(heading_deg as u16),
        }
    }

    pub fn new(accel: &AccelerometerReading, mag: &MagnetometerReading) -> Self {
        defmt::info!("x: {}, y: {}, z: {}", -mag.x, mag.y, -mag.z);
        let mx = -((mag.x as f32 - MAG_OFFSET_X) / MAG_SCALE_X);
        let my = (mag.y as f32 - MAG_OFFSET_Y) / MAG_SCALE_Y;
        let mz = -((mag.z as f32 - MAG_OFFSET_Z) / MAG_SCALE_Z);
        defmt::info!("mx: {}, my: {} mz: {}", mx, my, mz);
        let pitch = accel.pitch();
        let roll = accel.roll();
        let x_comp = mx * pitch.cos() + mz * pitch.sin();
        let y_comp =
            mx * roll.sin() * pitch.sin() + my * roll.cos() - mz * roll.sin() * pitch.cos();
        // let heading_rad = f32::atan2(y_comp, x_comp);
        let heading_rad = (my).atan2(mz);
        let mut heading_deg = heading_rad.to_degrees();
        if heading_deg < 0.0 {
            heading_deg += 360.0;
        }

        match heading_deg as u16 {
            338..=360 | 0..=22 => Heading::North(heading_deg as u16),
            23..=67 => Heading::NorthEast(heading_deg as u16),
            68..=112 => Heading::East(heading_deg as u16),
            113..=157 => Heading::SouthEast(heading_deg as u16),
            158..=202 => Heading::South(heading_deg as u16),
            203..=247 => Heading::SouthWest(heading_deg as u16),
            248..=292 => Heading::West(heading_deg as u16),
            293..=337 => Heading::NorthWest(heading_deg as u16),
            _ => Heading::North(heading_deg as u16),
        }
    }
}
