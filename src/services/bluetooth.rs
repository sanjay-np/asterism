//! Bluetooth, through BlueZ's object manager: the adapter, your devices, others nearby while
//! looking, and pairing. For pairing the shell is BlueZ's agent: when a device wants a code
//! confirmed, typed or shown, the question goes to a card and the answer comes back here.

use super::Out;
use super::dbus;
use crate::model::{Bluetooth, BluetoothDevice, Command, PairAsk, Pairing, Update};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const BLUEZ: &str = "org.bluez";
const DEVICE: &str = "org.bluez.Device1";
const ADAPTER: &str = "org.bluez.Adapter1";
const AGENT_PATH: &str = "/dev/asterism/bluetooth_agent";
/// How long a look for devices lasts.
const LOOKING: Duration = Duration::from_secs(30);

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// A name a person gave, not an address standing in for one.
fn real_name(name: &str) -> bool {
    !name.is_empty() && !(name.len() == 17 && name.chars().filter(|c| *c == ':' || *c == '-').count() == 5)
}

async fn read(bus: &Connection) -> Bluetooth {
    let Ok(reply) = bus.call_method(Some(BLUEZ), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &()).await else {
        return Bluetooth::default();
    };
    let Ok(objects) = reply.body().deserialize::<Objects>() else { return Bluetooth::default() };
    let mut bluetooth = Bluetooth::default();
    let mut paths: Vec<_> = objects.keys().collect();
    paths.sort_by_key(|p| p.as_str().to_string());
    for path in paths {
        let interfaces = &objects[path];
        if let Some(adapter) = interfaces.get(ADAPTER)
            && !bluetooth.available
        {
            bluetooth.available = true;
            bluetooth.adapter = path.to_string();
            bluetooth.powered = dbus::flag(adapter, "Powered");
            bluetooth.discovering = dbus::flag(adapter, "Discovering");
        }
        if let Some(device) = interfaces.get(DEVICE) {
            let mut name = dbus::text(device, "Alias");
            if name.is_empty() {
                name = dbus::text(device, "Name");
            }
            let paired = dbus::flag(device, "Paired");
            let connected = dbus::flag(device, "Connected");
            let rssi = device.get("RSSI").and_then(|v| dbus::as_f64(v)).map(|r| r as i16);
            let entry = BluetoothDevice {
                path: path.to_string(),
                name,
                icon: dbus::text(device, "Icon"),
                connected,
                paired,
                battery: interfaces.get("org.bluez.Battery1").map_or(-1, |b| dbus::number(b, "Percentage") as i32),
                rssi,
            };
            if paired || connected {
                bluetooth.devices.push(entry);
            } else if rssi.is_some() && real_name(&entry.name) {
                // Strangers only while looking, and only with a name
                bluetooth.nearby.push(entry);
            }
        }
    }
    bluetooth.devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
    bluetooth.nearby.sort_by_key(|d| std::cmp::Reverse(d.rssi));
    bluetooth.nearby.truncate(8);
    bluetooth
}

