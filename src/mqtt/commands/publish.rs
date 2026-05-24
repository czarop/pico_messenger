use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::mqtt::commands::{MqttQos, MqttRetainFlag, OkResponse};

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTPUB", OkResponse, timeout_ms = 20000)]
pub struct MqttPublish {
    topic: String<50>,           // Publish topic, no wildcards
    message: String<50>,         // Payload, max 50 chars
    retry_number: u8,            // 0-20 retransmit attempts on failure
    qos: MqttQos,                // 0 = at most once, 1 = at least once, 2 = exactly once
    retain_flag: MqttRetainFlag, // 0 = not retained, 1 = retained by broker for new subscribers
}

impl MqttPublish {
    pub fn new(topic: String<50>, message: String<50>) -> Self {
        Self {
            topic,
            message,
            retry_number: 3,
            qos: MqttQos::AtLeastOnce,
            retain_flag: MqttRetainFlag::NotRetained,
        }
    }
}
