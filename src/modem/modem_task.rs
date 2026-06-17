use defmt::{error, info, warn};
use embassy_futures::select::select;
use embassy_time::{Duration, with_timeout};

use crate::modem::{
    UpdateIntervalSecs, command_task::ModemCommand, communication::{self, COMMAND_CHANNEL, PUBLISH_RESULT}, gnss::{commands::init::GnssInit, state::GNSSState}, gnss_task::GnssCommand, mqtt::{
        commands::{MqttQos, publish::MqttPublish, subscribe::MqttSubscribe}, payload::LocationPayload, state::MqttStackState
    }
};

const READY_TIMEOUT: Duration = Duration::from_secs(120);

#[embassy_executor::task]
pub async fn modem_task(
    gnss_interval: UpdateIntervalSecs,
    mqtt_topic: heapless::String<50>,
) -> ! {
    defmt::info!("modem task spawned");
    let mut mqtt_watcher = communication::MQTT_STATE.receiver().unwrap();
    let mut gnss_watcher = communication::GNSS_STATE.receiver().unwrap();
    loop {
    // wait for modem ready before starting GNSS
    embassy_time::Timer::after(Duration::from_secs(10)).await;

    // communication::MQTT_COMMAND.signal(crate::modem::network_task::MqttCommand::Start);
    
    
    if matches!(gnss_watcher.try_get(), Some(GNSSState::Off) | None) {
        communication::GNSS_COMMAND.signal(GnssCommand::Start(
            GnssInit::default(),
            Some(gnss_interval),
        ));
    }
    
    

    match with_timeout(READY_TIMEOUT, async {
        let mut subscribed = false;
        loop {
            let gnss_ready = matches!(gnss_watcher.try_get(), Some(GNSSState::Fix(_)));

            if matches!(mqtt_watcher.try_get(), Some(MqttStackState::MqttReady)) && !subscribed {
                let mqtt_topic_subscribe = MqttSubscribe {
                    topic: mqtt_topic.clone(),
                    qos: MqttQos::AtLeastOnce,
                };
                communication::MQTT_COMMAND.signal(
                    crate::modem::network_task::MqttCommand::Subscribe(mqtt_topic_subscribe),
                );
                match communication::SUBSCRIBE_RESULT.wait().await {
                    Ok(_) => {
                        subscribed = true;
                    }
                    Err(e) => {
                        error!("Subscribe failed: {:?}", e);
                        // subscribed stays false — will retry on next loop iteration
                        // when MqttReady is still set
                    }
                }
            }

            if subscribed && gnss_ready {
                break;
            }

            // sleep until either state changes, then re-check both
            select(mqtt_watcher.changed(), gnss_watcher.changed()).await;
        }
    })
    .await
    {
        Ok(_) => {
            // add to publish gnss and sleep
            if let Some(GNSSState::Fix(loc)) = gnss_watcher.try_get() {
                let is_moving = true;
                warn!("add access to sensor data, placeholder provided");
                let payload = LocationPayload::from_gnss(loc, is_moving);
                let encoded = payload.to_base64();
                
                
                let publish = MqttPublish::new(
                    mqtt_topic.clone(),
                    encoded,
                );
                
                COMMAND_CHANNEL.send(ModemCommand::MqttPublish(publish)).await;
                
                match PUBLISH_RESULT.wait().await {
                    Ok(_) => {
                        info!("Published successfully");
                        embassy_time::Timer::after(Duration::from_secs(gnss_interval as u64)).await;
                    },
                    Err(e) => {
                        error!("Publish failed: {:?}", e);
                        warn!("implement retry loop");
                    },
                }
            }
        }
        Err(TimeoutError) => {
            let mqtt_ok = matches!(mqtt_watcher.try_get(), Some(MqttStackState::MqttReady));
            let gnss_ok = matches!(gnss_watcher.try_get(), Some(GNSSState::Fix(_)));

            match (mqtt_ok, gnss_ok) {
                (true, false) => { /* MQTT up, no fix — publish "alive, no fix" */ }
                (false, _) => { /* MQTT failed — store fix if we have one, retry next cycle */ }
                _ => { /* both failed — sleep and retry */ }
            }

            // add to sleep
            embassy_time::Timer::after(Duration::from_secs(gnss_interval as u64)).await;
        }
    }
}

    
}
