use crate::modem::error::ModemError;
use crate::modem::gnss_task::GnssCommand;
use crate::modem::mqtt::commands::{cereg, cesq};
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

/// Set by `network_task` when it resets the modem during teardown (a failed
/// MQTTDISC leaves a wedged socket that only a reset clears). `modem_task`
/// checks and clears this before attempting PSM: entering PSM into a rebooting
/// modem is pointless (SLEEPMODE would just fail its #SLEEP wait and burn ~45s).
/// The modem re-attaches on its own after the reboot, ready for the next cycle.
pub static MODEM_RESET_ON_TEARDOWN: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Result of `ModemCommand::EnterPsm` / `ModemCommand::ExitPsm`.
///
/// Shared by both directions because PSM transitions are strictly serialised:
/// `modem_task` is the only caller, and it never has an enter and an exit in
/// flight at once. `psm::enter_psm`/`exit_psm` `reset()` this before sending, so
/// a result abandoned by an earlier timeout cannot satisfy the next wait.
///
/// `Ok(())` from an enter means the `#SLEEP` URC was observed -- not merely that
/// `AT#SLEEPMODE` returned `OK`. Never sleep the RP2350 on anything weaker.
pub static PSM_RESULT: Signal<CriticalSectionRawMutex, Result<(), ModemError>> = Signal::new();

// Issue commands to the modem task
pub static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, ModemCommand, 4> = Channel::new();

//Issue commands from app_task to control gnss
pub static GNSS_COMMAND: Signal<CriticalSectionRawMutex, GnssCommand> = Signal::new();
pub static MQTT_COMMAND: Signal<CriticalSectionRawMutex, MqttCommand> = Signal::new();

/// Result of an `AT+CEREG?` query.
///
/// Needed because the registration flag maintained from `+CEREG` URCs goes stale
/// inside `network_task`'s CGPADDR poll: that loop runs in the monitor's command
/// branch and never reaches the URC arm, so a registration that completes DURING
/// the poll is invisible to it. Querying is a command, so it works there.
/// Result of an `AT+CESQ` signal-quality query. Diagnostic only: logged at
/// bring-up so a marginal link (registered but too weak to hold a TLS MQTT
/// session) is visible as a number rather than inferred from cell reselection.
pub static CESQ_RESULT: Signal<CriticalSectionRawMutex, Result<cesq::CesqStatus, ModemError>> =
    Signal::new();

/// Fired once by `command_task` when the one-shot network diagnostic
/// (`ModemCommand::NetDiag`) has finished. Carries no data -- the diagnostic
/// logs its findings over defmt; this only unblocks `network_task` so bring-up
/// can proceed.
pub static DIAG_RESULT: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub static CEREG_RESULT: Signal<CriticalSectionRawMutex, Result<cereg::CeregStatus, ModemError>> =
    Signal::new();