use num_enum::{IntoPrimitive, TryFromPrimitive};

pub mod ll;

#[repr(u8)]
#[derive(Clone, Copy, Debug, TryFromPrimitive, IntoPrimitive)]
pub enum Ctrl1OpCode {
    /// Read the sector data
    ReadSector = 0x00,
    /// Load the Program Load Register
    LoadPlr = 0x01,
    /// Load the Sector Erase Register
    LoadSer = 0x02,
    /// Dump the Program Load Register
    DumpPlr = 0x03,
    /// Dump the Sector Erase Register
    DumpSer = 0x04,
    /// Erase the specified sectors
    EraseSectors = 0x05,
    /// Program the sector data to EEPROM
    WriteSector = 0x06,
}
