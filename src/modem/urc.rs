use crate::{
    modem::gnss::urc::{fix::GnssFixUrcRaw, init::GnssInitUrcRaw},
    modem::mqtt::urc::{
        ip_stack::CgevUrc,
        receive::MqttRecvUrc,
        socket::SocketClosedUrc,
    },
};
use atat::atat_derive::AtatUrc;

use {defmt_rtt as _, panic_probe as _};

#[derive(Clone, AtatUrc, defmt::Format)]
pub enum ModemUrc {
    #[at_urc("#GNSSINIT")]
    GnssStatus(GnssInitUrcRaw),
    #[at_urc("#GNSSFIX")]
    Location(GnssFixUrcRaw),
    #[at_urc("+CGEV")]
    IPStackUpdate(CgevUrc),
    #[at_urc("#SOCKETCLOSED")]
    SocketClosed(SocketClosedUrc),
    #[at_urc("#MQTTRECV")]
    MqttReceived(MqttRecvUrc),

    // Network-emitted URCs. These arrive unsolicited (registration status and
    // network-time updates) and can land *during* a long command like
    // MQTTCONNECT. They carry no action for us, but they MUST be registered:
    // an unregistered URC is not recognised by the digester and gets folded
    // into the pending command's response buffer, corrupting the parse (this
    // caused false MQTTCONNECT ParseErrors and the CGPADDR/CCLK misreads).
    // Capture the body in a generously-sized String so a longer PSM-form
    // +CEREG line can't overflow and fall back to polluting the response.
    #[at_urc("+CEREG")]
    Cereg(heapless::String<96>),
    #[at_urc("+CTZEU")]
    CtzEu(heapless::String<64>),
    // Fires whenever a PDP context is established (every connect/reboot cycle).
    // Seen in logs bleeding into the SOCKETCREATE? response; would break a
    // command with a strict OkResponse (e.g. MQTTCONNECT) the same way +CEREG
    // did if it lands mid-command. Body: "<context_id>,<ip_mode>,<ip_status>".
    #[at_urc("#IPCFG")]
    IpCfg(heapless::String<32>),

    // ---------------------------------------------------------------------
    // Power Saving Mode URCs (gated on `AT#SLEEPIND`, see `modem::psm`)
    // ---------------------------------------------------------------------
    //
    // Emitted around every PSM entry/exit once `AT#SLEEPIND=0x44` has been
    // provisioned. They must be registered here for the same reason as +CEREG
    // above -- an unregistered URC pollutes the pending command's response
    // buffer. `#SLEEP` in particular arrives immediately after the `OK` for the
    // bare `AT#SLEEPMODE`, i.e. exactly when a command is still in flight.
    //
    // CRITICAL -- these tokens only match because `AT#SLEEPIND` bit4 (verbosity)
    // is OFF. atat's `urc_helper` recognises a URC only as
    // `\r\n{token}(:.*)?\r\n`: the token must be followed immediately by `:` or
    // CRLF. With verbosity enabled the modem emits `#SLEEP PSM 3599.9s` -- token
    // then a *space* -- which matches neither form and would be silently folded
    // into a response buffer. See `psm::SLEEPIND_PSM_AND_ENERGY`.
    //
    // The upside of that same strictness: `#SLEEP` is a strict prefix of
    // `#SLEEPMODE:` / `#SLEEPIND:`, and `#WAKEUP` of `#WAKEUPEVENT:`. Since the
    // token must be followed by `:` or CRLF, and those responses have `M`/`E`
    // next, these arms cannot steal the corresponding read-command responses.

    /// `#SLEEP` -- the module is entering sleep mode (AT manual Table 6).
    ///
    /// Arrives shortly after the `OK` for the bare `AT#SLEEPMODE`. This, not the
    /// `OK`, is the confirmation that PSM was actually entered.
    #[at_urc("#SLEEP")]
    Sleep,

    /// `#WAKEUP` -- the module has just woken up (AT manual Table 6).
    ///
    /// Expected after pulsing GPIO10 low (the `WAKE_UP` pin), or after the modem
    /// wakes itself for a periodic TAU.
    #[at_urc("#WAKEUP")]
    Wakeup,

    /// `#ENERGY: <uWh>` -- consumption since the previous `#ENERGY` report.
    ///
    /// Emitted alongside `#SLEEP` when `AT#SLEEPIND` bit6 is set. Purely
    /// informational, but it is the cheapest per-cycle power telemetry available
    /// without a meter, so it is worth logging.
    ///
    /// Body is a decimal float as text, e.g. `414.7`.
    #[at_urc("#ENERGY")]
    Energy(heapless::String<16>),

    #[at_urc("#SYSSTART")]
    SysStart,
    #[at_urc("#REBOOT_RESET")]
    RebootReset,
    #[at_urc("#REBOOT_HOST")]
    RebootHost,
    #[at_urc("#REBOOT_WD")]
    RebootWD(heapless::String<4>),
}