use defmt::{error, info, warn};
use embassy_futures::join::join;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Receiver};
use embassy_time::{Duration, Timer, with_timeout};

use crate::modem::{
    UpdateIntervalSecs, command_task::ModemCommand, communication::{self, COMMAND_CHANNEL, GNSS_COMMAND, PUBLISH_RESULT}, gnss::{commands::init::GnssInit, speed::FixPair, state::GNSSState, urc::fix::GnssLocation}, gnss_task::GnssCommand, mqtt::{
        commands::{MqttQos, publish::MqttPublish, subscribe::MqttSubscribe}, payload::LocationPayload, state::MqttStackState
    }, network_task::MqttCommand
};


const READY_TIMEOUT: Duration = Duration::from_secs(120);
const SUBSCRIBE_RETRY: Duration = Duration::from_secs(1);
/// Seconds between the two GNSS fixes used to compute speed (the `#GNSSFIX`
/// `<period>`). Short, decoupled from the publish cadence. ~3s gives a usable
/// vehicle-speed baseline; lengthen toward 5s if pedestrian speed matters.
const GNSS_FIX_PERIOD_SECS: UpdateIntervalSecs = 3;
const GNSS_STOP_TIMEOUT: Duration = Duration::from_secs(10);


/// Await the next genuine GNSS `Fix`, skipping transient states
/// (`Acquiring`, `Initialising`, ...). Caller must bound this with a timeout.
async fn next_fix(
    rx: &mut Receiver<'_, CriticalSectionRawMutex, GNSSState, 2>,
) -> GnssLocation {
    loop {
        if let GNSSState::Fix(f) = rx.changed().await {
            return f;
        }
    }
}

/// Acquire two consecutive fixes and pair them. Blocks until both arrive;
/// wrap in a timeout at the call site.
async fn acquire_fix_pair(
    rx: &mut Receiver<'_, CriticalSectionRawMutex, GNSSState, 2>,
) -> FixPair {
    let start = next_fix(rx).await;
    let end = next_fix(rx).await;
    FixPair { start, end }
}

/// Wait until the MQTT stack reports ready, then subscribe to `topic`.
/// Retries on subscribe failure with a small backoff. Completes only once
/// subscribed; the call-site timeout bounds a stack that never comes up.
async fn ready_and_subscribe(
    rx: &mut Receiver<'_, CriticalSectionRawMutex, MqttStackState, 3>,
    topic: heapless::String<50>,
) {
    loop {
        if matches!(rx.try_get(), Some(MqttStackState::MqttReady)) {
            let sub = MqttSubscribe {
                topic: topic.clone(),
                qos: MqttQos::AtLeastOnce,
            };
            communication::MQTT_COMMAND.signal(MqttCommand::Subscribe(sub));
            match communication::SUBSCRIBE_RESULT.wait().await {
                Ok(_) => {
                    info!("subscribed!");
                    return;
                }
                Err(e) => {
                    error!("Subscribe failed: {:?}", e);
                    Timer::after(SUBSCRIBE_RETRY).await;
                    // fall through and re-check readiness / retry
                }
            }
        } else {
            // not ready yet — wait for the next state change, then re-check
            rx.changed().await;
        }
    }
}

/// Stop GNSS and wait for it to confirm `Off` before sleeping, so the receiver
/// isn't drawing current through the sleep window. Bounded by a timeout: a
/// failed/slow deinit (real) or an unresponsive source can't hang the cycle.
/// No-op if already `Off` (gnss_task ignores a `Stop` in that state and would
/// never re-emit `Off`, so waiting on `changed()` would block forever).
async fn stop_gnss(rx: &mut Receiver<'_, CriticalSectionRawMutex, GNSSState, 2>) {
    if matches!(rx.try_get(), Some(GNSSState::Off)) {
        return;
    }
    communication::GNSS_COMMAND.signal(GnssCommand::Stop);
    let confirmed = with_timeout(GNSS_STOP_TIMEOUT, async {
        loop {
            if let GNSSState::Off = rx.changed().await {
                break;
            }
        }
    })
    .await;
    if confirmed.is_err() {
        warn!("GNSS did not confirm Off before timeout — sleeping anyway");
    }
}

