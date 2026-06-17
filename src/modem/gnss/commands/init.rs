use atat::atat_derive::{AtatCmd, AtatEnum, AtatLen, AtatResp};
use heapless::String;
use serde::Serialize;

#[derive(AtatEnum, Clone)]
#[repr(u8)]
pub enum Constellation {
    Gps = 0,
    Galileo = 1,
    GpsGalileo = 2,
}

#[derive(AtatEnum, Clone)]
#[repr(u8)]
pub enum Assistance {
    ColdStart = 0,
    Auto = 1,
    SUPLHotStart = 2,
}

#[derive(AtatLen, Clone, Serialize)]
pub struct SuplParams {
    pub context_id: u8,
    pub supl_server_ip: String<39>,
    pub supl_server_port: u16,
    pub supl_session_timeout: Option<u16>,
    pub security_profile_id: Option<u8>,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("#GNSSINIT", GnssInitResponse, timeout_ms = 10000)]
pub struct GnssInit {
    #[at_arg(position = 0)]
    assistance: Assistance,
    #[at_arg(position = 1)]
    constellation: Constellation,
    #[at_arg(position = 2)]
    context_id: Option<u8>,
    #[at_arg(position = 3)]
    supl_server_ip: Option<String<39>>,
    #[at_arg(position = 4)]
    supl_server_port: Option<u16>,
    #[at_arg(position = 5)]
    supl_session_timeout: Option<u16>,
    #[at_arg(position = 6)]
    security_profile_id: Option<u8>,
}

impl Default for GnssInit {
    fn default() -> Self {
        Self {
            assistance: Assistance::ColdStart,
            constellation: Constellation::Gps,
            context_id: None,
            supl_server_ip: None,
            supl_server_port: None,
            supl_session_timeout: None,
            security_profile_id: None,
        }
    }
}

impl GnssInit {
    pub fn new(
        assistance: Assistance,
        constellation: Constellation,
        supl_params: Option<SuplParams>,
    ) -> Self {
        if let Some(params) = supl_params {
            Self {
                assistance,
                constellation,
                context_id: Some(params.context_id),
                supl_server_ip: Some(params.supl_server_ip),
                supl_server_port: Some(params.supl_server_port),
                supl_session_timeout: params.supl_session_timeout,
                security_profile_id: params.security_profile_id,
            }
        } else {
            Self {
                assistance,
                constellation,
                ..Default::default()
            }
        }
    }
}

#[derive(Clone, AtatResp)]
pub struct GnssInitResponse {}
