use core::fmt::Debug;

use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    mutex::Mutex,
    pubsub::{PubSubChannel, Subscriber},
};
use embassy_time::{Duration, Timer};
use serde::Serialize;
use static_cell::StaticCell;

use crate::bsp;

#[derive(Serialize, Clone)]
pub struct Millivolts(u16);

impl Debug for Millivolts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{}mV", self.0))
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct StatsData {
    pub vsupply: Millivolts,
    pub vprog: Millivolts,
    pub vout: Millivolts,
}

const DATA_CAP: usize = 1;
const DATA_SUBS: usize = 4;
const DATA_PUBS: usize = 1;

type DataChannel<T> = PubSubChannel<NoopRawMutex, T, DATA_CAP, DATA_SUBS, DATA_PUBS>;
pub type DataSubscriber<T> = Subscriber<'static, NoopRawMutex, T, DATA_CAP, DATA_SUBS, DATA_PUBS>;

pub struct Stats {
    data: Mutex<NoopRawMutex, Option<StatsData>>,
    notifier: DataChannel<StatsData>,
}

impl Stats {
    pub fn init(bsp: bsp::Stats, spawner: &Spawner) -> &'static Self {
        static STATS: StaticCell<Stats> = StaticCell::new();
        let stats = STATS.init(Stats {
            data: Mutex::new(None),
            notifier: DataChannel::new(),
        });

        spawner.spawn(task(bsp, stats)).unwrap();

        stats
    }

    pub async fn latest_data(&self) -> Option<StatsData> {
        self.data.lock().await.clone()
    }

    pub fn subscriber(&'static self) -> DataSubscriber<StatsData> {
        self.notifier.subscriber().unwrap()
    }
}

async fn poll_until_ready<T, E>(mut f: impl FnMut() -> nb::Result<T, E>) -> Result<T, E> {
    loop {
        match f() {
            Ok(value) => return Ok(value),
            Err(nb::Error::WouldBlock) => {
                Timer::after(Duration::from_millis(1)).await;
                continue;
            }
            Err(nb::Error::Other(e)) => return Err(e),
        }
    }
}

fn factor_vprog(value: u16) -> Millivolts {
    // voltage divider (100 + 100) / 100
    Millivolts(value * 2)
}

fn factor_high(value: u16) -> Millivolts {
    // voltage divider (887 + 100) / 100
    Millivolts(((value as u32) * 987 / 100) as u16)
}

#[embassy_executor::task]
async fn task(mut bsp: bsp::Stats, system: &'static Stats) {
    let publisher = system.notifier.publisher().unwrap();

    loop {
        let vsupply = poll_until_ready(|| bsp.adc.read_oneshot(&mut bsp.pins.vsupply))
            .await
            .unwrap();
        Timer::after(Duration::from_millis(1)).await;
        let vprog = poll_until_ready(|| bsp.adc.read_oneshot(&mut bsp.pins.vprog))
            .await
            .unwrap();
        Timer::after(Duration::from_millis(1)).await;
        let vout = poll_until_ready(|| bsp.adc.read_oneshot(&mut bsp.pins.vout))
            .await
            .unwrap();
        Timer::after(Duration::from_millis(1)).await;

        let vsupply = factor_high(vsupply);
        let vprog = factor_vprog(vprog);
        let vout = factor_high(vout);

        let data = StatsData {
            vsupply,
            vprog,
            vout,
        };

        system.data.lock().await.replace(data.clone());

        if publisher.try_publish(data).is_err() {
            log::warn!("Notifier queue full, stats messages are not picked up on time");
        }

        Timer::after(Duration::from_millis(1000)).await;
    }
}