#[embassy_executor::task]
pub async fn modem_task(gnss_interval: UpdateIntervalSecs, mqtt_topic: heapless::String<50>) -> ! {
    info!("modem task spawned");
    let mut mqtt_watcher = communication::MQTT_STATE.receiver().unwrap();
    let mut gnss_watcher = communication::GNSS_STATE.receiver().unwrap();
 
    loop {
        // wait for modem ready before starting GNSS
        Timer::after(Duration::from_secs(12)).await;
 
        // Kick BOTH subsystems off, then let them come up concurrently.
        communication::MQTT_COMMAND.signal(MqttCommand::Start);
 
        if matches!(gnss_watcher.try_get(), Some(GNSSState::Off) | None) {
            // NOTE: the fix burst period (seconds between the two fixes) is the
            // 2nd arg. It must be a SHORT value (~3s)
            communication::GNSS_COMMAND.signal(GnssCommand::Start(
                GnssInit::default(),
                Some(GNSS_FIX_PERIOD_SECS),
            ));
        }
 
        // Concurrent bring-up: run GNSS two-fix acquisition and MQTT
        // ready+subscribe at the same time, on this one task. `join` completes
        // only when BOTH have finished; the timeout bounds the whole thing.
        let outcome = with_timeout(
            READY_TIMEOUT,
            join(
                acquire_fix_pair(&mut gnss_watcher),
                ready_and_subscribe(&mut mqtt_watcher, mqtt_topic.clone()),
            ),
        )
        .await;
 
        match outcome {
            Ok((fix_pair, ())) => {
                GNSS_COMMAND.signal(GnssCommand::Stop);
                loop {
                    if let GNSSState::Off = gnss_watcher.changed().await {
                        break;
                    }
                }
                
                // Both ready: build the payload from the fix pair and publish.
                let speed = fix_pair.speed_mps();
                // is_moving: a measured non-zero speed means moving; Some(0.0) is
                // a confirmed-stationary reading; None means unmeasurable.
                let is_moving = matches!(speed, Some(s) if s > 0.0);
                warn!("heading placeholder (None) — wire BNO085 next");
 
                let payload = LocationPayload::from_gnss(fix_pair.end, speed, None, is_moving);
                
                for _ in 1..10 {
                    let publish = MqttPublish::new(mqtt_topic.clone(), payload.to_base64());
                    COMMAND_CHANNEL.send(ModemCommand::MqttPublish(publish)).await;
    
                    match PUBLISH_RESULT.wait().await {
                        Ok(_) => {
                            info!("Published successfully");
                            // Tear down the session (unsub -> disconnect -> close
                            // socket) before sleeping; next loop re-Starts against
                            // the still-active PDP context.
                            communication::MQTT_COMMAND.signal(MqttCommand::Stop);
                            break;
                        }
                        Err(e) => {
                            error!("Publish failed: {:?}", e);
                            warn!("implement retry loop");
                        }
                    }

                }
                
            }
            Err(_) => {
                // Timed out: one or both of GNSS / MQTT did not become ready.
                let mqtt_ok = matches!(mqtt_watcher.try_get(), Some(MqttStackState::MqttReady));
                let gnss_ok = matches!(gnss_watcher.try_get(), Some(GNSSState::Fix(_)));
                warn!("ready timeout (mqtt_ok={}, gnss_ok={})", mqtt_ok, gnss_ok);
 
                match (mqtt_ok, gnss_ok) {
                    (true, false) => { /* MQTT up, no fix — publish "alive, no fix" */ }
                    (false, _) => { /* MQTT failed — store fix if we have one, retry next cycle */ }
                    _ => { /* both failed — sleep and retry */ }
                }
                // ensure the session is town down before sleeping
                communication::MQTT_COMMAND.signal(MqttCommand::Stop);
            }
        }

        // Stop GNSS before sleeping so the receiver isn't draining current
        // through the sleep window. Covers both match arms — GNSS is started at
        // the top of the loop regardless of how the cycle ended.
        stop_gnss(&mut gnss_watcher).await;

        loop {
            if let MqttStackState::Down = mqtt_watcher.changed().await {
                break;
            }
        }

        crate::modem::psm::enter_psm().await.expect("Failed to enter psm");
        embassy_time::Timer::after(embassy_time::Duration::from_secs(300)).await;
        // crate::power::sleep_until(Duration::from_secs(gnss_interval as u64)).await;
        // crate::modem::psm::exit_psm().await.expect("Failed to exit psm");
 
        Timer::after(Duration::from_secs(gnss_interval as u64)).await;
    }
}