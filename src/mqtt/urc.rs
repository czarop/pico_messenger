use atat::atat_derive::{AtatResp, AtatUrc};
use heapless::String;
// the "Unsolicited Result Codes" (URCs) that the modem sends
// when it receives a message from the broker
// from someone sending you a message, or the modem sending you a message eg
#[derive(Clone, AtatUrc)]
pub enum Urc {
    #[at_urc("+UMQTT")]
    MessageWaitingIndication(MqttMessage),
    
    #[at_urc("+CREG")]
    NetworkRegistration(NetworkRegistration),
    
    #[at_urc("+CMT")]
    IncomingSms(IncomingSms),
}

#[derive(Clone, Debug, AtatResp)]
pub struct NetworkRegistration {
    pub stat: u8,
}

#[derive(Clone, Debug, AtatResp)]
pub struct IncomingSms {
    #[at_arg(position = 0, len = 20)]
    pub number: String<20>,
    #[at_arg(position = 1, len = 160)]
    pub text: String<160>,
}

// the response enum
#[derive(Clone, AtatResp)]
pub struct MqttMessage {
    #[at_arg(position = 0, len = 64)]
    pub topic: String<64>,
    #[at_arg(position = 1)]
    pub len: usize,
    #[at_arg(position = 2, len = 128)]
    pub message: String<128>,
}