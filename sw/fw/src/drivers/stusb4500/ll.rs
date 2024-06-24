use bitvec::array::BitArray;
use device_driver::{AsyncRegisterDevice, Register};
use embedded_hal_async::i2c::I2c;

const MAX_TRANSACTION_SIZE: usize = 3;

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
            register vref {
                type RWType = RW;
                type ByteOrder = LE;
                const ADDRESS: u8 = 0x00;
                const SIZE_BITS: usize = 16;

                vref: u16 = 0..11,
            },
        }
    );
}
