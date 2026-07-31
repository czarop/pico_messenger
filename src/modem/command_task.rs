use super::setup::{INGRESS_BUF_SIZE, URC_CAPACITY, URC_SUBSCRIBERS};
use crate::modem::communication::{self, COMMAND_CHANNEL};
use crate::modem::error::ModemError;
use crate::modem::gnss::commands::{deinit::GnssDeinit, fix::GnssFix, init::GnssInit};
use crate::modem::mqtt::commands::{cereg, cesq, clock, config, connect, pdn, publish, reset, socket, subscribe};
use crate::modem::psm::{
    AtPing, EnterSleep, WAKE_PING_ATTEMPTS, WAKE_PING_INTERVAL_MS, WAKE_PULSE_MS, WAKE_SETTLE_MS,
};
use crate::modem::urc::ModemUrc;
use atat::asynch::AtatClient;
use atat::asynch::Client;
use embassy_rp::gpio::Output;
use embassy_rp::uart;
use embassy_time::{Duration, Timer, with_timeout};

/// How long to wait for the `#SLEEP` URC after `AT#SLEEPMODE` returns `OK`.
const SLEEP_URC_TIMEOUT: Duration = Duration::from_secs(45);

pub enum ModemCommand {
    GnssInit(GnssInit),
    GnssDeinit,
    GetLocation(GnssFix),
    MqttConfig(config::MqttConfig),
    SocketCreate(socket::SocketCreate),
    SocketClose(socket::SocketClose),
    SocketQuery(socket::SocketQuery),
    ClockQuery(clock::CclkQuery),
    ModemReset(reset::ModemReset),
    MqttConnect(connect::MqttConnect),
    MqttDisconnect(connect::MqttDisconnect),
    MqttPublish(publish::MqttPublish),
    MqttSubscribe(subscribe::MqttSubscribe),
    MqttUnsubscribe(subscribe::MqttUnsubscribe),
    GetPdpAddress(pdn::CgPaddrQuery),
    EnterPsm,
    ExitPsm,
    CeregQuery(cereg::CeregQuery),
    CesqQuery(cesq::CesqQuery),
    /// One-shot network diagnostic: CFUN?/CEREG?/COPS? dumped to defmt.
    /// Fired manually (gated by a const in `network_task`); see `run_net_diag`.
    NetDiag,
}

