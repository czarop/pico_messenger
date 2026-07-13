use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::modem::{UpdateIntervalSecs, command_task::ModemCommand, communication::{self, COMMAND_CHANNEL, GNSS_COMMAND, GNSS_RESULT}, error::ModemError, gnss::{
         commands::{
            self,
            fix::GnssFix,
            init::GnssInit,
        }, state::{GNSSState, GnssInitState}, urc::{fix::GnssFixUrc, init::GnssInitUrc}
    }, urc::ModemUrc};
use defmt::{error, info, warn};
use embassy_futures::select::{Either3};
use embassy_rp::gpio::Output;
use embassy_time::{Duration, Timer};

pub enum GnssCommand {
    Start(GnssInit, Option<UpdateIntervalSecs>),
    Stop,
    SetFrequency(UpdateIntervalSecs),
}

#[embassy_executor::task]
pub async fn gnss_task(
    mut modem_urc_channel: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
    mut gnss_bias: Output<'static>,
) -> ! {
    defmt::info!("gnss task spawned");
    let state_sender = communication::GNSS_STATE.sender();
    state_sender.send(GNSSState::Off);
    #[cfg(not(feature = "mock_gnss"))]
    {
        defmt::info!("clearing any stale GNSS engine from a previous run");
        COMMAND_CHANNEL.send(ModemCommand::GnssDeinit).await;
        let _ = communication::GNSS_RESULT.wait().await;
    }

    let mut fix_interval: Option<UpdateIntervalSecs> = None;
    let incoming_commands = &GNSS_COMMAND;
    let mut state_watcher = communication::GNSS_STATE.receiver().unwrap();

    // monitor for incoming traffic from the modem
    loop {
        let timeout = match state_watcher.try_get() {
            Some(GNSSState::Initialising(_)) => Timer::after(Duration::from_secs(300)),
            _ => Timer::after(Duration::from_secs(3600)),
        };

        match embassy_futures::select::select3(
            modem_urc_channel.next_message_pure(),
            incoming_commands.wait(),
            timeout
        )
        .await
        {
            Either3::First(urc) => {
            info!("urc received: {}", urc);
            match urc {
                
                ModemUrc::GnssStatus(raw_status) => {
                    let status = GnssInitUrc::from(raw_status);
                    match status {
                        GnssInitUrc::NotStarted => {
                            defmt::info!("gnss off");
                            state_sender.send(GNSSState::Off);
                        }
                        GnssInitUrc::Starting => {
                            info!("GNSS is starting up");
                            state_sender.send(GNSSState::Initialising(GnssInitState::Starting));
                        }
                        GnssInitUrc::Ready { .. } => {
                            info!("GNSS is ready");
                            state_sender.send(GNSSState::Ready);
                            for i in 0..10 {
                                COMMAND_CHANNEL
                                    .send(ModemCommand::GetLocation(
                                        commands::fix::GnssFix::start_with_at_report_defaults(
                                            fix_interval,
                                        ),
                                    ))
                                    .await;
                                match GNSS_RESULT.wait().await {
                                    Ok(_) => {
                                        info!("GNSS fix initiated");
                                        fix_interval = None;
                                        break;
                                    }
                                    Err(e) => {
                                        if i < 9 {
                                            error!("GNSS fix initiation failed: {:?}", e);
                                            embassy_time::Timer::after(
                                                embassy_time::Duration::from_millis(500),
                                            )
                                            .await;
                                        } else {
                                            error!(
                                                "Failed to send GNSS fix command after 10 attempts"
                                            );
                                            state_sender
                                                .send(GNSSState::Error(GnssInitUrc::SystemFailure));
                                        }
                                    }
                                }
                            }
                        }
                        GnssInitUrc::DownloadingSupl => {
                            info!("GNSS is downloading SUPL data");
                            state_sender
                                .send(GNSSState::Initialising(GnssInitState::DownloadingSupl));
                        }
                        GnssInitUrc::SuplFailed | GnssInitUrc::SystemFailure => {
                            info!("GNSS SUPL failed");
                            state_sender.send(GNSSState::Error(status));
                        }
                        GnssInitUrc::StartupDelayed { nbiot_delay } => {
                            match nbiot_delay {
                                Some(d) => info!(
                                    "GNSS startup is delayed by {} ms due to NB-IoT activity",
                                    d
                                ),
                                None => info!(
                                    "GNSS startup is delayed due to NB-IoT activity, but no delay duration provided"
                                ),
                            }
                            state_sender
                                .send(GNSSState::Initialising(GnssInitState::Delayed(nbiot_delay)));
                        }
                    }
                }
                ModemUrc::Location(loc) => match GnssFixUrc::try_from_raw(loc) {
                    Ok(fix_validity) => match fix_validity {
                        GnssFixUrc::Searching(_) => {
                            info!("GNSS is acquiring");
                            state_sender.send(GNSSState::Acquiring);
                        }
                        GnssFixUrc::Fix(gnss_location) => {
                            info!("GNSS has a fix");
                            state_sender.send(GNSSState::Fix(gnss_location));
                        }
                    },
                    Err(e) => {
                        error!("Failed to parse GNSS location URC: {:?}", e);
                    }
                },
                ModemUrc::RebootHost | ModemUrc::RebootReset | ModemUrc::RebootWD(..) | ModemUrc::SysStart => {
                    gnss_bias.set_low();
                    state_sender.send(GNSSState::Off);
                    fix_interval = None;
                }
                _ => {}
            }},
            Either3::Second(cmd) => match cmd {
                GnssCommand::Start(params, interval_secs) => {
                    fix_interval = interval_secs;
                    if state_watcher.try_get() != Some(GNSSState::Off) {
                        warn!(
                            "Received GNSS start command while not in Off state. Command ignored."
                        );
                        continue;
                    } else {
                        info!("GNSS start called");
                        gnss_bias.set_high();
                        for i in 0..10 {
                            COMMAND_CHANNEL
                                .send(ModemCommand::GnssInit(params.clone()))
                                .await;
                            match GNSS_RESULT.wait().await {
                                Ok(_) => {
                                    info!("GNSS init command sent successfully");
                                    state_sender.send(GNSSState::Initialising(GnssInitState::Starting));
                                    break;
                                }
                                Err(ModemError::Modem(atat::Error::CmeError(_))) => {
                                    // GNSS already running — treat as success
                                    // info!("GNSS already initialised");
                                    // break;
                                    // GNSS already running (retained from previous session).
                                    // No #GNSSINIT: 2 URC will come, so start fixes directly.
                                    info!("GNSS already initialised, starting fixes directly");
                                    state_sender.send(GNSSState::Ready);
                                    let interval = fix_interval.take();
                                    for j in 0..10 {
                                        COMMAND_CHANNEL
                                            .send(ModemCommand::GetLocation(
                                                GnssFix::start_with_at_report_defaults(interval),
                                            ))
                                            .await;
                                        match GNSS_RESULT.wait().await {
                                            Ok(_) => {
                                                info!("GNSS fix initiated");
                                                break;
                                            }
                                            Err(e) => {
                                                if j < 9 {
                                                    error!("GNSS fix init failed: {:?}", e);
                                                    embassy_time::Timer::after(
                                                        embassy_time::Duration::from_millis(500),
                                                    ).await;
                                                } else {
                                                    error!("Failed to start fixes after 10 attempts");
                                                    state_sender.send(GNSSState::Error(GnssInitUrc::SystemFailure));
                                                }
                                            }
                                        }
                                    }
                                    break;
                                }
                                Err(e) => {
                                    if i < 9 {
                                        error!("GNSS init command failed: {:?}", e);
                                        embassy_time::Timer::after(
                                            embassy_time::Duration::from_millis(500),
                                        )
                                        .await;
                                    } else {
                                        error!(
                                            "Failed to send GNSS init command after 10 attempts"
                                        );
                                        state_sender
                                            .send(GNSSState::Error(GnssInitUrc::SystemFailure));
                                    }
                                }
                            }
                        }
                    }
                }
                GnssCommand::Stop => {
                    if state_watcher.try_get() == Some(GNSSState::Off) {
                        warn!("Received GNSS stop command while in Off state. Command ignored.");
                        continue;
                    } else {
                        for i in 0..10 {
                            COMMAND_CHANNEL.send(ModemCommand::GnssDeinit).await;
                            match GNSS_RESULT.wait().await {
                                Ok(_) => {
                                    info!("GNSS deinit command sent successfully");
                                    state_sender.send(GNSSState::Off);
                                    gnss_bias.set_low();
                                    break;
                                }
                                Err(e) => {
                                    if i < 9 {
                                        error!("GNSS deinit command failed: {:?}", e);
                                        embassy_time::Timer::after(
                                            embassy_time::Duration::from_millis(500),
                                        )
                                        .await;
                                    } else {
                                        error!(
                                            "Failed to send GNSS deinit command after 10 attempts"
                                        );
                                        state_sender
                                            .send(GNSSState::Error(GnssInitUrc::SystemFailure));
                                    }
                                }
                            }
                        }
                    }
                }
                GnssCommand::SetFrequency(frequency) => {
                    fix_interval = Some(frequency);
                    if matches!(
                        state_watcher.try_get(),
                        Some(GNSSState::Acquiring) | Some(GNSSState::Fix(_))
                    ) {
                        info!(
                            "Received GNSS set frequency command. Setting frequency to {}",
                            frequency
                        );
                        let mut success = false;
                        for i in 0..10 {
                            COMMAND_CHANNEL
                                .send(ModemCommand::GetLocation(GnssFix::stop_updates()))
                                .await;
                            match GNSS_RESULT.wait().await {
                                Ok(_) => {
                                    info!("GNSS stopped");
                                    success = true;
                                    break;
                                }
                                Err(e) => {
                                    if i < 9 {
                                        error!("GNSS stop failed: {:?}", e);
                                        embassy_time::Timer::after(
                                            embassy_time::Duration::from_millis(500),
                                        )
                                        .await;
                                    } else {
                                        error!(
                                            "Failed to send GNSS deinit command after 10 attempts"
                                        );
                                        state_sender
                                            .send(GNSSState::Error(GnssInitUrc::SystemFailure));
                                    }
                                }
                            }
                        }
                        if success {
                            for i in 0..10 {
                                COMMAND_CHANNEL
                                    .send(ModemCommand::GetLocation(
                                        commands::fix::GnssFix::start_with_at_report_defaults(
                                            Some(frequency),
                                        ),
                                    ))
                                    .await;
                                match GNSS_RESULT.wait().await {
                                    Ok(_) => {
                                        info!("GNSS fix initiated");
                                        break;
                                    }
                                    Err(e) => {
                                        if i < 9 {
                                            error!("GNSS fix initiation failed: {:?}", e);
                                            embassy_time::Timer::after(
                                                embassy_time::Duration::from_millis(500),
                                            )
                                            .await;
                                        } else {
                                            error!(
                                                "Failed to send GNSS fix command after 10 attempts"
                                            );
                                            state_sender
                                                .send(GNSSState::Error(GnssInitUrc::SystemFailure));
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        warn!("Received GNSS set frequency in wrong state. Command ignored.");
                    }
                }
            },
            Either3::Third(_) => {
                warn!("Timed out waiting for GNSS ready URC, resetting");
                gnss_bias.set_low();
                fix_interval = None;
                state_sender.send(GNSSState::Off);
            }
        }
    }
}
