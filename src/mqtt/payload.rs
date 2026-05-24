use heapless::String;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocationPayload {
    pub lat: i32,       // degrees × 1_000_000
    pub lon: i32,       // degrees × 1_000_000
    pub timestamp: u32, // unix epoch seconds
    pub altitude: i16,  // metres
    pub speed: u16,     // cm/s
    pub heading: u16,   // degrees × 10
    pub hdop: u8,       // hdop × 10
    pub vdop: u8,       // vdop × 10
    pub secs_since_fix: u16,
    pub flags: u8, // bit 0 = is_moving
}

impl LocationPayload {
    pub fn to_base64(&self) -> String<50> {
        use base64ct::{Base64, Encoding};

        let bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(
                self as *const LocationPayload as *const u8,
                core::mem::size_of::<LocationPayload>(),
            )
        };

        let mut buf = [0u8; 50];
        let encoded = Base64::encode(bytes, &mut buf).unwrap();
        String::try_from(encoded).unwrap()
    }
}
