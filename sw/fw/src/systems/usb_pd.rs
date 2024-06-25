use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use embedded_io_async::{Read, Write};
use static_cell::StaticCell;

use crate::{
    bsp::{self, I2cBusDevice, I2cError},
    drivers::stusb4500::{ll::STUSB4500, Ctrl1OpCode},
};

pub struct USBPD {
    ll: Mutex<NoopRawMutex, STUSB4500<I2cBusDevice, I2cError>>,
}

// const NVM_DATA: [[u8; 8]; 5] = [
//     [0x00, 0x00, 0xB0, 0xAB, 0x00, 0x45, 0x00, 0x00],
//     [0x00, 0x40, 0x9C, 0x1C, 0xFF, 0x01, 0x3C, 0xDF],
//     [0x02, 0x40, 0x0F, 0x00, 0x32, 0x00, 0xFC, 0xF1],
//     [0x00, 0x19, 0x54, 0xAF, 0xFB, 0x35, 0x5F, 0x00],
//     [0x00, 0x5F, 0x90, 0x21, 0x43, 0x00, 0x50, 0xFB],
// ];

const NVM_DATA: [[u8; 8]; 5] = [
    [0x00, 0x00, 0xB0, 0xAB, 0x00, 0x45, 0x00, 0x00],
    [0x00, 0x40, 0x9C, 0x1C, 0xFF, 0x01, 0x3C, 0xDF],
    [0x02, 0x40, 0x0F, 0x00, 0x32, 0x00, 0xFC, 0xF1],
    [0x00, 0x19, 0x54, 0xAF, 0xFB, 0x35, 0x5F, 0x00],
    [0x00, 0x64, 0x90, 0x21, 0x43, 0x00, 0x50, 0xFB],
];

impl USBPD {
    pub async fn init(mut bsp: bsp::USBPD, spawner: &Spawner) -> &'static Self {
        let ll = Mutex::new(STUSB4500::new(bsp.i2c));

        bsp.reset_pin.set_low();

        Timer::after(Duration::from_millis(30)).await;

        static STATS: StaticCell<USBPD> = StaticCell::new();
        let stats = STATS.init(Self { ll });

        {
            let mut ll = stats.ll.lock().await;
            assert_eq!(ll.device_id().read_async().await.unwrap().value(), 0x25);

            // Unlock NVM
            Self::unlock_nvm(&mut ll).await.unwrap();

            let mut difference = false;
            for sector in 0..5 {
                let data = Self::read_sector(&mut ll, sector).await.unwrap();
                if NVM_DATA[sector as usize] != data {
                    log::warn!("Overwrite {}: {:?}", sector, data);
                    difference = true;
                }
            }

            if difference {
                Self::erase_sectors(&mut ll).await.unwrap();
                for sector in 0..5 {
                    Self::write_sector(&mut ll, sector, &NVM_DATA[sector as usize])
                        .await
                        .unwrap();
                }
            } else {
                log::info!("NVM already OK");
            }

            Self::lock_nvm(&mut ll).await.unwrap();
            Timer::after(Duration::from_millis(2000)).await;
            Self::issue_pd_reset(&mut ll).await.unwrap();

            // loop {
            //     ll.gpio_sw_gpio()
            //         .write_async(|w| w.gpio_sw_gpio(true))
            //         .await
            //         .unwrap();
            //     Timer::after(Duration::from_millis(1000)).await;
            //     ll.gpio_sw_gpio()
            //         .write_async(|w| w.gpio_sw_gpio(false))
            //         .await
            //         .unwrap();
            //     Timer::after(Duration::from_millis(1000)).await;
            // }
        }

        // spawner.must_spawn(task(bsp.nint_pin, stats));

