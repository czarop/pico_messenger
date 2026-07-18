// //! PCF8523 RTC used purely as a periodic wake source.
// //!
// //! # Role
// //!
// //! The RP2350 in DORMANT stops every clock and can only be woken by a GPIO edge.
// //! Two hardware-verified facts forced an external timer:
// //!
// //! * The POWMAN/AON alarm cannot wake DORMANT -- it fires but the comatose ROSC
// //!   never restarts.
// //! * The modem's own PSM self-wake is floored by the network at a 4-hour TAU.
// //!
// //! The PCF8523's countdown timer toggles its INT pin on a schedule we set,
// //! providing exactly the periodic GPIO edge DORMANT needs.
// //!
// //! # This driver ignores the wall clock entirely
// //!
// //! The PCF8523 also keeps a calendar, but we do not use it. Location payloads are
// //! timestamped from GNSS time (decoded server-side), so the device never needs to
// //! know the civil time. This driver therefore touches only the timer, control,
// //! and CLKOUT registers -- never the seconds/minutes/hours calendar. The
// //! oscillator-stop flag (which flags an unseeded clock on every cold boot of a
// //! board with no backup cell) is irrelevant to us and deliberately not checked:
// //! the countdown timer runs off the same 32.768 kHz oscillator and works whether
// //! or not the calendar has ever been set.
// //!
// //! # The INT/SQW pin
// //!
// //! One pin, `INT1/CLKOUT`, shared between interrupt output and square-wave output
// //! (Adafruit label it `SQW`). Using it as an interrupt REQUIRES disabling CLKOUT
// //! -- they are mutually exclusive on that pad. That is also the low-power setting:
// //! 150 nA typical with CLKOUT off vs 1200 nA with 32 kHz CLKOUT on (datasheet
// //! Table, §12). [`new`] disables it.
// //!
// //! Open-drain, active LOW: wire to a spare RP2350 GPIO with a pull-up and wake on
// //! the FALLING edge.
// //!
// //! I2C address `0x68`, fixed.

// use embedded_hal_async::i2c::I2c;

// // ---------------------------------------------------------------------------
// // Parked instance + free-function API
// // ---------------------------------------------------------------------------
// //
// // `modem_task` needs to re-arm the countdown at the end of every cycle ("wake me
// // in N minutes", counted from now) rather than free-run a fixed pulse from
// // startup. Rather than thread the `Pcf8523` through `initiate_modem` and both
// // `modem_task` spawn sites, it lives in a static -- the same pattern as
// // `power::RTC_INT` and the old `psm` primitives.
// //
// // The concrete bus type is fixed here (I2C0, the RTC's bus), which is the price
// // of a static: it cannot stay generic. If the RTC ever moves buses, change this
// // alias and `init`'s argument to match.

// use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
// use embassy_rp::i2c::{Async, I2c as RpI2c};
// use embassy_rp::peripherals::I2C0;
// use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// use embassy_sync::mutex::Mutex;

// /// The RTC's concrete bus device. RTC shares I2C0 with the board's other I2C
// /// devices (the BNO085 is on I2C1).
// pub type RtcI2c = I2cDevice<'static, CriticalSectionRawMutex, RpI2c<'static, I2C0, Async>>;

// static RTC: Mutex<CriticalSectionRawMutex, Option<Pcf8523<RtcI2c>>> = Mutex::new(None);

// /// Hand the initialised RTC to the module. Call once from `startup`, after
// /// [`Pcf8523::new`]. The countdown is left stopped -- `modem_task` arms it
// /// per-cycle via [`arm_wake_minutes`].
// pub async fn init(rtc: Pcf8523<RtcI2c>) {
//     *RTC.lock().await = Some(rtc);
// }

