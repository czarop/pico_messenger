use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::modem::mqtt::commands::OkResponse;

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
            connection_timeout: Some(70),
            // Window the modem's MQTT stack waits for protocol responses
            // (PUBACK for a QoS1 publish). On this network the PUBACK lands
            // right at ~20s, so the previous value of 20 was a coin flip every
            // publish: the modem declared failure (+CME ERROR: 2215) while the
            // PUBLISH itself had already reached the broker. 60s clears the
            // observed latency with headroom.
            protocol_timeout: Some(60),
            // No modem-side retries. At QoS1 each retry is a duplicate PUBLISH
            // on the wire, and retries multiply the modem's total response time
            // past the host's timeouts. Send once; a genuine drop is a gap, not
            // a corruption (same policy as publish_once).
            publish_retry: Some(0),
            // Keep-alive (PINGREQ interval). Was 240, which is longer than a
            // test cadence -- a session orphaned by an IP change (Soracom
            // re-NAT across a PDP cycle) then lingered broker-side for ~360s
            // (1.5x), long enough to collide with the next cycle's reconnect as
            // the same client-id ("already connected, closing old connection").
            // 60s reaps a dead session well before any cycle reconnects. No
            // battery cost: the session lives ~7s per cycle, far short of one
            // PINGREQ interval, so a ping is never actually sent.
            keep_alive_pub_msg: Some(60),
        }
    }
}

impl MqttConfig {
    pub fn new(
        client_name: Option<String<25>>,
        connection_timeout: Option<u16>,
        protocol_timeout: Option<u16>,
        publish_retry: Option<u8>,
        keep_alive_pub_msg: Option<u16>,
    ) -> Self {
        Self {
            client_name,
            connection_timeout,
            protocol_timeout,
            publish_retry,
            keep_alive_pub_msg,
        }
    }
}
