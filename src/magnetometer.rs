use embedded_hal_async::i2c::I2c;
use thiserror::Error;

const BMM150_ADDR: u8 = 0x10;
const OPERATION_MODE_REGISTER: u8 = 0x4C;
const POWER_MODE_REGISTER: u8 = 0x4B;
const READ_REGISTER: u8 = 0x48;

enum PowerMode{
    Suspend,
    Sleep,
}

impl PowerMode {
    fn command(&self) -> [u8; 2] {
        match self {
            PowerMode::Suspend => [POWER_MODE_REGISTER, 0x00],
            PowerMode::Sleep => [POWER_MODE_REGISTER, 0x01],
        }
    }

    fn is_sleep(&self) -> bool {
        match self {
            PowerMode::Suspend => false,
            PowerMode::Sleep => true,
        }
    }
}

enum OperationMode {
    Sleep,
    Forced,
    Normal
}

impl OperationMode {
    fn command(&self, current_reg: u8) -> [u8; 2] {
        let opmode_bits = match self {
            OperationMode::Sleep  => 0b00000110,
            OperationMode::Forced => 0b00000010,
            OperationMode::Normal => 0b00000000,
        };
        // clear bits 2:1, set new opmode
        let new_reg = (current_reg & !0x06) | opmode_bits;
        [OPERATION_MODE_REGISTER, new_reg]
    }

    // assumes default register state - will overwrite settings if not
    fn command_default_reg(&self) -> [u8; 2] {
        match self {
            OperationMode::Sleep => [OPERATION_MODE_REGISTER, 0x06],
            OperationMode::Forced => [OPERATION_MODE_REGISTER, 0x02],
            OperationMode::Normal => [OPERATION_MODE_REGISTER, 0x00],
        }
    }

}

enum DataRate {
    Hz10,  // 0x00 default
    Hz2,   // 0x08
    Hz6,   // 0x10
    Hz8,   // 0x18
    Hz15,  // 0x20
    Hz20,  // 0x28
    Hz25,  // 0x30
    Hz30,  // 0x38
}

impl DataRate {
    fn bits(&self) -> u8 {
        match self {
            DataRate::Hz10 => 0x00,
            DataRate::Hz2  => 0x08,
            DataRate::Hz6  => 0x10,
            DataRate::Hz8  => 0x18,
            DataRate::Hz15 => 0x20,
            DataRate::Hz20 => 0x28,
            DataRate::Hz25 => 0x30,
            DataRate::Hz30 => 0x38,
        }
    }
    fn apply(&self, current_reg: u8) -> [u8; 2] {
        let new_reg = (current_reg & !0x38) | self.bits();
        [OPERATION_MODE_REGISTER, new_reg]
    }
}

pub struct Magnetometer<I> {
    i2c: I,
    recv_buffer: [u8; 8],
    power_state: PowerMode,
}

impl<I> Magnetometer<I>
where
    I: I2c,
{
    pub fn new(i2c: I) -> Self {

        Self {
            i2c,
            recv_buffer: [0u8; 8],
            power_state: PowerMode::Suspend
        }
    }

    pub async fn read_direction(
        &mut self,
    ) -> Result<MagnetometerReading, MagnetometerError<I::Error>> {
        if !self.power_state.is_sleep(){
            self.i2c.write(BMM150_ADDR, &PowerMode::Sleep.command()).await?;
            embassy_time::Timer::after(embassy_time::Duration::from_millis(3))
            .await;
            self.power_state = PowerMode::Sleep;
        }

        // default singular read
        self.i2c.write(BMM150_ADDR, &OperationMode::Forced.command_default_reg()).await?;

        // wait for data to be ready
        loop{
            embassy_time::Timer::after(embassy_time::Duration::from_millis(1))
            .await;
            self.i2c.write_read(BMM150_ADDR, &[0x42], &mut self.recv_buffer).await?;
            if self.recv_buffer[6] & 0x01 == 1 {
                break
            }
        }
        
        
        
        
        



        Ok()
    }
}

#[derive(Debug, Error)]
pub enum MagnetometerError<E: core::fmt::Debug> {
    #[error("I2C error: {0:?}")]
    I2cError(E),
    #[error("Invalid sensor data: {0}")]
    InvalidData(&'static str),

    #[error("Data not ready")]
    StaleData

}

impl<E: core::fmt::Debug> From<E> for MagnetometerError<E> {
    fn from(e: E) -> Self {
        MagnetometerError::I2cError(e)
    }
}

#[allow(dead_code)]
struct RawReading {
    x_lsb: u8,
    x_msb: u8,
    y_lsb: u8,
    y_msb: u8,
    z_lsb: u8,
    z_msb: u8,
    rhall_lsb_drdy: u8,
    rhall_msb: u8

}

impl From<[u8; 8]> for RawReading {
    fn from(buf: [u8; 8]) -> Self {
        Self {
            x_lsb: buf[0],
            x_msb: buf[1],
            y_lsb: buf[2],
            y_msb: buf[3],
            z_lsb: buf[4],
            z_msb: buf[5],
            rhall_lsb_drdy: buf[6],
            rhall_msb: buf[7]
        }
    }
}

impl RawReading {
    fn is_ready(&self) -> bool {
        self.rhall_lsb_drdy & 0x01 == 1
    }

