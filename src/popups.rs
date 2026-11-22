//! The menus that open from the bar, each one a card of night sky: the calendar, the music,
//! the sound, the network, bluetooth, the battery, the session, the machine, the keyboard and
//! the tray apps' own menus.

use crate::bar::{rate, send};
use crate::config::Config;
use crate::model::{Battery, Command, MenuEntry, Model};
use crate::render::text::{Font, Text};
use crate::theme::{Rgba, Theme};
use crate::ui::{Action, Animator, Ease, Piece, Prim, Slider};
use chrono::{Datelike, Duration, NaiveDate};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Calendar,
    Media,
    Sound,
    Network,
    Bluetooth,
    Power,
    Session,
    Machine,
    Traffic,
    Keyboard,
    Notifications,
    Timer,
    Weather,
    Capture,
    Tray(String),
}

impl Kind {
    fn width(&self) -> f32 {
        match self {
            Kind::Calendar => 300.0,
            Kind::Media | Kind::Sound => 320.0,
            Kind::Session | Kind::Keyboard => 230.0,
            Kind::Traffic => 260.0,
            Kind::Tray(_) => 270.0,
            Kind::Notifications => 380.0,
            _ => 300.0,
        }
    }
}

/// What a menu remembers while it's open.
#[derive(Default)]
pub struct State {
    pub month: i32,
    pub tray_path: Vec<i32>,
    pub armed: Option<String>,
    /// A slider being dragged keeps the value under the pointer, not the one last reported.
    pub dragging: Option<(String, f32)>,
    /// The day panel: what's being typed into (1 a new task, 2 the notes, 0 nothing), and the
    /// new task so far.
    pub day_field: u8,
    pub day_typed: String,
}

pub struct Ctx<'a> {
    pub model: &'a Model,
    pub config: &'a Config,
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub anim: &'a mut Animator,
    pub state: &'a State,
    pub now: chrono::DateTime<chrono::Local>,
    /// Asked for once, like an album cover not fetched yet.
    pub requests: Vec<Command>,
    pub requested: &'a HashSet<String>,
}

pub struct Built {
    pub w: f32,
    pub h: f32,
    pub pieces: Vec<Piece>,
}

const PAD: f32 = 18.0;
const ROW: f32 = 34.0;

/// How a row ends on the right.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    None,
    Switch(bool),
    Check(bool),
    Radio(bool),
}

struct Col<'c, 'a> {
    ctx: &'c mut Ctx<'a>,
    pieces: Vec<Piece>,
    y: f32,
    w: f32,
}

pub fn battery_line(b: &Battery) -> String {
    let level = (b.level * 100.0).round();
    let time = |seconds: i64| {
        let minutes = seconds / 60;
        if minutes >= 60 { format!("{}h {:02}min", minutes / 60, minutes % 60) } else { format!("{minutes} min") }
    };
    if b.full || (!b.on_battery && level >= 99.0) {
        format!("{level}% · full")
    } else if !b.on_battery {
        if b.time_to_full > 0 { format!("{level}% · full in {}", time(b.time_to_full)) } else { format!("{level}% · charging") }
    } else if b.time_to_empty > 0 {
        format!("{level}% · {} left", time(b.time_to_empty))
    } else {
        format!("{level}%")
    }
}

fn clock_time(micros: i64) -> String {
    let total = (micros.max(0) / 1_000_000) as u64;
    let (minutes, seconds) = (total / 60, total % 60);
    if minutes >= 60 { format!("{}:{:02}:{seconds:02}", minutes / 60, minutes % 60) } else { format!("{minutes}:{seconds:02}") }
}

/// The moon's age in days, 0 at new moon, from a known new moon and the synodic month.
fn moon_age(date: NaiveDate) -> f64 {
    let known = NaiveDate::from_ymd_opt(2000, 1, 6).expect("date");
    let days = (date - known).num_days() as f64 + 0.5 - 0.76;
    days.rem_euclid(29.530_588_853)
}

fn moon_name(age: f64) -> &'static str {
    let k = age / 29.530_588_853;
    match (k * 8.0).round() as i32 % 8 {
        0 => "New moon",
        1 => "Waxing crescent",
        2 => "First quarter",
        3 => "Waxing gibbous",
        4 => "Full moon",
        5 => "Waning gibbous",
        6 => "Last quarter",
        _ => "Waning crescent",
    }
}

impl<'c, 'a> Col<'c, 'a> {
    fn theme(&self) -> &'a Theme {
        self.ctx.theme
    }

    fn push(&mut self, mut piece: Piece, h: f32) {
        piece.x = PAD;
        piece.y = self.y;
        self.y += h;
        self.pieces.push(piece);
    }

    fn gap(&mut self, h: f32) {
        self.y += h;
    }

    fn title(&mut self, key: &str, title: &str, subtitle: &str) {
        let t = self.theme();
        let font = Font::new(t.text + 5.0).weight(500);
        let small = Font::new(t.text - 1.0);
        let (_, th) = self.ctx.text.measure(title, font);
        let (_, sh) = if subtitle.is_empty() { (0.0, 0.0) } else { self.ctx.text.measure(subtitle, small) };
        let mut piece = Piece::new(key, self.w, th + sh);
        piece.prims.push(Prim::Text { x: 0.0, y: 0.0, text: title.into(), font, color: t.color_text });
        if !subtitle.is_empty() {
            let shown = crate::bar::Ctx::ellipsize_with(self.ctx.text, subtitle, small, self.w);
            piece.prims.push(Prim::Text { x: 0.0, y: th - 2.0, text: shown, font: small, color: t.color_soft });
        }
        self.push(piece, th + sh + 8.0);
    }

    fn separator(&mut self) {
        let t = self.theme();
        let mut piece = Piece::new(format!("sep.{}", self.y), self.w, 9.0);
        piece.prims.push(Prim::Line { points: vec![(0.0, 4.5), (self.w, 4.5)], width: 1.0, color: t.outline_variant });
        self.push(piece, 9.0);
    }

    fn note(&mut self, text: &str) {
        let t = self.theme();
        let font = Font::new(t.text - 1.0);
        let (_, th) = self.ctx.text.measure(text, font);
        let shown = crate::bar::Ctx::ellipsize_with(self.ctx.text, text, font, self.w);
        let mut piece = Piece::new(format!("note.{}", self.y), self.w, th);
        piece.prims.push(Prim::Text { x: 0.0, y: 0.0, text: shown, font, color: t.color_soft });
        self.push(piece, th + 4.0);
    }

    /// A row of pill buttons sharing the width; a filled one is the main thing to do.
    fn buttons(&mut self, key: &str, items: Vec<(String, bool, Option<Action>)>) {
        let t = self.theme();
        let (gap, h) = (8.0, 34.0);
        let n = items.len().max(1) as f32;
        let w = ((self.w - gap * (n - 1.0)) / n).floor();
        let font = Font::new(t.text - 1.0).weight(500);
        for (i, (label, filled, action)) in items.into_iter().enumerate() {
            let mut piece = Piece::new(format!("{key}.{i}"), w, h);
            if filled {
                piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.primary });
            } else {
                outlined(&mut piece, t, w, h, h / 2.0, t.card_deep);
            }
            let shown = crate::bar::Ctx::ellipsize_with(self.ctx.text, &label, font, w - 12.0);
            let (lw, lh) = self.ctx.text.measure(&shown, font);
            piece.prims.push(Prim::Text { x: ((w - lw) / 2.0).round(), y: ((h - lh) / 2.0).round(), text: shown, font, color: if filled { t.on_primary } else { t.color_text } });
            piece.highlight = true;
            piece.actions.left = action;
            piece.x = PAD + i as f32 * (w + gap);
            piece.y = self.y;
            self.pieces.push(piece);
        }
        self.y += h + 10.0;
    }

    /// A big time, with a word beside it, and a thin bar for how far along it is.
    fn big_time(&mut self, key: &str, time: &str, about: &str, progress: Option<f32>, color: Rgba) {
        let t = self.theme();
        let font = Font::new(t.text + 17.0).weight(500).tnum();
        let small = Font::new(t.text - 1.0);
        let (tw, th) = self.ctx.text.measure(time, font);
        let (_, sh) = self.ctx.text.measure("Ag", small);
        let mut piece = Piece::new(key, self.w, th);
        piece.prims.push(Prim::Text { x: 0.0, y: 0.0, text: time.into(), font, color: t.color_text });
        let about = crate::bar::Ctx::ellipsize_with(self.ctx.text, about, small, self.w - tw - 12.0);
        piece.prims.push(Prim::Text { x: tw + 10.0, y: th - sh - 6.0, text: about, font: small, color: t.color_soft });
        self.push(piece, th + 2.0);
        if let Some(progress) = progress {
            let mut bar = Piece::new(format!("{key}.bar"), self.w, 4.0);
            bar.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: self.w, h: 4.0, radius: 2.0, color: t.outline_variant });
            bar.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: (self.w * progress.clamp(0.0, 1.0)).max(4.0), h: 4.0, radius: 2.0, color });
            self.push(bar, 14.0);
        }
    }

    /// A small letter-spaced capital label over a group, in the accent.
    fn label(&mut self, text: &str) {
        let t = self.theme();
        let font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
        let text = text.to_uppercase();
        let (_, th) = self.ctx.text.measure(&text, font);
        let shown = crate::bar::Ctx::ellipsize_with(self.ctx.text, &text, font, self.w);
        let mut piece = Piece::new(format!("label.{}", self.y), self.w, th);
        piece.prims.push(Prim::Text { x: 0.0, y: 0.0, text: shown, font, color: t.primary });
        self.push(piece, th + 6.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn row(&mut self, key: &str, icon: Option<&str>, label: &str, value: &str, mark: Mark, enabled: bool, action: Option<Action>) {
        let t = self.theme();
        let font = Font::new(t.text);
        let small = Font::new(t.text - 1.0).tnum();
        let mut piece = Piece::new(key, self.w, ROW);
        let dim = if enabled { 1.0 } else { 0.4 };
        let mut x = 6.0;
        if let Some(icon) = icon {
            piece.prims.push(Prim::Icon { x, y: (ROW - 18.0) / 2.0, size: 18.0, name: icon.into(), color: t.color_text.fade(dim) });
            x += 26.0;
        }
        let mark_w = match mark {
            Mark::None => 0.0,
            Mark::Switch(_) => 38.0,
            Mark::Check(_) | Mark::Radio(_) => 22.0,
        };
        let (vw, vh) = if value.is_empty() { (0.0, 0.0) } else { self.ctx.text.measure(value, small) };
        let right = self.w - 6.0 - mark_w - if vw > 0.0 { vw + 8.0 } else { 0.0 };
        let (_, lh) = self.ctx.text.measure("Ag", font);
        let shown = crate::bar::Ctx::ellipsize_with(self.ctx.text, label, font, (right - x).max(10.0));
        piece.prims.push(Prim::Text { x, y: (ROW - lh) / 2.0, text: shown, font, color: t.color_text.fade(dim) });
        if vw > 0.0 {
            piece.prims.push(Prim::Text { x: right + 8.0, y: (ROW - vh) / 2.0, text: value.into(), font: small, color: t.color_soft.fade(dim) });
        }
        let end = self.w - 6.0;
        match mark {
            Mark::Switch(on) => {
                // A compact switch: a quiet track with a soft thumb, the track in the accent when on
                let glide = self.ctx.anim.value(&format!("{key}.switch"), if on { 1.0 } else { 0.0 }, t.normal, Ease::Expressive);
                let (sw, sh) = (36.0, 20.0);
                let sx = end - sw;
                let sy = (ROW - sh) / 2.0;
                piece.prims.push(Prim::Rect { x: sx, y: sy, w: sw, h: sh, radius: sh / 2.0, color: t.outline_variant.mix(t.primary, glide.clamp(0.0, 1.0)) });
                let thumb = 14.0;
                let cx = sx + sh / 2.0 + (sw - sh) * glide;
                piece.prims.push(Prim::Rect { x: cx - thumb / 2.0, y: ROW / 2.0 - thumb / 2.0, w: thumb, h: thumb, radius: thumb / 2.0, color: t.color_soft.mix(t.on_primary, glide.clamp(0.0, 1.0)) });
            }
            Mark::Radio(on) => {
                // A ring, with a dot inside when chosen
                let (cx, cy) = (end - 10.0, ROW / 2.0);
                let ring = if on { t.primary } else { t.color_soft };
                piece.prims.push(Prim::Rect { x: cx - 10.0, y: cy - 10.0, w: 20.0, h: 20.0, radius: 10.0, color: ring });
                piece.prims.push(Prim::Rect { x: cx - 8.0, y: cy - 8.0, w: 16.0, h: 16.0, radius: 8.0, color: t.card_deep });
                if on {
                    piece.prims.push(Prim::Rect { x: cx - 5.0, y: cy - 5.0, w: 10.0, h: 10.0, radius: 5.0, color: t.primary });
                }
            }
            Mark::Check(on) => {
                if on {
                    piece.prims.push(Prim::Icon { x: end - 20.0, y: (ROW - 20.0) / 2.0, size: 20.0, name: "check".into(), color: t.primary });
                }
            }
            Mark::None => {}
        }
        if enabled && action.is_some() {
            piece.highlight = true;
            piece.actions.left = action;
        }
        self.push(piece, ROW);
    }

    /// A slider is a line with a star on it: the star is the value.
    fn slider(&mut self, key: &str, icon: Option<(&str, Option<Action>)>, value: f32, max: f32, slider: Slider, label: &str) {
        let t = self.theme();
        let value = match &self.ctx.state.dragging {
            Some((k, v)) if k == key => *v,
            _ => value,
        };
        let height = 30.0;
        let mut x0 = 4.0;
        if let Some((name, action)) = icon {
            let mut button = Piece::new(format!("{key}.icon"), 22.0, height);
            button.prims.push(Prim::Icon { x: 2.0, y: (height - 18.0) / 2.0, size: 18.0, name: name.into(), color: t.color_text });
            button.actions.left = action;
            button.lift = true;
            button.x = PAD;
            button.y = self.y;
            self.pieces.push(button);
            x0 = 30.0;
        }
        let font = Font::new(t.text - 1.0).tnum();
        let (lw, lh) = if label.is_empty() { (0.0, 0.0) } else { self.ctx.text.measure(label, font) };
        let label_room = if label.is_empty() { 0.0 } else { lw.max(34.0) + 10.0 };
        let mut piece = Piece::new(key, self.w - x0 - label_room, height);
        let track = piece.w - 14.0;
        let fraction = (value / max).clamp(0.0, 1.0);
        let cy = height / 2.0;
        // A thin track, filled in the accent up to a round thumb
        let at = 7.0 + track * fraction;
        let bar = 4.0;
        piece.prims.push(Prim::Rect { x: 7.0, y: cy - bar / 2.0, w: track, h: bar, radius: bar / 2.0, color: t.outline_variant });
        if at - 7.0 > 0.5 {
            piece.prims.push(Prim::Rect { x: 7.0, y: cy - bar / 2.0, w: at - 7.0, h: bar, radius: bar / 2.0, color: t.primary });
        }
        let thumb = 7.0;
        piece.prims.push(Prim::Rect { x: at - thumb, y: cy - thumb, w: thumb * 2.0, h: thumb * 2.0, radius: thumb, color: t.primary });
        piece.pad = 18.0;
        piece.actions.drag = Some((slider, max));
        piece.x = PAD + x0;
        piece.y = self.y;
        self.pieces.push(piece);
        if !label.is_empty() {
            let mut text = Piece::new(format!("{key}.label"), label_room, height);
            text.prims.push(Prim::Text { x: label_room - lw, y: (height - lh) / 2.0, text: label.into(), font, color: t.color_soft });
            text.x = PAD + self.w - label_room;
            text.y = self.y;
            self.pieces.push(text);
        }
        self.y += height + 2.0;
    }
}

impl crate::bar::Ctx<'_> {
    pub fn ellipsize_with(text: &mut Text, value: &str, font: Font, limit: f32) -> String {
        if text.measure(value, font).0 <= limit {
            return value.to_string();
        }
        let chars: Vec<char> = value.chars().collect();
        let (mut low, mut high) = (0, chars.len());
        while low < high {
            let mid = (low + high).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if text.measure(&candidate, font).0 <= limit { low = mid } else { high = mid - 1 }
        }
        chars[..low].iter().collect::<String>().trim_end().to_string() + "…"
    }
}

