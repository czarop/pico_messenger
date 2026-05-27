use defmt::{error, info, warn};

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    modem::{
        command,
        communication::{self, COMMAND_CHANNEL, MQTT_STATE},
        urc::ModemUrc,
    },
    mqtt::{
        commands::{
            config::MqttConfig,
            connect::{MqttConnect, MqttConnectStub},
            socket::SocketCreate,
            subscribe::MqttSubscribe,
        },
        state,
        urc::ip_stack,
    },
};

#[embassy_executor::task]
pub async fn network_task(
    mut sub: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
    socket_info: SocketCreate,
    mqtt_config: MqttConfig,
    mqtt_connection: MqttConnectStub,
    mqtt_subscription: MqttSubscribe,
) -> ! {
    let state_sender = communication::MQTT_STATE.sender();
    state_sender.send(state::MqttStackState::Down);
    let mut state_watcher = MQTT_STATE.receiver().unwrap();
    'outer: loop {
        if let Some(state::MqttStackState::Down) =state_watcher.try_get() {
            // wait for IP
            loop {
                if let ModemUrc::IPStackUpdate(cgev) = sub.next_message_pure().await {
                    if let ip_stack::CgevEvent::MePdnAct(5) = cgev.event() {
                        break;
                    }
                }
            }
            state_sender.send(state::MqttStackState::IpUp);
        }

        if let Some(state::MqttStackState::IpUp) = state_watcher.try_get()  {
            // bring up the mqtt stack
            let mut retries = 0;
            let socket_id;
            loop {
                COMMAND_CHANNEL
                    .send(command::ModemCommand::SocketCreate(socket_info.clone()))
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
                        if retries > 9 {
                            error!("Failed to create socket 10 times, Aborting");
                            continue 'outer;
                        }
                    } // back to the outer loop
                }
            }

            info!("Socket created, configuring MQTT stack");

            let mut retries = 0;
            loop {
                COMMAND_CHANNEL
                    .send(command::ModemCommand::MqttConfig(mqtt_config.clone()))
                    .await;

                match communication::NETWORK_RESULT.wait().await {
                    Ok(()) => {
                        info!("Mqtt config set");
                        break;
                    }
                    Err(e) => {
                        error!("Failed to configure MQTT stack: {:?}", e);
                        retries += 1;
                        if retries > 9 {
                            error!("Failed to configure MQTT stack 10 times, Aborting");
                            continue 'outer;
                        }
                    }
                }
            }

            let mut retries = 0;
            let mqtt_connection = MqttConnect::from_stub(socket_id, mqtt_connection.clone());
            loop {
                COMMAND_CHANNEL
                    .send(command::ModemCommand::MqttConnect(mqtt_connection.clone()))
                    .await;

                match communication::NETWORK_RESULT.wait().await {
                    Ok(()) => {
                        info!("Mqtt connected");
                        break;
                    }
                    Err(e) => {
                        error!("Failed to connect MQTT stack: {:?}", e);
                        retries += 1;
                        if retries > 9 {
                            error!("Failed to connect MQTT stack 10 times, Aborting");
                            continue 'outer;
                        }
                    }
                }
            }

            let mut retries = 0;
            loop {
                COMMAND_CHANNEL
                    .send(command::ModemCommand::MqttSubscribe(
                        mqtt_subscription.clone(),
                    ))
                    .await;

                match communication::NETWORK_RESULT.wait().await {
                    Ok(()) => {
                        info!("Mqtt subscribed");
                        state_sender.send(state::MqttStackState::MqttReady);
                        break;
                    }
                    Err(e) => {
                        error!("Failed to subscribe to MQTT: {:?}", e);
                        retries += 1;
                        if retries > 9 {
                            error!("Failed to subscribe to MQTT 10 times, Aborting");
                            continue 'outer;
                        }
                    }
                }
            }
        }
        // monitor for disconnect
        loop {
            match sub.next_message_pure().await {
                ModemUrc::SocketClosed(e) => {
                    warn!(
                        "Socket closed: context_id={}, socket_id={}",
                        e.context_id, e.socket_id
                    );
                    state_sender.send(state::MqttStackState::IpUp);
                    break;
                }
                ModemUrc::IPStackUpdate(cgev) => match cgev.event() {
                    ip_stack::CgevEvent::NwDetach 
                | ip_stack::CgevEvent::MeDetach
                | ip_stack::CgevEvent::MePdnDeact(_)
                | ip_stack::CgevEvent::NwPdnDeact(_) => {
                        warn!("Network detached");
                        state_sender.send(state::MqttStackState::Down);
                        break;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
