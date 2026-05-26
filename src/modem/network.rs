use defmt::{error, info};
use heapless::String;

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    modem::{
        command,
        communication::{self, COMMAND_CHANNEL},
        urc::ModemUrc,
    },
    mqtt::{
        commands::{
            config::MqttConfig, connect::MqttConnect, socket::SocketCreate,
            subscribe::MqttSubscribe,
        },
        state,
        urc::ip_stack,
    },
};

#[embassy_executor::task]
pub async fn network_task(
    mut sub: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) -> ! {
    let state_sender = communication::MQTT_STATE.sender();
    loop {
        // wait for IP
        state_sender.send(state::MqttStackState::Down);
        loop {
            if let ModemUrc::IPStackUpdate(cgev) = sub.next_message_pure().await {
                if let ip_stack::CgevEvent::MePdnAct(5) = cgev.event() {
                    break;
                }
            }
        }
        state_sender.send(state::MqttStackState::IpUp);

        // bring up the mqtt stack
        let mut retries = 0;
        let mut socket_id = 0;
        loop {
            if retries > 9 {
                error!(
                    "Failed to create socket after {} retries, giving up",
                    retries
                );
                panic!("Failed to create socket");
            }
            COMMAND_CHANNEL
                .send(command::ModemCommand::SocketCreate(SocketCreate::default()))
                .await;

            match communication::SOCKET_RESULT.wait().await {
                Ok(socket) => {
                    state_sender.send(state::MqttStackState::SocketReady(socket));
                    socket_id = socket;
                    break;
                }
                Err(e) => {
                    error!("Failed to create socket: {:?}", e);
                    retries += 1;
                    continue;
                } // back to the outer loop
            }
        }

        info!("Socket created, configuring MQTT stack");

        COMMAND_CHANNEL
            .send(command::ModemCommand::MqttConfig(MqttConfig::default()))
            .await;

        match communication::NETWORK_RESULT.wait().await {
            Ok(()) => {
                info!("Mqtt config set");
            }
            Err(e) => error!("Failed to configure MQTT stack: {:?}", e),
        }

        COMMAND_CHANNEL
            .send(command::ModemCommand::MqttConnect(MqttConnect::new(
                socket_id,
                String::try_from("test.test.test.test").unwrap(),
                1883,
                String::try_from("user").unwrap(),
                String::try_from("pass").unwrap(),
                String::try_from("pico-messenger/status").unwrap(),
                String::try_from("offline").unwrap(),
            )))
            .await;

        match communication::NETWORK_RESULT.wait().await {
            Ok(()) => {
                info!("Mqtt connected");
            }
            Err(e) => error!("Failed to configure MQTT stack: {:?}", e),
        }

        COMMAND_CHANNEL
            .send(command::ModemCommand::MqttSubscribe(MqttSubscribe {
                topic: String::try_from("my_topic").unwrap(),
                qos: crate::mqtt::commands::MqttQos::AtLeastOnce,
            }))
            .await;

        match communication::NETWORK_RESULT.wait().await {
            Ok(()) => {
                info!("Mqtt subscribed");
                state_sender.send(state::MqttStackState::MqttReady);
            }
            Err(e) => error!("Failed to configure MQTT stack: {:?}", e),
        }

        // monitor for disconnect
        loop {
            match sub.next_message_pure().await {
                ModemUrc::SocketClosed(_) => break,
                ModemUrc::IPStackUpdate(cgev) => match cgev.event() {
                    ip_stack::CgevEvent::NwDetach | ip_stack::CgevEvent::MeDetach => break,
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
