use crate::{
    modem::gnss::urc::{fix::GnssFixUrcRaw, init::GnssInitUrcRaw},
    modem::mqtt::urc::{ip_stack::CgevUrc, receive::MqttRecvUrc, socket::SocketClosedUrc},
};
use atat::atat_derive::AtatUrc;

use {defmt_rtt as _, panic_probe as _};

/// Trim ASCII whitespace from both ends, mirroring `urc_helper`'s own trimming
/// so the dispatch key matches the plain `#[at_urc]` code.
fn trim_ws(x: &[u8]) -> &[u8] {
    let start = x.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    let end = x
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    &x[start..end]
}

/// `#ENERGY` parser that does NOT swallow its trailing CRLF when another URC
/// follows immediately.
///
/// # The problem
///
/// On PSM entry the modem emits both URCs as a single burst with only ONE CRLF
/// between them:
///
/// ```text
/// \r\n#ENERGY: 1809.7\r\n#SLEEP \r\n
/// ```
///
/// This is unusual for this modem. Ordinary back-to-back URCs are double-CRLF
/// separated (`\r\n+CGEV: ME PDN ACT 5\r\n\r\n#IPCFG: ...`), so each keeps its own
/// leading CRLF and stock parsing works.
///
/// Stock `urc_helper` matches `\r\n{token}(:.*)?\r\n` and consumes the trailing
/// CRLF -- which here is the very CRLF `#SLEEP` needs as its *leading* one. The
/// digester then reaches `#SLEEP` with the buffer starting bare at `#`.
///
/// That is fatal, and not fixable from the `#SLEEP` side, because
/// `AtDigester::digest` strips echo BEFORE it tries any URC parser:
///
/// ```text
/// let buf = parser::trim_start_ascii_space(input);
/// let (buf, space_and_echo_bytes) = opt(parser::echo)(buf);   // take_until("\r\n")
/// ```
///
/// With no leading CRLF, `echo` swallows `#SLEEP ` as an AT echo before
/// `P::parse` is ever called. The hardware log shows precisely this:
///
/// ```text
/// Received URC/128 (19/25): b"#ENERGY: 1809.7"       // 19 = \r\n + 15 + \r\n
/// Received echo or whitespace (7/9): b"#SLEEP \r\n"   // eaten as echo
/// ```
///
/// # The fix
///
/// If another URC follows (`\r\n#`), consume only `\r\n#ENERGY: <body>` and leave
/// the CRLF behind to serve as the next URC's leading one. Otherwise fall back to
/// stock behaviour, so a standalone `#ENERGY` leaves no stray bytes in the buffer.
fn urc_keep_crlf<'a, T, Error: atat::nom::error::ParseError<&'a [u8]>>(
    token: T,
) -> impl Fn(&'a [u8]) -> atat::nom::IResult<&'a [u8], (&'a [u8], usize), Error>
where
    &'a [u8]: atat::nom::Compare<T> + atat::nom::FindSubstring<T>,
    T: atat::nom::InputLength + Clone + atat::nom::InputTake + atat::nom::InputIter,
{
    use atat::nom::{
        bytes::complete::{tag, take_till},
        character::complete::line_ending,
        combinator::recognize,
        sequence::tuple,
    };

    move |i| {
        // Branch A: another URC follows on the shared CRLF -- leave it for them.
        let attempt = tuple((
            line_ending::<_, Error>,
            recognize(tuple((
                tag(token.clone()),
                tag(":"),
                take_till(|c| c == b'\r'),
            ))),
        ))(i);

        if let Ok((rest, (le, matched))) = attempt {
            // Plain slice check rather than nom's `peek(tag(..))`: the turbofish's
            // tag type would unify with the outer generic `T` and fail to compile.
            if rest.starts_with(b"\r\n#") {
                return Ok((rest, (trim_ws(matched), le.len() + matched.len())));
            }
        }

        // Branch B: standalone. Stock behaviour, trailing CRLF consumed.
        atat::digest::parser::urc_helper(token.clone())(i)
    }
}

/// `#SLEEP` parser: tolerates the trailing space the modem appends.
///
/// The modem emits `#SLEEP \r\n` -- with a trailing space -- even with
/// `AT#SLEEPIND` bit4 (verbosity) OFF. Stock `urc_helper` requires `:` or CRLF
/// immediately after the token, and a space is neither.
///
/// Declaring the token as `"#SLEEP "` does NOT work, and the reason is worth
/// recording. The `AtatUrc` derive uses the same `#[at_urc]` code for two jobs:
///
///   1. the digester token, matched against raw bytes (space present); and
///   2. the dispatch key in `AtatUrc::parse`, compared against the *trimmed* tag
///      (space gone).
///
/// So `"#SLEEP "` would pass the digester and then fail the match arm, and the URC
/// would be dropped silently -- worse than not matching at all. Hence
/// `parse = ...`: override the digester's parser, keep the plain code for dispatch.
///
/// Relies on [`urc_keep_crlf`] having preserved the leading CRLF.
fn urc_trailing_space<'a, T, Error: atat::nom::error::ParseError<&'a [u8]>>(
    token: T,
) -> impl Fn(&'a [u8]) -> atat::nom::IResult<&'a [u8], (&'a [u8], usize), Error>
where
    &'a [u8]: atat::nom::Compare<T> + atat::nom::FindSubstring<T>,
    T: atat::nom::InputLength + Clone + atat::nom::InputTake + atat::nom::InputIter,
{
    use atat::nom::{
        bytes::complete::tag,
        character::complete::{line_ending, space0},
        combinator::{opt, recognize},
        sequence::tuple,
    };

    move |i| {
        // Leading CRLF is OPTIONAL, and that is the whole point of this parser.
        //
        // `#SLEEP` is emitted right after `#ENERGY`, and whether it still has its
        // leading CRLF depends on how the UART chunked the burst:
        //
        //   both in one read   -> `urc_keep_crlf` sees the following `\r\n#` and
        //                         leaves the shared CRLF, so `#SLEEP` has one
        //   split across reads -> `#ENERGY` is digested before `#SLEEP` arrives,
        //                         cannot see what follows, and consumes the
        //                         trailing CRLF -- so `#SLEEP` arrives bare
        //
        // Requiring the CRLF made this a race: the URC parsed on some cycles and
        // was swallowed as echo on others, which stalled `enter_psm` until its
        // 45s timeout and skipped the sleep entirely.
        let (i, (le, matched)) = tuple((
            opt(line_ending),
            recognize(tuple((tag(token.clone()), space0, tag("\r\n")))),
        ))(i)?;

        let le_len = le.map(|l| l.len()).unwrap_or(0);
        Ok((i, (trim_ws(matched), le_len + matched.len())))
    }
}

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
    // Power Saving Mode URCs (gated on `AT#SLEEPIND=0x44`, see `modem::psm`)
    // ---------------------------------------------------------------------
    /// `#SLEEP ` -- the module has entered sleep mode.
    ///
    /// Arrives some time *after* the `OK` for the bare `AT#SLEEPMODE` -- around
    /// 12 s in practice, because the modem cannot enter PSM until the network
    /// releases the RRC connection and T3324 (active time) expires. The `OK`
    /// means only "command accepted"; this URC is the real confirmation, and the
    /// only safe basis for putting the RP2350 to sleep.
    ///
    /// Note the custom parser -- the modem appends a trailing space. See
    /// [`urc_trailing_space`].
    #[at_urc("#SLEEP", parse = urc_trailing_space)]
    Sleep,

    /// `#WAKEUP` -- the module has woken up.
    ///
    /// Uses the same tolerant parser as `#SLEEP`. The manual does not show this
    /// URC's exact wire format and it has not been observed on hardware yet, so
    /// tolerance is the safe default: it accepts `#WAKEUP\r\n` and `#WAKEUP \r\n`
    /// alike.
    #[at_urc("#WAKEUP", parse = urc_trailing_space)]
    Wakeup,

    /// `#ENERGY: <uWh>` -- consumption since the previous report.
    ///
    /// Emitted alongside `#SLEEP` when `AT#SLEEPIND` bit6 is set. Confirmed on
    /// hardware (`#ENERGY: 2894.7`). Colon-delimited, so the stock parser is fine.
    ///
    /// The cheapest per-cycle power telemetry available before a meter is on the
    /// board -- worth logging every cycle.
    #[at_urc("#ENERGY", parse = urc_keep_crlf)]
    Energy(heapless::String<32>),

    #[at_urc("#SYSSTART")]
    SysStart,
    #[at_urc("#REBOOT_RESET")]
    RebootReset,
    #[at_urc("#REBOOT_HOST")]
    RebootHost,
    #[at_urc("#REBOOT_WD")]
    RebootWD(heapless::String<4>),
}