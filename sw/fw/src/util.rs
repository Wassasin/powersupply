use core::fmt::Debug;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    pubsub::{PubSubChannel, Subscriber},
};
use serde::{Deserialize, Serialize};

// pub mod statsbuffer;

const DATA_CAP: usize = 1;
const DATA_SUBS: usize = 4;
const DATA_PUBS: usize = 1;

pub type PubSub<T> = PubSubChannel<CriticalSectionRawMutex, T, DATA_CAP, DATA_SUBS, DATA_PUBS>;
pub type Sub<T> = Subscriber<'static, CriticalSectionRawMutex, T, DATA_CAP, DATA_SUBS, DATA_PUBS>;

#[derive(Serialize, Clone)]
pub struct Nanovolts(pub u32);

impl Debug for Nanovolts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{}nV", self.0))
    }
}

#[derive(PartialEq, Serialize, Deserialize, Default, Clone, Copy)]
pub struct Millivolts(pub u16);

impl Debug for Millivolts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{}mV", self.0))
    }
}

impl From<Nanovolts> for Millivolts {
    fn from(value: Nanovolts) -> Self {
        Millivolts((value.0 / 1_000_000) as u16)
    }
}

#[derive(PartialEq, Serialize, Deserialize, Default, Clone, Copy)]
pub struct Milliamps(pub u16);

impl Debug for Milliamps {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{}mA", self.0))
    }
}
