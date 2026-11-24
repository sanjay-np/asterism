//! The settings app: the config as pages of switches, choices and steppers. Every change is
//! written to config.toml straight away with toml_edit, which keeps your comments and order;
//! the shell then reads the file again, as it does for any edit.

use crate::config::Config;
use crate::model::Model;
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};
use std::path::Path;

/// A value to write.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<String>),
    /// Takes the key out (the default comes back).
    Remove,
}

pub enum Control {
    Switch(bool),
    /// A switch that writes this value when flipped (a list with or without something).
    SwitchTo(bool, Value),
    /// Labels and the values they write; which one is current.
    Choice(Vec<(String, Value)>, Option<usize>),
    Stepper { value: i64, min: i64, max: i64, step: i64, unit: &'static str },
    /// Typed in place: what's written now, and what's used while it's empty.
    Text(String, String),
    /// Not a setting: a group's title (the row's label) and what the group is for (its about),
    /// over a card of its own.
    Section,
    Button(&'static str, Action),
    /// Pictures to pick from, under the label, as many lines of them as they need.
    Gallery(Vec<Tile>),
    /// The bar's three parts with their items in order, those left out, and the one picked to
    /// move; under the label, the row's whole width.
    /// `usual`: it's the arrangement the shell comes with.
    Arrange { parts: [Vec<String>; 3], hidden: Vec<String>, picked: Option<String>, usual: bool },
}

/// One picture in a gallery: a pet drawn as it is (an animal and its coat) or an icon, a name
/// and a word under it; lit when it's the one chosen; what a click does, and a × when it can
/// be taken away.
pub struct Tile {
    pub picture: Option<(crate::pet::Species, String)>,
    pub icon: Option<&'static str>,
    pub label: String,
    pub sub: String,
    pub on: bool,
    pub action: Option<Action>,
    pub remove: Option<Action>,
}

/// Which pet the Pets page shows (its place in the list), or the animals to add one from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PetPick {
    pub selected: usize,
    pub adding: bool,
}

/// What's picked on the pages that pick something: a pet, and the bar's item being moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pick {
    pub pet: PetPick,
    pub bar: Option<String>,
    /// Arrange mode is on in the bar.
    pub arranging: bool,
}

/// Everything the bar can show: its name in `[layout]`, what it's called here, and its icon.
pub const BAR_ITEMS: [(&str, &str, &str); 10] = [
    ("launcher", "Apps button", "apps"),
    ("pages", "Workspaces", "layout"),
    ("resources", "Machine", "cpu"),
    ("clock", "Clock", "calendar"),
    ("day", "Your day", "checklist"),
    ("weather", "Weather", "cloud-sun"),
    ("media", "Music", "music"),
    ("tools", "Tools", "capture"),
    ("tray", "Tray", "dock"),
    ("status", "Status", "wifi-high"),
];

/// The bar's parts as they read across: left, middle, right.
pub const BAR_PARTS: [&str; 3] = ["left", "center", "right"];

/// A bar item's name here and its icon (one the shell doesn't know keeps its own name).
pub fn bar_item(name: &str) -> (String, &'static str) {
    BAR_ITEMS.iter().find(|(n, ..)| *n == name).map_or((name.to_string(), "layout"), |(_, label, icon)| (label.to_string(), *icon))
}

/// One step in arranging the bar.
#[derive(Clone, Debug, PartialEq)]
pub enum BarMove {
    /// One place to the left; from the start of a part, to the end of the part before.
    Earlier(String),
    /// One place to the right; from the end of a part, to the start of the next.
    Later(String),
    /// Into this part (0 left, 1 middle, 2 right), on the side it comes from.
    To(String, usize),
    /// Out of the bar.
    Off(String),
    /// Dropped on the bar: into this part, just after this item there (None: at its start).
    Place(String, usize, Option<String>),
    /// The usual arrangement back.
    Usual,
}

/// The bar's parts after a move. `None` when nothing changes (or for `Usual`, which writes no
/// lists but takes them out).
pub fn arrange_bar(parts: [&[String]; 3], how: &BarMove) -> Option<[Vec<String>; 3]> {
    let mut next: [Vec<String>; 3] = parts.map(<[String]>::to_vec);
    let find = |next: &[Vec<String>; 3], item: &str| next.iter().enumerate().find_map(|(p, list)| list.iter().position(|m| m == item).map(|i| (p, i)));
    match how {
        BarMove::Earlier(item) => {
            let (p, i) = find(&next, item)?;
            if i > 0 {
                next[p].swap(i, i - 1);
            } else if p > 0 {
                let moved = next[p].remove(i);
                next[p - 1].push(moved);
            } else {
                return None;
            }
        }
        BarMove::Later(item) => {
            let (p, i) = find(&next, item)?;
            if i + 1 < next[p].len() {
                next[p].swap(i, i + 1);
            } else if p < 2 {
                let moved = next[p].remove(i);
                next[p + 1].insert(0, moved);
            } else {
                return None;
            }
        }
        BarMove::To(item, to) => {
            let to = (*to).min(2);
            let from = find(&next, item);
            if from.is_some_and(|(p, _)| p == to) {
                return None;
            }
            for list in next.iter_mut() {
                list.retain(|m| m != item);
            }
            // Moving right it lands at the part's start, otherwise at its end: next to where it
            // was. Put back on the right, it goes before what's there (status stays at the end)
            match from {
                Some((p, _)) if p < to => next[to].insert(0, item.clone()),
                None if to == 2 => next[to].insert(0, item.clone()),
                _ => next[to].push(item.clone()),
            }
        }
        BarMove::Place(item, to, after) => {
            let to = (*to).min(2);
            for list in next.iter_mut() {
                list.retain(|m| m != item);
            }
            let at = after.as_ref().and_then(|a| next[to].iter().position(|m| m == a)).map_or(0, |i| i + 1);
            next[to].insert(at, item.clone());
            if next.iter().zip(parts).all(|(a, b)| a[..] == *b) {
                return None;
            }
        }
        BarMove::Off(item) => {
            find(&next, item)?;
            for list in next.iter_mut() {
                list.retain(|m| m != item);
            }
        }
        BarMove::Usual => return None,
    }
    Some(next)
}

/// The bar's arrangement: its parts, each item a chip; one picked moves along with the
/// buttons under them, or to another part by a click on that part.
fn arrangement(c: &Config, picked: Option<&str>) -> Row {
    let l = &c.layout;
    let parts = [l.left.clone(), l.center.clone(), l.right.clone()];
    let hidden: Vec<String> = BAR_ITEMS.iter().map(|(n, ..)| n.to_string()).filter(|n| !parts.iter().any(|p| p.contains(n))).collect();
    // A pick that's no longer anywhere (a name taken out of the file) is let go
    let picked = picked.filter(|p| parts.iter().any(|l| l.iter().any(|m| m == p)) || hidden.iter().any(|m| m == p));
    let about = match picked {
        None => "Click an item to move it",
        Some(p) if hidden.iter().any(|m| m == p) => "Click a part to put it there",
        Some(_) => "Move it along, or click another part to move it there",
    };
    let d = crate::config::Layout::default();
    let usual = l.left == d.left && l.center == d.center && l.right == d.right;
    Row { keys: Vec::new(), label: "What sits where".into(), about: about.into(), control: Control::Arrange { parts, hidden, picked: picked.map(str::to_string), usual } }
}

