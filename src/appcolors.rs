//! The scheme for other apps: terminal colours and GTK's named colours, written as files of
//! their own that the app's config includes (`[colors] apps`). Nothing of yours is edited; each
//! file is rewritten only when the colours change, and kitty is asked to read its config again.
//!
//! The sixteen terminal colours keep their usual meanings (red is still red) but lean a little
//! toward the wallpaper's colour, at tones that read on the scheme's dark surface.

use crate::palette::Palette;
use crate::theme::Rgba;
use material_colors::color::Rgb;
use material_colors::hct::Hct;
use std::path::PathBuf;

/// Red, green, yellow, blue, magenta, cyan: the hues a terminal's colours stand for.
const HUES: [(u8, u8, u8); 6] = [(0xe0, 0x4a, 0x4a), (0x4c, 0xb0, 0x5a), (0xd8, 0xb0, 0x3c), (0x4a, 0x7c, 0xe0), (0xc0, 0x5c, 0xc8), (0x3c, 0xb4, 0xc0)];

fn rgb(c: Rgba) -> Rgb {
    Rgb::new((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8)
}

fn rgba(c: Rgb) -> Rgba {
    Rgba::rgb(c.red, c.green, c.blue)
}

fn hex(c: Rgba) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

/// A terminal's colours: the sixteen, and the ones round them.
pub struct Terminal {
    pub background: Rgba,
    pub foreground: Rgba,
    pub cursor: Rgba,
    pub selection_background: Rgba,
    pub selection_foreground: Rgba,
    pub ansi: [Rgba; 16],
}

pub fn terminal(p: &Palette) -> Terminal {
    let source = rgb(p.source);
    let tinted = |(r, g, b): (u8, u8, u8), tone: f64| -> Rgba {
        let leaned: Hct = material_colors::blend::harmonize(Rgb::new(r, g, b), source).into();
        // Colourful enough to tell apart, never loud
        rgba(Hct::from(leaned.get_hue(), leaned.get_chroma().clamp(36.0, 56.0), tone).into())
    };
    let mut ansi = [Rgba::BLACK; 16];
    ansi[0] = p.surface_container_high;
    ansi[8] = p.outline;
    ansi[7] = p.on_surface_variant;
    ansi[15] = p.on_surface;
    for (i, hue) in HUES.iter().enumerate() {
        ansi[i + 1] = tinted(*hue, 70.0);
        ansi[i + 9] = tinted(*hue, 80.0);
    }
    Terminal { background: p.surface, foreground: p.on_surface, cursor: p.primary, selection_background: p.primary_container, selection_foreground: p.on_primary_container, ansi }
}

const HEADER: &str = "Written by Asterism from your wallpaper's colours; it's rewritten when they change.";

fn kitty(p: &Palette) -> String {
    let t = terminal(p);
    let mut out = format!("# {HEADER}\n# Include it from kitty.conf: include asterism-colors.conf\n\n");
    for (name, color) in [
        ("foreground", t.foreground),
        ("background", t.background),
        ("selection_foreground", t.selection_foreground),
        ("selection_background", t.selection_background),
        ("cursor", t.cursor),
        ("cursor_text_color", p.on_primary),
        ("url_color", p.primary),
        ("active_border_color", p.primary),
        ("inactive_border_color", p.outline_variant),
        ("active_tab_foreground", p.on_secondary_container),
        ("active_tab_background", p.secondary_container),
        ("inactive_tab_foreground", p.on_surface_variant),
        ("inactive_tab_background", p.surface_container),
        ("tab_bar_background", p.surface),
    ] {
        out.push_str(&format!("{name} #{}\n", hex(color)));
    }
    out.push('\n');
    for (i, color) in t.ansi.iter().enumerate() {
        out.push_str(&format!("color{i} #{}\n", hex(*color)));
    }
    out
}

fn foot(p: &Palette) -> String {
    let t = terminal(p);
    let mut out = format!("# {HEADER}\n# Include it from foot.ini: include=~/.config/foot/asterism-colors.ini\n\n[colors]\n");
    out.push_str(&format!("foreground={}\nbackground={}\n", hex(t.foreground), hex(t.background)));
    out.push_str(&format!("selection-foreground={}\nselection-background={}\n", hex(t.selection_foreground), hex(t.selection_background)));
    out.push_str(&format!("cursor={} {}\n", hex(p.on_primary), hex(t.cursor)));
    for i in 0..8 {
        out.push_str(&format!("regular{i}={}\n", hex(t.ansi[i])));
    }
    for i in 0..8 {
        out.push_str(&format!("bright{i}={}\n", hex(t.ansi[i + 8])));
    }
    out
}

fn alacritty(p: &Palette) -> String {
    let t = terminal(p);
    let names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    let mut out = format!("# {HEADER}\n# Import it from alacritty.toml: [general] import = [\"~/.config/alacritty/asterism-colors.toml\"]\n\n");
    out.push_str(&format!("[colors.primary]\nbackground = \"#{}\"\nforeground = \"#{}\"\n\n", hex(t.background), hex(t.foreground)));
    out.push_str(&format!("[colors.cursor]\ncursor = \"#{}\"\ntext = \"#{}\"\n\n", hex(t.cursor), hex(p.on_primary)));
    out.push_str(&format!("[colors.selection]\nbackground = \"#{}\"\ntext = \"#{}\"\n", hex(t.selection_background), hex(t.selection_foreground)));
    for (table, offset) in [("normal", 0), ("bright", 8)] {
        out.push_str(&format!("\n[colors.{table}]\n"));
        for (i, name) in names.iter().enumerate() {
            out.push_str(&format!("{name} = \"#{}\"\n", hex(t.ansi[i + offset])));
        }
    }
    out
}

/// libadwaita's named colours (GTK 4), which adw-gtk3 reads for GTK 3 too.
fn gtk(p: &Palette) -> String {
    let mut out = format!("/* {HEADER}\n   Import it from gtk.css: @import url(\"asterism-colors.css\"); */\n\n");
    for (name, color) in [
        ("accent_color", p.primary),
        ("accent_bg_color", p.primary),
        ("accent_fg_color", p.on_primary),
        ("window_bg_color", p.surface),
        ("window_fg_color", p.on_surface),
        ("view_bg_color", p.surface_container_lowest),
        ("view_fg_color", p.on_surface),
        ("headerbar_bg_color", p.surface_container),
        ("headerbar_fg_color", p.on_surface),
        ("sidebar_bg_color", p.surface_container),
        ("sidebar_fg_color", p.on_surface),
        ("card_bg_color", p.surface_container_high),
        ("card_fg_color", p.on_surface),
        ("popover_bg_color", p.surface_container_high),
        ("popover_fg_color", p.on_surface),
        ("dialog_bg_color", p.surface_container_high),
        ("dialog_fg_color", p.on_surface),
        ("destructive_color", p.error),
        ("borders", p.outline_variant),
    ] {
        out.push_str(&format!("@define-color {name} #{};\n", hex(color)));
    }
    out
}

/// Where each app's file goes (under ~/.config), and what's in it.
pub fn files(app: &str, p: &Palette) -> Vec<(PathBuf, String)> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| crate::config::home().join(".config"));
    match app {
        "kitty" => vec![(config.join("kitty/asterism-colors.conf"), kitty(p))],
        "foot" => vec![(config.join("foot/asterism-colors.ini"), foot(p))],
        "alacritty" => vec![(config.join("alacritty/asterism-colors.toml"), alacritty(p))],
        "gtk" => {
            let css = gtk(p);
            vec![(config.join("gtk-3.0/asterism-colors.css"), css.clone()), (config.join("gtk-4.0/asterism-colors.css"), css)]
        }
        _ => Vec::new(),
    }
}

