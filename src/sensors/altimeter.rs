use bmp390::{Bmp390, Configuration};

pub struct Altimeter<I> {
    sensor: Bmp390<I>,
}

impl<I, E> Altimeter<I>
where
    I: embedded_hal_async::i2c::I2c<Error = E>,
{
    pub async fn new(i2c: I, curr_elevation_meters: Option<f32>) -> Result<Self, bmp390::Error<E>> {
        let delay = embassy_time::Delay;
        let config = Configuration::default();
        let sensor = Bmp390::try_new(i2c, bmp390::Address::Down, delay, &config).await?;

        let mut bn390 = Self { sensor };
        if let Some(meters) = curr_elevation_meters {
            bn390.zero_altimeter(meters)
        }

        Ok(bn390)
    }

    pub fn zero_altimeter(&mut self, known_altitude_meters: f32) {
        let meters = uom::si::f32::Length::new::<uom::si::length::meter>(known_altitude_meters);
        self.sensor.set_reference_altitude(meters);
    }

    pub async fn read_altitude(&mut self) -> Result<f32, bmp390::Error<E>> {
        let res = self.sensor.measure().await?;
        let altitude = res.altitude.get::<uom::si::length::meter>();
        Ok(altitude)
    }

    pub async fn read_pressure(&mut self) -> Result<f32, bmp390::Error<E>> {
        let res = self.sensor.measure().await?;
        Ok(res.pressure.get::<uom::si::pressure::pascal>())
    }
}
