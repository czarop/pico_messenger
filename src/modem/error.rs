use thiserror::Error;

use crate::gnss::GnssError;

#[derive(Debug, Error, defmt::Format)]
pub enum ModemError {
    #[error("timeout")]
    Timeout,
    #[error("parse error")]
    ParseError,
    #[error("invalid response")]
    InvalidResponse,
    #[error("modem error: {0:?}")]
    Modem(atat::Error),
    #[error("GNSS error: {0:?}")]
    GnssError(GnssError),
}

impl From<atat::Error> for ModemError {
    fn from(e: atat::Error) -> Self {
        match e {
            atat::Error::Timeout => Self::Timeout,
            atat::Error::Parse => Self::ParseError,
            atat::Error::InvalidResponse => Self::InvalidResponse,
            other => Self::Modem(other),
        }
    }
}
