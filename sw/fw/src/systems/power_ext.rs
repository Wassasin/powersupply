use embassy_executor::SendSpawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

use crate::{
    bsp::{self, I2cBusDevice, I2cError},
    drivers::tps55289::{ll::Tps55289, IntFB, VRef},
    systems::{record::Record, usb_pd::USBPD},
    util::Millivolts,
};

pub struct PowerExt {
    ll: Mutex<CriticalSectionRawMutex, Tps55289<I2cBusDevice, I2cError>>,
    usbpd: &'static USBPD,
    record: &'static Record,
}

impl PowerExt {
    pub async fn init(
        mut bsp: bsp::PowerExt,
        usbpd: &'static USBPD,
        record: &'static Record,
        spawner: &SendSpawner,
    ) -> &'static Self {
        let ll = Mutex::new(Tps55289::new(bsp.i2c));

        bsp.enable_pin.set_high();

        Timer::after(Duration::from_millis(50)).await;

        static SYSTEM: StaticCell<PowerExt> = StaticCell::new();
        let system = SYSTEM.init(Self { ll, usbpd, record });

        let ratio = IntFB::Ratio0_0564;
        let vref = VRef::from_feedback(Millivolts(9000), ratio);

        {
            let mut ll = system.ll.lock().await;

            const CURRENT_SENSE_MILLIOHM: u32 = 20;
            let limit_ma = 500;
            let limit_uv = limit_ma * CURRENT_SENSE_MILLIOHM;
            let limit_value = (limit_uv / 500) as u8;

            ll.vref().write_async(|w| w.vref(vref)).await.unwrap();
            ll.vout_fs().modify_async(|w| w.intfb(ratio)).await.unwrap();
            ll.iout_limit()
                .modify_async(|w| w.setting(limit_value))
                .await
                .unwrap();
            ll.mode()
                .modify_async(|w| w.dischg(false).oe(false))
                .await
                .unwrap();
        }

        spawner.must_spawn(task(bsp.nint_pin, system));

        system
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
    let mut stabilized_at = None;
    let mut ocp_since = None;

    const BACKOFF_DURATION: Duration = Duration::from_millis(1000);
    const STABILIZATION_DURATION: Duration = Duration::from_millis(100);
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
                    stabilized_at = None;

                    if ocp_since.is_none() {
                        ocp_since = Some(Instant::now());
                    }

                    system.usbpd.set_pin(false).await;
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

                    system.usbpd.set_pin(true).await;

                    stabilized_at = Some(Instant::now() + STABILIZATION_DURATION);
                }
            } else if let Some(at) = stabilized_at {
                if at < Instant::now() {
                    log::info!("Stabilized");
                    stabilized_at = None;

                    if let Some(since) = ocp_since {
                        system
                            .record
                            .log_overcurrent(since.elapsed().as_secs())
                            .await;
                        ocp_since = None;
                    }
                }
            }
        }

        let awaken_anyway_at = Instant::now() + MAX_DURATION;
        let deadline =
            earliest_deadline([Some(awaken_anyway_at), backoff_until, stabilized_at].into_iter())
                .unwrap();

        match embassy_futures::select::select(nint_pin.wait_for_falling_edge(), Timer::at(deadline))
            .await
        {
            embassy_futures::select::Either::First(_) => {}
            embassy_futures::select::Either::Second(_) => {}
        }
    }
}
