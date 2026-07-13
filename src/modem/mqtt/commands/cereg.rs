use atat::atat_derive::{AtatCmd, AtatResp};

/// `AT+CEREG?` -- read registration status and, in `n=4`/`n=5` mode, the PSM
/// timers the network actually **granted**.
///
/// # Why this exists
///
/// Requested is not granted. `AT+CPSMS` only *asks*; the network decides at
/// attach/TAU and can hand back something entirely different. The AT manual (6.9)
/// is explicit: "To get the Active Time value and the extended periodic TAU value
/// that are allocated to the UE by the network, use the command AT+CEREG."
///
/// The `+CEREG` URC carries the same fields, but only fires on a registration
/// **state change** -- so a modem that is already registered stays silent, and one
/// that registers during the boot delay fires before atat's ingress is listening.
/// Either way you learn nothing. This query is the reliable path.
///
/// The number matters: the granted T3412 is the modem's self-wake period, which
/// (via `AT#RINGPIN`) is the only thing that can wake the RP2350 out of DORMANT.
/// If the carrier floors it at hours, a short tracking cadence is not achievable
/// this way at all.
///
/// # Response
///
/// ```text
/// +CEREG: 4,5,"242E","07BED015",9,,,"00000001","00011000"
///         │ │  │      │         │      │          └ Periodic-TAU  (T3412, granted)
///         │ │  │      │         │      └─────────── Active-Time   (T3324, granted)
///         │ │  │      │         └ AcT
///         │ │  │      └ ci
///         │ │  └ tac
///         │ └ stat  (1 = home, 5 = roaming)
///         └ n
/// ```
///
/// Note the two empty fields before Active-Time (`cause_type`, `reject_cause`),
/// and that the timers are the **last two** quoted fields. Parsing from the end is
/// therefore more robust than counting commas forwards.
///
/// Zero fields, so atat serialises the bare read form `AT+CEREG?`. Taken as raw
/// bytes via `parse =` for the same reason as `+CCLK?`: quoted fields, empty
/// middle args, and the ever-present risk of a URC landing in the same UART read.
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("+CEREG?", CeregStatus, parse = parse_cereg, timeout_ms = 1000)]
pub struct CeregQuery;

#[derive(Clone, AtatResp, defmt::Format)]
pub struct CeregStatus {
    /// `<stat>`: 1 = registered (home), 5 = registered (roaming). Others = not
    /// registered. `None` if unparseable.
    pub stat: Option<u8>,
    /// Granted T3324 (Active Time) as the raw 8-bit string, e.g. `"00000001"`.
    pub active_time_bits: Option<heapless::String<8>>,
    /// Granted T3412 (extended periodic TAU) as the raw 8-bit string, e.g.
    /// `"00011000"`.
    pub periodic_tau_bits: Option<heapless::String<8>>,
    /// Granted T3324, decoded to seconds. `None` if absent or deactivated.
    pub active_time_secs: Option<u32>,
    /// Granted T3412, decoded to seconds. `None` if absent or deactivated.
    pub periodic_tau_secs: Option<u32>,
}

/// Decode a GPRS Timer 3 octet (3GPP TS 24.008 10.5.7.4a) -- used for **T3412**.
///
/// bits 7..5 = unit, bits 4..0 = multiplier.
///
/// Validated against the AT manual's worked example: `"01000111"` =
/// 010 (10 h) x 00111 (7) = 70 hours.
///
/// `None` = deactivated.
fn decode_t3412(bits: u8) -> Option<u32> {
    let unit = bits >> 5;
    let val = (bits & 0x1f) as u32;
    let step = match unit {
        0b000 => 600,       // 10 minutes
        0b001 => 3600,      // 1 hour
        0b010 => 36000,     // 10 hours
        0b011 => 2,         // 2 seconds
        0b100 => 30,        // 30 seconds
        0b101 => 60,        // 1 minute
        0b110 => 1_152_000, // 320 hours
        _ => return None,   // 111 = deactivated
    };
    Some(val * step)
}

