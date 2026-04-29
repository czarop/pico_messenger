// the response from the modem - e.g. a message is received
// could also be.. loss of signal, error etc etc
#[derive(Clone, AtatUrc)]
pub enum Urc {
    #[at_urc("+UMWI")]
    MessageWaitingIndication(MqttResponse),
}
