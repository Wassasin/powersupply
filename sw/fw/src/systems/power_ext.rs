use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{
    bsp::{self, I2cBusDevice, I2cError},
    drivers::tps55289::ll::Tps55289,
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

        spawner.must_spawn(task(bsp.nint_pin, stats));

        stats
    }

    pub async fn run_test(&self) -> Result<(), I2cError> {
        let mut ll = self.ll.lock().await;

        ll.vref().write_async(|w| w.vref(0b00110100100 * 2)).await?;
        ll.iout_limit().modify_async(|w| w.setting(0b111)).await?;
        ll.mode().modify_async(|w| w.dischg(false).oe(true)).await
    }
}

#[embassy_executor::task]
async fn task(nint_pin: bsp::PowerExtNIntPin, system: &'static PowerExt) {
    // nint_pin.wait_for_low()

    loop {
        {
            let mut ll = system.ll.lock().await;
            let status = ll.status().read_async().await.unwrap();
            log::info!("{:?}", status);
        }

        Timer::after(Duration::from_millis(1000)).await;
    }
}
