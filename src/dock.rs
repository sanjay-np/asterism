//! The dock: a row of apps on a card along one edge of the screen (the foot by default), hidden
//! until the pointer touches the edge by it. Pinned apps keep their places; open apps join them,
//! a dot by each for its windows (the focused app's dots are a pill in the accent). An apps button may start it
//! and the trash end it. Everything here is pure: which apps, in what order, where they fall and
//! how they look; `app/dock.rs` gives it a surface, the pointer and the compositor.

use crate::config::DockSettings;
use crate::model::Image;
use crate::launcher::Entry;
use crate::model::Workspace;
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Animator, Ease, Piece, Prim};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// One of an app's windows.
#[derive(Debug, Clone, PartialEq)]
pub struct Win {
    pub id: u64,
    pub title: String,
    pub focused: bool,
}

/// An app on the dock: pinned, open, or both.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// The pinned name, else the desktop file id its windows were matched to, else their app id.
    pub key: String,
    pub name: String,
    pub pinned: bool,
    pub windows: Vec<Win>,
}

impl Item {
    pub fn focused(&self) -> bool {
        self.windows.iter().any(|w| w.focused)
    }
}

/// The key the trash goes by in menus and actions (no desktop file starts with "@").
pub const TRASH: &str = "@trash";

/// Whether a desktop file id and a window's app id name the same app. Apps that call their
/// windows "org.gnome.Nautilus" may be pinned as "nautilus", and the other way round.
pub fn same_app(id: &str, app_id: &str) -> bool {
    let a = id.trim_end_matches(".desktop").to_lowercase();
    let b = app_id.to_lowercase();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let last = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    a == b || last(&a) == b || a == last(&b)
}

/// The desktop entry for an app id: the exact id first, then a looser match.
pub fn entry<'a>(apps: &'a [Entry], app_id: &str) -> Option<&'a Entry> {
    apps.iter().find(|e| e.id.eq_ignore_ascii_case(app_id)).or_else(|| apps.iter().find(|e| same_app(&e.id, app_id)))
}

/// A readable name when no desktop file gives one: "org.gnome.Nautilus" → "Nautilus".
fn pretty(id: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id);
    let mut chars = last.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// What's on the dock: the pinned apps in their order, then the other open apps in the order
/// their first windows come.
pub fn items(pinned: &[String], workspaces: &[Workspace], apps: &[Entry]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for key in pinned {
        let key = key.trim();
        if key.is_empty() || items.iter().any(|i| i.key == key) {
            continue;
        }
        let name = entry(apps, key).map_or_else(|| pretty(key), |e| e.name.clone());
        items.push(Item { key: key.to_string(), name, pinned: true, windows: Vec::new() });
    }
    for window in workspaces.iter().flat_map(|w| &w.windows) {
        if window.app_id.is_empty() {
            continue;
        }
        let win = Win { id: window.id, title: window.title.clone(), focused: window.focused };
        if let Some(item) = items.iter_mut().find(|i| same_app(&i.key, &window.app_id)) {
            item.windows.push(win);
            continue;
        }
        let found = entry(apps, &window.app_id);
        let key = found.map_or_else(|| window.app_id.clone(), |e| e.id.clone());
        if let Some(item) = items.iter_mut().find(|i| i.key == key) {
            item.windows.push(win);
            continue;
        }
        let name = found.map_or_else(|| pretty(&window.app_id), |e| e.name.clone());
        items.push(Item { key, name, pinned: false, windows: vec![win] });
    }
    items
}

/// What a click on an app does.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    Launch,
    Focus(u64),
}

/// Not open: start it. Open: go to it, and when you're already there, on to its next window.
pub fn click(item: &Item) -> Click {
    if item.windows.is_empty() {
        return Click::Launch;
    }
    match item.windows.iter().position(|w| w.focused) {
        Some(i) => Click::Focus(item.windows[(i + 1) % item.windows.len()].id),
        None => Click::Focus(item.windows[0].id),
    }
}

/// The pinned list once `key` is dropped at `index` among the others (None: taken off).
pub fn repin(pinned: &[String], key: &str, index: Option<usize>) -> Vec<String> {
    let mut list: Vec<String> = pinned.iter().filter(|k| k.as_str() != key).cloned().collect();
    if let Some(i) = index {
        list.insert(i.min(list.len()), key.to_string());
    }
    list
}

// ---------------------------------------------------------------- layout

/// The edge of the screen the dock sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Bottom,
    Top,
    Left,
    Right,
}