/// Decode a GPRS Timer 2 octet (3GPP TS 24.008 10.5.7.3) -- used for **T3324**.
///
/// A DIFFERENT unit table from [`decode_t3412`]. The same octet means different
/// things in the two positions: `"00000001"` is 10 MINUTES as T3412 and 2 SECONDS
/// as T3324. This is why they get separate functions rather than a shared decoder
/// with a table argument -- the two must never be confusable at a call site.
///
/// Validated against the manual's example: `"00100100"` = 001 (1 min) x 00100 (4)
/// = 4 minutes.
fn decode_t3324(bits: u8) -> Option<u32> {
    let unit = bits >> 5;
    let val = (bits & 0x1f) as u32;
    let step = match unit {
        0b000 => 2,       // 2 seconds
        0b001 => 60,      // 1 minute
        0b010 => 360,     // 6 minutes (decihour)
        _ => return None, // 111 = deactivated; 011..110 unsupported (AT manual 6.9)
    };
    Some(val * step)
}

/// Parse an 8-character binary string into an octet. Rejects anything else.
fn parse_bits(s: &str) -> Option<u8> {
    if s.len() != 8 {
        return None;
    }
    let mut v: u8 = 0;
    for b in s.bytes() {
        v <<= 1;
        match b {
            b'0' => {}
            b'1' => v |= 1,
            _ => return None,
        }
    }
    Some(v)
}

/// Extract `<stat>` and the granted PSM timers from a `+CEREG?` response.
///
/// Robustness notes, both learned the hard way on this modem:
///
/// * **Isolate the `+CEREG:` line first.** Unrelated URCs can arrive in the same
///   UART read and land in the response buffer. Same defence as `parse_cclk`.
/// * **Parse the timers from the END.** They are the last two quoted fields. The
///   middle of the response contains empty positional args (`,,,`) whose count has
///   varied between `n` modes, so counting commas forwards is brittle; taking the
///   final two quoted fields is not.
///
/// Absent timers (the network granted no PSM, or `n<4`) yield `None` rather than
/// an error -- that is a legitimate answer, not a parse failure.
pub fn parse_cereg(resp: &[u8]) -> Result<CeregStatus, ()> {
    let text = core::str::from_utf8(resp).map_err(|_| ())?;

    // Isolate the +CEREG line; any URCs precede it.
    let line = match text.rfind("+CEREG:") {
        Some(i) => &text[i + "+CEREG:".len()..],
        None => return Err(()),
    };
    // Stop at the end of the line, so a trailing "OK" cannot be mistaken for data.
    let line = line.split(['\r', '\n']).next().unwrap_or(line);

    // <stat> is the 2nd numeric field in read form: "+CEREG: <n>,<stat>,..."
    let mut fields = line.split(',');
    let _n = fields.next();
    let stat = fields.next().and_then(|f| f.trim().parse::<u8>().ok());

    // Collect every quoted field, then take the last two: Active-Time, Periodic-TAU.
    let mut quoted: heapless::Vec<&str, 8> = heapless::Vec::new();
    let mut rest = line;
    while let Some((_, after_open)) = rest.split_once('"') {
        match after_open.split_once('"') {
            Some((inner, after_close)) => {
                let _ = quoted.push(inner);
                rest = after_close;
            }
            None => break,
        }
    }

    // Only the 8-bit binary strings are timers; tac/ci are hex of other lengths.
    // Taking the last two guards against a short/odd response shape.
    let n = quoted.len();
    let (at_str, tau_str) = if n >= 2 {
        (Some(quoted[n - 2]), Some(quoted[n - 1]))
    } else {
        (None, None)
    };

    let to_string = |s: &str| -> Option<heapless::String<8>> {
        let mut out: heapless::String<8> = heapless::String::new();
        out.push_str(s).ok()?;
        Some(out)
    };

    let at_bits = at_str.and_then(parse_bits);
    let tau_bits = tau_str.and_then(parse_bits);

    Ok(CeregStatus {
        stat,
        active_time_bits: at_str
            .filter(|s| parse_bits(s).is_some())
            .and_then(to_string),
        periodic_tau_bits: tau_str
            .filter(|s| parse_bits(s).is_some())
            .and_then(to_string),
        active_time_secs: at_bits.and_then(decode_t3324),
        periodic_tau_secs: tau_bits.and_then(decode_t3412),
    })
}
