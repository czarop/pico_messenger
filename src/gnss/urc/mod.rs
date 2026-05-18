pub mod init;
pub mod fix;


use atat::atat_derive::AtatUrc;

use crate::gnss::urc::{fix::GnssFixUrc, init::GnssInitUrc};

#[derive(Clone, AtatUrc)]
pub enum GnssUrc {
    #[at_urc("#GNSSINIT")]
    Status(GnssInitUrc),
    #[at_urc("#GNSSFIX")]
    Location(GnssFixUrc),
}