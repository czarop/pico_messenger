use atat::atat_derive::AtatEnum;

#[derive(Clone, AtatEnum)]
#[repr(u8)]
pub enum GnssInitUrc {
    NotStarted = 0,
    Starting = 1,
    Ready = 2,
    DownloadingSupl = 3,
    SuplFailed = 4,
    SystemFailure = 5,
    StartupDelayed = 6,
}
