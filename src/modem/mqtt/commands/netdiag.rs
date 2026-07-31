//! One-shot network diagnostics: is the radio on, are we registered, who are we
//! (or could we be) on. Used to answer "can't find the network" from the log
//! rather than by inference from repeated bring-up failures.
//!
//! # Why raw dumps
//!
//! These are for a human reading defmt output once, not for the state machine.
//! So instead of parsing each field into a typed struct, every command captures
//! the modem's response text verbatim into a [`RawAtResp`] and logs it. Less
//! code, and it can't silently misparse an unexpected format.
//!
//! # URC caveat (why `+CEREG` is *not* here)
//!
//! `+CFUN` and `+COPS` are not URC tokens, so their responses come straight back
//! to `client.send()` and parse cleanly here. `+CEREG` *is* a URC token, so the
//! reply to `AT+CEREG?` is routed to the URC channel and never reaches `send()`.
//! Registration state is therefore read in `command_task::run_net_diag` by
//! draining the URC subscription, not by a command in this module.

use atat::atat_derive::{AtatCmd, AtatResp};

/// Verbatim capture of an AT response. 512 bytes is ample for the single-line
/// CFUN?/CEREG/COPS? replies these diagnostics use; anything longer is truncated
/// in [`raw_dump`] rather than erroring.
#[derive(Clone, AtatResp, defmt::Format)]
pub struct RawAtResp {
    pub text: heapless::String<512>,
}

/// Capture the response bytes verbatim (as much as fits), never fail. AT
/// responses are ASCII; a non-UTF-8 read is reported as a marker string rather
/// than dropped, so a garbled line is still visible in the log.
pub fn raw_dump(resp: &[u8]) -> Result<RawAtResp, ()> {
    let s = core::str::from_utf8(resp).unwrap_or("<non-utf8 response>");
    let mut text = heapless::String::<512>::new();
    for c in s.chars() {
        if text.push(c).is_err() {
            break; // scan longer than the buffer: keep the prefix
        }
    }
    Ok(RawAtResp { text })
}

/// `AT+CFUN?` -- radio functionality. `+CFUN: 1` = full, `0` = minimum (RF off),
/// `4` = airplane. If this is not `1`, nothing else in the diagnostic matters.
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("+CFUN?", RawAtResp, parse = raw_dump, timeout_ms = 5000)]
pub struct CfunQuery;

/// `AT+COPS?` -- currently registered operator. Only meaningful once registered;
/// default long-alphanumeric format names the operator in words (e.g.
/// "vodafone UK"). `<Act>=9` = E-UTRAN NB-S1 (NB-IoT).
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("+COPS?", RawAtResp, parse = raw_dump, timeout_ms = 5000)]
pub struct CopsRead;