pub fn build(kind: &Kind, ctx: &mut Ctx<'_>) -> Built {
    let w = kind.width();
    let col_theme = ctx.theme;
    let mut col = Col { ctx, pieces: Vec::new(), y: PAD, w: w - PAD * 2.0 };
    match kind {
        Kind::Calendar => calendar(&mut col),
        Kind::Media => media(&mut col),
        Kind::Sound => sound(&mut col),
        Kind::Network => network(&mut col),
        Kind::Bluetooth => bluetooth(&mut col),
        Kind::Power => power(&mut col),
        Kind::Session => session(&mut col),
        Kind::Machine => machine(&mut col),
        Kind::Traffic => traffic(&mut col),
        Kind::Keyboard => keyboard(&mut col),
        Kind::Notifications => notifications(&mut col),
        Kind::Timer => timer(&mut col),
        Kind::Weather => weather(&mut col),
        Kind::Capture => capture(&mut col),
        Kind::Tray(id) => tray(&mut col, id),
    }
    let h = (col.y - 4.0 + PAD).round();
    let mut pieces = Vec::with_capacity(col.pieces.len() + 1);
    pieces.push(card(col_theme, w, h, if col_theme.on_top { 1 } else { 4 }));
    pieces.extend(col.pieces);
    Built { w, h, pieces }
}

/// What a menu or panel stands on: with the frame, a drawer in the bar's colour attached to
/// `attach` (bit 1 top, 2 right, 4 bottom, 8 left); without it, a floating card.
pub fn card(t: &Theme, w: f32, h: f32, attach: u8) -> Piece {
    let mut card = Piece::new("card", w, h);
    if t.framed() {
        card.prims.push(Prim::Drawer { x: 0.0, y: 0.0, w, h, radius: 14.0, attach, color: t.surface });
    } else {
        card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: t.radius });
    }
    card.pad = 14.0;
    card.blocks = true;
    card
}

impl Built {
    /// Stretches the card to `h` (the control centre runs the screen's height).
    pub fn stretch(&mut self, h: f32) {
        self.h = h;
        if let Some(card) = self.pieces.iter_mut().find(|p| p.key == "card") {
            card.h = h;
            for prim in &mut card.prims {
                if let Prim::Card { h: ch, .. } | Prim::Drawer { h: ch, .. } = prim {
                    *ch = h;
                }
            }
        }
    }

    /// Cuts the card down to `room` when the content is taller: what's below then scrolls.
    /// Returns the content's full height.
    pub fn fit(&mut self, room: f32) -> f32 {
        let content = self.h;
        if content <= room {
            return content;
        }
        self.h = room.round();
        if let Some(card) = self.pieces.iter_mut().find(|p| p.key == "card") {
            card.h = self.h;
            for prim in &mut card.prims {
                if let Prim::Card { h, .. } | Prim::Drawer { h, .. } = prim {
                    *h = self.h;
                }
            }
        }
        content
    }
}

