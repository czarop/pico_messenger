use defmt::{error, info, warn};
use embassy_futures::select::Either;
use embassy_time::{Duration, Timer};
use heapless::String;

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::modem::{command_task, communication::{self, COMMAND_CHANNEL, MQTT_COMMAND, MQTT_STATE, SUBSCRIBE_RESULT, UNSUBSCRIBE_RESULT}, mqtt::{
        commands::{
            config::MqttConfig,
            connect::{MqttConnect, MqttConnectStub, MqttDisconnect},
            pdn::CgPaddrQuery,
            socket::{SocketClose, SocketCreate},
            subscribe::{MqttSubscribe, MqttUnsubscribe},
        },
        state,
        urc::ip_stack,
    }, urc::ModemUrc};

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
    defmt::info!("network task spawned");
    let mut topics_to_subscribe: heapless::Vec<MqttSubscribe, 5> = heapless::Vec::new();
    let mut subscribed_topics: heapless::Vec<MqttSubscribe, 5> = heapless::Vec::new();
    let mut topics_to_unsubscribe: heapless::Vec<String<50>, 5> = heapless::Vec::new();
    let state_sender = communication::MQTT_STATE.sender();
    state_sender.send(state::MqttStackState::Down);
    let mut state_watcher = MQTT_STATE.receiver().unwrap();
    let incoming_commands = &MQTT_COMMAND;
    let mut try_disconnect = false;
    let mut socket_id = 0;
    'outer: loop {
        if !matches!(state_watcher.try_get(), Some(state::MqttStackState::Down)) && try_disconnect {
            defmt::info!("disconnecting");
            for topic in subscribed_topics.iter() {
                defmt::info!("unsubscribing");
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::MqttUnsubscribe(
                        MqttUnsubscribe {
                            topic: topic.topic.clone(),
                        },
                    ))
                    .await;
                let _ = communication::NETWORK_RESULT.wait().await;
            }
            subscribed_topics.clear();

            COMMAND_CHANNEL
                .send(command_task::ModemCommand::MqttDisconnect(
                    MqttDisconnect {},
                ))
                .await;
            let _ = communication::NETWORK_RESULT.wait().await;

            COMMAND_CHANNEL
                .send(command_task::ModemCommand::SocketClose(SocketClose {
                    context_id: socket_info.context_id(),
                    socket_id,
                }))
                .await;
            let _ = communication::NETWORK_RESULT.wait().await;

            state_sender.send(state::MqttStackState::Down);
        }

        if let Some(state::MqttStackState::IpUp) = state_watcher.try_get() {
            // Only one TCP socket exists (id 0). It can be left occupied by a previous
            // session or the modem's own bring-up, causing SOCKETCREATE to fail with
            // +CME ERROR: 2159 (max sockets reached). Close it first; the error is
            // benign when no socket is open. The Ok/Err result also tells us whether a
            // stale socket actually existed.
            defmt::info!("closing any stale socket before create");
            COMMAND_CHANNEL
                .send(command_task::ModemCommand::SocketClose(SocketClose {
                    context_id: socket_info.context_id(),
                    socket_id: 0,
                }))
                .await;
            match communication::NETWORK_RESULT.wait().await {
                Ok(()) => info!("closed a stale socket (id 0)"),
                Err(e) => info!("no stale socket to close (expected): {:?}", e),
            }

            defmt::info!("creating socket");
            // bring up the mqtt stack
            let mut retries = 0;

            loop {
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::SocketCreate(
                        socket_info.clone(),
                    ))
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
                            state_sender.send(state::MqttStackState::Down);
                            continue 'outer;
                        }
                    } // back to the outer loop
                }
            }
            info!("Socket created, configuring MQTT stack");
        }

        if let Some(state::MqttStackState::SocketReady(_)) = state_watcher.try_get() {
            defmt::info!("configuring mqtt");
            let mut retries = 0;
            loop {
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::MqttConfig(mqtt_config.clone()))
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
                            state_sender.send(state::MqttStackState::Down);
                            continue 'outer;
                        }
                    }
                }
            }

            let mut retries = 0;
            let mqtt_connection = MqttConnect::from_stub(socket_id, mqtt_connection.clone());
            loop {
                defmt::info!("connecting mqtt");
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::MqttConnect(
                        mqtt_connection.clone(),
                    ))
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
                            state_sender.send(state::MqttStackState::Down);
                            continue 'outer;
                        }
                    }
                }
            }
        }

        if let Some(state::MqttStackState::MqttReady) = state_watcher.try_get() {
            defmt::info!("subscribing");
            while let Some(mqtt_subscription) = topics_to_subscribe.pop() {
                if !subscribed_topics.is_full() {
                    let mut retries = 0;
                    loop {
                        COMMAND_CHANNEL
                            .send(command_task::ModemCommand::MqttSubscribe(
                                mqtt_subscription.clone(),
                            ))
                            .await;

                        match communication::NETWORK_RESULT.wait().await {
                            Ok(()) => {
                                info!("Mqtt subscribed");
                                SUBSCRIBE_RESULT.signal(Ok(mqtt_subscription.topic.clone()));
                                let _ = subscribed_topics.push(mqtt_subscription);
                                break;
                            }
                            Err(e) => {
                                error!("Failed to subscribe to MQTT: {:?}", e);
                                retries += 1;
                                if retries > 9 {
                                    SUBSCRIBE_RESULT.signal(Err(e));
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

            if let Some(topic_name) = topics_to_unsubscribe.pop() {
                if subscribed_topics
                    .iter()
                    .find(|t| t.topic == topic_name)
                    .is_some()
                {
                    let mut retries = 0;
                    loop {
                        COMMAND_CHANNEL
                            .send(command_task::ModemCommand::MqttUnsubscribe(
                                MqttUnsubscribe {
                                    topic: topic_name.clone(),
                                },
                            ))
                            .await;

                        match communication::NETWORK_RESULT.wait().await {
                            Ok(()) => {
                                info!("Mqtt unsubscribed");
                                UNSUBSCRIBE_RESULT.signal(Ok(topic_name.clone()));
                                let _ = subscribed_topics.retain(|t| &t.topic != &topic_name);
                                break;
                            }
                            Err(e) => {
                                error!("Failed to unsubscribe to MQTT: {:?}", e);
                                retries += 1;
                                if retries > 9 {
                                    error!(
                                        "Failed to unsubscribe {} 10 times, Aborting",
                                        &topic_name
                                    );
                                    // we just popped this so is fine to re-add
                                    UNSUBSCRIBE_RESULT.signal(Err(e));
                                    let _ = topics_to_unsubscribe.push(topic_name);
                                    continue 'outer;
                                }
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
                        topics_to_subscribe = subscribed_topics;
                        subscribed_topics = heapless::Vec::new();
                        state_sender.send(state::MqttStackState::IpUp);
                        break;
                    }
                    ModemUrc::IPStackUpdate(cgev) => match cgev.event() {
                        ip_stack::CgevEvent::NwDetach
                        | ip_stack::CgevEvent::MeDetach
                        | ip_stack::CgevEvent::MePdnDeact(_)
                        | ip_stack::CgevEvent::NwPdnDeact(_) => {
                            warn!("Network detached");
                            topics_to_subscribe = subscribed_topics;
                            subscribed_topics = heapless::Vec::new();
                            state_sender.send(state::MqttStackState::Down);
                            break;
                        }
                        ip_stack::CgevEvent::MePdnAct(5) => {
                            state_sender.send(state::MqttStackState::IpUp);
                            info!("Network IPuP");
                            break;
                        }
                        _ => {}
                    },
                    ModemUrc::MqttReceived(raw) => match raw.message() {
                        Some(msg) => {
                            info!(
                                "MQTT message received: topic={}, payload={}",
                                msg.topic.as_str(),
                                msg.payload.as_str()
                            );
                        }
                        None => {
                            warn!("MQTTRECV: could not split body: {}", raw.body.as_str());
                        }
                    },
                    ModemUrc::RebootHost | ModemUrc::RebootReset | ModemUrc::RebootWD(..) | ModemUrc::SysStart => {
                        warn!("modem rebooted, resetting mqtt network");
                            topics_to_subscribe = subscribed_topics;
                            subscribed_topics = heapless::Vec::new();
                            state_sender.send(state::MqttStackState::Down);
                            break;
                    }
                    _ => {}
                },
                Either::Second(cmd) => match cmd {
                    MqttCommand::Start => {
                        try_disconnect = false;
                        warn!("Network start called");
                        // The modem auto-activates context 5 once NB-IoT registration
                        // completes, which can take well over a minute on first attach.
                        // Poll rather than probe once, so we proceed the moment the
                        // context is up instead of waiting for the next orchestration tick.
                        // ~40 * 3s = 120s, matching modem_task's READY_TIMEOUT.
                        let mut seeded = false;
                        for _ in 0..40 {
                            COMMAND_CHANNEL
                                .send(command_task::ModemCommand::GetPdpAddress(
                                    CgPaddrQuery::default(),
                                ))
                                .await;
                            match communication::PDP_ADDRESS_RESULT.wait().await {
                                Ok(true) => {
                                    info!("PDP context active - seeding IpUp");
                                    state_sender.send(state::MqttStackState::IpUp);
                                    seeded = true;
                                    break;
                                }
                                Ok(false) => {
                                    info!("PDP context not active yet, polling...");
                                }
                                Err(e) => {
                                    info!("PDP query not ready ({:?}), polling...", e);
                                }
                            }
                            Timer::after(Duration::from_secs(3)).await;
                        }
                        if !seeded {
                            warn!(
                                "PDP context not active after polling - waiting for CGEV edge"
                            );
                        }
                        break;
                    }
                    MqttCommand::Stop => {
                        try_disconnect = true;
                        break;
                    }
                    MqttCommand::Subscribe(topic) => {
                        if !subscribed_topics.iter().any(|t| t.topic == topic.topic) {
                            if !subscribed_topics.is_full() {
                                match topics_to_subscribe.push(topic) {
                                    Ok(_) => break,
                                    Err(_) => warn!("trying to subscribe to too many topics"),
                                };
                            } else {
                                warn!("Cannot subscribe to more mqtt topics - already at capacity");
                            }
                        }
                    }
                    MqttCommand::Unsubscribe(topic) => {
                        if subscribed_topics.iter().any(|t| t.topic == topic) {
                            match topics_to_unsubscribe.push(topic) {
                                Ok(_) => break,
                                Err(_) => warn!("trying to unsubscribe to too many topics"),
                            }
                        } else {
                            warn!(
                                "Cannot unsubscribe from topic {} - not currently subscribed",
                                &topic
                            );
                        }
                    }
                },
            }
        }
    }
}