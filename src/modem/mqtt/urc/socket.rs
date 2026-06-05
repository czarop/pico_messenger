use atat::atat_derive::AtatResp;

#[derive(Clone, AtatResp, Debug)]
pub struct SocketClosedUrc {
    pub context_id: u8,
    pub socket_id: u8,
}