fn calendar(col: &mut Col) {
    let t = col.theme();
    let today = col.ctx.now.date_naive();
    let first_this = today.with_day(1).expect("first");
    let months = first_this.year() * 12 + first_this.month0() as i32 + col.ctx.state.month;
    let first = NaiveDate::from_ymd_opt(months.div_euclid(12), months.rem_euclid(12) as u32 + 1, 1).expect("month");
    let age = moon_age(today);
    let subtitle = if col.ctx.state.month == 0 { moon_name(age).to_string() } else { "Scroll or click the title to come back".into() };

    // The month as a title, the moon beside it, arrows to move
    let title_top = col.y;
    col.title("cal.title", &first.format("%B %Y").to_string(), &subtitle);
    if let Some(title) = col.pieces.last_mut() {
        title.actions.left = Some(Action::Month(0));
        title.actions.up = Some(Action::Month(-1));
        title.actions.down = Some(Action::Month(1));
    }
    let r = 9.0;
    let mut moon = Piece::new("cal.moon", r * 2.0, r * 2.0);
    let k = age / 29.530_588_853;
    let lit = (1.0 - (std::f64::consts::TAU * k).cos()) / 2.0;
    moon.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: r * 2.0, h: r * 2.0, radius: r, color: t.color_text.alpha(0.92) });
    let dx = if k < 0.5 { -2.0 * r * lit as f32 } else { 2.0 * r * lit as f32 };
    moon.prims.push(Prim::Rect { x: dx, y: 0.0, w: r * 2.0, h: r * 2.0, radius: r, color: t.card_deep.alpha(0.94) });
    moon.label = Some(moon_name(age).into());
    moon.x = PAD + col.w - 70.0;
    moon.y = title_top + 4.0;
    col.pieces.push(moon);
    for (i, (icon, step)) in [("caret-left", -1), ("caret-right", 1)].into_iter().enumerate() {
        let mut arrow = Piece::new(format!("cal.{icon}"), 20.0, 24.0);
        arrow.prims.push(Prim::Icon { x: 1.0, y: 3.0, size: 18.0, name: icon.into(), color: t.color_soft });
        arrow.actions.left = Some(Action::Month(col.ctx.state.month + step));
        arrow.lift = true;
        arrow.x = PAD + col.w - 42.0 + i as f32 * 22.0;
        arrow.y = title_top + 1.0;
        col.pieces.push(arrow);
    }

    let cell = col.w / 7.0;
    let small = Font::new(t.text - 3.0);
    let mut header = Piece::new("cal.days", col.w, 18.0);
    for (i, day) in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].iter().enumerate() {
        let (dw, _) = col.ctx.text.measure(day, small);
        header.prims.push(Prim::Text { x: i as f32 * cell + (cell - dw) / 2.0, y: 0.0, text: (*day).into(), font: small, color: t.color_soft.fade(0.8) });
    }
    col.push(header, 22.0);

    let start = first - Duration::days(first.weekday().num_days_from_sunday() as i64);
    let font = Font::new(t.text).tnum();
    let row_h = 30.0;
    let mut grid = Piece::new("cal.grid", col.w, row_h * 6.0);
    grid.pad = 16.0;
    for i in 0..42 {
        let day = start + Duration::days(i);
        let (cx, cy) = ((i % 7) as f32 * cell + cell / 2.0, (i / 7) as f32 * row_h + row_h / 2.0);
        let label = day.day().to_string();
        let (dw, dh) = col.ctx.text.measure(&label, font);
        let here = day.month() == first.month();
        let color = if day == today {
            // Today: a filled circle
            grid.prims.push(Prim::Rect { x: cx - 14.0, y: cy - 14.0, w: 28.0, h: 28.0, radius: 14.0, color: t.primary });
            t.on_primary
        } else if here {
            t.color_text
        } else {
            t.color_soft.alpha(0.3)
        };
        let font = if day == today { font.weight(600) } else { font };
        grid.prims.push(Prim::Text { x: cx - dw / 2.0, y: cy - dh / 2.0, text: label, font, color });
    }
    grid.actions.up = Some(Action::Month(col.ctx.state.month - 1));
    grid.actions.down = Some(Action::Month(col.ctx.state.month + 1));
    col.push(grid, row_h * 6.0 + 4.0);
}

fn media(col: &mut Col) {
    let t = col.theme();
    let model = col.ctx.model;
    let Some(player) = model.media.player() else {
        col.title("media.title", "Nothing playing", "");
        return;
    };
    let art = 76.0;
    let mut header = Piece::new("media.header", col.w, art);
    if let Some(image) = model.art.get(&player.art_url) {
        header.prims.push(Prim::Image { x: 0.0, y: 0.0, w: art, h: art, image: image.clone(), tint: None, radius: 12.0 });
    } else {
        if !player.art_url.is_empty() && !col.ctx.requested.contains(&player.art_url) {
            col.ctx.requests.push(Command::FetchArt(player.art_url.clone()));
        }
        header.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: art, h: art, radius: 12.0, color: t.container_highest });
        header.prims.push(Prim::Icon { x: art / 2.0 - 14.0, y: art / 2.0 - 14.0, size: 28.0, name: "music".into(), color: t.color_soft });
    }
    let x = art + 14.0;
    let room = col.w - x;
    let big = Font::new(t.text + 3.0).weight(500);
    let small = Font::new(t.text - 1.0);
    let title = crate::bar::Ctx::ellipsize_with(col.ctx.text, &player.title, big, room);
    let artist = crate::bar::Ctx::ellipsize_with(col.ctx.text, &player.artist, small, room);
    let album = crate::bar::Ctx::ellipsize_with(col.ctx.text, &player.album, small, room);
    let (_, bh) = col.ctx.text.measure("Ag", big);
    let (_, sh) = col.ctx.text.measure("Ag", small);
    let top = (art - bh - sh * 2.0) / 2.0;
    header.prims.push(Prim::Text { x, y: top, text: title, font: big, color: t.color_text });
    header.prims.push(Prim::Text { x, y: top + bh, text: artist, font: small, color: t.color_soft });
    header.prims.push(Prim::Text { x, y: top + bh + sh, text: album, font: small, color: t.color_soft.fade(0.7) });
    col.push(header, art + 10.0);

    if player.length > 0 {
        let position = player.position_now();
        let label = format!("{} / {}", clock_time(position), clock_time(player.length));
        col.slider("media.seek", None, position as f32, player.length as f32, Slider::Seek, &label);
    }

    // Back, play or pause, forward, in the middle
    let controls: [(&str, Option<Action>, f32); 3] = [
        ("skip-back", player.can_previous.then(|| Action::Send(Command::MediaPrevious(player.bus.clone()))), 22.0),
        (if player.playing { "pause" } else { "play" }, Some(Action::Send(Command::MediaPlayPause(player.bus.clone()))), 30.0),
        ("skip-forward", player.can_next.then(|| Action::Send(Command::MediaNext(player.bus.clone()))), 22.0),
    ];
    let row_h = 44.0;
    let total: f32 = controls.iter().map(|c| c.2).sum::<f32>() + 36.0 * 2.0;
    let mut x = (col.w - total) / 2.0;
    for (icon, action, size) in controls {
        let mut button = Piece::new(format!("media.{icon}"), size, size);
        let dim = if action.is_some() { 1.0 } else { 0.35 };
        let primary = size > 24.0;
        if primary {
            button.prims.push(Prim::Rect { x: -6.0, y: -6.0, w: size + 12.0, h: size + 12.0, radius: (size + 12.0) / 2.0, color: t.primary });
        }
        let inner = size * 0.75;
        button.prims.push(Prim::Icon { x: (size - inner) / 2.0, y: (size - inner) / 2.0, size: inner, name: icon.into(), color: if primary { t.on_primary } else { t.color_text.fade(dim) } });
        button.actions.left = action;
        button.lift = true;
        button.pad = 8.0;
        button.x = PAD + x;
        button.y = col.y + (row_h - size) / 2.0;
        col.pieces.push(button);
        x += size + 36.0;
    }
    col.gap(row_h + 4.0);

    if model.media.players.len() > 1 {
        col.separator();
        for (i, other) in model.media.players.iter().enumerate() {
            let mark = Mark::Radio(Some(i) == model.media.current);
            let value = if other.playing { "playing" } else { "" };
            col.row(&format!("media.player.{i}"), Some("music"), &other.identity, value, mark, true, send(Command::MediaChoose(other.bus.clone())));
        }
    }
}

fn sound(col: &mut Col) {
    let model = col.ctx.model;
    let audio = &model.audio;
    if !audio.available {
        col.title("sound.title", "Sound", "No sound server found");
        return;
    }
    let sink = audio.sinks.iter().find(|s| s.name == audio.sink).map(|s| s.description.clone()).unwrap_or_default();
    col.title("sound.title", "Sound", &sink);
    let icon = if audio.muted { "speaker-x" } else { "speaker-high" };
    let label = format!("{}%", (audio.volume * 100.0).round());
    col.slider("sound.volume", Some((icon, send(Command::ToggleMute))), audio.volume, 1.0, Slider::Volume, &label);
    let brightness = &model.brightness;
    if brightness.available {
        let label = format!("{}%", (brightness.value * 100.0).round());
        col.slider("sound.brightness", Some(("brightness", None)), brightness.value, 1.0, Slider::Brightness, &label);
    }
    if audio.has_mic {
        col.row("sound.mic", Some(if audio.mic_muted { "microphone-off" } else { "microphone" }), "Microphone", "", Mark::Switch(!audio.mic_muted), true, send(Command::ToggleMicMute));
    }
    if audio.sinks.len() > 1 {
        col.separator();
        for (i, device) in audio.sinks.iter().enumerate() {
            col.row(&format!("sound.sink.{i}"), None, &device.description, "", Mark::Radio(device.name == audio.sink), true, send(Command::SetDefaultSink(device.name.clone())));
        }
    }
    if !audio.streams.is_empty() {
        col.separator();
        for stream in &audio.streams {
            col.note(&stream.app);
            let label = format!("{}%", (stream.volume * 100.0).round());
            col.slider(&format!("sound.stream.{}", stream.index), None, stream.volume, 1.0, Slider::Stream(stream.index), &label);
        }
    }
}

fn network(col: &mut Col) {
    let n = &col.ctx.model.network;
    if !n.available {
        col.title("net.title", "Network", "NetworkManager isn't running");
        return;
    }
    if !col.ctx.requested.contains("scan") {
        col.ctx.requests.push(Command::ScanWifi);
    }
    let subtitle = match n.kind.as_str() {
        "" => "Offline".to_string(),
        _ if n.address.is_empty() => n.name.clone(),
        _ => format!("{} · {}", n.name, n.address),
    };
    col.title("net.title", "Network", &subtitle);
    col.row("net.wifi", Some("wifi-high"), "Wi-Fi", "", Mark::Switch(n.wifi_enabled), true, send(Command::SetWifi(!n.wifi_enabled)));
    if n.wifi_enabled {
        if n.access_points.is_empty() {
            col.note("Looking for networks…");
        }
        for (i, ap) in n.access_points.iter().enumerate() {
            let icon = if ap.strength > 66 { "wifi-high" } else if ap.strength > 33 { "wifi-medium" } else { "wifi-low" };
            let value = if ap.active {
                "connected"
            } else if ap.known {
                "saved"
            } else if ap.enterprise {
                "802.1X"
            } else if ap.secure {
                "secured"
            } else {
                ""
            };
            // Saved and open ones join at once; a new secured one asks for its password
            let action = if ap.active {
                None
            } else if ap.secure && !ap.known {
                Some(Action::WifiAsk(ap.ssid.clone()))
            } else {
                Some(Action::Both(Box::new(Action::Send(Command::ConnectWifi(ap.ssid.clone()))), Box::new(Action::CloseMenu)))
            };
            col.row(&format!("net.ap.{i}"), Some(icon), &ap.ssid, value, Mark::None, true, action);
        }
    }
    // The saved VPNs, each with its switch
    if !n.vpns.is_empty() {
        col.separator();
        col.label("VPN");
        for (i, vpn) in n.vpns.iter().enumerate() {
            let value = if vpn.connecting { "connecting…" } else { "" };
            col.row(&format!("net.vpn.{i}"), Some("lock"), &vpn.name, value, Mark::Switch(vpn.active || vpn.connecting), true, send(Command::SetVpn(vpn.path.clone(), !(vpn.active || vpn.connecting))));
        }
    }
    if !col.ctx.config.commands.network.is_empty() {
        col.separator();
        let command = col.ctx.config.commands.network.clone();
        col.row("net.settings", Some("gear-six"), "Network settings", "", Mark::None, true, Some(close_after(Command::Run(command))));
    }
}

