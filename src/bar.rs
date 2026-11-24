//! The bar's modules, built from the model each time it changes, and laid out in three
//! regions: the left end, the middle and the right end.

use crate::config::{Config, MediaTitle, Tool};
use crate::model::{Command, CompositorKind, Model, Workspace, WorkspaceTarget};
use crate::popups::Kind;
use crate::render::text::{Font, Text};
use crate::theme::{Icons, Rgba, Theme};
use crate::ui::{Action, Animator, Ease, Piece, Prim};
use std::collections::HashMap;

pub struct Ctx<'a> {
    pub model: &'a Model,
    pub config: &'a Config,
    pub theme: &'a Theme,
    pub tools: &'a [Tool],
    pub text: &'a mut Text,
    pub anim: &'a mut Animator,
    pub output: &'a str,
    pub hovered: Option<&'a str>,
    pub now: chrono::DateTime<chrono::Local>,
    pub width: f32,
    /// How far each sliding title has gone.
    pub marquee: &'a HashMap<String, f32>,
    /// How much narrower the music title must be for the middle to fit between the ends.
    pub squeeze: f32,
    /// Moving the items about: arrange mode, and the one being carried.
    pub arrange: Option<&'a Arranging>,
}

pub struct Layout {
    pub pieces: Vec<Piece>,
    /// Each region's span on the x axis, for the cards behind them.
    pub regions: Vec<(f32, f32)>,
    /// Where each item is, and where an empty part would take one: what a drop is measured by.
    pub placed: Placed,
}

/// Moving the bar's items about on the bar itself: arrange mode (every item shows it can be
/// picked up, clicks don't open anything, a Done button at the end), and the item carried.
#[derive(Clone, Debug, Default)]
pub struct Arranging {
    pub mode: bool,
    pub carry: Option<Carry>,
}

#[derive(Clone, Debug)]
pub struct Carry {
    pub item: String,
    /// Where the pointer is along the bar, and where it holds the item (from its left edge).
    pub x: f32,
    pub grab: f32,
    /// Pulled well off the bar: let go there, it's taken off.
    pub off: bool,
    /// Moved far enough to be a carry, not a click (a middle press waits for that).
    pub moved: bool,
}

/// Where one item sits on the bar: its part (0 left, 1 middle, 2 right) and its extent.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub name: String,
    pub part: usize,
    pub x0: f32,
    pub x1: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Placed {
    pub spans: Vec<Span>,
    /// Where each part would put an item if it has none.
    pub anchors: [f32; 3],
    /// The room between items.
    pub gap: f32,
}

/// Where a carried item would land: its part, the shown item it would follow (None: the part's
/// start), and where along the bar that is.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub part: usize,
    pub after: Option<String>,
    pub x: f32,
}

impl Placed {
    /// The place between the other items nearest to `x`, in any part.
    pub fn slot(&self, carried: &str, x: f32) -> Slot {
        let mut best: Option<(f32, Slot)> = None;
        for part in 0..3 {
            let here: Vec<&Span> = self.spans.iter().filter(|s| s.part == part && s.name != carried).collect();
            let mut places = Vec::new();
            match here.first() {
                None => places.push(Slot { part, after: None, x: self.anchors[part] }),
                Some(first) => places.push(Slot { part, after: None, x: first.x0 - self.gap / 2.0 }),
            }
            for (i, span) in here.iter().enumerate() {
                let x = here.get(i + 1).map_or(span.x1 + self.gap / 2.0, |next| (span.x1 + next.x0) / 2.0);
                places.push(Slot { part, after: Some(span.name.clone()), x });
            }
            for place in places {
                let d = (place.x - x).abs();
                if best.as_ref().is_none_or(|(b, _)| d < *b) {
                    best = Some((d, place));
                }
            }
        }
        best.map(|(_, slot)| slot).unwrap_or(Slot { part: 1, after: None, x })
    }

    /// The item at `x`, if any.
    pub fn at(&self, x: f32) -> Option<&Span> {
        let half = self.gap / 2.0;
        self.spans.iter().find(|s| x >= s.x0 - half && x <= s.x1 + half)
    }
}

/// One module's pieces and the space between them.
struct Module {
    pieces: Vec<Piece>,
    spacing: f32,
}

impl Module {
    fn width(&self) -> f32 {
        self.pieces.iter().map(|p| p.w).sum::<f32>() + self.spacing * (self.pieces.len() as f32 - 1.0).max(0.0)
    }
}


/// "225 kB/s", then "1.4 MB/s" once it gets big.
pub fn rate(bytes: f64) -> String {
    if bytes >= 1_048_576.0 { format!("{:.1} MB/s", bytes / 1_048_576.0) } else { format!("{} kB/s", (bytes / 1024.0).round()) }
}

pub fn send(command: Command) -> Option<Action> {
    Some(Action::Send(command))
}

pub fn menu(kind: Kind) -> Option<Action> {
    Some(Action::Menu(kind))
}

