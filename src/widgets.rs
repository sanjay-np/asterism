//! Desktop widgets: a clock, the weather, what's playing (with its lyrics, line by line), a
//! visualizer of what the speakers play (in a card, or along the screen's foot), a month, the
//! machine, a photo frame and a sticky note, on the wallpaper under your windows. Where each one
//! stands (as shares of the screen) and how big it is (a box, in pixels) is kept in
//! ~/.local/state/asterism/widgets.json, so a drag survives restarts and a different screen size.
//! The editor (`editor`) lays its own pieces over them: a frame and a remove button on each, and
//! a strip at the top to add the ones not shown.

use crate::model::{Command, Model};
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Every kind there is, in the order they're offered.
pub const KINDS: [&str; 9] = ["clock", "weather", "media", "visualizer", "edge", "calendar", "system", "photos", "note"];

/// The scales an older places file may give (from when the wheel sized widgets); a box has none.
pub const SMALLEST: f32 = 0.6;
pub const BIGGEST: f32 = 2.0;

/// The least a resized widget can be either way, so its handles can still be caught.
pub const LEAST_SIDE: f32 = 32.0;

/// A kind's name, as the settings and the editor say it.
pub fn name(kind: &str) -> &'static str {
    match kind {
        "clock" => "Clock",
        "weather" => "Weather",
        "media" => "What's playing",
        "visualizer" => "Visualizer",
        "edge" => "Visualizer along the edge",
        "calendar" => "Calendar",
        "photos" => "Photo frame",
        "note" => "Sticky note",
        k if k.starts_with("pet") => "Your pet",
        _ => "The machine",
    }
}

fn icon(kind: &str) -> &'static str {
    match kind {
        "clock" => "timer",
        "weather" => "cloud-sun",
        "media" => "music",
        "visualizer" | "edge" => "speaker-high",
        "calendar" => "calendar",
        "photos" => "image",
        "note" => "lyrics",
        _ => "cpu",
    }
}

fn places_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".local/state"));
    state.join("asterism").join("widgets.json")
}

/// Where a widget stands, as a share of the screen's width and height.
pub fn default_place(kind: &str) -> (f32, f32) {
    match kind {
        "clock" => (0.05, 0.10),
        "weather" => (0.05, 0.36),
        "media" => (0.05, 0.66),
        "visualizer" => (0.36, 0.80),
        "calendar" => (0.78, 0.10),
        "photos" => (0.56, 0.10),
        "note" => (0.24, 0.12),
        _ => (0.78, 0.56),
    }
}

/// Where a widget stands (shares of the screen) and how big it is: the box it was resized to, in
/// logical pixels, or (never resized, or from an older file) a plain scale of it as built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub size: Option<(f32, f32)>,
}

/// A widget's place: as saved, else where it starts out.
pub fn place_of(places: &HashMap<String, Place>, kind: &str) -> Place {
    places.get(kind).copied().unwrap_or_else(|| {
        let (x, y) = default_place(kind);
        Place { x, y, scale: 1.0, size: None }
    })
}

/// Places from their file: `[x, y, scale, w, h]` each (older files stop after the scale, or
/// before it).
pub fn read_places(text: &str) -> HashMap<String, Place> {
    let raw: HashMap<String, Vec<f32>> = serde_json::from_str(text).unwrap_or_default();
    raw.into_iter()
        .filter_map(|(kind, v)| {
            let size = match (v.get(3), v.get(4)) {
                (Some(w), Some(h)) => Some((w.max(1.0), h.max(1.0))),
                _ => None,
            };
            Some((kind, Place { x: *v.first()?, y: *v.get(1)?, scale: v.get(2).copied().unwrap_or(1.0).clamp(SMALLEST, BIGGEST), size }))
        })
        .collect()
}

/// Whether a kind's card takes whatever box it's given; the clock (bare words) only grows and
/// shrinks, and the bars along the screen's foot aren't sized at all.
pub fn fills(kind: &str) -> bool {
    !matches!(kind, "clock" | "edge")
}

/// How a widget came out last time it was laid out: its size as built (with no box asked) and
/// the scale it was drawn at. Resizing starts from these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub natural: (f32, f32),
    pub scale: f32,
}

/// A box of `size` for a widget that is `natural` as built: the largest scale at which it still
/// fits (no limit either way), and the room, before scaling, that it lays itself out in (never
/// less than it needs as built). So widening a card gives its words room, and only growing it
/// both ways makes them bigger.
pub fn fit_box(natural: (f32, f32), size: (f32, f32)) -> (f32, (f32, f32)) {
    let k = (size.0 / natural.0.max(1.0)).min(size.1 / natural.1.max(1.0)).max(0.01);
    (k, ((size.0 / k).max(natural.0), (size.1 / k).max(natural.1)))
}

/// A size as built, grown to the room asked for.
fn grow(built: (f32, f32), room: Option<(f32, f32)>) -> (f32, f32) {
    match room {
        Some((w, h)) => (built.0.max(w), built.1.max(h)),
        None => built,
    }
}

pub fn load_places() -> HashMap<String, Place> {
    std::fs::read_to_string(places_path()).map(|t| read_places(&t)).unwrap_or_default()
}

pub fn save_places(places: &HashMap<String, Place>) {
    let path = places_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Some(text) = write_places(places) {
        let _ = std::fs::write(path, text);
    }
}

/// Places as their file has them (`read_places` reads it back).
pub fn write_places(places: &HashMap<String, Place>) -> Option<String> {
    let raw: HashMap<&String, Vec<f32>> = places
        .iter()
        .map(|(kind, p)| {
            let mut v = vec![p.x, p.y, p.scale];
            if let Some((w, h)) = p.size {
                v.extend([w.round(), h.round()]);
            }
            (kind, v)
        })
        .collect();
    serde_json::to_string_pretty(&raw).ok()
}

/// Synced lyrics ("[01:23.45] a line") as times and lines, in order.
pub fn parse_lyrics(text: &str) -> Vec<(f64, String)> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        // A line may carry several times
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some((stamp, after)) = stripped.split_once(']') else { break };
            let Some((m, s)) = stamp.split_once(':') else { break };
            let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) else { break };
            times.push(m * 60.0 + s);
            rest = after;
        }
        for time in times {
            lines.push((time, rest.trim().to_string()));
        }
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    lines
}

