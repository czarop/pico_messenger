use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    gnss::urc::{fix::GnssFixUrcRaw, init::GnssInitUrcRaw},
    mqtt::urc::{
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
#[embassy_executor::task]
pub async fn modem_urc_task(
    mut sub: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) -> ! {
    loop {
        let urc = sub.next_message_pure().await;
        match urc {
            ModemUrc::GnssStatus(gnss_init_urc_raw) => todo!(),
            ModemUrc::Location(gnss_fix_urc_raw) => todo!(),
            ModemUrc::IPStackUpdate(ip_stack_state) => {
                match ip_stack_state.event() {
                    CgevEvent::MePdnAct(5) => { /* signal network ready */ }
                    CgevEvent::MePdnDeact(_)
                    | CgevEvent::NwPdnDeact(_)
                    | CgevEvent::MeDetach
                    | CgevEvent::NwDetach => { /* signal network down */ }
                    _ => {}
                }
            }
            ModemUrc::SocketClosed(socket_closed_urc) => {
                // Handle socket closed URC
                todo!()
            }
            ModemUrc::MqttReceived(mqtt_recv_urc) => todo!(),
        }
    }
}
