//! The network, through NetworkManager: what you're connected by, and the wifi networks
//! around when the menu wants them.

use super::Out;
use super::dbus;
use crate::model::{AccessPoint, Command, Network, Update};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";

fn kind(nm_type: &str) -> &'static str {
    match nm_type {
        "802-11-wireless" => "wifi",
        "802-3-ethernet" => "ethernet",
        "vpn" | "wireguard" => "vpn",
        "bluetooth" => "bluetooth",
        "gsm" | "cdma" => "mobile",
        _ => "other",
    }
}

async fn wifi_devices(bus: &Connection) -> Vec<String> {
    let mut found = Vec::new();
    let Some(props) = dbus::get_all(bus, NM, NM_PATH, NM).await else { return found };
    for device in props.get("Devices").map(|v| dbus::as_strings(v)).unwrap_or_default() {
        // DeviceType 2 is wifi
        if let Some(kind) = dbus::get(bus, NM, &device, "org.freedesktop.NetworkManager.Device", "DeviceType").await
            && dbus::as_f64(&kind) == Some(2.0)
        {
            found.push(device);
        }
    }
    found
}

/// The names of the wifi networks NetworkManager has saved.
async fn known_ssids(bus: &Connection) -> Vec<String> {
    let mut known = Vec::new();
    let settings = "org.freedesktop.NetworkManager.Settings";
    let Ok(reply) = bus.call_method(Some(NM), "/org/freedesktop/NetworkManager/Settings", Some(settings), "ListConnections", &()).await else { return known };
    let Ok(paths) = reply.body().deserialize::<Vec<zbus::zvariant::OwnedObjectPath>>() else { return known };
    for path in paths {
        let Ok(reply) = bus.call_method(Some(NM), path.as_str(), Some("org.freedesktop.NetworkManager.Settings.Connection"), "GetSettings", &()).await else { continue };
        let Ok(config) = reply.body().deserialize::<std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>>>() else { continue };
        if let Some(ssid) = config.get("802-11-wireless").and_then(|w| w.get("ssid")).map(|v| dbus::as_bytes(v)) {
            known.push(String::from_utf8_lossy(&ssid).to_string());
        }
    }
    known
}

/// The saved VPNs: name, settings path and uuid. Read now and then, not on every change (each
/// takes a call per saved connection).
async fn saved_vpns(bus: &Connection) -> Vec<(String, String, String)> {
    let mut vpns = Vec::new();
    let settings = "org.freedesktop.NetworkManager.Settings";
    let Ok(reply) = bus.call_method(Some(NM), "/org/freedesktop/NetworkManager/Settings", Some(settings), "ListConnections", &()).await else { return vpns };
    let Ok(paths) = reply.body().deserialize::<Vec<zbus::zvariant::OwnedObjectPath>>() else { return vpns };
    for path in paths {
        let Ok(reply) = bus.call_method(Some(NM), path.as_str(), Some("org.freedesktop.NetworkManager.Settings.Connection"), "GetSettings", &()).await else { continue };
        let Ok(config) = reply.body().deserialize::<std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>>>() else { continue };
        let Some(connection) = config.get("connection") else { continue };
        let text = |key: &str| connection.get(key).and_then(|v| dbus::as_str(v)).unwrap_or_default();
        if kind(&text("type")) == "vpn" {
            vpns.push((text("id"), path.to_string(), text("uuid")));
        }
    }
    vpns.sort_by_key(|v| v.0.to_lowercase());
    vpns
}

/// RSN/WPA key management bits, as NetworkManager reports them.
const KEY_MGMT_802_1X: u32 = 0x200;
const KEY_MGMT_SAE: u32 = 0x400;

