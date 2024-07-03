//! Input high current and voltage power supply control.

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{
    bsp::{self, I2cBusDevice, I2cError},
    drivers::stusb4500::{hl::STUSB4500, FixedPdo},
};

pub struct USBPD {
    hl: Mutex<CriticalSectionRawMutex, STUSB4500<I2cBusDevice, I2cError>>,
}

const NVM_DATA: [[u8; 8]; 5] = [
    [0x00, 0x00, 0xB0, 0xAB, 0x00, 0x45, 0x00, 0x00],
    [0x00, 0x40, 0x9C, 0x1C, 0xFF, 0x01, 0x3C, 0xDF],
    [0x02, 0x40, 0x0F, 0x00, 0x32, 0x00, 0xFC, 0xF1],
    [0x00, 0x19, 0x50, 0xAF, 0xF5, 0x35, 0x5F, 0x00],
    [0x00, 0x4B, 0x90, 0x21, 0x43, 0x00, 0x40, 0xFB],
];

impl USBPD {
    pub async fn init(mut bsp: bsp::USBPD, spawner: &Spawner) -> &'static Self {
        // Note: do not reset the chip, because it de-asserts the power supply, which we need to communicate to the chip.
        bsp.reset_pin.set_low();
        let hl = STUSB4500::new(bsp.i2c).await.unwrap();
        let mut nvm = hl.unlock_nvm().await.unwrap();

        let data = nvm.read_sectors().await.unwrap();
        if data != NVM_DATA {
            nvm.write_sectors(&NVM_DATA).await.unwrap();
            log::warn!("NVM updated");
        } else {
            log::info!("NVM OK");
        }

        let mut hl = nvm.lock_nvm().await.unwrap();

        let pdo = FixedPdo::new(20000 / 50, 1000 / 10);
        hl.set_pdo(crate::drivers::stusb4500::PdoChannel::PDO2, pdo)
            .await
            .unwrap();

        hl.set_pdo_num(2).await.unwrap();
        hl.issue_pd_reset().await.unwrap();

        let hl = Mutex::new(hl);

        static USBPD: StaticCell<USBPD> = StaticCell::new();
        let system = USBPD.init(Self { hl });

        spawner.must_spawn(state_task(system));

        system
    }

    /// Set output GPIO pin value. (connected to LED indicating a short)
    pub async fn set_pin(&self, level: bool) {
        let mut hl = self.hl.lock().await;
        hl.gpio_set_level(level).await.unwrap();
    }
}

#[embassy_executor::task]
async fn state_task(system: &'static USBPD) {
    let mut prev_state = {
        let mut hl = system.hl.lock().await;
        hl.fsm_state().await.unwrap()
    };

    loop {
        let (state, rdo) = {
            let mut hl = system.hl.lock().await;
            (hl.fsm_state().await.unwrap(), hl.rdo().await.unwrap())
        };

        if prev_state != state {
            log::info!("{:?} => {:?}", prev_state, state);
            log::info!("{:?}", rdo);
            prev_state = state;
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}
