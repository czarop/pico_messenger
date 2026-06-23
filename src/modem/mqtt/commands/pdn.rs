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
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGPADDR", CgPaddrResponse, timeout_ms = 1000)]
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
