use atat::atat_derive::{AtatEnum, AtatUrc};

#[derive(Clone, AtatUrc)]
pub enum GNSSUrc {
    #[at_urc("#GNSSINIT")]
    Status(GNSSInitMessage),
}

#[derive(Clone, AtatEnum)]
#[repr(u8)]
pub enum GNSSInitMessage {
    NotStarted = 0,
    Starting = 1,
    Ready = 2,
    DownloadingSupl = 3,
    SuplFailed = 4,
    SystemFailure = 5,
    StartupDelayed = 6,
}
