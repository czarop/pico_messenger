use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    modem::gnss::{
        commands::{
            self,
            fix::{GnssFix, GnssFixIntervalSecs},
            init::GnssInit,
        },
        state::{GNSSState, GnssInitState},
        urc::{fix::GnssFixUrc, init::GnssInitUrc},
    },
    modem::{
        command_task::ModemCommand,
        communication::{self, COMMAND_CHANNEL, GNSS_COMMAND, GNSS_RESULT},
        urc::ModemUrc,
    },
};
use defmt::{error, info, warn};
use embassy_futures::select::Either;

pub enum GnssCommand {
    Start(GnssInit, Option<GnssFixIntervalSecs>),
    Stop,
    SetFrequency(GnssFixIntervalSecs),
}

#[embassy_executor::task]
pub async fn gnss_task(
    mut modem_urc_channel: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) -> ! {
    let state_sender = communication::GNSS_STATE.sender();
    state_sender.send(GNSSState::Off);
    let mut fix_interval: Option<GnssFixIntervalSecs> = None;
    let incoming_commands = &GNSS_COMMAND;
    let mut state_watcher = communication::GNSS_STATE.receiver().unwrap();

    // monitor for incoming traffic from the modem
    loop {
        match embassy_futures::select::select(
            modem_urc_channel.next_message_pure(),
            incoming_commands.wait(),
        )
        .await
        {
            Either::First(urc) => match urc {
                ModemUrc::GnssStatus(raw_status) => {
                    let status = GnssInitUrc::from(raw_status);
                    match status {
                        GnssInitUrc::NotStarted => {
                            state_sender.send(GNSSState::Off);
                        }
                        GnssInitUrc::Starting => {
                            info!("GNSS is starting up");
                            state_sender.send(GNSSState::Initialising(GnssInitState::Starting));
                        }
                        GnssInitUrc::Ready { .. } => {
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
                            state_sender.send(GNSSState::Acquiring);
                        }
                        GnssFixUrc::Fix(gnss_location) => {
                            state_sender.send(GNSSState::Fix(gnss_location));
                        }
                    },
                    Err(e) => {
                        error!("Failed to parse GNSS location URC: {:?}", e);
                    }
                },
                _ => {}
            },
            Either::Second(cmd) => match cmd {
                GnssCommand::Start(params, interval_secs) => {
                    fix_interval = interval_secs;
                    if state_watcher.try_get() != Some(GNSSState::Off) {
                        warn!(
                            "Received GNSS start command while not in Off state. Command ignored."
                        );
                        continue;
                    } else {
                        for i in 0..10 {
                            COMMAND_CHANNEL
                                .send(ModemCommand::GnssInit(params.clone()))
                                .await;
                            match GNSS_RESULT.wait().await {
                                Ok(_) => {
                                    info!("GNSS init command sent successfully");
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
        }
    }
}
