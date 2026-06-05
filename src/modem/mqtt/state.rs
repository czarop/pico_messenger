#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MqttStackState {
    Down,
    IpUp,
    SocketReady(u8),
    MqttReady,
}
