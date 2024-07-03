//! Device configuration, persistent storage, and propagate changes to them.

use derive_builder::Builder;
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, pubsub::PubSubBehavior,
};
use embassy_time::{Duration, Timer};
use serde::{Deserialize, Serialize};
use static_cell::StaticCell;

use crate::{
    systems::storage::{Storage, StorageEntry, StorageKey},
    util::{Milliamps, Millivolts, PubSub, Sub},
};

const PUSH_PERIOD: Duration = Duration::from_secs(30);

#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, Copy, Builder)]
#[builder(no_std, build_fn(error(validation_error = false)))]
#[builder(derive(Deserialize))]
pub struct Settings {
    pub vout_mv: Millivolts,
    pub iout_ma: Milliamps,
    pub backoff_ms: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vout_mv: Millivolts(9000),
            iout_ma: Milliamps(500),
            backoff_ms: 500,
        }
    }
}

impl Settings {
    pub fn integrate(&mut self, value: SettingsBuilder) {
        if let Some(vout_mv) = value.vout_mv {
            self.vout_mv = vout_mv;
        }
        if let Some(iout_ma) = value.iout_ma {
            self.iout_ma = iout_ma;
        }
        if let Some(backoff_ms) = value.backoff_ms {
            self.backoff_ms = backoff_ms;
        }
    }
}

impl StorageEntry for Settings {
    const KEY: StorageKey = StorageKey::ConfigSettings;
}

struct Inner {
    settings: Settings,
}

pub struct Config {
    inner: Mutex<CriticalSectionRawMutex, Inner>,
    storage: &'static Storage,
    notifier: PubSub<Settings>,
}

impl Config {
    pub async fn init(storage: &'static Storage, spawner: &Spawner) -> &'static Self {
        let data = storage
            .fetch::<Settings>()
            .await
            .unwrap()
            .unwrap_or_default();

        let system = Config {
            inner: Mutex::new(Inner { settings: data }),
            storage,
            notifier: PubSub::new(),
        };

        static SYSTEM: StaticCell<Config> = StaticCell::new();
        let system = SYSTEM.init(system);

        spawner.must_spawn(push_task(system));

        system
    }

    pub async fn update(&self, f: impl FnOnce(Settings) -> Settings) {
        {
            let mut guard = self.inner.lock().await;
            guard.settings = f(guard.settings);
            self.storage.store(guard.settings).await.unwrap();

            let publisher = self.notifier.publisher().unwrap();
            publisher.publish(guard.settings).await; // Await until all consumers had their fill.
        };

        log::info!("Synced data");
    }

    pub async fn fetch(&self) -> Settings {
        let guard = self.inner.lock().await;
        guard.settings
    }

    /// Publish the current settings to all participants, immediately.
    pub async fn publish_immediate(&self) {
        let guard = self.inner.lock().await;
        self.notifier.publish_immediate(guard.settings);
    }

    pub fn subscriber(&'static self) -> Sub<Settings> {
        self.notifier.subscriber().unwrap()
    }
}

#[embassy_executor::task]
async fn push_task(system: &'static Config) {
    let mut subscriber = system.subscriber();
    loop {
        // Send the data every so often to consumers.
        match embassy_futures::select::select(subscriber.next_message(), Timer::after(PUSH_PERIOD))
            .await
        {
            embassy_futures::select::Either::First(_) => {}
            embassy_futures::select::Either::Second(_) => {
                let guard = system.inner.lock().await;
                system.notifier.publish_immediate(guard.settings); // Push immediate; non-critical.
            }
        }
    }
}
