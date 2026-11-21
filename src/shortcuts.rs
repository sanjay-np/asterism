//! The keybind cheatsheet: your compositor's binds, read from its config (niri's `binds { }`,
//! Hyprland's `bind = …` lines), sorted into groups, and drawn as a full-screen sheet.

use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Piece, Prim};

#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    /// "Mod+Shift+Return"
    pub keys: String,
    pub what: String,
    pub group: Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Apps,
    Windows,
    Workspaces,
    Screens,
    Media,
    Other,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Apps => "Apps and shell",
            Group::Windows => "Windows and columns",
            Group::Workspaces => "Workspaces",
            Group::Screens => "Screens",
            Group::Media => "Sound, light and media",
            Group::Other => "Everything else",
        }
    }
}

fn group_of(keys: &str, action: &str) -> Group {
    if keys.contains("XF86") {
        return Group::Media;
    }
    let a = action.to_lowercase();
    if a.starts_with("screenshot") {
        Group::Other
    } else if a.starts_with("spawn") || a.starts_with("exec") {
        Group::Apps
    } else if a.contains("workspace") {
        Group::Workspaces
    } else if a.contains("monitor") || a.contains("output") {
        Group::Screens
    } else if ["column", "window", "fullscreen", "maximize", "floating", "consume", "expel", "killactive", "movefocus", "movewindow", "resize", "togglefloating", "pseudo", "togglesplit"].iter().any(|w| a.contains(w)) {
        Group::Windows
    } else {
        Group::Other
    }
}

/// What the usual media and hardware keys do, in words.
fn media_key(keys: &str) -> Option<&'static str> {
    Some(match keys.rsplit('+').next()? {
        "XF86AudioRaiseVolume" => "Volume up",
        "XF86AudioLowerVolume" => "Volume down",
        "XF86AudioMute" => "Mute the sound",
        "XF86AudioMicMute" => "Mute the microphone",
        "XF86AudioPlay" | "XF86AudioPause" => "Play or pause",
        "XF86AudioStop" => "Stop playing",
        "XF86AudioPrev" => "Previous track",
        "XF86AudioNext" => "Next track",
        "XF86MonBrightnessUp" => "Brighter",
        "XF86MonBrightnessDown" => "Dimmer",
        _ => return None,
    })
}

/// "focus-column-left" → "Focus column left"
fn humanize(action: &str) -> String {
    let words = action.replace(['-', '_'], " ");
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// What a spawn runs, for people: `spawn "/x/asterism" "msg" "launcher"` → "asterism msg launcher".
fn spawned(args: &str) -> String {
    let words: Vec<String> = args.split('"').skip(1).step_by(2).map(|w| w.rsplit('/').next().unwrap_or(w).to_string()).collect();
    if words.is_empty() { args.trim().to_string() } else { words.join(" ") }
}

/// The binds in a niri config.
pub fn niri(config: &str) -> Vec<Bind> {
    let mut binds = Vec::new();
    // The binds block, without comments; one bind may span lines
    let mut depth = 0;
    let mut inside = false;
    let mut pending = String::new();
    for raw in config.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if !inside {
            if line.starts_with("binds") && line.contains('{') {
                inside = true;
                depth = 1;
            }
            continue;
        }
        pending.push(' ');
        pending.push_str(line);
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth <= 0 {
            break;
        }
        if depth == 1 && pending.contains('}') {
            if let Some(bind) = niri_bind(pending.trim()) {
                binds.push(bind);
            }
            pending.clear();
        }
    }
    binds
}

fn niri_bind(text: &str) -> Option<Bind> {
    // Mod+T hotkey-overlay-title="Terminal" repeat=false { spawn "kitty"; }
    let (head, body) = text.split_once('{')?;
    let action = body.trim().trim_end_matches('}').trim().trim_end_matches(';').trim();
    let keys = head.split_whitespace().next()?.to_string();
    let title = head.split("hotkey-overlay-title=").nth(1).and_then(|rest| rest.trim().strip_prefix('"')).and_then(|rest| rest.split('"').next()).map(str::to_string);
    let (name, args) = action.split_once(char::is_whitespace).unwrap_or((action, ""));
    let what = match title {
        Some(title) => title,
        None if media_key(&keys).is_some() => media_key(&keys).unwrap_or_default().to_string(),
        None if name.starts_with("spawn") => spawned(args),
        None if args.is_empty() => humanize(name),
        None => format!("{} {}", humanize(name), args.trim_matches('"')),
    };
    Some(Bind { group: group_of(&keys, name), keys, what })
}

