use crate::modem::mqtt::commands::OkResponse;
use atat::atat_derive::{AtatCmd, AtatResp};

// AT#RINGPIN=<enable>,<gpio>,<level>,<delay>
#[derive(Clone, AtatCmd)]
#[at_cmd("#RINGPIN", OkResponse, timeout_ms = 1000)]
pub struct SetRingPin {
    #[at_arg(position = 0)]
    pub enable: u8,
    #[at_arg(position = 1)]
    pub gpio: u8,
    #[at_arg(position = 2)]
    pub level: u8,
    #[at_arg(position = 3)]
    pub delay_ms: u16,
}

// AT#RINGPIN?
#[derive(Clone, AtatCmd)]
#[at_cmd("#RINGPIN?", RingPinConfig, timeout_ms = 1000)]
pub struct GetRingPin;

#[derive(Clone, AtatResp)]
pub struct RingPinConfig {
    pub enable: u8,
    pub gpio: u8,
    pub level: u8,
    pub delay_ms: u16,
}