impl Edge {
    pub fn of(name: &str) -> Edge {
        match name.trim().to_lowercase().as_str() {
            "top" => Edge::Top,
            "left" => Edge::Left,
            "right" => Edge::Right,
            _ => Edge::Bottom,
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

/// Everything is laid out as if the dock sat on the bottom edge: `a` along it, `c` across it
/// from its inner side (so the screen's edge is at `c = thick`). A frame turns that into x and y
/// on the edge it really sits on, for a whole surface or for one piece (`thick` is the size
/// across of what's being mapped). Icons then grow away from the edge on every side, and their
/// names sit on the inner side; text is never turned, only placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub edge: Edge,
    pub thick: f32,
}

impl Frame {
    /// A rectangle (a, c, length along, length across) as (x, y, w, h).
    pub fn rect(&self, a: f32, c: f32, la: f32, lc: f32) -> (f32, f32, f32, f32) {
        match self.edge {
            Edge::Bottom => (a, c, la, lc),
            Edge::Top => (a, self.thick - c - lc, la, lc),
            Edge::Left => (self.thick - c - lc, a, lc, la),
            Edge::Right => (c, a, lc, la),
        }
    }

    /// A point on the surface back into (a, c).
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        match self.edge {
            Edge::Bottom => (x, y),
            Edge::Top => (x, self.thick - y),
            Edge::Left => (y, self.thick - x),
            Edge::Right => (y, x),
        }
    }
}

/// The widest a name gets beside a dock on the left or right (wider ones are cut short).
const TIP_MAX: f32 = 200.0;

/// The dock's measures, from its settings, in the bottom edge's terms (see `Frame`).
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub edge: Edge,
    pub size: f32,
    /// One app's place along the card.
    pub cell: f32,
    pub pad: f32,
    pub card_h: f32,
    /// Room on the card's inner side for icons growing and their names (beside the dock when it
    /// stands on the left or right, so wider there).
    pub room: f32,
    /// How far the card sits off the screen's edge.
    pub foot: f32,
    /// Room at either end of the card for a name that's wider than its icon.
    pub side: f32,
}

/// How much an icon grows under the pointer, and its neighbours.
const GROW: f32 = 1.32;
const GROW_NEAR: f32 = 1.12;
const SEP: f32 = 13.0;

impl Geometry {
    /// From the settings: the icon size, and the look (a pill's round ends need room at the
    /// sides; attached, it sits on the edge).
    pub fn of(settings: &DockSettings) -> Self {
        let size = (settings.size as f32).clamp(24.0, 96.0);
        // Icon, 6 above it, a 3-pixel dot 4 below it, 3 under that
        let card_h = size + 16.0;
        let pad = if settings.corners == "pill" { (5.0 + card_h * 0.2).round() } else { 5.0 };
        let foot = if settings.attached { 0.0 } else { 8.0 };
        let edge = Edge::of(&settings.position);
        let lift = (size * (GROW - 1.0)).ceil();
        // A name goes over or under an icon on a level dock, beside it on an upright one
        let (room, side) = if edge.vertical() { (lift + 10.0 + TIP_MAX, 16.0) } else { (lift + 38.0, 72.0) };
        Geometry { edge, size, cell: size + 10.0, pad, card_h, room, foot, side }
    }

    /// The surface as a whole, for mapping (`Frame`).
    pub fn frame(&self) -> Frame {
        Frame { edge: self.edge, thick: self.thick() }
    }

    /// A piece of this dock `thick` across, for mapping what's inside it.
    fn piece_frame(&self, thick: f32) -> Frame {
        Frame { edge: self.edge, thick }
    }

    /// The length along the dock of a surface this size.
    pub fn along(&self, w: f32, h: f32) -> f32 {
        if self.edge.vertical() { h } else { w }
    }

    /// How round the card's corners are, from the settings and the theme's own radius.
    pub fn radius(&self, settings: &DockSettings, theme_radius: f32) -> f32 {
        match settings.corners.as_str() {
            "square" => 0.0,
            "small" => 6.0,
            "pill" => self.card_h / 2.0,
            _ => theme_radius,
        }
    }

    /// How far a grown icon reaches above where it stands at rest.
    pub fn lift(&self) -> f32 {
        (self.size * (GROW - 1.0)).ceil()
    }