/// The line being sung at `seconds`, and the one after it.
pub fn lyric_at(lines: &[(f64, String)], seconds: f64) -> (Option<&str>, Option<&str>) {
    let at = lines.iter().rposition(|(t, _)| *t <= seconds);
    let now = at.map(|i| lines[i].1.as_str()).filter(|l| !l.is_empty());
    let next = lines.get(at.map_or(0, |i| i + 1)).map(|(_, l)| l.as_str()).filter(|l| !l.is_empty());
    (now, next)
}

/// Which song lyrics are kept under.
pub fn lyrics_key(artist: &str, title: &str) -> String {
    format!("{artist}\u{1f}{title}")
}

pub struct View<'a> {
    pub model: &'a Model,
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub now: chrono::DateTime<chrono::Local>,
    pub lyrics: bool,
    pub requested: &'a HashSet<String>,
    pub requests: Vec<Command>,
    pub settings: &'a crate::config::WidgetSettings,
    /// Pictures skipped ahead of the clock, in the photo frame ("Next picture").
    pub photo_step: u64,
}

/// A translucent card, as solid as `[widgets] opacity` says (none at all at 0), with a hairline
/// unless it's turned off.
fn card(piece: &mut Piece, t: &Theme, w: f32, h: f32) {
    if t.widget_opacity > 0.0 {
        piece.prims.push(Prim::Rect { x: 0.5, y: 0.5, w: w - 1.0, h: h - 1.0, radius: 19.5, color: t.widget_card });
    }
    if t.widget_outline {
        piece.prims.push(outline(0.5, 0.5, w - 1.0, h - 1.0, 19.5, 1.0, t.hairline()));
    }
}

/// How round a widget's card is, for the blur behind it; None for those with no card (the
/// clock's words, the bars along the screen's foot).
pub fn card_radius(kind: &str) -> Option<f32> {
    match kind {
        "clock" | "edge" => None,
        "note" => Some(18.0),
        _ => Some(20.0),
    }
}

/// A rounded rectangle as the plain rectangles a Wayland region is made of: the middle whole,
/// the corners' rows each cut in to the curve (rows alike merged).
pub fn rounded_rows(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Vec<(i32, i32, i32, i32)> {
    let (x, y, w, h) = (x.round() as i32, y.round() as i32, w.round() as i32, h.round() as i32);
    let r = (radius.round() as i32).clamp(0, w.min(h) / 2);
    let mut rows: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut add = |rect: (i32, i32, i32, i32)| {
        // A row cut in as far as the one before it on the same side joins it
        if let Some(last) = rows.last_mut()
            && last.0 == rect.0
            && last.2 == rect.2
            && last.1 + last.3 == rect.1
        {
            last.3 += rect.3;
        } else if rect.2 > 0 && rect.3 > 0 {
            rows.push(rect);
        }
    };
    let inset = |i: i32| {
        let dy = r as f32 - i as f32 - 0.5;
        (r as f32 - (r as f32 * r as f32 - dy * dy).max(0.0).sqrt()).ceil() as i32
    };
    for i in 0..r {
        let cut = inset(i);
        add((x + cut, y + i, w - 2 * cut, 1));
    }
    add((x, y + r, w, h - 2 * r));
    for i in (0..r).rev() {
        let cut = inset(i);
        add((x + cut, y + h - 1 - i, w - 2 * cut, 1));
    }
    rows
}

/// Words on a faint card get the clock's soft shadow, so a busy wallpaper doesn't swallow them
/// (those already shadowed are left as they are).
fn shadow_words(piece: &mut Piece) {
    let shadow = crate::theme::Rgba::BLACK.alpha(0.35);
    let mut prims = Vec::with_capacity(piece.prims.len() * 2);
    for prim in std::mem::take(&mut piece.prims) {
        let had = matches!(prims.last(), Some(Prim::Text { color, .. }) if *color == shadow);
        match &prim {
            Prim::Text { x, y, text, font, .. } if !had => prims.push(Prim::Text { x: x + 1.5, y: y + 2.0, text: text.clone(), font: *font, color: shadow }),
            Prim::Marquee { x, y, width, text, font, offset, gap, .. } => {
                prims.push(Prim::Marquee { x: x + 1.5, y: y + 2.0, width: *width, text: text.clone(), font: *font, color: shadow, offset: *offset, gap: *gap })
            }
            _ => {}
        }
        prims.push(prim);
    }
    piece.prims = prims;
}

/// Text with a soft shadow, to read on any wallpaper.
fn shadowed(piece: &mut Piece, x: f32, y: f32, text: String, font: Font, color: crate::theme::Rgba) {
    piece.prims.push(Prim::Text { x: x + 1.5, y: y + 2.0, text: text.clone(), font, color: crate::theme::Rgba::BLACK.alpha(0.35) });
    piece.prims.push(Prim::Text { x, y, text, font, color });
}

/// The visualizer's bars across `width` from `x`, each reaching up and down from `mid` by as much
/// as its level of `reach`; a level of nothing leaves a dot.
pub fn bars(piece: &mut Piece, t: &Theme, levels: &[f32], x: f32, width: f32, mid: f32, reach: f32) {
    let n = crate::spectrum::BARS;
    let slot = width / n as f32;
    let thick = (slot * 0.5).clamp(3.0, 8.0);
    for i in 0..n {
        let level = levels.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let half = (reach * level).max(thick / 2.0);
        piece.prims.push(Prim::Rect {
            x: x + i as f32 * slot + (slot - thick) / 2.0,
            y: mid - half,
            w: thick,
            h: half * 2.0,
            radius: thick / 2.0,
            color: t.primary.alpha(0.5 + 0.5 * level),
        });
    }
}

/// A widget with nothing to show yet (nothing playing, the weather not known), as the editor
/// draws it: the size it will be, its name, and when it shows.
pub fn placeholder(kind: &str, t: &Theme, text: &mut Text, room: Option<(f32, f32)>) -> Piece {
    let built = match kind {
        "media" => (400.0, 112.0),
        "visualizer" => (420.0, 128.0),
        "weather" => (300.0, 116.0),
        "photos" => (336.0, 216.0),
        _ => (292.0, 112.0),
    };
    let (w, h) = grow(built, room);
    let mut piece = Piece::new(format!("widget.{kind}"), w, h);
    card(&mut piece, t, w, h);
    if kind == "visualizer" {
        bars(&mut piece, t, &crate::spectrum::sample_levels(crate::spectrum::BARS), 24.0, w - 48.0, h / 2.0 - 10.0, 26.0);
    } else {
        piece.prims.push(Prim::Icon { x: 22.0, y: 24.0, size: 28.0, name: icon(kind).into(), color: t.widget_strong });
    }
    let when = match kind {
        "media" | "visualizer" | "edge" => "Shows while something plays",
        "weather" => "Shows once the weather is known",
        _ => "",
    };
    let small = Font::new(t.text - 1.0);
    let (_, sh) = text.measure(when, small);
    if kind == "visualizer" {
        let (ww, _) = text.measure(when, small);
        piece.prims.push(Prim::Text { x: ((w - ww) / 2.0).round(), y: h - 18.0 - sh, text: when.into(), font: small, color: t.color_soft });
    } else {
        let title = Font::new(t.text + 2.0).weight(500);
        piece.prims.push(Prim::Text { x: 66.0, y: 24.0, text: name(kind).into(), font: title, color: t.widget_strong });
        piece.prims.push(Prim::Text { x: 66.0, y: 50.0, text: when.into(), font: small, color: t.color_soft });
    }
    piece
}

/// The widgets in `show`, placed on a desktop of `size` by their saved shares (kept 16 px in
/// from every edge) and sized to their boxes; how each came out goes into `fits`. In the
/// editor, ones with nothing to show stand in as placeholders.
pub fn arrange(show: &[String], v: &mut View<'_>, places: &HashMap<String, Place>, size: (f32, f32), editing: bool, fits: &mut HashMap<String, Fit>) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for kind in show {
        // Along the screen's foot, the whole width: it isn't placed, nor made bigger
        if kind == "edge" {
            pieces.extend(edge(v, size, editing));
            continue;
        }
        let made = |v: &mut View<'_>, room: Option<(f32, f32)>| match build(kind, v, room) {
            Some(piece) => Some(piece),
            None if editing && KINDS.contains(&kind.as_str()) => Some(placeholder(kind, v.theme, v.text, room)),
            None => None,
        };
        let Some(built) = made(v, None) else { continue };
        let place = place_of(places, kind);
        let natural = (built.w, built.h);
        // A box asked for: built again in the room it leaves (what that asks to be fetched was
        // asked the first time already)
        let (mut piece, scale) = match place.size {
            Some(want) => {
                let (scale, room) = fit_box(natural, want);
                let asked = v.requests.len();
                let piece = if fills(kind) { made(v, Some(room)).unwrap_or(built) } else { built };
                v.requests.truncate(asked);
                (piece, scale)
            }
            None => (built, place.scale),
        };
        fits.insert(kind.clone(), Fit { natural, scale });
        piece.scale(scale);
        // Kept 16 px in from the edges, less when it's grown too big for that (up to the edges)
        let inset = |room: f32, side: f32| 16.0f32.min(((room - side) / 2.0).max(0.0));
        let (ix, iy) = (inset(size.0, piece.w), inset(size.1, piece.h));
        piece.x = (place.x * size.0).clamp(ix, (size.0 - piece.w - ix).max(ix)).round();
        piece.y = (place.y * size.1).clamp(iy, (size.1 - piece.h - iy).max(iy)).round();
        // A widget catches the pointer: to be moved, and for its menu on a right click
        piece.blocks = true;
        pieces.push(piece);
    }
    pieces
}

