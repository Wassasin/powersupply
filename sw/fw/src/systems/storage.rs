use core::ops::Range;

use derive_more::From;
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use esp_partition_table::{DataPartitionType, PartitionEntry, PartitionTable, PartitionType};
use esp_storage::{FlashStorage, FlashStorageError};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use sequential_storage::{cache::NoCache, map::SerializationError};
use serde::{Deserialize, Serialize};
use static_cell::StaticCell;

const BUFFER_SIZE: usize = 128;
type Cache = NoCache;

pub struct Storage(Mutex<CriticalSectionRawMutex, Inner>);

struct Inner {
    storage: BlockingAsync<FlashStorage>,
    partition: PartitionEntry,
    cache: Cache,
}

#[derive(Serialize, Deserialize)]
struct Marker;

#[derive(Clone, Copy, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum StorageKey {
    Marker = 0x01,
    RecordData = 0x02,
}

pub trait StorageEntry: Serialize + for<'a> Deserialize<'a> {
    const KEY: StorageKey;
}

impl StorageEntry for Marker {
    const KEY: StorageKey = StorageKey::Marker;
}

impl sequential_storage::map::Key for StorageKey {
    fn serialize_into(&self, buffer: &mut [u8]) -> Result<usize, SerializationError> {
        if buffer.len() < 1 {
            return Err(SerializationError::BufferTooSmall);
        }

        let determinator = u8::from(*self);
        buffer[0] = determinator;
        Ok(1)
    }

    fn deserialize_from(buffer: &[u8]) -> Result<(Self, usize), SerializationError> {
        if buffer.len() < 1 {
            log::error!("{:?}", buffer);
            return Err(SerializationError::InvalidFormat);
        }

        let determinator = buffer[0];
        let key = StorageKey::try_from_primitive(determinator)
            .map_err(|_| SerializationError::InvalidData)?;
        Ok((key, 1))
    }
}

struct Wrapper<T: StorageEntry>(T);

impl<'a, T: StorageEntry> sequential_storage::map::Value<'a> for Wrapper<T> {
    fn serialize_into(&self, buffer: &mut [u8]) -> Result<usize, SerializationError> {
        let buffer =
            postcard::to_slice(&self.0, buffer).map_err(|_| SerializationError::BufferTooSmall)?;
        Ok(buffer.len())
    }

    fn deserialize_from(buffer: &'_ [u8]) -> Result<Self, SerializationError>
    where
        Self: Sized,
    {
        let inner = postcard::from_bytes(buffer).map_err(|_| SerializationError::InvalidData)?;
        Ok(Wrapper(inner))
    }
}

#[derive(From, Debug)]
pub enum Error {
    Serialization(SerializationError),
    Flash(sequential_storage::Error<FlashStorageError>),
}

fn range(p: &PartitionEntry) -> Range<u32> {
    p.offset..(p.offset + p.size as u32)
}

impl Inner {
    async fn ensure_initialized(&mut self) -> Result<(), Error> {
        match self.fetch::<Marker>().await {
            Ok(Some(Marker)) => {
                log::debug!("Marker detected");
                return Ok(());
            }
            Ok(None) => {
                log::warn!("No marker detected");
            }
            Err(e) => {
                log::error!("Failed to ensure storage: {:?}", e);
            }
        }

        log::debug!("Erasing storage");
        sequential_storage::erase_all(&mut self.storage, range(&self.partition)).await?;

        log::debug!("Storing marker");
        self.store(Marker).await?;

        log::info!("Storage initialized");

        Ok(())
    }

    pub async fn fetch<T: StorageEntry>(&mut self) -> Result<Option<T>, Error> {
        let mut buffer = [0u8; BUFFER_SIZE];
        let res: Result<Option<Wrapper<T>>, sequential_storage::Error<FlashStorageError>> =
            sequential_storage::map::fetch_item(
                &mut self.storage,
                range(&self.partition),
                &mut self.cache,
                &mut buffer,
                T::KEY,
            )
            .await;
        let res = res?;

        Ok(res.map(|x| x.0))
    }

    pub async fn store<T: StorageEntry>(&mut self, value: T) -> Result<(), Error> {
        let mut buffer = [0u8; BUFFER_SIZE];
        sequential_storage::map::store_item(
            &mut self.storage,
            range(&self.partition),
            &mut self.cache,
            &mut buffer,
            T::KEY,
            &Wrapper(value),
        )
        .await?;
        Ok(())
    }
}

impl Storage {
    pub async fn init() -> &'static Self {
        let partition_table = PartitionTable::default();
        let mut storage = FlashStorage::new();

        let mut found_nvs = None;
        log::info!("Scanning partition table");
        for entry in partition_table.iter_storage(&mut storage, true) {
            let entry = entry.unwrap();
            log::debug!("{:?}", entry);

            if entry.type_ == PartitionType::Data(DataPartitionType::Nvs) {
                found_nvs = Some(entry);
            }
        }

        let found_nvs = found_nvs.expect("No NVS partition found");

        log::info!(
            "Using partition \"{}\", offset {:#x}, size {:#x} bytes",
            found_nvs.name(),
            found_nvs.offset,
            found_nvs.size
        );

        let mut inner = Inner {
            storage: BlockingAsync::new(storage),
            partition: found_nvs,
            cache: Cache::new(),
        };

        inner.ensure_initialized().await.unwrap();

        static SYSTEM: StaticCell<Storage> = StaticCell::new();
        SYSTEM.init(Self(Mutex::new(inner)))
    }

    pub async fn store<T: StorageEntry>(&self, value: T) -> Result<(), Error> {
        let mut guard = self.0.lock().await;
        guard.store(value).await
    }

    pub async fn fetch<T: StorageEntry>(&self) -> Result<Option<T>, Error> {
        let mut guard = self.0.lock().await;
        guard.fetch().await
    }
}
