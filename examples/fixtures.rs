//! A fake music player (MPRIS) and a fake tray app (StatusNotifierItem with a DBusMenu), to
//! see the bar's media and tray parts without real apps:
//!
//!     cargo run --example fixtures -- 30
//!
//! runs them for 30 seconds.

use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::{ObjectPath, OwnedValue, Structure, Value};

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Fixture Player".into()
    }
}

struct Player {
    playing: bool,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play_pause(&mut self) {
        self.playing = !self.playing;
        println!("player: play/pause → {}", self.playing);
    }
    fn next(&self) {
        println!("player: next");
    }
    fn previous(&self) {
        println!("player: previous");
    }
    #[zbus(property)]
    fn playback_status(&self) -> String {
        if self.playing { "Playing".into() } else { "Paused".into() }
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        let owned = |v: Value<'_>| v.try_to_owned().unwrap();
        map.insert("xesam:title".into(), owned(Value::from("Stargazing over the long quiet ridge")));
        map.insert("xesam:artist".into(), owned(Value::from(vec!["Kygo", "Justin Jesso"])));
        map.insert("xesam:album".into(), owned(Value::from("Stargazing EP")));
        map.insert("mpris:length".into(), owned(Value::from(236_000_000i64)));
        map.insert("mpris:trackid".into(), owned(Value::from(ObjectPath::try_from("/fixture/track/1").unwrap())));
        map
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        19_000_000
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
}

struct Item;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, x: i32, y: i32) {
        println!("tray: activate at {x},{y}");
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        println!("tray: secondary activate");
    }
    #[zbus(property)]
    fn id(&self) -> String {
        "fixture".into()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        "Fixture Tray App".into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        "Active".into()
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        "".into()
    }
    /// A 16×16 pixmap: a filled circle, ARGB in network byte order.
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        let mut bytes = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                let inside = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt() < 7.0;
                bytes.extend_from_slice(&if inside { [255, 120, 200, 255] } else { [0, 0, 0, 0] });
            }
        }
        vec![(16, 16, bytes)]
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::try_from("/MenuBar").unwrap()
    }
}

struct Menu;

type Entry = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn entry(id: i32, props: &[(&str, Value<'static>)], children: Vec<OwnedValue>) -> Entry {
    let props = props.iter().map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap())).collect();
    (id, props, children)
}

fn child(e: Entry) -> OwnedValue {
    Value::Structure(Structure::from(e)).try_to_owned().unwrap()
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(&self, _parent: i32, _depth: i32, _props: Vec<String>) -> (u32, Entry) {
        let sub = vec![
            child(entry(21, &[("label", "_Low".into()), ("toggle-type", "radio".into()), ("toggle-state", 1i32.into())], vec![])),
            child(entry(22, &[("label", "_High".into()), ("toggle-type", "radio".into()), ("toggle-state", 0i32.into())], vec![])),
        ];
        let top = vec![
            child(entry(1, &[("label", "_Open window".into())], vec![])),
            child(entry(2, &[("label", "Show notifications".into()), ("toggle-type", "checkmark".into()), ("toggle-state", 1i32.into())], vec![])),
            child(entry(3, &[("label", "Quality".into()), ("children-display", "submenu".into())], sub)),
            child(entry(4, &[("type", "separator".into())], vec![])),
            child(entry(5, &[("label", "Disabled entry".into()), ("enabled", false.into())], vec![])),
            child(entry(6, &[("label", "_Quit".into())], vec![])),
        ];
        (1, entry(0, &[("children-display", "submenu".into())], top))
    }
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn event(&self, id: i32, event: String, _data: Value<'_>, _time: u32) {
        println!("tray menu: {event} on entry {id}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> zbus::Result<()> {
    let seconds: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(30);
    let player = zbus::connection::Builder::session()?
        .name("org.mpris.MediaPlayer2.fixture")?
        .serve_at("/org/mpris/MediaPlayer2", Root)?
        .serve_at("/org/mpris/MediaPlayer2", Player { playing: true })?
        .build()
        .await?;
    let tray = zbus::connection::Builder::session()?.serve_at("/StatusNotifierItem", Item)?.serve_at("/MenuBar", Menu)?.build().await?;
    let name = tray.unique_name().map(|n| n.to_string()).unwrap_or_default();
    match tray
        .call_method(Some("org.kde.StatusNotifierWatcher"), "/StatusNotifierWatcher", Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &(name.as_str(),))
        .await
    {
        Ok(_) => println!("tray item registered as {name}"),
        Err(error) => println!("no tray watcher: {error}"),
    }
    println!("fixtures running for {seconds}s");
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    drop(player);
    Ok(())
}
