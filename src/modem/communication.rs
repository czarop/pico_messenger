use crate::modem::error::ModemError;
use crate::modem::gnss_task::GnssCommand;
use crate::modem::mqtt::commands::clock::ClockReady;
use crate::modem::mqtt::commands::socket::OpenSockets;
use crate::modem::network_task::MqttCommand;
use crate::modem::mqtt::state::MqttStackState;
use crate::{modem::gnss::state::GNSSState, modem::command_task::ModemCommand};
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
pub static SOCKET_QUERY_RESULT: Signal<CriticalSectionRawMutex, Result<OpenSockets, ModemError>> = Signal::new();
pub static CLOCK_RESULT: Signal<CriticalSectionRawMutex, Result<ClockReady, ModemError>> = Signal::new();
pub static GNSS_RESULT: Signal<CriticalSectionRawMutex, Result<(), ModemError>> = Signal::new();
pub static PUBLISH_RESULT: Signal<CriticalSectionRawMutex, Result<(), ModemError>> = Signal::new();
pub static SUBSCRIBE_RESULT: Signal<CriticalSectionRawMutex, Result<heapless::String<50>, ModemError>> = Signal::new();
pub static UNSUBSCRIBE_RESULT: Signal<CriticalSectionRawMutex, Result<heapless::String<50>, ModemError>> = Signal::new();

pub static PDP_ADDRESS_RESULT: Signal<CriticalSectionRawMutex, Result<bool, ModemError>> = Signal::new();

// Issue commands to the modem task
pub static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, ModemCommand, 4> = Channel::new();

//Issue commands from app_task to control gnss
pub static GNSS_COMMAND: Signal<CriticalSectionRawMutex, GnssCommand> = Signal::new();
pub static MQTT_COMMAND: Signal<CriticalSectionRawMutex, MqttCommand> = Signal::new();