use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::mqtt::commands::{MqttQos, OkResponse};

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTSUB", OkResponse, timeout_ms = 10000)]
pub struct MqttSubscribe {
    topic: String<50>, // supports wildcards: + = single level, # = multi-level
    qos: MqttQos,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTUNSUB", OkResponse, timeout_ms = 10000)]
pub struct MqttUnsubscribe {
    topic: String<50>,
}
