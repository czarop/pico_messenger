use super::{PACKET_HEADER_LENGTH, SensorCommon};

/// the i2c address normally used by BNO080
pub const DEFAULT_ADDRESS: u8 = 0x4A;
/// alternate i2c address for BNO080
pub const ALTERNATE_ADDRESS: u8 = 0x4B;

/// Length of our receive buffer:
/// Note that this likely needs to be < 256 to accommodate underlying HAL
pub(crate) const SEG_RECV_BUF_LEN: usize = 240;
pub(crate) const MAX_SEGMENT_READ: usize = SEG_RECV_BUF_LEN;

pub(crate) struct I2cCommon {
    /// address for i2c communications with the sensor hub
    address: u8,
    /// buffer for receiving segments of packets from the sensor hub
    seg_recv_buf: [u8; SEG_RECV_BUF_LEN],
    /// number of packets received
    received_packet_count: usize,
}

impl I2cCommon {
    pub(crate) fn new(address: u8) -> Self {
        Self {
            address,
            seg_recv_buf: [0; SEG_RECV_BUF_LEN],
            received_packet_count: 0,
        }
    }

    pub(crate) fn address(&self) -> u8 {
        self.address
    }

    pub(crate) fn seg_recv_buf(&self) -> &[u8; SEG_RECV_BUF_LEN] {
        &self.seg_recv_buf
    }

    pub(crate) fn seg_recv_buf_mut(&mut self) -> &mut [u8; SEG_RECV_BUF_LEN] {
        &mut self.seg_recv_buf
    }

    pub(crate) fn zero_recv_packet_header(&mut self) {
        Self::zero_buffer(&mut self.seg_recv_buf[..PACKET_HEADER_LENGTH]);
    }

    pub(crate) fn zero_buffer(buf: &mut [u8]) {
        for byte in buf {
            *byte = 0;
        }
    }

    pub(crate) fn packet_len_from_header(&self) -> usize {
        SensorCommon::parse_packet_header(&self.seg_recv_buf[..PACKET_HEADER_LENGTH])
    }

    pub(crate) fn record_received_packet(&mut self, packet_len: usize) {
        if packet_len > 0 {
            self.received_packet_count += 1;
        }
    }

    pub(crate) fn sized_read(total_packet_len: usize, packet_recv_buf: &mut [u8]) -> SizedRead {
        SizedRead::new(total_packet_len, packet_recv_buf)
    }
}

pub(crate) struct SizedRead {
    remaining_body_len: usize,
    already_read_len: usize,
    total_packet_len: usize,
}

impl SizedRead {
    fn new(total_packet_len: usize, packet_recv_buf: &mut [u8]) -> Self {
        let remaining_body_len = total_packet_len - PACKET_HEADER_LENGTH;

        Self::zero_packet_header(packet_recv_buf);

        Self {
            remaining_body_len,
            already_read_len: 0,
            total_packet_len,
        }
    }

    pub(crate) fn direct_read_len(&self) -> Option<usize> {
        if self.total_packet_len < MAX_SEGMENT_READ && self.total_packet_len > 0 {
            Some(self.total_packet_len)
        } else {
            None
        }
    }

    pub(crate) fn finish_direct_read(&mut self, already_read_len: usize) -> usize {
        self.already_read_len = already_read_len;
        self.already_read_len
    }

    pub(crate) fn has_remaining_segments(&self) -> bool {
        self.remaining_body_len > 0
    }

    pub(crate) fn next_segment_read_len(&self) -> usize {
        let whole_segment_length = self.remaining_body_len + PACKET_HEADER_LENGTH;
        if whole_segment_length > MAX_SEGMENT_READ {
            MAX_SEGMENT_READ
        } else {
            whole_segment_length
        }
    }

    pub(crate) fn transcribe_segment(
        &mut self,
        segment_read_len: usize,
        seg_recv_buf: &[u8],
        packet_recv_buf: &mut [u8],
    ) {
        let transcribe_start_idx = if self.already_read_len > 0 {
            PACKET_HEADER_LENGTH
        } else {
            0
        };
        let transcribe_len = if self.already_read_len > 0 {
            segment_read_len - PACKET_HEADER_LENGTH
        } else {
            segment_read_len
        };

        packet_recv_buf[self.already_read_len..self.already_read_len + transcribe_len]
            .copy_from_slice(
                &seg_recv_buf[transcribe_start_idx..transcribe_start_idx + transcribe_len],
            );
        self.already_read_len += transcribe_len;

        let body_read_len = segment_read_len - PACKET_HEADER_LENGTH;
        self.remaining_body_len -= body_read_len;
    }

    pub(crate) fn already_read_len(&self) -> usize {
        self.already_read_len
    }

    fn zero_packet_header(packet_recv_buf: &mut [u8]) {
        for byte in &mut packet_recv_buf[..PACKET_HEADER_LENGTH] {
            *byte = 0;
        }
    }
}