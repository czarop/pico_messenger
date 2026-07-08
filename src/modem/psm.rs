//! Modem Power Saving Mode (PSM) primitives.
//!
//! # Division of labour
//!
//! PSM on the ST87MXX is split across two lifetimes, because `AT#SLEEPMODE`'s
//! *parameters* are "SAVED to NVM using AT#RESET=1 / takes effect after module
//! reboot" (AT manual §4.30). You cannot enable sleep mode and use it in the
//! same session.
//!
//! * **Provision-time (once, then `AT#RESET=1`)** — see `src/bin/provision.rs`:
//!   - `AT+CPSMS=1,,,<T3412>,<T3324>`  network PSM contract
//!   - `AT#SLEEPMODE=1,<hold>,<awake>` enable modem-local sleep
//!   - `AT#WAKEUPEVENT=<pwrkey>,<uart>` arm the host->modem wake pin
//!   - `AT#SLEEPIND=0x44`               emit `#SLEEP`/`#WAKEUP`/`#ENERGY` URCs
//!                                      (NOT 0x7F -- see `SLEEPIND_PSM_AND_ENERGY`)
//!
//! * **Runtime (every wake cycle)** — this module:
//!   - enter: `AT#SLEEPMODE` (bare execution form) -> immediate sleep
//!   - exit:  pulse GPIO10 low -> modem wakes; confirm with a bare `AT`
//!
//! `CPSMS` is deliberately absent from this module. It requires empty middle
//! positional arguments (`AT+CPSMS=1,,,"...","..."`), and `serde_at` only
//! elides a *trailing* separator on `None` -- it cannot emit an empty middle
//! arg. Provisioning writes it over raw UART instead. See `provision.rs`.
//!
//! # Why the modem never fully powers off
//!
//! Challenger+ RP2350 NB-IoT datasheet §2.2: modem Sleep Current 1.2 uA,
//! Power-Off Current 0.5 uA. Full shutdown buys 0.7 uA and costs a cold
//! network re-attach (~160 s of radio, observed) plus a wiped GNSS assistance
//! cache on every wake. PSM everywhere, including DEEP_REST.

use atat::atat_derive::{AtatCmd, AtatResp};

use crate::modem::command_task::ModemCommand;
use crate::modem::communication::{COMMAND_CHANNEL, PSM_RESULT};
use crate::modem::error::ModemError;
use crate::modem::mqtt::commands::OkResponse;

// ---------------------------------------------------------------------------
// 3GPP timer encodings
// ---------------------------------------------------------------------------
//
// Both are one byte, rendered as an 8-character bit string:
//     bits 7..5 = unit selector, bits 4..0 = multiplier (0..31)
//
// They use *different unit tables*, so the SAME byte means different things
// depending on which parameter position it lands in. "00100100" is 4 HOURS as
// T3412 and 4 MINUTES as T3324. Do not share an encoder between them; do not
// swap the argument order.
//
// T3412 -- extended periodic TAU, GPRS Timer 3 (3GPP TS 24.008 10.5.7.4a):
//     000=10 min  001=1 hour  010=10 hours  011=2 s
//     100=30 s    101=1 min   110=320 hours 111=deactivated (unsupported here)
//   Manual's worked example: "01000111" = 010(10h) x 00111(7) = 70 hours. OK.
//
// T3324 -- active time, GPRS Timer 2 (3GPP TS 24.008 10.5.7.3):
//     000=2 s     001=1 min   010=6 min (decihour)   111=deactivated
//     (011/100/101/110 unsupported -- AT manual §6.9)
//   Manual's worked example: "00100100" = 001(1 min) x 00100(4) = 4 min. OK.

/// Requested extended periodic TAU (T3412): **4 hours**.
///
/// This is *not* the uplink cadence -- the RP2350 POWMAN timer owns that. T3412
/// only bounds how long the modem may stay radio-silent before it must wake
/// *itself* to perform a Tracking Area Update or be deregistered. Every uplink
/// we send resets it.
///
/// Long is strictly better for an uplink-only device: during ACTIVE_TRACKING our
/// 15-minute publishes reset it anyway, and during a multi-hour DEEP_REST park a
/// short value would burn radio on pointless TAUs all night. We have no downlink
/// reachability requirement while asleep, which is the only thing a short T3412
/// would buy.
///
/// Requested, not granted. The network decides; read back the granted value from
/// `AT+CEREG=4`/`=5` (`<Periodic-TAU>`), or observe the `#SLEEP PSM <secs>s` URC.
pub const T3412_REQUESTED: &str = "00100100"; // 001(1 hour) x 00100(4) = 4 h

