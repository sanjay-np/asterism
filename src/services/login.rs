//! logind, for the lock screen: it asks for the lock when something calls `loginctl
//! lock-session` (the session menu's Lock does), and before the machine sleeps. For the
//! second, the shell holds a "delay" inhibitor: sleep waits until the lock is up, then goes.
//!
//! And for power: sleep, hibernate and shut down when the shell decides to, and the lid and the
//! power button, held from logind by "block" inhibitors while the settings give them something
//! of their own to do (the lid is then watched through UPower). An inhibitor goes with the
//! shell, so if it stops, logind takes them back.

use super::{Out, dbus};
use crate::model::{Command, Update};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;
use zbus::zvariant::OwnedFd;

const LOGIN: &str = "org.freedesktop.login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const UPOWER: &str = "org.freedesktop.UPower";

/// Asks logind to hold sleep back until we let go.
async fn inhibit(bus: &Connection) -> Option<OwnedFd> {
    let reply = bus.call_method(Some(LOGIN), "/org/freedesktop/login1", Some(MANAGER), "Inhibit", &("sleep", "Asterism", "Locking the screen first", "delay")).await;
    match reply {
        Ok(message) => message.body().deserialize::<OwnedFd>().ok(),
        Err(error) => {
            log::warn!("lock: can't hold sleep back to lock first: {error}");
            None
        }
    }
}

/// Holds one of logind's keys ("handle-lid-switch", "handle-power-key") so logind leaves it to us.
async fn hold(bus: &Connection, what: &str, why: &str) -> Option<OwnedFd> {
    let reply = bus.call_method(Some(LOGIN), "/org/freedesktop/login1", Some(MANAGER), "Inhibit", &(what, "Asterism", why, "block")).await;
    match reply {
        Ok(message) => message.body().deserialize::<OwnedFd>().ok(),
        Err(error) => {
            log::warn!("power: logind won't let go of {what}: {error}");
            None
        }
    }
}

/// What logind does with the lid and the button when left to it, and whether it can hibernate.
async fn system_power(bus: &Connection) -> crate::model::SystemPower {
    let word = |name: &'static str| async move { dbus::get(bus, LOGIN, "/org/freedesktop/login1", MANAGER, name).await.and_then(|v| dbus::as_str(&v)).unwrap_or_default() };
    let hibernate = bus
        .call_method(Some(LOGIN), "/org/freedesktop/login1", Some(MANAGER), "CanHibernate", &())
        .await
        .ok()
        .and_then(|m| m.body().deserialize::<String>().ok())
        .is_some_and(|answer| answer == "yes" || answer == "challenge");
    crate::model::SystemPower {
        lid: word("HandleLidSwitch").await,
        lid_plugged: word("HandleLidSwitchExternalPower").await,
        lid_docked: word("HandleLidSwitchDocked").await,
        button: word("HandlePowerKey").await,
        hibernate,
    }
}

/// Asks logind to sleep, hibernate or shut down (false: no questions asked of you; logind
/// still refuses while something blocks it).
async fn power(bus: &Connection, method: &str) {
    if let Err(error) = bus.call_method(Some(LOGIN), "/org/freedesktop/login1", Some(MANAGER), method, &(false,)).await {
        log::warn!("power: logind {method}: {error}");
    }
}

async fn lid_closed(bus: &Connection) -> Option<bool> {
    dbus::get(bus, UPOWER, "/org/freedesktop/UPower", UPOWER, "LidIsClosed").await.and_then(|v| dbus::as_bool(&v))
}

async fn docked(bus: &Connection) -> bool {
    dbus::get(bus, LOGIN, "/org/freedesktop/login1", MANAGER, "Docked").await.and_then(|v| dbus::as_bool(&v)).unwrap_or(false)
}

/// Waits (a little) for the lock screen to say it's up.
async fn lock_up(patience: Duration) {
    let until = tokio::time::Instant::now() + patience;
    while tokio::time::Instant::now() < until && !crate::lock::running() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) {
    let session = super::polkit::session_id(&bus).await;
    let session_path = match &session {
        Some(id) => bus
            .call_method(Some(LOGIN), "/org/freedesktop/login1", Some(MANAGER), "GetSession", &(id.as_str(),))
            .await
            .ok()
            .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedObjectPath>().ok())
            .map(|p| p.to_string()),
        None => None,
    };
    let mut rules = vec![
        dbus::rule().sender(LOGIN).and_then(|r| r.interface(MANAGER)).and_then(|r| r.member("PrepareForSleep")).map(|r| r.build()),
        dbus::rule().sender(UPOWER).and_then(|r| r.path("/org/freedesktop/UPower")).and_then(|r| r.member("PropertiesChanged")).map(|r| r.build()),
    ];
    if let Some(path) = session_path.clone() {
        rules.push(dbus::rule().sender(LOGIN).and_then(|r| r.interface(SESSION)).and_then(|r| r.member("Lock")).and_then(|r| r.path(path)).map(|r| r.build()));
    }
    let mut signals = dbus::watch(&bus, rules).await;
    let mut before_sleep = true;
    let mut held = inhibit(&bus).await;
    // The lid and the button, while the shell does what they do; the lid as last seen
    let (mut lid_held, mut button_held): (Option<OwnedFd>, Option<OwnedFd>) = (None, None);
    let mut lid = lid_closed(&bus).await.unwrap_or(false);
    out.send(Update::SystemPower(system_power(&bus).await));
    log::info!("lock: listening to logind (session {})", session.as_deref().unwrap_or("unknown"));
    loop {
        tokio::select! {
            Some(message) = signals.recv() => {
                let member = message.header().member().map(|m| m.to_string()).unwrap_or_default();
                match member.as_str() {
                    "Lock" => out.send(Update::LockNow),
                    "PropertiesChanged" => {
                        if let Some(closed) = lid_closed(&bus).await
                            && closed != lid
                        {
                            lid = closed;
                            if lid_held.is_some() {
                                out.send(Update::Lid { closed, docked: docked(&bus).await });
                            }
                        }
                    }
                    "PrepareForSleep" => {
                        let going = message.body().deserialize::<bool>().unwrap_or(false);
                        if going {
                            if before_sleep {
                                out.send(Update::LockNow);
                                lock_up(Duration::from_secs(3)).await;
                            }
                            // Now it may sleep
                            held = None;
                        } else if before_sleep {
                            held = inhibit(&bus).await;
                        }
                    }
                    _ => {}
                }
            }
            command = commands.recv() => match command {
                Some(Command::LockBeforeSleep(on)) => {
                    before_sleep = on;
                    if on && held.is_none() {
                        held = inhibit(&bus).await;
                    } else if !on {
                        held = None;
                    }
                }
                Some(Command::Suspend) => power(&bus, "Suspend").await,
                Some(Command::Hibernate) => power(&bus, "Hibernate").await,
                Some(Command::PowerOff) => power(&bus, "PowerOff").await,
                Some(Command::TakePowerKeys { lid: take_lid, button }) => {
                    if take_lid != lid_held.is_some() {
                        lid_held = if take_lid { hold(&bus, "handle-lid-switch", "Asterism does what the lid does").await } else { None };
                        log::info!("power: the lid is {}", if lid_held.is_some() { "ours" } else { "logind's" });
                    }
                    if button != button_held.is_some() {
                        button_held = if button { hold(&bus, "handle-power-key", "Asterism does what the power button does").await } else { None };
                        log::info!("power: the power button is {}", if button_held.is_some() { "ours" } else { "logind's" });
                    }
                }
                Some(_) => {}
                None => return,
            },
        }
        let _ = (&held, &lid_held, &button_held);
    }
}
