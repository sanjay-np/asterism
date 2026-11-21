//! The colours, Material 3 style: a dark tonal scheme grown from one source colour, taken from
//! the wallpaper (the colour that stands out most in it) or given in the config.

use crate::theme::Rgba;
use material_colors::color::Rgb;
use material_colors::dynamic_color::Variant;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;
use material_colors::theme::ThemeBuilder;

/// The roles the shell paints with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub surface: Rgba,
    pub surface_container: Rgba,
    pub surface_container_high: Rgba,
    pub surface_container_highest: Rgba,
    pub surface_container_lowest: Rgba,
    pub on_surface: Rgba,
    pub on_surface_variant: Rgba,
    pub primary: Rgba,
    pub on_primary: Rgba,
    pub primary_container: Rgba,
    pub on_primary_container: Rgba,
    pub secondary_container: Rgba,
    pub on_secondary_container: Rgba,
    pub secondary: Rgba,
    pub tertiary: Rgba,
    pub outline: Rgba,
    pub outline_variant: Rgba,
    pub error: Rgba,
    pub error_container: Rgba,
    pub on_error_container: Rgba,
    /// The colour everything grew from.
    pub source: Rgba,
}

/// A quiet blue, for when there's no wallpaper to take a colour from.
pub const DEFAULT_SOURCE: Rgba = Rgba::rgb(0x4a, 0x6f, 0xd9);

fn rgba(c: Rgb) -> Rgba {
    Rgba::rgb(c.red, c.green, c.blue)
}

fn rgb(c: Rgba) -> Rgb {
    Rgb::new((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8)
}

/// The colour a picture is about: its pixels grouped, then scored for how well each group
/// would carry a theme (common, colourful, not muddy).
pub fn source_of(path: &std::path::Path) -> Option<Rgba> {
    let image = crate::images::load(path, 112)?;
    let pixels: Vec<Rgb> = image
        .data
        .as_chunks::<4>().0.iter()
        .filter(|p| p[3] > 200)
        .map(|p| Rgb::new(p[0], p[1], p[2]))
        .collect();
    if pixels.is_empty() {
        return None;
    }
    let groups = QuantizerCelebi::quantize(&pixels, 128);
    let best = Score::score(&groups.color_to_count, Some(1), None, None);
    best.first().map(|c| rgba(*c))
}

pub fn from_source(source: Rgba) -> Palette {
    let theme = ThemeBuilder::with_source(rgb(source)).variant(Variant::TonalSpot).build();
    let s = theme.schemes.dark;
    Palette {
        surface: rgba(s.surface),
        surface_container: rgba(s.surface_container),
        surface_container_high: rgba(s.surface_container_high),
        surface_container_highest: rgba(s.surface_container_highest),
        surface_container_lowest: rgba(s.surface_container_lowest),
        on_surface: rgba(s.on_surface),
        on_surface_variant: rgba(s.on_surface_variant),
        primary: rgba(s.primary),
        on_primary: rgba(s.on_primary),
        primary_container: rgba(s.primary_container),
        on_primary_container: rgba(s.on_primary_container),
        secondary_container: rgba(s.secondary_container),
        on_secondary_container: rgba(s.on_secondary_container),
        secondary: rgba(s.secondary),
        tertiary: rgba(s.tertiary),
        outline: rgba(s.outline),
        outline_variant: rgba(s.outline_variant),
        error: rgba(s.error),
        error_container: rgba(s.error_container),
        on_error_container: rgba(s.on_error_container),
        source,
    }
}

/// The palette the config asks for: its accent, else the wallpaper's colour, else the default.
pub fn for_config(config: &crate::config::Config) -> Palette {
    let accent = config.colors.accent.as_deref().and_then(Rgba::hex);
    let from_wallpaper = || {
        let image = config.wallpaper.image.trim();
        (!image.is_empty()).then(|| source_of(&crate::config::expand_home(image))).flatten()
    };
    from_source(accent.or_else(from_wallpaper).unwrap_or(DEFAULT_SOURCE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_a_dark_scheme() {
        let p = from_source(Rgba::rgb(0x4a, 0x6f, 0xd9));
        let light = |c: Rgba| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        assert!(light(p.surface) < 0.1, "a dark surface");
        assert!(light(p.on_surface) > 0.75, "light text on it");
        assert!(light(p.surface_container_high) > light(p.surface), "containers rise above the surface");
        assert!(p.primary.b > p.primary.r, "a blue source gives a blue primary");
    }
}
