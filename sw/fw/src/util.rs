use core::fmt::Debug;
use serde::Serialize;

pub mod statsbuffer;

pub fn project_u8_f32(d: u8, max: u8, low: f32, high: f32) -> f32 {
    let range = high - low;
    d.clamp(0, max) as f32 / (max as f32) * range + low
}

pub fn project_f32_u8(d: f32, max: u8, low: f32, high: f32) -> u8 {
    let range = high - low;

    libm::roundf((d - low) / range * (max as f32)).clamp(0., max as f32) as u8
}

pub fn unitary_range(d: f32, low: f32, high: f32) -> f32 {
    let range = high - low;
    (d.clamp(low, high) - low) / range
}

pub fn unitary_range_u8(d: f32, low: f32, high: f32) -> u8 {
    (unitary_range(d, low, high) * 255.) as u8
}

#[derive(Serialize, Clone)]
pub struct Nanovolts(pub u32);

impl Debug for Nanovolts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{}nV", self.0))
    }
}

#[derive(Serialize, Clone)]
pub struct Millivolts(pub u16);

impl Debug for Millivolts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{}mV", self.0))
    }
}

impl From<Nanovolts> for Millivolts {
    fn from(value: Nanovolts) -> Self {
        Millivolts((value.0 / 1_000_000) as u16)
    }
}
