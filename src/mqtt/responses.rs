//! Responses for General Commands
use atat::atat_derive::{AtatResp, AtatUrc};
use atat::heapless::String;

#[derive(Clone, AtatResp)]
pub struct NoResponse;

/// 4.1 Manufacturer identification
/// Text string identifying the manufacturer.
#[derive(Clone, Debug, AtatResp)]
pub struct ManufacturerId {
    pub id: String<64>,
}

/// Model identification
/// Text string identifying the manufacturer.
#[derive(Clone, Debug, AtatResp)]
pub struct ModelId {
    pub id: String<64>,
}

/// Software version identification
/// Read a text string that identifies the software version of the module.
#[derive(Clone, Debug, AtatResp)]
pub struct SoftwareVersion {
    pub id: String<64>,
}

/// 7.11 Wi-Fi Access point station list +UWAPSTALIST
#[derive(Clone, AtatResp)]
pub struct WifiMac {
    pub mac_addr: atat::heapless_bytes::Bytes<12>,
}

//////////////////////////////////////////////////



// the response enum
#[derive(Clone, AtatResp)]
pub struct MqttResponse {
    #[at_arg(position = 0)]
    pub topic: String<64>,
    #[at_arg(position = 1)]
    pub len: usize,
    #[at_arg(position = 2)]
    pub message: String<128>,
}
