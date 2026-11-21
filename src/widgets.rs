//! Desktop widgets: a clock, the weather, what's playing (with its lyrics, line by line), a
//! visualizer of what the speakers play (in a card, or along the screen's foot), a month, the
//! machine, a photo frame and a sticky note, on the wallpaper under your windows. Where each one
//! stands, and how big it is, is kept in ~/.local/state/asterism/widgets.json (as shares of the
//! screen), so a drag survives restarts and a different screen size.
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

/// How much smaller or bigger a widget can be made.
pub const SMALLEST: f32 = 0.6;
pub const BIGGEST: f32 = 2.0;

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
        "pet" | "pet.friend" => "Your pet",
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

/// Where a widget stands (shares of the screen) and how big it is (1 as built).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

/// A widget's place: as saved, else where it starts out.
pub fn place_of(places: &HashMap<String, Place>, kind: &str) -> Place {
    places.get(kind).copied().unwrap_or_else(|| {
        let (x, y) = default_place(kind);
        Place { x, y, scale: 1.0 }
    })
}

/// Places from their file: `[x, y, scale]` each (older files have no scale).
pub fn read_places(text: &str) -> HashMap<String, Place> {
    let raw: HashMap<String, Vec<f32>> = serde_json::from_str(text).unwrap_or_default();
    raw.into_iter()
        .filter_map(|(kind, v)| Some((kind, Place { x: *v.first()?, y: *v.get(1)?, scale: v.get(2).copied().unwrap_or(1.0).clamp(SMALLEST, BIGGEST) })))
        .collect()
}

pub fn load_places() -> HashMap<String, Place> {
    std::fs::read_to_string(places_path()).map(|t| read_places(&t)).unwrap_or_default()
}

