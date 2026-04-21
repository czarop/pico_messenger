#![no_std]
#![no_main]
use core::str::from_utf8;
use cyw43::aligned_bytes;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::{bind_interrupts, dma};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use embassy_net::dns::DnsQueryType;
use embedded_io_async::Write;
use embedded_nal_async::TcpConnect;
use embedded_tls::{Aes128GcmSha256, TlsConfig, TlsConnection, TlsContext, UnsecureProvider};

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<
        'static,
        cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>,
        cyw43::Cyw43439,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    const WIFI_NETWORK: &str = env!("WIFI_SSID");
    const WIFI_PASSWORD: &str = env!("WIFI_PASS");

    let fw = aligned_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
    let nvram = aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");
    info!("fw={} clm={} nvram={}", fw.len(), clm.len(), nvram.len());

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download ../../cyw43-firmware/43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download ../../cyw43-firmware/43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    // let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    // let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    // let fw: &Aligned<A4, [u8]> = unsafe {
    //     &*(core::slice::from_raw_parts(0x10100000 as *const u8, 231077) as *const [u8]
    //         as *const Aligned<A4, [u8]>)
    // };
    // let clm: &Aligned<A4, [u8]> = unsafe {
    //     &*(core::slice::from_raw_parts(0x10140000 as *const u8, 984) as *const [u8]
    //         as *const Aligned<A4, [u8]>)
    // };
    // let nvram: &Aligned<A4, [u8]> = unsafe {
    //     &*(core::slice::from_raw_parts(0x10150000 as *const u8, 742) as *const [u8]
    //         as *const Aligned<A4, [u8]>)
    // };
    // info!("fw={} clm={} nvram={}", fw.len(), clm.len(), nvram.len());

    let pwr = Output::new(p.PIN_23, Level::Low);

    Timer::after_millis(250).await; // give chip time to boot
    info!("pwr on, starting cyw43");
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        dma::Channel::new(p.DMA_CH0, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;
    spawner.spawn(unwrap!(cyw43_task(runner)));
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = embassy_net::Config::dhcpv4(Default::default());
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let seed: u64 = RoscRng.next_u64();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(unwrap!(net_task(runner)));

    while let Err(err) = control
        .join(
            WIFI_NETWORK,
            cyw43::JoinOptions::new(WIFI_PASSWORD.as_bytes()),
        )
        .await
    {
        info!("join failed: {:?}", err);
    }

    info!("waiting for link...");
    stack.wait_link_up().await;

    info!("waiting for DHCP...");
    stack.wait_config_up().await;

    // And now we can use it!
    info!("Stack is up!");

    match stack.config_v4() {
        Some(a) => info!("IP Address appears to be: {}", a.address),
        None => core::panic!("DHCP completed but no IP address was assigned!"),
    }
    let mut rx_buffer = [0; 4096];
    let mut tls_read_buffer = [0; 16640];
    let mut tls_write_buffer = [0; 16640];

    let client_state = TcpClientState::<1, 1024, 1024>::new();
    let tcp_client = TcpClient::new(stack, &client_state);
    let dns_client = DnsSocket::new(stack);

    let url = "https://www.google.com";
    let ip = dns_client
        .query("google.com", DnsQueryType::A)
        .await
        .unwrap()[0];

    let addr = embassy_net::IpEndpoint::new(ip, 443);

    // 2. TCP connect
    let mut conn = tcp_client.connect(addr.into()).await.unwrap();

    // 3. Wrap in TLS
    let mut tls = TlsConnection::new(&mut conn, &mut tls_read_buffer, &mut tls_write_buffer);

    let mut rng = embassy_rp::clocks::RoscRng; // adjust if yours is different
    let mut provider = UnsecureProvider::new::<Aes128GcmSha256>(&mut rng);
    let config = TlsConfig::new().with_server_name("google.com");

    tls.open(TlsContext::new(&config, &mut provider))
        .await
        .unwrap();

    // 4. Send request
    tls.write_all(
        b"GET / HTTP/1.1\r\n\
      Host: www.google.com\r\n\
      Connection: close\r\n\
      \r\n",
    )
    .await
    .unwrap();
    tls.flush().await.unwrap();

    // 5. Read response (just enough for headers)
    let n = tls.read(&mut rx_buffer).await.unwrap();

    // 6. Parse status code - "HTTP/1.1 301 ..." -> bytes 9..12
    let status = core::str::from_utf8(&rx_buffer[9..12]).unwrap();
    info!("Status: {}", status); // expect 301 for google.com

    if status != "200" && status != "301" {
        info!("Response was not OK, not reading body");
        loop {
            Timer::after(Duration::from_secs(1)).await;
        }
    } else {
        let delay = Duration::from_secs(1);
        for _ in 0..3 {
            control.gpio_set(0, true).await;
            Timer::after(delay).await;
            control.gpio_set(0, false).await;
            Timer::after(delay).await;
        }
    }
}
