use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GnssInitUrcRaw {
    pub status: u8,
    pub nbiot_delay: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GnssInitUrc {
    NotStarted,
    Starting,
    Ready { nbiot_delay: Option<u32> },
    DownloadingSupl,
    SuplFailed,
    SystemFailure,
    StartupDelayed { nbiot_delay: Option<u32> },
}

impl GnssInitUrc {
    pub fn from_raw(raw: GnssInitUrcRaw) -> Option<Self> {
        match raw.status {
            0 => Some(Self::NotStarted),
            1 => Some(Self::Starting),
            2 => Some(Self::Ready {
                nbiot_delay: raw.nbiot_delay,
            }),
            3 => Some(Self::DownloadingSupl),
            4 => Some(Self::SuplFailed),
            5 => Some(Self::SystemFailure),
            6 => Some(Self::StartupDelayed {
                nbiot_delay: raw.nbiot_delay,
            }),
            _ => None,
        }
    }
}
