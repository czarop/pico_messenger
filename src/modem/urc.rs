use crate::{
    modem::gnss::urc::{fix::GnssFixUrcRaw, init::GnssInitUrcRaw},
    modem::mqtt::urc::{
        ip_stack::CgevUrc,
        receive::MqttRecvUrc,
        socket::SocketClosedUrc,
    },
};
use atat::atat_derive::AtatUrc;

use {defmt_rtt as _, panic_probe as _};

#[derive(Clone, AtatUrc, defmt::Format)]
pub enum ModemUrc {
    #[at_urc("#GNSSINIT")]
    GnssStatus(GnssInitUrcRaw),
    #[at_urc("#GNSSFIX")]
    Location(GnssFixUrcRaw),
    #[at_urc("+CGEV")]
    IPStackUpdate(CgevUrc),
    #[at_urc("#SOCKETCLOSED")]
    SocketClosed(SocketClosedUrc),
    #[at_urc("#MQTTRECV")]
    MqttReceived(MqttRecvUrc),

    // Network-emitted URCs. These arrive unsolicited (registration status and
    // network-time updates) and can land *during* a long command like
    // MQTTCONNECT. They carry no action for us, but they MUST be registered:
    // an unregistered URC is not recognised by the digester and gets folded
    // into the pending command's response buffer, corrupting the parse (this
    // caused false MQTTCONNECT ParseErrors and the CGPADDR/CCLK misreads).
    // Capture the body in a generously-sized String so a longer PSM-form
    // +CEREG line can't overflow and fall back to polluting the response.
    #[at_urc("+CEREG")]
    Cereg(heapless::String<96>),
    #[at_urc("+CTZEU")]
    CtzEu(heapless::String<64>),
    // Fires whenever a PDP context is established (every connect/reboot cycle).
    // Seen in logs bleeding into the SOCKETCREATE? response; would break a
    // command with a strict OkResponse (e.g. MQTTCONNECT) the same way +CEREG
    // did if it lands mid-command. Body: "<context_id>,<ip_mode>,<ip_status>".
    #[at_urc("#IPCFG")]
    IpCfg(heapless::String<32>),

    #[at_urc("#SYSSTART")]
    SysStart,
    #[at_urc("#REBOOT_RESET")]
    RebootReset,
    #[at_urc("#REBOOT_HOST")]
    RebootHost,
    #[at_urc("#REBOOT_WD")]
    RebootWD(heapless::String<4>),
}