use atat::{AtatIngress, DefaultDigester, Ingress, ResponseSlot, UrcChannel, asynch::Client};

use crate::modem::command::modem_command_task;
use crate::modem::urc::{ModemUrc, modem_urc_task};
use embassy_executor::Spawner;
use embassy_rp::peripherals::{PIN_12, PIN_13, UART0};
use embassy_rp::{
    Peri, bind_interrupts,
    uart::{self, BufferedInterruptHandler, BufferedUart, BufferedUartRx},
};
use static_cell::StaticCell;

pub const INGRESS_BUF_SIZE: usize = 1024; // where incoming requests sit
pub const URC_CAPACITY: usize = 128; // number of broadcast 'messages' in the queue
pub const URC_SUBSCRIBERS: usize = 3; // number of async tasks listening for broadcast messages

bind_interrupts!(struct Irqs {
    UART0_IRQ => BufferedInterruptHandler<UART0>;
}); // interrupt ongoing tasks to add incoming messages to the buffer

pub async fn initiate_modem(
    spawner: Spawner,
    tx_pin: Peri<'static, PIN_12>,
    rx_pin: Peri<'static, PIN_13>,
    uart: Peri<'static, UART0>,
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
    spawner.spawn(modem_urc_task(URC_CHANNEL.subscribe().unwrap()).unwrap());

    // for messaging
    let client = CLIENT.init(Client::new(
        writer,
        &RES_SLOT,
        BUF.init([0; 1024]),
        atat::Config::default(),
    ));

    spawner.spawn(modem_command_task(client).unwrap());
    // returns here - everything is owned by the spawned tasks
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
    ingress.read_from(&mut reader).await
}