/// How wide the desktop's menu is.
pub const DESKTOP_WIDTH: f32 = 252.0;

/// The desktop's own menu, from a right click on the wallpaper or on a widget (`widget`): what
/// that widget can do, then the desktop's things. Each closes the menu once done.
pub fn desktop(ctx: &mut Ctx<'_>, widget: Option<&str>) -> Built {
    if let Some(key) = widget.and_then(|w| w.strip_prefix("dock:")) {
        return dock(ctx, key);
    }
    let w = DESKTOP_WIDTH;
    let theme = ctx.theme;
    let mut col = Col { ctx, pieces: Vec::new(), y: PAD - 6.0, w: w - PAD * 2.0 };
    let then = |action: Action| Some(Action::Both(Box::new(action), Box::new(Action::CloseMenu)));
    if let Some(kind) = widget {
        col.label(crate::widgets::name(kind));
        match kind {
            "note" => col.row("desk.note", Some("lyrics"), "Write the note", "", Mark::None, true, then(Action::EditNote)),
            "photos" => col.row("desk.photo", Some("skip-forward"), "Next picture", "", Mark::None, true, then(Action::NextPhoto)),
            "pet" | "pet.friend" => {
                let slot = u8::from(kind == "pet.friend");
                let bed = col.ctx.config.pet.bed >= 0;
                col.row("desk.pet.feed", Some("coffee"), "Feed them", "", Mark::None, true, then(Action::PetFeed));
                col.row("desk.pet.play", Some("play"), "Play with a ball", "", Mark::None, true, then(Action::PetPlay));
                col.row("desk.pet.yarn", Some("play"), "Play with yarn", "", Mark::None, true, then(Action::PetYarn));
                col.row("desk.pet.bed", Some("sleep"), if bed { "Take the bed away" } else { "Give them a bed" }, "", Mark::None, true, then(Action::PetBed(!bed)));
                col.row("desk.pet.next", Some("refresh"), "Another animal", "", Mark::None, true, then(Action::PetNext(slot)));
                col.row("desk.pet.away", Some("close"), "Put it away", "", Mark::None, true, then(Action::PetAway(slot)));
            }
            _ => {}
        }
        if kind.starts_with("pet") {
        } else {
            col.row("desk.remove", Some("close"), "Take it off the desktop", "", Mark::None, true, then(Action::WidgetRemove(kind.to_string())));
        }
        col.separator();
        col.gap(4.0);
    }
    col.row("desk.arrange", Some("apps"), "Arrange widgets", "", Mark::None, true, then(Action::EditWidgets(Some(true))));
    col.row("desk.wallpaper", Some("image"), "Change wallpaper", "", Mark::None, true, then(Action::ToggleWallpapers));
    col.row("desk.random", Some("refresh"), "Another wallpaper", "", Mark::None, true, then(Action::RandomWallpaper));
    col.row("desk.settings", Some("gear-six"), "Settings", "", Mark::None, true, then(Action::ToggleSettings));
    let h = (col.y - 10.0 + PAD).round();
    let mut card = Piece::new("card", w, h);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: theme.radius });
    card.pad = 14.0;
    card.blocks = true;
    let mut pieces = vec![card];
    pieces.extend(col.pieces);
    Built { w, h, pieces }
}

/// An app's menu on the dock: its windows, another one, pinned or not, close. For the trash:
/// open it, or empty it (a second click to be sure; "@trash!" is the armed one).
fn dock(ctx: &mut Ctx<'_>, key: &str) -> Built {
    let w = DESKTOP_WIDTH;
    let theme = ctx.theme;
    let mut col = Col { ctx, pieces: Vec::new(), y: PAD - 6.0, w: w - PAD * 2.0 };
    let then = |action: Action| Some(Action::Both(Box::new(action), Box::new(Action::CloseMenu)));
    if let Some(armed) = key.strip_prefix(crate::dock::TRASH) {
        let count = col.ctx.model.trash;
        col.label("Trash");
        col.row("dock.trash.open", Some("trash"), "Open the trash", "", Mark::None, true, then(Action::DockTrash));
        let many = if count == 1 { "1 item".to_string() } else { format!("{count} items") };
        if count == 0 {
            col.row("dock.trash.empty", Some("check"), "Nothing in it", "", Mark::None, false, None);
        } else if armed == "!" {
            col.row("dock.trash.empty", Some("trash-full"), "Sure? Click again", &many, Mark::None, true, then(Action::DockTrashEmpty));
        } else {
            col.row("dock.trash.empty", Some("trash-full"), "Empty the trash", &many, Mark::None, true, Some(Action::DockTrashArm));
        }
    } else {
        let items = crate::dock::items(&col.ctx.config.dock.pinned, &col.ctx.model.compositor.workspaces, &col.ctx.model.apps);
        if let Some(item) = items.into_iter().find(|i| i.key == key) {
            col.label(&item.name);
            for (n, win) in item.windows.iter().take(8).enumerate() {
                let title = if win.title.is_empty() { item.name.clone() } else { win.title.clone() };
                col.row(&format!("dock.win.{n}"), None, &title, "", Mark::Radio(win.focused), true, then(Action::Send(Command::FocusWindow(win.id))));
            }
            if !item.windows.is_empty() {
                col.separator();
                col.gap(4.0);
            }
            let open = !item.windows.is_empty();
            col.row("dock.new", Some("add"), if open { "New window" } else { "Open" }, "", Mark::None, true, then(Action::DockLaunch(item.key.clone())));
            let (icon, label) = if item.pinned { ("close", "Unpin from the dock") } else { ("check", "Keep in the dock") };
            col.row("dock.pin", Some(icon), label, "", Mark::None, true, then(Action::DockPin(item.key.clone(), !item.pinned)));
            if open {
                let label = if item.windows.len() > 1 { format!("Close all {}", item.windows.len()) } else { "Close".into() };
                col.row("dock.close", Some("close"), &label, "", Mark::None, true, then(Action::DockClose(item.key.clone())));
            }
        }
    }
    let h = (col.y - 10.0 + PAD).round();
    let mut card = Piece::new("card", w, h);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: theme.radius });
    card.pad = 14.0;
    card.blocks = true;
    let mut pieces = vec![card];
    pieces.extend(col.pieces);
    Built { w, h, pieces }
}

fn close_after(command: Command) -> Action {
    Action::Both(Box::new(Action::Send(command)), Box::new(Action::CloseMenu))
}

fn bluetooth(col: &mut Col) {
    let b = &col.ctx.model.bluetooth;
    if !b.available {
        col.title("bt.title", "Bluetooth", "No adapter found");
        return;
    }
    let connected = b.connected().count();
    let subtitle = if !b.powered { "Off".to_string() } else if connected == 0 { "Nothing connected".into() } else { format!("{connected} connected") };
    col.title("bt.title", "Bluetooth", &subtitle);
    col.row("bt.power", Some("bluetooth"), "Bluetooth", "", Mark::Switch(b.powered), true, send(Command::SetBluetoothPower(!b.powered)));
    // Opening the menu looks around for a while
    if b.powered && !col.ctx.requested.contains("btscan") {
        col.ctx.requests.push(Command::ScanBluetooth);
    }
    if b.powered && !b.devices.is_empty() {
        col.separator();
        let armed = col.ctx.state.armed.clone();
        for (i, device) in b.devices.iter().enumerate() {
            let key = format!("bt.dev.{i}");
            let forgetting = armed.as_deref() == Some(key.as_str());
            let value = match (forgetting, device.connected, device.battery) {
                (true, ..) => "right-click again".to_string(),
                (_, true, level) if level >= 0 => format!("{level}%"),
                (_, true, _) => "connected".into(),
                _ => String::new(),
            };
            let label = if forgetting { format!("Forget {}?", device.name) } else { device.name.clone() };
            let icon = if device.connected { "bluetooth-connected" } else { "bluetooth" };
            col.row(&key, Some(icon), &label, &value, Mark::None, true, send(Command::BluetoothConnect(device.path.clone(), !device.connected)));
            // A right-click forgets it, after a second one
            if let Some(row) = col.pieces.iter_mut().rev().find(|p| p.key == key) {
                row.actions.right = Some(if forgetting { close_after(Command::BluetoothForget(device.path.clone())) } else { Action::Arm(key.clone()) });
            }
        }
    }
    if b.powered {
        col.separator();
        col.gap(4.0);
        col.label("Nearby");
        if b.nearby.is_empty() {
            col.note(if b.discovering { "Looking for devices…" } else { "Nothing new nearby" });
        }
        for (i, device) in b.nearby.iter().enumerate() {
            col.row(&format!("bt.near.{i}"), Some("bluetooth"), &device.name, "pair", Mark::None, true, Some(close_after(Command::BluetoothPair(device.path.clone()))));
        }
    }
    if !col.ctx.config.commands.bluetooth.is_empty() {
        col.separator();
        let command = col.ctx.config.commands.bluetooth.clone();
        col.row("bt.settings", Some("gear-six"), "Bluetooth settings", "", Mark::None, true, Some(close_after(Command::Run(command))));
    }
}

fn power(col: &mut Col) {
    let t = col.theme();
    let b = &col.ctx.model.battery;
    col.title("power.title", "Battery", &battery_line(b));
    // The level as a thick progress bar
    let mut level = Piece::new("power.level", col.w, 18.0);
    let fill = b.level.clamp(0.0, 1.0);
    let color = if b.on_battery && b.level < 0.15 { t.warn } else { t.primary };
    level.prims.push(Prim::Rect { x: 0.0, y: 4.0, w: col.w, h: 10.0, radius: 5.0, color: t.secondary_container });
    level.prims.push(Prim::Rect { x: 0.0, y: 4.0, w: (col.w * fill).max(10.0), h: 10.0, radius: 5.0, color });
    level.pad = 16.0;
    col.push(level, 26.0);
    if !b.profiles.is_empty() {
        col.separator();
        for (i, profile) in b.profiles.iter().enumerate() {
            let (icon, label) = match profile.as_str() {
                "power-saver" => ("sleep", "Power saver"),
                "balanced" => ("energy", "Balanced"),
                "performance" => ("speed", "Performance"),
                other => ("energy", other),
            };
            col.row(&format!("power.profile.{i}"), Some(icon), label, "", Mark::Radio(*profile == b.profile), true, send(Command::SetPowerProfile(profile.clone())));
        }
    }
}

