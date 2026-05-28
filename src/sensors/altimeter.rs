use bmp390::{Bmp390, Configuration};

pub struct Altimeter<I> {
    sensor: Bmp390<I>,
}

impl<I, E> Altimeter<I>
where
    I: embedded_hal_async::i2c::I2c<Error = E>,
{
    pub async fn new(i2c: I) -> Result<Self, bmp390::Error<E>> {
        let delay = embassy_time::Delay;
        let config = Configuration::default();
        let sensor = Bmp390::try_new(i2c, bmp390::Address::Down, delay, &config).await?;
        Ok(Self { sensor })
    }

    pub async fn read_altitude(&mut self) -> Result<f32, bmp390::Error<E>> {
        let res = self.sensor.measure().await?;
        Ok(res.altitude.get::<uom::si::length::meter>())
    }

    pub async fn read_pressure(&mut self) -> Result<f32, bmp390::Error<E>> {
        let res = self.sensor.measure().await?;
        Ok(res.pressure.get::<uom::si::pressure::pascal>())
    }
}
