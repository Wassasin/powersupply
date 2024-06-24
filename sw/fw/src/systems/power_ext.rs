use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

use crate::{
    bsp::{self, I2cBusDevice, I2cError},
    drivers::tps55289::{ll::Tps55289, IntFB, VRef},
    util::Millivolts,
};

pub struct PowerExt {
    ll: Mutex<NoopRawMutex, Tps55289<I2cBusDevice, I2cError>>,
}

impl PowerExt {
    pub async fn init(mut bsp: bsp::PowerExt, spawner: &Spawner) -> &'static Self {
        let ll = Mutex::new(Tps55289::new(bsp.i2c));

        bsp.enable_pin.set_high();

        Timer::after(Duration::from_millis(50)).await;

        static STATS: StaticCell<PowerExt> = StaticCell::new();
        let stats = STATS.init(Self { ll });

        let ratio = IntFB::Ratio0_0564;
        let vref = VRef::from_feedback(Millivolts(9000), ratio);

        {
            let mut ll = stats.ll.lock().await;

            ll.vref().write_async(|w| w.vref(vref)).await.unwrap();
            ll.vout_fs().modify_async(|w| w.intfb(ratio)).await.unwrap();
            ll.iout_limit()
                .modify_async(|w| w.setting(0x10))
                .await
                .unwrap();
            ll.mode()
                .modify_async(|w| w.dischg(false).oe(false))
                .await
                .unwrap();
        }

        spawner.must_spawn(task(bsp.nint_pin, stats));

        stats
    }
}

fn earliest_deadline(iter: impl Iterator<Item = Option<Instant>>) -> Option<Instant> {
    let mut res = None;

    for d in iter {
        if let Some(d) = d {
            if let Some(res) = res.as_mut() {
                if *res > d {
                    *res = d;
                }
            } else {
                res = Some(d);
            }
        }
    }

    res
}

#[embassy_executor::task]
async fn task(mut nint_pin: bsp::PowerExtNIntPin, system: &'static PowerExt) {
    let mut enabled = false;
    let mut backoff_until = None;

    const BACKOFF_DURATION: Duration = Duration::from_millis(1000);
    const MAX_DURATION: Duration = Duration::from_secs(5);

    loop {
        {
            let mut ll = system.ll.lock().await;
            let status = ll.status().read_async().await.unwrap();

            log::info!("{:?}", status);

            if status.ocp() || status.scp() {
                if enabled {
                    log::error!("OCP!");

                    ll.mode().modify_async(|w| w.oe(false)).await.unwrap();
                    enabled = false;
                    backoff_until = Some(Instant::now() + BACKOFF_DURATION);
                }
            } else if !enabled {
                let activate = if let Some(until) = backoff_until {
                    until < Instant::now()
                } else {
                    true
                };

                if activate {
                    log::info!("Enabling");

                    ll.mode().modify_async(|w| w.oe(true)).await.unwrap();
                    enabled = true;
                    backoff_until = None;
                }
            }
        }

        let awaken_anyway_at = Instant::now() + MAX_DURATION;
        let deadline =
            earliest_deadline([Some(awaken_anyway_at), backoff_until].into_iter()).unwrap();

        match embassy_futures::select::select(nint_pin.wait_for_falling_edge(), Timer::at(deadline))
            .await
        {
            embassy_futures::select::Either::First(_) => {}
            embassy_futures::select::Either::Second(_) => {}
        }
    }
}
