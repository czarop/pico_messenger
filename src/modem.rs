use atat::atat_derive::AtatUrc;

use crate::gnss::urc::location::GNSSInitMessage;

#[derive(Clone, AtatUrc)]
pub enum ModemUrc {
    #[at_urc("#GNSSINIT")]
    GnssInit(GNSSInitMessage),
}