// /// Clear the last wake's interrupt flag, then arm the next wake for `minutes`,
// /// counted from now. Call at the end of each cycle, just before sleeping.
// ///
// /// The clear matters: after a countdown fires, `CTBF` is latched and INT stays
// /// asserted. Re-arming without clearing would leave the pending flag able to
// /// re-trigger the wake immediately. Doing both here keeps the one ordering
// /// constraint in one place.
// ///
// /// Errors are logged and swallowed: an I2C hiccup arming the RTC should not panic
// /// the tracker. The consequence of a missed arm is a missed wake, which the
// /// deferred hardware watchdog (RTC RSTn, or the RP2350 WDT) is the backstop for.
// pub async fn arm_wake_minutes(minutes: u8) {
//     let mut guard = RTC.lock().await;
//     let Some(rtc) = guard.as_mut() else {
//         defmt::error!("rtc::arm_wake_minutes before rtc::init");
//         return;
//     };
//     if let Err(e) = rtc.clear_flag().await {
//         defmt::error!("RTC clear_flag failed: {:?}", e);
//     }
//     if let Err(e) = rtc.set_countdown_minutes(minutes).await {
//         defmt::error!("RTC arm ({} min) failed: {:?}", minutes, e);
//     }
// }

// /// Stop the countdown entirely -- DEEP_REST, where only the BNO085 motion INT can
// /// wake the host. Clears the pending flag first, same reasoning as
// /// [`arm_wake_minutes`].
// pub async fn disarm_wake() {
//     let mut guard = RTC.lock().await;
//     let Some(rtc) = guard.as_mut() else {
//         defmt::error!("rtc::disarm_wake before rtc::init");
//         return;
//     };
//     if let Err(e) = rtc.clear_flag().await {
//         defmt::error!("RTC clear_flag failed: {:?}", e);
//     }
//     if let Err(e) = rtc.stop_countdown().await {
//         defmt::error!("RTC disarm failed: {:?}", e);
//     }
// }

// /// 7-bit I2C address (fixed).
// pub const ADDR: u8 = 0x68;

// // Register map (datasheet Table 6). Only the ones we use.
// mod reg {
//     pub const CONTROL_1: u8 = 0x00;
//     pub const CONTROL_2: u8 = 0x01;
//     pub const CONTROL_3: u8 = 0x02;
//     pub const TMR_CLKOUT_CTRL: u8 = 0x0F;
//     pub const TMR_B_FREQ_CTRL: u8 = 0x12;
//     pub const TMR_B_REG: u8 = 0x13;
// }

// // Tmr_CLKOUT_ctrl bit patterns.
// //
// //   bit7 TAM | bit6 TBM | bit5..3 COF[2:0] | bit2 TAC[1:0] hi | bit1..0 TBC etc.
// //
// // We only ever want: CLKOUT disabled (COF=111), Timer A off, Timer B either off
// // or on with pulsed interrupt. Rather than track individual bits we use two
// // fixed values, both with COF=111.
// mod clkout_ctrl {
//     /// CLKOUT off, both timers OFF. Used at init and to stop Timer B.
//     pub const TIMERS_OFF: u8 = 0b0011_1000;
//     /// CLKOUT off, Timer B ON, pulsed interrupt (TBM=1). Used when armed.
//     pub const TIMER_B_ON_PULSED: u8 = 0b0111_1001;
// }

// #[derive(Debug, defmt::Format)]
// pub struct RtcError<E>(pub E);

// impl<E> From<E> for RtcError<E> {
//     fn from(e: E) -> Self {
//         RtcError(e)
//     }
// }

// pub struct Pcf8523<I2C> {
//     i2c: I2C,
// }

