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

/// What a page holds, from the config as it is now.
pub fn rows(page: usize, c: &Config, model: &Model) -> Vec<Row> {
    let lower = |s: &str| s.to_lowercase();
    match page {
        0 => vec![
            row(&["style"], "Bar style", "A slim line, a frame round the screen, or floating", choice(&[("Line", "line"), ("Frame", "frame"), ("Floating", "floating"), ("Islands", "islands"), ("Bare", "transparent")], &lower(&format!("{:?}", c.style)))),
            row(&["size"], "Size", "", choice(&[("Compact", "compact"), ("Normal", "normal"), ("Large", "large")], &lower(&format!("{:?}", c.size)))),
            row(&["position"], "Where the bar sits", "", choice(&[("Top", "top"), ("Bottom", "bottom")], &lower(&format!("{:?}", c.position)))),
            row(&["icons"], "Icons", "Thin outlines, rounded, or filled", choice(&[("Outline", "outline"), ("Material", "material"), ("Phosphor", "phosphor")], &c.icons)),
            row(&["page_style"], "Workspaces", "Joined dots, numbers, or kanji (一 二 三)", choice(&[("Dots", "dots"), ("Numbers", "numbers"), ("Kanji", "kanji")], &lower(&format!("{:?}", c.page_style)))),
        ],
        1 => vec![
            row(&["percent"], "Numbers beside icons", "Volume, battery and the machine's load as percentages", Control::Switch(c.percent)),
            row(&["media_title"], "The song's title", "Slides by when long, or is cut short", choice(&[("Slide", "scroll"), ("Short", "short"), ("Off", "off")], &lower(&format!("{:?}", c.media_title)))),
            row(&["clock", "hours24"], "24-hour clock", "", Control::Switch(c.clock.hours24)),
            row(&["clock", "seconds"], "Seconds", "", Control::Switch(c.clock.seconds)),
            row(&["clock", "date"], "Date beside the time", "", choice(&[("Saturday, November 21", "%A, %B %-d"), ("Sat 21 Nov", "%a %-d %b"), ("21/11/2026", "%d/%m/%Y"), ("None", "")], &c.clock.date)),
            row(&["hot_corner"], "Hot corner", "The top-right corner opens the control centre", Control::Switch(c.hot_corner)),
        ],
        2 => vec![
            row(&[], "Choose a picture", "Your wallpapers as a grid; the colours follow the picture", Control::Button("Open", Action::ToggleWallpapers)),
            row(&["wallpaper", "interval"], "Change by itself", "Minutes between pictures, 0 never", Control::Stepper { value: c.wallpaper.interval as i64, min: 0, max: 240, step: 5, unit: "min" }),
            row(&["wallpaper", "fit"], "Fit", "", choice(&[("Cover", "cover"), ("Contain", "contain"), ("Stretch", "stretch")], &lower(&format!("{:?}", c.wallpaper.fit)))),
            row(&["wallpaper", "enabled"], "Draw the wallpaper", "Off leaves it to another program", Control::Switch(c.wallpaper.enabled)),
        ],
        3 => {
            let mut list: Vec<Row> = crate::widgets::KINDS
                .iter()
                .map(|kind| {
                    let on = c.widgets.show.iter().any(|k| k == kind);
                    let mut show: Vec<String> = c.widgets.show.clone();
                    if on {
                        show.retain(|k| k != kind);
                    } else {
                        show.push(kind.to_string());
                    }
                    let label = match *kind {
                        "clock" => "Clock",
                        "weather" => "Weather",
                        "media" => "What's playing",
                        "calendar" => "Calendar",
                        _ => "The machine",
                    };
                    Row { keys: vec!["widgets".into(), "show".into()], label: label.into(), about: String::new(), control: Control::Choice(vec![("Off".into(), Value::List(show.clone())), ("On".into(), Value::List(show))], Some(on as usize)) }
                })
                .collect();
            list.push(row(&["widgets", "lyrics"], "Song words", "Under what's playing, line by line (asks LRCLIB with the song's name)", Control::Switch(c.widgets.lyrics)));
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
}

impl Settings {
    pub fn new() -> Self {
        Self { page: 0, editing: None }
    }

    pub fn build(&self, c: &Config, model: &Model, t: &Theme, text: &mut Text, w: f32, h: f32) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let span = (w * 0.8).min(1100.0).round();
        let left = ((w - span) / 2.0).round();
        let top = (h * 0.08).round().max(28.0);
        let title_font = Font::new(t.text + 9.0).weight(500);
        let (tw, th) = text.measure("Settings", title_font);
        let mut title = Piece::new("set.title", tw, th);
        title.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Settings".into(), font: title_font, color: t.color_text });
        title.x = left;
        title.y = top;
        title.blocks = true;
        pieces.push(title);

        // The pages, down the left
        let nav_w = 210.0;
        let mut y = top + th + 24.0;
        let nav_font = Font::new(t.text + 1.0);
        for (i, name) in PAGES.iter().enumerate() {
            let mut item = Piece::new(format!("set.page.{i}"), nav_w, 40.0);
            let on = i == self.page;
            if on {
                item.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: nav_w, h: 40.0, radius: 20.0, color: t.secondary_container });
            }
            let (_, nh) = text.measure(name, nav_font);
            item.prims.push(Prim::Text { x: 18.0, y: ((40.0 - nh) / 2.0).round(), text: name.to_string(), font: nav_font, color: if on { t.on_secondary_container } else { t.color_soft } });
            item.highlight = !on;
            item.actions.left = Some(Action::SettingsPage(i));
            item.x = left;
            item.y = y;
            pieces.push(item);
            y += 44.0;
        }

        // The page itself
        let cx = left + nav_w + 40.0;
        let cw = span - nav_w - 40.0;
        let mut y = top + th + 24.0;
        let label_font = Font::new(t.text + 1.0).weight(500);
        let about_font = Font::new(t.text - 2.0);
        let control_font = Font::new(t.text - 1.0).weight(500);
        for (r, row) in rows(self.page, c, model).into_iter().enumerate() {
            let row_h = 66.0;
            let mut line = Piece::new(format!("set.row.{r}"), cw, row_h);
            let (_, lh) = text.measure("Ag", label_font);
            let (_, ah) = text.measure("Ag", about_font);
            let top_y = if row.about.is_empty() { (row_h - lh) / 2.0 } else { (row_h - lh - ah) / 2.0 };
            let label = crate::bar::Ctx::ellipsize_with(text, &row.label, label_font, cw * 0.45);
            line.prims.push(Prim::Text { x: 0.0, y: top_y, text: label, font: label_font, color: t.color_text });
            if !row.about.is_empty() {
                let about = crate::bar::Ctx::ellipsize_with(text, &row.about, about_font, cw * 0.45);
                line.prims.push(Prim::Text { x: 0.0, y: top_y + lh, text: about, font: about_font, color: t.color_soft });
            }
            line.prims.push(Prim::Line { points: vec![(0.0, row_h - 0.5), (cw, row_h - 0.5)], width: 1.0, color: t.outline_variant.alpha(0.5) });
            line.x = cx;
            line.y = y;
            pieces.push(line);

            // The control, at the right
            let right = cx + cw;
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
                p.y = y + (row_h - bh) / 2.0;
                pieces.push(p);
                bw
            };
            let set = |value: Value| Some(Action::SetSetting(row.keys.clone(), value));
            match row.control {
                Control::Switch(on) => {
                    let (sw, sh) = (44.0, 24.0);
                    let mut s = Piece::new(format!("set.switch.{r}"), sw, sh);
                    s.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: sw, h: sh, radius: sh / 2.0, color: if on { t.primary } else { t.outline_variant } });
                    let k = 16.0;
                    let kx = if on { sw - k - 4.0 } else { 4.0 };
                    s.prims.push(Prim::Rect { x: kx, y: (sh - k) / 2.0, w: k, h: k, radius: k / 2.0, color: if on { t.on_primary } else { t.color_soft } });
                    s.highlight = true;
                    s.actions.left = set(Value::Bool(!on));
                    s.x = right - sw;
                    s.y = y + (row_h - sh) / 2.0;
                    pieces.push(s);
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
                    v.y = y + (row_h - 32.0) / 2.0;
                    pieces.push(v);
                    pill(&mut pieces, format!("set.minus.{r}"), "−", right - plus - 92.0, false, (value > min).then(|| Action::SetSetting(row.keys.clone(), Value::Int((value - step).max(min)))), text);
                }
                Control::Text(current) => {
                    let editing = self.editing.as_ref().filter(|(k, _)| *k == row.keys).map(|(_, typed)| typed.clone());
                    let (fw, fh) = (240.0, 34.0);
                    let mut f = Piece::new(format!("set.text.{r}"), fw, fh);
                    f.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: fh, radius: fh / 2.0, color: if editing.is_some() { t.primary } else { t.hairline() } });
                    f.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: fh - 2.0, radius: fh / 2.0 - 1.0, color: t.card_deep });
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
                    f.y = y + (row_h - fh) / 2.0;
                    pieces.push(f);
                }
                Control::Button(label, action) => {
                    pill(&mut pieces, format!("set.button.{r}"), label, right, true, Some(action), text);
                }
            }
            y += row_h;
        }
        let hint = if self.editing.is_some() { "Type, then Enter to keep it · Esc lets it be" } else { "Changes are saved to config.toml as you make them · Esc closes" };
        let font = Font::new(t.text - 2.0);
        let (hw, hh) = text.measure(hint, font);
        let mut footer = Piece::new("set.footer", hw, hh);
        footer.prims.push(Prim::Text { x: 0.0, y: 0.0, text: hint.into(), font, color: t.color_soft.alpha(0.7) });
        footer.x = ((w - hw) / 2.0).round();
        footer.y = (h - 22.0 - hh).round();
        pieces.push(footer);
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
