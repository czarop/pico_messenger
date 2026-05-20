use bno080::Error;
use bno080::interface::i2c_async::I2cInterfaceAsync;
use bno080::wrapper_async::{BNO080Async, WrapperError};

pub struct Imu<I2C> {
    inner: BNO080Async<I2cInterfaceAsync<I2C>>,
}

impl<I2C, CommE> Imu<I2C>
where
    I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
    CommE: core::fmt::Debug,
{
    pub async fn new(i2c: I2C) -> Self {
        let iface = I2cInterfaceAsync::default(i2c);
        let mut inner = BNO080Async::new_with_interface(iface);
        inner
            .init(&mut embassy_time::Delay)
            .await
            .expect("BNO085 init failed");
        Self { inner }
    }

    pub async fn enable_rotation_vector(
        &mut self,
        millis: u16,
    ) -> Result<(), WrapperError<Error<CommE, ()>>> {
        self.inner.enable_rotation_vector(millis).await
    }

    pub async fn heading(&mut self) -> Result<[f32; 4], WrapperError<Error<CommE, ()>>> {
        defmt::info!("starting");
        let handled = self
            .inner
            .handle_all_messages(&mut embassy_time::Delay, 150)
            .await;
        defmt::info!("handled {} messages", handled);
        self.inner.rotation_quaternion()
    }
}
