use atat::atat_derive::AtatCmd;
use heapless::String;

use crate::modem::mqtt::commands::netdiag::{raw_dump, RawAtResp};
use crate::modem::mqtt::commands::OkResponse;

#[derive(Clone)]
pub struct MqttConnectStub {
    context_id: u8,             // Always 5 (default PDP context)
    broker_address: String<50>, // IPv4 address of MQTT broker
    broker_port: u16,           // 1883 = plain MQTT, 8883 = MQTT over TLS
    username: String<25>,       // Broker auth username (max 25 chars)
    passwd: String<50>,         // Broker auth password (max 50 chars)
    will_topic: String<50>,     // Topic broker publishes to on unclean disconnect (max 50 chars)
    will_message: String<50>,   // Payload published to will_topic (max 50 chars)
    will_qos: u8,               // 0 = at most once, 1 = at least once, 2 = exactly once
    will_retain_flag: u8,       // 0 = not retained, 1 = retained (new subscribers see last state)
}

impl MqttConnectStub {
    pub fn new(
        broker_address: String<50>,
        broker_port: u16,
        username: String<25>,
        passwd: String<50>,
        will_topic: String<50>,
        will_message: String<50>,
    ) -> Self {
        Self {
            context_id: 5,
            broker_address,
            broker_port,
            username,
            passwd,
            will_topic,
            will_message,
            will_qos: 1,
            will_retain_flag: 1,
        }
    }
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#MQTTCONNECT", OkResponse, timeout_ms = 70000)]
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

impl MqttConnect {
    pub fn new(
        socket_id: u8,
        broker_address: String<50>,
        broker_port: u16,
        username: String<25>,
        passwd: String<50>,
        will_topic: String<50>,
        will_message: String<50>,
    ) -> Self {
        Self {
            context_id: 5,
            socket_id,
            broker_address,
            broker_port,
            username,
            passwd,
            will_topic,
            will_message,
            will_qos: 1,
            will_retain_flag: 1,
        }
    }

    pub fn from_stub(socket_id: u8, stub: MqttConnectStub) -> Self {
        Self {
            context_id: stub.context_id,
            socket_id,
            broker_address: stub.broker_address,
            broker_port: stub.broker_port,
            username: stub.username,
            passwd: stub.passwd,
            will_topic: stub.will_topic,
            will_message: stub.will_message,
            will_qos: stub.will_qos,
            will_retain_flag: stub.will_retain_flag,
        }
    }
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
// Manual gives #MQTTDISC a 20s max response time. It is a GRACEFUL disconnect
// (a broker round-trip), so on a silently-dead TLS link it hangs until it times
// out -- and hardware shows it hangs even when #MQTTCONNECT? still reports
// connected (a stale 1), so we can't gate it away. Previously 40s, which meant
// a wedge burned 40s before the teardown reset could reclaim it. Cut to 22s
// (the modem's own 20s ceiling + ~2s UART/processing slack): a legitimately
// slow-but-valid disconnect still completes, but a true hang is reclaimed ~18s
// sooner, and this now fires before the 30s "did not reach Down" watchdog.
#[at_cmd("#MQTTDISC", OkResponse, timeout_ms = 22000)]
pub struct MqttDisconnect;

/// `AT#MQTTCONNECT?` -- read the stack's own view of the connection:
/// `#MQTTCONNECT: 1,<ctx>,<socket>,<broker>,<port>,<user>` when it believes it
/// is connected, or `#MQTTCONNECT: 0` when not. This is a LOCAL state read (the
/// modem reporting its own belief), not a broker round-trip, so it returns fast
/// and -- unlike the graceful `#MQTTDISC` -- cannot hang on a dead TLS link.
///
/// Captured raw for now: we only want to observe what it reports right before a
/// teardown, to confirm it reliably says `0` on a dead session before we gate
/// DISC on it. `20000` matches the manual's max response time as a safety net;
/// in practice a local read is near-instant.
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("#MQTTCONNECT?", RawAtResp, parse = raw_dump, timeout_ms = 20000)]
pub struct MqttConnectQuery;