    /// The surface's size across the dock (whole pixels, as the surface is).
    pub fn thick(&self) -> f32 {
        (self.room + self.card_h + self.foot).ceil()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot {
    Launcher,
    App(usize),
    /// Where an app being carried would land.
    Hole,
    Sep,
    Trash,
}

impl Slot {
    fn width(self, g: &Geometry) -> f32 {
        if self == Slot::Sep { SEP } else { g.cell }
    }
}

/// An app being moved along the dock: which, where it would land among the pinned (None: off
/// the dock), and where the pointer is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Carry {
    pub item: usize,
    pub hole: Option<usize>,
    pub x: f32,
    pub y: f32,
}

/// The order along the dock: apps button | pinned | other open apps | trash, a thin line
/// between groups. The carried app leaves its place, and a hole opens where it would land; an
/// open app that wouldn't land among the pinned keeps its own place open (the row stays still).
pub fn slots(items: &[Item], settings: &DockSettings, carry: Option<(usize, Option<usize>)>) -> Vec<Slot> {
    let carried = carry.map(|c| c.0);
    let mut pinned: Vec<Slot> = items.iter().enumerate().filter(|(i, it)| it.pinned && Some(*i) != carried).map(|(i, _)| Slot::App(i)).collect();
    if let Some((_, Some(hole))) = carry {
        pinned.insert(hole.min(pinned.len()), Slot::Hole);
    }
    let open: Vec<Slot> = items
        .iter()
        .enumerate()
        .filter(|(i, it)| !it.pinned && (Some(*i) != carried || matches!(carry, Some((_, None)))))
        .map(|(i, _)| if Some(i) == carried { Slot::Hole } else { Slot::App(i) })
        .collect();
    let mut out = Vec::new();
    for group in [pinned, open] {
        if group.is_empty() {
            continue;
        }
        if settings.separators && (!out.is_empty() || settings.launcher) {
            out.push(Slot::Sep);
        }
        out.extend(group);
    }
    if settings.launcher {
        out.insert(0, Slot::Launcher);
    }
    if settings.trash {
        if settings.separators && !out.is_empty() {
            out.push(Slot::Sep);
        }
        out.push(Slot::Trash);
    }
    out
}

/// Where the card and each slot fall on a surface `w` wide: (card x, card width, slots' x).
pub fn layout(slots: &[Slot], g: &Geometry, w: f32) -> (f32, f32, Vec<(Slot, f32)>) {
    let card_w = g.pad * 2.0 + slots.iter().map(|s| s.width(g)).sum::<f32>();
    let x0 = ((w - card_w) / 2.0).round();
    let mut x = x0 + g.pad;
    let mut placed = Vec::with_capacity(slots.len());
    for &slot in slots {
        placed.push((slot, x));
        x += slot.width(g);
    }
    (x0, card_w, placed)
}

/// The surface the dock wants: long enough for every slot (none carried) and a name at either
/// end, as wide and high as its edge has it.
pub fn surface_size(items: &[Item], settings: &DockSettings, g: &Geometry) -> (u32, u32) {
    let (_, card_w, _) = layout(&slots(items, settings, None), g, 0.0);
    let (along, across) = ((card_w + g.side * 2.0).ceil() as u32, g.thick() as u32);
    if g.edge.vertical() { (across, along) } else { (along, across) }
}

/// Where an app carried to (x, y) (along and across, see `Frame`) would land among the pinned: None when it's
/// lifted well off the dock (a pinned app is then taken off), or when an app that isn't pinned
/// is still over the open apps (it stays as it is). Measured against the dock as it stands with
/// nothing carried, so the places don't move under the pointer as the hole opens and closes.
pub fn hole(items: &[Item], settings: &DockSettings, g: &Geometry, w: f32, item: usize, x: f32, y: f32) -> Option<usize> {
    if y < g.room - 40.0 {
        return None;
    }
    let (_, _, placed) = layout(&slots(items, settings, None), g, w);
    let pinned: Vec<f32> = placed.iter().filter_map(|(s, sx)| matches!(s, Slot::App(i) if items[*i].pinned && *i != item).then_some(*sx)).collect();
    if !items[item].pinned {
        // Past the end of the pinned group (or, with none pinned yet, the start of the open apps)
        let start = placed.iter().find(|(s, _)| matches!(s, Slot::App(_))).map(|(_, sx)| *sx);
        let edge = pinned.last().map(|sx| sx + g.cell * 1.5).or(start).unwrap_or(x);
        if x > edge {
            return None;
        }
    }
    Some(pinned.iter().filter(|sx| *sx + g.cell / 2.0 < x).count())
}

// ---------------------------------------------------------------- drawing

pub struct View<'a> {
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub anim: &'a mut Animator,
    pub items: &'a [Item],
    pub settings: &'a DockSettings,
    pub icons: &'a HashMap<String, Option<Arc<Image>>>,
    /// The piece under the pointer, and the one held down.
    pub hovered: Option<&'a str>,
    pub pressed: Option<&'a str>,
    pub shown: bool,
    pub carry: Option<Carry>,
    pub trash: usize,
    /// The surface's length along the dock.
    pub w: f32,
    /// Icons not loaded yet, by app key: asked for by the caller.
    pub wanted: Vec<String>,
}

pub fn piece_key(slot: Slot, items: &[Item]) -> Option<String> {
    match slot {
        Slot::Launcher => Some("dock.launcher".into()),
        Slot::Trash => Some("dock.trash".into()),
        Slot::App(i) => Some(format!("dock.app.{}", items[i].key)),
        _ => None,
    }
}