pub fn save_places(places: &HashMap<String, Place>) {
    let path = places_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let raw: HashMap<&String, [f32; 3]> = places.iter().map(|(kind, p)| (kind, [p.x, p.y, p.scale])).collect();
    if let Ok(text) = serde_json::to_string_pretty(&raw) {
        let _ = std::fs::write(path, text);
    }
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

/// A translucent card with a hairline, for the widgets that need one.
fn card(piece: &mut Piece, t: &Theme, w: f32, h: f32) {
    piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: 20.0, color: t.hairline() });
    piece.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0, radius: 19.0, color: t.widget_card });
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
pub fn placeholder(kind: &str, t: &Theme, text: &mut Text) -> Piece {
    let (w, h) = match kind {
        "media" => (400.0, 112.0),
        "visualizer" => (420.0, 128.0),
        "weather" => (300.0, 116.0),
        "photos" => (336.0, 216.0),
        _ => (292.0, 112.0),
    };
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
/// from every edge). In the editor, ones with nothing to show stand in as placeholders.
pub fn arrange(show: &[String], v: &mut View<'_>, places: &HashMap<String, Place>, size: (f32, f32), editing: bool) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for kind in show {
        // Along the screen's foot, the whole width: it isn't placed, nor made bigger
        if kind == "edge" {
            pieces.extend(edge(v, size, editing));
            continue;
        }
        let piece = match build(kind, v) {
            Some(piece) => Some(piece),
            None if editing && KINDS.contains(&kind.as_str()) => Some(placeholder(kind, v.theme, v.text)),
            None => None,
        };
        let Some(mut piece) = piece else { continue };
        let place = place_of(places, kind);
        piece.scale(place.scale);
        piece.x = (place.x * size.0).clamp(16.0, (size.0 - piece.w - 16.0).max(16.0)).round();
        piece.y = (place.y * size.1).clamp(16.0, (size.1 - piece.h - 16.0).max(16.0)).round();
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

/// The editor over the widgets (already placed): a frame round each, lit when pointed at or
/// carried; a button to take it off; and a strip at `top` to put the others on, and to finish.
#[allow(clippy::too_many_arguments)]
pub fn editor(widgets: Vec<Piece>, show: &[String], places: &HashMap<String, Place>, active: Option<&str>, t: &Theme, text: &mut Text, w: f32, top: f32) -> Vec<Piece> {
    let mut frames = Vec::new();
    let mut removes = Vec::new();
    for widget in &widgets {
        let Some(kind) = widget.key.strip_prefix("widget.") else { continue };
        let lit = active == Some(kind);
        let mut frame = Piece::new(format!("edit.frame.{kind}"), widget.w + 16.0, widget.h + 16.0);
        if lit {
            frame.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: frame.w, h: frame.h, radius: 26.0, color: t.primary.alpha(0.10) });
        }
        frame.prims.push(outline(0.75, 0.75, frame.w - 1.5, frame.h - 1.5, 26.0, 1.5, if lit { t.primary } else { t.outline.alpha(0.8) }));
        // Its size, while it's pointed at and not as built
        let scale = place_of(places, kind).scale;
        if lit && (scale - 1.0).abs() > 0.01 {
            let label = format!("{:.0}%", scale * 100.0);
            let font = Font::new(t.text - 2.0).weight(600).tnum();
            let (lw, lh) = text.measure(&label, font);
            let (bw, bh) = (lw + 16.0, lh + 4.0);
            frame.prims.push(Prim::Rect { x: 14.0, y: frame.h - bh / 2.0 - 1.0, w: bw, h: bh, radius: bh / 2.0, color: t.primary });
            frame.prims.push(Prim::Text { x: 22.0, y: frame.h - bh / 2.0 + 1.0, text: label, font, color: t.on_primary });
            frame.pad = bh / 2.0 + 2.0;
        }
        frame.x = widget.x - 8.0;
        frame.y = widget.y - 8.0;
        frame.pad = 2.0;
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

    let hint = "Drag a widget to move it · scroll on it to make it bigger or smaller · Esc or Done when finished";
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

    frames.into_iter().chain(widgets).chain(removes).chain(pieces_after).collect()
}

/// One widget, at the origin; `None` when it has nothing to show (nothing playing, say).
pub fn build(kind: &str, v: &mut View<'_>) -> Option<Piece> {
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
            let (cw, ch) = (300.0, 116.0);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            piece.prims.push(Prim::Icon { x: 20.0, y: 22.0, size: 44.0, name: crate::weather::icon(w.code, w.is_day).into(), color: t.primary });
            let big = Font::new(34.0).weight(400).tnum();
            let temp = format!("{:.0}°", w.temperature);
            let (tw, th) = v.text.measure(&temp, big);
            piece.prims.push(Prim::Text { x: 80.0, y: 16.0, text: temp, font: big, color: t.widget_strong });
            let what = Font::new(t.text + 1.0).weight(500);
            piece.prims.push(Prim::Text { x: 88.0 + tw, y: 22.0, text: crate::weather::describe(w.code).into(), font: what, color: t.color_text });
            let small = Font::new(t.text - 1.0);
            piece.prims.push(Prim::Text { x: 88.0 + tw, y: 42.0, text: format!("feels like {:.0}°", w.feels_like), font: small, color: t.color_soft });
            let mut line = w.place.clone();
            if let Some(today) = w.days.first() {
                line = format!("{}{}H {:.0}°  L {:.0}°", line, if line.is_empty() { "" } else { " · " }, today.max, today.min);
            }
            let line = crate::bar::Ctx::ellipsize_with(v.text, &line, small, cw - 40.0);
            piece.prims.push(Prim::Text { x: 20.0, y: 16.0 + th + 14.0, text: line, font: small, color: t.color_soft });
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
            let (cw, ch) = (400.0, if has_words { 172.0 } else { 112.0 });
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            match m.art.get(&player.art_url) {
                Some(image) => piece.prims.push(Prim::Image { x: 18.0, y: 18.0, w: 64.0, h: 64.0, image: image.clone(), tint: None, radius: 12.0 }),
                None => {
                    if !player.art_url.is_empty() && !v.requested.contains(&player.art_url) {
                        v.requests.push(Command::FetchArt(player.art_url.clone()));
                    }
                    piece.prims.push(Prim::Rect { x: 18.0, y: 18.0, w: 64.0, h: 64.0, radius: 12.0, color: t.container_high });
                    piece.prims.push(Prim::Icon { x: 36.0, y: 36.0, size: 28.0, name: "music".into(), color: t.widget_strong });
                }
            }
            let title_font = Font::new(t.text + 2.0).weight(500);
            let small = Font::new(t.text - 1.0);
            let room = cw - 100.0 - 18.0;
            let title = crate::bar::Ctx::ellipsize_with(v.text, &player.title, title_font, room);
            let artist = crate::bar::Ctx::ellipsize_with(v.text, &player.artist, small, room);
            piece.prims.push(Prim::Text { x: 98.0, y: 22.0, text: title, font: title_font, color: t.widget_strong });
            piece.prims.push(Prim::Text { x: 98.0, y: 46.0, text: artist, font: small, color: t.color_soft });
            if player.length > 0 {
                let done = (player.position_now() as f32 / player.length as f32).clamp(0.0, 1.0);
                piece.prims.push(Prim::Rect { x: 98.0, y: 74.0, w: room, h: 4.0, radius: 2.0, color: t.outline_variant });
                piece.prims.push(Prim::Rect { x: 98.0, y: 74.0, w: (room * done).max(4.0), h: 4.0, radius: 2.0, color: t.primary });
            }
            if has_words {
                // The line now, bright; the next one, quiet
                let sung = Font::new(t.text + 2.0).weight(500);
                let coming = Font::new(t.text);
                let now_text = crate::bar::Ctx::ellipsize_with(v.text, now_line.unwrap_or("♪"), sung, cw - 36.0);
                let next_text = crate::bar::Ctx::ellipsize_with(v.text, next_line.unwrap_or(""), coming, cw - 36.0);
                piece.prims.push(Prim::Text { x: 18.0, y: 100.0, text: now_text, font: sung, color: t.primary });
                piece.prims.push(Prim::Text { x: 18.0, y: 128.0, text: next_text, font: coming, color: t.color_soft });
            }
            Some(piece)
        }
        "visualizer" => {
            // Only while music plays: a row of dots that rise into bars, up and down from the middle
            let player = m.media.player()?;
            if !player.playing {
                return None;
            }
            let (cw, ch) = (420.0, 128.0);
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
            let (cw, ch) = (336.0, 216.0);
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
            let cw = 260.0;
            let font = Font::new(t.text + 1.0);
            let (_, lh) = v.text.measure("Ag", font);
            let written = v.settings.note.trim();
            let mut lines = Vec::new();
            for paragraph in written.lines() {
                if paragraph.trim().is_empty() {
                    lines.push(String::new());
                } else {
                    lines.extend(v.text.wrap(paragraph, font, cw - 40.0, 14));
                }
            }
            lines.truncate(14);
            let empty = lines.is_empty();
            if empty {
                lines = v.text.wrap("Right-click to write a note", font, cw - 40.0, 2);
            }
            let ch = 40.0 + lines.len() as f32 * lh;
            let mut piece = Piece::new(key, cw, ch);
            piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: cw, h: ch, radius: 18.0, color: t.primary_container.alpha(0.92) });
            for (i, line) in lines.into_iter().enumerate() {
                piece.prims.push(Prim::Text { x: 20.0, y: 20.0 + i as f32 * lh, text: line, font, color: t.on_primary_container.alpha(if empty { 0.7 } else { 1.0 }) });
            }
            Some(piece)
        }
        "calendar" => {
            let (cw, ch) = (292.0, 268.0);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
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
                let (x, y) = (20.0 + col * cell, 74.0 + row * 30.0);
                let label = (d + 1).to_string();
                let (lw, lh) = v.text.measure(&label, day_font);
                let is_today = d + 1 == today.day() as i64;
                if is_today {
                    piece.prims.push(Prim::Rect { x: x + cell / 2.0 - 13.0, y: y + 15.0 - 13.0, w: 26.0, h: 26.0, radius: 13.0, color: t.primary });
                }
                piece.prims.push(Prim::Text { x: x + (cell - lw) / 2.0, y: y + 15.0 - lh / 2.0, text: label, font: day_font, color: if is_today { t.on_primary } else { t.color_text } });
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
            let (cw, ch) = (292.0, 32.0 + rows.len() as f32 * 40.0);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            let font = Font::new(t.text - 1.0).weight(500);
            let figure = Font::new(t.text - 1.0).tnum();
            for (i, (name, value, label)) in rows.into_iter().enumerate() {
                let y = 20.0 + i as f32 * 40.0;
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
    fn reads_places_old_and_new() {
        let places = read_places(r#"{"clock": [0.1, 0.2], "media": [0.3, 0.4, 1.5], "note": [0.5, 0.5, 9.0], "odd": [0.1]}"#);
        assert_eq!(places["clock"], Place { x: 0.1, y: 0.2, scale: 1.0 });
        assert_eq!(places["media"].scale, 1.5);
        assert_eq!(places["note"].scale, BIGGEST);
        assert!(!places.contains_key("odd"));
        assert_eq!(place_of(&places, "calendar"), Place { x: 0.78, y: 0.10, scale: 1.0 });
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