    fn rhall_lsb(&self) -> u8 {
        // mask out DRDY bit 0 and fixed bit 1
        self.rhall_lsb_drdy & 0b11111100
    }

    fn x(&self) -> i16 {
        // 13-bit two's complement, LSB in bits 7:3 of x_lsb, MSB in x_msb
        let raw = ((self.x_msb as i16) << 5) | ((self.x_lsb as i16) >> 3);
        // sign extend from 13 bits
        (raw << 3) >> 3
    }

    fn y(&self) -> i16 {
        let raw = ((self.y_msb as i16) << 5) | ((self.y_lsb as i16) >> 3);
        (raw << 3) >> 3
    }

    fn z(&self) -> i16 {
        // 15-bit two's complement, LSB in bits 7:1 of z_lsb, MSB in z_msb
        let raw = ((self.z_msb as i16) << 7) | ((self.z_lsb as i16) >> 1);
        // sign extend from 15 bits
        (raw << 1) >> 1
    }

    fn rhall(&self) -> u16 {
        // 14-bit unsigned, LSB in bits 7:2 of rhall_lsb, MSB in rhall_msb
        ((self.rhall_msb as u16) << 6) | ((self.rhall_lsb_drdy as u16) >> 2)
    }
}

pub struct MagnetometerReading {
    pub x: f32,
    pub y: f32,
    pub z: f32
}

impl TryFrom<RawReading> for MagnetometerReading {
    type Error = MagnetometerError<core::convert::Infallible>;

    fn try_from(raw: RawReading) -> Result<Self, Self::Error> {
        if !raw.is_ready() {
            return Err(MagnetometerError::StaleData);
        } else {
            Ok(Self{
                x: raw.x(),
                y: raw.y(),
                z: raw.z(),
            })
        }
        // convert...
    }
}


use crate::types::MagneticFieldData;
const MAG_OVERFLOW_XY: i16 = -4096;
const MAG_OVERFLOW_OUTPUT: i16 = -32768;
const MAG_OVERFLOW_ADCVAL_ZAXIS_HALL: i16 = -16384;
const MAG_NEGATIVE_SATURATION_Z: i32 = -32767;
const MAG_POSITIVE_SATURATION_Z: i32 = 32767;

impl MagneticFieldData {
    /// Compensated X in micro tesla
    #[inline]
    pub fn x_compensated_ut(&self) -> i16 {
        let mut retval: i16;
        let process_comp_x0: u16;
        let process_comp_x1: i32;
        let process_comp_x2: u16;
        let process_comp_x3: i32;
        let process_comp_x4: i32;
        let process_comp_x5: i32;
        let process_comp_x6: i32;
        let process_comp_x7: i32;
        let process_comp_x8: i32;
        let process_comp_x9: i32;
        let process_comp_x10: i32;

        let mag_data_x = self.x_unscaled();
        let data_rhall = self.rhall_unscaled();
        /* Overflow condition check */
        if mag_data_x != MAG_OVERFLOW_XY {
            if data_rhall != 0 {
                /* Availability of valid data */
                process_comp_x0 = data_rhall;
            } else if self.trim_data.dig_xyz1 != 0 {
                process_comp_x0 = self.trim_data.dig_xyz1;
            } else {
                process_comp_x0 = 0;
            }

            if process_comp_x0 != 0 {
                /* Processing compensation equations */
                process_comp_x1 = i32::from(self.trim_data.dig_xyz1) * 16384;
                process_comp_x2 = u16::try_from(process_comp_x1 / i32::from(process_comp_x0))
                    .unwrap()
                    .wrapping_sub(0x4000);
                let tmp_val: i16 = process_comp_x2 as i16;
                process_comp_x3 = i32::from(tmp_val) * i32::from(tmp_val);
                process_comp_x4 = i32::from(self.trim_data.dig_xy2) * (process_comp_x3 / 128);
                process_comp_x5 = i32::from(i16::from(self.trim_data.dig_xy1) * 128);
                process_comp_x6 = i32::from(tmp_val) * process_comp_x5;
                process_comp_x7 = ((process_comp_x4 + process_comp_x6) / 512) + 0x100000;
                process_comp_x8 = i32::from(i16::from(self.trim_data.dig_x2) + 0xA0);
                process_comp_x9 = (process_comp_x7 * process_comp_x8) / 4096;
                process_comp_x10 = i32::from(mag_data_x) * process_comp_x9;
                retval = i16::try_from(process_comp_x10 / 8192).unwrap();
                retval = (retval + (i16::from(self.trim_data.dig_x1) * 8)) / 16;
            } else {
                retval = MAG_OVERFLOW_OUTPUT;
            }
        } else {
            /* Overflow condition */
            retval = MAG_OVERFLOW_OUTPUT;
        }

        retval
    }

