use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub enum GnssValidity {
    Valid,
    Invalid(u8),
}

impl GnssValidity {
    pub fn not_enough_satellites(&self) -> bool {
        self.has_flag(1)
    }
    pub fn not_enough_ephemerides(&self) -> bool {
        self.has_flag(2)
    }
    pub fn position_impossible(&self) -> bool {
        self.has_flag(4)
    }
    pub fn dop_too_high(&self) -> bool {
        self.has_flag(8)
    }
    pub fn ssr_too_high(&self) -> bool {
        self.has_flag(16)
    }
    pub fn accuracy_too_low(&self) -> bool {
        self.has_flag(32)
    }
    pub fn altitude_out_of_bounds(&self) -> bool {
        self.has_flag(64)
    }
    pub fn integrity_check_failed(&self) -> bool {
        self.has_flag(128)
    }

    fn has_flag(&self, flag: u8) -> bool {
        match self {
            Self::Invalid(v) => v & flag != 0,
            Self::Valid => false,
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            GnssValidity::Valid => true,
            GnssValidity::Invalid(_) => false,
        }
    }
}

impl From<u8> for GnssValidity {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Valid,
            v => Self::Invalid(v),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GnssPosition {
    pub week_number: u16,
    pub time_of_week: u32,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f32,
    pub accuracy: f32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GnssAccuracy {
    pub std_dev_altitude: f32,
    pub hdop: f32,
    pub gdop: f32,
    pub pdop: f32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GnssLocation {
    pub position: GnssPosition,
    pub accuracy: Option<GnssAccuracy>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub enum GnssFixUrc {
    Searching(GnssValidity),
    Fix(GnssLocation),
}