async fn read(bus: &Connection, with_aps: bool, vpns: &[(String, String, String)]) -> Network {
    let Some(nm) = dbus::get_all(bus, NM, NM_PATH, NM).await else { return Network::default() };
    let mut network = Network { available: true, wifi_enabled: dbus::flag(&nm, "WirelessEnabled"), ..Default::default() };

    let primary = dbus::text(&nm, "PrimaryConnection");
    let actives = nm.get("ActiveConnections").map(|v| dbus::as_strings(v)).unwrap_or_default();
    let mut connections = Vec::new();
    for path in &actives {
        if let Some(props) = dbus::get_all(bus, NM, path, ACTIVE).await {
            connections.push((path.clone(), props));
        }
    }
    // The saved VPNs, and which of them are up (2) or coming up (1)
    for (name, path, uuid) in vpns {
        let state = connections.iter().find(|(_, p)| dbus::text(p, "Uuid") == *uuid).map_or(0, |(_, p)| dbus::number(p, "State") as u32);
        network.vpns.push(crate::model::Vpn { name: name.clone(), path: path.clone(), active: state == 2, connecting: state == 1 });
    }
    // The primary connection names it, but wifi or a cable underneath decide the icon
    let primary_props = connections.iter().find(|(p, _)| *p == primary).map(|(_, props)| props);
    if let Some(props) = primary_props {
        network.name = dbus::text(props, "Id");
        network.kind = kind(&dbus::text(props, "Type")).to_string();
        let ip4 = dbus::text(props, "Ip4Config");
        if ip4.len() > 1
            && let Some(config) = dbus::get(bus, NM, &ip4, "org.freedesktop.NetworkManager.IP4Config", "AddressData").await
            && let zbus::zvariant::Value::Array(list) = dbus::inner(&config)
        {
            network.address = list.inner().first().and_then(|a| dbus::dict(a).get("address").and_then(|v| dbus::as_str(v))).unwrap_or_default();
        }
    }
    let carrier = connections.iter().find(|(_, p)| kind(&dbus::text(p, "Type")) == "wifi").or_else(|| connections.iter().find(|(_, p)| kind(&dbus::text(p, "Type")) == "ethernet"));
    if let Some((_, props)) = carrier {
        let carrier_kind = kind(&dbus::text(props, "Type"));
        if network.kind.is_empty() || network.kind == "vpn" || network.kind == "other" {
            network.kind = carrier_kind.to_string();
            if network.name.is_empty() {
                network.name = dbus::text(props, "Id");
            }
        }
        if carrier_kind == "wifi" {
            let ap = dbus::text(props, "SpecificObject");
            if let Some(ap) = dbus::get_all(bus, NM, &ap, AP).await {
                network.strength = dbus::number(&ap, "Strength") as u8;
                let ssid = String::from_utf8_lossy(&ap.get("Ssid").map(|v| dbus::as_bytes(v)).unwrap_or_default()).to_string();
                if !ssid.is_empty() {
                    network.name = ssid;
                }
            }
        }
    }

    if with_aps && network.wifi_enabled {
        let active_ssid = if network.kind == "wifi" { network.name.clone() } else { String::new() };
        let known = known_ssids(bus).await;
        for device in wifi_devices(bus).await {
            let points = dbus::get(bus, NM, &device, WIRELESS, "AccessPoints").await.map(|v| dbus::as_strings(&v)).unwrap_or_default();
            for point in points {
                let Some(ap) = dbus::get_all(bus, NM, &point, AP).await else { continue };
                let ssid = String::from_utf8_lossy(&ap.get("Ssid").map(|v| dbus::as_bytes(v)).unwrap_or_default()).to_string();
                if ssid.is_empty() {
                    continue;
                }
                let strength = dbus::number(&ap, "Strength") as u8;
                let (wpa, rsn) = (dbus::number(&ap, "WpaFlags") as u32, dbus::number(&ap, "RsnFlags") as u32);
                let secure = wpa != 0 || rsn != 0 || dbus::number(&ap, "Flags") as u32 & 1 != 0;
                let enterprise = (wpa | rsn) & KEY_MGMT_802_1X != 0;
                // One entry per name, the strongest
                if let Some(seen) = network.access_points.iter_mut().find(|a| a.ssid == ssid) {
                    seen.strength = seen.strength.max(strength);
                } else {
                    let known = known.contains(&ssid);
                    network.access_points.push(AccessPoint { active: ssid == active_ssid, ssid, strength, secure, known, enterprise });
                }
            }
        }
        network.access_points.sort_by(|a, b| b.active.cmp(&a.active).then(b.strength.cmp(&a.strength)));
        network.access_points.truncate(12);
    }
    network
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) {
    let mut wants_aps_until: Option<Instant> = None;
    let mut vpns = saved_vpns(&bus).await;
    let mut vpns_read = Instant::now();
    let mut current = read(&bus, false, &vpns).await;
    out.send(Update::Network(current.clone()));
    // Only NetworkManager's own object: access points report every wobble in strength,
    // and the ticker below follows that closely enough
    let rules = vec![dbus::rule().sender(NM).and_then(|r| r.path(NM_PATH)).map(|r| r.build())];
    let mut signals = dbus::watch(&bus, rules).await;
    let mut pending = false;
    // Signal strength drifts without much said, a look every so often follows it
    let mut ticker = tokio::time::interval(Duration::from_secs(10));
    loop {
        let settle = tokio::time::sleep(Duration::from_millis(if pending { 300 } else { 3_600_000 }));
        tokio::select! {
            Some(_) = signals.recv() => pending = true,
            _ = ticker.tick() => pending = true,
            Some(command) = commands.recv() => {
                match command {
                    Command::SetWifi(on) => dbus::set(&bus, NM, NM_PATH, NM, "WirelessEnabled", on.into()).await,
                    Command::ScanWifi => {
                        wants_aps_until = Some(Instant::now() + Duration::from_secs(60));
                        for device in wifi_devices(&bus).await {
                            let options: std::collections::HashMap<&str, zbus::zvariant::Value> = std::collections::HashMap::new();
                            dbus::call(&bus, NM, &device, WIRELESS, "RequestScan", &(options,)).await;
                        }
                    }
                    Command::ConnectWifiWith(ssid, password) => {
                        let (bus, out) = (bus.clone(), out.clone());
                        tokio::spawn(async move {
                            let (ok, why) = join(&bus, &ssid, password).await;
                            out.send(Update::WifiJoined(ssid, ok, why));
                        });
                    }
                    Command::SetVpn(path, on) => {
                        if on {
                            let root = zbus::zvariant::ObjectPath::try_from("/").expect("path");
                            if let Ok(connection) = zbus::zvariant::ObjectPath::try_from(path.as_str()) {
                                dbus::call(&bus, NM, NM_PATH, NM, "ActivateConnection", &(connection, root.clone(), root)).await;
                            }
                        } else {
                            // Down: the active connection made from it
                            for active in dbus::get(&bus, NM, NM_PATH, NM, "ActiveConnections").await.map(|v| dbus::as_strings(&v)).unwrap_or_default() {
                                if dbus::get(&bus, NM, &active, ACTIVE, "Connection").await.is_some_and(|c| dbus::as_str(&c).is_some_and(|c| c == path))
                                    && let Ok(active) = zbus::zvariant::ObjectPath::try_from(active.as_str())
                                {
                                    dbus::call(&bus, NM, NM_PATH, NM, "DeactivateConnection", &(active,)).await;
                                }
                            }
                        }
                    }
                    Command::ConnectWifi(ssid) => {
                        // Known networks and open ones connect straight away; others need a password, which nmcli asks for elsewhere
                        let _ = tokio::process::Command::new("nmcli").args(["device", "wifi", "connect", &ssid]).stdin(std::process::Stdio::null()).status().await;
                    }
                    _ => {}
                }
                pending = true;
            }
            _ = settle, if pending => {
                pending = false;
                let with_aps = wants_aps_until.is_some_and(|until| Instant::now() < until);
                if vpns_read.elapsed() > Duration::from_secs(60) {
                    vpns = saved_vpns(&bus).await;
                    vpns_read = Instant::now();
                }
                let now = read(&bus, with_aps, &vpns).await;
                if now != current {
                    current = now;
                    out.send(Update::Network(current.clone()));
                }
            }
        }
    }
}

