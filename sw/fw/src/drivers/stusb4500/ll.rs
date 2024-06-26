use bitvec::array::BitArray;
use device_driver::{AsyncBufferDevice, AsyncRegisterDevice, Register};
use embedded_hal_async::i2c::I2c;

const MAX_TRANSACTION_SIZE: usize = 9;

const ADDRESS: u8 = 0x28;

use super::*;

pub struct STUSB4500<I2C: I2c<Error = E>, E> {
    i2c: I2C,
}

impl<I2C, E> AsyncRegisterDevice for STUSB4500<I2C, E>
where
    I2C: I2c<Error = E>,
{
    type Error = E;
    type AddressType = u8;

    async fn write_register<R, const SIZE_BYTES: usize>(
        &mut self,
        data: &BitArray<[u8; SIZE_BYTES]>,
    ) -> Result<(), Self::Error>
    where
        R: Register<SIZE_BYTES, AddressType = Self::AddressType>,
    {
        let data = data.as_raw_slice();

        let mut buf = [0u8; MAX_TRANSACTION_SIZE];
        buf[0] = R::ADDRESS;
        buf[1..data.len() + 1].copy_from_slice(data);
        let buf = &buf[0..data.len() + 1];

        self.i2c.write(ADDRESS, &buf).await
    }

    async fn read_register<R, const SIZE_BYTES: usize>(
        &mut self,
        data: &mut BitArray<[u8; SIZE_BYTES]>,
    ) -> Result<(), Self::Error>
    where
        R: Register<SIZE_BYTES, AddressType = Self::AddressType>,
    {
        self.i2c
            .write_read(ADDRESS, &[R::ADDRESS], data.as_raw_mut_slice())
            .await
    }
}

impl<I2C, E> AsyncBufferDevice for STUSB4500<I2C, E>
where
    I2C: I2c<Error = E>,
{
    async fn write(&mut self, id: u32, data: &[u8]) -> Result<usize, embedded_io::ErrorKind> {
        let mut buf = [0u8; MAX_TRANSACTION_SIZE];
        buf[0] = id as u8;
        buf[1..data.len() + 1].copy_from_slice(data);
        self.i2c
            .write(ADDRESS, &buf)
            .await
            .map_err(|_| embedded_io::ErrorKind::Other)?;

        Ok(data.len())
    }

    async fn read(&mut self, id: u32, buf: &mut [u8]) -> Result<usize, embedded_io::ErrorKind> {
        self.i2c
            .write_read(ADDRESS, &[id as u8], buf)
            .await
            .map_err(|_| embedded_io::ErrorKind::Other)?;

        Ok(buf.len())
    }
}

impl<I2C, E> STUSB4500<I2C, E>
where
    I2C: I2c<Error = E>,
{
    pub fn new(i2c: I2C) -> Self {
        Self { i2c }
    }

    pub fn take(self) -> I2C {
        self.i2c
    }
}

pub mod registers {
    use super::*;

    device_driver_macros::implement_device!(
        impl<I2C, E> STUSB4500<I2C, E> where
        I2C: I2c<Error = E>{
            register PDCommandCtrl {
                type RWType = RW;
                const ADDRESS: u8 = 0x1A;
                const SIZE_BITS: usize = 8;

                send_command: u8 = 0..6,
            },
            register PolicyEngineFSM {
                type RWType = R;
                const ADDRESS: u8 = 0x29;
                const SIZE_BITS: usize = 8;

                pe_fsm_state: u8 as PolicyEngineFSMState = 0..8,
            },
            register GPIOSwGPIO {
                type RWType = RW;
                const ADDRESS: u8 = 0x2D;
                const SIZE_BITS: usize = 8;

                gpio_sw_gpio: bool = 0,
            },
            register DeviceID {
                type RWType = R;
                const ADDRESS: u8 = 0x2F;
                const SIZE_BITS: usize = 8;

                value: u8 = 0..8,
            },
            register TXHeader {
                type RWType = RW;
                const ADDRESS: u8 = 0x51;
                const SIZE_BITS: usize = 8;

                tx_header: u16 = 0..8,
            },
            buffer RWBuffer: RW = 0x53,
            //DPM_PDO_NUMB
            register DPMPDONUMB {
                type RWType = RW;
                const ADDRESS: u8 = 0x70;
                const SIZE_BITS: usize = 8;

                number: u8 = 0..3,
            },
            register DPMSNKPDO1 {
                type RWType = RW;
                type ByteOrder = LE;
                const ADDRESS: u8 = 0x85;
                const SIZE_BITS: usize = 32;

                value: u32 = 0..32,
            },
            register DPMSNKPDO2 {
                type RWType = RW;
                type ByteOrder = LE;
                const ADDRESS: u8 = 0x89;
                const SIZE_BITS: usize = 32;

                value: u32 = 0..32,
            },
            register DPMSNKPDO3 {
                type RWType = RW;
                type ByteOrder = LE;
                const ADDRESS: u8 = 0x8D;
                const SIZE_BITS: usize = 32;

                value: u32 = 0..32,
            },
            register RDOStatus {
                type RWType = R;
                type ByteOrder = LE;
                const ADDRESS: u8 = 0x91;
                const SIZE_BITS: usize = 32;

                max_current: u16 = 0..10,
                current: u16 = 10..20,
                extended_supported: bool = 23,
                no_usb_suspend: bool = 24,
                usb_comms_capable: bool = 25,
                capability_mismatch: bool = 26,
                give_back: bool = 27,
                object_position: u8 = 28..32,
            },
            register NVMPassword {
                type RWType = RW;
                const ADDRESS: u8 = 0x95;
                const SIZE_BITS: usize = 8;

                password: u8 = 0..8,
            },
            register NVMCtrl0 {
                type RWType = RW;
                const ADDRESS: u8 = 0x96;
                const SIZE_BITS: usize = 8;

                sector: u8 = 0..4,
                request: bool = 4,
                enable: bool = 6,
                power: bool = 7,

                value: u8 = 0..8,
            },
            register NVMCtrl1 {
                type RWType = RW;
                const ADDRESS: u8 = 0x97;
                const SIZE_BITS: usize = 8;

                op_code: u8 as Ctrl1OpCode = 0..3,
                erase_sector0: bool = 3,
                erase_sector1: bool = 4,
                erase_sector2: bool = 5,
                erase_sector3: bool = 6,
                erase_sector4: bool = 7,

                value: u8 = 0..8,
            }
        }
    );
}