/// The binds in a Hyprland (hyprlang) config: `bind = SUPER SHIFT, Q, killactive,`.
pub fn hyprland(config: &str) -> Vec<Bind> {
    let mut binds = Vec::new();
    for raw in config.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        if !key.trim().starts_with("bind") {
            continue;
        }
        let parts: Vec<&str> = value.splitn(4, ',').map(str::trim).collect();
        if parts.len() < 3 {
            continue;
        }
        let mods: Vec<&str> = parts[0].split_whitespace().collect();
        let keys = mods.iter().chain(std::iter::once(&parts[1])).filter(|k| !k.is_empty()).copied().collect::<Vec<_>>().join("+");
        let (action, args) = (parts[2], parts.get(3).copied().unwrap_or(""));
        let what = if let Some(media) = media_key(&keys) { media.to_string() } else if action == "exec" { args.to_string() } else if args.is_empty() { humanize(action) } else { format!("{} {args}", humanize(action)) };
        binds.push(Bind { group: group_of(&keys, action), keys, what });
    }
    binds
}

/// Your binds, from whichever compositor runs.
pub fn load() -> Vec<Bind> {
    let home = crate::config::home();
    let config_dir = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let on_niri = std::env::var_os("NIRI_SOCKET").is_some() || config_dir.join("niri/config.kdl").is_file() && std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none();
    if on_niri && let Ok(text) = std::fs::read_to_string(config_dir.join("niri/config.kdl")) {
        return niri(&text);
    }
    std::fs::read_to_string(config_dir.join("hypr/hyprland.conf")).map(|t| hyprland(&t)).unwrap_or_default()
}

/// "Mod+Shift+Return" → ["Super", "Shift", "Return"]
pub fn keycaps(keys: &str) -> Vec<String> {
    keys.split('+')
        .filter(|k| !k.is_empty())
        .map(|k| match k {
            "Mod" | "SUPER" | "Super" => "Super".to_string(),
            "Ctrl" | "CTRL" | "Control" => "Ctrl".to_string(),
            "SHIFT" => "Shift".to_string(),
            "ALT" => "Alt".to_string(),
            "Return" => "Enter".to_string(),
            "Slash" => "/".to_string(),
            "Comma" => ",".to_string(),
            "Period" => ".".to_string(),
            "Minus" => "−".to_string(),
            "Equal" => "=".to_string(),
            "WheelScrollDown" => "Wheel ↓".to_string(),
            "WheelScrollUp" => "Wheel ↑".to_string(),
            "XF86AudioRaiseVolume" => "Vol +".to_string(),
            "XF86AudioLowerVolume" => "Vol −".to_string(),
            "XF86AudioMicMute" => "Mic".to_string(),
            "XF86MonBrightnessUp" => "Bright +".to_string(),
            "XF86MonBrightnessDown" => "Bright −".to_string(),
            other => other.strip_prefix("XF86").map(|k| k.replace("Audio", "")).unwrap_or_else(|| other.to_string()),
        })
        .collect()
}

// ---------------------------------------------------------------- the sheet

pub struct Sheet {
    pub binds: Vec<Bind>,
    pub query: String,
}

impl Sheet {
    pub fn new() -> Self {
        Self { binds: load(), query: String::new() }
    }

    /// Columns of groups, filling the screen; what's typed narrows them.
    pub fn build(&self, t: &Theme, text: &mut Text, w: f32, h: f32) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let span = (w * 0.86).round();
        let left = ((w - span) / 2.0).round();
        let top = (h * 0.06).round().max(24.0);
        let title_font = Font::new(t.text + 9.0).weight(500);
        let small = Font::new(t.text - 1.0);
        let (tw, th) = text.measure("Keyboard shortcuts", title_font);
        let mut title = Piece::new("keys.title", tw, th);
        title.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Keyboard shortcuts".into(), font: title_font, color: t.color_text });
        title.x = left;
        title.y = top;
        title.blocks = true;
        pieces.push(title);
        let about = if self.query.is_empty() { "Type to narrow them · Esc closes".to_string() } else { format!("Showing ones about “{}” · Backspace widens · Esc closes", self.query) };
        let (aw, ah) = text.measure(&about, small);
        let mut sub = Piece::new("keys.about", aw, ah);
        sub.prims.push(Prim::Text { x: 0.0, y: 0.0, text: about, font: small, color: t.color_soft });
        sub.x = left;
        sub.y = top + th;
        sub.blocks = true;
        pieces.push(sub);

        let wanted = self.query.to_lowercase();
        let shown: Vec<&Bind> = self.binds.iter().filter(|b| wanted.is_empty() || b.what.to_lowercase().contains(&wanted) || b.keys.to_lowercase().contains(&wanted)).collect();
        let mut groups: Vec<(Group, Vec<&Bind>)> = Vec::new();
        for bind in shown {
            match groups.iter_mut().find(|(g, _)| *g == bind.group) {
                Some((_, list)) => list.push(bind),
                None => groups.push((bind.group, vec![bind])),
            }
        }
        groups.sort_by_key(|(g, _)| *g);

