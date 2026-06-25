use atat::atat_derive::{AtatCmd, AtatResp};

/// `AT+CCLK?` — read the modem real-time clock.
///
/// The RTC only synchronises after the UE receives EMM INFORMATION signalling
/// (post-registration, AT manual §3.3). A TLS handshake started before the clock
/// is set times out silently (surfacing later as MQTT `+CME ERROR: 2214`,
/// connection failed). After a modem reboot we can reach MQTTCONNECT before the
/// clock lands, so we poll this query and gate the connect on a sane date.
///
/// Read response: `+CCLK: "YY/MM/DD,hh:mm:ss±zz"`.
///
/// Zero fields, so atat serialises the bare read form `AT+CCLK?`. The datetime
/// contains a comma inside quotes, which doesn't fit atat's value-splitting
/// deserialiser, so we take the raw bytes via `parse =` and read the year
/// directly.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CCLK?", ClockReady, parse = parse_cclk, timeout_ms = 1000)]
pub struct CclkQuery {}

#[derive(Clone, AtatResp)]
pub struct ClockReady {
    /// True once the clock reads a plausible current year (see `parse_cclk`).
    pub ready: bool,
    /// Two-digit year parsed from the response, if any (for logging).
    pub year: Option<u8>,
}

/// Extract the two-digit year from a `+CCLK?` response and decide if the clock
/// has synced.
///
/// The response can arrive concatenated with unrelated URCs that landed in the
/// same UART read (e.g. `+CEREG: 5,... +CTZEU: ... +CCLK: "26/06/25,..."`), so
/// we first isolate the `+CCLK:` line — otherwise the leading digit of `+CEREG:
/// 5` would be misread as the year. Within that line the datetime is the first
/// quoted field `"YY/MM/DD,..."`; the year is the two digits after the quote.
///
/// Readiness uses a plausible-current-year window (`25..=35`) rather than a
/// simple lower bound, because the modem's pre-sync default may be an epoch year
/// like 1980/2004/1970 — as two digits `80`/`04`/`70` — and `80 >= 25` would be
/// a false positive. The window rejects every common default and only passes
/// real 2025+ dates. Adjust the upper bound if this is still flying in 2035.
pub fn parse_cclk(resp: &[u8]) -> Result<ClockReady, ()> {
    let text = core::str::from_utf8(resp).map_err(|_| ())?;

    // Isolate the actual +CCLK line (last occurrence; any +CEREG/+CTZEU URCs
    // precede it).
    let after = match text.rfind("+CCLK:") {
        Some(i) => &text[i + "+CCLK:".len()..],
        None => text,
    };
    // Skip to the datetime: the first quoted field if present, else trim.
    let datetime = match after.split_once('"') {
        Some((_, rest)) => rest,
        None => after.trim(),
    };

    let mut year: Option<u8> = None;
    let mut val: u16 = 0;
    let mut n = 0u8;
    for &b in datetime.as_bytes() {
        if b.is_ascii_digit() {
            val = val * 10 + (b - b'0') as u16;
            n += 1;
            if n == 2 {
                year = Some(val as u8);
                break;
            }
        } else if n > 0 {
            year = Some(val as u8);
            break;
        } else {
            // Non-digit before any year digit — unexpected; give up cleanly.
            break;
        }
    }

    let ready = matches!(year, Some(y) if (25..=35).contains(&y));
    Ok(ClockReady { ready, year })
}