fn session(col: &mut Col) {
    let commands = &col.ctx.config.commands;
    let armed = col.ctx.state.armed.clone();
    let entries: [(&str, &str, &str, Command, bool); 5] = [
        ("lock", "lock", "Lock", Command::Run(commands.lock.clone()), false),
        ("suspend", "sleep", "Sleep", Command::Run(commands.suspend.clone()), false),
        ("logout", "logout", "Log out", if commands.logout.is_empty() { Command::Quit } else { Command::Run(commands.logout.clone()) }, true),
        ("reboot", "restart", "Restart", Command::Run(commands.reboot.clone()), true),
        ("poweroff", "power", "Shut down", Command::Run(commands.poweroff.clone()), true),
    ];
    for (key, icon, label, command, drastic) in entries {
        // Drastic ones ask for a second click
        let is_armed = armed.as_deref() == Some(key);
        let action = if drastic && !is_armed { Action::Arm(key.into()) } else { close_after(command) };
        let label = if is_armed { format!("{label}? Click again") } else { label.to_string() };
        col.row(&format!("session.{key}"), Some(icon), &label, "", Mark::None, true, Some(action));
    }
}

fn machine(col: &mut Col) {
    let t = col.theme();
    let m = &col.ctx.model.machine;
    let subtitle = if m.temperature > 0.0 { format!("{} cores · {}°C", m.cores.len(), m.temperature.round()) } else { format!("{} cores", m.cores.len()) };
    col.title("machine.title", "Machine", &subtitle);
    // A bar per core, as tall as it works
    let per_row = 16usize;
    let rows = m.cores.len().div_ceil(per_row).max(1);
    let gap = col.w / per_row as f32;
    let row_h = 44.0;
    let mut stars = Piece::new("machine.cores", col.w, rows as f32 * row_h);
    for (i, load) in m.cores.iter().enumerate() {
        let (x, top) = ((i % per_row) as f32 * gap + gap * 0.2, (i / per_row) as f32 * row_h + 4.0);
        let (w, h) = (gap * 0.6, row_h - 10.0);
        stars.prims.push(Prim::Rect { x, y: top, w, h, radius: w / 2.0, color: t.secondary_container });
        let filled = (h * load.clamp(0.0, 1.0)).max(w);
        stars.prims.push(Prim::Rect { x, y: top + h - filled, w, h: filled, radius: w / 2.0, color: if *load > 0.85 { t.warn } else { t.primary } });
    }
    let h = stars.h;
    col.push(stars, h + 8.0);
    col.row("machine.cpu", Some("cpu"), "Processor", &format!("{}%", (m.cpu * 100.0).round()), Mark::None, true, None);
    let memory = format!("{:.1} / {:.1} GB", m.memory_used_kib as f64 / 1_048_576.0, m.memory_total_kib as f64 / 1_048_576.0);
    col.row("machine.memory", Some("ram"), "Memory", &memory, Mark::None, true, None);
    if !col.ctx.config.commands.monitor.is_empty() {
        col.separator();
        let command = col.ctx.config.commands.monitor.clone();
        col.row("machine.open", Some("apps"), "System monitor", "", Mark::None, true, Some(close_after(Command::Run(command))));
    }
}

fn traffic(col: &mut Col) {
    let m = &col.ctx.model.machine;
    col.title("traffic.title", "Traffic", "Every interface but the loopback");
    col.row("traffic.down", Some("down"), "Download", &rate(m.download), Mark::None, true, None);
    col.row("traffic.up", Some("up"), "Upload", &rate(m.upload), Mark::None, true, None);
    let size = |bytes: u64| if bytes >= 1 << 30 { format!("{:.2} GB", bytes as f64 / (1u64 << 30) as f64) } else { format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64) };
    col.separator();
    col.note("Since the bar started");
    col.row("traffic.total.down", Some("down"), "Received", &size(m.downloaded), Mark::None, true, None);
    col.row("traffic.total.up", Some("up"), "Sent", &size(m.uploaded), Mark::None, true, None);
}

fn notifications(col: &mut Col) {
    let n = &col.ctx.model.notifications;
    let subtitle = match n.list.len() {
        0 => "Nothing new".to_string(),
        1 => "One notification".into(),
        count => format!("{count} notifications"),
    };
    col.title("centre.title", "Notifications", &subtitle);
    col.row("centre.quiet", Some(if n.quiet { "bell-slash" } else { "bell" }), "Do not disturb", "", Mark::Switch(n.quiet), true, send(Command::NotificationsQuiet));
    if n.list.is_empty() {
        return;
    }
    col.row("centre.clear", Some("close"), "Clear all", "", Mark::None, true, send(Command::NotificationsClear));
    col.separator();
    let list = n.list.clone();
    for notification in &list {
        let (pieces, h) = crate::cards::entry(col.ctx.theme, col.ctx.text, notification, PAD, col.y, col.w);
        col.pieces.extend(pieces);
        col.y += h;
    }
}

fn timer(col: &mut Col) {
    use crate::model::TimerAction as T;
    use crate::timers::{Phase, clock_text, length_text};
    let t = col.theme();
    let timers = &col.ctx.model.timers;
    let now = std::time::Instant::now();
    let act = |a: T| Some(Action::Timer(a));

    col.label("Countdown");
    match timers.countdown {
        Some(c) => {
            let left = c.clock.left(now);
            let about = if c.clock.running() { format!("of {}", length_text(c.total)) } else { "paused".to_string() };
            col.big_time("timer.countdown", &clock_text(left), &about, Some(1.0 - left.as_secs_f32() / c.total.as_secs_f32().max(1.0)), t.primary);
            col.buttons(
                "timer.countdown.do",
                vec![
                    ("+1 min".into(), false, act(T::CountdownAdd(60))),
                    (if c.clock.running() { "Pause" } else { "Go on" }.into(), true, act(T::CountdownPause)),
                    ("Cancel".into(), false, act(T::CountdownCancel)),
                ],
            );
        }
        None => {
            let chip = |m: u64| (length_text(crate::timers::mins(m)), false, act(T::Countdown(m * 60)));
            col.buttons("timer.presets.a", vec![chip(1), chip(5), chip(10), chip(15)]);
            col.buttons("timer.presets.b", vec![chip(25), chip(45), chip(60), chip(120)]);
        }
    }
    col.separator();
    col.gap(4.0);

    col.label("Focus");
    let s = timers.settings;
    match timers.focus {
        Some(f) => {
            let left = f.clock.left(now);
            let about = match (f.phase, f.clock.running()) {
                (_, false) => "paused".to_string(),
                (Phase::Work, _) => format!("focus · round {} of {}", (f.round - 1) % s.rounds.max(1) + 1, s.rounds),
                (Phase::ShortBreak, _) => "short break".into(),
                (Phase::LongBreak, _) => "long break".into(),
            };
            let color = if f.phase == Phase::Work { t.primary } else { t.palette.tertiary };
            col.big_time("timer.focus", &clock_text(left), &about, Some(1.0 - left.as_secs_f32() / f.total.as_secs_f32().max(1.0)), color);
            let skip = if f.phase == Phase::Work { "Break now" } else { "Skip break" };
            col.buttons(
                "timer.focus.do",
                vec![(if f.clock.running() { "Pause" } else { "Go on" }.into(), true, act(T::FocusPause)), (skip.into(), false, act(T::FocusSkip)), ("Stop".into(), false, act(T::Focus))],
            );
        }
        None => {
            col.note(&format!("{} focus · {} break · {} every {}th", length_text(s.work), length_text(s.short_break), length_text(s.long_break), s.rounds));
            col.gap(4.0);
            col.buttons("timer.focus.start", vec![(format!("Start {}", length_text(s.work)), true, act(T::Focus))]);
        }
    }
    col.separator();
    col.gap(4.0);

    col.label("Stopwatch");
    let watch = &timers.stopwatch;
    let elapsed = watch.elapsed(now);
    let running = watch.since.is_some();
    let about = if running { "running" } else if elapsed.is_zero() { "" } else { "stopped" };
    col.big_time("timer.watch", &clock_text(elapsed), about, None, t.primary);
    col.gap(6.0);
    let main = if running { "Stop" } else if elapsed.is_zero() { "Start" } else { "Go on" };
    col.buttons(
        "timer.watch.do",
        vec![(main.into(), true, act(T::Stopwatch)), ("Lap".into(), false, if running { act(T::StopwatchLap) } else { None }), ("Reset".into(), false, act(T::StopwatchReset))],
    );
    // The newest laps, each with its own length
    let laps = &watch.laps;
    for (n, at) in laps.iter().enumerate().rev().take(5) {
        let before = if n > 0 { laps[n - 1] } else { std::time::Duration::ZERO };
        col.row(&format!("timer.lap.{n}"), None, &format!("Lap {}", n + 1), &format!("{}   +{}", clock_text(*at), clock_text(*at - before)), Mark::None, true, None);
    }
}

fn weather(col: &mut Col) {
    use crate::weather::{describe, icon};
    let t = col.theme();
    let w = &col.ctx.model.weather;
    if !w.available {
        col.title("weather.title", "Weather", "Not here yet: no connection, or the place wasn't found");
        col.buttons("weather.refresh", vec![("Try again".into(), true, Some(Action::Send(Command::WeatherRefresh)))]);
        return;
    }
    col.title("weather.title", if w.place.is_empty() { "Weather" } else { &w.place }, &format!("{}, feels like {:.0}°", describe(w.code), w.feels_like));
    // Now, big
    let font = Font::new(t.text + 23.0).weight(500).tnum();
    let now = format!("{:.0}{}", w.temperature, w.temperature_unit);
    let (tw, th) = col.ctx.text.measure(&now, font);
    let mut big = Piece::new("weather.now", col.w, th);
    big.prims.push(Prim::Icon { x: 0.0, y: (th - 40.0) / 2.0, size: 40.0, name: icon(w.code, w.is_day).into(), color: t.primary });
    big.prims.push(Prim::Text { x: 52.0, y: 0.0, text: now, font, color: t.color_text });
    let _ = tw;
    col.push(big, th + 8.0);
    col.row("weather.humidity", Some("drizzle"), "Humidity", &format!("{:.0}%", w.humidity), Mark::None, true, None);
    col.row("weather.wind", Some("speed"), "Wind", &format!("{:.0} {}", w.wind, w.wind_unit), Mark::None, true, None);
    if !w.sunrise.is_empty() {
        col.row("weather.sun", Some("sun"), "Sunrise · sunset", &format!("{} · {}", w.sunrise, w.sunset), Mark::None, true, None);
    }
    if !w.days.is_empty() {
        col.separator();
        col.gap(4.0);
        col.label("Next days");
        let today = col.ctx.now.date_naive();
        for day in &w.days {
            let name = match (day.date - today).num_days() {
                0 => "Today".to_string(),
                1 => "Tomorrow".to_string(),
                _ => day.date.format("%A").to_string(),
            };
            let rain = if day.rain >= 20 { format!("   {}% rain", day.rain) } else { String::new() };
            col.row(&format!("weather.day.{}", day.date), Some(icon(day.code, true)), &name, &format!("{:.0}° / {:.0}°{rain}", day.max, day.min), Mark::None, true, None);
        }
    }
    col.gap(6.0);
    col.buttons("weather.refresh", vec![("Refresh".into(), false, Some(Action::Send(Command::WeatherRefresh)))]);
}

