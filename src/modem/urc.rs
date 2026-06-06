
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

#[derive(Clone, AtatUrc)]
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
}

