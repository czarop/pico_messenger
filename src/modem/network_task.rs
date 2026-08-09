use defmt::{error, info, warn};
use embassy_futures::select::Either;
use embassy_time::{Duration, Timer};
use heapless::String;

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::modem::{command_task::{self, ModemCommand}, communication::{self, COMMAND_CHANNEL, MQTT_COMMAND, MQTT_STATE, SUBSCRIBE_RESULT, UNSUBSCRIBE_RESULT}, mqtt::{
        commands::{
            cereg, cesq, clock::CclkQuery, config::MqttConfig, connect::{MqttConnect, MqttConnectStub, MqttDisconnect}, pdn::CgPaddrQuery, reset::ModemReset, socket::{SocketClose, SocketCreate, SocketQuery}, subscribe::{MqttSubscribe, MqttUnsubscribe},
        }, state, urc::ip_stack,
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

    // TWO SHAPES, and getting this wrong silently wedges bring-up:
    //
    //   URC form   `+CEREG: <stat>,"<tac>",...`      -> stat is field 0
    //   Read form  `+CEREG: <n>,<stat>,"<tac>",...`  -> stat is field 1
    //
    // The read form arrives here too: `+CEREG` is a URC token, so the response to
    // AT+CEREG? gets consumed by the URC arm rather than by the command. Reading
    // field 0 there yields <n> (the URC config mode, 4), which is never 1 or 5 --
    // so the device decides it is unregistered and never brings MQTT up, no matter
    // what the radio is actually doing.
    //
    // Discriminate on field 1: a bare number means the read form. A quoted TAC
    // like "242E" cannot parse as a number, so the two can't be confused.
    let mut fields = s.split(',');
    let first = fields.next();
    let second = fields.next();

    let stat = match second.and_then(|f| f.trim().parse::<u8>().ok()) {
        Some(stat) => Some(stat), // read form: field 1
        None => first.and_then(|f| f.trim().trim_matches('"').parse::<u8>().ok()),
    };

    matches!(stat, Some(1) | Some(5))
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
    // Consecutive SOCKETCREATE failures. A failed create is the authoritative
    // "network not actually usable" signal (CGPADDR and CEREG both report
    // registered-with-address from cold boot while the IP session is dead, so
    // they cannot be trusted as gates). Repeated failures while registered mean
    // a wedged modem IP stack -> escalate to AT#RESET=0.
    let mut bringup_failures: u8 = 0;
    // Consecutive Start cycles whose registration poll fully exhausted without
    // reaching stat 1/5. The modem can wedge in stat 2 ("searching") with the
    // PDP context still ACTIVE -- so neither the connect-failure path nor the
    // CGEV/#IPCFG recovery arm fires (both need a PDP-down edge that never
    // comes), and the device polls "searching" forever. Seen on hardware for
    // 250s+ after a botched teardown. AT#RESET=0 is the only exit; this counter
    // triggers it after two dead polls (~4 min) rather than hanging.
    let mut search_failures: u8 = 0;
    let mut registered = true;

    // One-shot network diagnostic (CFUN?/CEREG?/COPS? dumped to defmt).
    // Set true to run once on the first bring-up after boot, then it self-clears.
    const DIAG_SCAN: bool = true;
    let mut diag_done = false;

    'outer: loop {
        if DIAG_SCAN && !diag_done {
            diag_done = true;
            defmt::info!("firing one-shot network diagnostic (COPS scan may take minutes)");
            COMMAND_CHANNEL
                .send(command_task::ModemCommand::NetDiag)
                .await;
            communication::DIAG_RESULT.wait().await;
        }
        if !matches!(state_watcher.try_get(), Some(state::MqttStackState::Down)) && try_disconnect {
            defmt::info!("disconnecting");

            // Disarm immediately. A teardown is a one-shot: it is armed by a
            // Stop and must fire exactly once. If it stays armed past this block,
            // every subsequent IpUp transition re-enters teardown -- and after a
            // reset the modem re-attaches on its own, emitting +CGEV ME PDN ACT 5
            // / +CEREG(registered) URCs that drive the stack back to IpUp with no
            // MQTT session established. Each of those then issues an MQTTDISC that
            // can only fail with 2218 ("no session"), which the reset-on-failed-
            // DISC path below reads as a wedged socket and answers with another
            // AT#RESET=0 -- which produces the next re-attach URC. That is the
            // 3-4 reset burst seen in the diag counters, ending only when the next
            // cycle's Start finally clears the flag. Clearing it here breaks the
            // loop at the source: re-arming requires a fresh Stop.
            try_disconnect = false;

            // Settle before tearing down. On a marginal link, MQTTDISC issued
            // immediately after a publish OK races the still-in-flight PUBLISH
            // and TLS bytes: the modem's disconnect then gets no confirmation
            // in its 25s window, times out, and leaves the socket wedged --
            // which forces a full AT#RESET=0 the NEXT cycle (seen on hardware:
            // hung DISC -> no PSM entry -> skipped sleep -> stale socket ->
            // reset). QoS0 removed the publish-side ack wait but not the bytes
            // on the wire; this gives them time to drain before DISC. Cheap
            // insurance against an expensive cascade.
            embassy_time::Timer::after(embassy_time::Duration::from_secs(2)).await;

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

            // Observation-only: log what the modem thinks the connection state is
            // right before the graceful DISC. Ordered ahead of MqttDisconnect on
            // the command channel, so command_task logs the state first. No wait
            // -- we don't gate on it yet, just gathering evidence that it reliably
            // reports 0 on the dead sessions that make DISC wedge.
            COMMAND_CHANNEL
                .send(command_task::ModemCommand::MqttStateProbe)
                .await;

            COMMAND_CHANNEL
                .send(command_task::ModemCommand::MqttDisconnect(
                    MqttDisconnect {},
                ))
                .await;
            let disconnected = communication::NETWORK_RESULT.wait().await;

            // A SUCCESSFUL MQTTDISC frees the socket, so nothing more is needed.
            // A FAILED one (timeout) leaves the socket wedged: forcing a close
            // just answers 2104 ("invalid socket id" -- won't close), the wedge
            // survives to the next cycle, blocks PSM entry THIS cycle (no #SLEEP,
            // battery burned), and forces a reset next cycle anyway.
            //
            // Broker logs show the disconnect almost always REACHES the broker
            // even when the host sees a timeout -- the session is genuinely gone
            // network-side; only the modem's local socket state is confused. And
            // that confused state provably won't clear with SOCKETCLOSE (2104).
            // So on a failed disconnect, go STRAIGHT to AT#RESET=0: it is the
            // only thing that clears the wedge, and doing it now (rather than
            // discovering the wedge next cycle) means we still enter PSM cleanly
            // this cycle instead of skipping sleep and resetting later.
            //
            // Cost: one reset (~reboot + re-attach) on a failed disconnect. But
            // that reset was already happening next cycle -- this just moves it
            // earlier and reclaims the sleep. Net: one fewer skipped PSM per
            // failed DISC.
            if let Err(e) = disconnected {
                defmt::warn!(
                    "MQTTDISC did not confirm ({:?}) - socket is wedged, resetting modem now to reclaim PSM",
                    e
                );
                COMMAND_CHANNEL
                    .send(command_task::ModemCommand::ModemReset(ModemReset::default()))
                    .await;
                let _ = communication::NETWORK_RESULT.wait().await;
                // Tell modem_task not to attempt PSM this cycle: the modem is
                // rebooting, SLEEPMODE would fail its #SLEEP wait and burn ~45s.
                communication::MODEM_RESET_ON_TEARDOWN
                    .store(true, core::sync::atomic::Ordering::Relaxed);
            }

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
            // SINGLE attempt, deliberately. SOCKETCREATE does not fail
            // transiently when the IP session is genuinely up; every failure
            // mode seen on hardware (2100/2106 network down, wedged stack) is
            // non-retryable at command cadence, and the old 10x zero-backoff
            // retry loop only hammered the modem -- one drained stale
            // registered-looking URC could trigger 10 create errors in ~1.5s.
            // The create IS the liveness probe: fail -> the network is not
            // usable right now, drop to Down and let a fresh registration /
            // PDN-ACT edge re-drive.
            COMMAND_CHANNEL
                .send(command_task::ModemCommand::SocketCreate(
                    socket_info.clone(),
                ))
                .await;

            match communication::SOCKET_RESULT.wait().await {
                Ok(socket) => {
                    bringup_failures = 0;
                    state_sender.send(state::MqttStackState::SocketReady(socket));
                    socket_id = socket;
                }
                Err(e) => {
                    bringup_failures = bringup_failures.saturating_add(1);
                    error!(
                        "socket create failed ({:?}), bring-up failure {} - network not usable, dropping to Down",
                        e, bringup_failures
                    );
                    // Escalation backstop: repeated failures WHILE REGISTERED
                    // mean the modem's IP stack is wedged (seen surviving
                    // reflash and reporting a phantom socket) and only a reboot
                    // clears it. When not registered, a reset cannot help --
                    // the network is simply absent -- and resetting mid-attach
                    // would restart the (60s+) attach and loop forever.
                    if bringup_failures >= 5 && registered {
                        warn!(
                            "socket create failed {} times while registered - resetting modem (AT#RESET=0)",
                            bringup_failures
                        );
                        bringup_failures = 0;
                        COMMAND_CHANNEL
                            .send(command_task::ModemCommand::ModemReset(ModemReset::default()))
                            .await;
                        let _ = communication::NETWORK_RESULT.wait().await;
                    } else {
                        // Brief settle before draining more URCs: rate-limits
                        // churn from a backlog of stale attach URCs and gives a
                        // genuine in-progress attach time to complete.
                        Timer::after(Duration::from_secs(2)).await;
                    }
                    state_sender.send(state::MqttStackState::Down);
                    continue 'outer;
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
                    crate::modem::diag::note_connect_failure();
                    error!(
                        "Failed to connect MQTT stack ({:?}), failure {} - dropping to Down",
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
                        registered = cereg_registered(body.as_str());
                        crate::display::status::set_registered(registered);
                        // Network registration changed. Only act on it as a
                        // recovery signal: if we're sitting in Down (e.g. after a
                        // connect dropped mid-handshake) and the radio has
                        // re-registered, re-drive bring-up. During healthy
                        // operation a routine +CEREG must not disturb the stack.
                        if registered
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
                        // Poll rather than probe once.
                        let mut seeded = false;
                        // Set if a teardown URC surfaces mid-poll (network/modem
                        // dropped during bring-up): abandon the poll and drop to
                        // Down so bring-up restarts against reality.
                        let mut abort_bringup = false;
                        for _ in 0..40 {
                            // Drain any URCs the ingest task has buffered since we
                            // last looked. THIS is how registration is observed
                            // during the poll.
                            //
                            // Why not a CEREG query (the old approach)? A +CEREG
                            // query RESPONSE and an unsolicited +CEREG URC are the
                            // same wire shape, so the atat digester classifies the
                            // reply as a URC and the query's wait() never sees it --
                            // the device sat "searching" for 50s while registered
                            // (stat 5). The URC is already parsed and waiting in the
                            // subscription buffer; we just have to read it. This is
                            // the registration-gate anti-pattern's proper fix:
                            // registration comes from the URC stream (single source
                            // of truth), never from a racing query.
                            //
                            // try_next_message_pure() is non-blocking (returns None
                            // when the buffer is empty), verified present on atat's
                            // UrcSubscription at rev edceb1a (embassy Subscriber,
                            // rev 24da56d).
                            while let Some(urc) = sub.try_next_message_pure() {
                                match urc {
                                    // The one we're here for.
                                    ModemUrc::Cereg(body) => {
                                        registered = cereg_registered(body.as_str());
                                        crate::display::status::set_registered(registered);
                                    }
                                    // Teardown URCs: the link/modem dropped during
                                    // bring-up. Continuing to poll would bring a
                                    // stack up on a dead link. Abort and let the
                                    // outer loop re-drive from Down -- same effect
                                    // the main URC arm has for these.
                                    ModemUrc::SocketClosed(_)
                                    | ModemUrc::RebootHost
                                    | ModemUrc::RebootReset
                                    | ModemUrc::RebootWD(..)
                                    | ModemUrc::SysStart => {
                                        warn!("teardown URC during bring-up poll - aborting");
                                        abort_bringup = true;
                                    }
                                    ModemUrc::IPStackUpdate(cgev) => match cgev.event() {
                                        ip_stack::CgevEvent::NwDetach
                                        | ip_stack::CgevEvent::MeDetach
                                        | ip_stack::CgevEvent::MePdnDeact(_)
                                        | ip_stack::CgevEvent::NwPdnDeact(_) => {
                                            warn!("detach URC during bring-up poll - aborting");
                                            abort_bringup = true;
                                        }
                                        // An attach event only helps us; the poll is
                                        // already heading to IpUp, so nothing to do.
                                        _ => {}
                                    },
                                    // Irrelevant to the searching decision and the
                                    // poll is transient: safe to drop. (MqttReceived
                                    // can't occur pre-connect anyway.)
                                    _ => {}
                                }
                            }
                            if abort_bringup {
                                break;
                            }

                            COMMAND_CHANNEL
                                .send(command_task::ModemCommand::GetPdpAddress(
                                    CgPaddrQuery::default(),
                                ))
                                .await;
                            let pdp = communication::PDP_ADDRESS_RESULT.wait().await;

                            let now_registered = registered;

                            match pdp {
                                Ok(true) if now_registered => {
                                    // One-shot signal-quality read for the log. Purely
                                    // diagnostic; a failed query must not hold up bring-up.
                                    COMMAND_CHANNEL
                                        .send(command_task::ModemCommand::CesqQuery(
                                            cesq::CesqQuery,
                                        ))
                                        .await;
                                    match communication::CESQ_RESULT.wait().await {
                                        Ok(q) => {
                                            crate::display::status::set_rsrp(q.rsrp_dbm);
                                            match q.rsrp_dbm {
                                                Some(dbm) => info!(
                                                    "signal: RSRP {} dBm (rsrq idx {:?})",
                                                    dbm, q.rsrq_index
                                                ),
                                                None => info!(
                                                    "signal: RSRP unknown (idx {:?})",
                                                    q.rsrp_index
                                                ),
                                            }
                                        }
                                        Err(e) => warn!("CESQ query failed: {:?}", e),
                                    }

                                    info!("PDP context active - seeding IpUp");
                                    search_failures = 0;
                                    state_sender.send(state::MqttStackState::IpUp);
                                    seeded = true;
                                    break;
                                }
                                Ok(true) => {
                                    // PDP context is up with a valid IP, but the
                                    // tracked CEREG stat still says 2 (searching). On
                                    // this Soracom/roaming setup that state can stay
                                    // frozen for tens of minutes while the data bearer
                                    // is already up and stable -- observed: the same IP
                                    // (10.229.109.102) held rock-steady across a 100s+
                                    // poll with CGPADDR returning OK every time and NO
                                    // +CEREG URC ever arriving to flip the stat. A modem
                                    // with no coverage loses the PDP context and the IP;
                                    // this one keeps both. Waiting on CEREG here means
                                    // never bringing MQTT up despite a usable bearer.
                                    //
                                    // So treat a valid PDP address as the go signal and
                                    // let the SOCKETCREATE + TLS connect be the real
                                    // liveness probe. CGPADDR is a direct query with a
                                    // reliable response, unlike the URC-tracked CEREG
                                    // flag which can miss the edge that would clear it.
                                    // If the bearer isn't actually routable the create
                                    // fails and we drop to Down exactly as today -- one
                                    // attempt per poll pass (the break below), not a 3s
                                    // busy-loop.
                                    info!(
                                        "PDP active, CEREG still searching - seeding IpUp anyway (socket is the probe)"
                                    );
                                    search_failures = 0;
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
                        if abort_bringup {
                            // A teardown/detach URC surfaced mid-poll: the link
                            // died during bring-up. Drop to Down so a fresh
                            // registration edge re-drives, and do NOT count this as
                            // a "dead poll" -- it isn't a stuck-searching modem, the
                            // network went away underneath us.
                            state_sender.send(state::MqttStackState::Down);
                        } else if !seeded {
                            // Distinguish "still searching" (stat 2, wedged --
                            // resettable) from a plain missing PDP (wait for the
                            // CGEV edge as before). Only the former escalates.
                            if registered {
                                // Shouldn't happen: registered but not seeded.
                                warn!("registered but PDP poll did not seed - waiting for CGEV edge");
                            } else {
                                search_failures = search_failures.saturating_add(1);
                                // NB-IoT cold attach here (roaming onto Vodafone,
                                // indoors) can take tens of minutes -- observed ~45
                                // min before it finally camped. This branch used to
                                // AT#RESET=0 after ~2 dead polls (~4 min), but a reset
                                // reboots the modem and DISCARDS all acquisition
                                // progress, restarting the cold attach from zero. Any
                                // attach slower than that budget could therefore never
                                // complete -- the modem reset itself mid-search in a
                                // loop, which is exactly the endless "PDP query not
                                // ready" behaviour seen on hardware. So do NOT reset.
                                // Drop to Down and let the modem keep searching
                                // autonomously; when it finally attaches, the
                                // registration edge (+CEREG stat 5 / +CGEV ME PDN ACT
                                // 5, both handled while Down) re-drives bring-up via
                                // the IpUp block above. Slow, but it completes rather
                                // than sabotaging itself.
                                warn!(
                                    "still searching after full poll (attempt {}) - waiting for attach, NOT resetting",
                                    search_failures
                                );
                                state_sender.send(state::MqttStackState::Down);
                            }
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