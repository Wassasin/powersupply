use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, mutex::Mutex,
    pubsub::PubSubBehavior,
};
use embassy_time::{Duration, Instant, Timer};
use serde::{Deserialize, Serialize};
use static_cell::StaticCell;

use crate::{
    systems::storage::{Storage, StorageEntry, StorageKey},
    util::{DataChannel, DataSubscriber},
};

// Write records every once in a while to prevent flash wear.
const SYNC_PERIOD: Duration = Duration::from_secs(10);
const PUSH_PERIOD: Duration = Duration::from_secs(10);

type NotifyChannel = Channel<CriticalSectionRawMutex, (), 1>;

#[derive(PartialEq, Debug, Serialize, Deserialize, Default, Clone, Copy)]
pub struct Data {
    pub overcurrent_count: u64,
    pub overcurrent_secs: u64,
}

impl StorageEntry for Data {
    const KEY: StorageKey = StorageKey::RecordData;
}

struct Inner {
    data: Data,
    sync_at: Option<Instant>,
}

pub struct Record {
    inner: Mutex<CriticalSectionRawMutex, Inner>,
    storage: &'static Storage,
    sync_notifier: NotifyChannel,
    data_notifier: DataChannel<Data>,
}

impl Record {
    pub async fn init(storage: &'static Storage, spawner: &Spawner) -> &'static Self {
        let data = storage.fetch::<Data>().await.unwrap().unwrap_or_default();

        let statistics = Record {
            inner: Mutex::new(Inner {
                data,
                sync_at: None,
            }),
            storage,
            sync_notifier: NotifyChannel::new(),
            data_notifier: DataChannel::new(),
        };

        static SYSTEM: StaticCell<Record> = StaticCell::new();
        let system = SYSTEM.init(statistics);

        spawner.must_spawn(sync_task(system));
        spawner.must_spawn(push_task(system));

        system
    }

    pub async fn log_overcurrent(&self, duration_secs: u64) {
        let mut guard = self.inner.lock().await;
        guard.data.overcurrent_count += 1;
        guard.data.overcurrent_secs += duration_secs;
        self.schedule_sync(&mut guard).await;
    }

    async fn schedule_sync(&self, inner: &mut Inner) {
        self.data_notifier.publish_immediate(inner.data);

        if inner.sync_at.is_none() {
            inner.sync_at = Some(Instant::now() + SYNC_PERIOD);
            self.sync_notifier.send(()).await;
        }
    }

    pub fn subscriber(&'static self) -> DataSubscriber<Data> {
        self.data_notifier.subscriber().unwrap()
    }
}

#[embassy_executor::task]
async fn sync_task(system: &'static Record) {
    loop {
        system.sync_notifier.receive().await;

        let sync_at = {
            let guard = system.inner.lock().await;
            guard.sync_at.unwrap()
        };
        Timer::at(sync_at).await;

        let data = {
            let mut guard = system.inner.lock().await;
            guard.sync_at = None;
            guard.data.clone()
        };

        system.storage.store(data).await.unwrap();
        log::info!("Synced data");
    }
}

#[embassy_executor::task]
async fn push_task(system: &'static Record) {
    let mut subscriber = system.subscriber();

    loop {
        // Send the data every so often to consumers.
        match embassy_futures::select::select(subscriber.next_message(), Timer::after(PUSH_PERIOD))
            .await
        {
            embassy_futures::select::Either::First(_) => {}
            embassy_futures::select::Either::Second(_) => {
                let guard = system.inner.lock().await;
                system.data_notifier.publish_immediate(guard.data);
            }
        }
    }
}
