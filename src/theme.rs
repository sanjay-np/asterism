//! The whole look in one place, worked out from the config: sizes from ETEA's Theme.qml,
//! colours from the planet you're on.

use crate::config::{BarSize, BarStyle, Config, PageStyle, Position};

/// A colour, straight (not premultiplied), 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);

    pub fn hex(text: &str) -> Option<Self> {
        let digits = text.trim().trim_start_matches('#');
        let value = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
        match digits.len() {
            6 => Some(Self::rgb(value(0)?, value(2)?, value(4)?)),
            8 => Some(Self::rgb(value(0)?, value(2)?, value(4)?).alpha(value(6)? as f32 / 255.0)),
            _ => None,
        }
    }

    pub fn alpha(self, a: f32) -> Self {
        Self { a: a.clamp(0.0, 1.0), ..self }
    }

    /// Multiplies the alpha it already has.
    pub fn fade(self, by: f32) -> Self {
        self.alpha(self.a * by)
    }

    /// `amount` of the way towards `other`.
    pub fn mix(self, other: Rgba, amount: f32) -> Self {
        let t = amount.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    pub fn skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(self.r, self.g, self.b, self.a).unwrap_or(tiny_skia::Color::BLACK)
    }

    /// A key for caches: colours that look the same hash the same.
    pub fn key(self) -> u32 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        q(self.r) << 24 | q(self.g) << 16 | q(self.b) << 8 | q(self.a)
    }
}

/// A world: its sky from the top down to the horizon, its hills from the farthest to the
/// closest, its light, and where its sun sits (0 left, 1 right).
pub struct Planet {
    pub id: &'static str,
    pub sky_top: Rgba,
    pub sky_mid: Rgba,
    pub sky_low: Rgba,
    pub terrain: [Rgba; 4],
    pub light: Rgba,
    pub sun: f32,
}

pub const PLANETS: &[Planet] = &[
    Planet {
        id: "aster",
        sky_top: Rgba::rgb(0x2b, 0x3a, 0x55),
        sky_mid: Rgba::rgb(0xc9, 0x74, 0x4b),
        sky_low: Rgba::rgb(0xf2, 0xb8, 0x72),
        terrain: [Rgba::rgb(0xa8, 0x55, 0x3c), Rgba::rgb(0x7a, 0x3a, 0x33), Rgba::rgb(0x4f, 0x26, 0x30), Rgba::rgb(0x2c, 0x16, 0x22)],
        light: Rgba::rgb(0xff, 0xe2, 0xa8),
        sun: 0.75,
    },
    Planet {
        id: "nere",
        sky_top: Rgba::rgb(0x0e, 0x2a, 0x3a),
        sky_mid: Rgba::rgb(0x2a, 0x7f, 0x86),
        sky_low: Rgba::rgb(0xb9, 0xe5, 0xd3),
        terrain: [Rgba::rgb(0x3f, 0x90, 0x89), Rgba::rgb(0x2a, 0x6a, 0x70), Rgba::rgb(0x1a, 0x4a, 0x55), Rgba::rgb(0x0e, 0x2c, 0x3a)],
        light: Rgba::rgb(0xc8, 0xff, 0xf0),
        sun: 0.5,
    },
    Planet {
        id: "ocre",
        sky_top: Rgba::rgb(0x3a, 0x4a, 0x43),
        sky_mid: Rgba::rgb(0xb5, 0x9a, 0x3c),
        sky_low: Rgba::rgb(0xec, 0xdc, 0xa0),
        terrain: [Rgba::rgb(0x8a, 0x7a, 0x35), Rgba::rgb(0x5f, 0x5a, 0x2e), Rgba::rgb(0x3f, 0x42, 0x27), Rgba::rgb(0x22, 0x28, 0x1b)],
        light: Rgba::rgb(0xff, 0xf0, 0xb0),
        sun: 0.3,
    },
    Planet {
        id: "vesper",
        sky_top: Rgba::rgb(0x15, 0x10, 0x2e),
        sky_mid: Rgba::rgb(0x6b, 0x3f, 0x9e),
        sky_low: Rgba::rgb(0xe5, 0x9a, 0xc0),
        terrain: [Rgba::rgb(0x7a, 0x3f, 0x8a), Rgba::rgb(0x54, 0x30, 0x6e), Rgba::rgb(0x38, 0x23, 0x52), Rgba::rgb(0x1c, 0x14, 0x30)],
        light: Rgba::rgb(0xff, 0xc8, 0xe6),
        sun: 0.5,
    },
];

