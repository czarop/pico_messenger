use defmt::{error, info, warn};
use embassy_futures::select::Either;
use embassy_time::{Duration, Timer};
use heapless::String;

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::modem::{command_task, communication::{self, COMMAND_CHANNEL, MQTT_COMMAND, MQTT_STATE, SUBSCRIBE_RESULT, UNSUBSCRIBE_RESULT}, mqtt::{
        commands::{
            clock::CclkQuery,
            config::MqttConfig,
            connect::{MqttConnect, MqttConnectStub, MqttDisconnect},
            pdn::CgPaddrQuery,
            reset::ModemReset,
            socket::{SocketClose, SocketCreate, SocketQuery},
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

/// True when a `+CEREG` URC body indicates the radio is registered (stat 1 =
/// registered home, 5 = registered roaming). The body may or may not include the
/// `+CEREG:` prefix depending on how it was captured; in the URC form the status
/// is the first field. Tolerates the surrounding quotes/whitespace.
fn cereg_registered(body: &str) -> bool {
    let s = body.trim();
    let s = s.strip_prefix("+CEREG:").map(|r| r.trim()).unwrap_or(s);
    matches!(
        s.split(',')
            .next()
            .and_then(|f| f.trim().trim_matches('"').parse::<u8>().ok()),
        Some(1) | Some(5)
    )
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
    let mut connect_failures: u8 = 0;
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

            // COMMAND_CHANNEL
            //     .send(command_task::ModemCommand::SocketClose(SocketClose {
            //         context_id: socket_info.context_id(),
            //         socket_id,
            //     }))
            //     .await;
            // let _ = communication::NETWORK_RESULT.wait().await;

            state_sender.send(state::MqttStackState::Down);
        }

        if let Some(state::MqttStackState::IpUp) = state_watcher.try_get() {
            // A previous session's TCP socket can still occupy the modem's single
            // TCP slot after an RP2350 reflash, causing SOCKETCREATE to fail with
            // +CME ERROR: 2159 (max sockets reached). Query the modem for the
            // sockets it actually holds open (by real ID) and close each one. If
            // a listed socket then refuses to close, it is wedged and only a
            // modem reset clears it (handled below).
            defmt::info!("querying modem for open sockets before create");
            COMMAND_CHANNEL
                .send(command_task::ModemCommand::SocketQuery(SocketQuery {}))
                .await;

            // A socket the modem lists as open but then refuses to close
            // (+CME ERROR: 2104) is wedged: it keeps occupying the single TCP
            // slot (create fails with +CME ERROR: 2159) and no host-side
            // SOCKETCLOSE can clear it. This survives an RP2350 reflash; only a
            // modem reboot clears it. Track whether we hit that so we can reset.
            let mut needs_modem_reset = false;
            match communication::SOCKET_QUERY_RESULT.wait().await {
                Ok(open) => {
                    if open.ids.is_empty() {
                        info!("no sockets open - proceeding to create");
                    } else {
                        for id in open.ids.iter() {
                            info!("closing stale socket id={}", id);
                            COMMAND_CHANNEL
                                .send(command_task::ModemCommand::SocketClose(SocketClose {
                                    context_id: socket_info.context_id(),
                                    socket_id: *id,
                                }))
                                .await;
                            match communication::NETWORK_RESULT.wait().await {
                                Ok(()) => info!("closed stale socket id={}", id),
                                Err(e) => {
                                    warn!(
                                        "socket id={} listed open but won't close ({:?}) - modem reset required",
                                        id, e
                                    );
                                    needs_modem_reset = true;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("socket query failed ({:?}) - proceeding to create", e);
                }
            }

            if needs_modem_reset {
                // Reboot the modem to clear the wedged socket. AT#RESET=0 keeps
                // provisioned config. The module may reboot before replying, so
                // ignore the command result and rely on the #REBOOT_HOST URC
                // (handled in the monitor loop) to re-drive bring-up.
                warn!("resetting modem (AT#RESET=0) to clear wedged socket");
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::ModemReset(ModemReset::default()))
                    .await;
                let _ = communication::NETWORK_RESULT.wait().await;
                state_sender.send(state::MqttStackState::Down);
                continue 'outer;
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

            // TLS needs the modem RTC, which only syncs after EMM INFORMATION
            // (post-registration). After a reboot we can reach connect before the
            // clock lands, making the handshake time out (surfacing as MQTT
            // +CME ERROR: 2214). Poll AT+CCLK? and gate connect on a sane date.
            // (Whether the modem returns a stale default date or errors before
            // sync, both keep us polling.)
            defmt::info!("waiting for modem clock before connect");
            let mut clock_ready = false;
            for _ in 0..30 {
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::ClockQuery(CclkQuery {}))
                    .await;
                match communication::CLOCK_RESULT.wait().await {
                    Ok(c) if c.ready => {
                        info!("modem clock ready (year={})", c.year.unwrap_or(0));
                        clock_ready = true;
                        break;
                    }
                    Ok(c) => info!("clock not ready yet (year={:?}), polling...", c.year),
                    Err(e) => info!("clock query not ready ({:?}), polling...", e),
                }
                Timer::after(Duration::from_secs(1)).await;
            }
            if !clock_ready {
                warn!("modem clock not ready after polling - attempting connect anyway");
            }

            let mqtt_connection = MqttConnect::from_stub(socket_id, mqtt_connection.clone());
            defmt::info!("connecting mqtt");
            COMMAND_CHANNEL
                .send(command_task::ModemCommand::MqttConnect(
                    mqtt_connection.clone(),
                ))
                .await;

            match communication::NETWORK_RESULT.wait().await {
                Ok(()) => {
                    info!("Mqtt connected");
                    connect_failures = 0;
                    state_sender.send(state::MqttStackState::MqttReady);
                }
                Err(e) => {
                    // A failed MQTTCONNECT tears the socket down (subsequent ops
                    // return +CME ERROR: 2104, invalid socket id), so retrying
                    // connect on the same socket is futile. Rebuild from socket
                    // creation instead. After repeated failures, escalate to a
                    // modem reset for a clean slate.
                    connect_failures = connect_failures.saturating_add(1);
                    error!(
                        "Failed to connect MQTT stack ({:?}), failure {} - rebuilding socket",
                        e, connect_failures
                    );
                    if connect_failures >= 5 {
                        warn!("MQTT connect failed {} times - resetting modem", connect_failures);
                        connect_failures = 0;
                        COMMAND_CHANNEL
                            .send(command_task::ModemCommand::ModemReset(ModemReset::default()))
                            .await;
                        let _ = communication::NETWORK_RESULT.wait().await;
                        state_sender.send(state::MqttStackState::Down);
                    } else {
                        // The connection most likely dropped mid-handshake — a
                        // registration loss (+CEREG stat 2, #IPCFG ip_status 0)
                        // leaves the network down, so immediately rebuilding just
                        // hammers SOCKETCREATE with +CME ERROR: 2106 (network
                        // down). Drop to Down and let the monitor loop re-drive
                        // when the radio re-registers (+CEREG stat 1/5 or
                        // +CGEV ME PDN ACT 5).
                        state_sender.send(state::MqttStackState::Down);
                    }
                    continue 'outer;
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
                            // Only treat a context (re)activation as a trigger to
                            // rebuild when we're actually Down. A genuine
                            // mid-session re-attach is always preceded by a detach
                            // (handled above) that drops us to Down, so this still
                            // catches real recovery. Without the guard, a stale
                            // ME PDN ACT 5 buffered during the initial cold attach
                            // gets drained right after a successful connect and
                            // tears the live session down.
                            if matches!(
                                state_watcher.try_get(),
                                Some(state::MqttStackState::Down)
                            ) {
                                state_sender.send(state::MqttStackState::IpUp);
                                info!("Network IPuP");
                                break;
                            }
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
                    ModemUrc::Cereg(body) => {
                        // Network registration changed. Only act on it as a
                        // recovery signal: if we're sitting in Down (e.g. after a
                        // connect dropped mid-handshake) and the radio has
                        // re-registered, re-drive bring-up. During healthy
                        // operation a routine +CEREG must not disturb the stack.
                        if cereg_registered(body.as_str())
                            && matches!(
                                state_watcher.try_get(),
                                Some(state::MqttStackState::Down)
                            )
                        {
                            info!("network re-registered - rebuilding mqtt stack");
                            state_sender.send(state::MqttStackState::IpUp);
                            break;
                        }
                    }
                    _ => {}
                },
                Either::Second(cmd) => match cmd {
                    MqttCommand::Start => {
                        // Ignore a Start unless we're actually Down. A spontaneous
                        // +CGEV ME PDN ACT 5 attach can drive the stack up before
                        // modem_task's orchestrated Start arrives; acting on the
                        // redundant Start then launches a *second* bring-up that
                        // finds the first's live socket via SOCKETCREATE? and
                        // mistakes it for a wedged one (SOCKETCLOSE -> 2104 ->
                        // modem reset). modem_task only waits on MQTT_STATE reaching
                        // MqttReady, which the in-flight bring-up already provides,
                        // so dropping the duplicate Start is safe. After a normal
                        // teardown the state is Down, so the next cycle's Start is
                        // honoured as usual.
                        if !matches!(
                            state_watcher.try_get(),
                            Some(state::MqttStackState::Down)
                        ) {
                            info!("Start ignored - bring-up already in progress");
                            break;
                        }
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