use atat::atat_derive::{AtatCmd, AtatResp};
use heapless::String;

/// `AT+CGPADDR=<cid>` — show the PDP address(es) for a context.
///
/// The ST87MXX auto-defines and activates context 5 at startup and holds it
/// across an RP2350 reflash, so the one-shot `+CGEV: ME PDN ACT 5` edge often
/// fires before `network_task` has subscribed. Querying the address lets us
/// detect an already-active context instead of waiting for an edge that won't
/// be replayed.
///
/// Response (active):   `+CGPADDR: 5,"100.115.11.113"`
/// Response (inactive): `+CGPADDR: 5`        (address omitted — see AT manual)
///
/// Uses a custom `parse =` rather than the derive deserialiser because the
/// response can arrive concatenated with unrelated URCs from the same UART read
/// (e.g. `+CEREG: 5,"242E",... +CGPADDR: 5,"..."`), which made the derived
/// parser choke with a ParseError on the leading `+CEREG` line.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGPADDR", CgPaddrResponse, parse = parse_cgpaddr, timeout_ms = 1000)]
pub struct CgPaddrQuery {
    pub cid: u8,
}

impl Default for CgPaddrQuery {
    fn default() -> Self {
        Self { cid: 5 } // always 5 for this modem (default context)
    }
}

/// The manual states both address fields are omitted when none is available,
/// so `address` is `None` for an inactive context and `Some(addr)` when up.
#[derive(Clone, AtatResp)]
pub struct CgPaddrResponse {
    pub cid: u8,
    pub address: Option<String<64>>,
}

impl CgPaddrResponse {
    /// True when the context has an assigned address, i.e. it is up.
    pub fn is_active(&self) -> bool {
        self.address.as_ref().map_or(false, |a| !a.is_empty())
    }
}

/// Parse `AT+CGPADDR=<cid>` by isolating the `+CGPADDR:` line, so concatenated
/// URCs (`+CEREG`, etc.) preceding it in the same read can't corrupt the parse.
/// Extracts `<cid>` and the first address (if any); a missing or empty address
/// means the context is inactive.
pub fn parse_cgpaddr(resp: &[u8]) -> Result<CgPaddrResponse, ()> {
    let text = core::str::from_utf8(resp).map_err(|_| ())?;

    // Last occurrence is the actual response; any +CEREG/URC lines precede it.
    let after = text
        .rfind("+CGPADDR:")
        .map(|i| &text[i + "+CGPADDR:".len()..])
        .ok_or(())?;
    // Trim to the end of that line.
    let line = after
        .split(|c| c == '\r' || c == '\n')
        .next()
        .unwrap_or(after)
        .trim();

    let mut fields = line.split(',');
    let cid = fields
        .next()
        .and_then(|c| c.trim().parse::<u8>().ok())
        .ok_or(())?;
    let address = match fields.next() {
        Some(a) => {
            let a = a.trim().trim_matches('"');
            if a.is_empty() {
                None
            } else {
                Some(String::try_from(a).map_err(|_| ())?)
            }
        }
        None => None,
    };

    Ok(CgPaddrResponse { cid, address })
}