fn capture(col: &mut Col) {
    use crate::model::Capture;
    col.label("Screenshot");
    col.row("capture.region", Some("capture"), "A region you choose", "", Mark::None, true, Some(close_after(Command::Screenshot(Capture::Region))));
    col.row("capture.screen", Some("image"), "The whole screen", "", Mark::None, true, Some(close_after(Command::Screenshot(Capture::Screen))));
    if crate::services::detect_compositor() == crate::model::CompositorKind::Niri {
        col.row("capture.window", Some("layout"), "The window in focus", "", Mark::None, true, Some(close_after(Command::Screenshot(Capture::Window))));
    }
    // Text off the screen: what's in a region, onto the clipboard
    if crate::services::capture::TEXT_TOOLS.iter().all(|t| crate::config::command_available(t)) {
        col.row("capture.text", Some("clipboard"), "Copy text from a region", "", Mark::None, true, Some(close_after(Command::ScreenText)));
    } else {
        col.note("Copying text off the screen needs grim, slurp and tesseract.");
    }
    col.separator();
    col.gap(4.0);
    col.label("Record");
    if col.ctx.model.recording.is_some() {
        col.row("capture.stop", Some("close"), "Stop recording", "", Mark::None, true, Some(close_after(Command::RecordStop)));
    } else if crate::config::command_available("wf-recorder") {
        col.row("capture.record", Some("play"), "Record the screen", "", Mark::None, true, Some(close_after(Command::RecordStart { audio: false })));
        col.row("capture.record.audio", Some("speaker-high"), "Record with sound", "", Mark::None, true, Some(close_after(Command::RecordStart { audio: true })));
    } else {
        col.note("Recording needs wf-recorder: install it, and it shows up here.");
    }
}

fn keyboard(col: &mut Col) {
    let c = &col.ctx.model.compositor;
    col.title("kb.title", "Keyboard", "");
    for (i, code) in c.layouts.iter().enumerate() {
        let name = c.layout_names.get(i).cloned().unwrap_or_else(|| code.to_uppercase());
        col.row(&format!("kb.{i}"), Some("keyboard"), &name, &code.to_uppercase(), Mark::Radio(i == c.layout), true, send(Command::SetLayout(i)));
    }
}

fn tray(col: &mut Col, id: &str) {
    let model = col.ctx.model;
    let title = model.tray.iter().find(|i| i.id == id).map(|i| i.title.clone()).unwrap_or_default();
    let Some(entries) = model.menus.get(id) else {
        col.title("tray.title", &title, "");
        col.note("…");
        return;
    };
    // Walk down to the submenu being shown
    let mut level: &[MenuEntry] = entries;
    let path = col.ctx.state.tray_path.clone();
    let mut depth = 0;
    for step in &path {
        match level.iter().find(|e| e.id == *step) {
            Some(entry) if !entry.children.is_empty() => {
                level = &entry.children;
                depth += 1;
            }
            _ => break,
        }
    }
    if depth > 0 {
        let parent = path[..depth - 1].to_vec();
        col.row("tray.back", Some("caret-left"), "Back", "", Mark::None, true, Some(Action::Submenu(parent)));
        col.separator();
    } else if level.is_empty() {
        col.title("tray.title", &title, "");
        col.note("This app's menu is empty");
        return;
    }
    let mut last_separator = true;
    for entry in level {
        if entry.separator {
            if !last_separator {
                col.separator();
                last_separator = true;
            }
            continue;
        }
        if entry.label.is_empty() {
            continue;
        }
        last_separator = false;
        let key = format!("tray.entry.{}", entry.id);
        if !entry.children.is_empty() {
            let mut deeper = path[..depth].to_vec();
            deeper.push(entry.id);
            col.row(&key, None, &entry.label, "›", Mark::None, entry.enabled, Some(Action::Submenu(deeper)));
            continue;
        }
        let mark = match entry.toggle.as_str() {
            "checkmark" => Mark::Check(entry.checked),
            "radio" => Mark::Radio(entry.checked),
            _ => Mark::None,
        };
        col.row(&key, None, &entry.label, "", mark, entry.enabled, Some(close_after(Command::TrayMenuClick(id.to_string(), entry.id))));
    }
}

// ---------------------------------------------------------------- the control centre

pub const CONTROL_WIDTH: f32 = 400.0;
const TILE_H: f32 = 58.0;

/// A quick tile: an icon in a little sky, its name and its state, lit when on.
struct Tile {
    key: &'static str,
    icon: &'static str,
    label: &'static str,
    detail: String,
    on: bool,
    action: Option<Action>,
}

/// A rounded box with a hairline edge, filled with `fill` inside.
fn outlined(piece: &mut Piece, t: &Theme, w: f32, h: f32, radius: f32, fill: Rgba) {
    piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius, color: t.hairline() });
    piece.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0, radius: radius - 1.0, color: fill });
}

fn tiles(col: &mut Col, tiles: Vec<Tile>) {
    let t = col.theme();
    let gap = 10.0;
    let w = ((col.w - gap) / 2.0).floor();
    let font = Font::new(t.text).weight(500);
    let small = Font::new(t.text - 2.0);
    let (_, lh) = col.ctx.text.measure("Ag", font);
    let (_, sh) = col.ctx.text.measure("Ag", small);
    for (i, tile) in tiles.into_iter().enumerate() {
        let glide = col.ctx.anim.value(&format!("tile.{}", tile.key), if tile.on { 1.0 } else { 0.0 }, t.normal, Ease::OutCubic);
        let mut piece = Piece::new(format!("tile.{}", tile.key), w, TILE_H);
        // Off: just a hairline outline. On: filled, the icon in an accent disc
        let ink = t.color_text.mix(t.on_primary_container, glide);
        outlined(&mut piece, t, w, TILE_H, 14.0, t.card_deep.mix(t.primary_container, glide));
        if glide > 0.01 {
            piece.prims.push(Prim::Rect { x: 10.0, y: 11.0, w: 36.0, h: 36.0, radius: 18.0, color: t.primary.alpha(glide) });
        }
        piece.prims.push(Prim::Icon { x: 18.0, y: 19.0, size: 20.0, name: tile.icon.into(), color: t.color_soft.mix(t.on_primary, glide) });
        let room = w - 58.0 - 8.0;
        let top = (TILE_H - lh - sh) / 2.0;
        piece.prims.push(Prim::Text { x: 56.0, y: top, text: crate::bar::Ctx::ellipsize_with(col.ctx.text, tile.label, font, room), font, color: ink });
        let detail = crate::bar::Ctx::ellipsize_with(col.ctx.text, &tile.detail, small, room);
        piece.prims.push(Prim::Text { x: 56.0, y: top + lh - 2.0, text: detail, font: small, color: ink.fade(0.75) });
        piece.pad = 4.0;
        piece.highlight = true;
        piece.actions.left = tile.action;
        piece.x = PAD + (i % 2) as f32 * (w + gap);
        piece.y = col.y + (i / 2) as f32 * (TILE_H + gap);
        col.pieces.push(piece);
    }
    let count = col.pieces.iter().filter(|p| p.key.starts_with("tile.")).count();
    let rows = count.div_ceil(2) as f32;
    col.gap(rows * (TILE_H + gap) + 4.0);
}

/// What's playing, in one strip: cover, title and artist, and the three buttons.
fn now_playing(col: &mut Col) {
    let t = col.theme();
    let model = col.ctx.model;
    let Some(player) = model.media.player() else { return };
    let h = 56.0;
    let mut strip = Piece::new("control.media", col.w, h);
    outlined(&mut strip, t, col.w, h, 14.0, t.card_deep);
    match model.art.get(&player.art_url) {
        Some(image) => strip.prims.push(Prim::Image { x: 8.0, y: 8.0, w: 40.0, h: 40.0, image: image.clone(), tint: None, radius: 10.0 }),
        None => {
            if !player.art_url.is_empty() && !col.ctx.requested.contains(&player.art_url) {
                col.ctx.requests.push(Command::FetchArt(player.art_url.clone()));
            }
            strip.prims.push(Prim::Icon { x: 16.0, y: 16.0, size: 24.0, name: "music".into(), color: t.color_soft });
        }
    }
    let font = Font::new(t.text).weight(500);
    let small = Font::new(t.text - 2.0);
    let room = col.w - 56.0 - 110.0;
    let (_, lh) = col.ctx.text.measure("Ag", font);
    let (_, sh) = col.ctx.text.measure("Ag", small);
    let top = (h - lh - sh) / 2.0;
    let title = crate::bar::Ctx::ellipsize_with(col.ctx.text, &player.title, font, room);
    let artist = crate::bar::Ctx::ellipsize_with(col.ctx.text, &player.artist, small, room);
    strip.prims.push(Prim::Text { x: 58.0, y: top, text: title, font, color: t.color_text });
    strip.prims.push(Prim::Text { x: 58.0, y: top + lh - 2.0, text: artist, font: small, color: t.color_soft });
    strip.x = PAD;
    strip.y = col.y;
    strip.actions.right = send(Command::MediaPlayPause(player.bus.clone()));
    col.pieces.push(strip);
    let buttons: [(&str, Option<Action>); 3] = [
        ("skip-back", player.can_previous.then(|| Action::Send(Command::MediaPrevious(player.bus.clone())))),
        (if player.playing { "pause" } else { "play" }, Some(Action::Send(Command::MediaPlayPause(player.bus.clone())))),
        ("skip-forward", player.can_next.then(|| Action::Send(Command::MediaNext(player.bus.clone())))),
    ];
    for (i, (icon, action)) in buttons.into_iter().enumerate() {
        let mut button = Piece::new(format!("control.media.{icon}"), 28.0, 28.0);
        let dim = if action.is_some() { 1.0 } else { 0.35 };
        button.prims.push(Prim::Icon { x: 4.0, y: 4.0, size: 20.0, name: icon.into(), color: t.color_text.fade(dim) });
        button.actions.left = action;
        button.lift = true;
        button.pad = 6.0;
        button.x = PAD + col.w - 104.0 + i as f32 * 34.0;
        button.y = col.y + (h - 28.0) / 2.0;
        col.pieces.push(button);
    }
    col.gap(h + 10.0);
}

