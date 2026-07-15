//! PCF8523 RTC used purely as a periodic wake source.
//!
//! # Role
//!
//! The RP2350 in DORMANT stops every clock and can only be woken by a GPIO edge.
//! Two hardware-verified facts forced an external timer:
//!
//! * The POWMAN/AON alarm cannot wake DORMANT -- it fires but the comatose ROSC
//!   never restarts.
//! * The modem's own PSM self-wake is floored by the network at a 4-hour TAU.
//!
//! The PCF8523's countdown timer toggles its INT pin on a schedule we set,
//! providing exactly the periodic GPIO edge DORMANT needs.
//!
//! # This driver ignores the wall clock entirely
//!
//! The PCF8523 also keeps a calendar, but we do not use it. Location payloads are
//! timestamped from GNSS time (decoded server-side), so the device never needs to
//! know the civil time. This driver therefore touches only the timer, control,
//! and CLKOUT registers -- never the seconds/minutes/hours calendar. The
//! oscillator-stop flag (which flags an unseeded clock on every cold boot of a
//! board with no backup cell) is irrelevant to us and deliberately not checked:
//! the countdown timer runs off the same 32.768 kHz oscillator and works whether
//! or not the calendar has ever been set.
//!
//! # The INT/SQW pin
//!
//! One pin, `INT1/CLKOUT`, shared between interrupt output and square-wave output
//! (Adafruit label it `SQW`). Using it as an interrupt REQUIRES disabling CLKOUT
//! -- they are mutually exclusive on that pad. That is also the low-power setting:
//! 150 nA typical with CLKOUT off vs 1200 nA with 32 kHz CLKOUT on (datasheet
//! Table, §12). [`new`] disables it.
//!
//! Open-drain, active LOW: wire to a spare RP2350 GPIO with a pull-up and wake on
//! the FALLING edge.
//!
//! I2C address `0x68`, fixed.

use embedded_hal_async::i2c::I2c;

/// 7-bit I2C address (fixed).
pub const ADDR: u8 = 0x68;

// Register map (datasheet Table 6). Only the ones we use.
mod reg {
    pub const CONTROL_1: u8 = 0x00;
    pub const CONTROL_2: u8 = 0x01;
    pub const CONTROL_3: u8 = 0x02;
    pub const TMR_CLKOUT_CTRL: u8 = 0x0F;
    pub const TMR_B_FREQ_CTRL: u8 = 0x12;
    pub const TMR_B_REG: u8 = 0x13;
}

// Tmr_CLKOUT_ctrl bit patterns.
//
//   bit7 TAM | bit6 TBM | bit5..3 COF[2:0] | bit2 TAC[1:0] hi | bit1..0 TBC etc.
//
// We only ever want: CLKOUT disabled (COF=111), Timer A off, Timer B either off
// or on with pulsed interrupt. Rather than track individual bits we use two
// fixed values, both with COF=111.
mod clkout_ctrl {
    /// CLKOUT off, both timers OFF. Used at init and to stop Timer B.
    pub const TIMERS_OFF: u8 = 0b0011_1000;
    /// CLKOUT off, Timer B ON, pulsed interrupt (TBM=1). Used when armed.
    pub const TIMER_B_ON_PULSED: u8 = 0b0111_1001;
}

#[derive(Debug, defmt::Format)]
pub struct RtcError<E>(pub E);

impl<E> From<E> for RtcError<E> {
    fn from(e: E) -> Self {
        RtcError(e)
    }
}

pub struct Pcf8523<I2C> {
    i2c: I2C,
}

