use serde::Deserialize;

use crate::modem::gnss::GnssError;

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct GnssPosition {
    pub week_number: u16,
    pub time_of_week: u32,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f32,
    pub accuracy: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GnssAccuracy {
    pub std_dev_altitude: f32,
    pub hdop: f32,
    pub gdop: f32,
    pub pdop: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GnssLocation {
    pub position: GnssPosition,
    pub accuracy: GnssAccuracy,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GnssFixUrc {
    Searching(GnssValidity),
    Fix(GnssLocation),
}

fn field<T>(opt: Option<T>, name: &str) -> Result<T, GnssError> {
    opt.ok_or_else(|| GnssError::MissingField(heapless::String::try_from(name).unwrap_or_default()))
}

impl GnssFixUrc {
    pub fn try_from_raw(raw: GnssFixUrcRaw) -> Result<Self, GnssError> {
        match raw.validity {
            0 => Ok(Self::Fix(GnssLocation {
                position: GnssPosition {
                    week_number: field(raw.week_number, "week_number")?,
                    time_of_week: field(raw.time_of_week, "time_of_week")?,
                    latitude: field(raw.latitude, "latitude")?,
                    longitude: field(raw.longitude, "longitude")?,
                    altitude: field(raw.altitude, "altitude")?,
                    accuracy: field(raw.accuracy, "accuracy")?,
                },
                accuracy: GnssAccuracy {
                    std_dev_altitude: field(raw.std_dev_altitude, "std_dev_altitude")?,
                    hdop: field(raw.hdop, "hdop")?,
                    gdop: field(raw.gdop, "gdop")?,
                    pdop: field(raw.pdop, "pdop")?,
                },
            })),
            v => Ok(Self::Searching(GnssValidity::from(v))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, defmt::Format)]
pub struct GnssFixUrcRaw {
    pub validity: u8,
    pub week_number: Option<u16>,
    pub time_of_week: Option<u32>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f32>,
    pub accuracy: Option<f32>,
    pub std_dev_altitude: Option<f32>,
    pub hdop: Option<f32>,
    pub gdop: Option<f32>,
    pub pdop: Option<f32>,
}
