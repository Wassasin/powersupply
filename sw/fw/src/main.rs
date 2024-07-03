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

use esp_hal::{peripherals::Peripherals, prelude::*};
use esp_println::println;
use systems::events::Events;

use crate::{bsp::Bsp, serialnumber::SerialNumber};

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

    // Do USB-PD first thing, because the protocol demands it.
    let usb_pd = systems::usb_pd::USBPD::init(bsp.usb_pd, &spawner).await;

    let storage = systems::storage::Storage::init().await;
    let config = systems::config::Config::init(storage, &spawner).await;
    let record = systems::record::Record::init(storage, &spawner).await;

    let power_ext = systems::power_ext::PowerExt::init(
        bsp.power_ext,
        usb_pd,
        record,
        config,
        &bsp.high_prio_spawner,
    )
    .await;

    let stats = systems::stats::Stats::init(bsp.stats, power_ext, &spawner);

    let net = systems::net::Net::init(bsp.wifi, &spawner).await;

    Events::init(stats, record, config, net, &spawner).await;
}
