use atat::atat_derive::{AtatCmd, AtatResp};
use heapless::String;

use crate::modem::mqtt::commands::OkResponse;

#[derive(Clone, AtatCmd)]
#[at_cmd("#SOCKETCREATE", SocketCreateResponse, timeout_ms = 2000)]
pub struct SocketCreate {
    context_id: u8,
    ip_version: u8,
    socket_type: String<4>, // "TCP"
    local_port: u16,        // 0 = random
    send_timeout: u16,
    receive_timeout: u16,
    frame_received_urc: u8, // 0 = disabled
}

impl Default for SocketCreate {
    fn default() -> Self {
        Self {
            context_id: 5, // this is always 5 for this modem
            ip_version: 0, // IPv4
            socket_type: String::try_from("TCP").unwrap(),
            local_port: 0, // random port assigned by stack
            send_timeout: 10,
            receive_timeout: 10,
            frame_received_urc: 0, // disabled, MQTT handles its own URCs
        }
    }
}

impl SocketCreate {
    pub fn context_id(&self) -> u8 {
        self.context_id
    }
    pub fn new(send_timeout: u16, receive_timeout: u16) -> Self {
        Self {
            context_id: 5, // this is always 5 for this modem
            ip_version: 0, // IPv4
            socket_type: String::try_from("TCP").unwrap(),
            local_port: 0, // random port assigned by stack
            send_timeout,
            receive_timeout,
            frame_received_urc: 0, // disabled, MQTT handles its own URCs
        }
    }
}

// Response — captures the returned socket_id
#[derive(Clone, AtatResp)]
pub struct SocketCreateResponse {
    pub socket_id: u8,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#SOCKETCLOSE", OkResponse, timeout_ms = 60000)]
pub struct SocketClose {
    pub context_id: u8,
    pub socket_id: u8,
}