/// The dock's pieces, sliding out from its edge while it's shown and back into it out of sight;
/// none once it's gone. Flat like the rest of the shell: a card with a hairline, the apps' own
/// pictures, the dock's own things as thin glyphs, a soft state layer under the pointer. Laid
/// out along and across (`Frame`), then put on the edge it sits on.
pub fn build(v: &mut View<'_>) -> Vec<Piece> {
    let t = v.theme;
    let g = Geometry::of(v.settings);
    let frame = g.frame();
    let shown = v.anim.value("dock.shown", if v.shown { 1.0 } else { 0.0 }, t.normal, Ease::OutCubic);
    if !v.shown && shown < 0.02 {
        return Vec::new();
    }
    // Out of sight is toward the edge
    let dy = ((1.0 - shown) * (g.card_h + g.foot + 16.0)).round();
    let slots = slots(v.items, v.settings, v.carry.map(|c| (c.item, c.hole)));
    let (x0, card_w, placed) = layout(&slots, &g, v.w);
    let (sw, sh) = if g.edge.vertical() { (g.thick(), v.w) } else { (v.w, g.thick()) };
    let place = |piece: &mut Piece, a: f32, c: f32, la: f32, lc: f32| {
        let (x, y, w, h) = frame.rect(a, c, la, lc);
        (piece.x, piece.y, piece.w, piece.h) = (x, y, w, h);
    };
    let mut pieces = Vec::new();

    // Attached, the card runs on past the screen's edge, so its outer corners go out of sight
    let radius = g.radius(v.settings, t.radius);
    let card_h = if v.settings.attached { g.card_h + radius + 2.0 } else { g.card_h };
    let mut card = Piece::new("dock.card", 0.0, 0.0);
    place(&mut card, x0, g.room + dy, card_w, card_h);
    let (w, h) = (card.w, card.h);
    match v.settings.background.as_str() {
        "none" => {}
        "flat" => {
            card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius, color: t.hairline() });
            card.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0, radius: (radius - 1.0).max(0.0), color: t.card_deep });
        }
        "glass" => {
            card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius, color: t.hairline().alpha(0.5) });
            card.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0, radius: (radius - 1.0).max(0.0), color: t.card_deep.alpha(0.62) });
        }
        _ => card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius }),
    }
    card.pad = 14.0 + radius;
    card.blocks = true;
    card.opacity = shown;
    pieces.push(card);

    // Grown under the pointer, a little beside it (never while carrying: the row is moving)
    let hovered_at = v.hovered.and_then(|h| placed.iter().position(|(s, _)| piece_key(*s, v.items).as_deref() == Some(h)));
    let lift = g.lift();
    let mut tip = None;
    for (n, &(slot, sx)) in placed.iter().enumerate() {
        let Some(key) = piece_key(slot, v.items) else {
            if slot == Slot::Sep {
                // A short hairline, level with the icons' middles
                let mut sep = Piece::new(format!("dock.sep.{n}"), 0.0, 0.0);
                place(&mut sep, sx, g.room + dy, SEP, g.card_h);
                let h = (g.size * 0.5).round();
                let (x, y, w, h) = g.piece_frame(g.card_h).rect((SEP / 2.0).floor(), (ICON_TOP + (g.size - h) / 2.0).round(), 1.0, h);
                sep.prims.push(Prim::Rect { x, y, w, h, radius: 0.0, color: t.outline_variant });
                sep.opacity = shown;
                pieces.push(sep);
            }
            continue;
        };
        // (A separator is a slot of its own: across one, nothing grows)
        let target = match hovered_at {
            Some(h) if v.settings.magnify && v.carry.is_none() && h == n => GROW,
            Some(h) if v.settings.magnify && v.carry.is_none() && h.abs_diff(n) == 1 => GROW_NEAR,
            _ => 1.0,
        };
        let held = v.pressed == Some(key.as_str());
        let grow = v.anim.value(&format!("{key}.grow"), if held { target * 0.92 } else { target }, t.fast, Ease::OutCubic);
        let hover = v.anim.value(&format!("{key}.hover"), if held { 1.6 } else if hovered_at == Some(n) && v.carry.is_none() { 1.0 } else { 0.0 }, t.fast, Ease::OutCubic);
        let mut piece = slot_piece(v, &g, slot, &key, grow, hover);
        place(&mut piece, sx, g.room - lift + dy, g.cell, g.card_h + lift);
        piece.opacity = shown;
        if hovered_at == Some(n) && v.carry.is_none() {
            // The icon as it stands now, on the surface
            let s = (g.size * grow).round();
            let icon = frame.rect(sx + ((g.cell - s) / 2.0).round(), g.room + ICON_TOP + g.size - s + dy, s, s);
            tip = Some((icon, slot_name(v, slot)));
        }
        pieces.push(piece);
    }
    // The app being carried follows the pointer, faint once it would come off
    if let Some(carry) = v.carry
        && let Some(key) = piece_key(Slot::App(carry.item), v.items)
    {
        let mut piece = slot_piece(v, &g, Slot::App(carry.item), &key, 1.0, 0.0);
        piece.key = format!("{key}.carried");
        place(&mut piece, (carry.x - g.cell / 2.0).round(), (g.room - lift + (carry.y - (g.room + g.card_h / 2.0)).min(0.0)).round(), g.cell, g.card_h + lift);
        piece.opacity = if carry.hole.is_none() && v.items[carry.item].pinned { 0.45 } else { 1.0 };
        pieces.push(piece);
    }
    // Its name, the bar's tooltip pill, on the icon's inner side
    if let Some(((ix, iy, s, _), name)) = tip {
        let font = Font::new(t.text - 1.0);
        let name = if g.edge.vertical() { crate::bar::Ctx::ellipsize_with(v.text, &name, font, TIP_MAX - 20.0) } else { name };
        let (tw, th) = v.text.measure(&name, font);
        let (w, h) = ((tw + 20.0).ceil(), (th + 6.0).ceil());
        let (x, y) = match g.edge {
            Edge::Bottom => (ix + s / 2.0 - w / 2.0, iy - h - 6.0),
            Edge::Top => (ix + s / 2.0 - w / 2.0, iy + s + 6.0),
            Edge::Left => (ix + s + 8.0, iy + s / 2.0 - h / 2.0),
            Edge::Right => (ix - w - 8.0, iy + s / 2.0 - h / 2.0),
        };
        let mut label = Piece::new("dock.tip", w, h);
        label.x = x.round().clamp(2.0, (sw - w - 2.0).max(2.0));
        label.y = y.round().clamp(2.0, (sh - h - 2.0).max(2.0));
        label.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.container_highest });
        label.prims.push(Prim::Text { x: 10.0, y: 3.0, text: name, font, color: t.color_text });
        label.opacity = shown * v.anim.value_from("dock.tip", 0.0, 1.0, t.fast, Ease::OutCubic);
        pieces.push(label);
    }
    pieces
}

