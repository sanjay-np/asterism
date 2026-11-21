//! Desktop widgets: a clock, the weather, what's playing (with its lyrics, line by line), a
//! month, and the machine, on the wallpaper under your windows. Where each one stands is kept
//! as a share of the screen in ~/.local/state/asterism/widgets.json, so a drag survives restarts
//! and a different screen size.

use crate::model::{Command, Model};
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Piece, Prim};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Every kind there is, in the order they're offered.
pub const KINDS: [&str; 5] = ["clock", "weather", "media", "calendar", "system"];

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
        "calendar" => (0.78, 0.10),
        _ => (0.78, 0.56),
    }
}

pub fn load_places() -> HashMap<String, (f32, f32)> {
    std::fs::read_to_string(places_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_places(places: &HashMap<String, (f32, f32)>) {
    let path = places_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(places) {
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
}

/// A translucent card with a hairline, for the widgets that need one.
fn card(piece: &mut Piece, t: &Theme, w: f32, h: f32) {
    piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: 20.0, color: t.hairline() });
    piece.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0, radius: 19.0, color: t.surface.alpha(0.62) });
}

/// Text with a soft shadow, to read on any wallpaper.
fn shadowed(piece: &mut Piece, x: f32, y: f32, text: String, font: Font, color: crate::theme::Rgba) {
    piece.prims.push(Prim::Text { x: x + 1.5, y: y + 2.0, text: text.clone(), font, color: crate::theme::Rgba::BLACK.alpha(0.35) });
    piece.prims.push(Prim::Text { x, y, text, font, color });
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
            shadowed(&mut piece, 0.0, 0.0, time, big, t.color_text);
            shadowed(&mut piece, 4.0, th - 14.0, date, small, t.color_text.alpha(0.85));
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
            piece.prims.push(Prim::Text { x: 80.0, y: 16.0, text: temp, font: big, color: t.color_text });
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
                    piece.prims.push(Prim::Icon { x: 36.0, y: 36.0, size: 28.0, name: "music".into(), color: t.color_soft });
                }
            }
            let title_font = Font::new(t.text + 2.0).weight(500);
            let small = Font::new(t.text - 1.0);
            let room = cw - 100.0 - 18.0;
            let title = crate::bar::Ctx::ellipsize_with(v.text, &player.title, title_font, room);
            let artist = crate::bar::Ctx::ellipsize_with(v.text, &player.artist, small, room);
            piece.prims.push(Prim::Text { x: 98.0, y: 22.0, text: title, font: title_font, color: t.color_text });
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
        "calendar" => {
            let (cw, ch) = (292.0, 268.0);
            let mut piece = Piece::new(key, cw, ch);
            card(&mut piece, t, cw, ch);
            let today = v.now.date_naive();
            let head = Font::new(t.text + 2.0).weight(500);
            piece.prims.push(Prim::Text { x: 20.0, y: 16.0, text: v.now.format("%B %Y").to_string(), font: head, color: t.color_text });
            let small = Font::new(t.text - 2.0).weight(500);
            let day_font = Font::new(t.text - 1.0).tnum();
            let cell = (cw - 40.0) / 7.0;
            for (i, name) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
                let (nw, _) = v.text.measure(name, small);
                piece.prims.push(Prim::Text { x: 20.0 + i as f32 * cell + (cell - nw) / 2.0, y: 50.0, text: name.to_string(), font: small, color: t.color_soft });
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
                piece.prims.push(Prim::Text { x: 20.0, y, text: name.to_string(), font, color: t.color_text });
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
