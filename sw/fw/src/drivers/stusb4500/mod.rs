use bitfield::bitfield;
use num_enum::{IntoPrimitive, TryFromPrimitive};

pub mod hl;
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

#[repr(u8)]
#[derive(Clone, Copy, Debug, TryFromPrimitive, IntoPrimitive, PartialEq)]
pub enum PolicyEngineFSMState {
    Init = 0b00000000,
    SoftReset = 0b00000001,
    HardReset = 0b00000010,
    SendSoftReset = 0b00000011,
    CBist = 0b00000100,
    SnkStartup = 0b00010010,
    SnkDiscovery = 0b00010011,
    SnkWaitForCapabilities = 0b00010100,
    SnkEvaluateCapabilities = 0b00010101,
    SnkSelectCapabilities = 0b00010110,
    SnkTransitionSink = 0b00010111,
    SnkReady = 0b00011000,
    SnkReadySending = 0b00011001,
    HardResetShutdown = 0b00111010,
    HardResetRecovery = 0b00111011,
    Errorrecovery = 0b01000000,
}

bitfield! {
    pub struct FixedPdo(u32);
    impl Debug;

    pub fixed, _: 31, 30;
    pub dual_role_power, set_dual_role_power: 29;
    pub higher_capability, set_higher_capability: 28;
    pub unconstrained_power, set_unconstrained_power: 27;
    pub usb_communications_capable, set_usb_communications_capable: 26;
    pub dual_role_data, set_dual_role_data: 25;
    pub fast_role_swap, set_fast_role_swap: 24, 23;
    pub reserved, _: 22, 20;
    pub voltage, set_voltage: 19, 10;
    pub current, set_current: 9, 0;
}

impl FixedPdo {
    pub fn new(voltage: u16, current: u16) -> Self {
        let mut pdo: Self = Self(0);
        pdo.set_voltage(voltage as u32);
        pdo.set_current(current as u32);
        pdo
    }
}

#[derive(Debug)]
pub enum PdoChannel {
    PDO1,
    PDO2,
    PDO3,
}