// impl<I2C, E> Pcf8523<I2C>
// where
//     I2C: I2c<Error = E>,
// {
//     /// Construct and initialise as a low-power periodic-wake source.
//     ///
//     /// Software-resets, disables CLKOUT (mandatory for INT use; also the low-power
//     /// setting), disables battery switch-over (no backup cell fitted -- `VBAT`
//     /// must be tied to `VDD` on the board), and clears all interrupt state. Leaves
//     /// the countdown timer stopped; call [`set_countdown_minutes`] to arm it.
//     ///
//     /// Does NOT read or clear the oscillator-stop flag: we never use the calendar,
//     /// and the timer works regardless. The only error path is an I2C failure,
//     /// which means the chip isn't responding at all.
//     pub async fn new(i2c: I2C) -> Result<Self, RtcError<E>> {
//         let mut rtc = Self { i2c };

//         // Software reset: 0x58 -> Control_1 (datasheet §8.3). Puts registers to a
//         // known state (also selects 24h mode, which we don't use but is harmless).
//         rtc.write(reg::CONTROL_1, 0x58).await?;

//         // CLKOUT off so INT1 is usable, and to sit at 150 nA not 1200 nA.
//         rtc.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;

//         // Control_3 PM[2:0] = 111: battery switch-over and battery-low detection
//         // both disabled (no backup cell). Datasheet Table 11.
//         rtc.write(reg::CONTROL_3, 0b1110_0000).await?;

//         // Control_2: all interrupt enables and flags cleared.
//         rtc.write(reg::CONTROL_2, 0x00).await?;

//         Ok(rtc)
//     }

//     /// Arm Timer B to assert INT every `minutes` (1..=255).
//     ///
//     /// Source clock 1/60 Hz, so the register value equals the period in whole
//     /// minutes. On expiry the flag sets, INT pulses low, and the counter
//     /// **auto-reloads** -- no re-arming for a fixed cadence. Safe to call again to
//     /// change the period: the timer is disabled before `T_B` is written (a live
//     /// change can latch a corrupt value, datasheet §8.9.3).
//     ///
//     /// Regime mapping: 15 for ACTIVE_TRACKING, 10 for STATIONARY_PENDING.
//     pub async fn set_countdown_minutes(&mut self, minutes: u8) -> Result<(), RtcError<E>> {
//         debug_assert!(minutes >= 1, "0 stops the timer; use stop_countdown()");

//         // 1. Timer B off before touching T_B.
//         self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;

//         // 2. Source clock = 1/60 Hz (TBQ[2:0] = 011).
//         self.write(reg::TMR_B_FREQ_CTRL, 0b0000_0011).await?;

//         // 3. Period, in minutes.
//         self.write(reg::TMR_B_REG, minutes).await?;

//         // 4. Enable countdown-Timer-B interrupt (CTBIE=1), clearing stale flags.
//         self.write(reg::CONTROL_2, 0b0000_0001).await?;

//         // 5. Timer B on, pulsed interrupt.
//         self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMER_B_ON_PULSED)
//             .await?;

//         Ok(())
//     }

//     /// Stop the countdown entirely -- no periodic wake.
//     ///
//     /// This is DEEP_REST: with the timer off, only the BNO085 significant-motion
//     /// INT (on its own GPIO) can wake the host. Loading `T_B = 0` stops the timer
//     /// (datasheet §8.9.3); we also drop TBC and the interrupt enable so INT is
//     /// released.
//     pub async fn stop_countdown(&mut self) -> Result<(), RtcError<E>> {
//         self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;
//         self.write(reg::TMR_B_REG, 0x00).await?;
//         self.write(reg::CONTROL_2, 0x00).await?;
//         Ok(())
//     }

//     /// Clear the countdown interrupt flag (CTBF). Call once per wake, or INT stays
//     /// asserted and the next dormant entry wakes immediately.
//     ///
//     /// Flags clear on a 0 write and are unchanged on a 1 write (datasheet §8.7.5),
//     /// so write every bit 1 except CTBF (bit 5), and keep CTBIE (bit 0) set.
//     ///
//     ///   WTAF CTAF CTBF SF AF WTAIE CTAIE CTBIE
//     ///    1    1    0   1  1   1     1     1     = 0xDF
//     pub async fn clear_flag(&mut self) -> Result<(), RtcError<E>> {
//         self.write(reg::CONTROL_2, 0b1101_1111).await?;
//         Ok(())
//     }

