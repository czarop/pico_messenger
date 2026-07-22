//! MQTT location payload.
//!
//! `AT#MQTTPUB` caps the `<message>` field at 50 UTF-8 chars (verified against
//! the ST87MXX AT command manual, §11.4). Raw binary cannot be sent through a
//! UTF-8 field, so the packed struct is base64-encoded. 26 bytes -> 36 base64
//! chars (padded), comfortably within the 50-char limit (the ceiling is 36
//! bytes: 37 would encode to 52 chars and overflow).
//!
//! # Wire format (little-endian, fixed 26 bytes)
//!
//! Broker-side decoder reads exactly these fields, in this order, LE:
//!
//! | offset | field        | type | encoding                         |
//! |--------|--------------|------|----------------------------------|
//! | 0      | lat          | i32  | degrees x 1_000_000              |
//! | 4      | lon          | i32  | degrees x 1_000_000              |
//! | 8      | week_number  | u16  | GPS week number                  |
//! | 10     | time_of_week | u32  | ms within GPS week               |
//! | 14     | altitude     | i16  | metres                           |
//! | 16     | speed        | u16  | cm/s (valid iff flags bit1)      |
//! | 18     | heading      | u16  | decidegrees 0..3599 (bit2)       |
//! | 20     | hdop         | u8   | hdop x 10                        |
//! | 21     | vdop         | u8   | vdop x 10 (derived, see below)   |
//! | 22     | battery_soc  | u8   | percent 0..100 (bit4)            |
//! | 23     | temperature  | i16  | degC x 10 (bit5)                 |
//! | 25     | flags        | u8   | bit0 is_moving, bit1 speed_valid,|
//! |        |              |      | bit2 heading_valid, bit3 charging|
//! |        |              |      | bit4 battery_valid, bit5 temp_ok |
//!
//! `vdop` is NOT reported by the modem in AT format; it is derived as
//! `sqrt(pdop^2 - hdop^2)` (the geometric decomposition of 3D DOP into its
//! horizontal and vertical components). Flagged as derived, not measured.
//!
//! `speed` is computed upstream from two consecutive GNSS fixes; `heading`
//! comes from the BNO085 IMU. Either may be unavailable, in which case its
//! field is 0 and the corresponding validity bit is clear. A clear validity
//! bit means "no data", which is distinct from a zero value.

use heapless::String;

use crate::modem::gnss::urc::fix::GnssLocation;

/// Fixed on-the-wire length of the packed payload, in bytes.
pub const WIRE_LEN: usize = 26;

/// base64 of [`WIRE_LEN`] bytes (padded): ceil(26 / 3) * 4 = 36 chars.
pub const BASE64_LEN: usize = 36;

/// Flag bit: asset is moving.
pub const FLAG_IS_MOVING: u8 = 0b0000_0001;
/// Flag bit: `speed` field holds valid data.
pub const FLAG_SPEED_VALID: u8 = 0b0000_0010;
/// Flag bit: `heading` field holds valid data.
pub const FLAG_HEADING_VALID: u8 = 0b0000_0100;
/// Flag bit: battery is charging. Only meaningful with `FLAG_BATTERY_VALID`.
pub const FLAG_CHARGING: u8 = 0b0000_1000;
/// Flag bit: `battery_soc` holds valid data.
pub const FLAG_BATTERY_VALID: u8 = 0b0001_0000;
/// Flag bit: `temperature` holds valid data.
pub const FLAG_TEMP_VALID: u8 = 0b0010_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocationPayload {
    /// Latitude, degrees x 1_000_000.
    pub lat: i32,
    /// Longitude, degrees x 1_000_000.
    pub lon: i32,
    /// GPS week number.
    pub week_number: u16,
    /// Milliseconds within the GPS week.
    pub time_of_week: u32,
    /// Altitude, metres.
    pub altitude: i16,
    /// Speed over ground, cm/s. Valid only if `FLAG_SPEED_VALID` is set.
    pub speed: u16,
    /// Heading, decidegrees (0..=3599). Valid only if `FLAG_HEADING_VALID`.
    pub heading: u16,
    /// Horizontal DOP x 10.
    pub hdop: u8,
    /// Vertical DOP x 10 (derived from pdop/hdop, not modem-reported).
    pub vdop: u8,
    /// Battery state of charge, percent. Valid only if `FLAG_BATTERY_VALID`.
    pub battery_soc: u8,
    /// Temperature, degC x 10. Valid only if `FLAG_TEMP_VALID`.
    pub temperature: i16,
    /// Bitfield: see `FLAG_*` constants.
    pub flags: u8,
}

