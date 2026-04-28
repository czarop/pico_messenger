use crate::state::{EmbassyStorage, load_state, save_state};
use crate::telegram::UpdateResponse;
use crate::wifi::connect_wifi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};

use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::InterruptHandler;
use embassy_rp::{bind_interrupts, dma};
use embassy_time::{Duration, Timer};
use littlefs2::fs::{Allocation, Filesystem};
use {defmt_rtt as _, panic_probe as _};

use embassy_net::dns::DnsQueryType;
use embedded_io_async::Write as _;

use core::fmt::Write;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embedded_nal_async::TcpConnect;
use embedded_tls::{TlsConfig, TlsConnection, TlsContext, UnsecureProvider};
use heapless::String;
use static_cell::StaticCell;
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

static STORAGE: StaticCell<EmbassyStorage> = StaticCell::new();
static ALLOC: StaticCell<Allocation<EmbassyStorage>> = StaticCell::new();
static FS: StaticCell<Mutex<ThreadModeRawMutex, Filesystem<'static, EmbassyStorage>>> =
    StaticCell::new();

pub async fn startup(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // let stack = connect_wifi(
    //     &spawner, p.PIN_23, p.PIN_24, p.PIN_25, p.PIN_29, p.PIO0, p.DMA_CH0,
    // )
    // .await;

    let flash = embassy_rp::flash::Flash::new_blocking(p.FLASH);

    // StaticCell::init() gives you &'static mut T
    let storage = STORAGE.init(EmbassyStorage { flash });
    let alloc = ALLOC.init(Allocation::new());

    if Filesystem::mount(alloc, storage).is_err() {
        Filesystem::format(storage).expect("Format failed");
    }
    let fs = Filesystem::mount(alloc, storage).expect("Mount failed");

    // Wrap in Mutex and store globally
    let fs: &'static mut Mutex<_, _> = FS.init(Mutex::new(fs));

    let mut file_system = fs.lock().await;
    let mut state = load_state(&mut *file_system);
    info!("on startup message count was: {}", state.msg_count);
    let mut last_update_id: i64 = 0;

    let delay = Duration::from_secs(5);

    // const TOKEN: &str = "8699589319:AAFBpa8sBXzj8dXr5-_mvRwCb258LhMpsNY";

    //

    // let client_state = TcpClientState::<2, 1024, 1024>::new();

    // let tcp_client = TcpClient::new(*stack, &client_state);
    // let dns_client = DnsSocket::new(*stack);
    // let mut rx_buffer = [0; 4096];
    // let mut tls_read_buffer = [0; 16640];
    // let mut tls_write_buffer = [0; 16640];

    // loop {
    //     tls_read_buffer.fill(0);
    //     tls_write_buffer.fill(0);

    //     // 1. Resolve Telegram every iteration (or cache the IP)
    //     let ip = dns_client
    //         .query("api.telegram.org", DnsQueryType::A)
    //         .await
    //         .unwrap()[0];

    //     let addr = embassy_net::IpEndpoint::new(ip, 443);

    //     // 2. Fresh TCP + TLS connection each poll
    //     let mut conn = tcp_client.connect(addr.into()).await.unwrap();

    //     let mut rng = embassy_rp::clocks::RoscRng;

    //     let mut provider = UnsecureProvider::new::<embedded_tls::Aes256GcmSha384>(&mut rng);
    //     let mut tls = TlsConnection::<_, embedded_tls::Aes256GcmSha384>::new(
    //         &mut conn,
    //         &mut tls_read_buffer,
    //         &mut tls_write_buffer,
    //     );
    //     let config = TlsConfig::new()
    //         .with_server_name("api.telegram.org")
    //         .enable_rsa_signatures();

    //     tls.open(TlsContext::new(&config, &mut provider))
    //         .await
    //         .unwrap();

    //     // 3. Build and send request
    //     let offset = last_update_id + 1;
    //     let mut request: String<256> = String::new();
    //     core::write!(
    //         request,
    //         "GET /bot{TOKEN}/getUpdates?offset={offset} HTTP/1.1\r\n\
    //      Host: api.telegram.org\r\n\
    //      Connection: close\r\n\
    //      \r\n"
    //     )
    //     .unwrap();

    //     tls.write_all(request.as_bytes()).await.unwrap();
    //     tls.flush().await.unwrap();

    //     // 4. Read response
    //     let mut total = 0;
    //     loop {
    //         match tls.read(&mut rx_buffer[total..]).await {
    //             Ok(0) => break,
    //             Ok(n) => total += n,
    //             Err(_) => break,
    //         }
    //     }

    //     let n = total;

    //     info!("Read {} bytes total", n);
    //     if n > 0 {
    //         if let Ok(s) = core::str::from_utf8(&rx_buffer[..n.min(200)]) {
    //             info!("Response: {}", s);
    //         }
    //     } else {
    //         info!("No response body received");
    //     }

    //     let body_start = rx_buffer[..n]
    //         .windows(4)
    //         .position(|w| w == b"\r\n\r\n")
    //         .unwrap()
    //         + 4;
    //     let response = match postcard::from_bytes::<UpdateResponse>(&rx_buffer[body_start..n]) {
    //         Ok(res) => res,
    //         Err(_err) => {
    //             info!("Failed to parse JSON");
    //             continue;
    //         }
    //     };

    //     if &response.result.len() == &0 {
    //         info!("No new messages");
    //     } else {
    //         info!("Got {} updates", response.result.len());
    //     }

    //     for update in &response.result {
    //         info!("we have a message with update_id {}", update.update_id);
    //         last_update_id = update.update_id;
    //         if let Some(msg) = &update.message {
    //             if let Some(text) = msg.text {
    //                 info!("Got: {}", text);
    //             }
    //         }
    //     }

    //     Timer::after(delay).await;
    // }
}