/// The arrangement's pieces from its top left, `w` wide: the three parts side by side with their
/// items' chips, those left out under them, then the buttons for the one picked. And its height.
fn arrange_pieces(parts: &[Vec<String>; 3], hidden: &[String], picked: Option<&str>, usual: bool, t: &Theme, text: &mut Text, w: f32) -> (Vec<Piece>, f32) {
    const GAP: f32 = 10.0;
    const CHIP: f32 = 32.0;
    const STEP: f32 = CHIP + 6.0;
    const HEAD: f32 = 34.0;
    let head_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
    let chip_font = Font::new(t.text - 1.0).weight(500);
    let mut pieces = Vec::new();
    let col_w = ((w - GAP * 2.0) / 3.0).floor();
    let most = parts.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let col_h = HEAD + most as f32 * STEP + 4.0;
    let picked_part = picked.and_then(|p| parts.iter().position(|l| l.iter().any(|m| m == p)));
    let chip_w = |name: &str, text: &mut Text| text.measure(&bar_item(name).0, chip_font).0 + 52.0;
    // An item: its icon and name on a pill; the picked one lit
    let chip = |pieces: &mut Vec<Piece>, name: &str, x: f32, y: f32, cw: f32, text: &mut Text| {
        let (label, icon) = bar_item(name);
        let on = picked == Some(name);
        let mut p = Piece::new(format!("set.bar.{name}"), cw, CHIP);
        if on {
            p.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: cw, h: CHIP, radius: CHIP / 2.0, color: t.primary });
            p.prims.push(Prim::Rect { x: 1.5, y: 1.5, w: cw - 3.0, h: CHIP - 3.0, radius: CHIP / 2.0 - 1.5, color: t.secondary_container });
        } else {
            p.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: cw, h: CHIP, radius: CHIP / 2.0, color: t.hairline() });
            p.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: cw - 2.0, h: CHIP - 2.0, radius: CHIP / 2.0 - 1.0, color: t.card_deep });
        }
        p.prims.push(Prim::Icon { x: 13.0, y: (CHIP - 16.0) / 2.0, size: 16.0, name: icon.into(), color: if on { t.primary } else { t.color_soft } });
        let label = crate::bar::Ctx::ellipsize_with(text, &label, chip_font, cw - 52.0);
        let (_, lh) = text.measure(&label, chip_font);
        p.prims.push(Prim::Text { x: 37.0, y: ((CHIP - lh) / 2.0).round(), text: label, font: chip_font, color: if on { t.on_secondary_container } else { t.color_text } });
        p.highlight = !on;
        p.actions.left = Some(Action::SettingsBarPick(if on { None } else { Some(name.to_string()) }));
        p.x = x;
        p.y = y;
        pieces.push(p);
    };
    for (p, list) in parts.iter().enumerate() {
        let x = p as f32 * (col_w + GAP);
        // With an item picked, every other part is somewhere to send it
        let target = picked.filter(|_| picked_part != Some(p));
        let mut col = Piece::new(format!("set.part.{p}"), col_w, col_h);
        col.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: col_w, h: col_h, radius: 14.0, color: if target.is_some() { t.primary.alpha(0.6) } else { t.hairline() } });
        col.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: col_w - 2.0, h: col_h - 2.0, radius: 13.0, color: t.surface });
        let title = ["LEFT", "MIDDLE", "RIGHT"][p];
        col.prims.push(Prim::Text { x: 14.0, y: 12.0, text: title.into(), font: head_font, color: if target.is_some() { t.primary } else { t.color_soft } });
        if list.is_empty() {
            col.prims.push(Prim::Text { x: 14.0, y: HEAD + 8.0, text: "Empty".into(), font: chip_font, color: t.color_soft.alpha(0.6) });
        }
        match target {
            Some(item) => {
                col.highlight = true;
                col.actions.left = Some(Action::BarArrange(BarMove::To(item.to_string(), p)));
            }
            None => col.blocks = true,
        }
        col.x = x;
        pieces.push(col);
        for (i, name) in list.iter().enumerate() {
            chip(&mut pieces, name, x + 8.0, HEAD + i as f32 * STEP, col_w - 16.0, text);
        }
    }
    let mut y = col_h;
    // What the bar could show but doesn't, in lines as long as the row
    if !hidden.is_empty() {
        y += 18.0;
        let title = "NOT IN THE BAR";
        let (tw, th) = text.measure(title, head_font);
        let mut words = Piece::new("set.bar.off", tw, th);
        words.prims.push(Prim::Text { x: 0.0, y: 0.0, text: title.into(), font: head_font, color: t.color_soft });
        words.x = 4.0;
        words.y = y;
        pieces.push(words);
        y += th + 10.0;
        let mut x = 0.0;
        for name in hidden {
            let cw = chip_w(name, text).min(w);
            if x > 0.0 && x + cw > w {
                x = 0.0;
                y += STEP;
            }
            chip(&mut pieces, name, x, y, cw, text);
            x += cw + 8.0;
        }
        y += CHIP;
    }
    // The picked one's moves (those that would do nothing are dimmed), and the usual arrangement
    let refs = [&parts[0][..], &parts[1][..], &parts[2][..]];
    let mut buttons: Vec<(&str, &str, Option<Action>, bool)> = Vec::new();
    if let Some(item) = picked.filter(|_| picked_part.is_some()) {
        let can = |how: BarMove| arrange_bar(refs, &how).is_some().then_some(Action::BarArrange(how));
        buttons.push(("set.bar.earlier", "‹  Earlier", can(BarMove::Earlier(item.into())), true));
        buttons.push(("set.bar.later", "Later  ›", can(BarMove::Later(item.into())), true));
        buttons.push(("set.bar.remove", "Take it off", Some(Action::BarArrange(BarMove::Off(item.into()))), false));
    }
    if !usual {
        buttons.push(("set.bar.usual", "Usual arrangement", Some(Action::BarArrange(BarMove::Usual)), false));
    }
    if !buttons.is_empty() {
        y += 16.0;
        let mut x: f32 = 0.0;
        for (key, label, action, filled) in buttons {
            let (lw, lh) = text.measure(label, chip_font);
            let (bw, bh) = (lw + 26.0, 32.0);
            // The usual arrangement at the far end, apart from the moves
            if key == "set.bar.usual" {
                x = x.max(w - bw);
            }
            let live = action.is_some();
            let ink = if !live { t.color_soft.alpha(0.5) } else if filled { t.on_secondary_container } else { t.color_text };
            let mut b = Piece::new(key, bw, bh);
            if filled && live {
                b.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.secondary_container });
            } else {
                b.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.hairline() });
                b.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: bw - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
            }
            b.prims.push(Prim::Text { x: 13.0, y: ((bh - lh) / 2.0).round(), text: label.into(), font: chip_font, color: ink });
            b.highlight = live;
            b.blocks = true;
            b.actions.left = action;
            b.x = x;
            b.y = y;
            pieces.push(b);
            x += bw + 8.0;
        }
        y += 32.0;
    }
    (pieces, y)
}

pub struct Row {
    /// Where in the config, table by table.
    pub keys: Vec<String>,
    pub label: String,
    pub about: String,
    pub control: Control,
}

fn row(keys: &[&str], label: &str, about: &str, control: Control) -> Row {
    Row { keys: keys.iter().map(|k| k.to_string()).collect(), label: label.into(), about: about.into(), control }
}

/// A group's title over its own card.
fn section(label: &str, about: &str) -> Row {
    Row { keys: Vec::new(), label: label.into(), about: about.into(), control: Control::Section }
}

fn choice(options: &[(&str, &str)], current: &str) -> Control {
    let list: Vec<(String, Value)> = options.iter().map(|(label, value)| (label.to_string(), Value::Str(value.to_string()))).collect();
    let at = options.iter().position(|(_, v)| v.eq_ignore_ascii_case(current));
    Control::Choice(list, at)
}

pub const PAGES: [&str; 11] = ["Look", "Bar", "Wallpaper", "Desktop", "Dock", "Pets", "Night light", "Lock", "Notifications", "Weather", "Timers"];
/// A gallery's pictures: their size, and the room between them.
const TILE: (f32, f32) = (124.0, 140.0);
const TILE_GAP: f32 = 10.0;

/// The Pets page's place among them.
pub const PETS_PAGE: usize = 5;
const PAGE_ICONS: [&str; 11] = ["picker", "layout", "image", "apps", "dock", "paw", "nightlight", "lock", "bell", "cloud-sun", "timer"];

/// What a typed value says now, to start editing from.
pub fn text_value(c: &Config, keys: &[String]) -> String {
    match keys.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["weather", "city"] => c.weather.city.clone(),
        ["widgets", "note"] => c.widgets.note.clone(),
        ["widgets", "photos"] => c.widgets.photos.clone(),
        _ => String::new(),
    }
}

/// A switch for one thing in a list setting: flipping it writes the list without it, or with it.
fn in_list(keys: &[&str], list: &[String], item: &str, label: &str, about: &str) -> Row {
    let on = list.iter().any(|k| k == item);
    let mut next: Vec<String> = list.to_vec();
    if on {
        next.retain(|k| k != item);
    } else {
        next.push(item.to_string());
    }
    row(keys, label, about, Control::SwitchTo(on, Value::List(next)))
}

/// A switch for one of the bar's resources, which keeps them in their usual order when turned on.
/// Your day's button in the bar: on, it goes just after the clock (in whichever part of the bar
/// the clock is, else the middle); off, it's taken out wherever it was.
fn day_button(c: &Config) -> Row {
    let l = &c.layout;
    let parts: [(&str, &Vec<String>); 3] = [("left", &l.left), ("center", &l.center), ("right", &l.right)];
    let label = "Your day";
    let about = "A button for timers, to-do and notes, with how many tasks are left (the clock opens the calendar)";
    if let Some((part, list)) = parts.iter().find(|(_, list)| list.iter().any(|m| m == "day")) {
        let next: Vec<String> = list.iter().filter(|m| *m != "day").cloned().collect();
        return row(&["layout", part], label, about, Control::SwitchTo(true, Value::List(next)));
    }
    let (part, list) = parts.iter().find(|(_, list)| list.iter().any(|m| m == "clock")).copied().unwrap_or(("center", &l.center));
    let mut next = list.clone();
    let at = next.iter().position(|m| m == "clock").map_or(next.len(), |i| i + 1);
    next.insert(at, "day".into());
    row(&["layout", part], label, about, Control::SwitchTo(false, Value::List(next)))
}

fn resource(list: &[String], item: &str, label: &str, about: &str) -> Row {
    const ORDER: [&str; 4] = ["cpu", "memory", "temperature", "network"];
    let on = list.iter().any(|k| k == item);
    let mut next: Vec<String> = list.iter().filter(|k| *k != item).cloned().collect();
    if !on {
        let rank = |k: &str| ORDER.iter().position(|o| *o == k).unwrap_or(ORDER.len());
        let at = next.iter().position(|k| rank(k) > rank(item)).unwrap_or(next.len());
        next.insert(at, item.to_string());
    }
    row(&["resources"], label, about, Control::SwitchTo(on, Value::List(next)))
}