fn slot_name(v: &View<'_>, slot: Slot) -> String {
    match slot {
        Slot::Launcher => "Apps".into(),
        Slot::Trash => match v.trash {
            0 => "Trash".into(),
            1 => "Trash · 1 item".into(),
            n => format!("Trash · {n} items"),
        },
        Slot::App(i) => {
            let item = &v.items[i];
            match item.windows.len() {
                0 | 1 => item.name.clone(),
                n => format!("{} · {n} windows", item.name),
            }
        }
        _ => String::new(),
    }
}

/// Where an icon's top sits below the card's top, before it grows.
const ICON_TOP: f32 = 6.0;

/// One place on the dock: its icon standing on a floor (growing away from the edge), a soft
/// state layer behind it under the pointer (stronger while held), and its windows' marks
/// between it and the edge. Laid out along and across, then turned to the dock's edge.
fn slot_piece(v: &mut View<'_>, g: &Geometry, slot: Slot, key: &str, grow: f32, hover: f32) -> Piece {
    let t = v.theme;
    let lift = g.lift();
    let local = g.piece_frame(g.card_h + lift);
    let (pw, ph) = if g.edge.vertical() { (g.card_h + lift, g.cell) } else { (g.cell, g.card_h + lift) };
    let mut piece = Piece::new(key, pw, ph);
    let rect = |a: f32, c: f32, la: f32, lc: f32, radius: f32, color: crate::theme::Rgba| {
        let (x, y, w, h) = local.rect(a, c, la, lc);
        Prim::Rect { x, y, w, h, radius, color }
    };
    let s = (g.size * grow).round();
    let floor = lift + ICON_TOP + g.size;
    let (ia, ic) = (((g.cell - s) / 2.0).round(), floor - s);
    // The icon's square, wherever the edge puts it (a square stays a square)
    let (ix, iy, _, _) = local.rect(ia, ic, s, s);
    // The state layer: a rounded square a little larger than the icon, growing with it
    if hover > 0.01 {
        let r = s + 6.0;
        piece.prims.push(rect(ia - 3.0, ic - 3.0, r, r, (r * 0.3).round(), t.color_text.alpha(0.07 * hover)));
    }
    // The dock's own things are glyphs in the icons' tint, the size an app's picture reads at
    let glyph = |piece: &mut Piece, name: &str| {
        let i = (s * 0.56).round();
        piece.prims.push(Prim::Icon { x: ix + ((s - i) / 2.0).round(), y: iy + ((s - i) / 2.0).round(), size: i, name: name.into(), color: t.icon_tint(crate::theme::Icons::Tools) });
    };
    match slot {
        Slot::Launcher => {
            glyph(&mut piece, "apps");
            piece.actions.left = Some(Action::ToggleLauncher);
        }
        Slot::Trash => {
            glyph(&mut piece, if v.trash > 0 { "trash-full" } else { "trash" });
            piece.actions.left = Some(Action::DockTrash);
            piece.actions.right = Some(Action::DockMenu(TRASH.into()));
        }
        Slot::App(i) => {
            let item = &v.items[i];
            match v.icons.get(&item.key) {
                Some(Some(image)) => piece.prims.push(Prim::Image { x: ix, y: iy, w: s, h: s, image: image.clone(), tint: None, radius: 0.0 }),
                found => {
                    if found.is_none() {
                        v.wanted.push(item.key.clone());
                    }
                    // No picture: its initial on a quiet tile, edged with a hairline
                    let b = (s * 0.84).round();
                    let (bx, by) = (ix + ((s - b) / 2.0).round(), iy + ((s - b) / 2.0).round());
                    let radius = (b * 0.3).round();
                    piece.prims.push(Prim::Rect { x: bx, y: by, w: b, h: b, radius, color: t.outline_variant });
                    piece.prims.push(Prim::Rect { x: bx + 1.0, y: by + 1.0, w: b - 2.0, h: b - 2.0, radius: radius - 1.0, color: t.container_high });
                    let letter: String = item.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                    let font = Font::new((b * 0.44).round()).weight(500);
                    let (lw, lh) = v.text.measure(&letter, font);
                    piece.prims.push(Prim::Text { x: bx + ((b - lw) / 2.0).round(), y: by + ((b - lh) / 2.0).round(), text: letter, font, color: t.color_soft });
                }
            }
            // A dot a window (three at most); the focused app's are one short pill in the accent.
            // Or one line by any open app, longer and in the accent for the focused one. Both
            // run along the dock, between the icon and the edge
            let n = item.windows.len().min(3) as f32;
            let dot_c = floor + 4.0;
            let style = if n == 0.0 { "none" } else { v.settings.indicator.as_str() };
            match style {
                "none" => {}
                "line" => {
                    let (w, color) = if item.focused() { ((g.size * 0.5).round(), t.primary) } else { ((g.size * 0.3).round(), t.color_soft) };
                    piece.prims.push(rect(((g.cell - w) / 2.0).round(), dot_c + 0.5, w, 2.0, 1.0, color));
                }
                _ if item.focused() => {
                    let w = 10.0 + 5.0 * (n - 1.0);
                    piece.prims.push(rect(((g.cell - w) / 2.0).round(), dot_c, w, 3.0, 1.5, t.primary));
                }
                _ => {
                    let w = 3.0 * n + 3.0 * (n - 1.0);
                    for d in 0..n as usize {
                        piece.prims.push(rect(((g.cell - w) / 2.0).round() + d as f32 * 6.0, dot_c, 3.0, 3.0, 1.5, t.color_soft));
                    }
                }
            }
            piece.actions.left = Some(Action::DockClick(item.key.clone()));
            piece.actions.middle = Some(Action::DockLaunch(item.key.clone()));
            piece.actions.right = Some(Action::DockMenu(item.key.clone()));
        }
        _ => {}
    }
    piece
}

