//! PCF8523 Real-Time Clock driver.
//!
//! # Role in the system
//!
//! This is the periodic wake source for the whole power architecture. The RP2350
//! in DORMANT stops every clock and can only be woken by a GPIO edge (verified:
//! the POWMAN/AON alarm cannot rouse a comatose ROSC, and the modem's own TAU
//! wake is floored by the network at 4 hours). The PCF8523's countdown timer
//! toggles its INT pin on a schedule we control, giving us exactly the periodic
//! GPIO edge DORMANT needs.
//!
//! It also carries a real wall clock (32-bit BCD calendar), which closes the
//! separate "production clock from GNSS" item: seed it once from the first fix
//! and it holds UTC between fixes.
//!
//! # The INT/SQW pin
//!
//! The PCF8523 has ONE pin, `INT1/CLKOUT`, shared between the interrupt output
//! and a square-wave clock output. Adafruit's breakout labels it `SQW`. It is the
//! same pad. Using it as an interrupt REQUIRES disabling CLKOUT (they are mutually
//! exclusive on that pin) -- which [`init`] does. That is also the low-power
//! choice: I_DD is 150 nA typical with CLKOUT off, versus 1200 nA with 32 kHz
//! CLKOUT on (datasheet §12).
//!
//! It is open-drain, active LOW. Wire it to a spare RP2350 GPIO configured with a
//! pull-up, and wake on the FALLING edge.
//!
//! # Bus sharing
//!
//! This driver is generic over any `embedded_hal_async::i2c::I2c`, exactly like
//! `sensors::bno085::Imu`, so it composes with an `I2cDevice` off a shared-bus
//! `Mutex`. Note: the BNO085 driver's type alias currently pins it to `I2C1`
//! while the intent stated is `I2C0` -- confirm which bus the RTC actually shares
//! before wiring, but the driver itself does not care.
//!
//! I2C address: `0x68` (fixed; datasheet §8.11.5).

use embedded_hal_async::i2c::I2c;

/// 7-bit I2C address (fixed).
pub const ADDR: u8 = 0x68;

// Register map (datasheet Table 6).
mod reg {
    pub const CONTROL_1: u8 = 0x00;
    pub const CONTROL_2: u8 = 0x01;
    pub const CONTROL_3: u8 = 0x02;
    pub const SECONDS: u8 = 0x03;
    // 0x04 Minutes, 0x05 Hours, 0x06 Days, 0x07 Weekdays, 0x08 Months, 0x09 Years
    pub const TMR_CLKOUT_CTRL: u8 = 0x0F;
    pub const TMR_B_FREQ_CTRL: u8 = 0x12;
    pub const TMR_B_REG: u8 = 0x13;
}

/// A decoded wall-clock time. All fields are plain integers (already un-BCD'd).
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct DateTime {
    pub year: u16, // full year, e.g. 2026
    pub month: u8, // 1..=12
    pub day: u8,   // 1..=31
    pub hour: u8,  // 0..=23
    pub minute: u8,
    pub second: u8,
}

#[derive(Debug, defmt::Format)]
pub enum RtcError<E> {
    I2c(E),
    /// The oscillator-stop flag was set: timekeeping is not trustworthy and must
    /// be reseeded (e.g. from GNSS). Not a hardware fault -- expected on any cold
    /// power-up of a board with no backup cell.
    ClockUnreliable,
}

impl<E> From<E> for RtcError<E> {
    fn from(e: E) -> Self {
        RtcError::I2c(e)
    }
}

fn to_bcd(v: u8) -> u8 {
    ((v / 10) << 4) | (v % 10)
}
fn from_bcd(v: u8) -> u8 {
    ((v >> 4) * 10) + (v & 0x0f)
}

pub struct Pcf8523<I2C> {
    i2c: I2C,
}

