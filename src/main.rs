#![no_std]
#![no_main]

use embassy_executor::Spawner;
use pico_messenger::startup::startup;
use {defmt_rtt as _, panic_probe as _};



#[embassy_executor::main]
async fn main(spawner: Spawner) {
    
    
    startup(spawner).await;



    // // 5. Read response (just enough for headers)
    // let n = tls.read(&mut rx_buffer).await.unwrap();

    // // 6. Parse status code - "HTTP/1.1 301 ..." -> bytes 9..12
    // let status = core::str::from_utf8(&rx_buffer[9..12]).unwrap();
    // info!("Status: {}", status); // expect 301 for google.com

    // if status != "200" && status != "301" {
    //     info!("Response was not OK, not reading body");
    //     loop {
    //         Timer::after(Duration::from_secs(1)).await;
    //     }
    // } else {
    //     let delay = Duration::from_secs(1);
    //     for _ in 0..3 {
    //         control.gpio_set(0, true).await;
    //         Timer::after(delay).await;
    //         control.gpio_set(0, false).await;
    //         Timer::after(delay).await;
    //     }
    // }
}