        let columns = ((span / 380.0).floor() as usize).max(1);
        let gap = 28.0;
        let col_w = ((span - gap * (columns - 1) as f32) / columns as f32).floor();
        let row_h = 30.0;
        let label_h = 30.0;
        let grid_top = top + th + ah + 28.0;
        let bottom = h - 24.0;
        let mut heights = vec![0.0f32; columns];
        let mut hidden = 0;
        let key_font = Font::new(t.text - 2.0).weight(500);
        let what_font = Font::new(t.text - 1.0);
        for (group, list) in groups {
            // The shortest column takes the next group, as much of it as fits
            let column = (0..columns).min_by(|a, b| heights[*a].total_cmp(&heights[*b])).unwrap_or(0);
            let x = left + column as f32 * (col_w + gap);
            let mut y = grid_top + heights[column];
            if y + label_h + row_h > bottom {
                hidden += list.len();
                continue;
            }
            let name = group.name().to_uppercase();
            let label_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
            let (lw, lh) = text.measure(&name, label_font);
            let mut label = Piece::new(format!("keys.group.{group:?}"), lw, lh);
            label.prims.push(Prim::Text { x: 0.0, y: 0.0, text: name, font: label_font, color: t.primary });
            label.x = x;
            label.y = y + 6.0;
            pieces.push(label);
            y += label_h;
            for (i, bind) in list.iter().enumerate() {
                if y + row_h > bottom {
                    hidden += list.len() - i;
                    break;
                }
                let mut row = Piece::new(format!("keys.{group:?}.{i}"), col_w, row_h);
                // Keycaps on the left, outlined
                let mut kx = 0.0;
                for cap in keycaps(&bind.keys) {
                    let (cw, ch) = text.measure(&cap, key_font);
                    let (bw, bh) = (cw + 12.0, 22.0);
                    row.prims.push(Prim::Rect { x: kx, y: (row_h - bh) / 2.0, w: bw, h: bh, radius: 6.0, color: t.hairline() });
                    row.prims.push(Prim::Rect { x: kx + 1.0, y: (row_h - bh) / 2.0 + 1.0, w: bw - 2.0, h: bh - 2.0, radius: 5.0, color: t.container_high });
                    row.prims.push(Prim::Text { x: kx + 6.0, y: ((row_h - ch) / 2.0).round(), text: cap, font: key_font, color: t.color_text });
                    kx += bw + 4.0;
                }
                let room = col_w - kx - 10.0;
                let what = crate::bar::Ctx::ellipsize_with(text, &bind.what, what_font, room.max(20.0));
                let (_, wh) = text.measure(&what, what_font);
                row.prims.push(Prim::Text { x: kx + 6.0, y: ((row_h - wh) / 2.0).round(), text: what, font: what_font, color: t.color_soft });
                row.x = x;
                row.y = y;
                pieces.push(row);
                y += row_h;
            }
            heights[column] = y - grid_top + 14.0;
        }
        if hidden > 0 || self.binds.is_empty() {
            let note = if self.binds.is_empty() { "No shortcuts found in your compositor's config".to_string() } else { format!("{hidden} more: type to narrow them") };
            let (nw, nh) = text.measure(&note, small);
            let mut more = Piece::new("keys.more", nw, nh);
            more.prims.push(Prim::Text { x: 0.0, y: 0.0, text: note, font: small, color: t.color_soft });
            more.x = ((w - nw) / 2.0).round();
            more.y = (h - 20.0 - nh).round();
            pieces.push(more);
        }
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_niri_binds() {
        let config = r#"
input { keyboard { } }
binds {
    // Mod+Q { nothing; }
    Mod+Return hotkey-overlay-title="Open a Terminal: kitty" { spawn "kitty"; }
    Mod+Space { spawn "/home/u/.local/bin/asterism" "msg" "launcher"; }
    Mod+Left  { focus-column-left; }
    Mod+1 { focus-workspace 1; }
    Mod+Shift+Ctrl+Left { move-column-to-monitor-left; }
    XF86AudioMute allow-when-locked=true { spawn-sh "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"; }
    Mod+Shift+E {
        quit;
    }
}
window-rule { }
"#;
        let binds = niri(config);
        let pairs: Vec<(&str, &str, Group)> = binds.iter().map(|b| (b.keys.as_str(), b.what.as_str(), b.group)).collect();
        assert_eq!(
            pairs,
            [
                ("Mod+Return", "Open a Terminal: kitty", Group::Apps),
                ("Mod+Space", "asterism msg launcher", Group::Apps),
                ("Mod+Left", "Focus column left", Group::Windows),
                ("Mod+1", "Focus workspace 1", Group::Workspaces),
                ("Mod+Shift+Ctrl+Left", "Move column to monitor left", Group::Screens),
                ("XF86AudioMute", "Mute the sound", Group::Media),
                ("Mod+Shift+E", "Quit", Group::Other),
            ]
        );
        assert_eq!(keycaps("Mod+Shift+Return"), ["Super", "Shift", "Enter"]);
    }

    #[test]
    fn reads_hyprland_binds() {
        let binds = hyprland("bind = SUPER SHIFT, Q, killactive,\nbind = SUPER, SPACE, exec, asterism msg launcher # apps\n$mod = SUPER\n");
        assert_eq!(binds.len(), 2);
        assert_eq!((binds[0].keys.as_str(), binds[0].group), ("SUPER+SHIFT+Q", Group::Windows));
        assert_eq!(binds[1].what, "asterism msg launcher");
    }
}
