//! Notifications. The bar is the notification daemon (org.freedesktop.Notifications) when no
//! other one runs: it keeps them, shows them as pop-ups and in its centre, and runs their
//! actions. When another daemon holds the name and it's SwayNotificationCenter, the bar shows
//! its count and opens its panel instead.

use super::Out;
use super::dbus;
use crate::config::AppRule;
use crate::model::{Command, Image, Notification, Notifications, Update};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// How many the centre keeps.
const KEEP: usize = 100;

/// Why a notification closed, as the spec numbers it.
mod reason {
    pub const EXPIRED: u32 = 1;
    pub const DISMISSED: u32 = 2;
    pub const CLOSED: u32 = 3;
}

#[derive(Default)]
struct Store {
    next_id: u32,
    list: Vec<Notification>,
    quiet: bool,
}

enum Change {
    /// Arrived or replaced: show it.
    Toast(u32),
    Refresh,
}

struct Daemon {
    store: Arc<Mutex<Store>>,
    changes: UnboundedSender<Change>,
    /// App name (lower case) → what to do with its notifications.
    rules: Arc<Mutex<HashMap<String, AppRule>>>,
}

/// What a rule says for this app, by its name or its desktop file's.
fn rule_for(rules: &HashMap<String, AppRule>, app: &str, entry: &str) -> AppRule {
    [app, entry].iter().filter(|n| !n.is_empty()).find_map(|n| rules.get(&n.to_lowercase()).copied()).unwrap_or_default()
}

/// "<b>Hi</b> &amp; welcome" → "Hi & welcome": the bar shows plain text.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&").trim().to_string()
}

/// The (iiibiiay) picture some notifications carry, as premultiplied RGBA.
fn raw_image(value: &Value<'_>) -> Option<Arc<Image>> {
    let Value::Structure(s) = dbus::inner(value) else { return None };
    let f = s.fields();
    let number = |i: usize| f.get(i).and_then(|v| dbus::as_f64(v)).unwrap_or(0.0) as usize;
    let (width, height, stride) = (number(0), number(1), number(2));
    let alpha = f.get(3).and_then(|v| dbus::as_bool(v)).unwrap_or(false);
    let channels = number(5);
    let bytes = f.get(6).map(dbus::as_bytes).unwrap_or_default();
    if width == 0 || height == 0 || channels < 3 || bytes.len() < stride * (height - 1) + width * channels {
        return None;
    }
    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let p = &bytes[y * stride + x * channels..];
            let a = if alpha && channels >= 4 { p[3] as u32 } else { 255 };
            data.extend_from_slice(&[(p[0] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[2] as u32 * a / 255) as u8, a as u8]);
        }
    }
    let image = Image::new(width as u32, height as u32, data);
    // Big avatars and covers are cut down to what a card shows
    if width > 96 || height > 96 {
        let pixmap = tiny_skia::Pixmap::from_vec(image.data.clone(), tiny_skia::IntSize::from_wh(width as u32, height as u32)?)?;
        let png = pixmap.encode_png().ok()?;
        return crate::images::decode(&png, 96);
    }
    Some(image)
}

/// A file path, a file:// URL or an icon name, as a picture.
fn picture_from(name: &str) -> Option<Arc<Image>> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let path = name.strip_prefix("file://").unwrap_or(name);
    if path.starts_with('/') {
        return crate::images::load(std::path::Path::new(path), 96);
    }
    crate::images::find_icon(path, 64, None).and_then(|p| crate::images::load(&p, 64))
}

