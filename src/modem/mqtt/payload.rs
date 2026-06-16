use heapless::String;

use crate::modem::gnss::urc::fix::GnssLocation;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocationPayload {
    pub lat: i32,            // degrees × 1_000_000
    pub lon: i32,            // degrees × 1_000_000
    pub week_number: u16,      
    pub time_of_week: u32,
    pub altitude: i16,       // metres
    pub hdop: u8,            // hdop × 10
    pub flags: u8,           // bit 0 = is_moving
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

    pub fn from_gnss(loc: GnssLocation, is_moving: bool) -> Self {
        Self {
            lat: (loc.position.latitude * 1_000_000.0) as i32,
            lon: (loc.position.longitude * 1_000_000.0) as i32,
            week_number: loc.position.week_number,
            time_of_week: loc.position.time_of_week,
            altitude: loc.position.altitude as i16,
            hdop: (loc.accuracy.hdop * 10.0).clamp(0.0, 255.0) as u8,
            flags: if is_moving { 1 } else { 0 },
        }
    }
}