/// The dock's page: how it behaves and looks, then its apps (pinned ones to unpin, open ones
/// to pin; the order is changed by dragging on the dock itself).
fn dock_rows(c: &Config, model: &Model) -> Vec<Row> {
    let d = &c.dock;
    let mut list = vec![row(&["dock", "enabled"], "Dock", "Your apps on a card along an edge of the screen", Control::Switch(d.enabled))];
    if !d.enabled {
        return list;
    }
    list.push(section("Behaviour", ""));
    list.push(row(&["dock", "position"], "Position", "The edge of the screen it sits on", choice(&[("Bottom", "bottom"), ("Top", "top"), ("Left", "left"), ("Right", "right")], &d.position)));
    list.push(row(&["dock", "autohide"], "Hide it", "Out of sight until the pointer touches the edge by it, or Mod+D; off: always out, windows kept clear of it", Control::Switch(d.autohide)));
    if d.autohide {
        list.push(row(&["dock", "hide_delay"], "Stays after the pointer leaves", "", Control::Stepper { value: d.hide_delay as i64, min: 100, max: 2000, step: 50, unit: "ms" }));
    }
    list.push(row(&["dock", "running"], "Open apps", "Apps with windows open join the pinned ones, a dot for each window", Control::Switch(d.running)));
    list.push(row(&["dock", "launcher"], "Apps button", "At the start: opens the launcher", Control::Switch(d.launcher)));
    list.push(row(&["dock", "trash"], "Trash", "At the end: click opens it, right click empties it", Control::Switch(d.trash)));
    list.push(section("Look", ""));
    list.push(row(&["dock", "size"], "Icon size", "", Control::Stepper { value: d.size as i64, min: 32, max: 72, step: 4, unit: "px" }));
    list.push(row(&["dock", "magnify"], "Grow under the pointer", "The icon you point at, and a little its neighbours", Control::Switch(d.magnify)));
    list.push(row(&["dock", "corners"], "Corners", "", choice(&[("Rounded", "rounded"), ("Small", "small"), ("Square", "square"), ("Pill", "pill")], &d.corners)));
    list.push(row(&["dock", "background"], "Background", "Card: edged, with a soft shadow; glass: the wallpaper shows through", choice(&[("Card", "card"), ("Flat", "flat"), ("Glass", "glass"), ("None", "none")], &d.background)));
    list.push(row(&["dock", "attached"], "On the screen's edge", "Sits on the edge, its lower corners square, rather than floating just above it", Control::Switch(d.attached)));
    list.push(row(&["dock", "indicator"], "Open windows", "How an open app is marked", choice(&[("Dots", "dots"), ("Line", "line"), ("None", "none")], &d.indicator)));
    list.push(row(&["dock", "separators"], "Separators", "Thin lines between the apps button, pinned apps, open apps and the trash", Control::Switch(d.separators)));
    let items = crate::dock::items(&d.pinned, &model.compositor.workspaces, &model.apps);
    if !items.is_empty() {
        list.push(section("Apps", "Pinned ones stay on the dock; drag them along it to reorder, or right-click one"));
    }
    for item in items {
        let windows = match item.windows.len() {
            0 => String::new(),
            1 => "open".into(),
            n => format!("{n} windows"),
        };
        let (about, control) = if item.pinned {
            let about = if windows.is_empty() { "Pinned".to_string() } else { format!("Pinned · {windows}") };
            (about, Control::Button("Unpin", Action::DockPin(item.key.clone(), false)))
        } else {
            let mut about = windows.clone();
            about[..1].make_ascii_uppercase();
            (about, Control::Button("Pin", Action::DockPin(item.key.clone(), true)))
        };
        list.push(Row { keys: Vec::new(), label: item.name, about, control });
    }
    list
}

/// A coat's name as a label ("tabby" → "Tabby").
fn title_case(name: &str) -> String {
    let mut label = name.to_string();
    if let Some(first) = label.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    label
}

/// The Pets page: your pets as pictures of themselves (click one to change it, × to put it
/// away, + to add one), then the one chosen (its animal, and its coats as pictures) or the
/// animals to add, then what they all share.
fn pet_rows(c: &Config, pick: &PetPick) -> Vec<Row> {
    use crate::pet::Species;
    let mut list = Vec::new();
    let entries = c.pet.entries();
    let keys = vec!["pet".to_string(), "pets".to_string()];
    let with = |at: usize, entry: String| -> Value {
        let mut next = entries.clone();
        next[at] = entry;
        Value::List(next)
    };
    // Those naming an animal there is, by their place in the list
    let pets: Vec<(usize, Species, String)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let (animal, coat) = crate::config::pet_entry(e);
            Some((i, Species::from_name(&animal)?, coat))
        })
        .collect();
    let adding = pick.adding || pets.is_empty();
    let chosen = (!adding).then(|| pets.iter().find(|(i, ..)| *i == pick.selected).or(pets.first())).flatten().cloned();
    let mut tiles: Vec<Tile> = pets
        .iter()
        .map(|(i, species, coat)| {
            let names = crate::pet::draw::coats(*species);
            let coat_name = names.iter().find(|n| n.eq_ignore_ascii_case(coat)).unwrap_or(&names[0]);
            Tile {
                picture: Some((*species, coat.clone())),
                icon: None,
                label: species.title().into(),
                sub: title_case(coat_name),
                on: chosen.as_ref().is_some_and(|(at, ..)| at == i),
                action: Some(Action::SettingsPet(Some(*i))),
                remove: Some(Action::PetAway(*i)),
            }
        })
        .collect();
    tiles.push(Tile { picture: None, icon: Some("add"), label: "Add a pet".into(), sub: if pets.is_empty() { "Pick an animal".into() } else { "As many as you like".into() }, on: adding, action: Some(Action::SettingsPet(None)), remove: None });
    let about = if pets.is_empty() { "Companions roaming the screen above your windows" } else { "Click one to change it, × puts it away; they greet, chase and nap together" };
    list.push(Row { keys: Vec::new(), label: "Your pets".into(), about: about.into(), control: Control::Gallery(tiles) });

    match chosen {
        None => {
            // The animals, each in its own coat: a click adds one
            let tiles = Species::ALL.iter().map(|s| Tile { picture: Some((*s, String::new())), icon: None, label: s.title().into(), sub: String::new(), on: false, action: Some(Action::PetAdd(s.name().into())), remove: None }).collect();
            list.push(Row { keys: Vec::new(), label: "Add which animal?".into(), about: "Its coat can be changed after".into(), control: Control::Gallery(tiles) });
        }
        Some((at, species, coat)) => {
            let animals: Vec<(String, Value)> = Species::ALL.iter().map(|s| (s.title().to_string(), with(at, s.name().to_string()))).collect();
            let current = Species::ALL.iter().position(|s| *s == species);
            list.push(Row { keys: keys.clone(), label: "Animal".into(), about: "Turns this one into another".into(), control: Control::Choice(animals, current) });
            let names = crate::pet::draw::coats(species);
            let current = names.iter().position(|n| n.eq_ignore_ascii_case(&coat)).unwrap_or(0);
            let tiles = names
                .iter()
                .enumerate()
                .map(|(n, name)| Tile {
                    picture: Some((species, name.to_string())),
                    icon: None,
                    label: title_case(name),
                    sub: String::new(),
                    on: n == current,
                    action: Some(Action::SetSetting(keys.clone(), with(at, format!("{}:{name}", species.name())))),
                    remove: None,
                })
                .collect();
            list.push(Row { keys: Vec::new(), label: "Coat".into(), about: "Its own colours, or the wallpaper's".into(), control: Control::Gallery(tiles) });
        }
    }
    if !pets.is_empty() {
        list.push(section("All of them", ""));
        list.push(row(&["pet", "sounds"], "Sounds", "Meows, yips, chirps, roars and bleats now and then; purrs when petted", Control::Switch(c.pet.sounds)));
        list.push(row(&["pet", "volume"], "Their volume", "", Control::Stepper { value: c.pet.volume as i64, min: 10, max: 100, step: 10, unit: "%" }));
        list.push(row(&["pet", "size"], "Their size", "", Control::Stepper { value: c.pet.size as i64, min: 70, max: 140, step: 10, unit: "%" }));
        let bed = c.pet.bed >= 0;
        list.push(row(&[], "A bed", "Where they nap, and sleep while you're away", Control::Button(if bed { "Take it away" } else { "Put one down" }, Action::PetBed(!bed))));
    }
    list
}

