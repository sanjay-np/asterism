//! logind, for the lock screen: it asks for the lock when something calls `loginctl
//! lock-session` (the session menu's Lock does), and before the machine sleeps. For the
//! second, the shell holds a "delay" inhibitor: sleep waits until the lock is up, then goes.

use super::{Out, dbus};
use crate::model::{Command, Update};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;
use zbus::zvariant::OwnedFd;

const LOGIN: &str = "org.freedesktop.login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";

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
    let mut rules = vec![dbus::rule().sender(LOGIN).and_then(|r| r.interface(MANAGER)).and_then(|r| r.member("PrepareForSleep")).map(|r| r.build())];
    if let Some(path) = session_path.clone() {
        rules.push(dbus::rule().sender(LOGIN).and_then(|r| r.interface(SESSION)).and_then(|r| r.member("Lock")).and_then(|r| r.path(path)).map(|r| r.build()));
    }
    let mut signals = dbus::watch(&bus, rules).await;
    let mut before_sleep = true;
    let mut held = inhibit(&bus).await;
    log::info!("lock: listening to logind (session {})", session.as_deref().unwrap_or("unknown"));
    loop {
        tokio::select! {
            Some(message) = signals.recv() => {
                let member = message.header().member().map(|m| m.to_string()).unwrap_or_default();
                match member.as_str() {
                    "Lock" => out.send(Update::LockNow),
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
                Some(_) => {}
                None => return,
            },
        }
        let _ = &held;
    }
}
