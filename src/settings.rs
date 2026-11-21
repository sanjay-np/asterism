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
    /// Typed in place; what's written now.
    Text(String),
    Button(&'static str, Action),
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

fn choice(options: &[(&str, &str)], current: &str) -> Control {
    let list: Vec<(String, Value)> = options.iter().map(|(label, value)| (label.to_string(), Value::Str(value.to_string()))).collect();
    let at = options.iter().position(|(_, v)| v.eq_ignore_ascii_case(current));
    Control::Choice(list, at)
}

pub const PAGES: [&str; 9] = ["Look", "Bar", "Wallpaper", "Desktop", "Night light", "Lock", "Notifications", "Weather", "Timers"];
const PAGE_ICONS: [&str; 9] = ["picker", "layout", "image", "apps", "nightlight", "lock", "bell", "cloud-sun", "timer"];

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

/// What a page holds, from the config as it is now.
pub fn rows(page: usize, c: &Config, model: &Model) -> Vec<Row> {
    let lower = |s: &str| s.to_lowercase();
    match page {
        0 => vec![
            row(&["style"], "Bar style", "A slim line, a frame round the screen, or floating", choice(&[("Line", "line"), ("Frame", "frame"), ("Floating", "floating"), ("Islands", "islands"), ("Bare", "transparent")], &lower(&format!("{:?}", c.style)))),
            row(&["size"], "Size", "", choice(&[("Compact", "compact"), ("Normal", "normal"), ("Large", "large")], &lower(&format!("{:?}", c.size)))),
            row(&["position"], "Where the bar sits", "", choice(&[("Top", "top"), ("Bottom", "bottom")], &lower(&format!("{:?}", c.position)))),
            row(&["icons"], "Icons", "Thin outlines, rounded, or filled", choice(&[("Outline", "outline"), ("Material", "material"), ("Phosphor", "phosphor")], &c.icons)),
            row(&["page_style"], "Workspaces", "Dots (busy ones joined by a line), numbers, or kanji (一 二 三)", choice(&[("Dots", "dots"), ("Numbers", "numbers"), ("Kanji", "kanji")], &lower(&format!("{:?}", c.page_style)))),
            row(
                &["colors", "bar_icons"],
                "Bar icons",
                "In the wallpaper's colours, or plain",
                choice(&[("Tinted", "tinted"), ("Accent", "accent"), ("Colourful", "colourful"), ("Plain", "plain")], if c.colors.bar_icons.trim().is_empty() { "tinted" } else { c.colors.bar_icons.trim() }),
            ),
            row(&["colors", "widgets"], "Widgets", "Clock, headings and cards in the wallpaper's colours, or plain", choice(&[("Tinted", "tinted"), ("Plain", "plain")], if c.colors.widgets.trim().is_empty() { "tinted" } else { c.colors.widgets.trim() })),
            in_list(&["colors", "apps"], &c.colors.apps, "kitty", "Colours for kitty", "Add to kitty.conf: include asterism-colors.conf"),
            in_list(&["colors", "apps"], &c.colors.apps, "foot", "Colours for foot", "Add to foot.ini: include=~/.config/foot/asterism-colors.ini"),
            in_list(&["colors", "apps"], &c.colors.apps, "alacritty", "Colours for Alacritty", "Import ~/.config/alacritty/asterism-colors.toml"),
            in_list(&["colors", "apps"], &c.colors.apps, "gtk", "Colours for GTK apps", "Add to gtk.css: @import url(\"asterism-colors.css\");"),
        ],
        1 => vec![
            row(&["percent"], "Numbers beside icons", "Volume, battery and the machine's load as percentages", Control::Switch(c.percent)),
            row(&["media_title"], "The song's title", "Slides by when long, or is cut short", choice(&[("Slide", "scroll"), ("Short", "short"), ("Off", "off")], &lower(&format!("{:?}", c.media_title)))),
            row(&["clock", "hours24"], "24-hour clock", "", Control::Switch(c.clock.hours24)),
            row(&["clock", "seconds"], "Seconds", "", Control::Switch(c.clock.seconds)),
            row(&["clock", "date"], "Date beside the time", "", choice(&[("Saturday, November 21", "%A, %B %-d"), ("Sat 21 Nov", "%a %-d %b"), ("21/11/2026", "%d/%m/%Y"), ("None", "")], &c.clock.date)),
            row(&["hot_corner"], "Hot corner", "The top-right corner opens the control centre", Control::Switch(c.hot_corner)),
            row(&["song_popup"], "New song card", "Its cover and name for a moment, when the song changes", Control::Switch(c.song_popup)),
        ],
        2 => vec![
            row(&[], "Choose a picture", "Your wallpapers as a grid; the colours follow the picture", Control::Button("Open", Action::ToggleWallpapers)),
            row(&["wallpaper", "interval"], "Change by itself", "Minutes between pictures from the folder, 0 never", Control::Stepper { value: c.wallpaper.interval as i64, min: 0, max: 240, step: 5, unit: "min" }),
            row(
                &["wallpaper", "transition"],
                "Animation",
                "How one picture gives way to the next; Random is a different one each time",
                choice(&[("Random", "random"), ("Fade", "fade"), ("Wipe", "wipe"), ("Circle", "circle"), ("Blinds", "blinds"), ("Ink", "ink"), ("Pixels", "pixels"), ("Slide", "slide"), ("None", "none")], &c.wallpaper.transition),
            ),
            row(&["wallpaper", "transition_ms"], "Animation length", "", Control::Stepper { value: c.wallpaper.transition_ms as i64, min: 400, max: 3000, step: 200, unit: "ms" }),
            row(&["wallpaper", "fit"], "Fit", "", choice(&[("Cover", "cover"), ("Contain", "contain"), ("Stretch", "stretch")], &lower(&format!("{:?}", c.wallpaper.fit)))),
            row(&["wallpaper", "enabled"], "Draw the wallpaper", "Off leaves it to another program", Control::Switch(c.wallpaper.enabled)),
        ],
        3 => {
            let mut list = vec![
                row(&[], "Arrange widgets", "Add, take off, move and resize them, on the desktop itself", Control::Button("Arrange", Action::EditWidgets(Some(true)))),
                row(&["widgets", "note"], "Sticky note", "What the note on the desktop says", Control::Text(c.widgets.note.clone())),
            ];
            for kind in crate::widgets::KINDS {
                let about = match kind {
                    "media" => "The song, its cover and its progress",
                    "visualizer" => "Bars that move with the music, while something plays",
                    "edge" => "The same bars along the foot of the screen",
                    "system" => "Processor, memory and disk",
                    "photos" => "A picture from a folder, a new one every so often",
                    _ => "",
                };
                list.push(in_list(&["widgets", "show"], &c.widgets.show, kind, crate::widgets::name(kind), about));
            }
            list.push(row(&["widgets", "lyrics"], "Song words", "Under what's playing, line by line (asks LRCLIB with the song's name)", Control::Switch(c.widgets.lyrics)));
            list.push(row(&["widgets", "photos"], "Photo folder", "Empty takes ~/Pictures", Control::Text(c.widgets.photos.clone())));
            list.push(row(&["widgets", "photo_minutes"], "Each picture stays", "", Control::Stepper { value: c.widgets.photo_minutes as i64, min: 1, max: 120, step: 1, unit: "min" }));
            list
        }
        4 => vec![
            row(&["night_light", "mode"], "Night light", "Warmer colours: always, or from evening to morning", choice(&[("Off", "off"), ("On", "on"), ("Schedule", "schedule")], &c.night_light.mode)),
            row(&["night_light", "temperature"], "Warmth", "6500 K is the screen as it is", Control::Stepper { value: c.night_light.temperature as i64, min: 2500, max: 6000, step: 250, unit: "K" }),
            row(&["night_light", "start"], "From", "", choice(&[("Sunset", "sunset"), ("19:00", "19:00"), ("20:00", "20:00"), ("21:00", "21:00"), ("22:00", "22:00")], &c.night_light.start)),
            row(&["night_light", "end"], "Until", "", choice(&[("Sunrise", "sunrise"), ("06:00", "06:00"), ("07:00", "07:00"), ("08:00", "08:00")], &c.night_light.end)),
        ],
        5 => vec![
            row(&["lock", "idle"], "Lock when idle", "Minutes without input, 0 never; Stay awake holds it off", Control::Stepper { value: c.lock.idle as i64, min: 0, max: 120, step: 5, unit: "min" }),
            row(&["lock", "before_sleep"], "Lock before sleep", "Closing the lid, or suspend from anywhere", Control::Switch(c.lock.before_sleep)),
            row(&[], "Lock now", "", Control::Button("Lock", Action::LockNow)),
        ],
        6 => {
            let mut list = vec![
                row(&["notifications", "timeout"], "Pop-ups stay", "Seconds, when the app doesn't say", Control::Stepper { value: c.notifications.timeout as i64, min: 2, max: 30, step: 1, unit: "s" }),
                row(&["notifications", "max_popups"], "Pop-ups at once", "", Control::Stepper { value: c.notifications.max_popups as i64, min: 1, max: 8, step: 1, unit: "" }),
                row(&["notifications", "battery_low"], "Battery low at", "0 never", Control::Stepper { value: c.notifications.battery_low as i64, min: 0, max: 50, step: 5, unit: "%" }),
                row(&["notifications", "battery_critical"], "Battery critical at", "0 never", Control::Stepper { value: c.notifications.battery_critical as i64, min: 0, max: 20, step: 1, unit: "%" }),
            ];
            // Every app that has notified, and every app with a rule
            let mut apps: Vec<String> = c.notifications.apps.keys().cloned().collect();
            for n in &model.notifications.list {
                if !n.app.is_empty() && !apps.iter().any(|a| a.eq_ignore_ascii_case(&n.app)) {
                    apps.push(n.app.clone());
                }
            }
            apps.sort_by_key(|a| a.to_lowercase());
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
                    about: "Pop up, stay in the centre quietly, or be dropped".into(),
                    control: Control::Choice(vec![("Pop up".into(), Value::Remove), ("Quiet".into(), Value::Str("silence".into())), ("Drop".into(), Value::Str("discard".into()))], Some(at)),
                });
            }
            list
        }
        7 => vec![
            row(&["weather", "enabled"], "Weather", "From Open-Meteo: only the place is sent", Control::Switch(c.weather.enabled)),
            row(&["weather", "city"], "Place", "Empty takes your time zone's city", Control::Text(c.weather.city.clone())),
            row(&["weather", "units"], "Units", "", choice(&[("°C, km/h", "metric"), ("°F, mph", "imperial")], &c.weather.units)),
        ],
        _ => vec![
            row(&["timers", "focus"], "Focus", "", Control::Stepper { value: c.timers.focus as i64, min: 5, max: 120, step: 5, unit: "min" }),
            row(&["timers", "short_break"], "Short break", "", Control::Stepper { value: c.timers.short_break as i64, min: 1, max: 30, step: 1, unit: "min" }),
            row(&["timers", "long_break"], "Long break", "", Control::Stepper { value: c.timers.long_break as i64, min: 5, max: 60, step: 5, unit: "min" }),
            row(&["timers", "rounds"], "Long break after", "Rounds of focus", Control::Stepper { value: c.timers.rounds as i64, min: 2, max: 8, step: 1, unit: "" }),
        ],
    }
}