/// What a page holds, from the config as it is now.
pub fn rows(page: usize, c: &Config, model: &Model, pick: &Pick) -> Vec<Row> {
    let lower = |s: &str| s.to_lowercase();
    match page {
        0 => vec![
            section("Colours", "Grown from the wallpaper"),
            row(
                &["colors", "bar_icons"],
                "Bar icons",
                "In the wallpaper's colours, the accent, a colour for each group, or plain",
                choice(&[("Tinted", "tinted"), ("Accent", "accent"), ("Colourful", "colourful"), ("Plain", "plain")], if c.colors.bar_icons.trim().is_empty() { "tinted" } else { c.colors.bar_icons.trim() }),
            ),
            row(&["colors", "widgets"], "Desktop widgets", "The clock, headings and cards in the wallpaper's colours, or plain", choice(&[("Tinted", "tinted"), ("Plain", "plain")], if c.colors.widgets.trim().is_empty() { "tinted" } else { c.colors.widgets.trim() })),
            section("Frosted glass", "The compositor blurs what's behind (niri 26, and others that offer it)"),
            row(&["blur", "dock"], "Dock", "", Control::Switch(c.blur.dock)),
            row(&["blur", "launcher"], "App launcher", "", Control::Switch(c.blur.launcher)),
            row(&["blur", "overview"], "Window overview", "", Control::Switch(c.blur.overview)),
            row(&["blur", "wallpapers"], "Wallpaper picker", "", Control::Switch(c.blur.wallpapers)),
            row(&["blur", "control"], "Control centre", "Not with the frame style, where it's joined to the frame", Control::Switch(c.blur.control)),
            row(&["blur", "notifications"], "Notifications", "The pop-ups and the notification centre", Control::Switch(c.blur.notifications)),
            row(&["widgets", "blur"], "Desktop widgets", "Goes well with a lighter widget background (Desktop)", Control::Switch(c.widgets.blur)),
            section("Your apps' colours", "The wallpaper's colours written for them; each needs one line in its own config"),
            in_list(&["colors", "apps"], &c.colors.apps, "kitty", "kitty", "kitty.conf: include asterism-colors.conf"),
            in_list(&["colors", "apps"], &c.colors.apps, "foot", "foot", "foot.ini: include=~/.config/foot/asterism-colors.ini"),
            in_list(&["colors", "apps"], &c.colors.apps, "alacritty", "Alacritty", "alacritty.toml: import ~/.config/alacritty/asterism-colors.toml"),
            in_list(&["colors", "apps"], &c.colors.apps, "gtk", "GTK apps", "gtk.css: @import url(\"asterism-colors.css\");"),
        ],
        1 => {
            // The date's choices shown as today would be
            let today = chrono::Local::now();
            let dates: Vec<(String, &str)> = ["%A, %B %-d", "%a %-d %b", "%d/%m/%Y"].iter().map(|f| (today.format(f).to_string(), *f)).collect();
            let mut date_options: Vec<(&str, &str)> = dates.iter().map(|(l, f)| (l.as_str(), *f)).collect();
            date_options.push(("None", ""));
            vec![
                section("Layout", ""),
                row(&["style"], "Style", "A slim line, a frame round the screen, floating, islands, or bare", choice(&[("Line", "line"), ("Frame", "frame"), ("Floating", "floating"), ("Islands", "islands"), ("Bare", "transparent")], &lower(&format!("{:?}", c.style)))),
                row(&["position"], "Edge", "Where the bar sits", choice(&[("Top", "top"), ("Bottom", "bottom")], &lower(&format!("{:?}", c.position)))),
                row(&["size"], "Size", "How tall the bar is, and its text", choice(&[("Compact", "compact"), ("Normal", "normal"), ("Large", "large")], &lower(&format!("{:?}", c.size)))),
                row(&["icons"], "Icon set", "Thin outlines, rounded, or filled", choice(&[("Outline", "outline"), ("Material", "material"), ("Phosphor", "phosphor")], &c.icons)),
                row(&["page_style"], "Workspaces", "Dots (busy ones joined by a line), numbers, or kanji (一 二 三)", choice(&[("Dots", "dots"), ("Numbers", "numbers"), ("Kanji", "kanji")], &lower(&format!("{:?}", c.page_style)))),
                row(&["pages"], "Workspaces shown", "At least this many, empty ones too (niri keeps only those in use)", Control::Stepper { value: c.pages as i64, min: 4, max: 10, step: 1, unit: "" }),
                section("Arrangement", "Each part reads from top to bottom, as the bar does from left to right"),
                arrangement(c, pick.bar.as_deref()),
                row(
                    &[],
                    "On the bar itself",
                    "Drag items along the bar, or off it to take them away; a middle-button drag works any time",
                    Control::Button(if pick.arranging { "Done" } else { "Arrange" }, Action::ArrangeBar(Some(!pick.arranging))),
                ),
                section("What it shows", ""),
                row(&["percent"], "Numbers beside icons", "Volume, battery and the machine's load as percentages", Control::Switch(c.percent)),
                resource(&c.resources, "temperature", "Temperature gauge", "The processor's heat as a ring beside the load"),
                row(&["media_title"], "The song's title", "Slides by when long, or is cut short", choice(&[("Slide", "scroll"), ("Short", "short"), ("Off", "off")], &lower(&format!("{:?}", c.media_title)))),
                row(&["song_popup"], "New song card", "Its cover and name for a moment, when the song changes", Control::Switch(c.song_popup)),
                day_button(c),
                section("Clock", ""),
                row(&["clock", "hours24"], "24-hour clock", "", Control::Switch(c.clock.hours24)),
                row(&["clock", "seconds"], "Seconds", "", Control::Switch(c.clock.seconds)),
                row(&["clock", "date"], "Date beside the time", "", choice(&date_options, &c.clock.date)),
                section("Privacy", "A pill beside the status icons while an app uses one of these; click it to see which"),
                row(&["privacy", "microphone"], "Microphone", "Right-click the pill to mute it", Control::Switch(c.privacy.microphone)),
                row(&["privacy", "camera"], "Camera", "", Control::Switch(c.privacy.camera)),
                row(&["privacy", "screen"], "Screen sharing and recording", "On niri the pill's menu can stop it", Control::Switch(c.privacy.screen)),
                section("Corners", "Push the pointer into a corner of the screen"),
                row(&["hot_corner"], "Control centre", "The top-right corner", Control::Switch(c.hot_corner)),
                row(&["day", "corner"], "Day panel", "The top-left corner: calendar, timers, to-do, notes (clicking the clock opens it too)", Control::Switch(c.day.corner)),
                section("Day panel", ""),
                row(&["day", "folder"], "To-do and notes folder", "Where todo.md and notes.md are kept", Control::Text(c.day.folder.clone(), "~/Documents/Asterism".into())),
            ]
        }
        2 => {
            let w = &c.wallpaper;
            if !w.enabled {
                return vec![row(&["wallpaper", "enabled"], "Draw the wallpaper", "Off: the desktop is left to another program", Control::Switch(w.enabled))];
            }
            let mut list = vec![
                section("Picture", "Its colours become the shell's"),
                row(&[], "Choose a picture", "Your wallpapers as a grid", Control::Button("Open", Action::ToggleWallpapers)),
                row(&[], "Wallpaper folder", &format!("Pictures come from {}", crate::wallpapers::short(&crate::wallpapers::directory(w))), Control::Button("Choose…", Action::ChooseWallpaperFolder)),
                row(&["wallpaper", "fit"], "Fit", "Fill the screen and crop, show it whole, or stretch it", choice(&[("Cover", "cover"), ("Contain", "contain"), ("Stretch", "stretch")], &lower(&format!("{:?}", w.fit)))),
                section("Changing", ""),
                row(&["wallpaper", "interval"], "Change by itself", "Minutes between pictures from the folder; 0 never", Control::Stepper { value: w.interval as i64, min: 0, max: 240, step: 5, unit: "min" }),
                row(
                    &["wallpaper", "transition"],
                    "Animation",
                    "How one picture gives way to the next; Random is a different one each time",
                    choice(&[("Random", "random"), ("Fade", "fade"), ("Wipe", "wipe"), ("Circle", "circle"), ("Blinds", "blinds"), ("Ink", "ink"), ("Pixels", "pixels"), ("Slide", "slide"), ("None", "none")], &w.transition),
                ),
            ];
            if !w.transition.eq_ignore_ascii_case("none") {
                list.push(row(&["wallpaper", "transition_ms"], "Animation length", "", Control::Stepper { value: w.transition_ms as i64, min: 400, max: 3000, step: 200, unit: "ms" }));
            }
            list.push(section("Moving wallpapers", "Videos and animated GIFs"));
            list.push(row(&["wallpaper", "live"], "Play them", "Off keeps their first frame", Control::Switch(w.live)));
            if w.live {
                list.push(row(&["wallpaper", "live_fps"], "Frame rate", "Fewer frames cost less", Control::Stepper { value: w.live_fps as i64, min: 10, max: 60, step: 5, unit: "fps" }));
                list.push(row(&["wallpaper", "live_scale"], "Sharpness", "Made at this share of the screen's size, then scaled up; less costs less", Control::Stepper { value: w.live_scale as i64, min: 25, max: 100, step: 25, unit: "%" }));
                list.push(row(&["wallpaper", "live_battery"], "Play on battery", "Off: still until the charger's back", Control::Switch(w.live_battery)));
                list.push(row(&["wallpaper", "live_idle"], "Still when you're away", "Minutes without input before it stops; 0 never", Control::Stepper { value: w.live_idle as i64, min: 0, max: 60, step: 1, unit: "min" }));
            }
            list.push(section("Other", ""));
            list.push(row(&["wallpaper", "enabled"], "Draw the wallpaper", "Off: the desktop is left to another program", Control::Switch(w.enabled)));
            list
        }
        3 => {
            let shown = |kind: &str| c.widgets.show.iter().any(|k| k == kind);
            let mut list = vec![
                section("Widgets", "On the wallpaper, under your windows"),
                row(&[], "Arrange them", "Add, take off, move and resize them on the desktop itself (or right-click the desktop)", Control::Button("Arrange", Action::EditWidgets(Some(true)))),
            ];
            for kind in crate::widgets::KINDS {
                let about = match kind {
                    "clock" => "The time and the date",
                    "weather" => "Now, and the days ahead when it's tall",
                    "media" => "The song, its cover and its progress",
                    "visualizer" => "Bars that move with the music, while something plays",
                    "edge" => "The same bars along the foot of the screen",
                    "calendar" => "This month",
                    "system" => "Processor, memory and disk",
                    "photos" => "A picture from a folder, a new one every so often",
                    "note" => "A few lines of your own",
                    _ => "",
                };
                list.push(in_list(&["widgets", "show"], &c.widgets.show, kind, crate::widgets::name(kind), about));
            }
            list.push(section("Their look", ""));
            list.push(row(&["widgets", "opacity"], "Background", "How solid their cards are; 0 leaves the words alone on the wallpaper, with a soft shadow", Control::Stepper { value: c.widgets.opacity as i64, min: 0, max: 100, step: 10, unit: "%" }));
            list.push(row(&["widgets", "outline"], "Outline", "A hairline round each card", Control::Switch(c.widgets.outline)));
            if shown("note") {
                list.push(section("Sticky note", ""));
                list.push(row(&["widgets", "note"], "It says", "Right-click the note to write here", Control::Text(c.widgets.note.clone(), "Nothing yet".into())));
            }
            if shown("photos") {
                list.push(section("Photo frame", ""));
                list.push(row(&["widgets", "photos"], "Folder", "", Control::Text(c.widgets.photos.clone(), "~/Pictures".into())));
                list.push(row(&["widgets", "photo_minutes"], "Each picture stays", "", Control::Stepper { value: c.widgets.photo_minutes as i64, min: 1, max: 120, step: 1, unit: "min" }));
            }
            if shown("media") {
                list.push(section("What's playing", ""));
                list.push(row(&["widgets", "lyrics"], "Song words", "Under the song, line by line (asks LRCLIB with the song's name)", Control::Switch(c.widgets.lyrics)));
            }
            list
        }
        4 => dock_rows(c, model),
        PETS_PAGE => pet_rows(c, &pick.pet),
        6 => {
            let n = &c.night_light;
            let mut list = vec![row(&["night_light", "mode"], "Night light", "Warmer colours: always, or from evening to morning", choice(&[("Off", "off"), ("On", "on"), ("Schedule", "schedule")], &n.mode))];
            if !n.mode.eq_ignore_ascii_case("off") {
                list.push(row(&["night_light", "temperature"], "Warmth", "Lower is warmer; 6500 K is the screen as it is", Control::Stepper { value: n.temperature as i64, min: 2500, max: 6000, step: 250, unit: "K" }));
            }
            if n.mode.eq_ignore_ascii_case("schedule") {
                list.push(section("Schedule", "Sunset and sunrise where the weather is"));
                list.push(row(&["night_light", "start"], "From", "", choice(&[("Sunset", "sunset"), ("19:00", "19:00"), ("20:00", "20:00"), ("21:00", "21:00"), ("22:00", "22:00")], &n.start)));
                list.push(row(&["night_light", "end"], "Until", "", choice(&[("Sunrise", "sunrise"), ("06:00", "06:00"), ("07:00", "07:00"), ("08:00", "08:00")], &n.end)));
            }
            list
        }
        7 => vec![
            row(&["lock", "idle"], "Lock when idle", "Minutes without input, 0 never; Stay awake holds it off", Control::Stepper { value: c.lock.idle as i64, min: 0, max: 120, step: 5, unit: "min" }),
            row(&["lock", "before_sleep"], "Lock before sleep", "Closing the lid, or suspend from anywhere", Control::Switch(c.lock.before_sleep)),
            row(&["lock", "fingerprint"], "Fingerprint", "Unlock with a finger too, when there's a reader fprintd knows", Control::Switch(c.lock.fingerprint)),
            row(&[], "Lock now", "", Control::Button("Lock", Action::LockNow)),
        ],
        8 => {
            let mut list = vec![
                section("Pop-ups", ""),
                row(&["notifications", "timeout"], "They stay", "Seconds, when the app doesn't say", Control::Stepper { value: c.notifications.timeout as i64, min: 2, max: 30, step: 1, unit: "s" }),
                row(&["notifications", "max_popups"], "At once", "More wait their turn", Control::Stepper { value: c.notifications.max_popups as i64, min: 1, max: 8, step: 1, unit: "" }),
                section("Battery warnings", "At these charges; 0 turns one off"),
                row(&["notifications", "battery_low"], "Low at", "", Control::Stepper { value: c.notifications.battery_low as i64, min: 0, max: 50, step: 5, unit: "%" }),
                row(&["notifications", "battery_critical"], "Critical at", "", Control::Stepper { value: c.notifications.battery_critical as i64, min: 0, max: 20, step: 1, unit: "%" }),
            ];
            // Every app that has notified, and every app with a rule
            let mut apps: Vec<String> = c.notifications.apps.keys().cloned().collect();
            for n in &model.notifications.list {
                if !n.app.is_empty() && !apps.iter().any(|a| a.eq_ignore_ascii_case(&n.app)) {
                    apps.push(n.app.clone());
                }
            }
            apps.sort_by_key(|a| a.to_lowercase());
            if !apps.is_empty() {
                list.push(section("Apps", "Each pops up, stays quietly in the notification centre, or is dropped; apps appear here once they've notified"));
            }
            for app in apps {
                let rule = c.notifications.apps.iter().find(|(k, _)| k.eq_ignore_ascii_case(&app)).map(|(_, r)| *r).unwrap_or_default();
                let at = match rule {
                    crate::config::AppRule::Allow => 0,
                    crate::config::AppRule::Silence => 1,
                    crate::config::AppRule::Discard => 2,
                };
                list.push(Row {
                    keys: vec!["notifications".into(), "apps".into(), app.clone()],
                    label: app,
                    about: String::new(),
                    control: Control::Choice(vec![("Pop up".into(), Value::Remove), ("Quiet".into(), Value::Str("silence".into())), ("Drop".into(), Value::Str("discard".into()))], Some(at)),
                });
            }
            list
        }
        9 => {
            let mut list = vec![row(&["weather", "enabled"], "Weather", "From Open-Meteo: only the place is sent", Control::Switch(c.weather.enabled))];
            if c.weather.enabled {
                list.push(row(&["weather", "city"], "Place", "A city's name", Control::Text(c.weather.city.clone(), "Your time zone's city".into())));
                list.push(row(&["weather", "units"], "Units", "", choice(&[("°C, km/h", "metric"), ("°F, mph", "imperial")], &c.weather.units)));
            }
            list
        }
        _ => vec![
            section("Focus rounds", "Work, then a break; a longer one after a few rounds"),
            row(&["timers", "focus"], "Focus", "", Control::Stepper { value: c.timers.focus as i64, min: 5, max: 120, step: 5, unit: "min" }),
            row(&["timers", "short_break"], "Short break", "", Control::Stepper { value: c.timers.short_break as i64, min: 1, max: 30, step: 1, unit: "min" }),
            row(&["timers", "long_break"], "Long break", "", Control::Stepper { value: c.timers.long_break as i64, min: 5, max: 60, step: 5, unit: "min" }),
            row(&["timers", "rounds"], "Long break after", "Rounds of focus", Control::Stepper { value: c.timers.rounds as i64, min: 2, max: 8, step: 1, unit: "" }),
        ],
    }
}

