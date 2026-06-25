// use heapless::String;
// use serde::Deserialize;

// #[derive(Clone, Deserialize, Debug, defmt::Format)]
// pub struct MqttRecvUrc {
//     pub topic: String<50>,
//     pub payload: String<50>,
// }

use heapless::String;
use serde::Deserialize;
 
/// Raw `#MQTTRECV: <topicName>,<payload>` URC.
///
/// atat's unquoted-string deserializer (`parse_bytes`) consumes every printable
/// byte up to the line terminator and does NOT stop at commas, so a two-field
/// struct fails: the first field swallows the whole `topic,payload` and the
/// second field gets nothing (`EofWhileParsingValue`). We therefore capture the
/// entire remainder in one field and split it manually on the first comma -
/// the same pattern as `CgevUrc`. The topic name carries no comma here, so the
/// first comma is the divider and any commas inside the payload are preserved.
#[derive(Clone, Deserialize, Debug, defmt::Format)]
pub struct MqttRecvUrc {
    // "<topic>,<payload>"; topic and payload are each capped at 50 by the modem,
    // so 110 leaves headroom for the comma and any leading space.
    pub body: String<110>,
}
 
/// Topic and payload split out from an `#MQTTRECV` URC.
#[derive(Clone, Debug, defmt::Format)]
pub struct MqttMessage {
    pub topic: String<50>,
    pub payload: String<50>,
}
 
impl MqttRecvUrc {
    /// Split the raw body into topic and payload on the first comma.
    /// Returns `None` if there is no comma or either part exceeds 50 chars.
    pub fn message(&self) -> Option<MqttMessage> {
        let s = self.body.as_str().trim_start();
        let (topic, payload) = s.split_once(',')?;
        Some(MqttMessage {
            topic: String::try_from(topic).ok()?,
            payload: String::try_from(payload).ok()?,
        })
    }
}