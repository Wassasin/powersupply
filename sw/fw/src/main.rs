#![no_main]
#![no_std]

#[allow(unused)]
mod bsp;

mod logger;
mod serialnumber;
mod systems;
mod util;

// We plan on open-sourcing all drivers eventually.
// Hence allow unused code, which will be useful for the eventual library crates.
#[allow(unused)]
mod drivers;

use embassy_executor::Spawner;
use esp_backtrace as _;

use esp_hal::{delay::Delay, peripherals::Peripherals, prelude::*, rtc_cntl::Rtc};
use esp_println::println;

use crate::{bsp::Bsp, serialnumber::SerialNumber};

pub struct State {
    spawner: Spawner,

    rtc: Rtc<'static>,
    delay: Delay,
}

#[main]
async fn main(spawner: Spawner) {
    println!("=== SARIF Slakkotron main ===");
    logger::init_logger_from_env();

    let reset_reason = esp_hal::reset::get_reset_reason();
    let wakeup_cause = esp_hal::reset::get_wakeup_cause();

    log::info!("Serial: {}", SerialNumber::fetch());
    log::info!("Reset reason: {:?}", reset_reason);
    log::info!("Wakeup cause: {:?}", wakeup_cause);

    let bsp = Bsp::init(Peripherals::take());

    let net = systems::net::Net::init(bsp.wifi, &spawner).await;
    let stats = systems::stats::Stats::init(bsp.stats, &spawner);
    let power_ext = systems::power_ext::PowerExt::init(bsp.power_ext, &spawner).await;

    let mut subscriber = stats.subscriber();
    loop {
        match subscriber.next_message().await {
            embassy_sync::pubsub::WaitResult::Lagged(_) => {}
            embassy_sync::pubsub::WaitResult::Message(message) => {
                log::info!("{:#?}", message);
                net.send(
                    systems::net::Message::new(&systems::net::Topic::Stats, &message).unwrap(),
                )
                .await;
            }
        }
    }
}