/// Writes one value into the config file, keeping everything else as it was.
#[cfg(test)]
pub fn write(path: &Path, keys: &[String], value: &Value) -> anyhow::Result<()> {
    write_all(path, &[(keys.to_vec(), value.clone())])
}

/// Several values at once (the bar's three parts), in one read and one write of the file.
pub fn write_all(path: &Path, writes: &[(Vec<String>, Value)]) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    for (keys, value) in writes {
        put(&mut doc, keys, value)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

fn put(doc: &mut toml_edit::DocumentMut, keys: &[String], value: &Value) -> anyhow::Result<()> {
    let Some((last, tables)) = keys.split_last() else { return Ok(()) };
    let mut table = doc.as_table_mut();
    for key in tables {
        if !table.contains_key(key) {
            let mut fresh = toml_edit::Table::new();
            fresh.set_implicit(false);
            table.insert(key, toml_edit::Item::Table(fresh));
        }
        table = table.get_mut(key).and_then(|item| item.as_table_mut()).ok_or_else(|| anyhow::anyhow!("[{key}] isn't a table here"))?;
    }
    let mut new = match value {
        Value::Remove => {
            table.remove(last);
            None
        }
        Value::Bool(b) => Some(toml_edit::Value::from(*b)),
        Value::Int(i) => Some(toml_edit::Value::from(*i)),
        Value::Str(s) => Some(toml_edit::Value::from(s.as_str())),
        Value::List(list) => Some(toml_edit::Value::Array(list.iter().map(String::as_str).collect())),
    };
    if let Some(new) = new.as_mut() {
        // The comment after the old value (`= 10  # minutes`) stays with the new one
        if let Some(old) = table.get(last).and_then(|item| item.as_value()) {
            *new.decor_mut() = old.decor().clone();
        }
        table[last.as_str()] = toml_edit::Item::Value(new.clone());
    }
    Ok(())
}

pub struct Settings {
    pub page: usize,
    /// A text being typed: where it goes, and what's typed so far.
    pub editing: Option<(Vec<String>, String)>,
    /// How far the page is scrolled, and how far it can be.
    pub scroll: f32,
    pub max_scroll: f32,
    /// What's typed in the search field: the rows of every page that match, instead of a page.
    pub query: String,
    /// The pet the Pets page shows, or its animals to add.
    pub pet: PetPick,
    /// The bar's item picked to move (Bar › Arrangement).
    pub bar: Option<String>,
    /// Arrange mode is on in the bar (its button says Done).
    pub arranging: bool,
}

/// The rows of every page whose name, description or page matches `query` (any case); each
/// says which page it's on.
pub fn search(query: &str, c: &Config, model: &Model, pick: &Pick) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut found = Vec::new();
    for (page, name) in PAGES.iter().enumerate() {
        let mut group = String::new();
        for mut row in rows(page, c, model, pick) {
            if let Control::Section = row.control {
                group = row.label.clone();
                continue;
            }
            // Where it is: the page, and its group there
            let place = if group.is_empty() { name.to_string() } else { format!("{name} › {group}") };
            let haystack = format!("{} {} {}", row.label, row.about, place).to_lowercase();
            if query.split_whitespace().all(|word| haystack.contains(word)) {
                row.about = if row.about.is_empty() { place } else { format!("{place} · {}", row.about) };
                found.push(row);
            }
        }
    }
    found
}

/// How wide a row's control is.
fn control_width(control: &Control, text: &mut Text, font: Font) -> f32 {
    let pill = |label: &str, text: &mut Text| text.measure(label, font).0 + 26.0;
    match control {
        Control::Switch(_) | Control::SwitchTo(..) => 44.0,
        Control::Choice(options, _) => options.iter().map(|(label, _)| pill(label, text)).sum::<f32>() + 8.0 * options.len().saturating_sub(1) as f32,
        Control::Stepper { .. } => pill("+", text) + 92.0 + pill("−", text),
        Control::Text(..) => 240.0,
        Control::Section => 0.0,
        Control::Button(label, _) => pill(label, text),
        // Always under their label, the row's whole width
        Control::Gallery(_) | Control::Arrange { .. } => f32::MAX,
    }
}

impl Settings {
    pub fn new() -> Self {
        Self { page: 0, editing: None, scroll: 0.0, max_scroll: 0.0, query: String::new(), pet: PetPick::default(), bar: None, arranging: false }
    }