/// The visualizer along the screen's foot: bars across the whole width, the low notes in the
/// middle and the high ones out to both sides, rising from the edge (or from the bar, when the
/// bar is down there). While arranging, with nothing playing, a still picture of it.
fn edge(v: &View<'_>, size: (f32, f32), editing: bool) -> Option<Piece> {
    let t = v.theme;
    let playing = v.model.media.player().is_some_and(|p| p.playing);
    if !playing && !editing {
        return None;
    }
    let sample;
    let levels: &[f32] = if playing {
        &v.model.spectrum
    } else {
        sample = crate::spectrum::sample_levels(crate::spectrum::BARS);
        &sample
    };
    let n = crate::spectrum::BARS;
    let h = 80.0;
    let mut piece = Piece::new("widget.edge", size.0, h);
    let slot = size.0 / (n * 2) as f32;
    let thick = (slot * 0.42).clamp(3.0, 14.0);
    let still = if playing { 1.0 } else { 0.6 };
    for i in 0..n * 2 {
        let band = if i < n { n - 1 - i } else { i - n };
        let level = levels.get(band).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let reach = ((h - 8.0) * level).max(thick);
        // Rounded at the top only: the foot runs past the piece's edge and is cut off there
        piece.prims.push(Prim::Rect { x: i as f32 * slot + (slot - thick) / 2.0, y: h - reach, w: thick, h: reach + thick, radius: thick / 2.0, color: t.primary.alpha((0.3 + 0.5 * level) * still) });
    }
    let under = if t.on_top { 0.0 } else { t.reserve() };
    piece.y = size.1 - h - under;
    Some(piece)
}

/// A rounded rectangle's outline, as a line.
fn outline(x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, color: crate::theme::Rgba) -> Prim {
    let mut points = Vec::with_capacity(40);
    let corners = [(x + w - r, y + r, -90.0f32), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0)];
    for (cx, cy, start) in corners {
        for step in 0..=8 {
            let angle = (start + step as f32 * 90.0 / 8.0).to_radians();
            points.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
    }
    points.push(points[0]);
    Prim::Line { points, width, color }
}

/// The handles a widget is resized by, as (name, which way its edges move: -1 the left or top
/// one, 1 the right or bottom one). Cards have all four sides and three corners (the fourth
/// holds the ×); the clock, which only grows and shrinks, the corners alone.
pub fn handles(kind: &str) -> &'static [(&'static str, (i8, i8))] {
    const ALL: [(&str, (i8, i8)); 7] = [("n", (0, -1)), ("s", (0, 1)), ("w", (-1, 0)), ("e", (1, 0)), ("nw", (-1, -1)), ("sw", (-1, 1)), ("se", (1, 1))];
    match kind {
        "edge" => &[],
        _ if fills(kind) => &ALL,
        _ => &ALL[4..],
    }
}