/// Writes one value into the config file, keeping everything else as it was.
pub fn write(path: &Path, keys: &[String], value: &Value) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse()?;
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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

pub struct Settings {
    pub page: usize,
    /// A text being typed: where it goes, and what's typed so far.
    pub editing: Option<(Vec<String>, String)>,
    /// How far the page is scrolled, and how far it can be.
    pub scroll: f32,
    pub max_scroll: f32,
}

/// How wide a row's control is.
fn control_width(control: &Control, text: &mut Text, font: Font) -> f32 {
    let pill = |label: &str, text: &mut Text| text.measure(label, font).0 + 26.0;
    match control {
        Control::Switch(_) | Control::SwitchTo(..) => 44.0,
        Control::Choice(options, _) => options.iter().map(|(label, _)| pill(label, text)).sum::<f32>() + 8.0 * options.len().saturating_sub(1) as f32,
        Control::Stepper { .. } => pill("+", text) + 92.0 + pill("−", text),
        Control::Text(_) => 240.0,
        Control::Button(label, _) => pill(label, text),
    }
}

impl Settings {
    pub fn new() -> Self {
        Self { page: 0, editing: None, scroll: 0.0, max_scroll: 0.0 }
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
        let pages_end = 80.0 + 44.0 * PAGES.len() as f32;
        let note = if h - 24.0 - nh * note.len() as f32 > pages_end + 16.0 { note } else { Vec::new() };
        for (i, line) in note.iter().rev().enumerate() {
            side.prims.push(Prim::Text { x: 24.0, y: h - 24.0 - nh * (i + 1) as f32, text: line.clone(), font: note_font, color: t.color_soft.alpha(0.75) });
        }
        side.actions.left = Some(Action::MoveWindow);
        pieces.push(side);
        let nav_font = Font::new(t.text + 1.0);
        let mut y = 80.0;
        for (i, name) in PAGES.iter().enumerate() {
            let (iw, ih) = (side_w - 24.0, 40.0);
            let mut item = Piece::new(format!("set.page.{i}"), iw, ih);
            let on = i == self.page;
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
        let card_index = pieces.len();
        let top = head_h + 4.0 - self.scroll;
        let mut y = top + 6.0;
        let rows = rows(self.page, c, model);
        let count = rows.len();
        for (r, row) in rows.into_iter().enumerate() {
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
            let row_h = if stacked { 104.0 + 40.0 * lines.len().saturating_sub(1) as f32 } else { 66.0 };
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
            if r + 1 < count {
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
                Control::Text(current) => {
                    let editing = self.editing.as_ref().filter(|(k, _)| *k == row.keys).map(|(_, typed)| typed.clone());
                    let (fw, fh) = (240.0, 34.0);
                    let mut f = Piece::new(format!("set.text.{r}"), fw, fh);
                    f.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: fh, radius: fh / 2.0, color: if editing.is_some() { t.primary } else { t.hairline() } });
                    f.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: fh - 2.0, radius: fh / 2.0 - 1.0, color: t.surface });
                    let shown = editing.clone().unwrap_or_else(|| if current.is_empty() { "Not set".into() } else { current.clone() });
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
            }
            y += row_h;
        }
        let card_h = y + 6.0 - top;
        let mut card = Piece::new("set.card", cw, card_h);
        card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: cw, h: card_h, radius: t.radius, color: t.hairline() });
        card.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: cw - 2.0, h: card_h - 2.0, radius: t.radius - 1.0, color: t.card_deep });
        card.x = cx;
        card.y = top;
        pieces.insert(card_index, card);
        // How far it can scroll: the card's foot 32 px off the window's
        self.max_scroll = (card_h + head_h + 4.0 + 32.0 - h).max(0.0);
        if self.scroll > self.max_scroll {
            self.scroll = self.max_scroll;
        }
        if let Some(hint) = self.editing.as_ref().map(|_| "Type, then Enter to keep it · Esc lets it be") {
            let (hw, hh) = text.measure(hint, about_font);
            let mut under = Piece::new("set.hint", hw, hh);
            under.prims.push(Prim::Text { x: 0.0, y: 0.0, text: hint.into(), font: about_font, color: t.color_soft });
            under.x = cx + cw - hw;
            under.y = top + card_h + 10.0;
            pieces.push(under);
        }

        // The page's title, over rows scrolled under it; held, it moves the window
        let mut head = Piece::new("set.head", room - 1.0, head_h);
        head.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: room - 1.0, h: head_h, radius: 0.0, color: t.surface });
        let head_font = Font::new(t.text + 11.0).weight(400);
        let (_, hh) = text.measure("Ag", head_font);
        head.prims.push(Prim::Text { x: pad, y: ((head_h - hh) / 2.0).round() + 4.0, text: PAGES[self.page].into(), font: head_font, color: t.color_text });
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