        stats
    }

    async fn issue_pd_reset(ll: &mut STUSB4500<I2cBusDevice, I2cError>) -> Result<(), I2cError> {
        ll.tx_header().write_async(|w| w.tx_header(0x0D)).await?;
        ll.pd_command_ctrl()
            .write_async(|w| w.send_command(0x26))
            .await?;

        Ok(())
    }

    async fn unlock_nvm(ll: &mut STUSB4500<I2cBusDevice, I2cError>) -> Result<(), I2cError> {
        ll.nvm_password().write_async(|w| w.password(0x47)).await?;
        ll.nvm_ctrl_0().write_async(|w| w.value(0x00)).await?;
        ll.nvm_ctrl_0()
            .write_async(|w| w.power(true).enable(true))
            .await?;

        Ok(())
    }

    async fn lock_nvm(ll: &mut STUSB4500<I2cBusDevice, I2cError>) -> Result<(), I2cError> {
        ll.nvm_ctrl_0().write_async(|w| w.enable(true)).await?;
        ll.nvm_ctrl_1().write_async(|w| w.value(0x00)).await?;
        ll.nvm_password().write_async(|w| w.password(0x00)).await?;

        Ok(())
    }

    async fn issue_request_with_sector(
        ll: &mut STUSB4500<I2cBusDevice, I2cError>,
        sector: u8,
    ) -> Result<(), I2cError> {
        ll.nvm_ctrl_0()
            .write_async(|w| w.sector(sector).power(true).enable(true).request(true))
            .await?;

        // TODO timeout
        loop {
            if !ll.nvm_ctrl_0().read_async().await?.request() {
                break;
            }
        }

        Ok(())
    }

    async fn issue_request(ll: &mut STUSB4500<I2cBusDevice, I2cError>) -> Result<(), I2cError> {
        Self::issue_request_with_sector(ll, 0).await
    }

    async fn erase_sectors(ll: &mut STUSB4500<I2cBusDevice, I2cError>) -> Result<(), I2cError> {
        ll.nvm_ctrl_1()
            .write_async(|w| {
                w.op_code(Ctrl1OpCode::LoadSer)
                    .erase_sector_0(true)
                    .erase_sector_1(true)
                    .erase_sector_2(true)
                    .erase_sector_3(true)
                    .erase_sector_4(true)
            })
            .await?;
        Self::issue_request(ll).await?;
        ll.nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::EraseSectors))
            .await?;
        Self::issue_request(ll).await?;

        Ok(())
    }

    async fn read_sector(
        ll: &mut STUSB4500<I2cBusDevice, I2cError>,
        sector: u8,
    ) -> Result<[u8; 8], I2cError> {
        ll.nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::ReadSector))
            .await?;
        Self::issue_request_with_sector(ll, sector).await?;

        let mut buf = [0u8; 8];
        ll.rw_buffer().read_exact(&mut buf).await.unwrap(); // TODO error

        Ok(buf)
    }

    async fn write_sector(
        ll: &mut STUSB4500<I2cBusDevice, I2cError>,
        sector: u8,
        data: &[u8; 8],
    ) -> Result<(), I2cError> {
        ll.rw_buffer().write_all(data).await.unwrap(); // TODO error

        ll.nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::LoadPlr))
            .await?;
        Self::issue_request(ll).await?;

        ll.nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::WriteSector))
            .await?;

        Self::issue_request_with_sector(ll, sector).await?;

        Ok(())
    }
}

// #[embassy_executor::task]
// async fn task(mut nint_pin: bsp::PowerExtNIntPin, system: &'static USBPD) {
//     let mut enabled = false;
//     let mut backoff_until = None;

//     const BACKOFF_DURATION: Duration = Duration::from_millis(1000);
//     const MAX_DURATION: Duration = Duration::from_secs(5);

//     loop {
//         {
//             let mut ll = system.ll.lock().await;
//             let status = ll.status().read_async().await.unwrap();

//             log::info!("{:?}", status);

//             if status.ocp() || status.scp() {
//                 if enabled {
//                     log::error!("OCP!");

//                     ll.mode().modify_async(|w| w.oe(false)).await.unwrap();
//                     enabled = false;
//                     backoff_until = Some(Instant::now() + BACKOFF_DURATION);
//                 }
//             } else if !enabled {
//                 let activate = if let Some(until) = backoff_until {
//                     until < Instant::now()
//                 } else {
//                     true
//                 };

//                 if activate {
//                     log::info!("Enabling");

//                     ll.mode().modify_async(|w| w.oe(true)).await.unwrap();
//                     enabled = true;
//                     backoff_until = None;
//                 }
//             }
//         }

//         let awaken_anyway_at = Instant::now() + MAX_DURATION;
//         let deadline =
//             earliest_deadline([Some(awaken_anyway_at), backoff_until].into_iter()).unwrap();

//         match embassy_futures::select::select(nint_pin.wait_for_falling_edge(), Timer::at(deadline))
//             .await
//         {
//             embassy_futures::select::Either::First(_) => {}
//             embassy_futures::select::Either::Second(_) => {}
//         }
//     }
// }
