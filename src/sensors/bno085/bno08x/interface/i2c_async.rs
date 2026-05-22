use crate::sensors::bno085::bno08x::Error;

use super::i2c_common::I2cCommon;
use super::{PACKET_HEADER_LENGTH, SensorInterfaceAsync};

use embedded_hal_async::delay::DelayNs;

pub use super::i2c_common::{ALTERNATE_ADDRESS, DEFAULT_ADDRESS};

pub struct I2cInterfaceAsync<I2C, HINT> {
    /// i2c port
    i2c_port: I2C,
    common: I2cCommon,
    hint: HINT,
}

impl<I2C, HINT, CommE> I2cInterfaceAsync<I2C, HINT>
where
    I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
    HINT: embedded_hal::digital::InputPin + embedded_hal_async::digital::Wait,
{
    pub fn hint_low(&mut self) -> bool {
        self.hint.is_low().unwrap_or_default()
    }

    pub fn default(i2c: I2C, hint: HINT) -> Self {
        Self::new(i2c, DEFAULT_ADDRESS, hint)
    }

    pub fn alternate(i2c: I2C, hint: HINT) -> Self {
        Self::new(i2c, ALTERNATE_ADDRESS, hint)
    }

    pub fn new(i2c: I2C, addr: u8, hint: HINT) -> Self {
        Self {
            i2c_port: i2c,
            common: I2cCommon::new(addr),
            hint,
        }
    }

    pub fn free(self) -> I2C {
        self.i2c_port
    }

    pub async fn wait_for_hint(&mut self) -> Result<(), HINT::Error> {
        self.hint.wait_for_low().await
    }
    pub async fn wait_for_hint_high(&mut self) -> Result<(), HINT::Error> {
        self.hint.wait_for_high().await
    }

    async fn read_packet_header(&mut self) -> Result<(), Error<CommE, ()>> {
        self.common.zero_recv_packet_header();
        let address = self.common.address();
        self.i2c_port
            .read(
                address,
                &mut self.common.seg_recv_buf_mut()[..PACKET_HEADER_LENGTH],
            )
            .await
            .map_err(Error::Comm)?;

        Ok(())
    }

    /// Read the remainder of the packet after the packet header, if any
    async fn read_sized_packet(
        &mut self,
        total_packet_len: usize,
        packet_recv_buf: &mut [u8],
    ) -> Result<usize, Error<CommE, ()>> {
        let mut sized_read = I2cCommon::sized_read(total_packet_len, packet_recv_buf);

        // #[cfg(feature = "rttdebug")]
        // rprintln!("r.t {}", total_packet_len);

        if let Some(read_len) = sized_read.direct_read_len() {
            let address = self.common.address();
            self.i2c_port
                .read(address, &mut packet_recv_buf[..read_len])
                .await
                .map_err(Error::Comm)?;
            return Ok(sized_read.finish_direct_read(read_len));
        }

        while sized_read.has_remaining_segments() {
            let segment_read_len = sized_read.next_segment_read_len();
            // #[cfg(feature = "rttdebug")]
            // rprintln!("r.s {:x} {}", self.common.address(), segment_read_len);

            self.common.zero_recv_packet_header();
            let address = self.common.address();
            self.i2c_port
                .read(
                    address,
                    &mut self.common.seg_recv_buf_mut()[..segment_read_len],
                )
                .await
                .map_err(Error::Comm)?;

            let promised_packet_len = self.common.packet_len_from_header();
            if promised_packet_len <= PACKET_HEADER_LENGTH {
                return Ok(0);
            }

            sized_read.transcribe_segment(
                segment_read_len,
                self.common.seg_recv_buf(),
                packet_recv_buf,
            );
        }

        Ok(sized_read.already_read_len())
    }
}

impl<I2C, HINT, CommE> SensorInterfaceAsync for I2cInterfaceAsync<I2C, HINT>
where
    I2C: embedded_hal_async::i2c::I2c<Error = CommE>,
    HINT: embedded_hal::digital::InputPin + embedded_hal_async::digital::Wait,
{
    type SensorError = Error<CommE, ()>;

    fn requires_soft_reset(&self) -> bool {
        true
    }

    async fn setup(&mut self, delay_source: &mut impl DelayNs) -> Result<(), Self::SensorError> {
        // #[cfg(feature = "rttdebug")]
        // rprintln!("i2c setup");
        delay_source.delay_ms(5).await;
        Ok(())
    }

    async fn write_packet(&mut self, packet: &[u8]) -> Result<(), Self::SensorError> {
        let address = self.common.address();
        self.i2c_port
            .write(address, packet)
            .await
            .map_err(Error::Comm)?;
        Ok(())
    }

    async fn read_with_timeout(
        &mut self,
        recv_buf: &mut [u8],
        delay_source: &mut impl DelayNs,
        max_ms: u8,
    ) -> Result<usize, Self::SensorError> {
        let mut total_delay: u8 = 0;
        while total_delay < max_ms {
            match self.read_packet(recv_buf).await {
                Ok(read_size) => {
                    if 0 == read_size {
                        // no data available yet...wait a while longer
                        delay_source.delay_ms(1).await;
                        total_delay += 1;
                    } else {
                        return Ok(read_size);
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(0)
    }

    /// Read one packet into the receive buffer
    async fn read_packet(&mut self, recv_buf: &mut [u8]) -> Result<usize, Self::SensorError> {
        // #[cfg(feature = "rttdebug")]
        // rprintln!("rpkt");
        if self.hint.is_high().unwrap_or(true) {
            return Ok(0);
        }

        self.read_packet_header().await?;
        let packet_len = self.common.packet_len_from_header();

        // if packet_len == 0 {
        //     #[cfg(feature = "rttdebug")]
        //     rprintln!("eh {:x?}", &self.common.seg_recv_buf()[..PACKET_HEADER_LENGTH]);
        // }

        let received_len = if packet_len > PACKET_HEADER_LENGTH {
            self.read_sized_packet(packet_len, recv_buf).await?
        } else {
            packet_len
        };

        self.common.record_received_packet(packet_len);

        Ok(received_len)
    }

    async fn send_and_receive_packet(
        &mut self,
        send_buf: &[u8],
        recv_buf: &mut [u8],
    ) -> Result<usize, Self::SensorError> {
        // Cannot use write_read with bno080,
        // because it does not support repeated start with i2c.
        let address = self.common.address();
        self.i2c_port
            .write(address, send_buf)
            .await
            .map_err(Error::Comm)?;
        self.common.zero_recv_packet_header();
        I2cCommon::zero_buffer(recv_buf);
        self.i2c_port
            .read(
                address,
                &mut self.common.seg_recv_buf_mut()[..PACKET_HEADER_LENGTH],
            )
            .await
            .map_err(Error::Comm)?;
        let packet_len = self.common.packet_len_from_header();
        let received_len = if packet_len > PACKET_HEADER_LENGTH {
            self.read_sized_packet(packet_len, recv_buf).await?
        } else {
            packet_len
        };

        self.common.record_received_packet(packet_len);

        Ok(received_len)
    }
}

impl<I2C> I2cInterfaceAsync<I2C, embassy_rp::gpio::Input<'static>> {
    pub fn dormant_sleep_on_hint(&mut self) {
        let dormant = self.hint.dormant_wake(embassy_rp::gpio::DormantWakeConfig {
            edge_high: false,
            edge_low: true,
            level_high: false,
            level_low: false,
        });
        embassy_rp::clocks::dormant_sleep();
        drop(dormant);
    }
}