/// Requested active time (T3324): **2 seconds**.
///
/// How long the modem stays reachable after the RRC connection releases, before
/// dropping into PSM. Short = prompt PSM entry. We tear the MQTT session down
/// before sleeping, so there is no in-flight downlink to wait for.
pub const T3324_REQUESTED: &str = "00000001"; // 000(2 s) x 00001(1) = 2 s

// ---------------------------------------------------------------------------
// Provision-time parameters
// ---------------------------------------------------------------------------

/// `AT#SLEEPMODE` `<hold_time>`: seconds between the last AT command and
/// *automatic* sleep entry.
///
/// Deliberately long. GNSS fixes arrive as `#GNSSFIX` URCs with no AT traffic to
/// keep the hold timer alive, so a short hold time would let the modem drop into
/// PSM in the middle of a fix acquisition. We never rely on auto-sleep -- entry
/// is always the explicit bare `AT#SLEEPMODE` at the end of a cycle. This value
/// exists only as a backstop long enough to never fire inside a normal cycle.
pub const SLEEP_HOLD_TIME_SECS: u32 = 300;

/// `AT#SLEEPMODE` `<awake_time>`: modem-side stuck-watchdog, in seconds.
///
/// If the module stays awake this long it resets itself. `0` disables it; values
/// must exceed 600 to arm the feature (AT manual §4.30). Disabled for now --
/// revisit alongside the RP2350 hardware watchdog in the robustness pass.
pub const SLEEP_AWAKE_TIME_SECS: u32 = 0;

/// `AT#WAKEUPEVENT` `<pwrkey_evt>` = `0b1111`.
///
/// Bitmap (AT manual §4.34): bit3 pull type (1 = pull-up), bit2 pull enable,
/// bit1 polarity (1 = logic LOW), bit0 enable. So: the wake pin is enabled,
/// **active low**, with an internal pull-up.
///
/// Board mapping (Challenger datasheet §3.2): RP2350 `GPIO10` -> ST87M01
/// `WAKE_UP` (pin 39). Drive GPIO10 low to wake the modem; idle high.
pub const WAKEUPEVENT_PWRKEY: u8 = 0b1111;

/// `AT#WAKEUPEVENT` `<uart_evt>` = `0b0011`: UART wake enabled, active low.
///
/// Retained as a fallback wake path (sending bytes wakes the modem) in case the
/// GPIO10 pulse proves unreliable. Harmless to have both armed.
pub const WAKEUPEVENT_UART: u8 = 0b0011;

/// `AT#SLEEPIND` bitmap = `0x44`: **PSM event (b2) + energy reporting (b6) only.**
///
/// This is what makes PSM observable: `#SLEEP` on entry, `#WAKEUP` on exit.
/// Without it, PSM entry is silent and unverifiable.
///
/// # Why not 0x7F, as the manual's example shows
///
/// atat's digester recognises a URC only if it matches `\r\n{token}(:.*)?\r\n`
/// (`atat::digest::parser::urc_helper`) -- i.e. the token must be followed
/// immediately by either a `:` or a CRLF.
///
/// * **b4 (verbosity)** turns the URC into `#SLEEP PSM 3599.9s`. The token is
///   followed by a *space*, which matches neither alternative. The line is not
///   recognised as a URC and is folded into the pending command's response
///   buffer -- the same failure mode as the unregistered `+CEREG` URC.
/// * **b5 (boot lib info)** emits a bare `NBIOT SW version ...` line with no `#`
///   prefix at all. Same problem.
///
/// Without b4 the URCs are bare `#SLEEP\r\n` / `#WAKEUP\r\n`, which match the
/// CRLF alternative; `#ENERGY: 414.7` matches the colon alternative.
///
/// The sleep duration lost with b4 was only ever a proxy for the network-granted
/// T3412. Read that directly instead: `AT+CEREG=4` (or `=5`) appends
/// `<Active-Time>,<Periodic-TAU>` to the registration URC.
///
/// Requires `Sleep`, `Wakeup` and `Energy` arms in `ModemUrc`.
///
/// Note the near-miss that the same strictness protects us from: `#SLEEP` is a
/// strict prefix of `#SLEEPMODE:` and `#SLEEPIND:`, and `#WAKEUP` of
/// `#WAKEUPEVENT:`. Because `urc_helper` demands `:` or CRLF straight after the
/// token, the URC arms cannot hijack those read-command responses.
pub const SLEEPIND_PSM_AND_ENERGY: u8 = 0x44; // b2 (PSM) | b6 (energy)

// ---------------------------------------------------------------------------
// Wake-pin timing
// ---------------------------------------------------------------------------