pub fn planet(id: &str) -> &'static Planet {
    PLANETS.iter().find(|p| p.id == id.to_lowercase()).unwrap_or(&PLANETS[3])
}

/// The bar's icons in groups, each its own colour when icons are colourful: the status end
/// (and the launcher), the machine (resources, weather, music), and the tools.
#[derive(Clone, Copy)]
pub enum Icons {
    Status = 0,
    Machine = 1,
    Tools = 2,
}

/// Whether dark words read better than the scheme's light ones on a ground of this mean
/// luminance (linear, 0 to 1). The two inks' contrast is even near 0.17 (about #737373);
/// the switch waits a little past that, so a mid-grey wallpaper keeps the usual look.
pub fn wants_dark_ink(luminance: f32) -> bool {
    luminance > 0.22
}

/// How light the wallpaper is under the bar, from one end to the other: the mean luminance
/// (linear) of the band it covers, in narrow slices across the width. A bare bar picks its ink
/// item by item from it, since a picture can be light in the middle and dark at the ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ground {
    slices: Vec<f32>,
}

impl Ground {
    /// Slices of about 8 pixels (at most 400), from a sample of the band's pixels.
    pub fn measure(pixmap: &tiny_skia::Pixmap, rows: std::ops::Range<u32>) -> Ground {
        let (w, h) = (pixmap.width(), pixmap.height());
        let count = (w / 8).clamp(1, 400) as usize;
        let linear = |v: u8| {
            let c = v as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let (mut sums, mut counts) = (vec![0.0f32; count], vec![0u32; count]);
        for y in rows.filter(|y| *y < h).step_by(2) {
            for x in (0..w).step_by(2) {
                if let Some(p) = pixmap.pixel(x, y) {
                    let c = p.demultiply();
                    let slice = (x as usize * count / w as usize).min(count - 1);
                    sums[slice] += 0.2126 * linear(c.red()) + 0.7152 * linear(c.green()) + 0.0722 * linear(c.blue());
                    counts[slice] += 1;
                }
            }
        }
        let slices = sums.iter().zip(&counts).map(|(s, n)| if *n == 0 { 0.0 } else { s / *n as f32 }).collect();
        Ground { slices }
    }

    /// The mean luminance between two places across the width (0 the left edge, 1 the right).
    pub fn between(&self, from: f32, to: f32) -> f32 {
        let n = self.slices.len();
        if n == 0 {
            return 0.0;
        }
        let a = ((from.clamp(0.0, 1.0) * n as f32) as usize).min(n - 1);
        let b = ((to.clamp(0.0, 1.0) * n as f32).ceil() as usize).clamp(a + 1, n);
        self.slices[a..b].iter().sum::<f32>() / (b - a) as f32
    }

    /// Whether dark words read better between those places.
    pub fn light(&self, from: f32, to: f32) -> bool {
        wants_dark_ink(self.between(from, to))
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub style: BarStyle,
    pub on_top: bool,
    /// Workspaces drawn as a figure (numbers or kanji) rather than dots.
    pub numbers: bool,
    pub kanji: bool,
    pub scale: f32,
    pub bar_height: f32,
    /// The bar floats this far off the screen edge (none when it's part of the frame).
    pub bar_gap: f32,
    pub bar_side: f32,
    /// Room under the bar's strip for the frame's rounded corners and the hover labels.
    pub glow_room: f32,
    pub spacing: f32,
    pub icon: f32,
    pub text: f32,
    pub page_width: f32,
    pub page_gap: f32,
    pub glow_strength: f32,
    /// The frame around the screen: its thickness (0 for none) and its inner corners' radius.
    pub frame: f32,
    pub frame_radius: f32,
    /// Corners of cards and menus.
    pub radius: f32,

    /// Material roles, from the palette
    pub palette: crate::palette::Palette,
    pub surface: Rgba,
    pub container_high: Rgba,
    pub container_highest: Rgba,
    pub primary: Rgba,
    pub on_primary: Rgba,
    pub primary_container: Rgba,
    pub on_primary_container: Rgba,
    pub secondary_container: Rgba,
    pub on_secondary_container: Rgba,
    pub outline: Rgba,
    pub outline_variant: Rgba,
    pub error_container: Rgba,

    /// The older names the builders paint with, mapped onto those roles
    pub color_text: Rgba,
    pub color_soft: Rgba,
    pub glow: Rgba,
    pub card_deep: Rgba,
    pub warn: Rgba,
    pub lit: Rgba,
    /// The bar's icons, by group (`Icons`): all three alike unless they're colourful.
    pub icon_tints: [Rgba; 3],
    /// The widgets' big words and headings, their quieter lines, and their cards.
    pub widget_strong: Rgba,
    pub widget_soft: Rgba,
    pub widget_card: Rgba,
    /// How solid the widgets' cards are (0 to 1), whether they have a hairline, and whether
    /// their words need a shadow (a faint card doesn't keep a busy wallpaper from them).
    pub widget_opacity: f32,
    pub widget_outline: bool,
    pub widget_shadow: bool,
    /// Cards are frosted glass: the compositor blurs behind them, so they're drawn see-through
    /// and without a shadow (`Theme::frosted`).
    pub frosted: bool,

    /// Durations, in milliseconds.
    pub fast: f32,
    pub normal: f32,
    /// For the bare bar over a light wallpaper (`for_ground`): the light scheme's colours.
    pub light_ink: Option<Box<Theme>>,
}

impl Theme {
    pub fn new(config: &Config) -> Self {
        Self::with_palette(config, crate::palette::for_config(config))
    }

    pub fn with_palette(config: &Config, p: crate::palette::Palette) -> Self {
        let mut theme = Self::build(config, p, true);
        // The bare bar stands on the wallpaper itself: over a light one, its words and icons
        // take the light scheme's ink (your own text colours are for dark grounds, so not those)
        if config.style == BarStyle::Transparent {
            theme.light_ink = Some(Box::new(Self::build(config, crate::palette::light_from_source(p.source), false)));
        }
        theme
    }

    /// The ink for something on a light ground (the light scheme's, for a bare bar) or not.
    pub fn ink(&self, on_light: bool) -> &Theme {
        match &self.light_ink {
            Some(light) if on_light => light,
            _ => self,
        }
    }

    fn build(config: &Config, p: crate::palette::Palette, own_text: bool) -> Self {
        let scale = match config.size {
            BarSize::Compact => 0.85,
            BarSize::Normal => 1.0,
            BarSize::Large => 1.15,
        };
        let kanji = config.page_style == PageStyle::Kanji;
        let numbers = config.page_style == PageStyle::Numbers || kanji;
        let pick = |own: &Option<String>, fallback: Rgba| own.as_deref().and_then(Rgba::hex).unwrap_or(fallback);
        let pick_text = |own: &Option<String>, fallback: Rgba| if own_text { pick(own, fallback) } else { fallback };
        let framed = config.style == BarStyle::Frame;
        let primary = p.primary;
        let text = pick_text(&config.colors.text, p.on_surface);
        let soft = pick_text(&config.colors.text_soft, p.on_surface_variant);
        // Icons and widgets take the wallpaper's colour: a light text leaning toward the accent
        // keeps them readable while it shows whose wallpaper it is
        let tinted = text.mix(primary, 0.72);
        let icons = config.colors.bar_icons.trim().to_lowercase();
        let icon_tints = match icons.as_str() {
            "plain" => [text; 3],
            "accent" => [primary; 3],
            "colourful" | "colorful" => [primary, text.mix(p.secondary, 0.75), text.mix(p.tertiary, 0.75)],
            _ => [tinted; 3],
        };
        let widgets_tinted = config.colors.widgets.trim().to_lowercase() != "plain";
        let widget_opacity = config.widgets.opacity.min(100) as f32 / 100.0;
        let surface = pick(&config.colors.surface, p.surface);
        Theme {
            style: config.style,
            on_top: config.position == Position::Top,
            numbers,
            kanji,
            scale,
            bar_height: (36.0 * scale).round(),
            bar_gap: if matches!(config.style, BarStyle::Line | BarStyle::Frame | BarStyle::Transparent) { 0.0 } else { 6.0 },
            bar_side: if framed { 14.0 + config.frame as f32 } else { 18.0 },
            glow_room: 30.0,
            spacing: (10.0 * scale).round(),
            icon: (18.0 * scale).round(),
            text: (13.0 * scale).round(),
            page_width: if kanji { (28.0 * scale).round() } else if numbers { (24.0 * scale).round() } else { (20.0 * scale).round() },
            page_gap: if numbers { 2.0 } else { 0.0 },
            glow_strength: config.glow,
            frame: if framed { config.frame as f32 } else { 0.0 },
            frame_radius: config.frame_radius as f32,
            radius: 16.0,

            palette: p,
            surface,
            container_high: p.surface_container_high,
            container_highest: p.surface_container_highest,
            primary,
            on_primary: p.on_primary,
            primary_container: p.primary_container,
            on_primary_container: p.on_primary_container,
            secondary_container: p.secondary_container,
            on_secondary_container: p.on_secondary_container,
            outline: p.outline,
            outline_variant: p.outline_variant,
            error_container: p.error_container,

            color_text: text,
            color_soft: soft,
            glow: primary,
            card_deep: pick(&config.colors.card, p.surface_container),
            icon_tints,
            widget_strong: if widgets_tinted { text.mix(primary, 0.55) } else { text },
            widget_soft: if widgets_tinted { text.mix(primary, 0.35).alpha(0.9) } else { text.alpha(0.85) },
            widget_card: if widgets_tinted { surface.mix(p.primary_container, 0.3).alpha(widget_opacity) } else { surface.alpha(widget_opacity) },
            widget_opacity,
            widget_outline: config.widgets.outline,
            widget_shadow: widget_opacity < 0.4,
            frosted: false,
            warn: pick(&config.colors.warn, p.error),
            // With accent icons, a tool that's on needs another colour to stand out
            lit: if icons == "accent" { p.tertiary } else { primary },

            fast: 160.0,
            normal: 260.0,
            light_ink: None,
        }
    }

    /// The strip of screen the bar keeps for itself.
    pub fn reserve(&self) -> f32 {
        self.bar_gap + self.bar_height
    }

    /// The whole surface: the bar and the room under it.
    pub fn surface_height(&self) -> f32 {
        self.reserve() + self.glow_room
    }

    /// Where the bar's band starts inside the surface.
    pub fn line(&self) -> f32 {
        if self.on_top { self.bar_gap } else { self.glow_room }
    }

    /// The vertical middle of the bar inside the surface.
    pub fn middle(&self) -> f32 {
        self.line() + self.bar_height / 2.0
    }

    /// The bar is the frame's top (or bottom) edge: a solid strip, drawers hang from it.
    pub fn framed(&self) -> bool {
        self.style == BarStyle::Frame
    }

    /// A bar icon's colour, by its group (they differ only when icons are colourful).
    pub fn icon_tint(&self, group: Icons) -> Rgba {
        self.icon_tints[group as usize]
    }

    /// The 1px line that edges the bar and outlines cards.
    /// The same theme for a surface the compositor blurs behind: cards see-through, no shadow.
    pub fn frosted(&self) -> Theme {
        Theme { frosted: true, card_deep: self.card_deep.alpha(0.7), ..self.clone() }
    }

    pub fn hairline(&self) -> Rgba {
        self.outline_variant.alpha(0.75)
    }

    /// A hover's tonal layer.
    pub fn state_layer(&self, amount: f32) -> Rgba {
        self.color_text.alpha(0.08 * amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(source: Rgba, bar_icons: &str) -> Theme {
        let mut config = Config::default();
        config.colors.bar_icons = bar_icons.into();
        Theme::with_palette(&config, crate::palette::from_source(source))
    }

    #[test]
    fn a_bare_bar_takes_dark_ink_over_a_light_wallpaper() {
        let light = |c: Rgba| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        let mut config = Config::default();
        config.colors.text = Some("#ffffff".into());
        config.style = BarStyle::Transparent;
        let bare = Theme::with_palette(&config, crate::palette::from_source(Rgba::rgb(0x4a, 0x6f, 0xd9)));
        assert!(std::ptr::eq(bare.ink(false), &bare), "a dark ground keeps the usual ink");
        let ink = bare.ink(true);
        assert!(light(ink.color_text) < 0.2 && light(ink.primary) < light(bare.primary), "dark words, a deeper accent");
        assert!(light(bare.color_text) > 0.9, "your own text colour stays on dark grounds");
        // Bars that stand on their own surface never change
        config.style = BarStyle::Line;
        let line = Theme::with_palette(&config, crate::palette::from_source(Rgba::rgb(0x4a, 0x6f, 0xd9)));
        assert!(line.light_ink.is_none() && std::ptr::eq(line.ink(true), &line));
    }

    #[test]
    fn measures_how_light_the_ground_is_along_the_bar() {
        // Dark ends and a light middle (a moon), over a dark foot
        let mut picture = tiny_skia::Pixmap::new(800, 60).unwrap();
        picture.fill(tiny_skia::Color::from_rgba8(10, 20, 60, 255));
        let white = tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::WHITE), ..Default::default() };
        picture.fill_rect(tiny_skia::Rect::from_xywh(320.0, 0.0, 160.0, 20.0).unwrap(), &white, tiny_skia::Transform::identity(), None);
        let top = Ground::measure(&picture, 0..20);
        assert!(top.light(0.41, 0.59) && top.between(0.41, 0.59) > 0.99, "the light middle");
        assert!(!top.light(0.0, 0.3) && !top.light(0.7, 1.0), "the dark ends");
        assert!(!top.light(0.0, 1.0), "the whole width is mostly dark");
        assert!(!Ground::measure(&picture, 40..60).light(0.4, 0.6), "the foot under the moon is dark");
        assert!(top.light(0.45, 0.45), "a place with no width still reads its slice");
        assert!(!Ground::default().light(0.0, 1.0), "nothing measured: the usual ink");
        // Mid grey (#808080, 0.22 linear) is the edge; a little darker keeps the light words
        assert!(!wants_dark_ink(0.18) && wants_dark_ink(0.3));
    }

    #[test]
    fn frosted_cards_let_the_blur_through() {
        let plain = theme(Rgba::rgb(0x4a, 0x6f, 0xd9), "");
        let frosted = plain.frosted();
        assert!(!plain.frosted && frosted.frosted);
        assert!(plain.card_deep.a > 0.99 && frosted.card_deep.a < 0.8);
        // Only the card changes
        assert_eq!((frosted.primary, frosted.surface), (plain.primary, plain.surface));
    }

    #[test]
    fn widget_cards_can_be_see_through() {
        let blue = Rgba::rgb(0x4a, 0x6f, 0xd9);
        let at = |opacity: u32| {
            let mut config = Config::default();
            config.widgets.opacity = opacity;
            Theme::with_palette(&config, crate::palette::from_source(blue))
        };
        // The default is a solid-enough card, words without shadows
        let (usual, clear, solid) = (at(60), at(0), at(100));
        assert!((usual.widget_card.a - 0.6).abs() < 0.01 && !usual.widget_shadow);
        // See-through: no card at all, words shadowed; solid: opaque
        assert!(clear.widget_card.a == 0.0 && clear.widget_opacity == 0.0 && clear.widget_shadow);
        assert!((solid.widget_card.a - 1.0).abs() < 0.001 && !solid.widget_shadow);
        // More than 100 is 100
        assert_eq!(at(250).widget_opacity, 1.0);
    }

    #[test]
    fn icons_and_widgets_follow_the_wallpaper() {
        let (blue, orange) = (Rgba::rgb(0x4a, 0x6f, 0xd9), Rgba::rgb(0xe0, 0x8a, 0x3c));
        // Tinted by default: a blue wallpaper gives bluer icons than an orange one, and both
        // differ from plain text
        let (b, o) = (theme(blue, ""), theme(orange, ""));
        let icon_b = b.icon_tint(Icons::Status);
        let icon_o = o.icon_tint(Icons::Status);
        assert!(icon_b.b > icon_o.b && icon_o.r > icon_b.r, "{icon_b:?} {icon_o:?}");
        assert_ne!(icon_b, b.color_text);
        assert!(b.widget_strong.b > o.widget_strong.b);
        // Plain is the text's colour; accent the accent, with tools that are on in another
        assert_eq!(theme(blue, "plain").icon_tint(Icons::Tools), b.color_text);
        let accent = theme(blue, "accent");
        assert_eq!(accent.icon_tint(Icons::Machine), accent.primary);
        assert_ne!(accent.lit, accent.primary);
        // Colourful: the groups differ
        let colourful = theme(blue, "colourful");
        assert_ne!(colourful.icon_tint(Icons::Status), colourful.icon_tint(Icons::Tools));
    }
}
