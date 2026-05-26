use super::setup::INGRESS_BUF_SIZE;
use crate::gnss::commands::{deinit::GnssDeinit, fix::GnssFix, init::GnssInit};
use crate::modem::communication::COMMAND_CHANNEL;
use crate::mqtt::commands::{config, connect, publish, socket, subscribe};
use atat::asynch::AtatClient;
use atat::asynch::Client;
use embassy_rp::uart;


pub enum ModemCommand {
    GnssInit(GnssInit),
    GnssDeinit,
    GetLocation(GnssFix),
    MqttConfig(config::MqttConfig),
    SocketCreate(socket::SocketCreate),
    SocketClose(socket::SocketClose),
    MqttConnect(connect::MqttConnect),
    MqttDisconnect(connect::MqttDisconnect),
    MqttPublish(publish::MqttPublish),
    MqttSubscribe(subscribe::MqttSubscribe),
    MqttUnsubscribe(subscribe::MqttUnsubscribe),
}



#[embassy_executor::task]
pub async fn modem_command_task(
    client: &'static mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>,
) -> ! {
    loop {
        let cmd = COMMAND_CHANNEL.receive().await;
        match cmd {
            ModemCommand::GnssInit(g) => {
                match client.send(&g).await {
                    Ok(resp) => { /* update some shared Signal or signal strength */ }
                    Err(e) => { /* log/handle */ }
                    #[allow(unreachable_patterns)]
                    _ => unreachable!(),
                }
            }
            ModemCommand::GnssDeinit => match client.send(&GnssDeinit {}).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::GetLocation(f) => match client.send(&f).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttConfig(m) => match client.send(&m).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketCreate(s) => match client.send(&s).await {
                Ok(r) => {
                    let socket_id = r.socket_id;
                }
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketClose(s) => match client.send(&s).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttConnect(mqtt_connect) => match client.send(&mqtt_connect).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttDisconnect(mqtt_disconnect) => {
                match client.send(&mqtt_disconnect).await {
                    Ok(r) => {}
                    Err(e) => {}
                    #[allow(unreachable_patterns)]
                    _ => unreachable!(),
                }
            }
            ModemCommand::MqttPublish(mqtt_publish) => match client.send(&mqtt_publish).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttSubscribe(s) => match client.send(&s).await {
                Ok(r) => {}
                Err(e) => {}
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttUnsubscribe(mqtt_unsubscribe) => {
                match client.send(&mqtt_unsubscribe).await {
                    Ok(r) => {}
                    Err(e) => {}
                    #[allow(unreachable_patterns)]
                    _ => unreachable!(),
                }
            }
        }
    }
}
