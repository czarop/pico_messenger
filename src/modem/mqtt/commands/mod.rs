use atat::atat_derive::{AtatEnum, AtatResp};

pub mod config;
pub mod connect;
pub mod publish;
pub mod socket;
pub mod subscribe;
pub mod pdn;
pub mod reset;
pub mod clock;
pub mod cereg;
pub mod cesq;
pub mod netdiag;

#[derive(Clone, AtatResp)]
pub struct OkResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, AtatEnum)]
pub enum MqttQos {
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AtatEnum)]
pub enum MqttRetainFlag {
    NotRetained = 0,
    Retained = 1,
}