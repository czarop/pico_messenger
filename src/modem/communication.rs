use crate::modem::error::ModemError;
use crate::modem::gnss::GnssCommand;
use crate::modem::network::MqttCommand;
use crate::mqtt::state::MqttStackState;
use crate::{gnss::state::GNSSState, modem::command::ModemCommand};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal, watch::Watch,
};

// holds the latest value. Any number of receivers can read it at any time
// State watches - holds latest value
pub static MQTT_STATE: Watch<CriticalSectionRawMutex, MqttStackState, 3> = Watch::new();
pub static GNSS_STATE: Watch<CriticalSectionRawMutex, GNSSState, 2> = Watch::new();

// single-use notification. One sender, one receiver, no history
// Result signals back from modem_task
pub static NETWORK_RESULT: Signal<CriticalSectionRawMutex, Result<(), ModemError>> = Signal::new();
pub static SOCKET_RESULT: Signal<CriticalSectionRawMutex, Result<u8, ModemError>> = Signal::new();
pub static GNSS_RESULT: Signal<CriticalSectionRawMutex, Result<(), ModemError>> = Signal::new();
pub static PUBLISH_RESULT: Signal<CriticalSectionRawMutex, Result<(), ModemError>> = Signal::new();

// Issue commands to the modem task
pub static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, ModemCommand, 4> = Channel::new();

//Issue commands from app_task to control gnss
pub static GNSS_COMMAND: Signal<CriticalSectionRawMutex, GnssCommand> = Signal::new();
pub static MQTT_COMMAND: Signal<CriticalSectionRawMutex, MqttCommand> = Signal::new();
