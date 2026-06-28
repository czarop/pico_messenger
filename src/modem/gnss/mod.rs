use heapless::String;
use thiserror::Error;

pub mod commands;
pub mod state;
pub mod urc;
pub mod speed;

#[cfg(feature = "mock_gnss")]
pub mod mock;

#[derive(Debug, Error, defmt::Format)]
pub enum GnssError {
    #[error("Missing Field {0:?}")]
    MissingField(String<24>),
}
