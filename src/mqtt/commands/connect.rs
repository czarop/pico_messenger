use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::mqtt::commands::OkResponse;

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTCONNECT", OkResponse, timeout_ms = 20000)]
pub struct MqttConnect {
    context_id: u8,             // Always 5 (default PDP context)
    socket_id: u8,              // Returned from SocketCreate
    broker_address: String<50>, // IPv4 address of MQTT broker
    broker_port: u16,           // 1883 = plain MQTT, 8883 = MQTT over TLS
    username: String<25>,       // Broker auth username (max 25 chars)
    passwd: String<50>,         // Broker auth password (max 50 chars)
    will_topic: String<50>,     // Topic broker publishes to on unclean disconnect (max 50 chars)
    will_message: String<50>,   // Payload published to will_topic (max 50 chars)
    will_qos: u8,               // 0 = at most once, 1 = at least once, 2 = exactly once
    will_retain_flag: u8,       // 0 = not retained, 1 = retained (new subscribers see last state)
}

impl Default for MqttConnect {
    fn default() -> Self {
        Self {
            context_id: 5,
            socket_id: 0,
            broker_address: String::try_from("0.0.0.0").unwrap(),
            broker_port: 1883,
            username: String::try_from("").unwrap(),
            passwd: String::try_from("").unwrap(),
            will_topic: String::try_from("pico-messenger/status").unwrap(),
            will_message: String::try_from("offline").unwrap(),
            will_qos: 1,
            will_retain_flag: 1,
        }
    }
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTDISC", OkResponse, timeout_ms = 20000)]
pub struct MqttDisconnect;
