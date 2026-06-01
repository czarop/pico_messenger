use core::ops::Deref;

use crate::gnss::urc::fix::GnssLocation;

#[derive(Clone, Debug, PartialEq)]
pub enum GNSSState {
    Off,
    Acquiring,
    Fix(GnssLocation),
    FixLost,
    Error,
}

// impl Deref for GNSSState {
//     type Target = Option<GnssLocation>;

//     fn deref(&self) -> &Self::Target {
//         &self.latest_location
//     }
// }