/// How long GPIO10 is held low to wake the modem.
///
/// UNVERIFIED: the AT manual specifies a pulse width for `#RINGPIN` (10-300 ms,
/// modem->host) but gives no minimum for the `WAKE_UP` pin (host->modem). 50 ms
/// is chosen to sit comfortably inside the `#RINGPIN` range. Confirm against a
/// scope or by observing `#WAKEUP` latency on hardware.
pub const WAKE_PULSE_MS: u64 = 50;

/// Delay after releasing the wake pin before the first `AT` ping.
pub const WAKE_SETTLE_MS: u64 = 100;

/// How many `AT` pings to attempt before declaring the modem unresponsive.
pub const WAKE_PING_ATTEMPTS: u8 = 10;

/// Delay between `AT` ping attempts.
pub const WAKE_PING_INTERVAL_MS: u64 = 100;

// ---------------------------------------------------------------------------
// Runtime API  (the only PSM entry points `modem_task` should ever call)
// ---------------------------------------------------------------------------
//
// The `mock_modem_psm` gate lives HERE and nowhere else.
//
// It must, because under `--features mock_modem` there is no `command_task` at
// all -- nothing consumes `COMMAND_CHANNEL`, so sending `ModemCommand::EnterPsm`
// would block forever. `mock_modem` therefore implies `mock_modem_psm`, and
// these functions short-circuit before touching the channel.
//
// On the test rig (`mock_modem_psm` with real modem hardware) the modem simply
// stays awake and attached, so the MQTT session and the debug UART survive what
// the state machine believes was a sleep. Everything above these two functions
// -- regime transitions, wake-source arming, timing -- still runs for real.

/// Put the modem into PSM. Blocks until `#SLEEP` is observed, or errors.
#[cfg(not(feature = "mock_modem_psm"))]
pub async fn enter_psm() -> Result<(), ModemError> {
    PSM_RESULT.reset();
    COMMAND_CHANNEL.send(ModemCommand::EnterPsm).await;
    PSM_RESULT.wait().await
}

/// Wake the modem from PSM. Blocks until the modem's UART answers, or errors.
#[cfg(not(feature = "mock_modem_psm"))]
pub async fn exit_psm() -> Result<(), ModemError> {
    PSM_RESULT.reset();
    COMMAND_CHANNEL.send(ModemCommand::ExitPsm).await;
    PSM_RESULT.wait().await
}

#[cfg(feature = "mock_modem_psm")]
pub async fn enter_psm() -> Result<(), ModemError> {
    defmt::info!("MOCK modem PSM: enter (modem stays awake and attached)");
    Ok(())
}

#[cfg(feature = "mock_modem_psm")]
pub async fn exit_psm() -> Result<(), ModemError> {
    defmt::info!("MOCK modem PSM: exit (modem was never asleep)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Provision-time raw AT strings
// ---------------------------------------------------------------------------
//
// `AT+CPSMS` needs empty *middle* positional arguments. `serde_at` only elides a
// trailing separator on `None`; it cannot emit an empty middle arg. So CPSMS is
// written as a literal over the blocking UART in `src/bin/provision.rs`, which
// is the only place it is ever needed.
//
// `<requested_periodic_RAU>` and `<requested_GPRS_READY_timer>` are "not
// supported in NB-IOT. No value will be output, and any input will be ignored."
// (AT manual 6.9), hence the empty positions. The two timer values are String
// type and MUST be quoted -- the manual's own worked example writes "01000111".

/// `AT+CPSMS=1,,,"<T3412>","<T3324>"` -- request network PSM.
///
/// DUPLICATION HAZARD: the two bit strings below must stay in step with
/// [`T3412_REQUESTED`] and [`T3324_REQUESTED`]. They are inlined because a byte
/// string literal cannot be assembled from `&str` consts in a `const` context,
/// and provisioning writes raw bytes. Change one, change the other.
///
/// Order matters more than usual here: swapping the two arguments is silent.
/// `"00100100"` is 4 HOURS in position 4 (GPRS Timer 3) and 4 MINUTES in
/// position 5 (GPRS Timer 2).
pub const CPSMS_ENABLE: &[u8] = b"AT+CPSMS=1,,,\"00100100\",\"00000001\"\r";

/// `AT+CPSMS=0` -- stop requesting PSM (parameters retained).
pub const CPSMS_DISABLE: &[u8] = b"AT+CPSMS=0\r";

/// `AT+CEREG=4` -- registration URCs that carry `<Active-Time>,<Periodic-TAU>`.
///
/// The *only* way to learn what the network actually granted: "To get the Active
/// Time value and the extended periodic TAU value that are allocated to the UE by
/// the network, use the command AT+CEREG" (AT manual 6.9).
///
/// `<stat>` remains the first field, so `network_task`'s `cereg_registered()`
/// parse is unaffected. The body grows though -- `ModemUrc::Cereg` already
/// reserves `String<96>` for exactly this.
pub const CEREG_PSM_MODE: &[u8] = b"AT+CEREG=4\r";

// ---------------------------------------------------------------------------
// AT command definitions -- runtime
// ---------------------------------------------------------------------------

/// `AT#SLEEPMODE` -- the bare *execution* form. Enters sleep immediately.
///
/// "AT#SLEEPMODE without parameter cancels the hold time and the module enters
/// in sleep mode." (AT manual §4.30). Distinct from the set form below, which is
/// provision-only.
///
/// Serialisation: a zero-field `AtatCmd` never calls `serialize_field`, and the
/// `=` separator is only written from inside `serialize_field`. So this emits
/// `AT#SLEEPMODE\r\n` with no trailing `=`. Same mechanism as `GnssDeinit`.
///
/// The modem answers `OK` *before* sleeping, then emits `#ENERGY: <mJ>` and
/// `#SLEEP PSM <secs>s` as URCs (assuming `AT#SLEEPIND`). Treat `OK` as "command
/// accepted", not "modem asleep" -- `#SLEEP` is the confirmation.
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("#SLEEPMODE", OkResponse, timeout_ms = 2000)]
pub struct EnterSleep;

/// `AT` -- bare ping, used to confirm the modem has woken and its UART is live.
///
/// Emits `AT\r\n`. Expect `OK`. Failures during wake are normal and expected:
/// retry per [`WAKE_PING_ATTEMPTS`].
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("", AtResponse, timeout_ms = 1000)]
pub struct AtPing;

