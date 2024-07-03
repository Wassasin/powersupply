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

    // Do USB-PD first thing, because the protocol demands it.
    let usb_pd = systems::usb_pd::USBPD::init(bsp.usb_pd, &spawner).await;

    let storage = systems::storage::Storage::init().await;
    let config = systems::config::Config::init(storage, &spawner).await;
    let record = systems::record::Record::init(storage, &spawner).await;
    let stats = systems::stats::Stats::init(bsp.stats, &spawner);

    let net = systems::net::Net::init(bsp.wifi, &spawner).await;

    let _power_ext = systems::power_ext::PowerExt::init(
        bsp.power_ext,
        usb_pd,
        record,
        config,
        &bsp.high_prio_spawner,
    )
    .await;

    let mut stats_subscriber = stats.subscriber();
    let mut record_subscriber = record.subscriber();
    let mut config_subscriber = config.subscriber();
    loop {
        use embassy_futures::select::Either3;
        use embassy_sync::pubsub::WaitResult;

        match embassy_futures::select::select3(
            stats_subscriber.next_message(),
            record_subscriber.next_message(),
            config_subscriber.next_message(),
        )
        .await
        {
            Either3::First(WaitResult::Message(message)) => {
                log::info!("Stats {:#?}", message);
                net.send(
                    systems::net::Message::new(&systems::net::Topic::Stats, &message).unwrap(),
                )
                .await;
            }
            Either3::Second(WaitResult::Message(message)) => {
                log::info!("Record {:#?}", message);
                net.send(
                    systems::net::Message::new(&systems::net::Topic::Record, &message).unwrap(),
                )
                .await;
            }
            Either3::Third(WaitResult::Message(message)) => {
                log::info!("Config {:#?}", message);
                net.send(
                    systems::net::Message::new(&systems::net::Topic::Config, &message).unwrap(),
                )
                .await;
            }
            _ => {}
        }
    }
}
