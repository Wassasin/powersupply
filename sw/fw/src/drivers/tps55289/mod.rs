use derive_more::{From, Into};
use num_enum::{FromPrimitive, IntoPrimitive, TryFromPrimitive};

pub mod ll;

#[repr(u8)]
#[derive(Debug, TryFromPrimitive, IntoPrimitive)]
pub enum IntFB {
    Ratio0_2256 = 0b00,
    Ratio0_1128 = 0b01,
    Ratio0_0752 = 0b10,
    Ratio0_0564 = 0b11,
}