//     async fn write(&mut self, register: u8, value: u8) -> Result<(), E> {
//         self.i2c.write(ADDR, &[register, value]).await
//     }
// }

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

// ---------------------------------------------------------------------------
// Parked instance + free-function API
// ---------------------------------------------------------------------------
//
// `modem_task` needs to re-arm the countdown at the end of every cycle ("wake me
// in N minutes", counted from now) rather than free-run a fixed pulse from
// startup. Rather than thread the `Pcf8523` through `initiate_modem` and both
// `modem_task` spawn sites, it lives in a static -- the same pattern as
// `power::RTC_INT` and the old `psm` primitives.
//
// The concrete bus type is fixed here (I2C0, the RTC's bus), which is the price
// of a static: it cannot stay generic. If the RTC ever moves buses, change this
// alias and `init`'s argument to match.

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::i2c::{Async, I2c as RpI2c};
use embassy_rp::peripherals::I2C0;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

/// The RTC's concrete bus device. RTC shares I2C0 with the board's other I2C
/// devices (the BNO085 is on I2C1).
pub type RtcI2c = I2cDevice<'static, CriticalSectionRawMutex, RpI2c<'static, I2C0, Async>>;

static RTC: Mutex<CriticalSectionRawMutex, Option<Pcf8523<RtcI2c>>> = Mutex::new(None);

/// Hand the initialised RTC to the module. Call once from `startup`, after
/// [`Pcf8523::new`]. The countdown is left stopped -- `modem_task` arms it
/// per-cycle via [`arm_wake_minutes`].
pub async fn init(rtc: Pcf8523<RtcI2c>) {
    *RTC.lock().await = Some(rtc);
}

/// Clear the last wake's interrupt flag, then arm the next wake for `minutes`,
/// counted from now. Call at the end of each cycle, just before sleeping.
///
/// The clear matters: after a countdown fires, `CTBF` is latched and INT stays
/// asserted. Re-arming without clearing would leave the pending flag able to
/// re-trigger the wake immediately. Doing both here keeps the one ordering
/// constraint in one place.
///
/// Errors are logged and swallowed: an I2C hiccup arming the RTC should not panic
/// the tracker. The consequence of a missed arm is a missed wake, which the
/// deferred hardware watchdog (RTC RSTn, or the RP2350 WDT) is the backstop for.
pub async fn arm_wake_minutes(minutes: u8) {
    let mut guard = RTC.lock().await;
    let Some(rtc) = guard.as_mut() else {
        defmt::error!("rtc::arm_wake_minutes before rtc::init");
        return;
    };
    if let Err(e) = rtc.clear_flag().await {
        defmt::error!("RTC clear_flag failed: {:?}", e);
    }
    if let Err(e) = rtc.set_countdown_minutes(minutes).await {
        defmt::error!("RTC arm ({} min) failed: {:?}", minutes, e);
    }
}

/// Stop the countdown entirely -- DEEP_REST, where only the BNO085 motion INT can
/// wake the host. Clears the pending flag first, same reasoning as
/// [`arm_wake_minutes`].
pub async fn disarm_wake() {
    let mut guard = RTC.lock().await;
    let Some(rtc) = guard.as_mut() else {
        defmt::error!("rtc::disarm_wake before rtc::init");
        return;
    };
    if let Err(e) = rtc.clear_flag().await {
        defmt::error!("RTC clear_flag failed: {:?}", e);
    }
    if let Err(e) = rtc.stop_countdown().await {
        defmt::error!("RTC disarm failed: {:?}", e);
    }
}

