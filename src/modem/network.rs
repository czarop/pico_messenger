use defmt::{error, info, warn};
use embassy_futures::select::Either;
use heapless::String;

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    modem::{
        command,
        communication::{self, COMMAND_CHANNEL, MQTT_COMMAND, MQTT_STATE},
        urc::ModemUrc,
    },
    mqtt::{
        self,
        commands::{
            config::MqttConfig,
            connect::{MqttConnect, MqttConnectStub, MqttDisconnect},
            socket::{SocketClose, SocketCreate},
            subscribe::{MqttSubscribe, MqttUnsubscribe},
        },
        state,
        urc::ip_stack,
    },
};

pub enum MqttCommand {
    Start,
    Stop,
    Subscribe(MqttSubscribe),
    Unsubscribe(String<50>),
}

#[embassy_executor::task]
pub async fn network_task(
    mut sub: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
    socket_info: SocketCreate,
    mqtt_config: MqttConfig,
    mqtt_connection: MqttConnectStub,
) -> ! {
    let mut topics_to_subscribe: heapless::Vec<MqttSubscribe, 5> = heapless::Vec::new();
    let mut subscribed_topics: heapless::Vec<MqttSubscribe, 5> = heapless::Vec::new();
    let mut topic_to_unsubscribe: Option<String<50>> = None;
    let state_sender = communication::MQTT_STATE.sender();
    state_sender.send(state::MqttStackState::Down);
    let mut state_watcher = MQTT_STATE.receiver().unwrap();
    let incoming_commands = &MQTT_COMMAND;
    let mut try_connect = false;
    let mut try_disconnect = false;
    let mut socket_id = 0;
    'outer: loop {
        let mqtt_stack_state = state_watcher.try_get();

        if matches!(mqtt_stack_state, Some(state::MqttStackState::Down)) && try_connect {
            // wait for IP
            loop {
                if let ModemUrc::IPStackUpdate(cgev) = sub.next_message_pure().await {
                    if let ip_stack::CgevEvent::MePdnAct(5) = cgev.event() {
                        try_connect = false;
                        break;
                    }
                }
            }
            state_sender.send(state::MqttStackState::IpUp);
        }

        if !matches!(mqtt_stack_state, Some(state::MqttStackState::Down)) && try_disconnect {
            for topic in subscribed_topics.iter() {
                COMMAND_CHANNEL
                    .send(command::ModemCommand::MqttUnsubscribe(MqttUnsubscribe {
                        topic: topic.topic.clone(),
                    }))
                    .await;
                let _ = communication::NETWORK_RESULT.wait().await;
            }
            subscribed_topics.clear();

            COMMAND_CHANNEL
                .send(command::ModemCommand::MqttDisconnect(MqttDisconnect {
                }))
                .await;
            let _ = communication::NETWORK_RESULT.wait().await;

            COMMAND_CHANNEL
                .send(command::ModemCommand::SocketClose(SocketClose {
                    context_id: socket_info.context_id(),
                    socket_id,
                }))
                .await;
            let _ = communication::NETWORK_RESULT.wait().await;

            state_sender.send(state::MqttStackState::Down);
        }

        if let Some(state::MqttStackState::IpUp) = mqtt_stack_state {
            // bring up the mqtt stack
            let mut retries = 0;

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
                        state_sender.send(state::MqttStackState::MqttReady);
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

            while let Some(mqtt_subscription) = topics_to_subscribe.pop() {
                if !subscribed_topics.is_full() {
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
                                let _ = subscribed_topics.push(mqtt_subscription);
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
                } else {
                    warn!("Cannot subscribe to more mqtt topics - already at capacity");
                }
            }
        }

        if let Some(topic_name) = topic_to_unsubscribe.take() {
            if let Some(mqtt_subscription) =
                subscribed_topics.iter().find(|t| t.topic == topic_name)
            {
                let mut retries = 0;
                loop {
                    COMMAND_CHANNEL
                        .send(command::ModemCommand::MqttUnsubscribe(MqttUnsubscribe {
                            topic: topic_name.clone(),
                        }))
                        .await;

                    match communication::NETWORK_RESULT.wait().await {
                        Ok(()) => {
                            info!("Mqtt unsubscribed");
                            let _ = subscribed_topics.retain(|t| &t.topic != &topic_name);
                            break;
                        }
                        Err(e) => {
                            error!("Failed to unsubscribe to MQTT: {:?}", e);
                            retries += 1;
                            if retries > 9 {
                                error!("Failed to unsubscribe to MQTT 10 times, Aborting");
                                continue 'outer;
                            }
                        }
                    }
                }
            }
        }

        // monitor for disconnect or new command
        loop {
            match embassy_futures::select::select(sub.next_message_pure(), incoming_commands.wait())
                .await
            {
                Either::First(urc) => match urc {
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
                },
                Either::Second(cmd) => match cmd {
                    MqttCommand::Start => {
                        try_connect = true;
                        try_disconnect = false;
                        break;
                    }
                    MqttCommand::Stop => {
                        try_connect = false;
                        try_disconnect = true;
                        break;
                    }
                    MqttCommand::Subscribe(topic) => {
                        if !subscribed_topics.iter().any(|t| t.topic == topic.topic) {
                            match topics_to_subscribe.push(topic) {
                                Ok(_) => break,
                                Err(_) => warn!("too many topics subscribed"),
                            };
                        }
                    }
                    MqttCommand::Unsubscribe(topic) => {
                        if subscribed_topics.iter().any(|t| t.topic == topic) {
                            topic_to_unsubscribe = Some(topic);
                            break;
                        }
                    }
                },
            }
        }
    }
}
