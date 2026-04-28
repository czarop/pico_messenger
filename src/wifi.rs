use crate::startup::Irqs;
use cyw43::aligned_bytes;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::{Stack, StackResources};
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_24, PIN_25, PIN_29, PIO0};
use embassy_rp::pio::Pio;
use embassy_rp::{Peri, dma, peripherals::PIN_23};
use embassy_time::Timer;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

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

pub static STATE: StaticCell<cyw43::State> = StaticCell::new();
pub static STACK: StaticCell<Stack> = StaticCell::new();
pub static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();

pub async fn connect_wifi(
    spawner: &Spawner,
    pin23: Peri<'static, PIN_23>,
    pin24: Peri<'static, PIN_24>,
    pin25: Peri<'static, PIN_25>,
    pin29: Peri<'static, PIN_29>,
    pio0: Peri<'static, PIO0>,
    dma_ch0: Peri<'static, DMA_CH0>,
) -> &'static Stack<'static> {
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

    let pwr = Output::new(pin23, Level::Low);

    Timer::after_millis(250).await; // give chip time to boot
    info!("pwr on, starting cyw43");
    let cs = Output::new(pin25, Level::High);
    let mut pio = Pio::new(pio0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        pin24,
        pin29,
        dma::Channel::new(dma_ch0, Irqs),
    );
    let state = STATE.init_with(|| cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;
    spawner.spawn(unwrap!(cyw43_task(runner)));
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = embassy_net::Config::dhcpv4(Default::default());
    // static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let seed: u64 = RoscRng.next_u64();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init_with(|| StackResources::new()),
        seed,
    );
    let stack = STACK.init(stack);

    spawner.spawn(unwrap!(net_task(runner)));
    let delay = embassy_time::Duration::from_secs(5);
    while let Err(err) = control
        .join(
            WIFI_NETWORK,
            cyw43::JoinOptions::new(WIFI_PASSWORD.as_bytes()),
        )
        .await
    {
        info!("join failed: {:?}", err);
        Timer::after(delay).await;
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

    stack
}
