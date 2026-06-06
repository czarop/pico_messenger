use defmt::error;
use embassy_futures::select::select;
use embassy_time::{Duration, with_timeout};

use crate::modem::{
    UpdateIntervalSecs, communication,
    gnss::state::GNSSState,
    mqtt::{
        commands::{MqttQos, subscribe::MqttSubscribe},
        state::MqttStackState,
    },
};

const READY_TIMEOUT: Duration = Duration::from_secs(120);

#[embassy_executor::task]
pub async fn modem_task(
    gnss_interval: UpdateIntervalSecs,
    mqtt_interval: UpdateIntervalSecs,
    mqtt_topic: heapless::String<50>,
) -> ! {
    let mut mqtt_watcher = communication::MQTT_STATE.receiver().unwrap();
    let mut gnss_watcher = communication::GNSS_STATE.receiver().unwrap();

    communication::MQTT_COMMAND.signal(crate::modem::network_task::MqttCommand::Start);
    let gnss_init_params = crate::modem::gnss::commands::init::GnssInit::default();
    let delay_between_readings = gnss_interval;
    communication::GNSS_COMMAND.signal(crate::modem::gnss_task::GnssCommand::Start(
        gnss_init_params,
        Some(delay_between_readings),
    ));

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
        Ok(_) => {}
        Err(TimeoutError) => {
            let mqtt_ok = matches!(mqtt_watcher.try_get(), Some(MqttStackState::MqttReady));
            let gnss_ok = matches!(gnss_watcher.try_get(), Some(GNSSState::Fix(_)));

            match (mqtt_ok, gnss_ok) {
                (true, false) => { /* MQTT up, no fix — publish "alive, no fix" */ }
                (false, _) => { /* MQTT failed — store fix if we have one, retry next cycle */ }
                _ => { /* both failed — sleep and retry */ }
            }
        }
    }

    loop {}
}
