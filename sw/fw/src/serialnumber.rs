pub struct SerialNumber([u8; 6]);

impl SerialNumber {
    pub fn fetch() -> Self {
        Self(esp_hal::efuse::Efuse::get_mac_address())
    }
}

impl core::fmt::Display for SerialNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut buf = [0u8; 6 * 2];

        // Note(unwrap): could only fail if buffer was too small.
        hex::encode_to_slice(&self.0, &mut buf).unwrap();

        f.write_str(core::str::from_utf8(&buf).unwrap())
    }
}