/// Writes the files for these apps where they changed; returns the apps whose files did.
pub fn write(apps: &[String], p: &Palette) -> Vec<String> {
    let mut changed = Vec::new();
    for app in apps {
        for (path, text) in files(app, p) {
            if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
                continue;
            }
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(&path, text) {
                Ok(()) => {
                    if !changed.contains(app) {
                        changed.push(app.clone());
                    }
                }
                Err(error) => log::warn!("couldn't write {}: {error}", path.display()),
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(c: Rgba) -> f32 {
        0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
    }

    #[test]
    fn terminal_colours_keep_their_meaning() {
        // From a blue wallpaper and from an orange one, red stays the reddest and green the greenest
        for source in [Rgba::rgb(0x4a, 0x6f, 0xd9), Rgba::rgb(0xe0, 0x8a, 0x3c)] {
            let t = terminal(&crate::palette::from_source(source));
            let red = t.ansi[1];
            let green = t.ansi[2];
            assert!(red.r > red.g && red.r > red.b, "red {red:?}");
            assert!(green.g > green.r && green.g > green.b, "green {green:?}");
            // Readable on the background; bright ones brighter
            for i in 1..7 {
                assert!(light(t.ansi[i]) > light(t.background) + 0.25, "colour {i} too dark");
                assert!(light(t.ansi[i + 8]) > light(t.ansi[i]), "bright {i} not brighter");
            }
        }
    }

    #[test]
    fn writes_every_format() {
        let p = crate::palette::from_source(Rgba::rgb(0x4a, 0x6f, 0xd9));
        let kitty = kitty(&p);
        assert!(kitty.contains("\ncolor15 #") && kitty.contains("\nbackground #"));
        let foot = foot(&p);
        assert!(foot.contains("[colors]") && foot.contains("bright7="));
        let alacritty: toml::Table = toml::from_str(&alacritty(&p)).expect("alacritty's file is TOML");
        assert!(alacritty["colors"]["bright"]["white"].as_str().is_some_and(|c| c.len() == 7));
        assert_eq!(gtk(&p).matches("@define-color").count(), 19);
        assert_eq!(files("gtk", &p).len(), 2);
        assert!(files("nothing", &p).is_empty());
    }
}
