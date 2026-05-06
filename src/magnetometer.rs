use embedded_hal_async::i2c::I2c;
use thiserror::Error;

const BMM150_ADDR: u8 = 0x10;
const OPERATION_MODE_REGISTER: u8 = 0x4C;
const POWER_MODE_REGISTER: u8 = 0x4B;
const READ_REGISTER: u8 = 0x48;
const MAG_OVERFLOW_XY: i16 = -4096;
const MAG_OVERFLOW_OUTPUT: i16 = -32768;
const MAG_OVERFLOW_ADCVAL_ZAXIS_HALL: i16 = -16384;
const MAG_NEGATIVE_SATURATION_Z: i32 = -32767;
const MAG_POSITIVE_SATURATION_Z: i32 = 32767;

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
    trim_data: Option<TrimData>
}

impl<I> Magnetometer<I>
where
    I: I2c,
{
    pub fn new(i2c: I) -> Self {
        Self {
            i2c,
            recv_buffer: [0u8; 8],
            power_state: PowerMode::Suspend,
            trim_data: None
        }
    }

    pub async fn read_direction(
        &mut self,
    ) -> Result<MagnetometerReading, MagnetometerError<I::Error>> {
        if !self.power_state.is_sleep(){
            self.i2c.write(BMM150_ADDR, &PowerMode::Sleep.command()).await?;
            embassy_time::Timer::after(embassy_time::Duration::from_millis(3))
            .await;

            if self.trim_data.is_none() {

                let mut buf2  = [0u8; 2];
                let mut buf4  = [0u8; 4];
                let mut buf10 = [0u8; 10];

                self.i2c.write_read(BMM150_ADDR, &[0x5D], &mut buf2).await?;
                self.i2c.write_read(BMM150_ADDR, &[0x62], &mut buf4).await?;
                self.i2c.write_read(BMM150_ADDR, &[0x68], &mut buf10).await?;

                let trim_data = TrimData::from((
                    TrimX1Y1::from(buf2),
                    TrimXYZ::from(buf4),
                    TrimXY1XY2::from(buf10),
                ));
                self.trim_data = Some(trim_data);
            }
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
        
        let raw: RawReading = self.recv_buffer.into();

        if !raw.is_ready(){
            return Err(MagnetometerError::StaleData);
        }

        if let Some(trim) = &self.trim_data {
            MagnetometerReading::from_raw(&raw, trim)
        } else {
            Err(MagnetometerError::InvalidData("Data could not be converted from raw"))
        }
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

struct RawReading {
    x: u16,
    y: u16,
    z: u16,
    rhall: u16,
}

impl From<[u8; 8]> for RawReading {
    fn from(buf: [u8; 8]) -> Self {
        Self {
            x:     u16::from_le_bytes([buf[0], buf[1]]),
            y:     u16::from_le_bytes([buf[2], buf[3]]),
            z:     u16::from_le_bytes([buf[4], buf[5]]),
            rhall: u16::from_le_bytes([buf[6], buf[7]]),
        }
    }
}

impl RawReading {
    /// DRDY status bit is bit 0 of the RHALL LSB register
    fn is_ready(&self) -> bool {
        self.rhall & 0x01 == 1
    }

    /// 13-bit signed — data sits in bits 15:3, shift right to discard padding
    fn x_unscaled(&self) -> i16 {
        (self.x as i16) >> 3
    }

    /// 13-bit signed
    fn y_unscaled(&self) -> i16 {
        (self.y as i16) >> 3
    }

    /// 15-bit signed — data sits in bits 15:1
    fn z_unscaled(&self) -> i16 {
        (self.z as i16) >> 1
    }

    /// 14-bit unsigned — data sits in bits 15:2
    fn rhall_unscaled(&self) -> u16 {
        self.rhall >> 2
    }


}

pub struct MagnetometerReading {
    pub x: i16,
    pub y: i16,
    pub z: i16
}

impl MagnetometerReading {

    fn from_raw<E: core::fmt::Debug>(raw: &RawReading, trim: &TrimData) -> Result<Self, MagnetometerError<E>> 
    {
        if !raw.is_ready() { return Err(MagnetometerError::StaleData); }
        Ok(Self {
            x: compensate_x(raw.x_unscaled(), raw.rhall_unscaled(), trim),
            y: compensate_y(raw.y_unscaled(), raw.rhall_unscaled(), trim),
            z: compensate_z(raw.z_unscaled(), raw.rhall_unscaled(), trim),
        })
    }
}



struct TrimX1Y1 {
    dig_x1: i8,
    dig_y1: i8,
}

impl From<[u8; 2]> for TrimX1Y1 {
    fn from(data: [u8; 2]) -> Self {
        Self {
            dig_x1: data[0] as i8,
            dig_y1: data[1] as i8,
        }
    }
}

struct TrimXYZ {
    dig_z4: i16,
    dig_x2: i8,
    dig_y2: i8,
}

impl From<[u8; 4]> for TrimXYZ {
    fn from(data: [u8; 4]) -> Self {
        Self {
            dig_z4: i16::from_le_bytes([data[0], data[1]]),
            dig_x2: data[2] as i8,
            dig_y2: data[3] as i8,
        }
    }
}

struct TrimXY1XY2 {
    dig_z2:   i16,
    dig_z1:   i16,
    dig_xyz1: u16,
    dig_z3:   i16,
    dig_xy1:  u8,
    dig_xy2:  i8,
}

impl From<[u8; 10]> for TrimXY1XY2 {
    fn from(data: [u8; 10]) -> Self {
        Self {
            dig_z2:   i16::from_le_bytes([data[0], data[1]]),
            dig_z1:   i16::from_le_bytes([data[2], data[3]]),
            dig_xyz1: u16::from_le_bytes([data[4], data[5]]) & 0x7fff,
            dig_z3:   i16::from_le_bytes([data[6], data[7]]),
            dig_xy1:  data[8],
            dig_xy2:  data[9] as i8,
        }
    }
}

pub struct TrimData {
    dig_x1:  i8,
    dig_y1:  i8,
    dig_z4:  i16,
    dig_x2:  i8,
    dig_y2:  i8,
    dig_z2:  i16,
    dig_z1:  i16,
    dig_xyz1: u16,
    dig_z3:  i16,
    dig_xy1: u8,
    dig_xy2: i8,
}

impl From<(TrimX1Y1, TrimXYZ, TrimXY1XY2)> for TrimData {
    fn from((x1y1, xyz, xy1xy2): (TrimX1Y1, TrimXYZ, TrimXY1XY2)) -> Self {
        Self {
            dig_x1:  x1y1.dig_x1,
            dig_y1:  x1y1.dig_y1,
            dig_z4:  xyz.dig_z4,
            dig_x2:  xyz.dig_x2,
            dig_y2:  xyz.dig_y2,
            dig_z2:  xy1xy2.dig_z2,
            dig_z1:  xy1xy2.dig_z1,
            dig_xyz1: xy1xy2.dig_xyz1,
            dig_z3:  xy1xy2.dig_z3,
            dig_xy1: xy1xy2.dig_xy1,
            dig_xy2: xy1xy2.dig_xy2,
        }
    }
}

fn compensate_x(mag_data_x: i16, data_rhall: u16, trim: &TrimData) -> i16 {
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

    if mag_data_x != MAG_OVERFLOW_XY {
        if data_rhall != 0 {
            process_comp_x0 = data_rhall;
        } else if trim.dig_xyz1 != 0 {
            process_comp_x0 = trim.dig_xyz1;
        } else {
            process_comp_x0 = 0;
        }

        if process_comp_x0 != 0 {
            process_comp_x1 = i32::from(trim.dig_xyz1) * 16384;
            process_comp_x2 = u16::try_from(process_comp_x1 / i32::from(process_comp_x0))
                .unwrap()
                .wrapping_sub(0x4000);
            let tmp_val: i16 = process_comp_x2 as i16;
            process_comp_x3 = i32::from(tmp_val) * i32::from(tmp_val);
            process_comp_x4 = i32::from(trim.dig_xy2) * (process_comp_x3 / 128);
            process_comp_x5 = i32::from(i16::from(trim.dig_xy1) * 128);
            process_comp_x6 = i32::from(tmp_val) * process_comp_x5;
            process_comp_x7 = ((process_comp_x4 + process_comp_x6) / 512) + 0x100000;
            process_comp_x8 = i32::from(i16::from(trim.dig_x2) + 0xA0);
            process_comp_x9 = (process_comp_x7 * process_comp_x8) / 4096;
            process_comp_x10 = i32::from(mag_data_x) * process_comp_x9;
            retval = i16::try_from(process_comp_x10 / 8192).unwrap();
            retval = (retval + (i16::from(trim.dig_x1) * 8)) / 16;
        } else {
            retval = MAG_OVERFLOW_OUTPUT;
        }
    } else {
        retval = MAG_OVERFLOW_OUTPUT;
    }

    retval
}

fn compensate_y(mag_data_y: i16, data_rhall: u16, trim: &TrimData) -> i16 {
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

    if mag_data_y != MAG_OVERFLOW_XY {
        if data_rhall != 0 {
            process_comp_y0 = data_rhall;
        } else if trim.dig_xyz1 != 0 {
            process_comp_y0 = trim.dig_xyz1;
        } else {
            process_comp_y0 = 0;
        }

        if process_comp_y0 != 0 {
            process_comp_y1 =
                ((i32::from(trim.dig_xyz1)) * 16384) / i32::from(process_comp_y0);
            process_comp_y2 = u16::try_from(process_comp_y1).unwrap().wrapping_sub(0x4000);
            let tmp_val: i16 = process_comp_y2 as i16;
            process_comp_y3 = i32::from(tmp_val) * i32::from(tmp_val);
            process_comp_y4 = i32::from(trim.dig_xy2) * (process_comp_y3 / 128);
            process_comp_y5 = i32::from(i16::from(trim.dig_xy1) * 128);
            process_comp_y6 = (process_comp_y4 + (i32::from(tmp_val) * process_comp_y5)) / 512;
            process_comp_y7 = i32::from(i16::from(trim.dig_y2) + 0xA0);
            process_comp_y8 = ((process_comp_y6 + 0x100000) * process_comp_y7) / 4096;
            process_comp_y9 = i32::from(mag_data_y) * process_comp_y8;
            retval = i16::try_from(process_comp_y9 / 8192).unwrap();
            retval = (retval + (i16::from(trim.dig_y1) * 8)) / 16;
        } else {
            retval = MAG_OVERFLOW_OUTPUT;
        }
    } else {
        retval = MAG_OVERFLOW_OUTPUT;
    }

    retval
}

fn compensate_z(mag_data_z: i16, data_rhall: u16, trim: &TrimData) -> i16 {
    let mut retval: i32;
    let process_comp_z0: i16;
    let process_comp_z1: i32;
    let process_comp_z2: i32;
    let process_comp_z3: i32;
    let process_comp_z4: i16;

    if mag_data_z != MAG_OVERFLOW_ADCVAL_ZAXIS_HALL {
        if (trim.dig_z2 != 0)
            && (trim.dig_z1 != 0)
            && (data_rhall != 0)
            && (trim.dig_xyz1 != 0)
        {
            process_comp_z0 = i16::try_from(data_rhall).unwrap()
                - i16::try_from(trim.dig_xyz1).unwrap();
            process_comp_z1 =
                (i32::from(trim.dig_z3) * i32::from(process_comp_z0)) / 4;
            process_comp_z2 = (i32::from(mag_data_z - trim.dig_z4)) * 32768;
            process_comp_z3 = i32::from(trim.dig_z1) * (i32::from(data_rhall) * 2);
            process_comp_z4 = i16::try_from((process_comp_z3 + 32768) / 65536).unwrap();
            retval = (process_comp_z2 - process_comp_z1)
                / (i32::from(trim.dig_z2) + i32::from(process_comp_z4));

            if retval > MAG_POSITIVE_SATURATION_Z {
                retval = MAG_POSITIVE_SATURATION_Z;
            } else if retval < MAG_NEGATIVE_SATURATION_Z {
                retval = MAG_NEGATIVE_SATURATION_Z;
            }

            retval /= 16;
        } else {
            retval = i32::from(MAG_OVERFLOW_OUTPUT);
        }
    } else {
        retval = i32::from(MAG_OVERFLOW_OUTPUT);
    }

    retval.try_into().unwrap()
}