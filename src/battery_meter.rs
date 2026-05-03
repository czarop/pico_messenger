use embedded_hal_async::i2c::I2c;

const MAX17048_ADDR: u8 = 0x36;
const DEFAULT_RCOMP: u8 = 0x97;

pub struct Max17048<I> {
    i2c: I,
    recv_buffer: [u8; 2],
}

impl<I> Max17048<I>
where
    I: I2c,
{
    pub fn new(i2c: I) -> Self {
        let max = Max17048 {
            i2c: i2c,
            recv_buffer: [0u8; 2],
        };
        // max.compensation(DEFAULT_RCOMP).await.unwrap();
        max
    }

    pub async fn version(&mut self) -> Result<u16, I::Error> {
        self.read(0x08).await
    }

    pub async fn soc(&mut self) -> Result<u16, I::Error> {
        match self.read(0x04).await {
            Ok(val) => Ok(val / 256),
            Err(e) => Err(e),
        }
    }

    /// Return C/Rate in %/hr
    pub async fn charge_rate(&mut self) -> Result<f32, I::Error> {
        match self.read(0x16).await {
            Ok(val) => Ok(val as i16 as f32 * 0.208),
            Err(e) => Err(e),
        }
    }

    pub async fn vcell(&mut self) -> Result<f32, I::Error> {
        match self.read(0x02).await {
            Ok(val) => Ok(val as f32 * 0.000078125),
            Err(e) => Err(e),
        }
    }

    pub async fn temp_compensation(&mut self, temp: f32) -> Result<(), I::Error> {
        let rcomp = if temp > 20.0 {
            DEFAULT_RCOMP as f32 + (temp - 20.0) * -0.5
        } else {
            DEFAULT_RCOMP as f32 + (temp - 20.0) * -5.0
        };
        self.compensation(rcomp as u8).await
    }

    async fn compensation(&mut self, rcomp: u8) -> Result<(), I::Error> {
        // read the current reg vals
        match self.read(0x0C).await {
            Ok(mut value) => {
                value &= 0x00FF;
                value |= (rcomp as u16) << 8;
                // write to the rcomp bits only
                self.write(0x0C, value).await?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn read(&mut self, reg: u8) -> Result<u16, I::Error> {
        match self
            .i2c
            .write_read(MAX17048_ADDR, &[reg], &mut self.recv_buffer)
            .await
        {
            Ok(_) => Ok((self.recv_buffer[0] as u16) << 8 | self.recv_buffer[1] as u16),
            Err(e) => Err(e),
        }
    }

    async fn write(&mut self, reg: u8, value: u16) -> Result<(), I::Error> {
        // self.i2c.write(MAX17048_ADDR, &[reg]).await?;
        let msb = ((value & 0xFF00) >> 8) as u8;
        let lsb = ((value & 0x00FF) >> 0) as u8;
        // self.i2c.write(MAX17048_ADDR, &[msb, lsb]).await?;
        self.i2c.write(MAX17048_ADDR, &[reg, msb, lsb]).await?;
        Ok(())
    }
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub enum BatteryLevel {
    Empty,
    Critical,
    Low,
    Medium,
    High,
    Full,
    Charging,
}

impl BatteryLevel {
    pub fn from_soc(soc: u16, is_charging: bool) -> Self {
        if is_charging {
            BatteryLevel::Charging
        } else if soc <= 5 {
            BatteryLevel::Empty
        } else if soc <= 20 {
            BatteryLevel::Critical
        } else if soc <= 40 {
            BatteryLevel::Low
        } else if soc <= 60 {
            BatteryLevel::Medium
        } else if soc <= 80 {
            BatteryLevel::High
        } else {
            BatteryLevel::Full
        }
    }
}
