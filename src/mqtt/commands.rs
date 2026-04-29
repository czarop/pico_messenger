use super::responses::*;
use atat::atat_derive::{AtatCmd, AtatResp, AtatUrc};
use heapless::String;
// define your AT commands as Rust structs and then handle
// the "Unsolicited Result Codes" (URCs) that the modem sends
// when it receives a message from the broker

// #[derive(Clone, AtatCmd)]
// #[at_cmd("", NoResponse, timeout_ms = 1000)]
// pub struct AT;

// #[derive(Clone, AtatResp)]
// pub struct MessageWaitingIndication;

/// 4.1 Manufacturer identification +CGMI
///
/// Text string identifying the manufacturer.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMI", ManufacturerId)]
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