impl LocationPayload {
    /// Build a payload from a GNSS fix plus externally-sourced speed/heading.
    ///
    /// * `speed_mps` — speed over ground in m/s, computed upstream from two
    ///   consecutive fixes. `None` if it could not be computed; the field is
    ///   then 0 and `FLAG_SPEED_VALID` is left clear.
    /// * `heading_deg` — heading in degrees from the BNO085. `None` if the IMU
    ///   reading is unavailable/uncalibrated; field is 0 and the valid bit clear.
    /// * `is_moving` — drives `FLAG_IS_MOVING`.
    /// * `battery` — `(percent, charging)` from the fuel gauge. `None` if the
    ///   meter could not be read; field is 0 and `FLAG_BATTERY_VALID` stays clear.
    /// * `temperature_c10` — degC x 10. `None` if unread; field 0, bit clear.
    ///
    /// A clear validity bit means "no data", which is deliberately distinct from
    /// a zero value — 0% battery and 0.0 degC are both legitimate readings.
    pub fn from_gnss(
        loc: GnssLocation,
        speed_mps: Option<f32>,
        heading_deg: Option<f32>,
        is_moving: bool,
        battery: Option<(u8, bool)>,
        temperature_c10: Option<i16>,
    ) -> Self {
        // vdop = sqrt(pdop^2 - hdop^2), guarded against pdop < hdop (which
        // would be a degenerate report). libm::sqrtf for accurate no_std math.
        let p = loc.accuracy.pdop;
        let h = loc.accuracy.hdop;
        let vdop_f = if p >= h { libm::sqrtf(p * p - h * h) } else { 0.0 };

        let mut flags: u8 = 0;
        if is_moving {
            flags |= FLAG_IS_MOVING;
        }

        let speed = match speed_mps {
            Some(s) if s.is_finite() && s >= 0.0 => {
                flags |= FLAG_SPEED_VALID;
                // cm/s, saturated to u16 range. Sub-cm truncation is negligible.
                (s * 100.0).clamp(0.0, u16::MAX as f32) as u16
            }
            _ => 0,
        };

        let heading = match heading_deg {
            Some(d) if d.is_finite() => {
                flags |= FLAG_HEADING_VALID;
                // decidegrees, wrapped into 0..=3599. rem_euclid handles
                // negative inputs (BNO may report -180..180).
                (((d * 10.0) as i32).rem_euclid(3600)) as u16
            }
            _ => 0,
        };

        let battery_soc = match battery {
            Some((soc, charging)) => {
                flags |= FLAG_BATTERY_VALID;
                if charging {
                    flags |= FLAG_CHARGING;
                }
                soc.min(100)
            }
            None => 0,
        };

        let temperature = match temperature_c10 {
            Some(t) => {
                flags |= FLAG_TEMP_VALID;
                t
            }
            None => 0,
        };

        Self {
            lat: (loc.position.latitude * 1_000_000.0) as i32,
            lon: (loc.position.longitude * 1_000_000.0) as i32,
            week_number: loc.position.week_number,
            time_of_week: loc.position.time_of_week,
            altitude: loc.position.altitude as i16,
            speed,
            heading,
            hdop: (loc.accuracy.hdop * 10.0).clamp(0.0, u8::MAX as f32) as u8,
            vdop: (vdop_f * 10.0).clamp(0.0, u8::MAX as f32) as u8,
            battery_soc,
            temperature,
            flags,
        }
    }

    /// Pack into the fixed little-endian wire layout. Explicit field-by-field
    /// serialization — no `repr(C)` transmute, so no padding bytes and no UB.
    pub fn to_le_bytes(&self) -> [u8; WIRE_LEN] {
        let mut b = [0u8; WIRE_LEN];
        b[0..4].copy_from_slice(&self.lat.to_le_bytes());
        b[4..8].copy_from_slice(&self.lon.to_le_bytes());
        b[8..10].copy_from_slice(&self.week_number.to_le_bytes());
        b[10..14].copy_from_slice(&self.time_of_week.to_le_bytes());
        b[14..16].copy_from_slice(&self.altitude.to_le_bytes());
        b[16..18].copy_from_slice(&self.speed.to_le_bytes());
        b[18..20].copy_from_slice(&self.heading.to_le_bytes());
        b[20] = self.hdop;
        b[21] = self.vdop;
        b[22] = self.battery_soc;
        b[23..25].copy_from_slice(&self.temperature.to_le_bytes());
        b[25] = self.flags;
        b
    }

    /// Encode the packed bytes as base64 for the `AT#MQTTPUB` message field.
    pub fn to_base64(&self) -> String<50> {
        use base64ct::{Base64, Encoding};

        let bytes = self.to_le_bytes();
        let mut buf = [0u8; 64];
        let encoded = Base64::encode(&bytes, &mut buf).unwrap();
        String::try_from(encoded).unwrap()
    }
}