fn text_hint(hints: &HashMap<String, OwnedValue>, keys: &[&str]) -> String {
    keys.iter().find_map(|k| hints.get(*k).and_then(|v| dbus::as_str(v))).unwrap_or_default()
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Daemon {
    fn get_capabilities(&self) -> Vec<&str> {
        vec!["actions", "body", "icon-static", "persistence"]
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        // The picture: the one it carries, else its image path, its icon, its app's icon
        let carried = ["image-data", "image_data", "icon_data"].iter().find_map(|k| hints.get(*k)).and_then(|v| raw_image(v));
        let path = text_hint(&hints, &["image-path", "image_path"]);
        let entry = text_hint(&hints, &["desktop-entry"]);
        let rule = rule_for(&self.rules.lock().unwrap(), &app_name, &entry);
        if rule == AppRule::Discard {
            // Dropped, as asked: the app still gets an id, nothing is kept or shown
            let mut store = self.store.lock().unwrap();
            store.next_id = store.next_id.wrapping_add(1).max(1);
            return store.next_id;
        }
        let image = match carried {
            Some(image) => Some(image),
            None => tokio::task::spawn_blocking(move || {
                picture_from(&path)
                    .or_else(|| picture_from(&app_icon))
                    .or_else(|| crate::images::app(&entry).and_then(|a| picture_from(&a.icon)))
            })
            .await
            .ok()
            .flatten(),
        };
        let urgency = hints.get("urgency").and_then(|v| dbus::as_f64(v)).unwrap_or(1.0) as u8;
        let flag = |key: &str| hints.get(key).and_then(|v| dbus::as_bool(v)).unwrap_or(false);
        let id = {
            let mut store = self.store.lock().unwrap();
            let id = if replaces_id != 0 && store.list.iter().any(|n| n.id == replaces_id) {
                replaces_id
            } else {
                store.next_id = store.next_id.wrapping_add(1).max(1);
                store.next_id
            };
            let notification = Notification {
                id,
                app: app_name,
                summary: plain(&summary),
                body: plain(&body),
                image,
                actions: actions.chunks(2).map(|pair| (pair[0].clone(), pair.get(1).cloned().unwrap_or_default())).collect(),
                urgency,
                time: chrono::Local::now(),
                timeout: expire_timeout,
                resident: flag("resident"),
                transient: flag("transient"),
            };
            match store.list.iter().position(|n| n.id == id) {
                Some(at) => store.list[at] = notification,
                None => {
                    store.list.insert(0, notification);
                    store.list.truncate(KEEP);
                }
            }
            id
        };
        // Silenced apps go straight to the centre, without a pop-up
        let _ = self.changes.send(if rule == AppRule::Silence { Change::Refresh } else { Change::Toast(id) });
        id
    }

    async fn close_notification(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let removed = {
            let mut store = self.store.lock().unwrap();
            let before = store.list.len();
            store.list.retain(|n| n.id != id);
            store.list.len() != before
        };
        if removed {
            let _ = Self::notification_closed(&emitter, id, reason::CLOSED).await;
            let _ = self.changes.send(Change::Refresh);
        }
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("asterism".into(), "ETEA".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

fn snapshot(store: &Store) -> Notifications {
    Notifications { available: true, own: true, count: store.list.len() as u32, quiet: store.quiet, list: store.list.clone() }
}

/// Takes one out of the store, saying why.
async fn close(bus: &Connection, store: &Arc<Mutex<Store>>, id: u32, why: u32) {
    let removed = {
        let mut store = store.lock().unwrap();
        let before = store.list.len();
        store.list.retain(|n| n.id != id);
        store.list.len() != before
    };
    if removed && let Ok(emitter) = SignalEmitter::new(bus, PATH) {
        let _ = Daemon::notification_closed(&emitter, id, why).await;
    }
}

async fn daemon(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) -> Result<(), UnboundedReceiver<Command>> {
    let store = Arc::new(Mutex::new(Store::default()));
    let rules = Arc::new(Mutex::new(HashMap::new()));
    let (changes, mut changed) = unbounded_channel();
    if bus.object_server().at(PATH, Daemon { store: store.clone(), changes, rules: rules.clone() }).await.is_err() {
        return Err(commands);
    }
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    if !matches!(bus.request_name_with_flags(NAME, flags).await, Ok(zbus::fdo::RequestNameReply::PrimaryOwner)) {
        let _ = bus.object_server().remove::<Daemon, _>(PATH).await;
        log::info!("notifications: another daemon is running, leaving them to it");
        return Err(commands);
    }
    log::info!("notifications: the bar is the notification daemon");
    let publish = |store: &Arc<Mutex<Store>>| out.send(Update::Notifications(snapshot(&store.lock().unwrap())));
    publish(&store);
    loop {
        tokio::select! {
            Some(change) = changed.recv() => {
                publish(&store);
                if let Change::Toast(id) = change {
                    out.send(Update::Toast(id));
                }
            }
            Some(command) = commands.recv() => {
                match command {
                    Command::NotificationInvoke(id, key) => {
                        let resident = store.lock().unwrap().list.iter().find(|n| n.id == id).is_some_and(|n| n.resident);
                        if let Ok(emitter) = SignalEmitter::new(&bus, PATH) {
                            let _ = Daemon::action_invoked(&emitter, id, &key).await;
                        }
                        if !resident {
                            close(&bus, &store, id, reason::DISMISSED).await;
                        }
                    }
                    Command::NotificationDismiss(id) => close(&bus, &store, id, reason::DISMISSED).await,
                    Command::NotificationExpired(id) => {
                        let transient = store.lock().unwrap().list.iter().find(|n| n.id == id).is_some_and(|n| n.transient);
                        if transient {
                            close(&bus, &store, id, reason::EXPIRED).await;
                        }
                    }
                    Command::NotificationsClear => {
                        let ids: Vec<u32> = store.lock().unwrap().list.iter().map(|n| n.id).collect();
                        for id in ids {
                            close(&bus, &store, id, reason::DISMISSED).await;
                        }
                    }
                    Command::NotificationsQuiet => {
                        let mut store = store.lock().unwrap();
                        store.quiet = !store.quiet;
                    }
                    Command::NotificationRules(new) => *rules.lock().unwrap() = new,
                    _ => {}
                }
                publish(&store);
            }
        }
    }
}

async fn swaync(args: &[&str]) {
    let _ = tokio::process::Command::new("swaync-client").args(args).stdin(std::process::Stdio::null()).status().await;
}

pub async fn run(out: Out, bus: Option<Connection>, own: bool, commands: UnboundedReceiver<Command>) {
    let mut commands = commands;
    if own && let Some(bus) = bus {
        match daemon(out.clone(), bus, commands).await {
            Ok(()) => return,
            Err(back) => commands = back,
        }
    }
    if !crate::config::command_available("swaync-client") {
        out.send(Update::Notifications(Notifications::default()));
        while commands.recv().await.is_some() {}
        return;
    }
    loop {
        // One JSON line per change: {"text": "3", "alt": "dnd-notification", ...}
        let child = super::child("swaync-client").args(["--subscribe-waybar", "--skip-wait"]).stdout(std::process::Stdio::piped()).spawn();
        let Ok(mut child) = child else { return };
        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                        let alt = value.get("alt").and_then(|v| v.as_str()).unwrap_or("");
                        let count = value.get("text").and_then(|v| v.as_str()).and_then(|t| t.trim().parse().ok()).unwrap_or(0);
                        out.send(Update::Notifications(Notifications { available: true, count, quiet: alt.starts_with("dnd"), ..Default::default() }));
                    }
                    _ => break,
                },
                Some(command) = commands.recv() => match command {
                    Command::NotificationsToggle => swaync(&["--toggle-panel", "--skip-wait"]).await,
                    Command::NotificationsQuiet => swaync(&["--toggle-dnd", "--skip-wait"]).await,
                    _ => {}
                },
            }
        }
        let _ = child.kill().await;
        out.send(Update::Notifications(Notifications::default()));
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_match_by_name_or_desktop_file() {
        let rules: HashMap<String, AppRule> = [("spotify".to_string(), AppRule::Silence), ("discord".to_string(), AppRule::Discard)].into();
        assert_eq!(rule_for(&rules, "Spotify", ""), AppRule::Silence);
        assert_eq!(rule_for(&rules, "", "discord"), AppRule::Discard);
        assert_eq!(rule_for(&rules, "Firefox", "firefox"), AppRule::Allow);
    }
}
