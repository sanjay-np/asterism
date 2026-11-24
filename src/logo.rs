//! Asterism's mark: an asterism (Cassiopeia's W), five stars joined by a thin line that stops
//! short of each, the brightest one in the accent and ringed, on a dark tile edged with a
//! hairline. It's the bar's workspaces in small,
//! the one flourish the look allows. One geometry, drawn two ways: as a picture in the theme's
//! colours (the Settings window's icon in the dock, so it follows the wallpaper like everything
//! else) and as SVG in the default scheme's colours (`assets/logo/asterism.svg`, for the README
//! and the desktop entry).

use crate::model::Image;
use crate::theme::{Rgba, Theme};
use std::sync::Arc;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// The Settings window's app id: the dock, the overview and the previews draw the mark for it.
pub const SETTINGS_APP: &str = "asterism-settings";

/// The tile and the stars on a 256-unit square: a W of five stars of a few sizes (a zigzag of
/// even dots joined end to end reads as a chart), the last and brightest set apart by its
/// ring. The line between two stars leaves a gap round each, as the bar's workspaces do.
const SIDE: f32 = 256.0;
const TILE: (f32, f32, f32) = (8.0, 240.0, 60.0); // inset, side, corner radius
const STARS: [(f32, f32, f32); 5] = [(40.0, 106.0, 9.0), (80.0, 164.0, 11.0), (118.0, 122.0, 8.0), (156.0, 174.0, 10.0), (194.0, 104.0, 18.0)];
const LINE: f32 = 5.0;
const GAP: f32 = 8.0;
const RING: (f32, f32) = (30.0, 4.5); // radius, width

/// Each joining line, from star to star, cut short of both by its radius and the gap (the
/// bright one's by its ring).
fn segments() -> Vec<((f32, f32), (f32, f32))> {
    let room = |i: usize| if i == STARS.len() - 1 { RING.0 + RING.1 / 2.0 + GAP * 0.6 } else { STARS[i].2 + GAP };
    (0..STARS.len() - 1)
        .map(|i| {
            let ((x0, y0, _), (x1, y1, _)) = (STARS[i], STARS[i + 1]);
            let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
            let (a, b) = (room(i), room(i + 1));
            ((x0 + ux * a, y0 + uy * a), (x1 - ux * b, y1 - uy * b))
        })
        .collect()
}

/// What it's painted in.
pub struct Colours {
    pub tile: Rgba,
    pub edge: Rgba,
    pub line: Rgba,
    pub star: Rgba,
    pub bright: Rgba,
    pub ring: Rgba,
}

impl Colours {
    pub fn of(t: &Theme) -> Colours {
        Colours { tile: t.surface, edge: t.outline_variant, line: t.outline, star: t.color_text, bright: t.primary, ring: t.primary.alpha(0.55) }
    }

