use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize, defmt::Format)]
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

impl From<GnssInitUrcRaw> for GnssInitUrc {
    fn from(raw: GnssInitUrcRaw) -> Self {
        match raw.status {
            0 => Self::NotStarted,
            1 => Self::Starting,
            2 => Self::Ready {
                nbiot_delay: raw.nbiot_delay,
            },
            3 => Self::DownloadingSupl,
            4 => Self::SuplFailed,
            5 => Self::SystemFailure,
            6 => Self::StartupDelayed {
                nbiot_delay: raw.nbiot_delay,
            },
            _ => unreachable!("Invalid status value in GnssInitUrcRaw"),
        }
    }
}
