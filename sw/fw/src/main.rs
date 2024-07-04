#![no_main]
#![no_std]

#[allow(unused)]
mod bsp;

mod executor;
mod logger;
mod serialnumber;
mod systems;
mod util;

// We plan on open-sourcing all drivers eventually.
// Hence allow unused code, which will be useful for the eventual library crates.
#[allow(unused)]
mod drivers;

use embassy_executor::Spawner;
use embassy_time::Timer;
use esp_backtrace as _;

use esp_hal::{peripherals::Peripherals, prelude::*};
use esp_println::println;
use systems::{
    events::Events,
    watchdog::{self, Watchdog},
};

use crate::{bsp::Bsp, serialnumber::SerialNumber};

#[doc(hidden)]
unsafe fn __make_static<T>(t: &mut T) -> &'static mut T {
    ::core::mem::transmute(t)
}

#[entry]
fn main() -> ! {
    let mut executor = executor::Executor::new();
    let executor = unsafe { __make_static(&mut executor) };
    executor.run(|spawner| {
        spawner.must_spawn(app(spawner));
    })
}

#[embassy_executor::task]
async fn app(spawner: Spawner) {
    println!("=== SARIF Slakkotron application ===");
    logger::init_logger_from_env();

    let reset_reason = esp_hal::reset::get_reset_reason();
    let wakeup_cause = esp_hal::reset::get_wakeup_cause();

    log::info!("Serial: {}", SerialNumber::fetch());
    log::info!("Reset reason: {:?}", reset_reason);
    log::info!("Wakeup cause: {:?}", wakeup_cause);

    let bsp = Bsp::init(Peripherals::take());
    let watchdog = Watchdog::init(bsp.watchdog).await;
    let watchdog_ticket = watchdog.ticket().await;

    // Do USB-PD first thing, because the protocol demands it.
    let usb_pd = systems::usb_pd::Usbpd::init(bsp.usb_pd, &spawner).await;

    let storage = systems::storage::Storage::init().await;
    let config = systems::config::Config::init(storage, &spawner).await;
    let record = systems::record::Record::init(storage, &spawner).await;

    let power_ext = systems::power_ext::PowerExt::init(
        bsp.power_ext,
        usb_pd,
        record,
        config,
        watchdog,
        &bsp.high_prio_spawner,
    )
    .await;

    let stats = systems::stats::Stats::init(bsp.stats, power_ext, &spawner);
    let net = systems::net::Net::init(bsp.wifi, config, watchdog, &spawner).await;

    Events::init(stats, record, config, net, &spawner).await;

    loop {
        watchdog_ticket.feed().await;
        Timer::after(watchdog::WATCHDOG_DEADLINE).await;
    }
}
