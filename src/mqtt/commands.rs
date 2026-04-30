use super::responses::*;
use atat::atat_derive::{AtatCmd, AtatResp};

// define your AT commands as Rust structs and link them to a response type
// how you ask the modem to do things

/// example
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMI", ExampleResponse, timeout_ms = 1000)]
pub struct ExampleWithFields{
    #[at_arg(position = 0)]
    pub arg1: u8,
    #[at_arg(position = 1, len = 64)]
    pub arg2: heapless::String<64>,
}

#[derive(Clone, AtatResp)]
pub struct ExampleResponse {
    #[at_arg(position = 0)]
    pub arg1: u8,
    #[at_arg(position = 1, len = 64)]
    pub arg2: heapless::String<64>,
}


/// 4.1 Manufacturer identification +CGMI
///
/// Text string identifying the manufacturer.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMI", ManufacturerId, timeout_ms = 1000)]
pub struct GetManufacturerId;

/// Model identification +CGMM
///
/// Read a text string that identifies the device model.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMM", ModelId)]
pub struct GetModelId;

/// Software version identification +CGMR
///
/// Read a text string that identifies the software version of the module
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMR", SoftwareVersion)]
pub struct GetSoftwareVersion;

/// 7.12 Wi-Fi MAC address +UWAPMACADDR
///
/// Lists the currently used MAC address.
#[derive(Clone, AtatCmd)]
#[at_cmd("+UWAPMACADDR", WifiMac)]
pub struct GetWifiMac;
