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