impl Ctx<'_> {
    fn icon_text(&mut self, key: &str, icon: &str, value: &str, tint: Rgba, value_color: Rgba) -> Piece {
        let t = self.theme;
        let size = t.icon;
        let font = Font::new(t.text - 1.0).tnum();
        let (tw, th) = if value.is_empty() { (0.0, 0.0) } else { self.text.measure(value, font) };
        let w = size + if value.is_empty() { 0.0 } else { 4.0 + tw };
        let h = size.max(th);
        let mut piece = Piece::new(key, w, h);
        piece.prims.push(Prim::Icon { x: 0.0, y: (h - size) / 2.0, size, name: icon.into(), color: tint });
        if !value.is_empty() {
            piece.prims.push(Prim::Text { x: size + 4.0, y: (h - th) / 2.0, text: value.into(), font, color: value_color });
        }
        piece.glow = true;
        piece.lift = true;
        piece
    }

    // ------------------------------------------------------------ pages

    fn pages(&mut self) -> Module {
        let t = self.theme;
        let state = &self.model.compositor;
        let mut cells: Vec<(i64, i64, Option<Workspace>)> = Vec::new();
        // niri keeps only the workspaces in use and one empty one after them; the bar still
        // shows `pages` of them, the ones niri doesn't have yet as places that all lead to that
        // empty one (niri numbers it next in line once something opens there)
        let mut empty_one = None;
        match state.kind {
            CompositorKind::Niri => {
                let mut on: Vec<&Workspace> = state.workspaces_on(self.output).collect();
                on.sort_by_key(|w| w.idx);
                empty_one = on.last().map(|w| w.id);
                let next = on.last().map_or(1, |w| w.idx + 1);
                cells.extend(on.into_iter().map(|w| (w.id, w.idx, Some(w.clone()))));
                // Their ids are made up (below zero), so nothing takes them for real ones
                cells.extend((next..=self.config.pages as i64).map(|idx| (-idx, idx, None)));
            }
            _ => {
                // Hyprland: a group of pages, the one holding this output's workspace
                let pages = self.config.pages.max(1) as i64;
                let active = state.active_on.get(self.output).copied().unwrap_or(1).max(1);
                let start = (active - 1) / pages * pages + 1;
                for id in start..start + pages {
                    let found = state.workspace(id).cloned();
                    cells.push((id, id, found));
                }
            }
        }
        let active_here = state.active_on.get(self.output).copied();
        let output = self.output.to_string();
        // Which cells are stars (something open there, or where you are): the line joins the
        // first to the last of them
        let stars: Vec<bool> = cells.iter().map(|(id, _, w)| active_here == Some(*id) || w.as_ref().is_some_and(|w| !w.windows.is_empty())).collect();
        let first = stars.iter().position(|s| *s);
        let last = stars.iter().rposition(|s| *s);
        let mut pieces = Vec::new();
        for (cell, (id, idx, workspace)) in cells.into_iter().enumerate() {
            let key = format!("page.{id}");
            let windows = workspace.as_ref().map_or(&[][..], |w| &w.windows[..]);
            let count = windows.len().min(6);
            let active = active_here == Some(id);
            let lit = active || self.hovered == Some(key.as_str());
            // A figure's place is as wide as it needs (十一 takes two)
            let figure_font = Font::new(if t.kanji { t.text + 2.0 } else { t.text }).weight(if active { 700 } else { 500 }).tnum();
            let figure = if t.kanji { kanji(idx) } else { idx.to_string() };
            let h = t.bar_height;
            let w = if t.numbers { t.page_width.max(self.text.measure(&figure, figure_font).0 + 14.0) } else { t.page_width };
            let mut piece = Piece::new(&key, w, h);
            piece.pad = 24.0;
            let to_come = state.kind == CompositorKind::Niri && id < 0;
            let go = if to_come { empty_one.unwrap_or(id) } else { id };
            piece.actions.left = send(Command::FocusWorkspace(WorkspaceTarget::Id(go)));
            if to_come {
                piece.opacity = 0.6;
            }
            piece.actions.right = Some(Action::ToggleOverview);
            piece.actions.up = send(Command::FocusWorkspace(WorkspaceTarget::Step(output.clone(), -1)));
            piece.actions.down = send(Command::FocusWorkspace(WorkspaceTarget::Step(output.clone(), 1)));
            let light = self.anim.value(&format!("{key}.lit"), if lit { 1.0 } else { 0.0 }, t.normal, Ease::OutCubic);

            // Who's there, for the tooltip
            let mut names: Vec<(String, usize)> = Vec::new();
            for w in windows {
                let name = crate::images::app(&w.app_id).map(|a| a.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| w.app_id.clone());
                match names.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, times)) => *times += 1,
                    None => names.push((name, 1)),
                }
            }
            let names: Vec<String> = names.into_iter().map(|(n, times)| if times > 1 { format!("{n} ×{times}") } else { n }).collect();
            piece.label = Some(if names.is_empty() { format!("Workspace {idx} · empty") } else { format!("{idx} · {}", names.join(", ")) });
            piece.lift = true;
            if t.numbers {
                let (tw, th) = self.text.measure(&figure, figure_font);
                if t.kanji {
                    // The one you're on takes the accent and a bōten (傍点), the emphasis dot of
                    // Japanese type, set under the character (as in horizontal text, beside the
                    // underline's place rather than over the figure); both fade across
                    let fade = self.anim.value(&format!("{key}.fill"), if active { 1.0 } else { 0.0 }, t.normal, Ease::OutCubic);
                    let rest = if count > 0 { t.icon_tint(Icons::Status) } else { t.outline };
                    let color = rest.mix(t.color_text, light * 0.5 * (1.0 - fade)).mix(t.primary, fade);
                    let (x, y) = (((w - tw) / 2.0).round(), ((h - th) / 2.0).round());
                    piece.prims.push(Prim::Text { x, y, text: figure, font: figure_font, color });
                    // The dot takes the underline's place: the one you're on is marked already
                    piece.lift = !active;
                    if fade > 0.01 {
                        // Well clear of the character's ink (二's lower stroke reaches about 0.47
                        // of the size below the middle): at 1× a nearer dot read as part of it
                        let r = (2.2 * t.scale).max(1.8);
                        let ink = h / 2.0 + (t.text + 2.0) * 0.47;
                        let below = ink + 4.0 * t.scale + r;
                        piece.prims.push(Prim::Rect { x: w / 2.0 - r, y: below - r, w: r * 2.0, h: r * 2.0, radius: r, color: t.primary.alpha(fade) });
                    }
                    pieces.push(piece);
                    continue;
                }
                let fill = self.anim.value(&format!("{key}.fill"), if active { 1.0 } else { 0.0 }, t.normal, Ease::Expressive);
                if fill > 0.01 {
                    // A circle round one figure, a pill round two; it swells in when you arrive
                    let size = (h * if t.kanji { 0.66 } else { 0.6 }).round();
                    let pw = (tw + 10.0).max(size) * (0.6 + 0.4 * fill);
                    let ph = size * (0.6 + 0.4 * fill);
                    piece.prims.push(Prim::Rect { x: (w - pw) / 2.0, y: (h - ph) / 2.0, w: pw, h: ph, radius: ph / 2.0, color: t.primary.fade(fill.clamp(0.0, 1.0)) });
                }
                let color = if active { t.on_primary } else if count > 0 { t.icon_tint(Icons::Status) } else { t.outline };
                piece.prims.push(Prim::Text { x: ((w - tw) / 2.0).round(), y: ((h - th) / 2.0).round(), text: figure, font: figure_font, color });
            } else {
                // An asterism: a dot per workspace, the busy ones solid and joined by a thin
                // line, the empty ones hollow, the one you're on bigger in the accent with a ring
                let grow = self.anim.value(&format!("{key}.grow"), if active { 1.0 } else { 0.0 }, t.normal, Ease::Expressive);
                let settled = grow.clamp(0.0, 1.0);
                let (cx, cy) = (w / 2.0, h / 2.0);
                let busy = count > 0;
                let r = if busy { 3.0 } else { 2.6 } + 1.6 * grow;
                let gap = r + 2.5 + 3.0 * grow;
                let thread = t.outline.alpha(0.55);
                // The line runs from the first star to the last, broken around each dot
                let (from, to) = (first.unwrap_or(usize::MAX), last.unwrap_or(0));
                if from < cell && cell <= to {
                    piece.prims.push(Prim::Line { points: vec![(-0.5, cy), ((cx - gap).max(0.0), cy)], width: 1.2, color: thread });
                }
                if from <= cell && cell < to {
                    piece.prims.push(Prim::Line { points: vec![((cx + gap).min(w), cy), (w + 0.5, cy)], width: 1.2, color: thread });
                }
                let rest = if busy { t.icon_tint(Icons::Status).mix(t.color_soft, 0.3) } else { t.outline };
                let color = rest.mix(t.color_text, light * 0.5 * (1.0 - settled)).mix(t.primary, settled);
                if busy || grow > 0.5 {
                    piece.prims.push(Prim::Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0, radius: r, color });
                } else {
                    piece.prims.push(Prim::Ring { cx, cy, r, width: 1.3, value: 1.0, color });
                }
                if grow > 0.02 {
                    piece.prims.push(Prim::Ring { cx, cy, r: r + 3.2, width: 1.2, value: 1.0, color: t.primary.alpha(0.5 * settled) });
                }
            }
            pieces.push(piece);
        }
        Module { pieces, spacing: t.page_gap }
    }

    // ------------------------------------------------------------ clock

    /// The weather where you are: its icon and the temperature; its menu has the rest.
    fn weather(&mut self) -> Module {
        let t = self.theme;
        let w = &self.model.weather;
        if !w.available {
            return Module { pieces: Vec::new(), spacing: 0.0 };
        }
        let icon = crate::weather::icon(w.code, w.is_day);
        let mut piece = self.icon_text("weather", icon, &format!("{:.0}°", w.temperature), t.icon_tint(Icons::Machine), t.color_text);
        let place = if w.place.is_empty() { String::new() } else { format!(" · {}", w.place) };
        piece.label = Some(format!("{}, feels like {:.0}°{place}", crate::weather::describe(w.code), w.feels_like));
        piece.actions.left = menu(Kind::Weather);
        Module { pieces: vec![piece], spacing: 0.0 }
    }

    fn clock(&mut self) -> Module {
        let t = self.theme;
        let clock = &self.config.clock;
        let format = match (clock.hours24, clock.seconds) {
            (true, false) => "%H:%M",
            (true, true) => "%H:%M:%S",
            (false, false) => "%-I:%M %p",
            (false, true) => "%-I:%M:%S %p",
        };
        let time = self.now.format(format).to_string();
        let date = if clock.date.is_empty() { String::new() } else { self.now.format(&clock.date).to_string() };
        let time_font = Font::new(t.text + 6.0).weight(500).tnum();
        let date_font = Font::new(t.text).spacing(0.3);
        let (tw, th) = self.text.measure(&time, time_font);
        let (dw, dh) = if date.is_empty() { (0.0, 0.0) } else { self.text.measure(&date, date_font) };
        let w = tw + if date.is_empty() { 0.0 } else { 8.0 + dw };
        let h = th.max(dh);
        let mut piece = Piece::new("clock", w, h);
        piece.prims.push(Prim::Text { x: 0.0, y: (h - th) / 2.0, text: time, font: time_font, color: t.color_text });
        if !date.is_empty() {
            piece.prims.push(Prim::Text { x: tw + 8.0, y: (h - dh) / 2.0 + 1.0, text: date, font: date_font, color: t.color_soft });
        }
        piece.glow = true;
        piece.lift = true;
        // A click opens the calendar; your day (timers, to-do, notes) has a button of its own
        piece.actions.left = menu(Kind::Calendar);
        Module { pieces: vec![piece], spacing: 0.0 }
    }

    // ------------------------------------------------------------ media

    fn media(&mut self) -> Module {
        let t = self.theme;
        let Some(player) = self.model.media.player() else { return Module { pieces: vec![], spacing: 0.0 } };
        let font = Font::new(t.text);
        let note = t.icon - 3.0;
        let limit = ((200.0 * t.scale).round() - self.squeeze).max(0.0);
        let mode = self.config.media_title;
        let text = if mode == MediaTitle::Scroll && !player.artist.is_empty() { format!("{} · {}", player.title, player.artist) } else { player.title.clone() };
        let (tw, th) = self.text.measure(&text, font);
        let moving = mode == MediaTitle::Scroll && tw > limit;
        let view = if mode == MediaTitle::Off || limit < 40.0 { 0.0 } else { tw.min(limit) };
        let w = note + if view > 0.0 { 6.0 + view } else { 0.0 };
        let h = note.max(th);
        let mut piece = Piece::new("media", w, h);
        // A sliding title brings its own glow, so the piece doesn't need painting again each frame
        piece.glow = !moving;
        piece.pad = if moving { crate::render::GLOW_ROOM } else { 0.0 };
        let tint = t.icon_tint(Icons::Machine);
        let icon = if moving { Prim::GlowIcon { x: 0.0, y: (h - note) / 2.0, size: note, name: "music".into(), color: tint } } else { Prim::Icon { x: 0.0, y: (h - note) / 2.0, size: note, name: "music".into(), color: tint } };
        piece.prims.push(icon);
        if view > 0.0 {
            let (x, y) = (note + 6.0, (h - th) / 2.0);
            if moving {
                let offset = self.marquee.get("media").copied().unwrap_or(0.0) % (tw + 48.0);
                piece.prims.push(Prim::Marquee { x, y, width: view, text, font, color: t.color_soft, offset, gap: 48.0 });
                piece.moving = player.playing && self.hovered != Some("media");
            } else {
                let shown = if tw > limit { self.ellipsize(&text, font, limit) } else { text };
                piece.prims.push(Prim::Text { x, y, text: shown, font, color: t.color_soft });
            }
        }
        piece.opacity = if player.playing { 1.0 } else { 0.6 };
        piece.lift = true;
        piece.actions.left = menu(Kind::Media);
        piece.actions.right = send(Command::MediaPlayPause(player.bus.clone()));
        Module { pieces: vec![piece], spacing: 0.0 }
    }

    pub fn ellipsize(&mut self, text: &str, font: Font, limit: f32) -> String {
        let chars: Vec<char> = text.chars().collect();
        let (mut low, mut high) = (0, chars.len());
        while low < high {
            let mid = (low + high).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if self.text.measure(&candidate, font).0 <= limit { low = mid } else { high = mid - 1 }
        }
        chars[..low].iter().collect::<String>().trim_end().to_string() + "…"
    }

    // ------------------------------------------------------------ resources

    fn resources(&mut self) -> Module {
        let t = self.theme;
        let machine = &self.model.machine;
        let mut pieces = Vec::new();
        for name in self.config.resources.clone() {
            match name.as_str() {
                "cpu" | "memory" | "temperature" => {
                    if name == "temperature" && machine.temperature <= 0.0 {
                        continue;
                    }
                    // The heat fills its ring from a cool 30 °C to 100 °C, and turns to the warning colour past 85
                    let (value, hot) = match name.as_str() {
                        "cpu" => (machine.cpu, machine.cpu > 0.85),
                        "memory" => (machine.memory, machine.memory > 0.85),
                        _ => (((machine.temperature - 30.0) / 70.0).clamp(0.0, 1.0), machine.temperature > 85.0),
                    };
                    let tint = if hot { t.warn } else { t.color_text };
                    let icon_tint = if hot { t.warn } else { t.icon_tint(Icons::Machine) };
                    let accent = if hot { t.warn } else { t.primary };
                    let ring = t.icon + 6.0;
                    let font = Font::new(t.text).tnum();
                    let label = if name == "temperature" { format!("{}°", machine.temperature.round()) } else { format!("{}%", (value * 100.0).round()) };
                    let (tw, th) = if self.config.percent { self.text.measure(&label, font) } else { (0.0, 0.0) };
                    let w = ring + if self.config.percent { 6.0 + tw } else { 0.0 };
                    let h = ring.max(th);
                    let mut piece = Piece::new(format!("res.{name}"), w, h);
                    let c = ring / 2.0;
                    piece.prims.push(Prim::Ring { cx: c, cy: h / 2.0, r: c - 2.0, width: 3.0, value, color: accent });
                    let inner = t.icon - 7.0;
                    let icon = match name.as_str() {
                        "cpu" => "cpu",
                        "memory" => "ram",
                        _ => "thermometer",
                    };
                    piece.prims.push(Prim::Icon { x: c - inner / 2.0, y: (h - inner) / 2.0, size: inner, name: icon.into(), color: icon_tint });
                    if self.config.percent {
                        piece.prims.push(Prim::Text { x: ring + 6.0, y: (h - th) / 2.0, text: label, font, color: tint });
                    }
                    piece.label = Some(match name.as_str() {
                        "cpu" => format!("Processor · {} cores", machine.cores.len().max(1)),
                        "memory" => format!("Memory · {:.1} of {:.1} GB", machine.memory_used_kib as f64 / 1_048_576.0, machine.memory_total_kib as f64 / 1_048_576.0),
                        _ => format!("Processor temperature · {:.0} °C", machine.temperature),
                    });
                    piece.glow = true;
                    piece.lift = true;
                    piece.actions.left = Some(Action::ToggleDashboard);
                    pieces.push(piece);
                }
                "network" => {
                    // Short and steady: the numbers never make the bar jump around
                    let font = Font::new(t.text - 1.0).tnum();
                    let widest = self.text.measure("88.8 MB/s", font).0;
                    let arrow = t.icon - 5.0;
                    let th = self.text.measure("0", font).1;
                    let h = arrow.max(th);
                    let half = arrow + 2.0 + widest;
                    let mut piece = Piece::new("res.network", half * 2.0 + 6.0, h);
                    for (i, (icon, bytes)) in [("down", machine.download), ("up", machine.upload)].into_iter().enumerate() {
                        let x = i as f32 * (half + 6.0);
                        piece.prims.push(Prim::Icon { x, y: (h - arrow) / 2.0, size: arrow, name: icon.into(), color: t.icon_tint(Icons::Machine) });
                        piece.prims.push(Prim::Text { x: x + arrow + 2.0, y: (h - th) / 2.0, text: rate(bytes), font, color: t.color_soft });
                    }
                    piece.glow = true;
                    piece.lift = true;
                    piece.actions.left = menu(Kind::Traffic);
                    pieces.push(piece);
                }
                _ => {}
            }
        }
        Module { pieces, spacing: t.spacing }
    }

    // ------------------------------------------------------------ tray

    fn tray(&mut self) -> Module {
        let t = self.theme;
        let size = t.icon - 1.0;
        let mut pieces = Vec::new();
        for item in &self.model.tray {
            let mut piece = Piece::new(format!("tray.{}", item.id), size, size);
            match &item.icon {
                Some(image) => {
                    let tint = self.config.tray_tint.then_some(t.icon_tint(Icons::Tools));
                    piece.prims.push(Prim::Image { x: 0.0, y: 0.0, w: size, h: size, image: image.clone(), tint, radius: 0.0 });
                }
                None => {
                    // An app with no picture still gets a place
                    piece.prims.push(Prim::Rect { x: size * 0.2, y: size * 0.2, w: size * 0.6, h: size * 0.6, radius: size * 0.3, color: t.color_soft });
                }
            }
            let open_menu = Action::Both(Box::new(Action::Send(Command::TrayMenu(item.id.clone()))), Box::new(Action::Menu(Kind::Tray(item.id.clone()))));
            piece.actions.left = if item.item_is_menu && item.has_menu { Some(open_menu.clone()) } else { send(Command::TrayActivate(item.id.clone(), 0, 0)) };
            piece.actions.right = if item.has_menu { Some(open_menu) } else { send(Command::TraySecondary(item.id.clone(), 0, 0)) };
            piece.actions.middle = send(Command::TraySecondary(item.id.clone(), 0, 0));
            piece.actions.up = send(Command::TrayScroll(item.id.clone(), -1));
            piece.actions.down = send(Command::TrayScroll(item.id.clone(), 1));
            piece.label = (!item.title.is_empty()).then(|| item.title.clone());
            piece.lift = true;
            pieces.push(piece);
        }
        Module { pieces, spacing: (t.spacing * 0.7).round() }
    }

    // ------------------------------------------------------------ tools

    fn tools(&mut self) -> Module {
        let t = self.theme;
        let mut pieces = Vec::new();
        for (i, tool) in self.tools.iter().enumerate() {
            let on = match tool.kind.as_str() {
                "microphone" => self.model.audio.has_mic && !self.model.audio.mic_muted,
                "awake" => self.model.awake,
                _ => false,
            };
            let icon = if tool.kind == "microphone" && !on { "microphone-off".to_string() } else { tool.icon.clone() };
            let mut piece = self.icon_text(&format!("tool.{i}"), &icon, "", if on { t.lit } else { t.icon_tint(Icons::Tools) }, t.color_soft);
            piece.label = (!tool.label.is_empty()).then(|| tool.label.clone());
            piece.actions.left = match tool.kind.as_str() {
                "microphone" => send(Command::ToggleMicMute),
                "awake" => send(Command::ToggleAwake),
                "clipboard" => Some(Action::ToggleClipboard),
                "capture" => menu(Kind::Capture),
                _ => send(Command::Run(tool.command.clone())),
            };
            pieces.push(piece);
        }
        Module { pieces, spacing: (t.spacing * 0.8).round() }
    }

    // ------------------------------------------------------------ privacy

    /// The privacy pill: a tonal capsule holding an icon for each sense an app is using, so it
    /// shows from across the room without shouting. A paused screencast is drawn faint.
    fn privacy(&mut self) -> Option<Piece> {
        let t = self.theme;
        let model = self.model;
        let using = crate::privacy::InUse::of(model, &self.config.privacy);
        if using.is_empty() {
            return None;
        }
        let mut senses: Vec<(&str, bool)> = Vec::new();
        if !using.microphone.is_empty() {
            senses.push((if model.audio.mic_muted { "microphone-off" } else { "microphone" }, true));
        }
        if !using.camera.is_empty() {
            senses.push(("camera", true));
        }
        if !using.screen.is_empty() {
            senses.push(("screen-share", using.screen.iter().any(|c| c.active)));
        }
        let size = (t.icon - 3.0).round();
        let (pad, gap) = (7.0, 4.0);
        let h = (t.icon + 4.0).round();
        let n = senses.len() as f32;
        let w = pad * 2.0 + size * n + gap * (n - 1.0);
        let mut piece = Piece::new("status.privacy", w, h);
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.primary_container });
        for (i, (icon, lit)) in senses.into_iter().enumerate() {
            let color = if lit { t.on_primary_container } else { t.on_primary_container.fade(0.45) };
            piece.prims.push(Prim::Icon { x: pad + i as f32 * (size + gap), y: ((h - size) / 2.0).round(), size, name: icon.into(), color });
        }
        piece.lift = true;
        piece.label = Some(using.summary());
        piece.actions.left = menu(Kind::Privacy);
        if !using.microphone.is_empty() {
            piece.actions.right = send(Command::ToggleMicMute);
        }
        Some(piece)
    }

    // ------------------------------------------------------------ status

    fn status(&mut self) -> Module {
        let t = self.theme;
        let model = self.model;
        let mut pieces = Vec::new();
        // Recording the screen: a red dot and how long, a click stops it
        if let Some(recording) = &model.recording {
            let seconds = (chrono::Local::now() - recording.since).num_seconds().max(0) as u64;
            let text = crate::timers::clock_text(std::time::Duration::from_secs(seconds));
            let font = Font::new(t.text - 1.0).tnum();
            let (tw, th) = self.text.measure(&text, font);
            let (w, h) = (14.0 + tw, th.max(12.0));
            let mut piece = Piece::new("status.recording", w, h);
            piece.prims.push(Prim::Rect { x: 0.0, y: (h - 9.0) / 2.0, w: 9.0, h: 9.0, radius: 4.5, color: t.warn });
            piece.prims.push(Prim::Text { x: 14.0, y: (h - th) / 2.0, text, font, color: t.color_text });
            piece.lift = true;
            piece.label = Some(format!("Recording{} · click to stop", if recording.audio { " with sound" } else { "" }));
            piece.actions.left = send(Command::RecordStop);
            pieces.push(piece);
        }
        // An app using the microphone, a camera or the screen: a pill with one icon for each
        if let Some(piece) = self.privacy() {
            pieces.push(piece);
        }
        let percent = self.config.percent;
        for name in self.config.status.clone() {
            let key = format!("status.{name}");
            let piece = match name.as_str() {
                "notifications" if model.notifications.available || !self.config.commands.notifications.is_empty() => {
                    let n = &model.notifications;
                    let count = if n.count > 0 { n.count.to_string() } else { String::new() };
                    let mut piece = self.icon_text(&key, if n.quiet { "bell-slash" } else { "bell" }, &count, t.icon_tint(Icons::Status), t.color_soft);
                    piece.actions.left = if !self.config.commands.notifications.is_empty() {
                        send(Command::Run(self.config.commands.notifications.clone()))
                    } else if n.own {
                        menu(Kind::Notifications)
                    } else {
                        send(Command::NotificationsToggle)
                    };
                    piece.actions.right = send(Command::NotificationsQuiet);
                    piece.label = Some(if n.quiet { "Do not disturb".into() } else { "Notifications".into() });
                    Some(piece)
                }
                "keyboard" if model.compositor.layouts.len() > 1 => {
                    let c = &model.compositor;
                    let code = c.layouts.get(c.layout).cloned().unwrap_or_default().to_uppercase();
                    let mut piece = self.icon_text(&key, "keyboard", &code, t.icon_tint(Icons::Status), t.color_soft);
                    piece.actions.left = send(Command::NextLayout);
                    piece.actions.right = menu(Kind::Keyboard);
                    piece.label = c.layout_names.get(c.layout).cloned();
                    Some(piece)
                }
                "network" if model.network.available => {
                    let n = &model.network;
                    let icon = match n.kind.as_str() {
                        "wifi" if n.strength > 66 => "wifi-high",
                        "wifi" if n.strength > 33 => "wifi-medium",
                        "wifi" => "wifi-low",
                        "" => "wifi-slash",
                        _ => "plugs-connected",
                    };
                    let mut piece = self.icon_text(&key, icon, "", t.icon_tint(Icons::Status), t.color_soft);
                    piece.label = Some(if n.kind.is_empty() { "Offline".into() } else { n.name.clone() });
                    piece.actions.left = menu(Kind::Network);
                    Some(piece)
                }
                "bluetooth" if model.bluetooth.available => {
                    let b = &model.bluetooth;
                    let connected: Vec<_> = b.connected().collect();
                    let lowest = connected.iter().filter(|d| d.battery >= 0).map(|d| d.battery).min();
                    let (icon, value) = if !b.powered {
                        ("bluetooth-slash", String::new())
                    } else if let (Some(level), true) = (lowest, percent) {
                        ("bluetooth-connected", format!("{level}%"))
                    } else if !connected.is_empty() {
                        ("bluetooth-connected", connected.len().to_string())
                    } else {
                        ("bluetooth", String::new())
                    };
                    let mut piece = self.icon_text(&key, icon, &value, t.icon_tint(Icons::Status), t.color_soft);
                    // Each device with its charge, when it says
                    piece.label = Some(if connected.is_empty() {
                        "Bluetooth".into()
                    } else {
                        connected.iter().map(|d| if d.battery >= 0 { format!("{} {}%", d.name, d.battery) } else { d.name.clone() }).collect::<Vec<_>>().join(" · ")
                    });
                    piece.actions.left = menu(Kind::Bluetooth);
                    piece.actions.right = send(Command::SetBluetoothPower(!b.powered));
                    Some(piece)
                }
                "volume" if model.audio.available => {
                    let a = &model.audio;
                    let icon = if a.muted { "speaker-x" } else if a.volume > 0.5 { "speaker-high" } else if a.volume > 0.0 { "speaker-low" } else { "speaker-none" };
                    let value = if percent { format!("{}%", (a.volume * 100.0).round()) } else { String::new() };
                    let mut piece = self.icon_text(&key, icon, &value, t.icon_tint(Icons::Status), t.color_soft);
                    piece.actions.left = menu(Kind::Sound);
                    piece.actions.right = send(Command::ToggleMute);
                    piece.actions.up = send(Command::StepVolume(0.05));
                    piece.actions.down = send(Command::StepVolume(-0.05));
                    piece.label = a.sinks.iter().find(|s| s.name == a.sink).map(|s| s.description.clone());
                    Some(piece)
                }
                "battery" if model.battery.present => {
                    let b = &model.battery;
                    let icon = if !b.on_battery {
                        "battery-charging"
                    } else if b.level > 0.8 {
                        "battery-full"
                    } else if b.level > 0.5 {
                        "battery-high"
                    } else if b.level > 0.25 {
                        "battery-medium"
                    } else if b.level > 0.12 {
                        "battery-low"
                    } else {
                        "battery-warning"
                    };
                    let low = b.on_battery && b.level < 0.15;
                    let tint = if low { t.warn } else { t.icon_tint(Icons::Status) };
                    let value = if percent { format!("{}%", (b.level * 100.0).round()) } else { String::new() };
                    let mut piece = self.icon_text(&key, icon, &value, tint, if low { t.warn } else { t.color_soft });
                    piece.actions.left = menu(Kind::Power);
                    piece.label = Some(crate::popups::battery_line(b));
                    Some(piece)
                }
                "settings" => {
                    // Its own command when there's one, else Asterism's settings
                    let mut piece = self.icon_text(&key, "gear-six", "", t.icon_tint(Icons::Status), t.color_soft);
                    piece.actions.left = if self.config.commands.settings.is_empty() { Some(Action::ToggleSettings) } else { send(Command::Run(self.config.commands.settings.clone())) };
                    piece.label = Some("Settings".into());
                    Some(piece)
                }
                "timer" => {
                    // Quiet until something runs: then a ring for what's left, and the time
                    let timers = &model.timers;
                    let now = std::time::Instant::now();
                    let running = timers
                        .countdown
                        .map(|c| (c.clock.left(now), c.total, t.primary, "Countdown"))
                        .or(timers.focus.map(|f| (f.clock.left(now), f.total, if f.phase == crate::timers::Phase::Work { t.primary } else { t.palette.tertiary }, "Focus")));
                    let mut piece = match running {
                        Some((left, total, color, what)) => {
                            let text = crate::timers::clock_text(left);
                            let font = Font::new(t.text - 1.0).tnum();
                            let (tw, th) = self.text.measure(&text, font);
                            let ring = t.icon + 6.0;
                            let (w, h) = (ring + 5.0 + tw, ring.max(th));
                            let mut piece = Piece::new(&key, w, h);
                            let c = ring / 2.0;
                            let value = left.as_secs_f32() / total.as_secs_f32().max(1.0);
                            piece.prims.push(Prim::Ring { cx: c, cy: h / 2.0, r: c - 2.0, width: 3.0, value, color });
                            let inner = t.icon - 7.0;
                            piece.prims.push(Prim::Icon { x: c - inner / 2.0, y: (h - inner) / 2.0, size: inner, name: "timer".into(), color: t.icon_tint(Icons::Status) });
                            piece.prims.push(Prim::Text { x: ring + 5.0, y: (h - th) / 2.0, text, font, color: t.color_text });
                            piece.lift = true;
                            piece.label = Some(format!("{what}: {} left", crate::timers::length_text(left + std::time::Duration::from_secs(59))));
                            piece
                        }
                        None if timers.stopwatch.since.is_some() => {
                            let text = crate::timers::clock_text(timers.stopwatch.elapsed(now));
                            let mut piece = self.icon_text(&key, "timer", &text, t.primary, t.color_text);
                            piece.label = Some("Stopwatch".into());
                            piece
                        }
                        None => {
                            let mut piece = self.icon_text(&key, "timer", "", t.icon_tint(Icons::Status), t.color_soft);
                            piece.label = Some("Timers".into());
                            piece
                        }
                    };
                    piece.actions.left = menu(Kind::Timer);
                    Some(piece)
                }
                "control" => {
                    let mut piece = self.icon_text(&key, "sliders", "", t.icon_tint(Icons::Status), t.color_soft);
                    piece.actions.left = Some(Action::ToggleControl);
                    piece.label = Some("Control centre".into());
                    Some(piece)
                }
                "session" => {
                    let mut piece = self.icon_text(&key, "power", "", t.icon_tint(Icons::Status), t.color_soft);
                    piece.actions.left = menu(Kind::Session);
                    piece.label = Some("Session".into());
                    Some(piece)
                }
                _ => None,
            };
            pieces.extend(piece);
        }
        Module { pieces, spacing: t.spacing }
    }

    fn module(&mut self, name: &str) -> Option<Module> {
        let module = match name {
            "pages" => self.pages(),
            "clock" => self.clock(),
            "media" => self.media(),
            "resources" => self.resources(),
            "weather" => self.weather(),
            "tray" => self.tray(),
            "tools" => self.tools(),
            "status" => self.status(),
            "day" => {
                // Your day: a checklist, with how many tasks are still to do
                let open = self.model.day.tasks.iter().filter(|task| !task.done).count();
                let value = if open > 0 { open.to_string() } else { String::new() };
                let mut piece = self.icon_text("day", "checklist", &value, self.theme.icon_tint(Icons::Tools), self.theme.color_soft);
                piece.actions.left = Some(Action::ToggleDay);
                piece.label = Some(match open {
                    0 => "Your day: timers, to-do, notes".into(),
                    1 => "Your day · 1 thing to do".into(),
                    n => format!("Your day · {n} things to do"),
                });
                Module { pieces: vec![piece], spacing: 0.0 }
            }
            "launcher" => {
                let mut piece = self.icon_text("launcher", "apps", "", self.theme.icon_tint(Icons::Status), self.theme.color_soft);
                piece.actions.left = Some(Action::ToggleLauncher);
                piece.label = Some("Apps".into());
                Module { pieces: vec![piece], spacing: 0.0 }
            }
            other => {
                log::warn!("no bar module called {other:?}");
                return None;
            }
        };
        (!module.pieces.is_empty()).then_some(module)
    }

