use bitvec::prelude::*;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::Duration;
use esp_hal::prelude::*;
use static_cell::StaticCell;

use crate::bsp;

struct Inner {
    watchdog: bsp::Watchdog,
    mask: BitArray<[u16; 1]>,
    count: usize,
}

pub struct Watchdog(Mutex<CriticalSectionRawMutex, Inner>);

pub struct WatchdogTicket {
    parent: &'static Watchdog,
    index: usize,
}

const WATCHDOG_DURATION_SECS: u64 = 30;
const WATCHDOG_WIGGLE_SECS: u64 = 1;

/// If you feed the watchdog every deadline, you should be OK.
pub const WATCHDOG_DEADLINE: Duration =
    Duration::from_secs(WATCHDOG_DURATION_SECS - WATCHDOG_WIGGLE_SECS);

impl Watchdog {
    pub async fn init(mut watchdog: bsp::Watchdog) -> &'static Watchdog {
        watchdog.peri.enable();
        watchdog.peri.set_timeout(WATCHDOG_DURATION_SECS.secs());

        static SYSTEM: StaticCell<Watchdog> = StaticCell::new();
        SYSTEM.init(Self(Mutex::new(Inner {
            watchdog,
            mask: BitArray::default(),
            count: 0,
        })))
    }

    pub async fn ticket(&'static self) -> WatchdogTicket {
        let mut inner = self.0.lock().await;

        let res = WatchdogTicket {
            parent: self,
            index: inner.count,
        };

        inner.count += 1;

        // Maybe that the watchdog is about to expire just after creating this ticket.
        inner.feed(res.index).await;

        res
    }
}

impl Inner {
    async fn feed(&mut self, index: usize) {
        log::debug!("Ticket {} fed", index);

        self.mask.set(index, true);

        let slice = self.mask.as_mut_bitslice();
        let slice = &mut slice[0..self.count]; // Constrain to registered slice.

        if slice.all() {
            // Reset the mask.
            self.mask = BitArray::default();
            self.watchdog.peri.feed();

            log::debug!("All tickets fed, fed the watchdog");
        }
    }
}

impl WatchdogTicket {
    pub async fn feed(&self) {
        let mut inner = self.parent.0.lock().await;
        inner.feed(self.index).await;
    }
}
