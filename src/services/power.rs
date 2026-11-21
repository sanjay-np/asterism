//! The battery, through UPower, and the power profile, through power-profiles-daemon.

use super::Out;
use super::dbus;
use crate::model::{Battery, Command, Update};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
/// Where power-profiles-daemon lives, newest name first.
const PROFILES: [(&str, &str); 2] = [
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

async fn read(bus: &Connection) -> Battery {
    let mut battery = Battery::default();
    if let Some(device) = dbus::get_all(bus, UPOWER, DISPLAY, "org.freedesktop.UPower.Device").await {
        // Type 2 is a battery; a desktop's display device is present but not one
        battery.present = dbus::flag(&device, "IsPresent") && dbus::number(&device, "Type") as u32 == 2;
        battery.level = (dbus::number(&device, "Percentage") / 100.0) as f32;
        let state = dbus::number(&device, "State") as u32;
        battery.charging = matches!(state, 1 | 5);
        battery.full = state == 4;
        battery.time_to_empty = dbus::number(&device, "TimeToEmpty") as i64;
        battery.time_to_full = dbus::number(&device, "TimeToFull") as i64;
    }
    battery.on_battery = dbus::get(bus, UPOWER, "/org/freedesktop/UPower", UPOWER, "OnBattery")
        .await
        .and_then(|v| dbus::as_bool(&v))
        .unwrap_or(false);
    for (name, path) in PROFILES {
        let Some(props) = dbus::get_all(bus, name, path, name).await else { continue };
        battery.profile = dbus::text(&props, "ActiveProfile");
        if let Some(zbus::zvariant::Value::Array(list)) = props.get("Profiles").map(|v| dbus::inner(v)) {
            battery.profiles = list
                .inner()
                .iter()
                .filter_map(|entry| dbus::dict(entry).get("Profile").and_then(|v| dbus::as_str(v)))
                .collect();
        }
        break;
    }
    battery
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) {
    let mut current = read(&bus).await;
    out.send(Update::Battery(current.clone()));
    let mut rules = vec![dbus::rule().sender(UPOWER).and_then(|r| r.member("PropertiesChanged")).map(|r| r.build())];
    for (name, _) in PROFILES {
        rules.push(dbus::rule().sender(name).and_then(|r| r.member("PropertiesChanged")).map(|r| r.build()));
    }
    let mut signals = dbus::watch(&bus, rules).await;
    let mut pending = false;
    // UPower is quiet between changes; a slow look now and then keeps the time left honest
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    loop {
        let settle = tokio::time::sleep(Duration::from_millis(if pending { 150 } else { 3_600_000 }));
        tokio::select! {
            Some(_) = signals.recv() => pending = true,
            _ = ticker.tick() => pending = true,
            Some(command) = commands.recv() => {
                if let Command::SetPowerProfile(profile) = command {
                    for (name, path) in PROFILES {
                        if dbus::get(&bus, name, path, name, "ActiveProfile").await.is_some() {
                            dbus::set(&bus, name, path, name, "ActiveProfile", profile.as_str().into()).await;
                            break;
                        }
                    }
                    pending = true;
                }
            }
            _ = settle, if pending => {
                pending = false;
                let now = read(&bus).await;
                if now != current {
                    current = now;
                    out.send(Update::Battery(current.clone()));
                }
            }
        }
    }
}