    #[cfg(test)]
    fn of_palette(p: &crate::palette::Palette) -> Colours {
        Colours { tile: p.surface, edge: p.outline_variant, line: p.outline, star: p.on_surface, bright: p.primary, ring: p.primary.alpha(0.55) }
    }
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8, (c.a * 255.0).round() as u8);
    paint.anti_alias = true;
    paint
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// The mark as a picture `px` pixels square.
pub fn picture(colours: &Colours, px: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(px, px)?;
    let k = px as f32 / SIDE;
    let at = Transform::from_scale(k, k);
    let (inset, side, radius) = TILE;
    let tile = rounded(inset, inset, side, side, radius)?;
    pixmap.fill_path(&tile, &paint(colours.tile), FillRule::Winding, at, None);
    // The hairline: a pixel wide at any size
    let edge = Stroke { width: 1.0 / k * 1.5, ..Default::default() };
    pixmap.stroke_path(&tile, &paint(colours.edge), &edge, at, None);
    let mut pb = PathBuilder::new();
    for ((x0, y0), (x1, y1)) in segments() {
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
    }
    let line = Stroke { width: LINE, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
    pixmap.stroke_path(&pb.finish()?, &paint(colours.line), &line, at, None);
    let circle = |x: f32, y: f32, r: f32| PathBuilder::from_circle(x, y, r);
    let (bx, by, br) = STARS[STARS.len() - 1];
    let ring = Stroke { width: RING.1, ..Default::default() };
    pixmap.stroke_path(&circle(bx, by, RING.0)?, &paint(colours.ring), &ring, at, None);
    for (x, y, r) in &STARS[..STARS.len() - 1] {
        pixmap.fill_path(&circle(*x, *y, *r)?, &paint(colours.star), FillRule::Winding, at, None);
    }
    pixmap.fill_path(&circle(bx, by, br)?, &paint(colours.bright), FillRule::Winding, at, None);
    Some(pixmap)
}

/// The mark in the theme's colours, as an app icon.
pub fn icon(t: &Theme, px: u32) -> Option<Arc<Image>> {
    let pixmap = picture(&Colours::of(t), px)?;
    Some(Image::new(px, px, pixmap.take()))
}

#[cfg(test)]
fn hex(c: Rgba) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

/// The mark as SVG, in the scheme grown from the default colour (written to
/// `assets/logo/asterism.svg` by its test).
#[cfg(test)]
fn svg() -> String {
    let c = Colours::of_palette(&crate::palette::from_source(crate::palette::DEFAULT_SOURCE));
    let (inset, side, radius) = TILE;
    let (bx, by, br) = STARS[STARS.len() - 1];
    let r1 = |v: f32| (v * 10.0).round() / 10.0;
    let mut out = String::new();
    out.push_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SIDE}\" height=\"{SIDE}\" viewBox=\"0 0 {SIDE} {SIDE}\">\n"));
    out.push_str("  <title>Asterism</title>\n");
    out.push_str(&format!("  <rect x=\"{inset}\" y=\"{inset}\" width=\"{side}\" height=\"{side}\" rx=\"{radius}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.5\"/>\n", hex(c.tile), hex(c.edge)));
    for ((x0, y0), (x1, y1)) in segments() {
        out.push_str(&format!("  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{LINE}\" stroke-linecap=\"round\"/>\n", r1(x0), r1(y0), r1(x1), r1(y1), hex(c.line)));
    }
    out.push_str(&format!("  <circle cx=\"{bx}\" cy=\"{by}\" r=\"{}\" fill=\"none\" stroke=\"{}\" stroke-opacity=\"0.55\" stroke-width=\"{}\"/>\n", RING.0, hex(c.bright), RING.1));
    for (x, y, r) in &STARS[..STARS.len() - 1] {
        out.push_str(&format!("  <circle cx=\"{x}\" cy=\"{y}\" r=\"{r}\" fill=\"{}\"/>\n", hex(c.star)));
    }
    out.push_str(&format!("  <circle cx=\"{bx}\" cy=\"{by}\" r=\"{br}\" fill=\"{}\"/>\n", hex(c.bright)));
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_logo_matches_the_code() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo/asterism.svg");
        if std::env::var_os("ASTERISM_WRITE_LOGO").is_some() {
            std::fs::write(path, svg()).unwrap();
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), svg(), "remake it with ASTERISM_WRITE_LOGO=1 cargo test logo");
    }

    #[test]
    fn the_icon_follows_the_theme() {
        let blue = Theme::with_palette(&crate::config::Config::default(), crate::palette::from_source(Rgba::rgb(0x4a, 0x6f, 0xd9)));
        let orange = Theme::with_palette(&crate::config::Config::default(), crate::palette::from_source(Rgba::rgb(0xd9, 0x7a, 0x2a)));
        let pixel = |t: &Theme| {
            let p = picture(&Colours::of(t), 128).unwrap();
            // The middle of the bright star
            let (x, y) = ((STARS[4].0 / SIDE * 128.0) as u32, (STARS[4].1 / SIDE * 128.0) as u32);
            p.pixel(x, y).unwrap().demultiply()
        };
        let (b, o) = (pixel(&blue), pixel(&orange));
        assert!(b.blue() > b.red() && o.red() > o.blue(), "the brightest star is the accent");
        let p = picture(&Colours::of(&blue), 64).unwrap();
        assert_eq!(p.pixel(0, 0).unwrap().alpha(), 0, "the tile's corners are clear");
        assert_eq!(p.pixel(32, 60).unwrap().alpha(), 255, "the tile is solid");
    }
}
