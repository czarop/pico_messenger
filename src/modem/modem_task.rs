use defmt::{error, info, warn};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Receiver};
use embassy_time::{Duration, Timer, with_timeout};

use crate::modem::{
    UpdateIntervalSecs, command_task::ModemCommand, communication::{self, COMMAND_CHANNEL, GNSS_COMMAND, PUBLISH_RESULT}, gnss::{commands::init::GnssInit, speed::FixPair, state::GNSSState, urc::fix::GnssLocation}, gnss_task::GnssCommand, mqtt::{
        commands::publish::MqttPublish, payload::LocationPayload, state::MqttStackState
    }, network_task::MqttCommand
};


/// Backstop for the whole bring-up block. The two inner timeouts below bound
/// their own work, so this only catches something hanging BETWEEN them. They run
/// concurrently, so this is `max(inner)` plus slack, not the sum.
const READY_TIMEOUT: Duration = Duration::from_secs(100);

/// Budget for the two-fix GNSS burst. Must cover a cold fix (no valid ephemeris)
/// plus GNSS_FIX_PERIOD_SECS for the second fix.
const GNSS_FIX_TIMEOUT: Duration = Duration::from_secs(90);

/// Budget for MQTT connect + subscribe. MUST stay above ~70s: `MQTTCONNECT` uses
/// a 60s connection_timeout with a 70s atat timeout, so a shorter value here
/// would abort connects that were still legitimately in progress.
const MQTT_READY_TIMEOUT: Duration = Duration::from_secs(80);

/// Consecutive cycles with no GNSS fix before the device is treated as unable to
/// see the sky (indoors, under cover). Reported in the heartbeat.
const NO_FIX_STRIKES: u8 = 3;

/// Sent once, when the strike limit is hit and the device parks itself.
const NO_FIX_PARKED_MSG: &str = "no gnss fix - sleeping until motion";