#[derive(Clone, AtatResp)]
pub struct AtResponse;

// ---------------------------------------------------------------------------
// Provision-time commands
// ---------------------------------------------------------------------------
//
// These are here for reference and for any future runtime reconfiguration. The
// provisioning binary currently issues them over raw blocking UART, alongside
// CPSMS (which cannot be expressed as an AtatCmd -- see module docs).

/// `AT#SLEEPMODE=<enable>,<hold_time>,<awake_time>` -- the *set* form.
///
/// Parameters are NVM-saved and take effect only after `AT#RESET=1`. Provision
/// once; do not call at runtime.
#[derive(Clone, AtatCmd)]
#[at_cmd("#SLEEPMODE", OkResponse, timeout_ms = 1000)]
pub struct SetSleepMode {
    #[at_arg(position = 0)]
    pub enable: u8,
    #[at_arg(position = 1)]
    pub hold_time_secs: u32,
    #[at_arg(position = 2)]
    pub awake_time_secs: u32,
}

impl Default for SetSleepMode {
    fn default() -> Self {
        Self {
            enable: 1,
            hold_time_secs: SLEEP_HOLD_TIME_SECS,
            awake_time_secs: SLEEP_AWAKE_TIME_SECS,
        }
    }
}

/// `AT#SLEEPMODE?` -- read back `<enable>,<hold_time>,<awake_time>`.
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("#SLEEPMODE?", SleepModeConfig, timeout_ms = 1000)]
pub struct GetSleepMode;

#[derive(Clone, AtatResp)]
pub struct SleepModeConfig {
    pub enable: u8,
    pub hold_time_secs: u32,
    pub awake_time_secs: u32,
}

/// `AT#WAKEUPEVENT=<pwrkey_evt>,<uart_evt>` -- arm the host->modem wake paths.
///
/// NVM-saved; takes effect after reboot. Provision once.
#[derive(Clone, AtatCmd)]
#[at_cmd("#WAKEUPEVENT", OkResponse, timeout_ms = 1000)]
pub struct SetWakeupEvent {
    #[at_arg(position = 0)]
    pub pwrkey_evt: u8,
    #[at_arg(position = 1)]
    pub uart_evt: u8,
}

impl Default for SetWakeupEvent {
    fn default() -> Self {
        Self {
            pwrkey_evt: WAKEUPEVENT_PWRKEY,
            uart_evt: WAKEUPEVENT_UART,
        }
    }
}

/// `AT#SLEEPIND=<msg_ind>` -- enable the sleep/wake/energy URCs.
///
/// NVM-saved; takes effect after reboot. Provision once.
///
/// Do not raise this to `0x7F` without also replacing the digester's URC parser:
/// see [`SLEEPIND_PSM_AND_ENERGY`].
#[derive(Clone, AtatCmd)]
#[at_cmd("#SLEEPIND", OkResponse, timeout_ms = 1000)]
pub struct SetSleepInd {
    pub msg_ind: u8,
}

impl Default for SetSleepInd {
    fn default() -> Self {
        Self {
            msg_ind: SLEEPIND_PSM_AND_ENERGY,
        }
    }
}