fn region(&mut self, names: &[String]) -> (Vec<(String, Module)>, f32) {
        let t = self.theme;
        let modules: Vec<(String, Module)> = names.iter().filter_map(|name| self.module(name).map(|m| (name.clone(), m))).collect();
        let total = modules.iter().map(|(_, m)| m.width()).sum::<f32>() + t.spacing * (modules.len() as f32 - 1.0).max(0.0);
        (modules, total)
    }

    /// Arrange mode's Done, at the end of the right part.
    fn done(&mut self) -> (String, Module) {
        let t = self.theme;
        let font = Font::new(t.text - 1.0).weight(500);
        let (tw, th) = self.text.measure("Done", font);
        let (w, h) = (tw + 24.0, (t.bar_height * 0.66).round());
        let mut piece = Piece::new("arrange.done", w, h);
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.primary });
        piece.prims.push(Prim::Text { x: 12.0, y: ((h - th) / 2.0).round(), text: "Done".into(), font, color: t.on_primary });
        piece.actions.left = Some(Action::ArrangeBar(Some(false)));
        piece.label = Some("Finish arranging the bar".into());
        ("@done".into(), Module { pieces: vec![piece], spacing: 0.0 })
    }

    pub fn build(&mut self) -> Layout {
        let t = self.theme;
        let middle = t.middle();
        let mut layout = Layout { pieces: Vec::new(), regions: Vec::new(), placed: Placed { gap: t.spacing, ..Default::default() } };
        let layout_names = self.config.layout.clone();
        let arrange = self.arrange.cloned().unwrap_or_default();
        let (left, left_w) = self.region(&layout_names.left);
        let (mut right, mut right_w) = self.region(&layout_names.right);
        if arrange.mode {
            let done = self.done();
            right_w += done.1.width() + if right.is_empty() { 0.0 } else { t.spacing };
            right.push(done);
        }
        let (mut center, mut center_w) = self.region(&layout_names.center);
        // The middle sits in the middle, unless an end is in the way; then it moves aside,
        // and if there still isn't room, the music title gives some up
        let gap = t.spacing * 2.0;
        let left_end = if left.is_empty() { 0.0 } else { t.bar_side + left_w + gap };
        let right_start = if right.is_empty() { self.width } else { self.width - t.bar_side - right_w - gap };
        let room = right_start - left_end;
        if center_w > room && layout_names.center.iter().any(|m| m == "media") {
            self.squeeze = center_w - room;
            (center, center_w) = self.region(&layout_names.center);
            self.squeeze = 0.0;
        }
        let ideal = ((self.width - center_w) / 2.0).round();
        let center_x = ideal.min(right_start - center_w).max(left_end).round();
        layout.placed.anchors = [t.bar_side, (self.width / 2.0).round(), self.width - t.bar_side];

        let carry = arrange.carry.as_ref().filter(|c| c.moved);
        let mut ghost: Vec<Piece> = Vec::new();
        let (box_h, box_y) = ((t.bar_height - 8.0).max(16.0), (middle - (t.bar_height - 8.0).max(16.0) / 2.0).round());
        for (part, (modules, total, start)) in [
            (left, left_w, t.bar_side),
            (center, center_w, center_x),
            (right, right_w, self.width - t.bar_side - right_w),
        ]
        .into_iter()
        .enumerate()
        {
            if modules.is_empty() {
                continue;
            }
            let mut x = start;
            layout.regions.push((x, x + total));
            for (m, (name, module)) in modules.into_iter().enumerate() {
                if m > 0 {
                    x += t.spacing;
                }
                let (x0, width) = (x, module.width());
                let item = name != "@done";
                if item {
                    layout.placed.spans.push(Span { name: name.clone(), part, x0, x1: x0 + width });
                }
                let carried = carry.is_some_and(|c| c.item == name);
                // In arrange mode each item sits on a faint tint of the accent, and the one under
                // the pointer a little more
                if arrange.mode && item {
                    let hovered = self.hovered == Some(format!("arrange.grab.{name}").as_str());
                    let mut back = Piece::new(format!("arrange.back.{name}"), width + 10.0, box_h);
                    back.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: width + 10.0, h: box_h, radius: (box_h / 2.0).min(12.0), color: self.theme.primary.alpha(if hovered && !carried { 0.2 } else { 0.1 }) });
                    back.x = x0 - 5.0;
                    back.y = box_y;
                    back.opacity = if carried { 0.3 } else { 1.0 };
                    layout.pieces.push(back);
                }
                let count = module.pieces.len();
                for (i, mut piece) in module.pieces.into_iter().enumerate() {
                    piece.x = x;
                    piece.y = (middle - piece.h / 2.0).round();
                    x += piece.w;
                    if i + 1 < count {
                        x += module.spacing;
                    }
                    if let Some(carry) = carry.filter(|_| carried) {
                        // It stays in its place, faint, and goes along under the pointer
                        let mut copy = piece.clone();
                        copy.key = format!("ghost.{}", piece.key);
                        copy.x = piece.x + (carry.x - carry.grab - x0);
                        copy.opacity = if carry.off { 0.45 } else { 1.0 };
                        copy.lift = false;
                        copy.actions = Default::default();
                        copy.label = None;
                        ghost.push(copy);
                        piece.opacity = 0.3;
                    }
                    if arrange.mode && item {
                        piece.lift = false;
                    }
                    layout.pieces.push(piece);
                }
                // In arrange mode a press anywhere on the item picks it up, and nothing under it
                // opens
                if arrange.mode && item && carry.is_none() {
                    let mut grab = Piece::new(format!("arrange.grab.{name}"), width + 10.0, t.bar_height);
                    grab.x = x0 - 5.0;
                    grab.y = t.line();
                    grab.blocks = true;
                    grab.label = Some(format!("{} · drag to move", crate::settings::bar_item(&name).0));
                    layout.pieces.push(grab);
                }
            }
        }
        // The carried item, on a lit card under the pointer, and a mark where it would land
        if let Some(carry) = carry {
            if let Some(span) = layout.placed.spans.iter().find(|s| s.name == carry.item) {
                let w = span.x1 - span.x0 + 16.0;
                let mut card = Piece::new("ghost.card", w, box_h);
                card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h: box_h, radius: (box_h / 2.0).min(12.0), color: t.primary });
                card.prims.push(Prim::Rect { x: 1.5, y: 1.5, w: w - 3.0, h: box_h - 3.0, radius: (box_h / 2.0).min(12.0) - 1.5, color: t.secondary_container });
                card.x = carry.x - carry.grab - 8.0;
                card.y = box_y;
                card.opacity = if carry.off { 0.45 } else { 1.0 };
                layout.pieces.push(card);
            }
            layout.pieces.append(&mut ghost);
            // Over the carried item, so it shows even right under it
            if !carry.off {
                let slot = layout.placed.slot(&carry.item, carry.x);
                let h = (t.bar_height * 0.6).round();
                let mut mark = Piece::new("arrange.mark", 3.0, h);
                mark.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: 3.0, h, radius: 1.5, color: t.primary });
                mark.x = (slot.x - 1.5).round();
                mark.y = (middle - h / 2.0).round();
                layout.pieces.push(mark);
            }
        }
        // The top-right corner opens the control centre; a star shows there under the pointer
        if self.config.hot_corner {
            let mut corner = Piece::new("corner.control", t.bar_side, t.bar_height);
            corner.x = self.width - t.bar_side;
            corner.y = t.line();
            corner.lift = true;
            corner.label = Some("Control centre".into());
            corner.actions.left = Some(Action::ToggleControl);
            layout.pieces.push(corner);
        }
        // And the top-left one, the day panel
        if self.config.day.corner {
            let mut corner = Piece::new("corner.day", t.bar_side, t.bar_height);
            corner.x = 0.0;
            corner.y = t.line();
            corner.lift = true;
            corner.label = Some("Your day".into());
            corner.actions.left = Some(Action::ToggleDay);
            layout.pieces.push(corner);
        }
        layout
    }
}