/// Arm the wake for approximately `secs`, choosing the coarsest granularity that
/// cannot overshoot.
///
/// Timer B's divider free-runs, so a request of N ticks fires somewhere in
/// `(N-1, N]` ticks. We therefore always arm SHORT of the target and let
/// `power::sleep_for_secs` re-arm for whatever is left:
///
/// * `>= 60 s` -> 1/60 Hz source, `floor(secs / 60)` minutes (never overshoots)
/// * `1..59 s` -> 1 Hz source, `secs` seconds (lands within 1 s)
///
/// Returns the number of seconds actually armed for, or 0 if nothing was armed.
pub async fn arm_wake_secs(secs: u32) -> u32 {
    if secs == 0 {
        return 0;
    }

    let mut guard = RTC.lock().await;
    let Some(rtc) = guard.as_mut() else {
        defmt::error!("rtc::arm_wake_secs before rtc::init");
        return 0;
    };

    if let Err(e) = rtc.clear_flag().await {
        defmt::error!("RTC clear_flag failed: {:?}", e);
    }

    if secs >= 60 {
        let mins = core::cmp::min(secs / 60, 255) as u8;
        if let Err(e) = rtc.set_countdown_minutes(mins).await {
            defmt::error!("RTC arm ({} min) failed: {:?}", mins, e);
            return 0;
        }
        (mins as u32) * 60
    } else {
        let s = secs as u8;
        if let Err(e) = rtc.set_countdown_secs(s).await {
            defmt::error!("RTC arm ({} s) failed: {:?}", s, e);
            return 0;
        }
        secs
    }
}

/// Seconds since midnight from the RTC's free-running calendar, or `None` if the
/// RTC is uninitialised, unreadable, or reports lost oscillator integrity.
///
/// Use with [`elapsed_secs`] to measure how long a sleep actually lasted rather
/// than trusting the countdown period.
pub async fn now_secs_of_day() -> Option<u32> {
    let mut guard = RTC.lock().await;
    let rtc = guard.as_mut()?;
    match rtc.secs_of_day().await {
        Ok(v) => v,
        Err(e) => {
            defmt::error!("RTC secs_of_day failed: {:?}", e);
            None
        }
    }
}

/// Elapsed seconds between two [`now_secs_of_day`] readings, handling the
/// midnight wrap. Only unambiguous for intervals under 24 h, which covers every
/// sleep regime we use.
pub fn elapsed_secs(then: u32, now: u32) -> u32 {
    (now + 86_400 - then) % 86_400
}

/// 7-bit I2C address (fixed).
pub const ADDR: u8 = 0x68;

// Register map (datasheet Table 6). Only the ones we use.
mod reg {
    pub const CONTROL_1: u8 = 0x00;
    pub const CONTROL_2: u8 = 0x01;
    pub const CONTROL_3: u8 = 0x02;
    /// Start of the calendar block: seconds, minutes, hours (0x03..=0x05).
    /// Read-only for us -- we never set the calendar, we only use it as a
    /// free-running 1 Hz elapsed-time reference.
    pub const SECONDS: u8 = 0x03;
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
    /// Clears the oscillator-stop flag and zeroes the seconds register so the
    /// calendar can be used as a free-running 1 Hz elapsed-time reference (see
    /// [`Self::secs_of_day`]). We never set or read wall-clock time from this
    /// part. The only error path is an I2C failure, which means the chip isn't
    /// responding at all.
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

        // Zero the seconds register. This does two things: it clears the
        // oscillator-stop flag (OS, bit 7), which the software reset above leaves
        // set and which would otherwise make every `secs_of_day` read return
        // `None`; and it starts the calendar from a known point so it can be used
        // as a free-running 1 Hz elapsed-time reference. We never set or read
        // wall-clock time from this part, so zeroing it costs nothing.
        rtc.write(reg::SECONDS, 0x00).await?;

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
    /// Arm Timer B to assert INT after `secs` (1..=255), using the 1 Hz source
    /// clock (TBQ = 010) instead of 1/60 Hz.
    ///
    /// Same free-running-divider caveat as [`Self::set_countdown_minutes`], but
    /// one tick is now a second, so the delay lands in `(secs-1, secs]` seconds
    /// rather than `(N-1, N]` minutes. Used for the final approach in
    /// `power::sleep_for_secs`.
    pub async fn set_countdown_secs(&mut self, secs: u8) -> Result<(), RtcError<E>> {
        debug_assert!(secs >= 1, "0 stops the timer; use stop_countdown()");

        // 1. Timer B off before touching T_B.
        self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMERS_OFF).await?;

