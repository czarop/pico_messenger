use crate::gnss::urc::{
    fix::{GnssFixUrc, GnssLocation, GnssValidity},
    init::GnssInitUrc,
};

#[derive(Clone, Debug, PartialEq)]
pub enum GNSSState {
    Off,
    Initialising(GnssInitState),
    Ready,
    Acquiring,
    Fix(GnssLocation),
    Error(GnssInitUrc),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GnssInitState {
    Starting,
    DownloadingSupl,
    Delayed(Option<u32>),
}

impl From<GnssInitUrc> for GNSSState {
    fn from(init: GnssInitUrc) -> Self {
        match init {
            GnssInitUrc::NotStarted => Self::Off,
            GnssInitUrc::Starting => Self::Initialising(GnssInitState::Starting),
            GnssInitUrc::Ready { .. } => Self::Ready,
            GnssInitUrc::DownloadingSupl => Self::Initialising(GnssInitState::DownloadingSupl),
            GnssInitUrc::SuplFailed => Self::Error(init),
            GnssInitUrc::SystemFailure => Self::Error(init),
            GnssInitUrc::StartupDelayed { nbiot_delay } => {
                Self::Initialising(GnssInitState::Delayed(nbiot_delay))
            }
        }
    }
}

impl From<GnssFixUrc> for GNSSState {
    fn from(value: GnssFixUrc) -> Self {
        match value {
            GnssFixUrc::Searching(gnss_validity) => {
                if !gnss_validity.is_valid() {
                    let reasons = heapless::Vec::<GnssInvalidReason, 8>::from(&gnss_validity);
                    for item in reasons {
                        defmt::info!("GNSS invalid reason: {:?}", item);
                    }
                }
                Self::Acquiring
            }
            GnssFixUrc::Fix(gnss_location) => Self::Fix(gnss_location),
        }
    }
}

#[derive(Clone, Debug, PartialEq, defmt::Format)]
pub enum GnssInvalidReason {
    NotEnoughSatellites,
    NotEnoughEphemerides,
    PositionImpossible,
    DopTooHigh,
    SsrTooHigh,
    AccuracyTooLow,
    AltitudeOutOfBounds,
    IntegrityCheckFailed,
}

impl From<&GnssValidity> for heapless::Vec<GnssInvalidReason, 8> {
    fn from(v: &GnssValidity) -> Self {
        let mut reasons = heapless::Vec::new();
        if v.not_enough_satellites() {
            reasons.push(GnssInvalidReason::NotEnoughSatellites).ok();
        }
        if v.not_enough_ephemerides() {
            reasons.push(GnssInvalidReason::NotEnoughEphemerides).ok();
        }
        if v.position_impossible() {
            reasons.push(GnssInvalidReason::PositionImpossible).ok();
        }
        if v.dop_too_high() {
            reasons.push(GnssInvalidReason::DopTooHigh).ok();
        }
        if v.ssr_too_high() {
            reasons.push(GnssInvalidReason::SsrTooHigh).ok();
        }
        if v.accuracy_too_low() {
            reasons.push(GnssInvalidReason::AccuracyTooLow).ok();
        }
        if v.altitude_out_of_bounds() {
            reasons.push(GnssInvalidReason::AltitudeOutOfBounds).ok();
        }
        if v.integrity_check_failed() {
            reasons.push(GnssInvalidReason::IntegrityCheckFailed).ok();
        }
        reasons
    }
}
