use micromath::F32Ext;

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
    pub fn from_quaternion(quat: [f32; 4]) -> Self {
        let qx = quat[0];
        let qy = quat[1];
        let qz = quat[2];
        let qw = quat[3];

        let heading_rad = f32::atan2(2.0 * (qw * qz + qx * qy), 1.0 - 2.0 * (qy * qy + qz * qz));

        let mut heading_deg = heading_rad.to_degrees();
        if heading_deg < 0.0 {
            heading_deg += 360.0;
        }

        let deg = heading_deg as u16;

        match deg {
            338..=360 | 0..=22 => Heading::North(deg),
            23..=67 => Heading::NorthEast(deg),
            68..=112 => Heading::East(deg),
            113..=157 => Heading::SouthEast(deg),
            158..=202 => Heading::South(deg),
            203..=247 => Heading::SouthWest(deg),
            248..=292 => Heading::West(deg),
            293..=337 => Heading::NorthWest(deg),
            _ => Heading::North(deg),
        }
    }
}

impl core::fmt::Display for Heading {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Heading::North(d) => write!(f, "North({}°)", d),
            Heading::NorthEast(d) => write!(f, "NorthEast({}°)", d),
            Heading::East(d) => write!(f, "East({}°)", d),
            Heading::SouthEast(d) => write!(f, "SouthEast({}°)", d),
            Heading::South(d) => write!(f, "South({}°)", d),
            Heading::SouthWest(d) => write!(f, "SouthWest({}°)", d),
            Heading::West(d) => write!(f, "West({}°)", d),
            Heading::NorthWest(d) => write!(f, "NorthWest({}°)", d),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct HeadingReading {
    pub heading: Heading,
    pub accuracy_deg: f32,
}

impl HeadingReading {
    pub fn is_reliable(&self) -> bool {
        self.accuracy_deg < 6.0
    }
}

impl core::fmt::Display for HeadingReading {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if !self.is_reliable() {
            write!(f, "{} +/-{:.0}°", self.heading, self.accuracy_deg)
        } else {
            write!(f, "{}", self.heading)
        }
    }
}

impl defmt::Format for HeadingReading {
    fn format(&self, f: defmt::Formatter) {
        if !self.is_reliable() {
            let accuracy = self.accuracy_deg as u16;
            defmt::write!(f, "{} +/-{}°", self.heading, accuracy)
        } else {
            defmt::write!(f, "{}", self.heading)
        }
    }
}