        // 2. Source clock = 1 Hz (TBQ[2:0] = 010).
        self.write(reg::TMR_B_FREQ_CTRL, 0b0000_0010).await?;

        // 3. Period, in seconds.
        self.write(reg::TMR_B_REG, secs).await?;

        // 4. Enable countdown-Timer-B interrupt (CTBIE=1), clearing stale flags.
        self.write(reg::CONTROL_2, 0b0000_0001).await?;

        // 5. Timer B on, pulsed interrupt.
        self.write(reg::TMR_CLKOUT_CTRL, clkout_ctrl::TIMER_B_ON_PULSED)
            .await?;

        Ok(())
    }

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
    /// Clear the Timer B countdown flag (CTBF, bit 5) without disturbing the
    /// interrupt enables. Writing 0 to a flag bit clears it on this part; the
    /// other flag bits are left set so they are not cleared accidentally, and the
    /// three enable bits (2..0) are left as they were.
    pub async fn clear_flag(&mut self) -> Result<(), RtcError<E>> {
        let mut cur = [0u8; 1];
        self.read_regs(reg::CONTROL_2, &mut cur).await?;
        // Clear only CTBF; preserve enables and the other flags.
        self.write(reg::CONTROL_2, cur[0] & !0b0010_0000).await?;
        Ok(())
    }

    async fn write(&mut self, register: u8, value: u8) -> Result<(), E> {
        self.i2c.write(ADDR, &[register, value]).await
    }

    async fn read_regs(&mut self, register: u8, buf: &mut [u8]) -> Result<(), E> {
        self.i2c.write_read(ADDR, &[register], buf).await
    }

    /// Seconds since midnight, from the free-running calendar.
    ///
    /// The calendar is never set by us, so this is NOT wall-clock time -- it is a
    /// monotonic 1 Hz counter that survives host DORMANT and modem PSM, which is
    /// exactly what we need to measure how long a sleep actually lasted. The
    /// countdown timer's period is only accurate to (N-1, N] minutes because its
    /// 1/60 Hz divider free-runs; measuring real elapsed time sidesteps that.
    ///
    /// Returns `None` if the oscillator-stop flag is set (seconds bit 7), i.e. the
    /// clock lost integrity and the value can't be trusted.
    pub async fn secs_of_day(&mut self) -> Result<Option<u32>, RtcError<E>> {
        // Single burst read: the address auto-increments, so seconds/minutes/hours
        // come from one transaction and can't straddle a tick boundary.
        let mut buf = [0u8; 3];
        self.read_regs(reg::SECONDS, &mut buf).await?;

        // Bit 7 of seconds is OS (oscillator stop), not part of the BCD value.
        if buf[0] & 0x80 != 0 {
            return Ok(None);
        }

        let secs = bcd_to_bin(buf[0] & 0x7F) as u32;
        let mins = bcd_to_bin(buf[1] & 0x7F) as u32;
        let hours = bcd_to_bin(buf[2] & 0x3F) as u32;

        if secs > 59 || mins > 59 || hours > 23 {
            return Ok(None);
        }

        Ok(Some(hours * 3600 + mins * 60 + secs))
    }
}
/// PCF8523 stores calendar values as packed BCD.
fn bcd_to_bin(v: u8) -> u8 {
    (v >> 4) * 10 + (v & 0x0F)
}