    /// The app's window: the pages down a sidebar, the page's rows on a card beside them.
    /// The rows scroll under the page's title when they don't fit.
    pub fn build(&mut self, c: &Config, model: &Model, t: &Theme, text: &mut Text, w: f32, h: f32) -> Vec<Piece> {
        let mut pieces = Vec::new();

        // The sidebar: the app's name, the pages, and where changes go. Held anywhere empty,
        // it moves the window
        let side_w = if w < 820.0 { 196.0 } else { 236.0 };
        let mut side = Piece::new("set.side", side_w, h);
        side.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: side_w, h, radius: 0.0, color: t.card_deep });
        side.prims.push(Prim::Rect { x: side_w - 1.0, y: 0.0, w: 1.0, h, radius: 0.0, color: t.hairline() });
        let title_font = Font::new(t.text + 7.0).weight(500);
        side.prims.push(Prim::Text { x: 24.0, y: 26.0, text: "Settings".into(), font: title_font, color: t.color_text });
        let note_font = Font::new(t.text - 2.0);
        let note = text.wrap("Saved to config.toml as you change things", note_font, side_w - 48.0, 2);
        let (_, nh) = text.measure("Ag", note_font);
        // Only where the pages leave room for it
        let pages_end = 120.0 + 44.0 * PAGES.len() as f32;
        let note = if h - 24.0 - nh * note.len() as f32 > pages_end + 16.0 { note } else { Vec::new() };
        for (i, line) in note.iter().rev().enumerate() {
            side.prims.push(Prim::Text { x: 24.0, y: h - 24.0 - nh * (i + 1) as f32, text: line.clone(), font: note_font, color: t.color_soft.alpha(0.75) });
        }
        side.actions.left = Some(Action::MoveWindow);
        pieces.push(side);
        // The search field: typing anywhere in the window (not into a value) goes here
        let (sw, sh) = (side_w - 24.0, 34.0);
        let mut field = Piece::new("set.search", sw, sh);
        let searching = !self.query.is_empty();
        field.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: sw, h: sh, radius: sh / 2.0, color: if searching { t.primary } else { t.hairline() } });
        field.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: sw - 2.0, h: sh - 2.0, radius: sh / 2.0 - 1.0, color: t.surface });
        field.prims.push(Prim::Icon { x: 12.0, y: (sh - 16.0) / 2.0, size: 16.0, name: "search".into(), color: t.color_soft });
        let field_font = Font::new(t.text);
        let shown = if searching { crate::bar::Ctx::ellipsize_with(text, &self.query, field_font, sw - 50.0) } else { "Search".into() };
        let (qw, qh) = text.measure(&shown, field_font);
        field.prims.push(Prim::Text { x: 36.0, y: ((sh - qh) / 2.0).round(), text: shown, font: field_font, color: if searching { t.color_text } else { t.color_soft } });
        if searching {
            field.prims.push(Prim::Rect { x: 37.0 + qw, y: 9.0, w: 2.0, h: sh - 18.0, radius: 1.0, color: t.primary });
        }
        field.blocks = true;
        field.x = 12.0;
        field.y = 72.0;
        pieces.push(field);
        let nav_font = Font::new(t.text + 1.0);
        let mut y = 120.0;
        for (i, name) in PAGES.iter().enumerate() {
            let (iw, ih) = (side_w - 24.0, 40.0);
            let mut item = Piece::new(format!("set.page.{i}"), iw, ih);
            // While searching no page is the one shown
            let on = i == self.page && self.query.trim().is_empty();
            if on {
                item.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: iw, h: ih, radius: ih / 2.0, color: t.secondary_container });
            }
            let color = if on { t.on_secondary_container } else { t.color_soft };
            item.prims.push(Prim::Icon { x: 14.0, y: 11.0, size: 18.0, name: PAGE_ICONS[i].into(), color: if on { t.primary } else { color } });
            let (_, nh) = text.measure(name, nav_font);
            item.prims.push(Prim::Text { x: 44.0, y: ((ih - nh) / 2.0).round(), text: name.to_string(), font: nav_font, color });
            item.highlight = !on;
            item.actions.left = Some(Action::SettingsPage(i));
            item.x = 12.0;
            item.y = y;
            pieces.push(item);
            y += 44.0;
        }

        // The page: its rows on a card, as wide as reads well
        let room = w - side_w;
        let pad = if room < 640.0 { 28.0 } else { 44.0 };
        let cw = (room - pad * 2.0).clamp(240.0, 780.0);
        let cx = side_w + pad;
        let head_h = 92.0;
        let (ix, iw) = (cx + 22.0, cw - 44.0);
        let label_font = Font::new(t.text + 1.0).weight(500);
        let about_font = Font::new(t.text - 2.0);
        let control_font = Font::new(t.text - 1.0).weight(500);
        let (_, lh) = text.measure("Ag", label_font);
        let (_, ah) = text.measure("Ag", about_font);
        // Each group's rows on a card of their own (where it starts: its place among the pieces,
        // and its top), the group's title over it
        let mut card: Option<(usize, f32)> = None;
        let close = |pieces: &mut Vec<Piece>, card: &mut Option<(usize, f32)>, y: &mut f32| {
            let Some((index, card_top)) = card.take() else { return };
            *y += 6.0;
            let card_h = *y - card_top;
            let mut back = Piece::new(format!("set.card.{index}"), cw, card_h);
            back.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: cw, h: card_h, radius: t.radius, color: t.hairline() });
            back.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: cw - 2.0, h: card_h - 2.0, radius: t.radius - 1.0, color: t.card_deep });
            back.x = cx;
            back.y = card_top;
            pieces.insert(index, back);
        };
        let top = head_h + 4.0 - self.scroll;
        let mut y = top;
        let pick = Pick { pet: self.pet, bar: self.bar.clone(), arranging: self.arranging };
        let rows = if self.query.trim().is_empty() { rows(self.page, c, model, &pick) } else { search(&self.query, c, model, &pick) };
        if rows.is_empty() {
            let words = format!("Nothing here matches “{}”", self.query.trim());
            let (nw, nh) = text.measure(&words, about_font);
            let mut none = Piece::new("set.none", nw, nh);
            none.prims.push(Prim::Text { x: 0.0, y: 0.0, text: words, font: about_font, color: t.color_soft });
            none.x = cx + 4.0;
            none.y = head_h + 12.0;
            pieces.push(none);
        }
        let count = rows.len();
        let breaks: Vec<bool> = rows.iter().map(|row| matches!(row.control, Control::Section)).collect();
        let section_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
        for (r, row) in rows.into_iter().enumerate() {
            if let Control::Section = row.control {
                // A group's title, in small spaced capitals, and what it's for
                close(&mut pieces, &mut card, &mut y);
                if r > 0 {
                    y += 18.0;
                }
                let title = row.label.to_uppercase();
                let (_, th) = text.measure(&title, section_font);
                let about = (!row.about.is_empty()).then(|| crate::bar::Ctx::ellipsize_with(text, &row.about, about_font, cw - 8.0));
                let head = th + about.as_ref().map_or(0.0, |_| ah + 2.0);
                let mut group = Piece::new(format!("set.section.{r}"), cw, head);
                group.prims.push(Prim::Text { x: 4.0, y: 0.0, text: title, font: section_font, color: t.primary });
                if let Some(about) = about {
                    group.prims.push(Prim::Text { x: 4.0, y: th + 2.0, text: about, font: about_font, color: t.color_soft });
                }
                group.x = cx;
                group.y = y + 10.0;
                pieces.push(group);
                y += head + 20.0;
                continue;
            }
            if card.is_none() {
                card = Some((pieces.len(), y));
                y += 6.0;
            }
            // The control beside its label; when there isn't room for both, under it
            let ctrl_w = control_width(&row.control, text, control_font);
            let stacked = iw - ctrl_w - 24.0 < (iw * 0.4).min(220.0);
            // Choices wider than the row go on as many lines as they need, under the label
            let lines: Vec<Vec<usize>> = match &row.control {
                Control::Choice(options, _) if stacked && ctrl_w > iw => {
                    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
                    let mut used = 0.0;
                    for (i, (label, _)) in options.iter().enumerate() {
                        let pw = text.measure(label, control_font).0 + 26.0;
                        if used + pw > iw && !lines.last().is_some_and(Vec::is_empty) {
                            lines.push(Vec::new());
                            used = 0.0;
                        }
                        used += pw + 8.0;
                        if let Some(line) = lines.last_mut() {
                            line.push(i);
                        }
                    }
                    lines
                }
                _ => Vec::new(),
            };
            // A gallery: its pictures in as many lines as they need, under the label
            let per_line = (((iw + TILE_GAP) / (TILE.0 + TILE_GAP)).floor() as usize).max(1);
            let gallery_top = 14.0 + lh + if row.about.is_empty() { 0.0 } else { ah } + 14.0;
            // The bar's arrangement, built once here for its height
            let arranged = match &row.control {
                Control::Arrange { parts, hidden, picked, usual } => Some(arrange_pieces(parts, hidden, picked.as_deref(), *usual, t, text, iw)),
                _ => None,
            };
            let row_h = match &row.control {
                Control::Gallery(tiles) => gallery_top + tiles.len().div_ceil(per_line) as f32 * (TILE.1 + TILE_GAP) + 6.0,
                Control::Arrange { .. } => gallery_top + arranged.as_ref().map_or(0.0, |(_, h)| *h) + 18.0,
                _ if stacked => 104.0 + 40.0 * lines.len().saturating_sub(1) as f32,
                _ => 66.0,
            };
            let label_w = if stacked { iw } else { iw - ctrl_w - 24.0 };
            let mut line = Piece::new(format!("set.row.{r}"), iw, row_h);
            let top_y = match (stacked, row.about.is_empty()) {
                (true, _) => 14.0,
                (false, true) => (row_h - lh) / 2.0,
                (false, false) => (row_h - lh - ah) / 2.0,
            };
            let label = crate::bar::Ctx::ellipsize_with(text, &row.label, label_font, label_w);
            line.prims.push(Prim::Text { x: 0.0, y: top_y, text: label, font: label_font, color: t.color_text });
            if !row.about.is_empty() {
                let about = crate::bar::Ctx::ellipsize_with(text, &row.about, about_font, label_w);
                line.prims.push(Prim::Text { x: 0.0, y: top_y + lh, text: about, font: about_font, color: t.color_soft });
            }
            // A line between rows of the same card
            if r + 1 < count && !breaks[r + 1] {
                line.prims.push(Prim::Line { points: vec![(0.0, row_h - 0.5), (iw, row_h - 0.5)], width: 1.0, color: t.outline_variant.alpha(0.5) });
            }
            line.x = ix;
            line.y = y;
            pieces.push(line);

            // The control: at the right, or at the left under the label
            let right = if stacked { ix + ctrl_w } else { ix + iw };
            let at_y = |hc: f32| if stacked { y + 58.0 + (36.0 - hc) / 2.0 } else { y + (row_h - hc) / 2.0 };
            let pill = |pieces: &mut Vec<Piece>, key: String, label: &str, x: f32, filled: bool, action: Option<Action>, text: &mut Text| -> f32 {
                let (lw, lh) = text.measure(label, control_font);
                let (bw, bh) = (lw + 26.0, 32.0);
                let mut p = Piece::new(key, bw, bh);
                if filled {
                    p.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.secondary_container });
                } else {
                    p.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.hairline() });
                    p.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: bw - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
                }
                p.prims.push(Prim::Text { x: 13.0, y: ((bh - lh) / 2.0).round(), text: label.to_string(), font: control_font, color: if filled { t.on_secondary_container } else { t.color_text } });
                p.highlight = true;
                p.actions.left = action;
                p.x = x - bw;
                p.y = at_y(bh);
                pieces.push(p);
                bw
            };
            let set = |value: Value| Some(Action::SetSetting(row.keys.clone(), value));
            let switch = |pieces: &mut Vec<Piece>, on: bool, action: Option<Action>| {
                let (sw, sh) = (44.0, 24.0);
                let mut s = Piece::new(format!("set.switch.{r}"), sw, sh);
                s.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: sw, h: sh, radius: sh / 2.0, color: if on { t.primary } else { t.outline_variant } });
                let k = 16.0;
                let kx = if on { sw - k - 4.0 } else { 4.0 };
                s.prims.push(Prim::Rect { x: kx, y: (sh - k) / 2.0, w: k, h: k, radius: k / 2.0, color: if on { t.on_primary } else { t.color_soft } });
                s.highlight = true;
                s.actions.left = action;
                s.x = right - sw;
                s.y = at_y(sh);
                pieces.push(s);
            };
            match row.control {
                Control::Switch(on) => switch(&mut pieces, on, set(Value::Bool(!on))),
                Control::SwitchTo(on, value) => switch(&mut pieces, on, set(value)),
                Control::Choice(options, current) if !lines.is_empty() => {
                    for (j, line) in lines.iter().enumerate() {
                        let mut x = ix;
                        for &i in line {
                            let (label, value) = &options[i];
                            let pw = text.measure(label, control_font).0 + 26.0;
                            pill(&mut pieces, format!("set.choice.{r}.{i}"), label, x + pw, current == Some(i), set(value.clone()), text);
                            if let Some(p) = pieces.last_mut() {
                                p.y += 40.0 * j as f32;
                            }
                            x += pw + 8.0;
                        }
                    }
                }
                Control::Choice(options, current) => {
                    let mut x = right;
                    for (i, (label, value)) in options.iter().enumerate().rev() {
                        let w = pill(&mut pieces, format!("set.choice.{r}.{i}"), label, x, current == Some(i), set(value.clone()), text);
                        x -= w + 8.0;
                    }
                }
                Control::Stepper { value, min, max, step, unit } => {
                    let plus = pill(&mut pieces, format!("set.plus.{r}"), "+", right, false, (value < max).then(|| Action::SetSetting(row.keys.clone(), Value::Int((value + step).min(max)))), text);
                    let shown = if unit.is_empty() { value.to_string() } else { format!("{value} {unit}") };
                    let (vw, vh) = text.measure(&shown, label_font);
                    let mut v = Piece::new(format!("set.value.{r}"), 92.0, 32.0);
                    v.prims.push(Prim::Text { x: ((92.0 - vw) / 2.0).round(), y: ((32.0 - vh) / 2.0).round(), text: shown, font: label_font, color: t.color_text });
                    v.x = right - plus - 92.0;
                    v.y = at_y(32.0);
                    pieces.push(v);
                    pill(&mut pieces, format!("set.minus.{r}"), "−", right - plus - 92.0, false, (value > min).then(|| Action::SetSetting(row.keys.clone(), Value::Int((value - step).max(min)))), text);
                }
                Control::Text(current, placeholder) => {
                    let editing = self.editing.as_ref().filter(|(k, _)| *k == row.keys).map(|(_, typed)| typed.clone());
                    let (fw, fh) = (240.0, 34.0);
                    let mut f = Piece::new(format!("set.text.{r}"), fw, fh);
                    f.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: fh, radius: fh / 2.0, color: if editing.is_some() { t.primary } else { t.hairline() } });
                    f.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: fh - 2.0, radius: fh / 2.0 - 1.0, color: t.surface });
                    // Empty, it shows what's used instead, quietly
                    let shown = editing.clone().unwrap_or_else(|| if current.is_empty() { placeholder.clone() } else { current.clone() });
                    let shown = crate::bar::Ctx::ellipsize_with(text, &shown, control_font, fw - 30.0);
                    let (sw, sh) = text.measure(&shown, control_font);
                    f.prims.push(Prim::Text { x: 14.0, y: ((fh - sh) / 2.0).round(), text: shown, font: control_font, color: if editing.is_some() || !current.is_empty() { t.color_text } else { t.color_soft } });
                    if editing.is_some() {
                        f.prims.push(Prim::Rect { x: 15.0 + sw, y: 8.0, w: 2.0, h: fh - 16.0, radius: 1.0, color: t.primary });
                    }
                    f.highlight = true;
                    f.actions.left = Some(Action::SettingsEdit(row.keys.clone()));
                    f.x = right - fw;
                    f.y = at_y(fh);
                    pieces.push(f);
                }
                Control::Button(label, action) => {
                    pill(&mut pieces, format!("set.button.{r}"), label, right, true, Some(action), text);
                }
                Control::Section => {}
                Control::Arrange { .. } => {
                    for mut piece in arranged.map(|(pieces, _)| pieces).unwrap_or_default() {
                        piece.x += ix;
                        piece.y += y + gallery_top;
                        pieces.push(piece);
                    }
                }
                Control::Gallery(tiles) => {
                    let (tw, th) = TILE;
                    let name_font = Font::new(t.text - 1.0).weight(500);
                    for (n, tile) in tiles.into_iter().enumerate() {
                        let tx = ix + (n % per_line) as f32 * (tw + TILE_GAP);
                        let ty = y + gallery_top + (n / per_line) as f32 * (th + TILE_GAP);
                        let mut card = Piece::new(format!("set.tile.{r}.{n}"), tw, th);
                        if tile.on {
                            card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: tw, h: th, radius: 16.0, color: t.primary });
                            card.prims.push(Prim::Rect { x: 2.0, y: 2.0, w: tw - 4.0, h: th - 4.0, radius: 14.0, color: t.secondary_container });
                        } else {
                            card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: tw, h: th, radius: 16.0, color: t.hairline() });
                            card.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: tw - 2.0, h: th - 2.0, radius: 15.0, color: t.surface });
                        }
                        if let Some(icon) = tile.icon {
                            card.prims.push(Prim::Rect { x: (tw - 48.0) / 2.0, y: 26.0, w: 48.0, h: 48.0, radius: 24.0, color: if tile.on { t.primary } else { t.primary_container } });
                            card.prims.push(Prim::Icon { x: (tw - 24.0) / 2.0, y: 38.0, size: 24.0, name: icon.into(), color: if tile.on { t.on_primary } else { t.on_primary_container } });
                        }
                        let ink = if tile.on { t.on_secondary_container } else { t.color_text };
                        let label = crate::bar::Ctx::ellipsize_with(text, &tile.label, name_font, tw - 12.0);
                        let (nw, nh) = text.measure(&label, name_font);
                        let name_y = if tile.sub.is_empty() { th - nh - 14.0 } else { th - nh - ah - 12.0 };
                        card.prims.push(Prim::Text { x: ((tw - nw) / 2.0).round(), y: name_y.round(), text: label, font: name_font, color: ink });
                        if !tile.sub.is_empty() {
                            let sub = crate::bar::Ctx::ellipsize_with(text, &tile.sub, about_font, tw - 12.0);
                            let (sw, _) = text.measure(&sub, about_font);
                            card.prims.push(Prim::Text { x: ((tw - sw) / 2.0).round(), y: (name_y + nh).round(), text: sub, font: about_font, color: if tile.on { ink.alpha(0.8) } else { t.color_soft } });
                        }
                        card.highlight = !tile.on;
                        card.actions.left = tile.action;
                        card.x = tx;
                        card.y = ty;
                        pieces.push(card);
                        // The animal itself, sitting, its feet on the tile's floor
                        if let Some((species, coat)) = &tile.picture {
                            let k = 0.95;
                            let mut picture = crate::pet::Pet::portrait(*species, coat, t);
                            picture.key = format!("set.pic.{r}.{n}");
                            picture.scale(k);
                            picture.x = (tx + tw / 2.0 - crate::pet::FEET.0 * k - 8.0).round();
                            picture.y = (ty + 90.0 - crate::pet::FEET.1 * k).round();
                            pieces.push(picture);
                        }
                        if let Some(remove) = tile.remove {
                            let size = 26.0;
                            let mut x = Piece::new(format!("set.untile.{r}.{n}"), size, size);
                            x.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: size, h: size, radius: size / 2.0, color: t.outline_variant });
                            x.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: size - 2.0, h: size - 2.0, radius: size / 2.0 - 1.0, color: t.container_highest });
                            x.prims.push(Prim::Icon { x: 6.0, y: 6.0, size: 14.0, name: "close".into(), color: t.color_text });
                            x.label = Some("Put it away".into());
                            x.highlight = true;
                            x.actions.left = Some(remove);
                            x.x = tx + tw - size - 6.0;
                            x.y = ty + 6.0;
                            pieces.push(x);
                        }
                    }
                }
            }
            y += row_h;
        }
        close(&mut pieces, &mut card, &mut y);
        // How far it can scroll: the last card's foot 32 px off the window's
        let content_h = y - top;
        self.max_scroll = (content_h + head_h + 4.0 + 32.0 - h).max(0.0);
        if self.scroll > self.max_scroll {
            self.scroll = self.max_scroll;
        }
        if let Some(hint) = self.editing.as_ref().map(|_| "Type, then Enter to keep it · Esc lets it be") {
            let (hw, hh) = text.measure(hint, about_font);
            let mut under = Piece::new("set.hint", hw, hh);
            under.prims.push(Prim::Text { x: 0.0, y: 0.0, text: hint.into(), font: about_font, color: t.color_soft });
            under.x = cx + cw - hw;
            under.y = y + 10.0;
            pieces.push(under);
        }

        // The page's title, over rows scrolled under it; held, it moves the window
        let mut head = Piece::new("set.head", room - 1.0, head_h);
        head.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: room - 1.0, h: head_h, radius: 0.0, color: t.surface });
        let head_font = Font::new(t.text + 11.0).weight(400);
        let (_, hh) = text.measure("Ag", head_font);
        head.prims.push(Prim::Text { x: pad, y: ((head_h - hh) / 2.0).round() + 4.0, text: if self.query.trim().is_empty() { PAGES[self.page].into() } else { "Search".into() }, font: head_font, color: t.color_text });
        if self.scroll > 0.5 {
            head.prims.push(Prim::Rect { x: 0.0, y: head_h - 1.0, w: room - 1.0, h: 1.0, radius: 0.0, color: t.hairline() });
        }
        head.actions.left = Some(Action::MoveWindow);
        head.x = side_w + 1.0;
        head.y = 0.0;
        pieces.push(head);
        let size = 36.0;
        let mut close = Piece::new("set.close", size, size);
        close.prims.push(Prim::Icon { x: 9.0, y: 9.0, size: 18.0, name: "close".into(), color: t.color_soft });
        close.highlight = true;
        close.actions.left = Some(Action::CloseSettings);
        close.x = w - size - 18.0;
        close.y = ((head_h - size) / 2.0).round() + 4.0;
        pieces.push(close);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.label.clone()).collect()
    }

    #[test]
    fn rows_that_do_nothing_are_left_out() {
        let model = Model::default();
        let pick = Pick::default();
        let mut c = Config::default();
        // Night light off: only its switch; on a schedule: its hours too
        c.night_light.mode = "off".into();
        assert_eq!(labels(&rows(6, &c, &model, &pick)), ["Night light"]);
        c.night_light.mode = "schedule".into();
        let shown = labels(&rows(6, &c, &model, &pick));
        assert!(shown.contains(&"Warmth".to_string()) && shown.contains(&"From".to_string()), "{shown:?}");
        // Moving wallpapers off: none of their knobs
        c.wallpaper.live = false;
        assert!(!labels(&rows(2, &c, &model, &pick)).contains(&"Frame rate".to_string()));
        // A widget's own settings only while it's on the desktop
        c.widgets.show = vec!["clock".into()];
        assert!(!labels(&rows(3, &c, &model, &pick)).contains(&"Folder".to_string()));
        c.widgets.show.push("photos".into());
        assert!(labels(&rows(3, &c, &model, &pick)).contains(&"Folder".to_string()));
    }

    #[test]
    fn the_day_button_goes_after_the_clock() {
        let mut c = Config::default();
        c.layout.center = vec!["resources".into(), "clock".into(), "media".into()];
        let on = day_button(&c);
        assert!(matches!(&on.control, Control::SwitchTo(false, Value::List(next)) if next == &["resources", "clock", "day", "media"]));
        c.layout.center = vec!["clock".into(), "day".into()];
        assert!(matches!(&day_button(&c).control, Control::SwitchTo(true, Value::List(next)) if next == &["clock"]));
    }

    #[test]
    fn bar_items_move_along_and_between_parts() {
        let list = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (l, c, r) = (list(&["launcher", "pages"]), list(&["clock", "media"]), list(&["tray", "status"]));
        let parts = [&l[..], &c[..], &r[..]];
        let go = |how: BarMove| arrange_bar(parts, &how);
        let at = |item: &str| item.to_string();
        // Along a part, then over its edge into the next one, on the side it came from
        assert_eq!(go(BarMove::Later(at("launcher"))).unwrap(), [list(&["pages", "launcher"]), c.clone(), r.clone()]);
        assert_eq!(go(BarMove::Later(at("media"))).unwrap(), [l.clone(), list(&["clock"]), list(&["media", "tray", "status"])]);
        assert_eq!(go(BarMove::Earlier(at("clock"))).unwrap(), [list(&["launcher", "pages", "clock"]), list(&["media"]), r.clone()]);
        // Nowhere further to go
        assert_eq!(go(BarMove::Earlier(at("launcher"))), None);
        assert_eq!(go(BarMove::Later(at("status"))), None);
        // To another part: moving right to its start, left to its end; into its own part, nothing
        assert_eq!(go(BarMove::To(at("pages"), 2)).unwrap(), [list(&["launcher"]), c.clone(), list(&["pages", "tray", "status"])]);
        assert_eq!(go(BarMove::To(at("tray"), 0)).unwrap(), [list(&["launcher", "pages", "tray"]), c.clone(), list(&["status"])]);
        assert_eq!(go(BarMove::To(at("clock"), 1)), None);
        // Off and back: back on the right it goes before status
        let off = go(BarMove::Off(at("tray"))).unwrap();
        assert_eq!(off, [l.clone(), c.clone(), list(&["status"])]);
        let back = arrange_bar([&off[0], &off[1], &off[2]], &BarMove::To(at("tray"), 2)).unwrap();
        assert_eq!(back, [l.clone(), c.clone(), r.clone()]);
        assert_eq!(go(BarMove::Off(at("weather"))), None);
        // Dropped on the bar: after an item, at a part's start, or where it already was
        assert_eq!(go(BarMove::Place(at("status"), 0, Some(at("launcher")))).unwrap(), [list(&["launcher", "status", "pages"]), c.clone(), list(&["tray"])]);
        assert_eq!(go(BarMove::Place(at("media"), 1, None)).unwrap(), [l.clone(), list(&["media", "clock"]), r.clone()]);
        assert_eq!(go(BarMove::Place(at("clock"), 1, None)), None);
        assert_eq!(go(BarMove::Place(at("pages"), 0, Some(at("launcher")))), None);
    }

    #[test]
    fn the_arrangement_lists_what_the_bar_leaves_out() {
        let mut c = Config::default();
        c.layout.right = vec!["status".into()];
        let row = arrangement(&c, Some("tray"));
        let Control::Arrange { parts, hidden, picked, usual } = row.control else { panic!("not an arrangement") };
        assert_eq!(parts[2], ["status"]);
        assert_eq!(hidden, ["tools", "tray"]);
        assert_eq!(picked.as_deref(), Some("tray"));
        assert!(!usual);
        assert_eq!(row.about, "Click a part to put it there");
        // A pick that isn't anywhere is let go
        let Control::Arrange { picked, usual, .. } = arrangement(&Config::default(), Some("nothing")).control else { panic!() };
        assert_eq!((picked, usual), (None, true));
    }

    #[test]
    fn search_finds_settings_and_says_where_they_are() {
        let model = Model::default();
        let found = search("corner", &Config::default(), &model, &Pick::default());
        assert!(!found.is_empty());
        // Groups' titles aren't results; each result says its page and group
        assert!(found.iter().all(|r| !matches!(r.control, Control::Section)));
        let day = found.iter().find(|r| r.label == "Day panel").expect("the day panel's corner");
        assert!(day.about.starts_with("Bar › Corners"), "{}", day.about);
    }

    #[test]
    fn writes_without_losing_comments() {
        let dir = std::env::temp_dir().join(format!("asterism-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# my bar\nstyle = \"line\"  # slim\n\n[lock]\nidle = 10 # minutes\n").unwrap();
        write(&path, &["lock".into(), "idle".into()], &Value::Int(15)).unwrap();
        write(&path, &["style".into()], &Value::Str("frame".into())).unwrap();
        write(&path, &["notifications".into(), "apps".into(), "Some App".into()], &Value::Str("silence".into())).unwrap();
        write(&path, &["widgets".into(), "show".into()], &Value::List(vec!["clock".into()])).unwrap();
        // The bar's parts together, into a table the file didn't have
        let parts = |list: &[&str]| Value::List(list.iter().map(|s| s.to_string()).collect());
        write_all(&path, &[(vec!["layout".into(), "left".into()], parts(&["pages"])), (vec!["layout".into(), "right".into()], parts(&["clock", "status"]))]).unwrap();
        let c: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((c.layout.left, c.layout.right), (vec!["pages".to_string()], vec!["clock".to_string(), "status".to_string()]));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my bar") && text.contains("# slim") && text.contains("# minutes"), "{text}");
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!(config.lock.idle, 15);
        assert_eq!(config.notifications.apps.get("Some App"), Some(&crate::config::AppRule::Silence));
        assert_eq!(config.widgets.show, ["clock"]);
        write(&path, &["notifications".into(), "apps".into(), "Some App".into()], &Value::Remove).unwrap();
        let config: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(config.notifications.apps.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_into_the_example_config() {
        let dir = std::env::temp_dir().join(format!("asterism-settings-example-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let example = include_str!("../config.example.toml");
        std::fs::write(&path, example).unwrap();
        for (keys, value) in [
            (vec!["lock", "idle"], Value::Int(20)),
            (vec!["night_light", "mode"], Value::Str("schedule".into())),
            (vec!["notifications", "apps", "Spotify"], Value::Str("silence".into())),
            (vec!["clock", "seconds"], Value::Bool(true)),
            (vec!["weather", "city"], Value::Str("Pokhara".into())),
        ] {
            write(&path, &keys.iter().map(|k| k.to_string()).collect::<Vec<_>>(), &value).unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!((config.lock.idle, config.night_light.mode.as_str(), config.clock.seconds, config.weather.city.as_str()), (20, "schedule", true, "Pokhara"));
        assert_eq!(config.notifications.apps.get("Spotify"), Some(&crate::config::AppRule::Silence));
        // Every comment line is still there
        assert_eq!(text.lines().filter(|l| l.trim_start().starts_with('#')).count(), example.lines().filter(|l| l.trim_start().starts_with('#')).count());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
