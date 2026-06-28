//! Feature-gated network/MQTT mock for running on a board with no modem.
//!
//! Enabled with `--features mock_modem` (which also pulls in `mock_gnss`).
//! Replaces the real `network_task` + `command_task` + UART/atat stack. It
//! fakes the happy-path MQTT lifecycle by driving `MQTT_STATE` and the result
//! signals exactly as the real stack would, so `modem_task` runs unchanged:
//!
//! * `MqttCommand::Start`       -> Down -> IpUp -> SocketReady -> MqttReady
//! * `MqttCommand::Subscribe`   -> `SUBSCRIBE_RESULT = Ok(topic)`
//! * `MqttCommand::Unsubscribe` -> `UNSUBSCRIBE_RESULT = Ok(topic)`
//! * `MqttCommand::Stop`        -> MQTT_STATE = Down
//! * `ModemCommand::MqttPublish` (from `modem_task` via `COMMAND_CHANNEL`)
//!                              -> logs the payload, `PUBLISH_RESULT = Ok(())`
//!
//! Under `mock_modem` this task is the sole consumer of `COMMAND_CHANNEL`
//! (command_task is not spawned) and `modem_task` only ever sends `MqttPublish`
//! on it, so the catch-all arm should not normally fire.

use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};

use crate::modem::command_task::ModemCommand;
use crate::modem::communication;
use crate::modem::mqtt::state::MqttStackState;
use crate::modem::network_task::MqttCommand;

/// Simulated time for the stack to come up (Start -> MqttReady).
const MOCK_BRINGUP: Duration = Duration::from_secs(2);
/// Simulated subscribe / publish round-trip.
const MOCK_RTT: Duration = Duration::from_millis(300);

#[embassy_executor::task]
pub async fn network_mock_task() -> ! {
    let state = communication::MQTT_STATE.sender();
    state.send(MqttStackState::Down);
    warn!("MOCK NETWORK task spawned — no real modem / broker in use");

    loop {
        match select(
            communication::MQTT_COMMAND.wait(),
            communication::COMMAND_CHANNEL.receive(),
        )
        .await
        {
            Either::First(cmd) => match cmd {
                MqttCommand::Start => {
                    Timer::after(MOCK_BRINGUP).await;
                    state.send(MqttStackState::IpUp);
                    state.send(MqttStackState::SocketReady(0));
                    state.send(MqttStackState::MqttReady);
                    info!("MOCK mqtt ready");
                }
                MqttCommand::Subscribe(sub) => {
                    Timer::after(MOCK_RTT).await;
                    communication::SUBSCRIBE_RESULT.signal(Ok(sub.topic));
                    info!("MOCK subscribed");
                }
                MqttCommand::Unsubscribe(topic) => {
                    Timer::after(MOCK_RTT).await;
                    communication::UNSUBSCRIBE_RESULT.signal(Ok(topic));
                }
                MqttCommand::Stop => {
                    state.send(MqttStackState::Down);
                    info!("MOCK mqtt stopped");
                }
            },
            Either::Second(cmd) => match cmd {
                ModemCommand::MqttPublish(p) => {
                    Timer::after(MOCK_RTT).await;
                    info!("MOCK publish: topic={}, payload={}", p.topic(), p.message());
                    communication::PUBLISH_RESULT.signal(Ok(()));
                }
                _ => warn!("MOCK network: ignoring unexpected modem command"),
            },
        }
    }
}