/// Sole owner of the atat client and of the modem wake pin -- the true modem
/// boundary. Every byte to the modem, and the only host->modem GPIO, originate
/// here.
///
/// `wake_pin` is RP2350 GPIO10 -> ST87M01 `WAKE_UP` (pin 39), per the Challenger+
/// RP2350 NB-IoT datasheet. It is **active low** (`AT#WAKEUPEVENT` `<pwrkey_evt>`
/// bit1 = 1) with an internal pull-up, so it idles high.
///
/// `urc_sub` exists solely so that [`ModemCommand::EnterPsm`] can await the
/// `#SLEEP` confirmation without routing it through another task. It also picks
/// up `#ENERGY`, which is the cheapest per-cycle power telemetry available before
/// a meter is on the board.
#[embassy_executor::task]
pub async fn command_task(
    client: &'static mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>,
    mut wake_pin: Output<'static>,
    mut urc_sub: atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) -> ! {
    defmt::info!("command task spawned");

    // The modem is awake at boot; hold the wake line at its idle level.
    wake_pin.set_high();

    loop {
        let cmd = COMMAND_CHANNEL.receive().await;
        match cmd {
            ModemCommand::GnssInit(g) => match client.send(&g).await {
                Ok(_) => communication::GNSS_RESULT.signal(Ok(())),
                Err(e) => communication::GNSS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::GnssDeinit => match client.send(&GnssDeinit {}).await {
                Ok(_) => communication::GNSS_RESULT.signal(Ok(())),
                Err(e) => communication::GNSS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::GetLocation(f) => match client.send(&f).await {
                Ok(_) => communication::GNSS_RESULT.signal(Ok(())),
                Err(e) => communication::GNSS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttConfig(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketCreate(s) => match client.send(&s).await {
                Ok(r) => communication::SOCKET_RESULT.signal(Ok(r.socket_id)),
                Err(e) => communication::SOCKET_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketClose(s) => match client.send(&s).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::SocketQuery(q) => match client.send(&q).await {
                Ok(r) => communication::SOCKET_QUERY_RESULT.signal(Ok(r)),
                Err(e) => communication::SOCKET_QUERY_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::ModemReset(r) => {
                // Single chokepoint for all AT#RESET=0 escalations (wedged
                // socket, stuck-searching, connect-retry) -- count here so the
                // diag `rst` field reflects every reset regardless of caller.
                crate::modem::diag::note_reset();
                match client.send(&r).await {
                    Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                    Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                    #[allow(unreachable_patterns)]
                    _ => unreachable!(),
                }
            }
            ModemCommand::ClockQuery(q) => match client.send(&q).await {
                Ok(r) => communication::CLOCK_RESULT.signal(Ok(r)),
                Err(e) => communication::CLOCK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttConnect(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttDisconnect(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttPublish(m) => match client.send(&m).await {
                Ok(_) => communication::PUBLISH_RESULT.signal(Ok(())),
                Err(e) => communication::PUBLISH_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttSubscribe(s) => match client.send(&s).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::MqttUnsubscribe(m) => match client.send(&m).await {
                Ok(_) => communication::NETWORK_RESULT.signal(Ok(())),
                Err(e) => communication::NETWORK_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::GetPdpAddress(q) => match client.send(&q).await {
                Ok(r) => communication::PDP_ADDRESS_RESULT.signal(Ok(r.is_active())),
                Err(e) => communication::PDP_ADDRESS_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },

            ModemCommand::EnterPsm => {
                let result = enter_psm(client, &mut urc_sub).await;
                communication::PSM_RESULT.signal(result);
            }
            ModemCommand::ExitPsm => {
                let result = exit_psm(client, &mut urc_sub, &mut wake_pin).await;
                communication::PSM_RESULT.signal(result);
            }
            ModemCommand::CeregQuery(q) => match client.send(&q).await {
                Ok(r) => communication::CEREG_RESULT.signal(Ok(r)),
                Err(e) => communication::CEREG_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::CesqQuery(q) => match client.send(&q).await {
                Ok(r) => communication::CESQ_RESULT.signal(Ok(r)),
                Err(e) => communication::CESQ_RESULT.signal(Err(ModemError::from(e))),
                #[allow(unreachable_patterns)]
                _ => unreachable!(),
            },
            ModemCommand::NetDiag => {
                run_net_diag(client, &mut urc_sub).await;
                communication::DIAG_RESULT.signal(());
            }
        }
    }
}

/// One-shot network diagnostic. Answers, from the log alone: is the radio on
/// (CFUN?), are we registered and how (CEREG), who did we land on (COPS?), and
/// and who did we land on (COPS?).
///
/// `+CFUN`/`+COPS` responses return to `send()` and are logged verbatim.
/// `+CEREG` is a URC token, so `AT+CEREG?` is fired only to provoke a fresh URC,
/// which is then read from the subscription (its reply never reaches `send()`).
///
/// Gated behind a const at the call site so it does not run every cycle.
async fn run_net_diag(
    client: &mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>,
    urc_sub: &mut atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) {
    use crate::modem::mqtt::commands::netdiag;

    defmt::info!("=== NET DIAG START ===");

    // Radio power state.
    match client.send(&netdiag::CfunQuery).await {
        Ok(r) => defmt::info!("CFUN? -> {=str}", r.text.as_str()),
        Err(e) => defmt::warn!("CFUN? failed: {:?}", ModemError::from(e)),
    }

    // Registration. The reply to AT+CEREG? is classified as a URC and routed to
    // the URC channel, so send() will not return it (it typically parse-errors on
    // an empty body). Clear the sub, fire the query to force a fresh +CEREG URC,
    // give the ingress task a moment to deliver it, then drain and log.
    drain_urcs(urc_sub);
    let _ = client.send(&cereg::CeregQuery).await; // reply lands as a URC, ignore
    Timer::after(Duration::from_millis(200)).await;
    let mut saw_cereg = false;
    while let Some(urc) = urc_sub.try_next_message_pure() {
        if let ModemUrc::Cereg(body) = urc {
            defmt::info!("CEREG -> {=str}", body.as_str());
            saw_cereg = true;
        }
    }
    if !saw_cereg {
        defmt::info!("CEREG -> (no URC seen)");
    }

    // Currently registered operator (meaningful only once registered).
    match client.send(&netdiag::CopsRead).await {
        Ok(r) => defmt::info!("COPS? -> {=str}", r.text.as_str()),
        Err(e) => defmt::warn!("COPS? failed: {:?}", ModemError::from(e)),
    }

    // NOTE: AT+COPS=? (full PLMN scan) is deliberately NOT issued. On this modem
    // it only ever returns +CME ERROR: 22 while camped/searching and, per
    // hardware notes, briefly detaches the modem -- which in the bring-up path
    // costs a multi-minute NB-IoT re-attach for zero information. CEREG + COPS?
    // already give the state we need.

    defmt::info!("=== NET DIAG END ===");
}

/// Discard any URCs queued since we last looked.
///
/// Our subscription is a broadcast cursor that nobody advances between PSM
/// transitions, so by the time we get here it is pointing at whatever arrived
/// during the last publish cycle. Without this, the `#SLEEP` wait below would
/// immediately chew through stale `#GNSSFIX`/`+CEREG` traffic, and worse, could
/// match a `#SLEEP` left over from the *previous* cycle.
fn drain_urcs(sub: &mut atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>) {
    while sub.try_next_message_pure().is_some() {}
}

/// `AT#SLEEPMODE` (bare) -> wait for `#SLEEP`.
///
/// Always the real thing. Mocking happens one level up, in `psm::enter_psm`,
/// which short-circuits before the command ever reaches `COMMAND_CHANNEL`.
async fn enter_psm(
    client: &mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>,
    urc_sub: &mut atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
) -> Result<(), ModemError> {
    {
        drain_urcs(urc_sub);

        // `OK` means "command accepted", not "modem asleep".
        client.send(&EnterSleep).await.map_err(ModemError::from)?;

        let confirmed = async {
            loop {
                match urc_sub.next_message_pure().await {
                    ModemUrc::Sleep => break,
                    // Consumption since the previous report, in uWh. Free
                    // per-cycle power telemetry -- log it.
                    ModemUrc::Energy(e) => defmt::info!("modem energy since last: {} uWh", e.as_str()),
                    _ => {}
                }
            }
        };

        match with_timeout(SLEEP_URC_TIMEOUT, confirmed).await {
            Ok(()) => {
                defmt::info!("modem entered PSM");
                Ok(())
            }
            Err(_) => {
                // The modem acknowledged but never announced sleep. Do not let
                // the caller put the RP2350 to sleep on the strength of an `OK`.
                defmt::warn!("no #SLEEP URC after AT#SLEEPMODE");
                Err(ModemError::Timeout)
            }
        }
    }
}

/// Pulse GPIO10 low, then ping until the modem's UART answers.
///
/// We confirm with a bare `AT` rather than by waiting for the `#WAKEUP` URC: a
/// successful `AT`/`OK` round trip proves the UART is actually serviceable, which
/// is what the next command needs. `#WAKEUP` only proves the modem left PSM. Both
/// are logged.
///
/// Ping failures during wake are expected, not exceptional -- the modem is
/// mid-resume and may not answer for tens of milliseconds. See
/// [`WAKE_PING_ATTEMPTS`].
async fn exit_psm(
    client: &mut Client<'static, uart::BufferedUartTx, INGRESS_BUF_SIZE>,
    urc_sub: &mut atat::UrcSubscription<'static, ModemUrc, URC_CAPACITY, URC_SUBSCRIBERS>,
    wake_pin: &mut Output<'static>,
) -> Result<(), ModemError> {
    {
        drain_urcs(urc_sub);

        // Active low, per AT#WAKEUPEVENT <pwrkey_evt> bit1.
        wake_pin.set_low();
        Timer::after(Duration::from_millis(WAKE_PULSE_MS)).await;
        wake_pin.set_high();
        Timer::after(Duration::from_millis(WAKE_SETTLE_MS)).await;

        for attempt in 1..=WAKE_PING_ATTEMPTS {
            if client.send(&AtPing).await.is_ok() {
                defmt::info!("modem awake after {} ping(s)", attempt);
                return Ok(());
            }
            Timer::after(Duration::from_millis(WAKE_PING_INTERVAL_MS)).await;
        }

        defmt::error!("modem unresponsive after {} pings", WAKE_PING_ATTEMPTS);
        Err(ModemError::Timeout)
    }
}