/// A workspace's number in kanji: 一 to 九, 十, 十一, 二十, 二十一… up to 99; past that, digits.
pub fn kanji(n: i64) -> String {
    const DIGITS: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    if !(1..=99).contains(&n) {
        return n.to_string();
    }
    let (tens, ones) = ((n / 10) as usize, (n % 10) as usize);
    let mut out = String::new();
    if tens > 1 {
        out.push_str(DIGITS[tens]);
    }
    if tens > 0 {
        out.push('十');
    }
    if ones > 0 {
        out.push_str(DIGITS[ones]);
    }
    out
}

#[cfg(test)]
mod kanji_tests {
    #[test]
    fn writes_kanji() {
        let k = super::kanji;
        assert_eq!([k(1), k(3), k(9), k(10), k(11), k(20), k(24), k(99)], ["一", "三", "九", "十", "十一", "二十", "二十四", "九十九"]);
        assert_eq!(k(0), "0");
        assert_eq!(k(120), "120");
    }
}

#[cfg(test)]
mod arrange_tests {
    use super::*;

    fn placed() -> Placed {
        let span = |name: &str, part, x0, x1| Span { name: name.into(), part, x0, x1 };
        Placed {
            spans: vec![span("launcher", 0, 10.0, 30.0), span("pages", 0, 40.0, 140.0), span("clock", 1, 900.0, 1000.0), span("status", 2, 1700.0, 1900.0)],
            anchors: [10.0, 960.0, 1910.0],
            gap: 10.0,
        }
    }

    #[test]
    fn a_drop_goes_to_the_nearest_gap() {
        let p = placed();
        // Between the two on the left
        assert_eq!(p.slot("clock", 36.0), Slot { part: 0, after: Some("launcher".into()), x: 35.0 });
        // Just past pages: the end of the left part, not the start of the middle
        assert_eq!(p.slot("clock", 200.0).after.as_deref(), Some("pages"));
        // Nearer the clock: before it, in the middle
        let near = p.slot("launcher", 850.0);
        assert_eq!((near.part, near.after), (1, None));
        // The carried item isn't a place of its own: dropping on it puts it back beside its neighbour
        assert_eq!(p.slot("pages", 90.0), Slot { part: 0, after: Some("launcher".into()), x: 35.0 });
    }

    #[test]
    fn an_empty_part_still_takes_an_item() {
        let mut p = placed();
        p.spans.retain(|s| s.part != 1);
        let slot = p.slot("status", 950.0);
        assert_eq!((slot.part, slot.after, slot.x), (1, None, 960.0));
        assert_eq!(p.at(20.0).map(|s| s.name.as_str()), Some("launcher"));
        assert!(p.at(500.0).is_none());
    }
}
