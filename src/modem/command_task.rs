use super::setup::INGRESS_BUF_SIZE;
use crate::modem::gnss::commands::{deinit::GnssDeinit, fix::GnssFix, init::GnssInit};
use crate::modem::communication::{self, COMMAND_CHANNEL};
use crate::modem::error::ModemError;
use crate::modem::mqtt::commands::{config, connect, publish, socket, subscribe};
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
pub async fn command_task(
    client: &'static mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>,
) -> ! {
    defmt::info!("command task spawned");
    loop {
        let cmd = COMMAND_CHANNEL.receive().await;
        match cmd {
            ModemCommand::GnssInit(g) => match client.send(&g).await {
                Ok(_) => communication::GNSS_RESULT.signal(Ok(())),
                Err(e) => communication::GNSS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::GnssDeinit => match client.send(&GnssDeinit {}).await {
                Ok(_) => communication::GNSS_RESULT.signal(Ok(())),
                Err(e) => communication::GNSS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::GetLocation(f) => match client.send(&f).await {
                Ok(_) => communication::GNSS_RESULT.signal(Ok(())),
                Err(e) => communication::GNSS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttConfig(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketCreate(s) => match client.send(&s).await {
                Ok(r) => communication::SOCKET_RESULT.signal(Ok(r.socket_id)),
                Err(e) => communication::SOCKET_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketClose(s) => match client.send(&s).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttConnect(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttDisconnect(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttPublish(m) => match client.send(&m).await {
                Ok(_) => communication::PUBLISH_RESULT.signal(Ok(())),
                Err(e) => communication::PUBLISH_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttSubscribe(s) => match client.send(&s).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttUnsubscribe(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
        }
    }
}
