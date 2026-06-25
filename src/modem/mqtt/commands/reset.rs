use atat::atat_derive::AtatCmd;

use crate::modem::mqtt::commands::OkResponse;

/// `AT#RESET=<type>` — reboot the module.
///
/// `<type>` = 0 resets *without* saving the latest custom command updates,
/// 1 saves them first, 3 performs a FOTA roll back (AT manual §4.22). We use 0:
/// a plain reboot that leaves provisioned config (security profiles, CTZR,
/// SLEEPMODE) untouched.
///
/// This is the recovery for a wedged TCP socket — the modem reports the socket
/// as open (`AT#SOCKETCREATE?` lists it) yet refuses to close it (`+CME ERROR:
/// 2104`, invalid socket id) while it still occupies the single TCP slot
/// (`+CME ERROR: 2159`, max sockets reached on create). That state is held on
/// the modem and survives an RP2350 reflash; only a modem reboot/power-cycle
/// clears it. The reboot emits a `#REBOOT_HOST` URC, which `network_task`
/// already handles by tearing the MQTT stack back down to `Down`.
///
/// Note: the module may reboot before emitting `OK`, so the caller should treat
/// a timeout/error from this command as success and proceed to wait for the
/// reboot URCs.
#[derive(Clone, AtatCmd)]
#[at_cmd("#RESET", OkResponse, timeout_ms = 3000)]
pub struct ModemReset {
    pub reset_type: u8, // 0 = reset without saving custom command updates
}

impl Default for ModemReset {
    fn default() -> Self {
        Self { reset_type: 0 }
    }
}
