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
    security_profile_id: Option<u8>, // Some(1) = use TLS security profile 1
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
            security_profile_id: Some(0)
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
            security_profile_id: Some(0)
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

// --- Read-form query: AT#SOCKETCREATE? ---------------------------------------
//
// Enumerates every socket the modem currently believes is open. This matters
// because socket IDs *increment* with each create (AT manual: "starts with 0
// and is incremented each time a new socket is created") and that counter — plus
// the single TCP slot — lives on the modem and survives an RP2350 reflash. Only a
// modem power-cycle/reboot resets it. So after several reflash-orphan cycles the
// lingering socket can have an ID greater than 2, which blind SOCKETCLOSE on 0..2
// will never reach. This query returns the real IDs to close.
//
// The struct has NO fields, so atat serialises it as the bare read form
// `AT#SOCKETCREATE?` (the `=` separator is only emitted when fields exist).
//
// Read response (per socket, zero or more lines):
//   #SOCKETCREATE: <context_id>,<socket_id>,<ip_version>,<socket_type>,
//                  <local_port>,<send_timeout>,<receive_timeout>,
//                  <frame_received_urc>,<security_profile_id>
// If no socket is open, the body is empty (just OK).
//
// The fixed multi-field-per-line, variable-line-count shape doesn't fit atat's
// derive deserialiser, so we take the raw bytes via `parse =` and scan them
// ourselves, mirroring the project's raw-String parsing pattern.
#[derive(Clone, AtatCmd)]
#[at_cmd("#SOCKETCREATE?", OpenSockets, parse = parse_open_sockets, timeout_ms = 2000)]
pub struct SocketQuery {}

#[derive(Clone, AtatResp)]
pub struct OpenSockets {
    /// Socket IDs the modem reports as currently open (max 3: 1 TCP + 2 UDP).
    pub ids: heapless::Vec<u8, 3>,
}

/// Scan the raw `AT#SOCKETCREATE?` response for the socket IDs of every open
/// socket. Robust to whether atat hands us the `#SOCKETCREATE:` prefix or a
/// bare CSV line, and silently skips anything unparseable (an empty result
/// simply means "no sockets open"). Never fails: a malformed/empty body yields
/// an empty list rather than an error, so the caller can act on the distinction
/// between "nothing open" and "command errored".
pub fn parse_open_sockets(resp: &[u8]) -> Result<OpenSockets, ()> {
    let mut ids: heapless::Vec<u8, 3> = heapless::Vec::new();
    let text = core::str::from_utf8(resp).map_err(|_| ())?;

    for line in text.split(&['\r', '\n'][..]) {
        let line = line.trim();
        // The socket CSV is `<context_id>,<socket_id>,...`. Accept it with or
        // without the response prefix.
        let csv = if let Some(rest) = line.strip_prefix("#SOCKETCREATE:") {
            rest.trim()
        } else if line.starts_with("5,") {
            line
        } else {
            continue;
        };

        let mut fields = csv.split(',');
        let _context_id = fields.next(); // field 0
        if let Some(socket_id) = fields.next() {
            // field 1
            if let Ok(id) = socket_id.trim().parse::<u8>() {
                let _ = ids.push(id);
            }
        }
    }

    Ok(OpenSockets { ids })
}