impl<I2C, E> Pcf8523<I2C>
where
    I2C: I2c<Error = E>,
{
    /// Construct and initialise as a low-power periodic-wake source.
    ///
    /// Software-resets, disables CLKOUT (mandatory for INT use; also the low-power
    /// setting), disables battery switch-over (no backup cell fitted -- `VBAT`
    /// must be tied to `VDD` on the board), and clears all interrupt state. Leaves
    /// the countdown timer stopped; call [`set_countdown_minutes`] to arm it.
    ///
    /// Does NOT read or clear the oscillator-stop flag: we never use the calendar,
    /// and the timer works regardless. The only error path is an I2C failure,
    /// which means the chip isn't responding at all.
    pub async fn new(i2c: I2C) -> Result<Self, RtcError<E>> {
        let mut rtc = Self { i2c };

        // Software reset: 0x58 -> Control_1 (datasheet §8.3). Puts registers to a
        // known state (also selects 24h mode, which we don't use but is harmless).
        rtc.write(reg::CONTROL_1, 0x58).await?;

        // CLKOUT off so INT1 is usable, and to sit at 150 nA not 1200 nA.
        rtc.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;

        // Control_3 PM[2:0] = 111: battery switch-over and battery-low detection
        // both disabled (no backup cell). Datasheet Table 11.
        rtc.write(reg::CONTROL_3, 0b1110_0000).await?;

        // Control_2: all interrupt enables and flags cleared.
        rtc.write(reg::CONTROL_2, 0x00).await?;

        Ok(rtc)
    }

    /// Arm Timer B to assert INT every `minutes` (1..=255).
    ///
    /// Source clock 1/60 Hz, so the register value equals the period in whole
    /// minutes. On expiry the flag sets, INT pulses low, and the counter
    /// **auto-reloads** -- no re-arming for a fixed cadence. Safe to call again to
    /// change the period: the timer is disabled before `T_B` is written (a live
    /// change can latch a corrupt value, datasheet §8.9.3).
    ///
    /// Regime mapping: 15 for ACTIVE_TRACKING, 10 for STATIONARY_PENDING.
    pub async fn set_countdown_minutes(&mut self, minutes: u8) -> Result<(), RtcError<E>> {
        debug_assert!(minutes >= 1, "0 stops the timer; use stop_countdown()");

        // 1. Timer B off before touching T_B.
        self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;

        // 2. Source clock = 1/60 Hz (TBQ[2:0] = 011).
        self.write(reg::TMR_B_FREQ_CTRL, 0b0000_0011).await?;

        // 3. Period, in minutes.
        self.write(reg::TMR_B_REG, minutes).await?;

        // 4. Enable countdown-Timer-B interrupt (CTBIE=1), clearing stale flags.
        self.write(reg::CONTROL_2, 0b0000_0001).await?;

        // 5. Timer B on, pulsed interrupt.
        self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMER_B_ON_PULSED)
            .await?;

        Ok(())
    }

    /// Stop the countdown entirely -- no periodic wake.
    ///
    /// This is DEEP_REST: with the timer off, only the BNO085 significant-motion
    /// INT (on its own GPIO) can wake the host. Loading `T_B = 0` stops the timer
    /// (datasheet §8.9.3); we also drop TBC and the interrupt enable so INT is
    /// released.
    pub async fn stop_countdown(&mut self) -> Result<(), RtcError<E>> {
        self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;
        self.write(reg::TMR_B_REG, 0x00).await?;
        self.write(reg::CONTROL_2, 0x00).await?;
        Ok(())
    }

    /// Clear the countdown interrupt flag (CTBF). Call once per wake, or INT stays
    /// asserted and the next dormant entry wakes immediately.
    ///
    /// Flags clear on a 0 write and are unchanged on a 1 write (datasheet §8.7.5),
    /// so write every bit 1 except CTBF (bit 5), and keep CTBIE (bit 0) set.
    ///
    ///   WTAF CTAF CTBF SF AF WTAIE CTAIE CTBIE
    ///    1    1    0   1  1   1     1     1     = 0xDF
    pub async fn clear_flag(&mut self) -> Result<(), RtcError<E>> {
        self.write(reg::CONTROL_2, 0b1101_1111).await?;
        Ok(())
    }

    async fn write(&mut self, register: u8, value: u8) -> Result<(), E> {
        self.i2c.write(ADDR, &[register, value]).await
    }
}