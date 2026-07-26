use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::modem::mqtt::commands::{MqttQos, MqttRetainFlag, OkResponse};

#[derive(Clone, AtatCmd)]
// Timeout ladder: the modem's own PUBACK window (#MQTTCFG protocol_timeout,
// 60s) must expire FIRST so we read the modem's real answer; this atat timeout
// sits above it at 70s. (History: at 20000 the atat timeout fired ~130ms before
// the modem's ~20s answer, calling a delivered publish a timeout and retrying
// into duplicates. Do not let these two race again.)
#[at_cmd("#MQTTPUB", OkResponse, timeout_ms = 70000)]
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
            // No retries: pairs with publish_retry=0 in #MQTTCFG. The manual
            // doesn't say which of the two wins, so both are zero.
            retry_number: 0,
            // QoS 0, deliberately. At QoS 1 the modem waits for the broker's
            // PUBACK, which on this network takes ~20s -- and hardware showed
            // that wait is capped by #MQTTPUB's fixed 20s max response time
            // REGARDLESS of #MQTTCFG protocol_timeout (set to 60, ack still
            // failed at 20.1s). Result: a guaranteed 20s high-current stall and
            // a spurious +CME 2215 every cycle, while the PUBLISH itself had
            // long since reached the broker. Delivery policy is already
            // best-effort (a drop is a gap, not a corruption), so QoS 1 bought
            // nothing. At QoS 0 the modem returns OK on send.
            qos: MqttQos::AtMostOnce,
            retain_flag: MqttRetainFlag::NotRetained,
        }
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}