impl<I2C, E> Pcf8523<I2C>
where
    I2C: I2c<Error = E>,
{
    /// Construct and initialise for low-power periodic-wake operation.
    ///
    /// Performs a software reset, disables CLKOUT (mandatory for INT use, and the
    /// low-power setting), and disables battery switch-over (we run without a
    /// backup cell, so `VBAT` must be tied to `VDD` on the board). Leaves the
    /// countdown timer stopped -- call [`set_countdown_minutes`] to arm it.
    ///
    /// Returns `Err(ClockUnreliable)` if the oscillator-stop flag is set, i.e. the
    /// clock lost time (always true on a cold start without a backup cell). This
    /// is informational: the caller should reseed the time from GNSS, then may
    /// proceed. The timer wake works regardless of clock validity.
    pub async fn new(i2c: I2C) -> Result<Self, RtcError<E>> {
        let mut rtc = Self { i2c };

        // Software reset: write 0x58 to Control_1 (datasheet §8.3).
        rtc.write(reg::CONTROL_1, 0x58).await?;

        // Disable CLKOUT so INT1 is available on the shared pin, and to drop from
        // 1200 nA to 150 nA. COF[2:0] = 111 in Tmr_CLKOUT_ctrl. All other bits 0:
        // timers disabled, interrupts permanent-active (irrelevant while off).
        rtc.write(reg::TMR_CLKOUT_CTRL, 0b0011_1000).await?;

        // Control_3: PM[2:0] = 111 -> battery switch-over disabled, battery-low
        // detection disabled (no backup cell fitted). Datasheet Table 11.
        rtc.write(reg::CONTROL_3, 0b1110_0000).await?;

        // Control_2: clear all interrupt enables and flags.
        rtc.write(reg::CONTROL_2, 0x00).await?;

        // Check (and clear) the oscillator-stop flag in Seconds bit 7.
        let secs = rtc.read(reg::SECONDS).await?;
        if secs & 0x80 != 0 {
            // Clear OS by rewriting Seconds without bit 7. Value is undefined on
            // cold start; zero it. The caller will reseed via set_time().
            rtc.write(reg::SECONDS, 0x00).await?;
            return Err(RtcError::ClockUnreliable);
        }

        Ok(rtc)
    }

    /// Arm the Timer B countdown to fire every `minutes` (1..=255), asserting INT.
    ///
    /// Uses the 1/60 Hz source clock, so the register value equals the period in
    /// whole minutes. On expiry the flag sets, INT pulses low, and the counter
    /// **auto-reloads** -- no re-arming needed for a fixed cadence. Change the
    /// period by calling again; passing a different value here is safe because we
    /// disable the timer before writing (a live `T_B` change can load a corrupt
    /// value, datasheet §8.9.3).
    ///
    /// Regime mapping: 15 for ACTIVE_TRACKING, 10 for STATIONARY_PENDING.
    pub async fn set_countdown_minutes(&mut self, minutes: u8) -> Result<(), RtcError<E>> {
        debug_assert!(minutes >= 1, "0 stops the timer; use stop_countdown");

        // 1. Disable Timer B before touching T_B (TBC = 0). Keep CLKOUT off.
        self.write(reg::TMR_CLKOUT_CTRL, 0b0011_1000).await?;

        // 2. Source clock = 1/60 Hz (TBQ[2:0] = 011); default pulse width.
        self.write(reg::TMR_B_FREQ_CTRL, 0b0000_0011).await?;

        // 3. Period in minutes.
        self.write(reg::TMR_B_REG, minutes).await?;

        // 4. Enable countdown-timer-B interrupt (CTBIE = 1) in Control_2, clearing
        //    any stale CTBF. Writing 0 to a flag bit clears it; writing 1 to an
        //    enable bit sets it (Table 8).
        self.write(reg::CONTROL_2, 0b0000_0001).await?;

        // 5. Enable Timer B (TBC = 1), pulsed interrupt (TBM = 1), CLKOUT still off.
        self.write(reg::TMR_CLKOUT_CTRL, 0b0111_1001).await?;

        Ok(())
    }

    /// Stop the countdown timer entirely (no periodic wake).
    ///
    /// This is DEEP_REST: with the timer off, only an external source (the BNO085
    /// significant-motion INT on its own GPIO) can wake the host. Loading T_B = 0
    /// stops the timer (datasheet §8.9.3); we also clear TBC and the interrupt
    /// enable so INT stays released.
    pub async fn stop_countdown(&mut self) -> Result<(), RtcError<E>> {
        self.write(reg::TMR_CLKOUT_CTRL, 0b0011_1000).await?; // TBC = 0, CLKOUT off
        self.write(reg::TMR_B_REG, 0x00).await?;
        self.write(reg::CONTROL_2, 0x00).await?; // CTBIE = 0, flags cleared
        Ok(())
    }

    /// Clear the countdown interrupt flag (CTBF). Call once per wake, or INT stays
    /// asserted and the next dormant entry would wake immediately.
    ///
    /// Clearing is a logical-AND write: 0 clears a flag, 1 leaves it unchanged
    /// (datasheet §8.7.5). We must preserve CTBIE (bit 0) and the other flags, so
    /// write all flag bits as 1 except CTBF (bit 5), and keep CTBIE set.
    pub async fn clear_flag(&mut self) -> Result<(), RtcError<E>> {
        // bits: WTAF CTAF CTBF SF AF WTAIE CTAIE CTBIE
        //        1    1    0   1  1   1     1     1
        // -> clear only CTBF, leave every enable and other flag untouched.
        self.write(reg::CONTROL_2, 0b1101_1111).await?;
        Ok(())
    }

    /// Set the wall clock. Seed this from the first GNSS fix (UTC).
    ///
    /// Writes seconds..years in one transaction, per the datasheet's requirement
    /// that the time be set in a single access to avoid carry corruption (§8.6.8).
    /// Weekday is written 0 (unused by us).
    pub async fn set_time(&mut self, t: &DateTime) -> Result<(), RtcError<E>> {
        let yr = (t.year % 100) as u8;
        let buf = [
            reg::SECONDS,
            to_bcd(t.second), // clears OS (bit 7 = 0)
            to_bcd(t.minute),
            to_bcd(t.hour), // 24h mode (set at reset)
            to_bcd(t.day),
            0x00, // weekday, unused
            to_bcd(t.month),
            to_bcd(yr),
        ];
        self.i2c.write(ADDR, &buf).await?;
        Ok(())
    }

    /// Read the wall clock. Reads seconds..years in one transaction (§8.6.8).
    ///
    /// Returns `Err(ClockUnreliable)` if the oscillator-stop flag is set.
    pub async fn now(&mut self) -> Result<DateTime, RtcError<E>> {
        let mut buf = [0u8; 7];
        self.i2c.write_read(ADDR, &[reg::SECONDS], &mut buf).await?;

        if buf[0] & 0x80 != 0 {
            return Err(RtcError::ClockUnreliable);
        }

        Ok(DateTime {
            second: from_bcd(buf[0] & 0x7f),
            minute: from_bcd(buf[1] & 0x7f),
            hour: from_bcd(buf[2] & 0x3f),
            day: from_bcd(buf[3] & 0x3f),
            // buf[4] = weekday, ignored
            month: from_bcd(buf[5] & 0x1f),
            year: 2000 + from_bcd(buf[6]) as u16,
        })
    }

    async fn write(&mut self, register: u8, value: u8) -> Result<(), E> {
        self.i2c.write(ADDR, &[register, value]).await
    }

    async fn read(&mut self, register: u8) -> Result<u8, E> {
        let mut buf = [0u8; 1];
        self.i2c.write_read(ADDR, &[register], &mut buf).await?;
        Ok(buf[0])
    }
}
