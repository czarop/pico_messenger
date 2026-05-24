use heapless::String;
use serde::Deserialize;

#[derive(Clone, Deserialize, Debug)]
pub struct MqttRecvUrc {
    pub topic: String<50>,
    pub payload: String<50>,
}
