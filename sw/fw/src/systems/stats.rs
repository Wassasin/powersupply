//! Non-persistent device metrics.

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Timer};
use serde::Serialize;
use static_cell::StaticCell;

use crate::{
    bsp,
    systems::power_ext::PowerExt,
    util::{Millivolts, PubSub, Sub},
};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Data {
    pub vsupply_mv: Millivolts,
    pub vprog_mv: Millivolts,
    pub vout_mv: Millivolts,
    pub uptime_secs: u64,
    pub vout_state: crate::systems::power_ext::State,
}

pub struct Stats {
    data: Mutex<NoopRawMutex, Option<Data>>,
    notifier: PubSub<Data>,
}

impl Stats {
    pub fn init(bsp: bsp::Stats, power_ext: &'static PowerExt, spawner: &Spawner) -> &'static Self {
        static STATS: StaticCell<Stats> = StaticCell::new();
        let stats = STATS.init(Stats {
            data: Mutex::new(None),
            notifier: PubSub::new(),
        });

        spawner.spawn(task(bsp, power_ext, stats)).unwrap();

        stats
    }

    pub async fn latest_data(&self) -> Option<Data> {
        self.data.lock().await.clone()
    }

    pub fn subscriber(&'static self) -> Sub<Data> {
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
async fn task(mut bsp: bsp::Stats, power_ext: &'static PowerExt, system: &'static Stats) {
    let publisher = system.notifier.publisher().unwrap();

    loop {
        let vsupply = poll_until_ready(|| bsp.adc.read_oneshot(&mut bsp.pins.vsupply))
            .await
            .unwrap();
        let vprog = poll_until_ready(|| bsp.adc.read_oneshot(&mut bsp.pins.vprog))
            .await
            .unwrap();
        let vout = poll_until_ready(|| bsp.adc.read_oneshot(&mut bsp.pins.vout))
            .await
            .unwrap();

        let vsupply_mv = factor_high(vsupply);
        let vprog_mv = factor_vprog(vprog);
        let vout_mv = factor_high(vout);

        let data = Data {
            vsupply_mv,
            vprog_mv,
            vout_mv,
            uptime_secs: Instant::now().as_secs(),
            vout_state: power_ext.state().await,
        };

        system.data.lock().await.replace(data.clone());

        if publisher.try_publish(data).is_err() {
            log::warn!("Notifier queue full, stats messages are not picked up on time");
        }

        Timer::after(Duration::from_millis(1000)).await;
    }
}
