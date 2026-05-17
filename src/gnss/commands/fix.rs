use atat::atat_derive::{AtatCmd, AtatEnum, AtatLen, AtatResp};
use heapless::String;
use serde::Serialize;

#[derive(AtatEnum, Clone)]
#[repr(u8)]
pub enum GnssFixState {
    Stop = 0,
    Start = 1,
}

#[derive(AtatEnum, Clone)]
#[repr(u8)]
pub enum GnssUrcEvents {
    Disable = 0,
    Enable = 1,
}

#[derive(AtatEnum, Clone)]
#[repr(u8)]
pub enum GnssFormatType {
    AT = 0,
    NMEA = 1,
}

#[derive(Clone, Copy)]
#[repr(u16)]
pub enum AtFormatArg {
    Position = 1000,
    Accuracy = 0100,
    Satellites = 0010,
    Orientation = 0001,
    None = 0000,
}

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum NmeaFormatArg {
    Gga = 100000,
    Gsa = 010000,
    Gsv = 001000,
    Gll = 000100,
    Rmc = 000010,
    Vtg = 000001,
    None = 000000,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#GNSSFIX", GnssFixResponse, timeout_ms = 10000)]
pub struct GnssFix {
    pub start_stop: GnssFixState,
    pub event_enable: GnssUrcEvents,
    pub format_type: GnssFormatType,
    pub format_argument: u16,
    period: Option<u32>,
}

impl Default for GnssFix {
    fn default() -> Self {
        Self {
            start_stop: GnssFixState::Stop,
            event_enable: GnssUrcEvents::Enable,
            format_type: GnssFormatType::AT,
            format_argument: 0,
            period: None,
        }
    }
}

impl GnssFix {
    pub fn new(
        start_stop: GnssFix,
        urc_events: GnssUrcEvents,
        format_type: GnssFormatType,
    ) -> Self {
    }
}

#[derive(Clone, AtatResp)]
pub struct GnssFixResponse;
