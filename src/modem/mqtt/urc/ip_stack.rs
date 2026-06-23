use heapless::String;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, defmt::Format)]
pub struct CgevUrc {
    pub body: String<32>, // "ME PDN ACT 5", "NW DETACH", etc.
}

#[derive(Debug, Clone, Deserialize, defmt::Format)]
pub enum CgevEvent {
    MePdnAct(u8),   // cid
    MePdnDeact(u8), // cid
    NwPdnDeact(u8), // cid
    MeDetach,
    NwDetach,
    Unknown,
}

impl CgevUrc {
    pub fn event(&self) -> CgevEvent {
        let s = self.body.as_str().trim();
        if let Some(rest) = s.strip_prefix("ME PDN ACT ") {
            return rest
                .parse()
                .map(CgevEvent::MePdnAct)
                .unwrap_or(CgevEvent::Unknown);
        }
        if let Some(rest) = s.strip_prefix("ME PDN DEACT ") {
            return rest
                .parse()
                .map(CgevEvent::MePdnDeact)
                .unwrap_or(CgevEvent::Unknown);
        }
        if let Some(rest) = s.strip_prefix("NW PDN DEACT ") {
            return rest
                .parse()
                .map(CgevEvent::NwPdnDeact)
                .unwrap_or(CgevEvent::Unknown);
        }
        match s {
            "ME DETACH" => CgevEvent::MeDetach,
            "NW DETACH" => CgevEvent::NwDetach,
            _ => CgevEvent::Unknown,
        }
    }
}
