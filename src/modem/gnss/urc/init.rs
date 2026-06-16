use heapless::String;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize, defmt::Format)]
pub struct GnssInitUrcRaw {
    // pub status: u8,
    // pub nbiot_delay: Option<u32>,
    pub body: String<32>,
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

// impl From<GnssInitUrcRaw> for GnssInitUrc {
//     fn from(raw: GnssInitUrcRaw) -> Self {
//         match raw.status {
//             0 => Self::NotStarted,
//             1 => Self::Starting,
//             2 => Self::Ready {
//                 nbiot_delay: raw.nbiot_delay,
//             },
//             3 => Self::DownloadingSupl,
//             4 => Self::SuplFailed,
//             5 => Self::SystemFailure,
//             6 => Self::StartupDelayed {
//                 nbiot_delay: raw.nbiot_delay,
//             },
//             _ => unreachable!("Invalid status value in GnssInitUrcRaw"),
//         }
//     }
// }
impl GnssInitUrcRaw {
    pub fn parse(&self) -> GnssInitUrc {
        let s = self.body.as_str().trim();
        let mut parts = s.splitn(2, ',');
        let status: u8 = parts.next()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(255);
        let nbiot_delay: Option<u32> = parts.next()
            .and_then(|s| s.trim().parse().ok());

        match status {
            0 => GnssInitUrc::NotStarted,
            1 => GnssInitUrc::Starting,
            2 => GnssInitUrc::Ready { nbiot_delay },
            3 => GnssInitUrc::DownloadingSupl,
            4 => GnssInitUrc::SuplFailed,
            5 => GnssInitUrc::SystemFailure,
            6 => GnssInitUrc::StartupDelayed { nbiot_delay },
            _ => GnssInitUrc::SystemFailure,
        }
    }
}
