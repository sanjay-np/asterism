//! The apps running in the background, through StatusNotifierItem. Apps register with a
//! watcher; when no other bar is running, the bar is that watcher, otherwise it listens to the
//! one there is. Menus come over DBusMenu.

use super::Out;
use super::dbus::{self, Props};
use crate::model::{Command, MenuEntry, TrayItem, Update};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
/// Icons are drawn at about 20 pixels; this leaves room for scaled screens.
const ICON_SIZE: u32 = 48;

enum WatcherEvent {
    Added(String),
}

struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
    events: UnboundedSender<WatcherEvent>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        // Apps give either their bus name or, for libappindicator, an object path on their own connection
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let id = if service.starts_with('/') { format!("{sender}{service}") } else { format!("{service}/StatusNotifierItem") };
        let added = {
            let mut items = self.items.lock().unwrap();
            if items.contains(&id) {
                false
            } else {
                items.push(id.clone());
                true
            }
        };
        if added {
            let _ = Self::status_notifier_item_registered(&emitter, &id).await;
            let _ = self.registered_status_notifier_items_changed(&emitter).await;
            let _ = self.events.send(WatcherEvent::Added(id));
        }
    }

    async fn register_status_notifier_host(&self, _service: &str, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// "bus/path" → ("bus", "/path").
fn split(id: &str) -> (&str, &str) {
    match id.find('/') {
        Some(at) => (&id[..at], &id[at..]),
        None => (id, "/StatusNotifierItem"),
    }
}

#[derive(serde::Deserialize, zbus::zvariant::Type)]
struct Layout {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<OwnedValue>,
}

/// "_Quit" → "Quit", "Save __as" → "Save _as".
fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn entry(id: i32, props: &HashMap<String, &Value<'_>>, children: Vec<MenuEntry>) -> Option<MenuEntry> {
    let text = |key: &str| props.get(key).and_then(|v| dbus::as_str(v)).unwrap_or_default();
    let flag = |key: &str, default: bool| props.get(key).and_then(|v| dbus::as_bool(v)).unwrap_or(default);
    if !flag("visible", true) {
        return None;
    }
    Some(MenuEntry {
        id,
        label: strip_mnemonic(&text("label")),
        enabled: flag("enabled", true),
        separator: text("type") == "separator",
        toggle: text("toggle-type"),
        checked: props.get("toggle-state").and_then(|v| dbus::as_f64(v)).unwrap_or(0.0) == 1.0,
        children,
    })
}

fn parse_child(value: &Value<'_>) -> Option<MenuEntry> {
    let Value::Structure(structure) = dbus::inner(value) else { return None };
    let fields = structure.fields();
    let id = fields.first().and_then(|v| dbus::as_f64(v))? as i32;
    let props = fields.get(1).map(dbus::dict).unwrap_or_default();
    let children = match fields.get(2).map(dbus::inner) {
        Some(Value::Array(array)) => array.inner().iter().filter_map(parse_child).collect(),
        _ => Vec::new(),
    };
    entry(id, &props, children)
}

struct Tray {
    bus: Connection,
    out: Out,
    order: Vec<String>,
    items: HashMap<String, TrayItem>,
    /// item id → (unique bus name, menu path)
    owners: HashMap<String, (String, String)>,
    /// Items whose menu is being shown, refreshed when it changes.
    open_menus: HashSet<String>,
}

impl Tray {
    async fn owner(&self, bus_name: &str) -> String {
        if bus_name.starts_with(':') {
            return bus_name.to_string();
        }
        let Ok(proxy) = zbus::fdo::DBusProxy::new(&self.bus).await else { return bus_name.to_string() };
        match zbus::names::BusName::try_from(bus_name) {
            Ok(name) => proxy.get_name_owner(name).await.map(|o| o.to_string()).unwrap_or_else(|_| bus_name.to_string()),
            Err(_) => bus_name.to_string(),
        }
    }

    async fn icon(props: &Props, attention: bool) -> Option<Arc<crate::model::Image>> {
        let theme_path = dbus::text(props, "IconThemePath");
        let (name_key, pixmap_key) = if attention { ("AttentionIconName", "AttentionIconPixmap") } else { ("IconName", "IconPixmap") };
        let name = dbus::text(props, name_key);
        if !name.is_empty() {
            let found = tokio::task::spawn_blocking(move || {
                let extra = (!theme_path.is_empty()).then(|| std::path::PathBuf::from(&theme_path));
                crate::images::find_icon(&name, ICON_SIZE, extra.as_deref()).and_then(|path| crate::images::load(&path, ICON_SIZE))
            })
            .await
            .ok()
            .flatten();
            if found.is_some() {
                return found;
            }
        }
        // a(iiay): several sizes, take the smallest that's big enough
        let Some(Value::Array(pixmaps)) = props.get(pixmap_key).map(|v| dbus::inner(v)) else { return None };
        let mut best: Option<(i32, i32, Vec<u8>)> = None;
        for pixmap in pixmaps.inner() {
            let Value::Structure(s) = dbus::inner(pixmap) else { continue };
            let fields = s.fields();
            let (Some(w), Some(h)) = (fields.first().and_then(|v| dbus::as_f64(v)), fields.get(1).and_then(|v| dbus::as_f64(v))) else { continue };
            let bytes = fields.get(2).map(dbus::as_bytes).unwrap_or_default();
            let (w, h) = (w as i32, h as i32);
            let better = match &best {
                None => true,
                Some((bw, _, _)) => (*bw < ICON_SIZE as i32 && w > *bw) || (w >= ICON_SIZE as i32 && w < *bw),
            };
            if better {
                best = Some((w, h, bytes));
            }
        }
        let (w, h, bytes) = best?;
        crate::images::from_argb(w as u32, h as u32, &bytes)
    }

    async fn fetch(&mut self, id: &str) {
        let (bus_name, path) = split(id);
        let Some(props) = dbus::get_all(&self.bus, bus_name, path, ITEM).await else {
            // Some items only speak the freedesktop name of the interface
            if let Some(props) = dbus::get_all(&self.bus, bus_name, path, "org.freedesktop.StatusNotifierItem").await {
                return self.store(id, props).await;
            }
            log::debug!("tray: {id} doesn't answer");
            return;
        };
        self.store(id, props).await;
    }

    async fn store(&mut self, id: &str, props: Props) {
        let (bus_name, _) = split(id);
        let status = dbus::text(&props, "Status");
        let attention = status == "NeedsAttention";
        let mut icon = Self::icon(&props, attention).await;
        if icon.is_none() && attention {
            icon = Self::icon(&props, false).await;
        }
        let mut title = dbus::text(&props, "Title");
        if title.is_empty() {
            title = dbus::text(&props, "Id");
        }
        let menu = dbus::text(&props, "Menu");
        let owner = self.owner(bus_name).await;
        self.owners.insert(id.to_string(), (owner, menu.clone()));
        self.items.insert(
            id.to_string(),
            TrayItem {
                id: id.to_string(),
                title,
                status,
                icon,
                item_is_menu: dbus::flag(&props, "ItemIsMenu"),
                has_menu: menu.len() > 1,
            },
        );
        if !self.order.iter().any(|o| o == id) {
            self.order.push(id.to_string());
        }
    }

    fn remove(&mut self, id: &str) {
        self.items.remove(id);
        self.owners.remove(id);
        self.order.retain(|o| o != id);
        self.open_menus.remove(id);
    }

    fn publish(&self) {
        let items = self
            .order
            .iter()
            .filter_map(|id| self.items.get(id))
            .filter(|item| item.status != "Passive")
            .cloned()
            .collect();
        self.out.send(Update::Tray(items));
    }

    async fn menu(&mut self, id: &str) {
        let Some((owner, path)) = self.owners.get(id).cloned() else { return };
        if path.len() <= 1 {
            return;
        }
        let _ = self.bus.call_method(Some(owner.as_str()), path.as_str(), Some(MENU), "AboutToShow", &(0i32,)).await;
        let reply = self.bus.call_method(Some(owner.as_str()), path.as_str(), Some(MENU), "GetLayout", &(0i32, -1i32, Vec::<String>::new())).await;
        let entries = match reply {
            Ok(reply) => match reply.body().deserialize::<(u32, Layout)>() {
                Ok((_, layout)) => {
                    let _ = (layout.id, &layout.props);
                    layout.children.iter().filter_map(|c| parse_child(c)).collect()
                }
                Err(error) => {
                    log::warn!("tray menu of {id}: {error}");
                    Vec::new()
                }
            },
            Err(error) => {
                log::warn!("tray menu of {id}: {error}");
                Vec::new()
            }
        };
        self.open_menus.insert(id.to_string());
        self.out.send(Update::TrayMenu(id.to_string(), entries));
    }

    async fn command(&mut self, command: Command) {
        let call = |id: &str, method: &'static str, x: i32, y: i32| {
            let (bus_name, path) = split(id);
            let (bus_name, path) = (bus_name.to_string(), path.to_string());
            let bus = self.bus.clone();
            // Apps can take their time answering; the bar doesn't wait
            tokio::spawn(async move { dbus::call(&bus, &bus_name, &path, ITEM, method, &(x, y)).await });
        };
        match command {
            Command::TrayActivate(id, x, y) => call(&id, "Activate", x, y),
            Command::TraySecondary(id, x, y) => call(&id, "SecondaryActivate", x, y),
            Command::TrayScroll(id, delta) => {
                let (bus_name, path) = split(&id);
                dbus::call(&self.bus, bus_name, path, ITEM, "Scroll", &(delta, "vertical")).await;
            }
            Command::TrayMenu(id) => self.menu(&id).await,
            Command::TrayMenuClick(id, entry) => {
                if let Some((owner, path)) = self.owners.get(&id).cloned() {
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
                    dbus::call(&self.bus, &owner, &path, MENU, "Event", &(entry, "clicked", Value::I32(0), stamp)).await;
                }
                self.open_menus.remove(&id);
            }
            _ => {}
        }
    }
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) {
    let (events, mut watcher_events) = unbounded_channel();
    let registered = Arc::new(Mutex::new(Vec::<String>::new()));
    let watcher = Watcher { items: registered.clone(), events };
    if let Err(error) = bus.object_server().at(WATCHER_PATH, watcher).await {
        log::warn!("tray: can't serve the watcher: {error}");
    }
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    let own_watcher = matches!(bus.request_name_with_flags(WATCHER, flags).await, Ok(zbus::fdo::RequestNameReply::PrimaryOwner));
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    let _ = bus.request_name(host.as_str()).await;

    let mut tray = Tray { bus: bus.clone(), out: out.clone(), order: Vec::new(), items: HashMap::new(), owners: HashMap::new(), open_menus: HashSet::new() };

    if own_watcher {
        log::info!("tray: watching for background apps");
    } else {
        // Another bar holds the watcher: join it as a host and read its list
        log::info!("tray: another watcher is running, listening to it");
        dbus::call(&bus, WATCHER, WATCHER_PATH, WATCHER, "RegisterStatusNotifierHost", &(host.as_str(),)).await;
        if let Some(value) = dbus::get(&bus, WATCHER, WATCHER_PATH, WATCHER, "RegisteredStatusNotifierItems").await {
            for id in dbus::as_strings(&value) {
                tray.fetch(&id).await;
            }
        }
    }
    tray.publish();

    let rules = vec![
        dbus::rule().interface(ITEM).map(|r| r.build()),
        dbus::rule().interface(MENU).and_then(|r| r.member("LayoutUpdated")).map(|r| r.build()),
        dbus::rule().sender("org.freedesktop.DBus").and_then(|r| r.member("NameOwnerChanged")).map(|r| r.build()),
        dbus::rule().interface(WATCHER).map(|r| r.build()),
    ];
    let mut signals = dbus::watch(&bus, rules).await;
    let mut dirty: HashSet<String> = HashSet::new();

    loop {
        let settle = tokio::time::sleep(Duration::from_millis(if dirty.is_empty() { 3_600_000 } else { 100 }));
        tokio::select! {
            Some(WatcherEvent::Added(id)) = watcher_events.recv() => {
                tray.fetch(&id).await;
                tray.publish();
            }
            Some(message) = signals.recv() => {
                let header = message.header();
                let member = header.member().map(|m| m.to_string()).unwrap_or_default();
                let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
                let interface = header.interface().map(|i| i.to_string()).unwrap_or_default();
                match (interface.as_str(), member.as_str()) {
                    ("org.freedesktop.DBus", "NameOwnerChanged") => {
                        let Ok((name, _old, new)) = message.body().deserialize::<(String, String, String)>() else { continue };
                        if !new.is_empty() {
                            continue;
                        }
                        // An app went away: its items go with it
                        let gone: Vec<String> = tray.order.iter().filter(|id| split(id).0 == name || tray.owners.get(*id).is_some_and(|(o, _)| *o == name)).cloned().collect();
                        if !gone.is_empty() {
                            registered.lock().unwrap().retain(|id| !gone.contains(id));
                            for id in &gone {
                                if own_watcher {
                                    let emitter = SignalEmitter::new(&bus, WATCHER_PATH).ok();
                                    if let Some(emitter) = emitter {
                                        let _ = Watcher::status_notifier_item_unregistered(&emitter, id).await;
                                    }
                                }
                                tray.remove(id);
                            }
                            tray.publish();
                        }
                    }
                    (WATCHER, "StatusNotifierItemRegistered") if !own_watcher => {
                        if let Ok(id) = message.body().deserialize::<String>() {
                            tray.fetch(&id).await;
                            tray.publish();
                        }
                    }
                    (WATCHER, "StatusNotifierItemUnregistered") if !own_watcher => {
                        if let Ok(id) = message.body().deserialize::<String>() {
                            tray.remove(&id);
                            tray.publish();
                        }
                    }
                    (ITEM, _) => {
                        // NewIcon, NewStatus, NewTitle…: read that app's items again, a beat later
                        for id in &tray.order {
                            if tray.owners.get(id).is_some_and(|(o, _)| *o == sender) || split(id).0 == sender {
                                dirty.insert(id.clone());
                            }
                        }
                    }
                    (MENU, "LayoutUpdated") => {
                        let open: Vec<String> = tray.open_menus.iter().filter(|id| tray.owners.get(*id).is_some_and(|(o, _)| *o == sender)).cloned().collect();
                        for id in open {
                            tray.menu(&id).await;
                        }
                    }
                    _ => {}
                }
            }
            Some(command) = commands.recv() => tray.command(command).await,
            _ = settle, if !dirty.is_empty() => {
                for id in std::mem::take(&mut dirty) {
                    tray.fetch(&id).await;
                }
                tray.publish();
            }
        }
    }
}