/// The control centre: the time, quick tiles, levels, music, notifications and the session,
/// as tall as `room` allows (older notifications give way first).
/// How wide the day panel is.
pub const DAY_WIDTH: f32 = 380.0;

/// The day panel, on the left: the day, the month, timers, the to-do list and notes. `room` is
/// the height it may take; the notes get what's left of it.
pub fn day(ctx: &mut Ctx<'_>, room: f32) -> Built {
    use crate::model::TimerAction as T;
    use crate::timers::clock_text;
    let w = DAY_WIDTH;
    let mut col = Col { ctx, pieces: Vec::new(), y: PAD, w: w - PAD * 2.0 };
    let t = col.theme();

    // The day, big, and the date with its week
    let now = col.ctx.now;
    let big = Font::new(t.text + 14.0).weight(500);
    let small = Font::new(t.text - 1.0);
    let name = now.format("%A").to_string();
    let date = now.format("%-d %B %Y · week %V").to_string();
    let (_, bh) = col.ctx.text.measure(&name, big);
    let (_, sh) = col.ctx.text.measure(&date, small);
    let mut header = Piece::new("day.today", col.w, bh + sh);
    header.prims.push(Prim::Text { x: 0.0, y: 0.0, text: name, font: big, color: t.color_text });
    header.prims.push(Prim::Text { x: 1.0, y: bh - 4.0, text: date, font: small, color: t.color_soft });
    col.push(header, bh + sh + 10.0);
    col.separator();
    col.gap(6.0);

    // The month
    calendar(&mut col);
    col.separator();
    col.gap(4.0);

    // Timers: what's running, or a start
    col.label("Timers");
    let timers = col.ctx.model.timers.clone();
    let instant = std::time::Instant::now();
    let act = |a: T| Some(Action::Timer(a));
    let mut running = false;
    if let Some(f) = timers.focus {
        running = true;
        let left = f.clock.left(instant);
        let about = if !f.clock.running() { "focus, paused".to_string() } else if f.phase == crate::timers::Phase::Work { "focus".into() } else { "break".into() };
        col.big_time("day.focus", &clock_text(left), &about, Some(1.0 - left.as_secs_f32() / f.total.as_secs_f32().max(1.0)), t.primary);
        col.buttons("day.focus.do", vec![(if f.clock.running() { "Pause" } else { "Go on" }.into(), true, act(T::FocusPause)), ("Stop".into(), false, act(T::Focus))]);
    }
    if let Some(c) = timers.countdown {
        running = true;
        let left = c.clock.left(instant);
        col.big_time("day.countdown", &clock_text(left), if c.clock.running() { "countdown" } else { "countdown, paused" }, Some(1.0 - left.as_secs_f32() / c.total.as_secs_f32().max(1.0)), t.palette.tertiary);
        col.buttons("day.countdown.do", vec![(if c.clock.running() { "Pause" } else { "Go on" }.into(), true, act(T::CountdownPause)), ("Cancel".into(), false, act(T::CountdownCancel))]);
    }
    if !running {
        col.buttons("day.start", vec![("Focus".into(), true, act(T::Focus)), ("5 min".into(), false, act(T::Countdown(300))), ("10 min".into(), false, act(T::Countdown(600))), ("25 min".into(), false, act(T::Countdown(1500)))]);
    }
    col.separator();
    col.gap(4.0);

    // To do: tasks with their boxes, a field to add one
    let tasks = col.ctx.model.day.tasks.clone();
    let label_y = col.y;
    col.label("To do");
    if tasks.iter().any(|task| task.done) {
        let font = Font::new(t.text - 2.0).weight(500);
        let (cw, ch) = col.ctx.text.measure("Clear done", font);
        let mut clear = Piece::new("day.clear", cw, ch);
        clear.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Clear done".into(), font, color: t.color_soft });
        clear.actions.left = Some(Action::DayClearDone);
        clear.lift = true;
        clear.x = PAD + col.w - cw;
        clear.y = label_y - 1.0;
        col.pieces.push(clear);
    }
    let shown = 8;
    let font = Font::new(t.text);
    let (_, fh) = col.ctx.text.measure("Ag", font);
    for (i, task) in tasks.iter().enumerate().take(shown) {
        let h = 30.0;
        let mut row = Piece::new(format!("day.task.{i}"), col.w - 28.0, h);
        let (bx, by, b) = (4.0, (h - 18.0) / 2.0, 18.0);
        if task.done {
            row.prims.push(Prim::Rect { x: bx, y: by, w: b, h: b, radius: 6.0, color: t.primary });
            row.prims.push(Prim::Icon { x: bx + 2.0, y: by + 2.0, size: 14.0, name: "check".into(), color: t.on_primary });
        } else {
            row.prims.push(Prim::Rect { x: bx, y: by, w: b, h: b, radius: 6.0, color: t.outline });
            row.prims.push(Prim::Rect { x: bx + 1.5, y: by + 1.5, w: b - 3.0, h: b - 3.0, radius: 4.5, color: t.card_deep });
        }
        let text = crate::bar::Ctx::ellipsize_with(col.ctx.text, &task.text, font, col.w - 28.0 - 36.0);
        row.prims.push(Prim::Text { x: 32.0, y: ((h - fh) / 2.0).round(), text, font, color: if task.done { t.color_soft.alpha(0.7) } else { t.color_text } });
        row.highlight = true;
        row.actions.left = Some(Action::DayToggle(i));
        row.x = PAD;
        row.y = col.y;
        col.pieces.push(row);
        let mut remove = Piece::new(format!("day.task.{i}.x"), 24.0, h);
        remove.prims.push(Prim::Icon { x: 4.0, y: (h - 16.0) / 2.0, size: 16.0, name: "close".into(), color: t.color_soft.alpha(0.8) });
        remove.label = Some("Remove".into());
        remove.actions.left = Some(Action::DayRemove(i));
        remove.lift = true;
        remove.x = PAD + col.w - 24.0;
        remove.y = col.y;
        col.pieces.push(remove);
        col.gap(h + 2.0);
    }
    if tasks.len() > shown {
        col.note(&format!("{} more in todo.md", tasks.len() - shown));
    }
    let field = col.ctx.state.day_field;
    let typed = col.ctx.state.day_typed.clone();
    let (fw, fh2) = (col.w, 34.0);
    let mut add = Piece::new("day.add", fw, fh2);
    let typing = field == 1;
    add.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: fh2, radius: fh2 / 2.0, color: if typing { t.primary } else { t.hairline() } });
    add.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: fh2 - 2.0, radius: fh2 / 2.0 - 1.0, color: t.card_deep });
    add.prims.push(Prim::Icon { x: 12.0, y: (fh2 - 16.0) / 2.0, size: 16.0, name: "add".into(), color: t.primary });
    let words = if typed.is_empty() && !typing { "Add a task".to_string() } else { crate::bar::Ctx::ellipsize_with(col.ctx.text, &typed, font, fw - 60.0) };
    let (ww, _) = col.ctx.text.measure(&words, font);
    add.prims.push(Prim::Text { x: 36.0, y: ((fh2 - fh) / 2.0).round(), text: words, font, color: if typed.is_empty() { t.color_soft } else { t.color_text } });
    if typing {
        let at = if typed.is_empty() { 36.0 } else { 37.0 + ww };
        add.prims.push(Prim::Rect { x: at, y: 9.0, w: 2.0, h: fh2 - 18.0, radius: 1.0, color: t.primary });
    }
    add.highlight = true;
    add.actions.left = Some(Action::DayFocus(1));
    col.push(add, fh2 + 10.0);
    col.separator();
    col.gap(4.0);

    // Notes: as many lines as there's room for, the end of them when they run longer
    col.label("Notes");
    let notes = col.ctx.model.day.notes.clone();
    let note_font = Font::new(t.text - 1.0);
    let (_, lh) = col.ctx.text.measure("Ag", note_font);
    let lines_room = (((room - col.y - PAD - 24.0) / lh).floor() as usize).clamp(3, 14);
    let mut lines: Vec<String> = Vec::new();
    for paragraph in notes.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(col.ctx.text.wrap(paragraph, note_font, col.w - 28.0, 40));
        }
    }
    let writing = field == 2;
    let empty = notes.trim().is_empty();
    if empty {
        lines = vec![if writing { String::new() } else { "Click to write…".into() }];
    }
    if lines.len() > lines_room {
        lines.drain(..lines.len() - lines_room);
    }
    let ah = lines.len().max(3) as f32 * lh + 20.0;
    let mut area = Piece::new("day.notes", col.w, ah);
    // Edged in the accent while it has the keyboard
    area.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: col.w, h: ah, radius: 12.0, color: if writing { t.primary } else { t.hairline() } });
    area.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: col.w - 2.0, h: ah - 2.0, radius: 11.0, color: t.card_deep });
    let last = lines.len().saturating_sub(1);
    for (i, line) in lines.iter().enumerate() {
        area.prims.push(Prim::Text { x: 14.0, y: 10.0 + i as f32 * lh, text: line.clone(), font: note_font, color: if empty { t.color_soft } else { t.color_text } });
        if writing && i == last {
            let (lw, _) = col.ctx.text.measure(line, note_font);
            area.prims.push(Prim::Rect { x: 15.0 + lw, y: 12.0 + i as f32 * lh, w: 2.0, h: lh - 4.0, radius: 1.0, color: t.primary });
        }
    }
    area.actions.left = Some(Action::DayFocus(2));
    area.blocks = true;
    col.push(area, ah + 8.0);
    let folder = crate::day::folder(&col.ctx.config.day.folder);
    let home = crate::config::home();
    let shown_folder = folder.strip_prefix(&home).map(|p| format!("~/{}", p.display())).unwrap_or_else(|_| folder.display().to_string());
    col.note(&format!("Kept in {shown_folder}"));

    let h = (col.y + PAD - 6.0).round();
    let mut pieces = Vec::with_capacity(col.pieces.len() + 1);
    // Under the clock in the middle; with the frame, hung from the bar like a menu
    pieces.push(card(t, w, h, if t.on_top { 1 } else { 4 }));
    pieces.extend(col.pieces);
    Built { w, h, pieces }
}

