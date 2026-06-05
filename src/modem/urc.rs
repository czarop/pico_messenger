use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    modem::gnss::urc::{fix::GnssFixUrcRaw, init::GnssInitUrcRaw},
    modem::mqtt::urc::{
        ip_stack::{CgevEvent, CgevUrc},
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

// react to messages
// #[embassy_executor::task]
// pub async fn modem_task(
//     mut sub: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
// ) -> ! {
//     loop {
//         let urc = sub.next_message_pure().await;
//         match urc {
//             ModemUrc::MqttReceived(mqtt_recv_urc) => todo!(),
//             _ => {}
//         }
//     }
// }