/// Which handle a piece is (`edit.size.KIND.DIR`): its widget and the way it moves.
pub fn handle_of(key: &str) -> Option<(&str, (i8, i8))> {
    let (kind, dir) = key.strip_prefix("edit.size.")?.rsplit_once('.')?;
    handles(kind).iter().find(|(name, _)| *name == dir).map(|(_, way)| (kind, *way))
}

/// The editor over the widgets (already placed): a frame round each, lit when pointed at or
/// carried, with handles on its sides and corners to resize it (its size shown while
/// `resizing`); a button to take it off; and a strip at `top` to put the others on, and to finish.
#[allow(clippy::too_many_arguments)]
pub fn editor(widgets: Vec<Piece>, show: &[String], resizing: Option<&str>, active: Option<&str>, t: &Theme, text: &mut Text, w: f32, top: f32) -> Vec<Piece> {
    let mut frames = Vec::new();
    let mut grips = Vec::new();
    let mut removes = Vec::new();
    for widget in &widgets {
        let Some(kind) = widget.key.strip_prefix("widget.") else { continue };
        let lit = active == Some(kind);
        let (fw, fh) = (widget.w + 16.0, widget.h + 16.0);
        let mut frame = Piece::new(format!("edit.frame.{kind}"), fw, fh);
        // As round as the frames are, unless it's been made too small for that
        let r = 26.0f32.min(fw.min(fh) / 2.0 - 1.0);
        if lit {
            frame.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: fh, radius: r, color: t.primary.alpha(0.10) });
        }
        frame.prims.push(outline(0.75, 0.75, fw - 1.5, fh - 1.5, r, 1.5, if lit { t.primary } else { t.outline.alpha(0.8) }));
        frame.pad = 2.0;
        // Where each handle is drawn and caught: a short bar on the middle of each side, a dot
        // where the corner's curve is (the frame's radius in from the corner, at 45°)
        let into = r * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
        // Small, the sides' handles shrink and the corners' take the rest
        let (side_w, side_h) = ((fw - 44.0).max(0.0), (fh - 44.0).max(0.0));
        for &(dir, (dx, dy)) in handles(kind) {
            let (hx, hy, hw, hh) = match (dx, dy) {
                (0, _) => ((fw - side_w) / 2.0, if dy < 0 { -6.0 } else { fh - 6.0 }, side_w, 12.0),
                (_, 0) => (if dx < 0 { -6.0 } else { fw - 6.0 }, (fh - side_h) / 2.0, 12.0, side_h),
                _ => (if dx < 0 { into - 11.0 } else { fw - into - 11.0 }, if dy < 0 { into - 11.0 } else { fh - into - 11.0 }, 22.0, 22.0),
            };
            if lit {
                let (cx, cy) = (hx + hw / 2.0, hy + hh / 2.0);
                let (gw, gh) = match (dx, dy) {
                    (0, _) => (28.0, 5.0),
                    (_, 0) => (5.0, 28.0),
                    _ => (9.0, 9.0),
                };
                frame.prims.push(Prim::Rect { x: cx - gw / 2.0, y: cy - gh / 2.0, w: gw, h: gh, radius: gw.min(gh) / 2.0, color: t.primary });
                frame.pad = 6.0;
            }
            let mut grip = Piece::new(format!("edit.size.{kind}.{dir}"), hw, hh);
            grip.x = widget.x - 8.0 + hx;
            grip.y = widget.y - 8.0 + hy;
            grip.blocks = true;
            grips.push(grip);
        }
        // Its size while it's being resized
        if resizing == Some(kind) {
            let label = format!("{:.0} × {:.0}", widget.w, widget.h);
            let font = Font::new(t.text - 2.0).weight(600).tnum();
            let (lw, lh) = text.measure(&label, font);
            let (bw, bh) = (lw + 16.0, lh + 4.0);
            let x = ((fw - bw) / 2.0).round();
            frame.prims.push(Prim::Rect { x, y: fh + 10.0, w: bw, h: bh, radius: bh / 2.0, color: t.primary });
            frame.prims.push(Prim::Text { x: x + 8.0, y: fh + 12.0, text: label, font, color: t.on_primary });
            frame.pad = bh + 14.0;
        }
        frame.x = widget.x - 8.0;
        frame.y = widget.y - 8.0;
        frames.push(frame);

        let size = 30.0;
        let mut remove = Piece::new(format!("edit.remove.{kind}"), size, size);
        remove.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: size, h: size, radius: size / 2.0, color: t.outline_variant });
        remove.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: size - 2.0, h: size - 2.0, radius: size / 2.0 - 1.0, color: t.container_highest });
        remove.prims.push(Prim::Icon { x: 7.0, y: 7.0, size: 16.0, name: "close".into(), color: t.color_text });
        remove.highlight = true;
        remove.actions.left = Some(Action::WidgetRemove(kind.to_string()));
        remove.x = (widget.x + widget.w + 8.0 - size / 2.0).clamp(4.0, (w - size - 4.0).max(4.0));
        remove.y = (widget.y - 8.0 - size / 2.0).max(4.0);
        removes.push(remove);
    }

    // The strip: a label, a chip for every widget not on the desktop, and Done
    let chip_font = Font::new(t.text - 1.0).weight(500);
    let label_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
    let (bh, gap, pad) = (34.0, 8.0, 14.0);
    let missing: Vec<&str> = KINDS.iter().copied().filter(|k| !show.iter().any(|s| s == k)).collect();
    let label = if missing.is_empty() { "ALL WIDGETS ARE ON THE DESKTOP" } else { "ADD" };
    let (lw, lh) = text.measure(label, label_font);
    let chips: Vec<(String, f32)> = missing.iter().map(|k| (name(k).to_string(), text.measure(name(k), chip_font).0 + 52.0)).collect();
    let (dw, dh) = text.measure("Done", chip_font);
    let done_w = dw + 36.0;
    let inner = lw + 16.0 + chips.iter().map(|(_, cw)| cw + gap).sum::<f32>() + 8.0 + done_w;
    let (cw, ch) = (inner + pad * 2.0, bh + pad * 2.0);
    let x0 = ((w - cw) / 2.0).round();
    let mut strip = Piece::new("edit.strip", cw, ch);
    strip.prims.push(Prim::Card { x: 0.0, y: 0.0, w: cw, h: ch, radius: ch / 2.0 });
    strip.prims.push(Prim::Text { x: pad + 8.0, y: ((ch - lh) / 2.0).round(), text: label.into(), font: label_font, color: t.primary });
    strip.x = x0;
    strip.y = top;
    strip.pad = 14.0;
    strip.blocks = true;
    let mut pieces_after = vec![strip];
    let mut x = x0 + pad + 8.0 + lw + 16.0;
    for ((chip_name, chip_w), kind) in chips.into_iter().zip(&missing) {
        let mut chip = Piece::new(format!("edit.add.{kind}"), chip_w, bh);
        chip.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: chip_w, h: bh, radius: bh / 2.0, color: t.hairline() });
        chip.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: chip_w - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
        chip.prims.push(Prim::Icon { x: 12.0, y: 9.0, size: 16.0, name: "add".into(), color: t.primary });
        let (_, th) = text.measure(&chip_name, chip_font);
        chip.prims.push(Prim::Text { x: 36.0, y: ((bh - th) / 2.0).round(), text: chip_name, font: chip_font, color: t.color_text });
        chip.highlight = true;
        chip.actions.left = Some(Action::WidgetAdd(kind.to_string()));
        chip.x = x;
        chip.y = top + pad;
        pieces_after.push(chip);
        x += chip_w + gap;
    }
    let mut done = Piece::new("edit.done", done_w, bh);
    done.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: done_w, h: bh, radius: bh / 2.0, color: t.primary });
    done.prims.push(Prim::Text { x: 18.0, y: ((bh - dh) / 2.0).round(), text: "Done".into(), font: chip_font, color: t.on_primary });
    done.highlight = true;
    done.actions.left = Some(Action::EditWidgets(Some(false)));
    done.x = x + 8.0;
    done.y = top + pad;
    pieces_after.push(done);

    let hint = "Drag a widget to move it · drag its sides or corners to resize it · Esc or Done when finished";
    let hint_font = Font::new(t.text - 2.0);
    let (hw, hh) = text.measure(hint, hint_font);
    // On a quiet pill, so it reads over a widget too
    let (uw, uh) = (hw + 24.0, hh + 8.0);
    let mut under = Piece::new("edit.hint", uw, uh);
    under.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: uw, h: uh, radius: uh / 2.0, color: t.surface.alpha(0.85) });
    under.prims.push(Prim::Text { x: 12.0, y: 4.0, text: hint.into(), font: hint_font, color: t.color_text.alpha(0.85) });
    under.x = ((w - uw) / 2.0).round();
    under.y = top + ch + 12.0;
    pieces_after.push(under);

    frames.into_iter().chain(widgets).chain(grips).chain(removes).chain(pieces_after).collect()
}