    /// Compensated Y in micro tesla
    #[inline]
    pub fn y_compensated_ut(&self) -> i16 {
        let mut retval: i16;
        let process_comp_y0: u16;
        let process_comp_y1: i32;
        let process_comp_y2: u16;
        let process_comp_y3: i32;
        let process_comp_y4: i32;
        let process_comp_y5: i32;
        let process_comp_y6: i32;
        let process_comp_y7: i32;
        let process_comp_y8: i32;
        let process_comp_y9: i32;

        let mag_data_y = self.y_unscaled();
        let data_rhall = self.rhall_unscaled();
        /* Overflow condition check */
        if mag_data_y != MAG_OVERFLOW_XY {
            if data_rhall != 0 {
                /* Availability of valid data */
                process_comp_y0 = data_rhall;
            } else if self.trim_data.dig_xyz1 != 0 {
                process_comp_y0 = self.trim_data.dig_xyz1;
            } else {
                process_comp_y0 = 0;
            }

            if process_comp_y0 != 0 {
                /* Processing compensation equations */
                process_comp_y1 =
                    ((i32::from(self.trim_data.dig_xyz1)) * 16384) / i32::from(process_comp_y0);
                process_comp_y2 = u16::try_from(process_comp_y1).unwrap().wrapping_sub(0x4000);
                let tmp_val: i16 = process_comp_y2 as i16;
                process_comp_y3 = i32::from(tmp_val) * i32::from(tmp_val);
                process_comp_y4 = (i32::from(self.trim_data.dig_xy2)) * (process_comp_y3 / 128);
                process_comp_y5 = i32::from(i16::from(self.trim_data.dig_xy1) * 128);
                process_comp_y6 = (process_comp_y4 + (i32::from(tmp_val) * process_comp_y5)) / 512;
                process_comp_y7 = i32::from((i16::from(self.trim_data.dig_y2)) + 0xA0);
                process_comp_y8 = ((process_comp_y6 + 0x100000) * process_comp_y7) / 4096;
                process_comp_y9 = i32::from(mag_data_y) * process_comp_y8;
                retval = i16::try_from(process_comp_y9 / 8192).unwrap();
                retval = (retval + (i16::from(self.trim_data.dig_y1) * 8)) / 16;
            } else {
                retval = MAG_OVERFLOW_OUTPUT;
            }
        } else {
            /* Overflow condition */
            retval = MAG_OVERFLOW_OUTPUT;
        }
        retval
    }

    /// Compensated Z in micro tesla
    #[inline]
    pub fn z_compensated_ut(&self) -> i16 {
        let mut retval: i32;
        let process_comp_z0: i16;
        let process_comp_z1: i32;
        let process_comp_z2: i32;
        let process_comp_z3: i32;
        let process_comp_z4: i16;

        let mag_data_z = self.z_unscaled();
        let data_rhall = self.rhall_unscaled();
        if mag_data_z != MAG_OVERFLOW_ADCVAL_ZAXIS_HALL {
            if (self.trim_data.dig_z2 != 0)
                && (self.trim_data.dig_z1 != 0)
                && (data_rhall != 0)
                && (self.trim_data.dig_xyz1 != 0)
            {
                /*Processing compensation equations */
                process_comp_z0 = i16::try_from(data_rhall).unwrap()
                    - i16::try_from(self.trim_data.dig_xyz1).unwrap();
                process_comp_z1 =
                    (i32::from(self.trim_data.dig_z3) * (i32::from(process_comp_z0))) / 4;
                process_comp_z2 = (i32::from(mag_data_z - self.trim_data.dig_z4)) * 32768;
                process_comp_z3 = (i32::from(self.trim_data.dig_z1)) * (i32::from(data_rhall) * 2);
                process_comp_z4 = i16::try_from((process_comp_z3 + (32768)) / 65536).unwrap();
                retval = (process_comp_z2 - process_comp_z1)
                    / (i32::from(self.trim_data.dig_z2) + i32::from(process_comp_z4));

                /* Saturate result to +/- 2 micro-tesla */
                if retval > MAG_POSITIVE_SATURATION_Z {
                    retval = MAG_POSITIVE_SATURATION_Z;
                } else if retval < MAG_NEGATIVE_SATURATION_Z {
                    retval = MAG_NEGATIVE_SATURATION_Z;
                }

                /* Conversion of LSB to micro-tesla */
                retval /= 16;
            } else {
                retval = i32::from(MAG_OVERFLOW_OUTPUT);
            }
        } else {
            /* Overflow condition */
            retval = i32::from(MAG_OVERFLOW_OUTPUT);
        }

        retval.try_into().unwrap()
    }
}