pub fn control(ctx: &mut Ctx<'_>, room: f32) -> Built {
    let w = CONTROL_WIDTH;
    let mut col = Col { ctx, pieces: Vec::new(), y: PAD, w: w - PAD * 2.0 };
    let t = col.theme();
    let model = col.ctx.model;

    // The time, big, and the day
    let now = col.ctx.now;
    let big = Font::new(t.text + 16.0).weight(500).tnum();
    let small = Font::new(t.text - 1.0);
    let time = now.format(if col.ctx.config.clock.hours24 { "%H:%M" } else { "%-I:%M %p" }).to_string();
    let (_, bh) = col.ctx.text.measure(&time, big);
    let (_, sh) = col.ctx.text.measure("Ag", small);
    let mut header = Piece::new("control.time", col.w, bh + sh);
    header.prims.push(Prim::Text { x: 0.0, y: 0.0, text: time, font: big, color: t.color_text });
    header.prims.push(Prim::Text { x: 2.0, y: bh - 4.0, text: now.format("%A, %B %-d").to_string(), font: small, color: t.color_soft });
    if model.battery.present {
        let line = battery_line(&model.battery);
        let (lw, _) = col.ctx.text.measure(&line, small);
        header.prims.push(Prim::Text { x: col.w - lw, y: 8.0, text: line, font: small, color: t.color_soft });
    }
    header.glow = true;
    let header_h = bh + sh;
    col.push(header, header_h + 12.0);

    // Quick tiles
    let mut list = Vec::new();
    let n = &model.network;
    if n.available {
        let detail = if !n.wifi_enabled { "Off".to_string() } else if n.kind == "wifi" { n.name.clone() } else if n.kind.is_empty() { "Not connected".into() } else { n.name.clone() };
        list.push(Tile { key: "wifi", icon: "wifi-high", label: "Wi-Fi", detail, on: n.wifi_enabled, action: send(Command::SetWifi(!n.wifi_enabled)) });
        // A VPN tile when there's one saved: the one up goes down, else the first comes up
        if let Some(first) = n.vpns.first() {
            let up = n.vpns.iter().find(|v| v.active || v.connecting);
            let detail = match up {
                Some(v) if v.connecting => format!("{}, connecting…", v.name),
                Some(v) => v.name.clone(),
                None if n.vpns.len() == 1 => format!("Off · {}", first.name),
                None => "Off".into(),
            };
            let action = match up {
                Some(v) => send(Command::SetVpn(v.path.clone(), false)),
                None => send(Command::SetVpn(first.path.clone(), true)),
            };
            list.push(Tile { key: "vpn", icon: "lock", label: "VPN", detail, on: up.is_some(), action });
        }
    }
    let b = &model.bluetooth;
    if b.available {
        let connected: Vec<_> = b.connected().map(|d| d.name.clone()).collect();
        let detail = if !b.powered { "Off".to_string() } else if connected.is_empty() { "On".into() } else { connected.join(", ") };
        list.push(Tile { key: "bluetooth", icon: "bluetooth", label: "Bluetooth", detail, on: b.powered, action: send(Command::SetBluetoothPower(!b.powered)) });
    }
    let quiet = model.notifications.quiet;
    list.push(Tile { key: "dnd", icon: if quiet { "bell-slash" } else { "bell" }, label: "Do not disturb", detail: if quiet { "On".into() } else { "Off".into() }, on: quiet, action: send(Command::NotificationsQuiet) });
    list.push(Tile { key: "awake", icon: "coffee", label: "Stay awake", detail: if model.awake { "On".into() } else { "Off".into() }, on: model.awake, action: send(Command::ToggleAwake) });
    let audio = &model.audio;
    if audio.has_mic {
        list.push(Tile {
            key: "mic",
            icon: if audio.mic_muted { "microphone-off" } else { "microphone" },
            label: "Microphone",
            detail: if audio.mic_muted { "Muted".into() } else { "On".into() },
            on: !audio.mic_muted,
            action: send(Command::ToggleMicMute),
        });
    }
    let power = &model.battery;
    if !power.profiles.is_empty() {
        let at = power.profiles.iter().position(|p| *p == power.profile).unwrap_or(0);
        let next = power.profiles[(at + 1) % power.profiles.len()].clone();
        let name = match power.profile.as_str() {
            "power-saver" => "Power saver",
            "performance" => "Performance",
            _ => "Balanced",
        };
        list.push(Tile { key: "profile", icon: "energy", label: "Power", detail: name.into(), on: power.profile != "balanced", action: send(Command::SetPowerProfile(next)) });
    }
    let night = &model.night;
    if night.available {
        list.push(Tile { key: "night", icon: "nightlight", label: "Night light", detail: night.detail.clone(), on: night.on, action: Some(Action::ToggleNightLight) });
    }
    // The wallpaper changer, named after the picture up now
    let image = col.ctx.config.wallpaper.image.trim();
    let detail = if image.is_empty() { "Choose a picture".to_string() } else { crate::wallpapers::name(std::path::Path::new(image)) };
    list.push(Tile { key: "wallpaper", icon: "image", label: "Wallpaper", detail, on: false, action: Some(Action::ToggleWallpapers) });
    list.push(Tile { key: "settings", icon: "gear-six", label: "Settings", detail: "Look, lock, widgets…".into(), on: false, action: Some(Action::ToggleSettings) });
    tiles(&mut col, list);

    // Levels
    if audio.available {
        let icon = if audio.muted { "speaker-x" } else { "speaker-high" };
        let label = format!("{}%", (audio.volume * 100.0).round());
        col.slider("control.volume", Some((icon, send(Command::ToggleMute))), audio.volume, 1.0, Slider::Volume, &label);
    }
    if model.brightness.available {
        let label = format!("{}%", (model.brightness.value * 100.0).round());
        col.slider("control.brightness", Some(("brightness", None)), model.brightness.value, 1.0, Slider::Brightness, &label);
    }
    col.gap(8.0);
    now_playing(&mut col);

    // The session, at the bottom: measured now, placed once the rest is known
    let session_h = 44.0;
    let armed = col.ctx.state.armed.clone();
    let commands = col.ctx.config.commands.clone();

    // Notifications, grouped by app, as many as fit
    let notifications = &model.notifications;
    if notifications.own && !notifications.list.is_empty() {
        col.separator();
        let font = Font::new(t.text + 1.0).weight(500);
        let (_, th) = col.ctx.text.measure("Ag", font);
        let mut head = Piece::new("control.notifications", col.w, th);
        head.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Notifications".into(), font, color: t.color_text });
        col.push(head, th + 4.0);
        let mut clear = Piece::new("control.clear", 80.0, th);
        let (cw, _) = col.ctx.text.measure("Clear all", small);
        clear.prims.push(Prim::Text { x: 80.0 - cw, y: 3.0, text: "Clear all".into(), font: small, color: t.color_soft });
        clear.x = PAD + col.w - 80.0;
        clear.y = col.y - th - 4.0;
        clear.lift = true;
        clear.actions.left = send(Command::NotificationsClear);
        col.pieces.push(clear);
        // Apps in the order of their latest notification
        let mut apps: Vec<&str> = Vec::new();
        for n in &notifications.list {
            if !apps.contains(&n.app.as_str()) {
                apps.push(&n.app);
            }
        }
        let limit = room - session_h - PAD - 12.0;
        let mut hidden = 0;
        for app in apps {
            let mut first = true;
            for n in notifications.list.iter().filter(|n| n.app == app) {
                // The app's name goes over its first entry, and only if that entry fits too
                let (before, y) = (col.pieces.len(), col.y);
                if first {
                    col.label(if app.is_empty() { "Other" } else { app });
                }
                // Under its app's name, the entry doesn't need to say it again
                let quiet = crate::model::Notification { app: String::new(), ..n.clone() };
                let (pieces, h) = crate::cards::entry(t, col.ctx.text, &quiet, PAD, col.y, col.w);
                if col.y + h > limit {
                    col.pieces.truncate(before);
                    col.y = y;
                    hidden += 1;
                    continue;
                }
                first = false;
                col.pieces.extend(pieces);
                col.y += h;
            }
        }
        if hidden > 0 {
            col.note(&format!("{hidden} more in the notification centre"));
        }
    }

    // Lock, sleep, log out, restart, shut down; the drastic ones ask twice
    col.separator();
    let entries: [(&str, &str, Command, bool); 5] = [
        ("lock", "lock", Command::Run(commands.lock.clone()), false),
        ("suspend", "sleep", Command::Run(commands.suspend.clone()), false),
        ("logout", "logout", if commands.logout.is_empty() { Command::Quit } else { Command::Run(commands.logout.clone()) }, true),
        ("reboot", "restart", Command::Run(commands.reboot.clone()), true),
        ("poweroff", "power", Command::Run(commands.poweroff.clone()), true),
    ];
    let slot = col.w / entries.len() as f32;
    for (i, (key, icon, command, drastic)) in entries.into_iter().enumerate() {
        let is_armed = armed.as_deref() == Some(key);
        let mut button = Piece::new(format!("control.session.{key}"), 40.0, 40.0);
        button.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0, radius: 20.0, color: if is_armed { t.error_container } else { t.container_high } });
        button.prims.push(Prim::Icon { x: 10.0, y: 10.0, size: 20.0, name: icon.into(), color: if is_armed { t.warn } else { t.color_text } });
        button.label = Some(if is_armed { "Click again".into() } else { key.into() });
        button.actions.left = Some(if drastic && !is_armed { Action::Arm(key.into()) } else { close_after(command) });
        button.lift = true;
        button.pad = 8.0;
        button.x = PAD + slot * i as f32 + (slot - 40.0) / 2.0;
        button.y = col.y;
        col.pieces.push(button);
    }
    col.gap(session_h);

    let h = (col.y + PAD - 6.0).round();
    let mut pieces = Vec::with_capacity(col.pieces.len() + 1);
    // Hung from the right edge of the frame, from the bar down to the bottom edge
    pieces.push(card(t, w, h, 1 | 2 | 4));
    pieces.extend(col.pieces);
    Built { w, h, pieces }
}
