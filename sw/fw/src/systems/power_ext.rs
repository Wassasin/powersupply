//! Output power supply control.

use embassy_executor::SendSpawner;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, pubsub::WaitResult,
};
use embassy_time::{Duration, Instant, Timer};
use serde::Serialize;
use static_cell::StaticCell;

use crate::{
    bsp::{self, I2cBusDevice, I2cError},
    drivers::tps55289::{ll::Tps55289, IntFB, VRef},
    systems::{
        config::{Config, Settings},
        record::Record,
        usb_pd::Usbpd,
        watchdog::{self, Watchdog, WatchdogTicket},
    },
};

#[derive(Debug, PartialEq, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Disabled,
    Enabled,
    Enabling,
    Ocp,
}

struct Inner {
    ll: Tps55289<I2cBusDevice, I2cError>,
    backoff_duration: Duration,
    state: State,
}

pub struct PowerExt {
    inner: Mutex<CriticalSectionRawMutex, Inner>,
    usbpd: &'static Usbpd,
    record: &'static Record,
    watchdog: WatchdogTicket,
}

const FEEDBACK: IntFB = IntFB::Ratio0_0564;

impl PowerExt {
    pub async fn init(
        mut bsp: bsp::PowerExt,
        usbpd: &'static Usbpd,
        record: &'static Record,
        config: &'static Config,
        watchdog: &'static Watchdog,
        spawner: &SendSpawner,
    ) -> &'static Self {
        bsp.enable_pin.set_high();
        Timer::after(Duration::from_millis(50)).await;

        // Configure device in idle mode with correct feedback.
        let mut ll = Tps55289::new(bsp.i2c);
        ll.mode()
            .modify_async(|w| w.dischg(true).oe(false))
            .await
            .unwrap();
        ll.vout_fs()
            .modify_async(|w| w.intfb(FEEDBACK))
            .await
            .unwrap();

        static SYSTEM: StaticCell<PowerExt> = StaticCell::new();
        let system = SYSTEM.init(Self {
            inner: Mutex::new(Inner {
                ll,
                backoff_duration: Duration::default(), // placeholder value until persist
                state: State::Disabled,
            }),
            usbpd,
            record,
            watchdog: watchdog.ticket().await,
        });

        system.persist(config.fetch().await).await;

        spawner.must_spawn(monitor_task(bsp.nint_pin, system));
        spawner.must_spawn(config_task(config, system));

        system
    }

    /// Persist configuration settings.
    async fn persist(&self, settings: Settings) {
        let vref = VRef::from_feedback(settings.vout_mv, FEEDBACK);

        const CURRENT_SENSE_MILLIOHM: u32 = 20;
        let limit_uv = settings.iout_ma.0 as u32 * CURRENT_SENSE_MILLIOHM;
        let limit_value = (limit_uv / 500) as u8;

        let mut guard = self.inner.lock().await;

        guard.backoff_duration = Duration::from_millis(settings.backoff_ms as u64);

        // TODO check with internal settings to prevent too many I2C transations.
        let ll = &mut guard.ll;
        ll.vref().write_async(|w| w.vref(vref)).await.unwrap();
        ll.iout_limit()
            .modify_async(|w| w.setting(limit_value))
            .await
            .unwrap();

        log::info!("Persisted {:?} {:?}", settings.vout_mv, settings.iout_ma);
    }

    pub async fn state(&self) -> State {
        let guard = self.inner.lock().await;
        guard.state
    }
}

fn earliest_deadline(iter: impl Iterator<Item = Option<Instant>>) -> Option<Instant> {
    let mut res = None;

    for d in iter.flatten() {
        if let Some(res) = res.as_mut() {
            if *res > d {
                *res = d;
            }
        } else {
            res = Some(d);
        }
    }

    res
}

#[embassy_executor::task]
async fn config_task(config: &'static Config, system: &'static PowerExt) {
    let mut subscriber = config.subscriber();
    loop {
        if let WaitResult::Message(settings) = subscriber.next_message().await {
            system.persist(settings).await;
        }
    }
}

#[embassy_executor::task]
async fn monitor_task(mut nint_pin: bsp::PowerExtNIntPin, system: &'static PowerExt) {
    let mut enabled = false;
    let mut backoff_until = None;
    let mut stabilized_at = None;
    let mut ocp_since = None;

    const STABILIZATION_DURATION: Duration = Duration::from_millis(100);
    const MAX_DURATION: Duration = watchdog::WATCHDOG_DEADLINE;

    loop {
        {
            let mut inner = system.inner.lock().await;
            let status = inner.ll.status().read_async().await.unwrap();

            system.watchdog.feed().await;

            log::debug!("{:?}", status);

            if status.ocp() || status.scp() {
                inner.state = State::Ocp;
                log::error!("OCP!");

                inner
                    .ll
                    .mode()
                    .modify_async(|w| w.dischg(true).oe(false))
                    .await
                    .unwrap();

                enabled = false;
                backoff_until = Some(Instant::now() + inner.backoff_duration);
                stabilized_at = None;

                if ocp_since.is_none() {
                    ocp_since = Some(Instant::now());
                }

                system.usbpd.set_pin(false).await;
            } else if !enabled {
                let activate = if let Some(until) = backoff_until {
                    until < Instant::now()
                } else {
                    true
                };

                if activate {
                    inner.state = State::Enabling;
                    log::info!("Enabling");

                    inner
                        .ll
                        .mode()
                        .modify_async(|w| w.dischg(false).oe(true))
                        .await
                        .unwrap();
                    enabled = true;
                    backoff_until = None;

                    system.usbpd.set_pin(true).await;

                    stabilized_at = Some(Instant::now() + STABILIZATION_DURATION);
                }
            } else if let Some(at) = stabilized_at {
                if at < Instant::now() {
                    inner.state = State::Enabled;
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
