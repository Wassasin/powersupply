use derive_more::{Deref, From, Into};
use num_enum::{FromPrimitive, IntoPrimitive, TryFromPrimitive};

use crate::util::{Millivolts, Nanovolts};

pub mod ll;

#[repr(u8)]
#[derive(Clone, Copy, Debug, TryFromPrimitive, IntoPrimitive)]
pub enum IntFB {
    Ratio0_2256 = 0b00,
    Ratio0_1128 = 0b01,
    Ratio0_0752 = 0b10,
    Ratio0_0564 = 0b11,
}

impl IntFB {
    pub fn multiply(&self, x: u32) -> u32 {
        let ratio = match self {
            IntFB::Ratio0_2256 => 2_256,
            IntFB::Ratio0_1128 => 1_128,
            IntFB::Ratio0_0752 => 752,
            IntFB::Ratio0_0564 => 564,
        };
        x.checked_mul(ratio).unwrap() / 10_000
    }
}

#[repr(u8)]
#[derive(Debug, TryFromPrimitive, IntoPrimitive)]
pub enum OperatingStatus {
    Boost = 0b00,
    Buck = 0b01,
    BuckBoost = 0b10,
}

#[derive(Debug, Deref, From, Into)]
pub struct VRef(u16);

impl VRef {
    pub fn into_nanovolts(self) -> Nanovolts {
        Nanovolts(45_000_000 + 564_500 * self.0 as u32)
    }

    pub fn from_nanovolts(from: Nanovolts) -> Self {
        Self(((from.0 - 45_000_000) / 564_500) as u16)
    }

    pub fn from_feedback(target: Millivolts, fb: IntFB) -> Self {
        let millivolts = target.0 as u32;
        let millivolts = fb.multiply(millivolts);
        let nanovolts = Nanovolts(millivolts * 1_000_000);
        VRef::from_nanovolts(nanovolts)
    }
}