// ---------------------------------------------------------------- the agent

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum Refused {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

/// Waits for the person's answer to the question shown.
type Waiting = Arc<Mutex<Option<oneshot::Sender<Option<String>>>>>;

struct Agent {
    out: Out,
    bus: Connection,
    waiting: Waiting,
}

impl Agent {
    async fn name(&self, device: &ObjectPath<'_>) -> String {
        let alias = dbus::get(&self.bus, BLUEZ, device.as_str(), DEVICE, "Alias").await.and_then(|v| dbus::as_str(&v)).unwrap_or_default();
        if alias.is_empty() { "A device".into() } else { alias }
    }

    /// Shows the question and waits (a minute at most) for the answer; None is no.
    async fn ask(&self, device: &ObjectPath<'_>, ask: PairAsk) -> Option<String> {
        let device = self.name(device).await;
        let (tell, answer) = oneshot::channel();
        *self.waiting.lock().unwrap() = Some(tell);
        self.out.send(Update::Pairing(Some(Pairing { device, ask })));
        let answer = tokio::time::timeout(Duration::from_secs(60), answer).await.ok().and_then(|a| a.ok()).flatten();
        self.out.send(Update::Pairing(None));
        answer
    }

    /// Shows something to type on the device; nothing to answer.
    async fn show(&self, device: &ObjectPath<'_>, ask: PairAsk) {
        let device = self.name(device).await;
        self.out.send(Update::Pairing(Some(Pairing { device, ask })));
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    async fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, Refused> {
        self.ask(&device, PairAsk::Pin).await.filter(|p| !p.is_empty()).ok_or_else(|| Refused::Rejected("no PIN".into()))
    }

    async fn display_pin_code(&self, device: OwnedObjectPath, pincode: String) {
        self.show(&device, PairAsk::ShowPin(pincode)).await;
    }

    async fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, Refused> {
        self.ask(&device, PairAsk::Passkey).await.and_then(|p| p.trim().parse().ok()).ok_or_else(|| Refused::Rejected("no passkey".into()))
    }

    async fn display_passkey(&self, device: OwnedObjectPath, passkey: u32, _entered: u16) {
        self.show(&device, PairAsk::ShowPasskey(passkey)).await;
    }

    async fn request_confirmation(&self, device: OwnedObjectPath, passkey: u32) -> Result<(), Refused> {
        self.ask(&device, PairAsk::Confirm(passkey)).await.map(|_| ()).ok_or_else(|| Refused::Rejected("not confirmed".into()))
    }

    async fn request_authorization(&self, device: OwnedObjectPath) -> Result<(), Refused> {
        self.ask(&device, PairAsk::Authorize).await.map(|_| ()).ok_or_else(|| Refused::Rejected("not allowed".into()))
    }

    async fn authorize_service(&self, device: OwnedObjectPath, uuid: String) -> Result<(), Refused> {
        // Your own, trusted devices may use what they like
        let trusted = dbus::get(&self.bus, BLUEZ, device.as_str(), DEVICE, "Trusted").await.and_then(|v| dbus::as_bool(&v)).unwrap_or(false);
        if trusted {
            return Ok(());
        }
        self.ask(&device, PairAsk::Service(uuid)).await.map(|_| ()).ok_or_else(|| Refused::Rejected("not allowed".into()))
    }

    fn cancel(&self) {
        // The device gave up: so does the question
        self.waiting.lock().unwrap().take();
        self.out.send(Update::Pairing(None));
    }
}

/// Tells BlueZ we answer pairing questions, by default.
async fn register(bus: &Connection) {
    let manager = "org.bluez.AgentManager1";
    let Ok(path) = ObjectPath::try_from(AGENT_PATH) else { return };
    match bus.call_method(Some(BLUEZ), "/org/bluez", Some(manager), "RegisterAgent", &(&path, "KeyboardDisplay")).await {
        Ok(_) => {
            dbus::call(bus, BLUEZ, "/org/bluez", manager, "RequestDefaultAgent", &(&path,)).await;
            log::info!("bluetooth: the shell answers pairing questions");
        }
        Err(error) => log::debug!("bluetooth: no agent ({error})"),
    }
}

/// Pairs, then trusts and connects; says how it went.
async fn pair(out: &Out, bus: &Connection, path: &str) {
    let name = dbus::get(bus, BLUEZ, path, DEVICE, "Alias").await.and_then(|v| dbus::as_str(&v)).unwrap_or_else(|| "the device".into());
    let result = bus.call_method(Some(BLUEZ), path, Some(DEVICE), "Pair", &()).await;
    let paired = match &result {
        Ok(_) => true,
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == "org.bluez.Error.AlreadyExists" => true,
        Err(_) => false,
    };
    if !paired {
        let why = match result {
            Err(zbus::Error::MethodError(_, Some(detail), _)) => detail,
            Err(error) => error.to_string(),
            Ok(_) => String::new(),
        };
        out.send(Update::Notify(crate::model::Note { summary: format!("Couldn't pair with {name}"), body: why, icon: "bluetooth".into(), urgency: 1 }));
        return;
    }
    dbus::set(bus, BLUEZ, path, DEVICE, "Trusted", true.into()).await;
    dbus::call(bus, BLUEZ, path, DEVICE, "Connect", &()).await;
    out.send(Update::Notify(crate::model::Note { summary: format!("Paired with {name}"), icon: "bluetooth".into(), ..Default::default() }));
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>, agent: bool) {
    let waiting: Waiting = Arc::new(Mutex::new(None));
    if agent && bus.object_server().at(AGENT_PATH, Agent { out: out.clone(), bus: bus.clone(), waiting: waiting.clone() }).await.is_ok() {
        register(&bus).await;
    }
    let mut current = read(&bus).await;
    out.send(Update::Bluetooth(current.clone()));
    let rules = vec![
        dbus::rule().sender(BLUEZ).map(|r| r.build()),
        // BlueZ coming or going
        dbus::rule().sender("org.freedesktop.DBus").and_then(|r| r.member("NameOwnerChanged")).and_then(|r| r.arg(0, BLUEZ)).map(|r| r.build()),
    ];
    let mut signals = dbus::watch(&bus, rules).await;
    let mut pending = false;
    loop {
        let settle = tokio::time::sleep(Duration::from_millis(if pending { 200 } else { 3_600_000 }));
        tokio::select! {
            Some(message) = signals.recv() => {
                // BlueZ (re)started: it forgot its agent
                if agent
                    && message.header().member().is_some_and(|m| m.as_str() == "NameOwnerChanged")
                    && message.body().deserialize::<(String, String, String)>().is_ok_and(|(_, _, new)| !new.is_empty())
                {
                    register(&bus).await;
                }
                pending = true;
            }
            Some(command) = commands.recv() => match command {
                Command::SetBluetoothPower(on) => {
                    if !current.adapter.is_empty() {
                        dbus::set(&bus, BLUEZ, &current.adapter, ADAPTER, "Powered", on.into()).await;
                    }
                }
                Command::BluetoothConnect(path, connect) => {
                    // Connecting can take a while: don't hold everything else up
                    let bus = bus.clone();
                    tokio::spawn(async move {
                        let method = if connect { "Connect" } else { "Disconnect" };
                        dbus::call(&bus, BLUEZ, &path, DEVICE, method, &()).await;
                    });
                }
                Command::ScanBluetooth => {
                    if current.powered && !current.discovering && !current.adapter.is_empty() {
                        let (bus, adapter) = (bus.clone(), current.adapter.clone());
                        tokio::spawn(async move {
                            dbus::call(&bus, BLUEZ, &adapter, ADAPTER, "StartDiscovery", &()).await;
                            tokio::time::sleep(LOOKING).await;
                            dbus::call(&bus, BLUEZ, &adapter, ADAPTER, "StopDiscovery", &()).await;
                        });
                    }
                }
                Command::BluetoothPair(path) => {
                    let (out, bus) = (out.clone(), bus.clone());
                    tokio::spawn(async move { pair(&out, &bus, &path).await });
                }
                Command::BluetoothForget(path) => {
                    if let Ok(device) = ObjectPath::try_from(path.as_str()) {
                        dbus::call(&bus, BLUEZ, &current.adapter, ADAPTER, "RemoveDevice", &(&device,)).await;
                    }
                }
                Command::PairAnswer(answer) => {
                    if let Some(tell) = waiting.lock().unwrap().take() {
                        let _ = tell.send(answer);
                    }
                }
                _ => {}
            },
            _ = settle, if pending => {
                pending = false;
                let now = read(&bus).await;
                if now != current {
                    current = now;
                    out.send(Update::Bluetooth(current.clone()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The agent over a real bus (the session one, on a private name): a yes goes back as a
    /// reply, a no as BlueZ's Rejected error, a passkey as a number.
    #[tokio::test]
    async fn answers_pairing_questions() {
        let (Ok(server), Ok(client)) = (Connection::session().await, Connection::session().await) else { return };
        let (updates, _seen) = smithay_client_toolkit::reexports::calloop::channel::channel();
        let waiting: Waiting = Arc::new(Mutex::new(None));
        server.object_server().at(AGENT_PATH, Agent { out: Out(updates), bus: server.clone(), waiting: waiting.clone() }).await.unwrap();
        let to = server.unique_name().unwrap().to_owned();
        let device = ObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap();
        // Answers, as the card would, once the question is up
        let answer = |reply: Option<&'static str>| {
            let waiting = waiting.clone();
            tokio::spawn(async move {
                loop {
                    if let Some(tell) = waiting.lock().unwrap().take() {
                        let _ = tell.send(reply.map(str::to_string));
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
        };
        answer(Some(""));
        assert!(client.call_method(Some(to.clone()), AGENT_PATH, Some("org.bluez.Agent1"), "RequestConfirmation", &(&device, 123_456u32)).await.is_ok());
        answer(None);
        match client.call_method(Some(to.clone()), AGENT_PATH, Some("org.bluez.Agent1"), "RequestConfirmation", &(&device, 123_456u32)).await {
            Err(zbus::Error::MethodError(name, ..)) => assert_eq!(name.as_str(), "org.bluez.Error.Rejected"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        answer(Some("482913"));
        let reply = client.call_method(Some(to), AGENT_PATH, Some("org.bluez.Agent1"), "RequestPasskey", &(&device,)).await.unwrap();
        assert_eq!(reply.body().deserialize::<u32>().unwrap(), 482_913);
    }

    #[test]
    fn tells_names_from_addresses() {
        assert!(super::real_name("Pixel Buds"));
        assert!(!super::real_name("A4:C1:38:0F:22:9B"));
        assert!(!super::real_name("A4-C1-38-0F-22-9B"));
        assert!(!super::real_name(""));
    }
}
