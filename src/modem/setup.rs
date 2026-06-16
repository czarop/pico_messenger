use atat::{AtatIngress, DefaultDigester, Ingress, ResponseSlot, UrcChannel, asynch::Client};
use embassy_rp::gpio::Output;


use crate::modem::command_task::command_task;
use crate::modem::urc::ModemUrc;
use crate::modem::mqtt::commands::config::MqttConfig;
use crate::modem::mqtt::commands::connect::MqttConnectStub;
use crate::modem::mqtt::commands::socket::SocketCreate;
use embassy_executor::Spawner;
use embassy_rp::peripherals::{PIN_4, PIN_5, UART1};
use embassy_rp::{
    Peri, bind_interrupts,
    uart::{self, BufferedInterruptHandler, BufferedUart, BufferedUartRx},
};
use static_cell::StaticCell;
use dotenvy_macro::dotenv;

pub const INGRESS_BUF_SIZE: usize = 1024; // where incoming requests sit
pub const URC_CAPACITY: usize = 128; // number of broadcast 'messages' in the queue
pub const URC_SUBSCRIBERS: usize = 3; // number of async tasks listening for broadcast messages

//mqtt params:
const BROKER_ADDRESS: &str = "test.mosquitto.org";
const BROKER_PORT: u16 = 1883;
const WILL_TOPIC: &str = "pico/mqtt/status";
const WILL_MESSAGE: &str = "offline";

const GNSS_INTERVAL: u32 = 10;
const GNSS_TOPIC: &str = "pico/mqtt/update";

bind_interrupts!(struct Irqs {
    UART1_IRQ => BufferedInterruptHandler<UART1>;
}); // interrupt ongoing tasks to add incoming messages to the buffer

pub fn initiate_modem(
    spawner: Spawner,
    tx_pin: Peri<'static, PIN_4>,
    rx_pin: Peri<'static, PIN_5>,
    gnss_bias: Output<'static>,
    uart: Peri<'static, UART1>,
) {
    // statically allocate the mutable memory for the buffers
    static INGRESS_BUF: StaticCell<[u8; INGRESS_BUF_SIZE]> = StaticCell::new();
    // just for the UART
    static TX_BUF: StaticCell<[u8; 16]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; 16]> = StaticCell::new();
    // the response slot - for responses to commands: only one at a time
    static RES_SLOT: ResponseSlot<INGRESS_BUF_SIZE> = ResponseSlot::new();
    // a broadcast task to send messages to all listeners when something is received
    static URC_CHANNEL: UrcChannel<ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS> = UrcChannel::new();
    static BUF: StaticCell<[u8; 1024]> = StaticCell::new();
    static CLIENT: StaticCell<Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>> =
        StaticCell::new();

    // combine the raw UART with the uart
    let uart = BufferedUart::new(
        uart,
        tx_pin,
        rx_pin,
        Irqs,
        TX_BUF.init([0; 16]),
        RX_BUF.init([0; 16]),
        uart::Config::default(),
    );

    // gives a reader and a writer
    // ingress tasks will own the reader, sender task will own the writer
    let (writer, reader) = uart.split();

    // uses a Digester (which knows the AT command syntax) to turn raw bytes into Rust enums
    let ingress = Ingress::new(
        DefaultDigester::<ModemUrc>::default(),
        INGRESS_BUF.init([0; INGRESS_BUF_SIZE]),
        &RES_SLOT,
        &URC_CHANNEL,
    );

    spawner.spawn(ingress_task(ingress, reader).unwrap());
    

    // for messaging
    let client = CLIENT.init(Client::new(
        writer,
        &RES_SLOT,
        BUF.init([0; 1024]),
        atat::Config::default(),
    ));

    spawner.spawn(command_task(client).unwrap());

    let urc_subscription = URC_CHANNEL.subscribe().expect("could not subscribe to urc channel");
    let socket = SocketCreate::new(10, 10);
    let mqtt_config = MqttConfig::default();
    let broker_address = heapless::String::try_from(BROKER_ADDRESS).unwrap();
    let broker_port = BROKER_PORT;
    let username = heapless::String::try_from(dotenv!("MQTT_USERNAME")).unwrap();
    let passwd = heapless::String::try_from(dotenv!("MQTT_PASSWORD")).unwrap();
    let will_topic = heapless::String::try_from(WILL_TOPIC).unwrap();
    let will_message = heapless::String::try_from(WILL_MESSAGE).unwrap();
    let mqtt_connection = MqttConnectStub::new(
        broker_address, 
        broker_port, 
        username, 
        passwd, 
        will_topic, 
        will_message
    );
    spawner.spawn(crate::modem::network_task::network_task(urc_subscription, socket, mqtt_config, mqtt_connection).unwrap());

    let urc_subscription = URC_CHANNEL.subscribe().expect("could not subscribe to urc channel");

    spawner.spawn(crate::modem::gnss_task::gnss_task( urc_subscription, gnss_bias).unwrap());

    spawner.spawn(crate::modem::modem_task::modem_task(GNSS_INTERVAL, heapless::String::try_from(GNSS_TOPIC).expect("topic error")).unwrap());
    // returns here - everything is owned by the spawned tasks
    defmt::info!("everything spawned");
}

// the listener task that converts uart to atat
#[embassy_executor::task]
async fn ingress_task(
    mut ingress: Ingress<
        'static,
        DefaultDigester<ModemUrc>,
        ModemUrc,
        INGRESS_BUF_SIZE,
        URC_CAPACITY,
        URC_SUBSCRIBERS,
    >,
    mut reader: BufferedUartRx,
) -> ! {
    // ingress.read_from(&mut reader).await

    use embedded_io_async::Read;
    loop {
        let buf = ingress.write_buf();
        if buf.is_empty() {
            ingress.clear();
            continue;
        }
        match reader.read(buf).await {
            Ok(received) => {
                if received > 0 {
                    defmt::info!("raw rx: {:?}", &buf[..received]);
                    ingress.advance(received).await;
                }
            }
            Err(_) => {
                ingress.clear();
            }
        }
    }
}
