use atat::atat_derive::{AtatCmd, AtatResp};

/// `AT+CESQ` -- extended signal quality.
///
/// # Why this exists
///
/// On NB-IoT the meaningful signal figure is RSRP, not the plain `AT+CSQ` RSSI
/// (which saturates at the low signal levels NB-IoT is designed to work at). This
/// reads it so a marginal link -- registered but too weak for a reliable TLS MQTT
/// exchange -- is visible in the logs rather than showing up only as publish
/// timeouts.
///
/// # Response
///
/// ```text
/// +CESQ: <rxlev>,<ber>,<rscp>,<ecno>,<rsrq>,<rsrp>
/// ```
///
/// Only the last two are populated on LTE-M/NB-IoT (the GERAN/UTRA fields report
/// their "not applicable" sentinels). We take `<rsrp>` and, for context, `<rsrq>`.
///
/// `<rsrp>` is an INDEX, not dBm: 0 = `rsrp < -140 dBm`, 97 = `rsrp >= -44 dBm`,
/// 255 = unknown. So `dBm = index - 141` for 1..=97. (AT manual 6.5.)
///
/// Zero args, so atat serialises the bare execution form `AT+CESQ`. Parsed from
/// raw bytes for the same reason as `+CEREG?`: a URC can land in the same UART
/// read, so the `+CESQ:` line must be isolated first.
#[derive(Clone, AtatCmd, Default)]
#[at_cmd("+CESQ", CesqStatus, parse = parse_cesq, timeout_ms = 1000)]
pub struct CesqQuery;

#[derive(Clone, AtatResp, defmt::Format)]
pub struct CesqStatus {
    /// Raw `<rsrp>` index (0..=97, or 255 = unknown). `None` if unparseable.
    pub rsrp_index: Option<u8>,
    /// `<rsrp>` decoded to dBm, or `None` if unknown/absent. More negative is
    /// weaker; useful reference points: >= -80 excellent, -90 good, -100 fair,
    /// -110 poor, < -115 marginal (attach may hold but data is unreliable).
    pub rsrp_dbm: Option<i16>,
    /// Raw `<rsrq>` index (0..=34, or 255 = unknown). Quality, not power.
    pub rsrq_index: Option<u8>,
}

/// `<rsrp>` index -> dBm. Index 0 means "< -140", which we report as -140.
fn rsrp_index_to_dbm(idx: u8) -> Option<i16> {
    match idx {
        0..=97 => Some(idx as i16 - 141),
        _ => None, // 255 = not known / not detectable
    }
}

/// Extract `<rsrq>` and `<rsrp>` (the last two fields) from a `+CESQ` response.
///
/// As with `+CEREG?`, isolate the `+CESQ:` line first (a URC can share the read)
/// and take the fields by position from the known 6-field layout.
pub fn parse_cesq(resp: &[u8]) -> Result<CesqStatus, ()> {
    let text = core::str::from_utf8(resp).map_err(|_| ())?;

    let line = match text.rfind("+CESQ:") {
        Some(i) => &text[i + "+CESQ:".len()..],
        None => return Err(()),
    };
    let line = line.split(['\r', '\n']).next().unwrap_or(line);

    // +CESQ: <rxlev>,<ber>,<rscp>,<ecno>,<rsrq>,<rsrp>
    let fields: heapless::Vec<&str, 6> = line.split(',').map(|f| f.trim()).collect();
    if fields.len() < 6 {
        return Err(());
    }

    let rsrq_index = fields[4].parse::<u8>().ok();
    let rsrp_index = fields[5].parse::<u8>().ok();
    let rsrp_dbm = rsrp_index.and_then(rsrp_index_to_dbm);

    Ok(CesqStatus {
        rsrp_index,
        rsrp_dbm,
        rsrq_index,
    })
}
