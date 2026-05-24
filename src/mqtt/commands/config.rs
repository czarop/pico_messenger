use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::mqtt::commands::OkResponse;

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTCFG", OkResponse, timeout_ms = 1000)]
pub struct MqttConfig {
    client_name: Option<String<25>>,
    connection_timeout: Option<u16>,
    protocol_timeout: Option<u16>,
    publish_retry: Option<u8>,
    keep_alive_pub_msg: Option<u16>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            client_name: Some(String::try_from("pico-messenger").unwrap()),
            connection_timeout: Some(20),
            protocol_timeout: Some(20),
            publish_retry: Some(10),
            keep_alive_pub_msg: Some(60),
        }
    }
}
