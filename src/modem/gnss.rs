use defmt::{error, info, warn};
use embassy_futures::select::Either;

use super::setup::{URC_CAPACITY, URC_SUBSCRIBERS};
use crate::{
    gnss::{commands::fix::GnssFix, state::GNSSState, urc::fix::GnssFixUrc},
    modem::{
        command,
        communication::{self, COMMAND_CHANNEL, GNSS_COMMAND, MQTT_STATE},
        urc::ModemUrc,
    },
};

pub enum GnssCommand {
    Start,
    Stop,
    SetFrequency(u32),
}

#[embassy_executor::task]
pub async fn network_task(
    mut modem_urc_channel: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) -> ! {
    let state_sender = communication::GNSS_STATE.sender();
    state_sender.send(GNSSState::Off);

    let incoming_commands = &GNSS_COMMAND;
    let mut state_watcher = communication::GNSS_STATE.receiver().unwrap();
    'outer: loop {
        // monitor for incoming traffic from the modem
        loop {
            match embassy_futures::select::select(
                modem_urc_channel.next_message_pure(),
                incoming_commands.wait(),
            )
            .await
            {
                Either::First(urc) => match urc {
                    ModemUrc::GnssStatus(status) => {}
                    ModemUrc::Location(loc) => match GnssFixUrc::try_from_raw(loc) {
                        Ok(fix_validity) => match fix_validity {
                            GnssFixUrc::Searching(gnss_validity) => todo!(),
                            GnssFixUrc::Fix(gnss_location) => todo!(),
                        },
                        Err(e) => todo!(),
                    },
                    _ => {}
                },
                Either::Second(cmd) => match cmd {
                    GnssCommand::Start => todo!(),
                    GnssCommand::Stop => todo!(),
                    GnssCommand::SetFrequency(f) => todo!(),
                },
            }
        }
    }
}
