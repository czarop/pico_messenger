use thiserror::Error;

#[derive(Debug, Error, defmt::Format)]
pub enum ModemError {
    #[error("timeout")]
    Timeout,
    #[error("parse error")]
    ParseError,
    #[error("invalid response")]
    InvalidResponse,
}