/// For pictures of the dock: a few common apps pinned when the query asks for "sample" and
/// nothing is pinned yet.
pub fn sample_pins(pinned: &[String], query: &str) -> Vec<String> {
    if pinned.is_empty() && query.split(',').any(|w| w == "sample") {
        ["firefox", "org.gnome.Nautilus", "kitty", "code", "spotify"].map(String::from).to_vec()
    } else {
        pinned.to_vec()
    }
}

// ---------------------------------------------------------------- the trash

fn trash_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".local/share"));
    data.join("Trash")
}

/// What's in your trash (the home one; trashes on other drives aren't counted).
pub fn trash_count() -> usize {
    std::fs::read_dir(trash_dir().join("files")).map(|d| d.count()).unwrap_or(0)
}

/// Opens the trash in the file manager.
pub fn open_trash() {
    if crate::config::command_available("gio") {
        crate::launcher::spawn_app("gio open trash:///");
    } else {
        let dir = trash_dir().join("files");
        crate::launcher::spawn_app(&format!("xdg-open '{}'", dir.display().to_string().replace('\'', "'\\''")));
    }
}

/// Empties it for good: through gio when it's there (it also clears other drives' trashes),
/// else the home trash's files and their notes by hand.
pub fn empty_trash() {
    if crate::config::command_available("gio") && std::process::Command::new("gio").args(["trash", "--empty"]).status().is_ok_and(|s| s.success()) {
        return;
    }
    let dir = trash_dir();
    for sub in ["files", "info"] {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            // Never follow a link out of the trash: a link is removed, not what it points to
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            let _ = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Window, Workspace};

    fn entry(id: &str, name: &str) -> Entry {
        Entry { id: id.into(), name: name.into(), generic: String::new(), comment: String::new(), keywords: String::new(), exec: id.to_lowercase(), terminal: false }
    }

    fn window(id: u64, app: &str, focused: bool) -> Window {
        Window { id, app_id: app.into(), title: format!("{app} {id}"), focused, rect: None, floating: false }
    }

    fn workspace(windows: Vec<Window>) -> Workspace {
        Workspace { windows, ..Default::default() }
    }

    fn settings() -> DockSettings {
        DockSettings::default()
    }

    #[test]
    fn matches_apps_by_their_ids() {
        assert!(same_app("org.gnome.Nautilus", "org.gnome.Nautilus"));
        assert!(same_app("org.gnome.Nautilus", "nautilus"));
        assert!(same_app("firefox", "Firefox"));
        assert!(same_app("kitty.desktop", "kitty"));
        assert!(same_app("code", "com.visualstudio.code"));
        assert!(!same_app("kitty", "foot"));
        assert!(!same_app("", "kitty"));
    }

    #[test]
    fn pinned_first_then_open_apps_with_their_windows() {
        let apps = vec![entry("firefox", "Firefox"), entry("org.gnome.Nautilus", "Files"), entry("kitty", "kitty")];
        let spaces = vec![workspace(vec![window(1, "kitty", false), window(2, "firefox", true)]), workspace(vec![window(3, "kitty", false), window(4, "nautilus", false), window(5, "mystery.App", false)])];
        let items = items(&["firefox".into(), "spotify".into(), "firefox".into()], &spaces, &apps);
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["firefox", "spotify", "kitty", "org.gnome.Nautilus", "mystery.App"]);
        assert!(items[0].pinned && items[0].focused() && items[0].windows.len() == 1);
        assert!(items[1].pinned && items[1].windows.is_empty());
        assert_eq!(items[1].name, "Spotify");
        assert_eq!(items[2].windows.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(items[3].name, "Files");
        assert_eq!(items[4].name, "App");
    }

    #[test]
    fn a_click_starts_goes_to_then_cycles() {
        let mut item = Item { key: "kitty".into(), name: "kitty".into(), pinned: true, windows: Vec::new() };
        assert_eq!(click(&item), Click::Launch);
        item.windows = vec![Win { id: 7, title: String::new(), focused: false }, Win { id: 9, title: String::new(), focused: false }];
        assert_eq!(click(&item), Click::Focus(7));
        item.windows[0].focused = true;
        assert_eq!(click(&item), Click::Focus(9));
        item.windows[0].focused = false;
        item.windows[1].focused = true;
        assert_eq!(click(&item), Click::Focus(7));
    }

    #[test]
    fn repinning_moves_adds_and_removes() {
        let list: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(repin(&list, "a", Some(2)), ["b", "c", "a"]);
        assert_eq!(repin(&list, "c", Some(0)), ["c", "a", "b"]);
        assert_eq!(repin(&list, "d", Some(1)), ["a", "d", "b", "c"]);
        assert_eq!(repin(&list, "b", None), ["a", "c"]);
        assert_eq!(repin(&list, "d", Some(99)), ["a", "b", "c", "d"]);
    }

    #[test]
    fn slots_fall_in_groups() {
        let apps = vec![entry("firefox", "Firefox"), entry("kitty", "kitty")];
        let items = items(&["firefox".into()], &[workspace(vec![window(1, "kitty", false)])], &apps);
        assert_eq!(slots(&items, &settings(), None), [Slot::Launcher, Slot::Sep, Slot::App(0), Slot::Sep, Slot::App(1), Slot::Sep, Slot::Trash]);
        // Carrying the open app onto the pinned ones: its group goes, a hole opens; not there
        // yet, its own place stays open
        assert_eq!(slots(&items, &settings(), Some((1, Some(0)))), [Slot::Launcher, Slot::Sep, Slot::Hole, Slot::App(0), Slot::Sep, Slot::Trash]);
        assert_eq!(slots(&items, &settings(), Some((1, None))), [Slot::Launcher, Slot::Sep, Slot::App(0), Slot::Sep, Slot::Hole, Slot::Sep, Slot::Trash]);
        let bare = DockSettings { launcher: false, trash: false, ..settings() };
        assert_eq!(slots(&items, &bare, None), [Slot::App(0), Slot::Sep, Slot::App(1)]);
        assert!(slots(&[], &bare, None).is_empty());
    }

    #[test]
    fn frames_put_the_dock_on_its_edge() {
        // A 10×4 rectangle 2 along and 1 in from the inner side, on a dock 30 across
        let at = |edge| Frame { edge, thick: 30.0 }.rect(2.0, 1.0, 10.0, 4.0);
        assert_eq!(at(Edge::Bottom), (2.0, 1.0, 10.0, 4.0));
        assert_eq!(at(Edge::Top), (2.0, 25.0, 10.0, 4.0));
        assert_eq!(at(Edge::Left), (25.0, 2.0, 4.0, 10.0));
        assert_eq!(at(Edge::Right), (1.0, 2.0, 4.0, 10.0));
        // And a point comes back where it was, on every edge
        for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
            let frame = Frame { edge, thick: 30.0 };
            let (x, y, _, _) = frame.rect(7.0, 3.0, 0.0, 0.0);
            assert_eq!(frame.point(x, y), (7.0, 3.0), "{edge:?}");
        }
        assert_eq!(Edge::of("Left"), Edge::Left);
        assert_eq!(Edge::of("sideways"), Edge::Bottom);
    }

    #[test]
    fn an_upright_dock_is_tall() {
        let apps = vec![entry("firefox", "Firefox")];
        let items = items(&["firefox".into()], &[], &apps);
        let level = settings();
        let upright = DockSettings { position: "right".into(), ..settings() };
        let (gl, gu) = (Geometry::of(&level), Geometry::of(&upright));
        let (w, h) = surface_size(&items, &level, &gl);
        let (vw, vh) = surface_size(&items, &upright, &gu);
        // The same card along it: across the screen on the bottom, down it on the right
        let (_, card, _) = layout(&slots(&items, &level, None), &gl, 0.0);
        assert_eq!((w as f32, h as f32), ((card + gl.side * 2.0).ceil(), gl.thick()));
        assert_eq!((vw as f32, vh as f32), (gu.thick(), (card + gu.side * 2.0).ceil()));
        assert_eq!(gu.along(vw as f32, vh as f32), vh as f32);
        // with room beside it for names
        assert!(vw as f32 > TIP_MAX);
    }

    #[test]
    fn the_look_changes_the_measures() {
        let plain = settings();
        let pill = DockSettings { corners: "pill".into(), attached: true, separators: false, ..settings() };
        let (a, b) = (Geometry::of(&plain), Geometry::of(&pill));
        assert_eq!(a.foot, 8.0);
        assert_eq!(b.foot, 0.0);
        assert!(b.pad > a.pad, "a pill's round ends need room");
        assert_eq!(b.radius(&pill, 16.0), b.card_h / 2.0);
        assert_eq!(a.radius(&DockSettings { corners: "square".into(), ..settings() }, 16.0), 0.0);
        assert_eq!(a.radius(&plain, 16.0), 16.0);
        let apps = vec![entry("firefox", "Firefox"), entry("kitty", "kitty")];
        let items = items(&["firefox".into()], &[workspace(vec![window(1, "kitty", false)])], &apps);
        assert_eq!(slots(&items, &pill, None), [Slot::Launcher, Slot::App(0), Slot::App(1), Slot::Trash]);
    }

    #[test]
    fn carrying_finds_its_place() {
        let apps = vec![entry("a", "A"), entry("b", "B"), entry("c", "C"), entry("k", "K")];
        let items = items(&["a".into(), "b".into(), "c".into()], &[workspace(vec![window(1, "k", false)])], &apps);
        let s = settings();
        let g = Geometry::of(&s);
        let (w, _) = surface_size(&items, &s, &g);
        let w = w as f32;
        let (_, _, placed) = layout(&slots(&items, &s, None), &g, w);
        let x_of = |i: usize| placed.iter().find(|(s, _)| *s == Slot::App(i)).map(|(_, x)| *x + g.cell / 2.0).unwrap();
        let y = g.room + g.card_h / 2.0;
        // "a" dragged past "c": last; to the far left: first
        assert_eq!(hole(&items, &s, &g, w, 0, x_of(2) + 10.0, y), Some(2));
        assert_eq!(hole(&items, &s, &g, w, 2, 0.0, y), Some(0));
        // Lifted off: no place
        assert_eq!(hole(&items, &s, &g, w, 0, x_of(1), 0.0), None);
        // The open app over its own place stays unpinned; over the pinned ones it would pin
        assert_eq!(hole(&items, &s, &g, w, 3, x_of(3), y), None);
        assert_eq!(hole(&items, &s, &g, w, 3, x_of(1), y), Some(1));
    }

    #[test]
    fn builds_and_hides() {
        let theme = Theme::new(&crate::config::Config::default());
        let mut text = Text::new("sans");
        let mut anim = Animator::new();
        let apps = vec![entry("firefox", "Firefox")];
        let items = items(&["firefox".into()], &[workspace(vec![window(1, "firefox", true)])], &apps);
        let s = settings();
        let icons = HashMap::new();
        let build_with = |anim: &mut Animator, text: &mut Text, shown: bool, hovered: Option<&str>| {
            anim.begin();
            let mut v = View { theme: &theme, text, anim, items: &items, settings: &s, icons: &icons, hovered, pressed: None, shown, carry: None, trash: 3, w: 600.0, wanted: Vec::new() };
            let pieces = build(&mut v);
            let wanted = v.wanted;
            anim.end();
            (pieces, wanted)
        };
        let build_once = |anim: &mut Animator, text: &mut Text, shown: bool| build_with(anim, text, shown, Some("dock.app.firefox"));
        // Nothing under the pointer: nothing grows (a separator once counted as hovered)
        let (rest, _) = build_with(&mut Animator::new(), &mut text, true, None);
        let tile = |pieces: &[Piece]| pieces.iter().find(|p| p.key == "dock.app.firefox").and_then(|p| p.prims.iter().find_map(|q| if let Prim::Rect { w, .. } = q { Some(*w) } else { None }));
        assert_eq!(tile(&rest), Some((44.0f32 * 0.84).round()));
        assert!(!rest.iter().any(|p| p.key == "dock.tip"));
        let (pieces, wanted) = build_once(&mut anim, &mut text, true);
        assert!(pieces.iter().any(|p| p.key == "dock.app.firefox"));
        assert!(pieces.iter().any(|p| p.key == "dock.tip"));
        assert_eq!(wanted, ["firefox"]);
        // Hidden from the start: nothing at all
        let mut fresh = Animator::new();
        assert!(build_once(&mut fresh, &mut text, false).0.is_empty());
    }
}
