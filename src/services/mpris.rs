//! Music players over MPRIS. The bar follows one player: the one picked by hand, else the
//! one playing, else the one it followed before (so pausing doesn't send it elsewhere), else
//! a paused one, else the first.

use super::dbus::{self, Props};
use super::Out;
use crate::model::{Command, Media, Player, Update};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

struct Mpris {
    bus: Connection,
    players: Vec<Player>,
    track_ids: HashMap<String, String>,
    remembered: Option<String>,
    chosen: Option<String>,
}

fn player_from(bus_name: &str, identity: String, props: &Props, track_ids: &mut HashMap<String, String>) -> Player {
    let status = dbus::text(props, "PlaybackStatus");
    let mut player = Player {
        bus: bus_name.to_string(),
        identity,
        playing: status == "Playing",
        paused: status == "Paused",
        position: dbus::number(props, "Position") as i64,
        position_at: Some(Instant::now()),
        rate: props.get("Rate").and_then(|v| dbus::as_f64(v)).unwrap_or(1.0),
        can_next: dbus::flag(props, "CanGoNext"),
        can_previous: dbus::flag(props, "CanGoPrevious"),
        ..Default::default()
    };
    if let Some(metadata) = props.get("Metadata") {
        let metadata = dbus::dict(metadata);
        let text = |key: &str| metadata.get(key).and_then(|v| dbus::as_str(v)).unwrap_or_default();
        player.title = text("xesam:title");
        player.artist = metadata.get("xesam:artist").map(|v| dbus::as_strings(v).join(", ")).unwrap_or_default();
        player.album = text("xesam:album");
        player.art_url = text("mpris:artUrl");
        player.length = metadata.get("mpris:length").and_then(|v| dbus::as_f64(v)).unwrap_or(0.0) as i64;
        let track = text("mpris:trackid");
        if !track.is_empty() {
            track_ids.insert(bus_name.to_string(), track);
        }
    }
    player
}

impl Mpris {
    async fn names(&self) -> Vec<String> {
        let Ok(proxy) = zbus::fdo::DBusProxy::new(&self.bus).await else { return Vec::new() };
        let mut names: Vec<String> = proxy
            .list_names()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.to_string())
            .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
            .collect();
        names.sort();
        names
    }

    async fn refresh(&mut self) {
        let mut players = Vec::new();
        for name in self.names().await {
            let Some(props) = dbus::get_all(&self.bus, &name, PATH, PLAYER).await else { continue };
            let identity = dbus::get(&self.bus, &name, PATH, "org.mpris.MediaPlayer2", "Identity")
                .await
                .and_then(|v| dbus::as_str(&v))
                .unwrap_or_else(|| name.trim_start_matches("org.mpris.MediaPlayer2.").split('.').next().unwrap_or("").to_string());
            let player = player_from(&name, identity, &props, &mut self.track_ids);
            // A player with nothing to show isn't one
            if !player.title.is_empty() {
                players.push(player);
            }
        }
        self.players = players;
    }

    fn media(&mut self) -> Media {
        let find = |test: &dyn Fn(&Player) -> bool| self.players.iter().position(test);
        let chosen = self.chosen.clone();
        let remembered = self.remembered.clone();
        let current = find(&|p| Some(&p.bus) == chosen.as_ref())
            .or_else(|| find(&|p| p.playing))
            .or_else(|| find(&|p| Some(&p.bus) == remembered.as_ref()))
            .or_else(|| find(&|p| p.paused))
            .or(if self.players.is_empty() { None } else { Some(0) });
        self.remembered = current.map(|i| self.players[i].bus.clone());
        Media { players: self.players.clone(), current }
    }

    async fn call(&self, bus_name: &str, method: &str) {
        dbus::call(&self.bus, bus_name, PATH, PLAYER, method, &()).await;
    }

    async fn command(&mut self, command: Command, out: &Out) {
        match command {
            Command::MediaPlayPause(bus) => self.call(&bus, "PlayPause").await,
            Command::MediaNext(bus) => self.call(&bus, "Next").await,
            Command::MediaPrevious(bus) => self.call(&bus, "Previous").await,
            Command::MediaSeek(bus, position) => {
                if let Some(track) = self.track_ids.get(&bus).cloned()
                    && let Ok(track) = zbus::zvariant::ObjectPath::try_from(track.as_str())
                {
                    dbus::call(&self.bus, &bus, PATH, PLAYER, "SetPosition", &(track, position)).await;
                }
            }
            Command::MediaChoose(bus) => {
                self.chosen = Some(bus);
                let media = self.media();
                out.send(Update::Media(media));
            }
            Command::FetchArt(url) => {
                let out = out.clone();
                tokio::spawn(async move {
                    if let Some(image) = fetch_art(&url).await {
                        out.send(Update::Art(url, image));
                    }
                });
            }
            _ => {}
        }
    }
}

/// A cover from a file, or downloaded once with curl into the cache.
async fn fetch_art(url: &str) -> Option<std::sync::Arc<crate::model::Image>> {
    let path = if let Some(path) = url.strip_prefix("file://") {
        std::path::PathBuf::from(percent_decode(path))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::config::home().join(".cache"))
            .join("asterism")
            .join("art");
        std::fs::create_dir_all(&cache).ok()?;
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut hasher);
        let file = cache.join(format!("{:016x}", hasher.finish()));
        if !file.exists() {
            let ok = tokio::process::Command::new("curl")
                .args(["-sfL", "--max-time", "10", "--max-filesize", "8000000", "-o"])
                .arg(&file)
                .arg(url)
                .status()
                .await
                .ok()?
                .success();
            if !ok {
                let _ = std::fs::remove_file(&file);
                return None;
            }
        }
        file
    } else {
        return None;
    };
    tokio::task::spawn_blocking(move || crate::images::load(&path, 256)).await.ok()?
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) {
    let mut mpris = Mpris { bus: bus.clone(), players: Vec::new(), track_ids: HashMap::new(), remembered: None, chosen: None };
    mpris.refresh().await;
    out.send(Update::Media(mpris.media()));

    let rules = vec![
        dbus::rule()
            .interface("org.freedesktop.DBus.Properties")
            .and_then(|r| r.member("PropertiesChanged"))
            .and_then(|r| r.path(PATH))
            .map(|r| r.build()),
        dbus::rule().interface(PLAYER).and_then(|r| r.member("Seeked")).map(|r| r.build()),
        dbus::rule()
            .sender("org.freedesktop.DBus")
            .and_then(|r| r.member("NameOwnerChanged"))
            .and_then(|r| r.arg0ns("org.mpris.MediaPlayer2"))
            .map(|r| r.build()),
    ];
    let mut signals = dbus::watch(&bus, rules).await;

    let mut pending = false;
    loop {
        let settle = tokio::time::sleep(Duration::from_millis(if pending { 80 } else { 3_600_000 }));
        tokio::select! {
            Some(_) = signals.recv() => pending = true,
            Some(command) = commands.recv() => mpris.command(command, &out).await,
            _ = settle, if pending => {
                pending = false;
                mpris.refresh().await;
                out.send(Update::Media(mpris.media()));
            }
        }
    }
}