/// The access point with this name on some wifi device: which device, which point, and its
/// key management bits.
async fn find_point(bus: &Connection, ssid: &str) -> Option<(String, String, u32)> {
    let mut best: Option<(String, String, u32, u8)> = None;
    for device in wifi_devices(bus).await {
        let points = dbus::get(bus, NM, &device, WIRELESS, "AccessPoints").await.map(|v| dbus::as_strings(&v)).unwrap_or_default();
        for point in points {
            let Some(ap) = dbus::get_all(bus, NM, &point, AP).await else { continue };
            let name = String::from_utf8_lossy(&ap.get("Ssid").map(|v| dbus::as_bytes(v)).unwrap_or_default()).to_string();
            let strength = dbus::number(&ap, "Strength") as u8;
            if name == ssid && best.as_ref().is_none_or(|b| strength > b.3) {
                let flags = dbus::number(&ap, "WpaFlags") as u32 | dbus::number(&ap, "RsnFlags") as u32;
                best = Some((device.clone(), point.clone(), flags, strength));
            }
        }
    }
    best.map(|(d, p, f, _)| (d, p, f))
}

/// Joins a network with a password through NetworkManager (never on a command line), and
/// waits to see whether it takes. A connection that doesn't work is deleted again, so a wrong
/// password isn't kept.
async fn join(bus: &Connection, ssid: &str, password: crate::model::Secret) -> (bool, String) {
    use zbus::zvariant::Value;
    use std::collections::HashMap;
    let Some((device, point, flags)) = find_point(bus, ssid).await else { return (false, "That network isn't in reach any more.".into()) };
    if flags & KEY_MGMT_802_1X != 0 {
        return (false, "This network needs more than a password (802.1X): use your network settings.".into());
    }
    // Secured, but by neither WPA nor RSN: the old WEP, which nothing should trust any more
    if flags == 0 {
        return (false, "This network uses old WEP security, which isn't supported.".into());
    }
    // WPA3-only networks take SAE; everything else with a password, WPA-PSK
    let key_mgmt = if flags & KEY_MGMT_SAE != 0 && flags & 0x100 == 0 { "sae" } else { "wpa-psk" };
    let mut psk = String::from_utf8_lossy(password.bytes()).into_owned();
    drop(password);
    let mut connection: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
    connection.insert("connection", HashMap::from([("id", Value::from(ssid)), ("type", Value::from("802-11-wireless"))]));
    connection.insert("802-11-wireless", HashMap::from([("ssid", Value::from(ssid.as_bytes().to_vec())), ("mode", Value::from("infrastructure"))]));
    connection.insert("802-11-wireless-security", HashMap::from([("key-mgmt", Value::from(key_mgmt)), ("psk", Value::from(psk.as_str()))]));
    let device_path = zbus::zvariant::ObjectPath::try_from(device.as_str()).ok();
    let point_path = zbus::zvariant::ObjectPath::try_from(point.as_str()).ok();
    let (Some(device_path), Some(point_path)) = (device_path, point_path) else { return (false, "NetworkManager didn't answer.".into()) };
    let reply = bus.call_method(Some(NM), NM_PATH, Some(NM), "AddAndActivateConnection", &(connection, device_path, point_path)).await;
    // The password is in NetworkManager's hands now; ours goes
    // SAFETY: zeroes bytes in place; zero bytes are valid UTF-8
    unsafe { psk.as_bytes_mut().fill(0) };
    let (saved, active) = match reply.and_then(|r| r.body().deserialize::<(zbus::zvariant::OwnedObjectPath, zbus::zvariant::OwnedObjectPath)>()) {
        Ok(paths) => paths,
        Err(error) => return (false, format!("NetworkManager said no: {error}")),
    };
    // Activated (2), or it falls back to deactivated (4) / goes away: wrong password, most often
    let until = Instant::now() + Duration::from_secs(45);
    while Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(500)).await;
        match dbus::get(bus, NM, active.as_str(), ACTIVE, "State").await.and_then(|v| dbus::as_f64(&v)) {
            Some(2.0) => return (true, String::new()),
            Some(state) if state >= 3.0 => break,
            None => break,
            _ => {}
        }
    }
    dbus::call(bus, NM, saved.as_str(), "org.freedesktop.NetworkManager.Settings.Connection", "Delete", &()).await;
    (false, "That password didn't work.".into())
}