/// One widget, at the origin, laid out in `room` when given (never smaller than as built);
/// `None` when it has nothing to show (nothing playing, say).
pub fn build(kind: &str, v: &mut View<'_>, room: Option<(f32, f32)>) -> Option<Piece> {
    let mut piece = build_widget(kind, v, room)?;
    if v.theme.widget_shadow {
        shadow_words(&mut piece);
    }
    Some(piece)
}

fn build_widget(kind: &str, v: &mut View<'_>, room: Option<(f32, f32)>) -> Option<Piece> {
    let t = v.theme;
    let m = v.model;
    let key = format!("widget.{kind}");
    match kind {
        "clock" => {
            let big = Font::new(104.0).weight(300).tnum();
            let small = Font::new(22.0).weight(400);
            let time = v.now.format("%H:%M").to_string();
            let date = v.now.format("%A, %B %-d").to_string();
            let (tw, th) = v.text.measure(&time, big);
            let (dw, dh) = v.text.measure(&date, small);
            let mut piece = Piece::new(key, tw.max(dw) + 8.0, th + dh - 10.0);
            shadowed(&mut piece, 0.0, 0.0, time, big, t.widget_strong);
            shadowed(&mut piece, 4.0, th - 14.0, date, small, t.widget_soft);
            piece.pad = 6.0;
            Some(piece)
        }
        "weather" => {
            let w = &m.weather;
            if !w.available {
                return None;
            }
            let (cw, ch) = grow((300.0, 116.0), room);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            // Tall enough, the days ahead go along the foot; else today sits in the middle
            const DAYS_H: f32 = 84.0;
            let ahead = &w.days[w.days.len().min(1)..];
            let columns = if ch >= 116.0 + DAYS_H - 12.0 { (((cw - 24.0) / 64.0).floor() as usize).min(ahead.len()) } else { 0 };
            let oy = if columns > 0 { 0.0 } else { ((ch - 116.0) / 2.0).round() };
            piece.prims.push(Prim::Icon { x: 20.0, y: oy + 22.0, size: 44.0, name: crate::weather::icon(w.code, w.is_day).into(), color: t.primary });
            let big = Font::new(34.0).weight(400).tnum();
            let temp = format!("{:.0}°", w.temperature);
            let (tw, th) = v.text.measure(&temp, big);
            piece.prims.push(Prim::Text { x: 80.0, y: oy + 16.0, text: temp, font: big, color: t.widget_strong });
            let what = Font::new(t.text + 1.0).weight(500);
            let describe = crate::bar::Ctx::ellipsize_with(v.text, crate::weather::describe(w.code), what, cw - 108.0 - tw);
            piece.prims.push(Prim::Text { x: 88.0 + tw, y: oy + 22.0, text: describe, font: what, color: t.color_text });
            let small = Font::new(t.text - 1.0);
            piece.prims.push(Prim::Text { x: 88.0 + tw, y: oy + 42.0, text: format!("feels like {:.0}°", w.feels_like), font: small, color: t.color_soft });
            let mut line = w.place.clone();
            if let Some(today) = w.days.first() {
                line = format!("{}{}H {:.0}°  L {:.0}°", line, if line.is_empty() { "" } else { " · " }, today.max, today.min);
            }
            let line = crate::bar::Ctx::ellipsize_with(v.text, &line, small, cw - 40.0);
            piece.prims.push(Prim::Text { x: 20.0, y: oy + 16.0 + th + 14.0, text: line, font: small, color: t.color_soft });
            if columns > 0 {
                let col = (cw - 24.0) / columns as f32;
                let top = ch - DAYS_H;
                let day_font = Font::new(t.text - 2.0).weight(500);
                let figures = Font::new(t.text - 2.0).tnum();
                for (i, day) in ahead.iter().take(columns).enumerate() {
                    let mid = 12.0 + col * (i as f32 + 0.5);
                    let name = day.date.format("%a").to_string();
                    let (nw, _) = v.text.measure(&name, day_font);
                    piece.prims.push(Prim::Text { x: (mid - nw / 2.0).round(), y: top, text: name, font: day_font, color: t.widget_soft });
                    piece.prims.push(Prim::Icon { x: (mid - 12.0).round(), y: top + 22.0, size: 24.0, name: crate::weather::icon(day.code, true).into(), color: t.primary });
                    let range = format!("{:.0}° {:.0}°", day.max, day.min);
                    let (rw, _) = v.text.measure(&range, figures);
                    piece.prims.push(Prim::Text { x: (mid - rw / 2.0).round(), y: top + 52.0, text: range, font: figures, color: t.color_text });
                }
            }
            Some(piece)
        }
        "media" => {
            let player = m.media.player()?;
            if player.title.is_empty() {
                return None;
            }
            let lines = if v.lyrics { m.lyrics.get(&lyrics_key(&player.artist, &player.title)) } else { None };
            // Ask for the words once per song
            let lyrics_wanted = format!("lyrics:{}", lyrics_key(&player.artist, &player.title));
            if v.lyrics && lines.is_none() && !v.requested.contains(&lyrics_wanted) {
                v.requests.push(Command::FetchLyrics { artist: player.artist.clone(), title: player.title.clone(), album: player.album.clone(), seconds: (player.length / 1_000_000) as u32 });
            }
            let seconds = player.position_now() as f64 / 1_000_000.0;
            let (now_line, next_line) = lines.map(|l| lyric_at(l, seconds)).unwrap_or((None, None));
            let has_words = lines.is_some_and(|l| !l.is_empty());
            let (cw, ch) = grow((400.0, if has_words { 172.0 } else { 112.0 }), room);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            // The song at the top (in the middle when there are no words), the words at the foot
            let oy = if has_words { 0.0 } else { ((ch - 112.0) / 2.0).round() };
            match m.art.get(&player.art_url) {
                Some(image) => piece.prims.push(Prim::Image { x: 18.0, y: oy + 18.0, w: 64.0, h: 64.0, image: image.clone(), tint: None, radius: 12.0 }),
                None => {
                    if !player.art_url.is_empty() && !v.requested.contains(&player.art_url) {
                        v.requests.push(Command::FetchArt(player.art_url.clone()));
                    }
                    piece.prims.push(Prim::Rect { x: 18.0, y: oy + 18.0, w: 64.0, h: 64.0, radius: 12.0, color: t.container_high });
                    piece.prims.push(Prim::Icon { x: 36.0, y: oy + 36.0, size: 28.0, name: "music".into(), color: t.widget_strong });
                }
            }
            let title_font = Font::new(t.text + 2.0).weight(500);
            let small = Font::new(t.text - 1.0);
            let room = cw - 100.0 - 18.0;
            let title = crate::bar::Ctx::ellipsize_with(v.text, &player.title, title_font, room);
            let artist = crate::bar::Ctx::ellipsize_with(v.text, &player.artist, small, room);
            piece.prims.push(Prim::Text { x: 98.0, y: oy + 22.0, text: title, font: title_font, color: t.widget_strong });
            piece.prims.push(Prim::Text { x: 98.0, y: oy + 46.0, text: artist, font: small, color: t.color_soft });
            if player.length > 0 {
                let done = (player.position_now() as f32 / player.length as f32).clamp(0.0, 1.0);
                piece.prims.push(Prim::Rect { x: 98.0, y: oy + 74.0, w: room, h: 4.0, radius: 2.0, color: t.outline_variant });
                piece.prims.push(Prim::Rect { x: 98.0, y: oy + 74.0, w: (room * done).max(4.0), h: 4.0, radius: 2.0, color: t.primary });
            }
            if has_words {
                // The line now, bright; the next one, quiet
                let sung = Font::new(t.text + 2.0).weight(500);
                let coming = Font::new(t.text);
                let now_text = crate::bar::Ctx::ellipsize_with(v.text, now_line.unwrap_or("♪"), sung, cw - 36.0);
                let next_text = crate::bar::Ctx::ellipsize_with(v.text, next_line.unwrap_or(""), coming, cw - 36.0);
                piece.prims.push(Prim::Text { x: 18.0, y: ch - 72.0, text: now_text, font: sung, color: t.primary });
                piece.prims.push(Prim::Text { x: 18.0, y: ch - 44.0, text: next_text, font: coming, color: t.color_soft });
            }
            Some(piece)
        }
        "visualizer" => {
            // Only while music plays: a row of dots that rise into bars, up and down from the middle
            let player = m.media.player()?;
            if !player.playing {
                return None;
            }
            let (cw, ch) = grow((420.0, 128.0), room);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            bars(&mut piece, t, &m.spectrum, 24.0, cw - 48.0, ch / 2.0, (ch - 40.0) / 2.0);
            Some(piece)
        }
        "photos" => {
            // A picture from the folder, a new one every so many minutes (in an order of its own)
            let dir = v.settings.photo_dir();
            let list = match &m.photos {
                Some((listed, list)) if *listed == dir => Some(list),
                _ => None,
            };
            if list.is_none() && !v.requested.contains(&format!("photos:{}", dir.display())) {
                v.requests.push(Command::ListPhotos(dir.clone()));
            }
            let (cw, ch) = grow((336.0, 216.0), room);
            let (iw, ih) = (cw - 16.0, ch - 16.0);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            match list.filter(|l| !l.is_empty()) {
                Some(list) => {
                    let turn = (v.now.timestamp() / 60 / v.settings.photo_minutes.max(1) as i64) as u64 + v.photo_step;
                    let path = &list[(turn.wrapping_mul(2_654_435_761) % list.len() as u64) as usize];
                    match m.thumbnails.get(path) {
                        Some(Some(image)) => piece.prims.push(Prim::Image { x: 8.0, y: 8.0, w: iw, h: ih, image: image.clone(), tint: None, radius: 14.0 }),
                        found => {
                            if found.is_none() && !v.requested.contains(&format!("thumb:{}", path.display())) {
                                v.requests.push(Command::Thumbnail(path.clone()));
                            }
                            piece.prims.push(Prim::Rect { x: 8.0, y: 8.0, w: iw, h: ih, radius: 14.0, color: t.container_high });
                            piece.prims.push(Prim::Icon { x: (cw - 32.0) / 2.0, y: (ch - 32.0) / 2.0, size: 32.0, name: "image".into(), color: t.widget_strong });
                        }
                    }
                }
                None => {
                    piece.prims.push(Prim::Icon { x: (cw - 32.0) / 2.0, y: ch / 2.0 - 40.0, size: 32.0, name: "image".into(), color: t.widget_strong });
                    let small = Font::new(t.text - 1.0);
                    let words = if list.is_some() { format!("No pictures in {}", dir.display()) } else { "Looking for pictures…".into() };
                    let words = crate::bar::Ctx::ellipsize_with(v.text, &words, small, cw - 40.0);
                    let (ww, _) = v.text.measure(&words, small);
                    piece.prims.push(Prim::Text { x: ((cw - ww) / 2.0).round(), y: ch / 2.0 + 4.0, text: words, font: small, color: t.color_soft });
                }
            }
            Some(piece)
        }
        "note" => {
            // In the accent's container colour, like a slip of paper; empty, it says how to write one
            let (cw, room_h) = grow((260.0, 0.0), room);
            let font = Font::new(t.text + 1.0);
            let (_, lh) = v.text.measure("Ag", font);
            // As many lines as the paper holds (a long note shows its start)
            let most = (((room_h - 40.0) / lh).floor() as usize).max(14);
            let written = v.settings.note.trim();
            let mut lines = Vec::new();
            for paragraph in written.lines() {
                if paragraph.trim().is_empty() {
                    lines.push(String::new());
                } else {
                    lines.extend(v.text.wrap(paragraph, font, cw - 40.0, most));
                }
            }
            lines.truncate(most);
            let empty = lines.is_empty();
            if empty {
                lines = v.text.wrap("Right-click to write a note", font, cw - 40.0, 2);
            }
            let ch = (40.0 + lines.len() as f32 * lh).max(room_h);
            let mut piece = Piece::new(key, cw, ch);
            // A paper of its own colour, as see-through as the other cards are asked to be
            let paper = (0.92 * t.widget_opacity / 0.6).min(0.92);
            if paper > 0.0 {
                piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: cw, h: ch, radius: 18.0, color: t.primary_container.alpha(paper) });
            }
            if t.widget_outline && paper < 0.3 {
                piece.prims.push(outline(0.5, 0.5, cw - 1.0, ch - 1.0, 17.5, 1.0, t.hairline()));
            }
            for (i, line) in lines.into_iter().enumerate() {
                piece.prims.push(Prim::Text { x: 20.0, y: 20.0 + i as f32 * lh, text: line, font, color: t.on_primary_container.alpha(if empty { 0.7 } else { 1.0 }) });
            }
            Some(piece)
        }
        "calendar" => {
            let (cw, ch) = grow((292.0, 268.0), room);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            // Six weeks' rows share the height below the weekdays' names
            let row_h = ((ch - 88.0) / 6.0).max(30.0);
            let today = v.now.date_naive();
            let head = Font::new(t.text + 2.0).weight(500);
            piece.prims.push(Prim::Text { x: 20.0, y: 16.0, text: v.now.format("%B %Y").to_string(), font: head, color: t.widget_strong });
            let small = Font::new(t.text - 2.0).weight(500);
            let day_font = Font::new(t.text - 1.0).tnum();
            let cell = (cw - 40.0) / 7.0;
            for (i, name) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
                let (nw, _) = v.text.measure(name, small);
                piece.prims.push(Prim::Text { x: 20.0 + i as f32 * cell + (cell - nw) / 2.0, y: 50.0, text: name.to_string(), font: small, color: t.widget_soft });
            }
            use chrono::Datelike;
            let first = today.with_day(1).unwrap_or(today);
            let offset = first.weekday().num_days_from_monday() as i64;
            let days_in_month = (1..=31).filter(|d| today.with_day(*d).is_some()).count() as i64;
            for d in 0..days_in_month {
                let slot = offset + d;
                let (col, row) = ((slot % 7) as f32, (slot / 7) as f32);
                let (x, y) = (20.0 + col * cell, 74.0 + row * row_h);
                let mid = (y + row_h / 2.0).round();
                let label = (d + 1).to_string();
                let (lw, lh) = v.text.measure(&label, day_font);
                let is_today = d + 1 == today.day() as i64;
                if is_today {
                    piece.prims.push(Prim::Rect { x: x + cell / 2.0 - 13.0, y: mid - 13.0, w: 26.0, h: 26.0, radius: 13.0, color: t.primary });
                }
                piece.prims.push(Prim::Text { x: x + (cell - lw) / 2.0, y: mid - lh / 2.0, text: label, font: day_font, color: if is_today { t.on_primary } else { t.color_text } });
            }
            Some(piece)
        }
        "system" => {
            let machine = &m.machine;
            let mut rows: Vec<(&str, f32, String)> = vec![("CPU", machine.cpu, format!("{:.0}%", machine.cpu * 100.0)), ("Memory", machine.memory, format!("{:.0}%", machine.memory * 100.0))];
            if let Some(gpu) = machine.gpu {
                rows.push(("Graphics", gpu, format!("{:.0}%", gpu * 100.0)));
            }
            if let Some((point, used, total)) = machine.disks.first() {
                let share = *used as f32 / (*total).max(1) as f32;
                rows.push((if point == "/" { "Disk" } else { point.as_str() }, share, format!("{:.0}%", share * 100.0)));
            }
            let (cw, ch) = grow((292.0, 32.0 + rows.len() as f32 * 40.0), room);
            // The rows share the height, each in the middle of its share
            let step = ((ch - 32.0) / rows.len() as f32).max(40.0);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            let font = Font::new(t.text - 1.0).weight(500);
            let figure = Font::new(t.text - 1.0).tnum();
            for (i, (name, value, label)) in rows.into_iter().enumerate() {
                let y = (20.0 + i as f32 * step + (step - 40.0) / 2.0).round();
                piece.prims.push(Prim::Text { x: 20.0, y, text: name.to_string(), font, color: t.widget_strong });
                let (lw, _) = v.text.measure(&label, figure);
                piece.prims.push(Prim::Text { x: cw - 20.0 - lw, y, text: label, font: figure, color: t.color_soft });
                piece.prims.push(Prim::Rect { x: 20.0, y: y + 22.0, w: cw - 40.0, h: 4.0, radius: 2.0, color: t.outline_variant });
                piece.prims.push(Prim::Rect { x: 20.0, y: y + 22.0, w: ((cw - 40.0) * value.clamp(0.0, 1.0)).max(4.0), h: 4.0, radius: 2.0, color: if value > 0.9 { t.warn } else { t.primary } });
            }
            Some(piece)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blur_region_follows_the_cards_corners() {
        let rows = rounded_rows(10.0, 20.0, 100.0, 60.0, 20.0);
        let covered = |px: i32, py: i32| rows.iter().any(|&(x, y, w, h)| px >= x && px < x + w && py >= y && py < y + h);
        // The middle and the edges' middles are in, the very corners out
        assert!(covered(60, 50) && covered(10, 50) && covered(109, 50) && covered(60, 20) && covered(60, 79));
        assert!(!covered(10, 20) && !covered(109, 20) && !covered(10, 79) && !covered(109, 79));
        // Nothing outside the card, and alike rows merged (fewer than one per row)
        assert!(rows.iter().all(|&(x, y, w, h)| x >= 10 && y >= 20 && x + w <= 110 && y + h <= 80));
        assert!(rows.len() < 41, "{}", rows.len());
        // No radius: one rectangle; a radius too big for it is cut to half its side
        assert_eq!(rounded_rows(0.0, 0.0, 40.0, 30.0, 0.0), vec![(0, 0, 40, 30)]);
        assert!(rounded_rows(0.0, 0.0, 40.0, 30.0, 99.0).iter().all(|&(_, y, _, h)| y >= 0 && y + h <= 30));
        // Only cards are blurred behind
        assert_eq!((card_radius("clock"), card_radius("edge"), card_radius("weather")), (None, None, Some(20.0)));
    }

    #[test]
    fn reads_places_old_and_new() {
        let places = read_places(r#"{"clock": [0.1, 0.2], "media": [0.3, 0.4, 1.5], "note": [0.5, 0.5, 9.0], "odd": [0.1]}"#);
        assert_eq!(places["clock"], Place { x: 0.1, y: 0.2, scale: 1.0, size: None });
        assert_eq!(places["media"].scale, 1.5);
        assert_eq!(places["note"].scale, BIGGEST);
        assert!(!places.contains_key("odd"));
        assert_eq!(place_of(&places, "calendar"), Place { x: 0.78, y: 0.10, scale: 1.0, size: None });
    }

    #[test]
    fn a_resized_place_keeps_its_box() {
        let mut places = HashMap::new();
        places.insert("media".to_string(), Place { x: 0.25, y: 0.5, scale: 1.0, size: Some((520.4, 180.0)) });
        places.insert("clock".to_string(), Place { x: 0.5, y: 0.5, scale: 1.2, size: None });
        let back = read_places(&write_places(&places).unwrap());
        assert_eq!(back["media"].size, Some((520.0, 180.0)));
        assert_eq!(back["clock"], places["clock"]);
    }

    #[test]
    fn a_box_gives_room_before_it_makes_things_bigger() {
        let natural = (300.0, 100.0);
        // Wider only: the same size of words, with more room beside them
        assert_eq!(fit_box(natural, (600.0, 100.0)), (1.0, (600.0, 100.0)));
        // Twice as big both ways: twice the size, the room as built
        assert_eq!(fit_box(natural, (600.0, 200.0)), (2.0, (300.0, 100.0)));
        // Wide and a little taller: grown by the smaller share, the rest is room
        let (k, room) = fit_box(natural, (900.0, 150.0));
        assert_eq!(k, 1.5);
        assert_eq!(room, (600.0, 100.0));
        // Never past the limits, and never less room than it needs
        // No limits: as small or as big as the box, still laid out in at least its own room
        let (k, room) = fit_box(natural, (30.0, 10.0));
        assert!((k - 0.1).abs() < 0.001);
        assert_eq!(room, natural);
        assert_eq!(fit_box(natural, (3000.0, 1000.0)).0, 10.0);
    }

    #[test]
    fn handles_say_which_way_they_pull() {
        assert_eq!(handle_of("edit.size.media.se"), Some(("media", (1, 1))));
        assert_eq!(handle_of("edit.size.note.w"), Some(("note", (-1, 0))));
        // The clock only by its corners; the ×'s corner has none; the edge bars none at all
        assert_eq!(handle_of("edit.size.clock.e"), None);
        assert_eq!(handle_of("edit.size.clock.nw"), Some(("clock", (-1, -1))));
        assert_eq!(handle_of("edit.size.media.ne"), None);
        assert!(handles("edge").is_empty());
        assert_eq!(handle_of("widget.media"), None);
    }

    #[test]
    fn makes_a_piece_bigger() {
        let mut piece = Piece::new("widget.test", 100.0, 50.0);
        piece.prims.push(Prim::Text { x: 10.0, y: 5.0, text: "hi".into(), font: Font::new(12.0), color: crate::theme::Rgba::WHITE });
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0, radius: 8.0, color: crate::theme::Rgba::WHITE });
        piece.scale(1.5);
        assert_eq!((piece.w, piece.h), (150.0, 75.0));
        assert!(matches!(&piece.prims[0], Prim::Text { x, font, .. } if *x == 15.0 && font.size == 18.0));
        assert!(matches!(&piece.prims[1], Prim::Rect { w, radius, .. } if *w == 150.0 && *radius == 12.0));
    }

    #[test]
    fn reads_synced_lyrics() {
        let lines = parse_lyrics("[ar: someone]\n[00:12.50] First line\n[00:15.00][01:15.00] Chorus\n[00:20.00]\nplain words\n");
        let times: Vec<f64> = lines.iter().map(|l| l.0).collect();
        assert_eq!(times, [12.5, 15.0, 20.0, 75.0]);
        assert_eq!(lyric_at(&lines, 13.0), (Some("First line"), Some("Chorus")));
        assert_eq!(lyric_at(&lines, 16.0), (Some("Chorus"), None));
        assert_eq!(lyric_at(&lines, 5.0), (None, Some("First line")));
        assert_eq!(lyric_at(&lines, 80.0), (Some("Chorus"), None));
    }
}
