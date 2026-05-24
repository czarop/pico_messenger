use bmp390::{Address, Bmp390, Configuration};

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
        let sensor = Bmp390::try_new(i2c, bmp390::Address::Up, delay, &config).await?;
        Ok(Self { sensor })
    }

    pub async fn read_altitude(&mut self) -> f32 {
        let res = self.sensor.altitude().await.unwrap();
        res.into()
    }
}
