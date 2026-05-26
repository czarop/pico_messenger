use core::ops::Deref;

use crate::gnss::urc::fix::GnssLocation;

#[derive(Clone, Debug, PartialEq)]
pub struct GNSSState {
    pub latest_location: Option<GnssLocation>,
}

impl Deref for GNSSState {
    type Target = Option<GnssLocation>;

    fn deref(&self) -> &Self::Target {
        &self.latest_location
    }
}

