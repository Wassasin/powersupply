use core::mem::MaybeUninit;

use embassy_time::Timer;
use embedded_hal_async::i2c::I2c;
use embedded_io_async::{Read, Write};

use crate::drivers::stusb4500::{ll, Ctrl1OpCode};

use super::{ll::registers::rdo_status, FixedPdo, PdoChannel, PolicyEngineFSMState};

pub struct STUSB4500<I2C: I2c<Error = E>, E> {
    ll: ll::STUSB4500<I2C, E>,
}

pub struct STUSB4500Nvm<I2C: I2c<Error = E>, E>(STUSB4500<I2C, E>);

pub const NUM_SECTORS: usize = 5;

pub type NVMSector = [u8; 8];
pub type NVMSectors = [NVMSector; NUM_SECTORS];

#[derive(Debug)]
pub enum Error<E> {
    I2C(E),
    IO,
    DeviceIDMismatch,
    InvalidValue,
}

impl<E> From<E> for Error<E> {
    fn from(value: E) -> Self {
        Error::I2C(value)
    }
}

impl<I2C: I2c<Error = E>, E> STUSB4500<I2C, E> {
    pub async fn new(i2c: I2C) -> Result<Self, Error<E>> {
        let mut ll = ll::STUSB4500::new(i2c);

        while ll.device_id().read_async().await?.value() != 0x25 {
            // TODO timeout
            Timer::after_millis(1).await
        }

        Ok(Self { ll })
    }

    pub async fn unlock_nvm(mut self) -> Result<STUSB4500Nvm<I2C, E>, E> {
        self.ll
            .nvm_password()
            .write_async(|w| w.password(0x47))
            .await?;
        self.ll.nvm_ctrl_0().write_async(|w| w.value(0x00)).await?;
        self.ll
            .nvm_ctrl_0()
            .write_async(|w| w.power(true).enable(true))
            .await?;

        Ok(STUSB4500Nvm(self))
    }

    pub async fn issue_pd_reset(&mut self) -> Result<(), E> {
        self.ll
            .tx_header()
            .write_async(|w| w.tx_header(0x0D))
            .await?;
        self.ll
            .pd_command_ctrl()
            .write_async(|w| w.send_command(0x26))
            .await?;

        Ok(())
    }

    pub async fn fsm_state(&mut self) -> Result<PolicyEngineFSMState, Error<E>> {
        self.ll
            .policy_engine_fsm()
            .read_async()
            .await?
            .pe_fsm_state()
            .map_err(|_| Error::InvalidValue)
    }

    pub async fn gpio_set_level(&mut self, level: bool) -> Result<(), Error<E>> {
        Ok(self
            .ll
            .gpio_sw_gpio()
            .write_async(|w| w.gpio_sw_gpio(!level))
            .await?)
    }

    pub async fn rdo(&mut self) -> Result<rdo_status::R, E> {
        self.ll.rdo_status().read_async().await
    }

    pub async fn set_pdo(&mut self, channel: PdoChannel, pdo: FixedPdo) -> Result<(), E> {
        use PdoChannel::*;
        let bits = pdo.0;
        match channel {
            PDO1 => self.ll.dpmsnkpdo_1().write_async(|w| w.value(bits)).await,
            PDO2 => self.ll.dpmsnkpdo_2().write_async(|w| w.value(bits)).await,
            PDO3 => self.ll.dpmsnkpdo_3().write_async(|w| w.value(bits)).await,
        }
    }

    pub async fn set_pdo_num(&mut self, num: u8) -> Result<(), E> {
        self.ll.dpmpdonumb().modify_async(|w| w.number(num)).await
    }
}

impl<I2C: I2c<Error = E>, E> STUSB4500Nvm<I2C, E> {
    pub async fn lock_nvm(mut self) -> Result<STUSB4500<I2C, E>, E> {
        let ll = &mut self.0.ll;

        ll.nvm_ctrl_0().write_async(|w| w.enable(true)).await?;
        ll.nvm_ctrl_1().write_async(|w| w.value(0x00)).await?;
        ll.nvm_password().write_async(|w| w.password(0x00)).await?;

        Ok(self.0)
    }

    async fn issue_request_with_sector(&mut self, sector: u8) -> Result<(), E> {
        self.0
            .ll
            .nvm_ctrl_0()
            .write_async(|w| w.sector(sector).power(true).enable(true).request(true))
            .await?;

        // TODO timeout
        loop {
            if !self.0.ll.nvm_ctrl_0().read_async().await?.request() {
                break;
            }
        }

        Ok(())
    }

    async fn issue_request(&mut self) -> Result<(), E> {
        self.issue_request_with_sector(0).await
    }

    /// Erase all NVM sectors
    pub async fn erase_sectors(&mut self) -> Result<(), E> {
        self.0
            .ll
            .nvm_ctrl_1()
            .write_async(|w| {
                w.op_code(Ctrl1OpCode::LoadSer)
                    .erase_sector_0(true)
                    .erase_sector_1(true)
                    .erase_sector_2(true)
                    .erase_sector_3(true)
                    .erase_sector_4(true)
            })
            .await?;
        self.issue_request().await?;
        self.0
            .ll
            .nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::EraseSectors))
            .await?;
        self.issue_request().await?;

        Ok(())
    }

    async fn read_sector(&mut self, sector: u8) -> Result<NVMSector, Error<E>> {
        self.0
            .ll
            .nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::ReadSector))
            .await?;
        self.issue_request_with_sector(sector).await?;

        let mut buf = [0u8; 8];
        self.0
            .ll
            .rw_buffer()
            .read_exact(&mut buf)
            .await
            .map_err(|_| Error::IO)?;

        Ok(buf)
    }

    async fn write_sector(&mut self, sector: u8, data: &NVMSector) -> Result<(), Error<E>> {
        self.0
            .ll
            .rw_buffer()
            .write_all(data)
            .await
            .map_err(|_| Error::IO)?;

        self.0
            .ll
            .nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::LoadPlr))
            .await?;
        self.issue_request().await?;

        self.0
            .ll
            .nvm_ctrl_1()
            .write_async(|w| w.op_code(Ctrl1OpCode::WriteSector))
            .await?;

        self.issue_request_with_sector(sector).await?;

        Ok(())
    }

    pub async fn read_sectors(&mut self) -> Result<NVMSectors, Error<E>> {
        let mut res: [MaybeUninit<NVMSector>; NUM_SECTORS] =
            unsafe { [MaybeUninit::uninit().assume_init(); NUM_SECTORS] };

        for (i, sector) in res.iter_mut().enumerate() {
            sector.write(self.read_sector(i as u8).await?);
        }

        Ok(unsafe {
            core::mem::transmute::<[MaybeUninit<NVMSector>; NUM_SECTORS], [NVMSector; NUM_SECTORS]>(
                res,
            )
        })
    }

    pub async fn write_sectors(&mut self, buf: &NVMSectors) -> Result<(), Error<E>> {
        self.erase_sectors().await?;

        for (i, sector) in buf.iter().enumerate() {
            self.write_sector(i as u8, sector).await?;
        }
        Ok(())
    }
}
