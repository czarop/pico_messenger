use heapless::String;
use serde::Deserialize;

#[derive(Clone, Deserialize, Debug, defmt::Format)]
pub struct MqttRecvUrc {
    pub topic: String<50>,
    pub payload: String<50>,
}
