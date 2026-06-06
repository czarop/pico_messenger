use atat::atat_derive::{AtatCmd, AtatEnum, AtatResp};

use crate::modem::UpdateIntervalSecs;

#[derive(AtatEnum, Clone)]
#[repr(u8)]
enum GnssFixState {
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
enum GnssFormatType {
    AT = 0,
    NMEA = 1,
}

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum AtFormatArg {
    Position = 1000,
    Accuracy = 100,
    // Satellites = 10,
    // Orientation = 1,
}

// #[derive(Clone, Copy)]
// #[repr(u32)]
// pub enum NmeaFormatArg {
//     Gga = 100000,
//     Gsa = 10000,
//     Gsv = 1000,
//     Gll = 100,
//     Rmc = 10,
//     Vtg = 1,
// }

#[derive(Clone, AtatCmd)]
#[at_cmd("#GNSSFIX", GnssFixResponse, timeout_ms = 10000)]
pub struct GnssFix {
    start_stop: GnssFixState,
    event_enable: GnssUrcEvents,
    format_type: GnssFormatType,
    format_argument: u32,
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
    pub fn start_with_at_report(
        urc_events: GnssUrcEvents,
        report_output: heapless::Vec<AtFormatArg, 4>,
        update_interval_secs: Option<UpdateIntervalSecs>,
    ) -> Self {
        let format_argument: u32 = report_output.iter().map(|&v| v as u32).sum::<u32>();

        Self {
            start_stop: GnssFixState::Start,
            event_enable: urc_events,
            format_type: GnssFormatType::AT,
            format_argument,
            period: update_interval_secs,
        }
    }

    pub fn start_with_at_report_defaults(
        update_interval_secs: Option<UpdateIntervalSecs>,
    ) -> Self {
        Self::start_with_at_report(
            GnssUrcEvents::Enable,
            heapless::Vec::from_slice(&[AtFormatArg::Position, AtFormatArg::Accuracy]).unwrap(),
            update_interval_secs,
        )
    }

    // pub fn start_with_nmea_report(
    //     urc_events: GnssUrcEvents,
    //     report_output: heapless::Vec<NmeaFormatArg, 6>,
    //     update_interval_secs: Option<u32>
    // ) -> Self {

    //     let format_argument = report_output.iter().map(|&v| v as u32).sum();

    //     Self {
    //         start_stop: GnssFixState::Start,
    //         event_enable: urc_events,
    //         format_type: GnssFormatType::NMEA,
    //         format_argument,
    //         period: update_interval_secs
    //     }
    // }

    pub fn stop_updates() -> Self {
        Self {
            start_stop: GnssFixState::Stop,
            ..Default::default()
        }
    }
}

#[derive(Clone, AtatResp)]
pub struct GnssFixResponse;
