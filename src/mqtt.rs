use rust_mqtt::{buffer::AllocBuffer, client::{event::Event, options::ConnectOptions}, config::SessionExpiryInterval, session::CPublishFlightState, types::{MqttBinary, MqttString}};
use rust_mqtt::client::options::SubscriptionOptions;
use rust_mqtt::client::options::PublicationOptions;
use rust_mqtt::client::options::DisconnectOptions;

pub async fn connect_mqtt() {
    let mut buffer = AllocBuffer;
    let mut client = rust_mqtt::client::Client::new(&mut buffer);

    let transport = ...;    // Any Read/Write implementation (TCP, TLS, ...)

    let connect_options = ConnectOptions::new()
        .clean_start()
        .session_expiry_interval(SessionExpiryInterval::NeverEnd)
        .user_name(MqttString::from_str("user").unwrap())
        .password(MqttBinary::from_slice("pass".as_bytes()).unwrap());

    client.connect(
        transport,
        &connect_options,
        Some(MqttString::from_str("rust-mqtt-demo").unwrap()),
    ).await.unwrap();

    let topic = rust_mqtt::types::TopicName::new(MqttString::from_str("demo/topic").unwrap()).unwrap();

    client.subscribe(
        topic.as_borrowed().into(),
        SubscriptionOptions::new().exactly_once(),
    ).await.unwrap();

    let packet_identifier = client.publish(
        &PublicationOptions::new(topic.as_borrowed().into()).exactly_once(),
        "Hello World!".into(),
    ).await.unwrap().unwrap();

    while let Ok(event) = client.poll().await {
        if let Event::PublishComplete(_) = event {
            // Publish succeeded, we can disconnect
            client.disconnect(&DisconnectOptions::new()).await.unwrap();
            return;
        }
    }

    // An error has occured (e.g. network failure)
    client.abort().await;

    let transport = ...;    // Open a fresh connection

    client.connect(
        transport,
        &connect_options,
        Some(MqttString::from_str("rust-mqtt-demo").unwrap()),
    ).await.unwrap();


    // Recover the in-flight Quality of Service 2 publish.

    match client.session().cpublish_flight_state(packet_identifier) {
        // - Republish if PUBLISH / PUBREC may have been lost
        Some(CPublishFlightState::AwaitingPubrec) => client.republish(
            packet_identifier,
            &PublicationOptions::new(topic.into()).exactly_once(),
            "Hello World!".into(),
        ).await.unwrap(),
        // - Re-release if PUBREL / PUBCOMP may have been lost
        Some(CPublishFlightState::AwaitingPubcomp) => client.rerelease().await.unwrap(),
        // - Flight state already completed
        _ => {}
    }
}