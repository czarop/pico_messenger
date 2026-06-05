use atat::atat_derive::{AtatCmd, AtatResp};

#[derive(Clone, AtatCmd, Default)]
#[at_cmd("#GNSSDEINIT", GnssDeinitResponse, timeout_ms = 10000)]
pub struct GnssDeinit;

#[derive(Clone, AtatResp)]
pub struct GnssDeinitResponse;
