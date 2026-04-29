use atat::{
    AtatIngress, DefaultDigester, Ingress, ResponseSlot, UrcChannel,
    asynch::{AtatClient, Client},
};

use crate::mqtt::{commands, responses};
use embassy_executor::Spawner;
use embassy_rp::peripherals::{PIN_0, PIN_1, UART0};
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

pub async fn initiate_mqtt(
    spawner: Spawner,
    tx_pin: Peri<'static, PIN_0>,
    rx_pin: Peri<'static, PIN_1>,
    uart: Peri<'static, UART0>,
) {
    // statically allocate the mutable memory for the buffers
    static INGRESS_BUF: StaticCell<[u8; INGRESS_BUF_SIZE]> = StaticCell::new();
    // just for the UART
    static TX_BUF: StaticCell<[u8; 16]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; 16]> = StaticCell::new();
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
    // ingress tasks will own the reader, main task will own the writer
    let (writer, reader) = uart.split();

    // the response slot:
    // Main task sends a command and watches res_slot for a response
    // ingress task accepts bytes and understands there is a response - sends to res_slot
    static RES_SLOT: ResponseSlot<INGRESS_BUF_SIZE> = ResponseSlot::new();
    // a broadcast task to send messages like 'signal lost' to all listeners
    static URC_CHANNEL: UrcChannel<responses::Urc, URC_CAPACITY, URC_SUBSCRIBERS> =
        UrcChannel::new();

    // uses a Digester (which knows the AT command syntax) to turn raw bytes into Rust enums
    let ingress = Ingress::new(
        DefaultDigester::<responses::Urc>::default(),
        INGRESS_BUF.init([0; INGRESS_BUF_SIZE]),
        &RES_SLOT,
        &URC_CHANNEL,
    );

    // the voice of the sender - turns rust struct into AT command
    static BUF: StaticCell<[u8; 1024]> = StaticCell::new();
    let mut client = Client::new(
        writer,
        &RES_SLOT,
        BUF.init([0; 1024]),
        atat::Config::default(),
    );

    // the task that watches UART for incoming messages
    let token = ingress_task(ingress, reader);
    spawner.spawn(token.unwrap());

    // state machine - send command and wait for responses coming through uart
    // via
    let mut state: u8 = 0;
    loop {
        // Currently these will all timeout after 1 sec, as there is no response
        // .ok() should be replaced with error checking from the modem
        match state {
            0 => {
                client.send(&commands::GetManufacturerId).await.ok();
            }
            1 => {
                client.send(&commands::GetModelId).await.ok();
            }
            2 => {
                client.send(&commands::GetSoftwareVersion).await.ok();
            }
            3 => {
                client.send(&commands::GetWifiMac).await.ok();
            }
            _ => cortex_m::asm::bkpt(),
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;

        state += 1;
    }
}

// the listener task that converts uart to atat
#[embassy_executor::task]
async fn ingress_task(
    mut ingress: Ingress<
        'static,
        DefaultDigester<responses::Urc>,
        responses::Urc,
        INGRESS_BUF_SIZE,
        URC_CAPACITY,
        URC_SUBSCRIBERS,
    >,
    mut reader: BufferedUartRx,
) -> ! {
    ingress.read_from(&mut reader).await
}
