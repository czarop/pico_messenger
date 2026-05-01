use atat::{
    AtatIngress, DefaultDigester, Ingress, ResponseSlot, UrcChannel,
    asynch::{AtatClient, Client},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use heapless::String;

use crate::mqtt::{commands::{self, ModemCommand}, urc};
use embassy_executor::Spawner;
use embassy_rp::peripherals::{PIN_12, PIN_13, UART0};
use embassy_rp::{
    Peri, bind_interrupts,
    uart::{self, BufferedInterruptHandler, BufferedUart, BufferedUartRx},
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

const INGRESS_BUF_SIZE: usize = 1024; // where incoming requests sit
const URC_CAPACITY: usize = 128; // number of broadcast 'messages' in the queue
const URC_SUBSCRIBERS: usize = 3; // number of async tasks listening for broadcast messages

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
    static URC_CHANNEL: UrcChannel<urc::Urc, URC_CAPACITY, URC_SUBSCRIBERS> = UrcChannel::new();
    static BUF: StaticCell<[u8; 1024]> = StaticCell::new();
    static CLIENT: StaticCell<Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>> = StaticCell::new();

    // combine the raw UART with the uart
    let uart = BufferedUart::new(
        uart, tx_pin, rx_pin, Irqs,
        TX_BUF.init([0; 16]),
        RX_BUF.init([0; 16]),
        uart::Config::default(),
    );

    // gives a reader and a writer
    // ingress tasks will own the reader, sender task will own the writer
    let (writer, reader) = uart.split();

    // uses a Digester (which knows the AT command syntax) to turn raw bytes into Rust enums
    let ingress = Ingress::new(
        DefaultDigester::<urc::Urc>::default(),
        INGRESS_BUF.init([0; INGRESS_BUF_SIZE]),
        &RES_SLOT,
        &URC_CHANNEL,
    );

    spawner.spawn(ingress_task(ingress, reader).unwrap());
    spawner.spawn(urc_task(URC_CHANNEL.subscribe().unwrap()).unwrap());

    // for messaging
    let client = CLIENT.init(Client::new(
        writer,
        &RES_SLOT,
        BUF.init([0; 1024]),
        atat::Config::default(),
    ));

    spawner.spawn(modem_task(client).unwrap());
    // returns here - everything is owned by the spawned tasks
}

// the listener task that converts uart to atat
#[embassy_executor::task]
async fn ingress_task(
    mut ingress: Ingress<
        'static,
        DefaultDigester<urc::Urc>,
        urc::Urc,
        INGRESS_BUF_SIZE,
        URC_CAPACITY,
        URC_SUBSCRIBERS,
    >,
    mut reader: BufferedUartRx,
) -> ! {
    ingress.read_from(&mut reader).await
}

// react to messages
#[embassy_executor::task]
async fn urc_task(
    mut sub: atat::UrcSubscription<'static,urc::Urc,URC_CAPACITY,URC_SUBSCRIBERS> ,
) -> ! {
    loop {
        let urc = sub.next_message_pure().await;
        match urc {
            urc::Urc::MessageWaitingIndication(msg) => {
                // do something with msg
            }
            urc::Urc::NetworkRegistration(reg) => {
                // handle registration change
            }
            urc::Urc::IncomingSms(incoming_sms) => todo!(),
        }
    }
}

pub static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, ModemCommand, 4> = Channel::new();

#[embassy_executor::task]
async fn modem_task(
    client: &'static mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>
) -> ! {
    loop {
        let cmd = COMMAND_CHANNEL.receive().await;
        match cmd {
            ModemCommand::GetSignalStrength => {
                match client.send(&commands::GetManufacturerId).await {
                    Ok(resp) => { /* update some shared Signal or signal strength */ }
                    Err(e) => { /* log/handle */ }
                }
            }
            ModemCommand::SendSms { number, body } => {
                client.send(&commands::ExampleWithFields { arg1: 0, arg2: String::<64>::new() }).await.ok();
            }
            ModemCommand::Connect => todo!()
        }
    }
}