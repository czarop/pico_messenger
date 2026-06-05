use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::modem::mqtt::commands::{MqttQos, OkResponse};

#[derive(Clone, AtatCmd, PartialEq)]
#[at_cmd("#MQTTSUB", OkResponse, timeout_ms = 10000)]
pub struct MqttSubscribe {
    pub topic: String<50>, // supports wildcards: + = single level, # = multi-level
    pub qos: MqttQos,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTUNSUB", OkResponse, timeout_ms = 10000)]
pub struct MqttUnsubscribe {
    pub topic: String<50>,
}