/// Message published when the device is alive and connected but cannot fix.
const NO_FIX_MSG: &str = "alive, no GNSS fix";
/// Seconds between the two GNSS fixes used to compute speed (the `#GNSSFIX`
/// `<period>`). Short, decoupled from the publish cadence. ~3s gives a usable
/// vehicle-speed baseline; lengthen toward 5s if pedestrian speed matters.
const GNSS_FIX_PERIOD_SECS: UpdateIntervalSecs = 3;
const GNSS_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// Wake interval, in whole minutes, armed on the RTC before each dormant sleep.
///
/// This is the tracking cadence. The RTC countdown resolution is 1 minute (1/60 Hz
/// source), so this is a `u8` of minutes, 1..=255. When the regime state machine
/// lands, this becomes per-regime (ACTIVE=15, STATIONARY_PENDING=10) rather than a
/// single constant.
///
/// `gnss_interval` (the old `Timer::after` seconds value) is now used only on the
/// enter_psm-failure fallback path, where no real sleep happens.
const SLEEP_MINUTES: u8 = 1;


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
async fn wait_ready(rx: &mut Receiver<'_, CriticalSectionRawMutex, MqttStackState, 3>) {
    // Subscription intentionally removed: the device only publishes, so
    // subscribing to its own topic served no purpose and made the broker echo
    // every publish straight back via `#MQTTRECV` (visible in the logs, and a
    // waste of airtime). Re-add a subscribe here if downlink is ever needed.
    loop {
        if matches!(rx.try_get(), Some(MqttStackState::MqttReady)) {
            return;
        }
        rx.changed().await;
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

/// Full cycle length. Every location update is scheduled this far apart,
/// regardless of what happens in between.
const CYCLE_SECS: u32 = SLEEP_MINUTES as u32 * 60;

/// How long a stationary device waits for motion before declaring itself parked
/// and dropping into the indefinite motion-only sleep.
const PROBE_SECS: u32 = 5 * 60;

/// Message published once, just before entering indefinite sleep.
const NOT_MOVING_MSG: &str = "device not moving";

/// Bring MQTT up, publish a one-off text message, and tear the session down.
///
/// The normal cycle has already stopped MQTT and put the modem into PSM by the
/// time the probe times out, so this has to re-establish the session. Used only
/// for the "not moving" notice, which is rare by definition.
async fn publish_status(
    rx: &mut Receiver<'_, CriticalSectionRawMutex, MqttStackState, 3>,
    topic: heapless::String<50>,
    text: &str,
) -> bool {
    if let Err(e) = crate::modem::psm::exit_psm().await {
        error!("status publish: exit_psm failed: {:?}", e);
    }

    communication::MQTT_COMMAND.signal(MqttCommand::Start);
    if with_timeout(READY_TIMEOUT, wait_ready(rx))
        .await
        .is_err()
    {
        warn!("status publish: MQTT not ready before timeout");
        communication::MQTT_COMMAND.signal(MqttCommand::Stop);
        return false;
    }

    let Ok(message) = heapless::String::<50>::try_from(text) else {
        error!("status publish: message too long for String<50>");
        communication::MQTT_COMMAND.signal(MqttCommand::Stop);
        return false;
    };

    let ok = publish_once(topic, message, "status").await;

    // Same ordering rule as the main cycle: Stop is fire-and-forget, so we must
    // wait for the teardown to actually reach Down before asking the modem to
    // sleep. Otherwise AT#SLEEPMODE races the outstanding AT#MQTTDISC — the modem
    // enters PSM, then the late MQTTDISC arrives and immediately wakes it again
    // (#WAKEUP), leaving it awake for the whole of what should be an indefinite
    // motion-only rest.
    communication::MQTT_COMMAND.signal(MqttCommand::Stop);
    if with_timeout(MQTT_DOWN_TIMEOUT, async {
        loop {
            if let MqttStackState::Down = rx.changed().await {
                break;
            }
        }
    })
    .await
    .is_err()
    {
        warn!("status publish: MQTT teardown did not reach Down — sleeping anyway");
    }

    if let Err(e) = crate::modem::psm::enter_psm().await {
        error!("status publish: enter_psm failed: {:?}", e);
    }
    ok
}

/// Seconds left in the current cycle, measured against the RTC calendar so the
/// schedule holds regardless of how long the cycle's work took. Falls back to a
/// full period if the clock is unreadable.
async fn secs_left(cycle_start: Option<u32>) -> u32 {
    match (cycle_start, crate::rtc::now_secs_of_day().await) {
        (Some(a), Some(b)) => CYCLE_SECS.saturating_sub(crate::rtc::elapsed_secs(a, b)),
        _ => CYCLE_SECS,
    }
}

/// Ceiling on waiting for the MQTT stack to reach `Down` before sleeping.
///
/// MUST be bounded. `network_task` only reports `Down` once its teardown
/// completes, but if a connect is failing it sits in a rebuild loop (each attempt
/// up to the 60s connection timeout) and may never pass through `Down` at all.
/// An unbounded wait here parks `modem_task` permanently: no sleep, no further
/// cycles, no logs — the device just goes quiet until reset.
const MQTT_DOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Host-side ceiling on waiting for a publish result. Top of the timeout
/// ladder: modem PUBACK window (60s, #MQTTCFG protocol_timeout) < atat
/// `#MQTTPUB` timeout (70s) < this (75s), so each layer reads the layer
/// below's real answer rather than cutting it off early.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(75);

enum PublishOutcome {
    Ok,
    Timeout,
    Rejected,
}

/// Send one publish and wait, distinguishing a genuine rejection from a mere
/// host-side timeout. Resets the result signal first so a late result from a
/// previous publish can't be misread as this one's.
async fn publish_and_wait(topic: heapless::String<50>, message: heapless::String<50>) -> PublishOutcome {
    PUBLISH_RESULT.reset();
    COMMAND_CHANNEL
        .send(ModemCommand::MqttPublish(MqttPublish::new(topic, message)))
        .await;
    match with_timeout(PUBLISH_TIMEOUT, PUBLISH_RESULT.wait()).await {
        Ok(Ok(())) => PublishOutcome::Ok,
        Ok(Err(e)) => {
            error!("publish error: {:?}", e);
            PublishOutcome::Rejected
        }
        Err(_) => PublishOutcome::Timeout,
    }
}

/// Publish once. No retry on timeout: at QoS 1 a timeout almost always means the
/// message DID reach the broker but the modem was slow to confirm locally, so a
/// retry only delivers a duplicate. Send once; timeout is treated as
/// probably-delivered, an explicit rejection as a real drop. The next cycle is
/// minutes away and the payload is not buffered, so a genuine drop is a gap not a
/// corruption. Returns whether the message is believed delivered.
async fn publish_once(topic: heapless::String<50>, message: heapless::String<50>, label: &str) -> bool {
    let delivered = match publish_and_wait(topic, message).await {
        PublishOutcome::Ok => {
            info!("{} published", label);
            true
        }
        PublishOutcome::Timeout => {
            warn!("{} result timed out — treating as sent (QoS0), not retrying", label);
            true
        }
        PublishOutcome::Rejected => {
            error!("{} rejected — dropped", label);
            false
        }
    };
    // Status panel: one call here covers every publish path.
    crate::display::status::note_publish(delivered);
    delivered
}

/// Sleep sensor and host for `secs`, host waking on the RTC.
///
/// The ack matters: dormanting while the BNO085 sleep commands are still in
/// flight would cut them off mid-sequence.
async fn rest_on_rtc(secs: u32) {
    use crate::sensors::bno085::bno085::{ENTER_SLEEP, SENSOR_ASLEEP, SleepMode, WAKE_SENSOR};

    if secs == 0 {
        return;
    }
    crate::display::status::note_sleep_entry(
        crate::display::status::SleepPhase::Resting,
        crate::rtc::now_secs_of_day().await,
    );
    SENSOR_ASLEEP.reset();
    ENTER_SLEEP.signal(SleepMode::SensorOnly);
    SENSOR_ASLEEP.wait().await;

    // Closed-loop sleep: measures against the RTC calendar and re-arms, so the
    // cadence is accurate to ~1s and does not drift (a single arm fires anywhere
    // in (N-1, N] minutes).
    crate::power::sleep_for_secs(secs).await;

    WAKE_SENSOR.signal(());
}

/// Consecutive no-fix cycles. RAM only: a reboot is itself a reason to start
/// fresh, and this never needs to outlive one power cycle.
static NO_FIX_COUNT: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

#[embassy_executor::task]
pub async fn modem_task(
    gnss_interval: UpdateIntervalSecs,
    mqtt_topic: heapless::String<50>,
    status_topic: heapless::String<50>,
) -> ! {
    info!("modem task spawned");
    let mut mqtt_watcher = communication::MQTT_STATE.receiver().unwrap();
    let mut gnss_watcher = communication::GNSS_STATE.receiver().unwrap();
 
    loop {
        // Anchor for the cadence. Every update is scheduled CYCLE_SECS after this
        // point, so a probe or an early motion wake shortens the following sleep
        // rather than shifting the schedule.
        let cycle_start = crate::rtc::now_secs_of_day().await;
        crate::display::status::set_sleep(crate::display::status::SleepPhase::Awake);

        // wait for modem ready before starting GNSS
        Timer::after(Duration::from_secs(12)).await;
 
        // Sequential, MQTT FIRST. The link is needed in BOTH outcomes -- to send
        // a location OR a no-fix heartbeat -- whereas a fix is only worth
        // acquiring if there is something to send it over. So if MQTT fails there
        // is no point powering the receiver at all. It also avoids running the
        // GNSS and NB-IoT radios at once, which contend for the shared RF front
        // end on the ST87M01.
        communication::MQTT_COMMAND.signal(MqttCommand::Start);
        let mqtt_ok = with_timeout(
            MQTT_READY_TIMEOUT,
            wait_ready(&mut mqtt_watcher),
        )
        .await
        .is_ok();
        if !mqtt_ok {
            warn!("MQTT not ready after {} s", MQTT_READY_TIMEOUT.as_secs());
        }

        // Sleep-regime selector, defaulted conservatively: if we can't get a fix
        // this cycle we stay on the RTC cadence rather than deep-resting blind.
        let mut is_moving = true;

        let fix_opt = if mqtt_ok {
            if matches!(gnss_watcher.try_get(), Some(GNSSState::Off) | None) {
                // NOTE: the fix burst period (seconds between the two fixes) is
                // the 2nd arg. It must be a SHORT value (~3s)
                communication::GNSS_COMMAND.signal(GnssCommand::Start(
                    GnssInit::default(),
                    Some(GNSS_FIX_PERIOD_SECS),
                ));
            }

            let r = with_timeout(GNSS_FIX_TIMEOUT, acquire_fix_pair(&mut gnss_watcher)).await;
            if r.is_err() {
                warn!("GNSS fix timed out after {} s", GNSS_FIX_TIMEOUT.as_secs());
            }
            // Stop the receiver before publishing so it is not drawing current or
            // contending for the RF front end while the modem transmits.
            stop_gnss(&mut gnss_watcher).await;
            r.ok()
        } else {
            warn!("skipping GNSS this cycle — nothing to publish over");
            None
        };

        use core::sync::atomic::Ordering;

        match (fix_opt, mqtt_ok) {
            // Both good: the normal path.
            (Some(fix_pair), true) => {
                NO_FIX_COUNT.store(0, Ordering::Relaxed);
                crate::display::status::note_fix();

                let speed = fix_pair.speed_mps();
                // is_moving: a measured non-zero speed means moving; Some(0.0) is
                // a confirmed-stationary reading; None means unmeasurable. Writes
                // the hoisted selector used at the sleep decision below.
                is_moving = matches!(speed, Some(s) if s > 0.0);

                // BNO085 fused heading, not GPS course-over-ground: it is valid
                // when stationary and does not need movement to be meaningful.
                // `None` when the sensor is asleep, has not reported yet, or the
                // reading is not accurate enough to trust — from_gnss then leaves
                // the heading-valid flag clear rather than publishing a guess.
                let heading = crate::sensors::bno085::bno085::latest_heading_deg();
                if heading.is_none() {
                    warn!("no reliable heading this cycle — publishing without it");
                }

                // Battery and temperature from the telemetry task's shared slots
                // — sampled while the host is awake, so at most a few seconds old
                // here. `None` means unread or a failed read, and the payload
                // leaves the corresponding validity bit clear.
                let battery = crate::sensors::telemetry::battery();
                let temperature = crate::sensors::telemetry::temperature_c10();

                let payload = LocationPayload::from_gnss(
                    fix_pair.end,
                    speed,
                    heading,
                    is_moving,
                    battery,
                    temperature,
                );

                publish_once(mqtt_topic.clone(), payload.to_base64(), "location").await;
                communication::MQTT_COMMAND.signal(MqttCommand::Stop);
            }

            // Connected but blind. Most likely indoors or under cover. Say so, so
            // the absence of location updates is distinguishable from a dead
            // device, and count the strike.
            (None, true) => {
                let strikes = NO_FIX_COUNT.load(Ordering::Relaxed).saturating_add(1);
                NO_FIX_COUNT.store(strikes, Ordering::Relaxed);
                crate::display::status::note_nofix(strikes);
                warn!("no GNSS fix ({} consecutive)", strikes);

                let mut text: heapless::String<50> = heapless::String::new();
                if core::fmt::write(
                    &mut text,
                    format_args!("{} ({}x)", NO_FIX_MSG, strikes),
                )
                .is_err()
                {
                    text.clear();
                    let _ = text.push_str(NO_FIX_MSG);
                }

                publish_once(status_topic.clone(), text, "no-fix heartbeat").await;

                // Strike limit: say so now, while the session is still up. Doing
                // it after the teardown would mean a second connect just for one
                // short message.
                if strikes >= NO_FIX_STRIKES {
                    if let Ok(parked) = heapless::String::<50>::try_from(NO_FIX_PARKED_MSG) {
                        publish_once(status_topic.clone(), parked, "parked notice").await;
                    }
                }

                communication::MQTT_COMMAND.signal(MqttCommand::Stop);
            }

            // No link. GNSS was never run, so this is not a no-fix strike — the
            // device may be able to see the sky perfectly well. Leave the counter
            // alone and retry next cadence.
            (_, false) => {
                warn!("no MQTT link — nothing published this cycle");
                communication::MQTT_COMMAND.signal(MqttCommand::Stop);
            }
        }

        // Strike limit reached: unable to see the sky for NO_FIX_STRIKES cycles
        // running. Stop burning a radio + GNSS session every cadence and wait to
        // be physically moved somewhere with a view.
        //
        // This bypasses the 5-minute motion probe: the probe exists to confirm a
        // MOVING device has settled, which is a different question from a device
        // that cannot fix where it is.
        let blind = NO_FIX_COUNT.load(Ordering::Relaxed) >= NO_FIX_STRIKES && mqtt_ok;
        if blind {
            warn!("no fix x{} — resting until motion", NO_FIX_STRIKES);
        }

        // Wait for network_task to finish tearing down (unsub -> disconnect ->
        // Down). MqttCommand::Stop is fire-and-forget, so without this the modem
        // would still hold a live MQTT session and open TCP socket when
        // AT#SLEEPMODE arrives — it answers OK and then does NOT sleep.
        if with_timeout(MQTT_DOWN_TIMEOUT, async {
            loop {
                if let MqttStackState::Down = mqtt_watcher.changed().await {
                    break;
                }
            }
        })
        .await
        .is_err()
        {
            // Almost always means the connect never succeeded, so there is no
            // session to tear down and nothing to wait for. Press on to PSM: a
            // modem left awake costs power, but hanging here costs everything.
            warn!("MQTT stack did not reach Down in {} s — sleeping anyway", MQTT_DOWN_TIMEOUT.as_secs());
        }

        // Modem into PSM. enter_psm awaits the #SLEEP URC (~7-12s: the modem can't
        // sleep until the network releases RRC), not merely the OK. A failure here
        // means the modem is still awake — don't dormant the host on top of that,
        // just skip sleeping this cycle and retry.
        match crate::modem::psm::enter_psm().await {
            Ok(()) => {
                use crate::sensors::bno085::bno085::{
                    ENTER_SLEEP, MOTION_WOKE, PROBE_RESULT, SleepMode, WakeSource,
                };

                // FORCE_STATIONARY is a TEMP test override — the mock GNSS always
                // reports moving, so without it the stationary path is never taken
                // in the mock. Set to `false` for production behaviour.
                const FORCE_STATIONARY: bool = true;
                let stationary = FORCE_STATIONARY || !is_moving;

                if blind {
                    // Cannot see the sky here. Only being moved changes that, so
                    // motion is the only wake source worth arming.
                    crate::display::status::note_sleep_entry(
                        crate::display::status::SleepPhase::DeepRest,
                        crate::rtc::now_secs_of_day().await,
                    );
                    crate::rtc::disarm_wake().await;
                    MOTION_WOKE.reset();
                    ENTER_SLEEP.signal(SleepMode::DeepRest);
                    MOTION_WOKE.wait().await;
                    info!("blind rest: motion woke us — retrying fix");
                    // Moved: clean slate, rather than parking again on strikes
                    // earned somewhere else.
                    NO_FIX_COUNT.store(0, Ordering::Relaxed);
                } else if !stationary {
                    // Moving: hold the cadence with sensor and host both down.
                    let secs = secs_left(cycle_start).await;
                    info!("moving: resting {} s to next update", secs);
                    rest_on_rtc(secs).await;
                } else {
                    // Stationary: probe for motion for up to PROBE_SECS, never
                    // running past the end of the cycle.
                    let probe = core::cmp::min(PROBE_SECS, secs_left(cycle_start).await);
                    info!("stationary: probing {} s for motion", probe);

                    crate::display::status::note_sleep_entry(
                        crate::display::status::SleepPhase::Probe,
                        crate::rtc::now_secs_of_day().await,
                    );
                    crate::rtc::arm_wake_secs(probe).await;
                    PROBE_RESULT.reset();
                    ENTER_SLEEP.signal(SleepMode::Probe);

                    match PROBE_RESULT.wait().await {
                        WakeSource::Motion => {
                            // Moving again. Per spec: no immediate publish, just
                            // serve out the rest of the cycle and carry on, so the
                            // 15-minute schedule is preserved.
                            let secs = secs_left(cycle_start).await;
                            info!("probe: motion — resting {} s to next update", secs);
                            rest_on_rtc(secs).await;
                        }
                        WakeSource::Rtc => {
                            // Parked. Announce it, then sleep indefinitely with
                            // motion as the only wake source.
                            info!("probe: no motion — going to indefinite rest");
                            publish_status(&mut mqtt_watcher, status_topic.clone(), NOT_MOVING_MSG).await;

                            crate::display::status::note_sleep_entry(
                                crate::display::status::SleepPhase::DeepRest,
                                crate::rtc::now_secs_of_day().await,
                            );
                            crate::rtc::disarm_wake().await;
                            MOTION_WOKE.reset();
                            ENTER_SLEEP.signal(SleepMode::DeepRest);

                            MOTION_WOKE.wait().await;
                            // Fall straight through to the next cycle, which
                            // publishes immediately.
                            info!("indefinite rest: motion woke us");
                        }
                    }
                }

                // Woken. Bring the modem back before the next cycle's AT traffic.
                if let Err(e) = crate::modem::psm::exit_psm().await {
                    error!(
                        "exit_psm failed: {:?} — continuing; next cmd may re-wake via UART",
                        e
                    );
                }
            }
            Err(e) => {
                warn!("enter_psm failed: {:?} — skipping sleep this cycle", e);
                // Modem stayed awake; fall through to a plain timed wait so we
                // don't spin. embassy_time still runs (no dormant happened).
                Timer::after(Duration::from_secs(gnss_interval as u64)).await;
            }
        }
    }
}