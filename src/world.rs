//! The wallpaper: the planet's world. A sky falling from deep to its horizon colour, stars in
//! its upper part (a few of them ETEA's four-pointed ones), the light of a low sun, and hills in
//! four layers that curve away at the edges as on a small planet, with a fine grain so the
//! gradients never band. Or a picture of your own instead.
//!
//! Drawn once per output size and scale, away from the Wayland thread.

use crate::config::{Fit, WallpaperSettings};
use crate::theme::{Planet, Rgba};
use tiny_skia::{FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, RadialGradient, SpreadMode, Transform};

/// A small fast random number source, the same sequence for the same seed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Smooth 1D noise: values at whole numbers, eased between them.
fn noise(x: f32, seed: u32) -> f32 {
    let hash = |i: i32| {
        let mut h = (i as u32).wrapping_mul(374_761_393) ^ seed.wrapping_mul(668_265_263);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
    };
    let i = x.floor();
    let f = x - i;
    let t = f * f * (3.0 - 2.0 * f);
    hash(i as i32) * (1.0 - t) + hash(i as i32 + 1) * t
}

/// Ridges: a few octaves of noise, sharper with more `rough`ness.
fn ridge(x: f32, seed: u32, rough: f32) -> f32 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut total = 0.0;
    for octave in 0..5 {
        value += noise(x * frequency, seed + octave * 97) * amplitude;
        total += amplitude;
        amplitude *= 0.45 + 0.1 * rough;
        frequency *= 2.1;
    }
    value / total
}

fn solid(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.skia());
    paint.anti_alias = true;
    paint
}

fn sky(pixmap: &mut Pixmap, planet: &Planet, horizon: f32) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let stops = vec![
        GradientStop::new(0.0, planet.sky_top.skia()),
        GradientStop::new(0.55, planet.sky_mid.skia()),
        GradientStop::new(1.0, planet.sky_low.skia()),
    ];
    if let Some(shader) = LinearGradient::new(Point::from_xy(0.0, 0.0), Point::from_xy(0.0, horizon), stops, SpreadMode::Pad, Transform::identity())
        && let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)
    {
        pixmap.fill_rect(rect, &Paint { shader, ..Default::default() }, Transform::identity(), None);
    }
}

fn stars(pixmap: &mut Pixmap, planet: &Planet, horizon: f32, scale: f32, amount: f32) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let mut random = Random(0x5eed_57a5 ^ (w as u64) << 20 ^ h as u64);
    // Dust: many tiny points, thinning out towards the horizon where the sky is bright
    let count = ((w * h) / (5200.0 * scale * scale) * amount) as usize;
    for _ in 0..count {
        let (x, depth) = (random.next() * w, random.next().powf(1.7));
        let y = depth * horizon * 0.85;
        let size = (0.35 + random.next() * random.next() * 1.1) * scale;
        let alpha = (1.0 - depth).powf(1.3) * (0.35 + 0.65 * random.next());
        if let Some(dot) = PathBuilder::from_circle(x, y, size) {
            pixmap.fill_path(&dot, &solid(Rgba::WHITE.alpha(alpha)), FillRule::Winding, Transform::identity(), None);
        }
    }
    // A few of ETEA's stars, with their halo
    let bright = (7.0 * amount).round() as usize;
    for _ in 0..bright {
        let x = (0.05 + random.next() * 0.9) * w;
        let y = (0.05 + random.next().powf(1.4) * 0.45) * horizon;
        let size = (3.0 + random.next() * 3.5) * scale;
        crate::render::draw_star(pixmap, x, y, size, Rgba::WHITE.alpha(0.9), 0.55, planet.light, Transform::identity());
    }
}

/// The light of a sun just below the horizon.
fn glow(pixmap: &mut Pixmap, planet: &Planet, horizon: f32) {
    let w = pixmap.width() as f32;
    let (cx, cy) = (planet.sun * w, horizon);
    let radius = w * 0.5;
    let stops = vec![
        GradientStop::new(0.0, planet.light.alpha(0.42).skia()),
        GradientStop::new(0.35, planet.light.alpha(0.14).skia()),
        GradientStop::new(1.0, planet.light.alpha(0.0).skia()),
    ];
    if let Some(shader) = RadialGradient::new(Point::from_xy(cx, cy), 0.0, Point::from_xy(cx, cy), radius, stops, SpreadMode::Pad, Transform::identity())
        && let Some(circle) = PathBuilder::from_circle(cx, cy, radius)
    {
        pixmap.fill_path(&circle, &Paint { shader, anti_alias: true, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
    }
}

fn hills(pixmap: &mut Pixmap, planet: &Planet, settings: &WallpaperSettings, scale: f32) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let height = h * settings.terrain.clamp(0.0, 0.8);
    if height < 1.0 {
        return;
    }
    // The planet's curve: on a 1600 wide screen its radius is 3600
    let radius = 3600.0 * w / 1600.0;
    let drop = |x: f32| {
        let dx = x - w / 2.0;
        radius - (radius * radius - dx * dx).max(0.0).sqrt()
    };
    let relief = settings.relief.clamp(0.0, 3.0);
    for (layer, color) in planet.terrain.iter().enumerate() {
        let near = layer as f32 / 3.0;
        // Farther hills sit higher, smaller and hazier; closer ones are bigger and darker
        let base = h - height * (1.0 - near * 0.62);
        let amplitude = height * (0.22 + near * 0.28) * relief;
        let wavelength = w * (0.16 + near * 0.22);
        let haze = planet.sky_low.mix(*color, 0.55 + near * 0.45);
        let mut path = PathBuilder::new();
        path.move_to(0.0, h);
        let step = (3.0 * scale).max(2.0);
        let mut x = 0.0;
        while x <= w + step {
            let y = base - ridge(x / wavelength, 11 + layer as u32 * 31, 0.6) * amplitude + drop(x) * (0.6 + near * 0.4);
            path.line_to(x, y);
            x += step;
        }
        path.line_to(w, h);
        path.close();
        let Some(path) = path.finish() else { continue };
        let top = base - amplitude;
        let stops = vec![GradientStop::new(0.0, haze.skia()), GradientStop::new(1.0, haze.mix(Rgba::BLACK, 0.25).skia())];
        if let Some(shader) = LinearGradient::new(Point::from_xy(0.0, top), Point::from_xy(0.0, h), stops, SpreadMode::Pad, Transform::identity()) {
            pixmap.fill_path(&path, &Paint { shader, anti_alias: true, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
        }
        // The ridge catches the light, faintly
        let mut rim = PathBuilder::new();
        let mut x = 0.0;
        while x <= w + step {
            let y = base - ridge(x / wavelength, 11 + layer as u32 * 31, 0.6) * amplitude + drop(x) * (0.6 + near * 0.4);
            if x == 0.0 { rim.move_to(x, y) } else { rim.line_to(x, y) }
            x += step;
        }
        if let Some(rim) = rim.finish() {
            let stroke = tiny_skia::Stroke { width: 1.2 * scale, ..Default::default() };
            pixmap.stroke_path(&rim, &solid(planet.light.alpha(0.10 * (1.0 - near * 0.6))), &stroke, Transform::identity(), None);
        }
    }
}

/// Fine noise over everything, so long gradients don't show steps.
fn grain(pixmap: &mut Pixmap, amount: f32) {
    let strength = (amount.clamp(0.0, 2.0) * 3.0) as i32;
    if strength == 0 {
        return;
    }
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for px in pixmap.data_mut().as_chunks_mut::<4>().0 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let n = (state >> 59) as i32 % (strength * 2 + 1) - strength;
        for c in &mut px[..3] {
            *c = (*c as i32 + n).clamp(0, 255) as u8;
        }
    }
}

/// A picture of your own, fitted to the screen.
fn picture(pixmap: &mut Pixmap, path: &str, fit: Fit) -> bool {
    let path = crate::config::expand_home(path);
    // A video or a GIF: its first frame
    let Some(image) = crate::images::still(&path, 8192) else {
        log::warn!("wallpaper: can't read {}", path.display());
        return false;
    };
    let Some(source) = tiny_skia::IntSize::from_wh(image.width, image.height).and_then(|size| Pixmap::from_vec(image.data.clone(), size)) else {
        return false;
    };
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let (iw, ih) = (image.width as f32, image.height as f32);
    let (sx, sy) = match fit {
        Fit::Stretch => (w / iw, h / ih),
        Fit::Cover => {
            let s = (w / iw).max(h / ih);
            (s, s)
        }
        Fit::Contain => {
            let s = (w / iw).min(h / ih);
            (s, s)
        }
    };
    let (x, y) = ((w - iw * sx) / 2.0, (h - ih * sy) / 2.0);
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
    pixmap.draw_pixmap(0, 0, source.as_ref(), &paint, Transform::from_row(sx, 0.0, 0.0, sy, x, y), None);
    true
}

/// A quiet wallpaper in the scheme's own colours: deep at the top, a breath of the accent low
/// down, and grain so it never bands.
fn gradient(pixmap: &mut Pixmap, palette: &crate::palette::Palette) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let top = palette.surface_container_lowest;
    let low = palette.primary_container.mix(palette.surface, 0.55);
    let stops = vec![GradientStop::new(0.0, top.skia()), GradientStop::new(0.6, palette.surface.skia()), GradientStop::new(1.0, low.skia())];
    if let Some(shader) = LinearGradient::new(Point::from_xy(0.0, 0.0), Point::from_xy(w * 0.25, h), stops, SpreadMode::Pad, Transform::identity())
        && let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)
    {
        pixmap.fill_rect(rect, &Paint { shader, ..Default::default() }, Transform::identity(), None);
    }
    // A soft light from one corner
    let (cx, cy, radius) = (w * 0.85, h * 1.05, w.max(h) * 0.7);
    let stops = vec![GradientStop::new(0.0, palette.primary.alpha(0.12).skia()), GradientStop::new(1.0, palette.primary.alpha(0.0).skia())];
    if let Some(shader) = RadialGradient::new(Point::from_xy(cx, cy), 0.0, Point::from_xy(cx, cy), radius, stops, SpreadMode::Pad, Transform::identity())
        && let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)
    {
        pixmap.fill_rect(rect, &Paint { shader, ..Default::default() }, Transform::identity(), None);
    }
}

/// The whole wallpaper for a screen of `w`×`h` pixels at `scale`: your picture, the planet's
/// hills if asked for, or a quiet gradient in the scheme's colours.
pub fn render(w: u32, h: u32, scale: f32, planet: &Planet, settings: &WallpaperSettings, palette: &crate::palette::Palette) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(w.max(1), h.max(1))?;
    let image = settings.image.trim();
    if !image.is_empty() {
        // Behind a picture that doesn't cover the screen: the scheme's surface
        pixmap.fill(palette.surface.skia());
        if picture(&mut pixmap, image, settings.fit) {
            return Some(pixmap);
        }
    }
    if settings.world {
        let horizon = h as f32 * (1.0 - settings.terrain.clamp(0.0, 0.8) * 0.75);
        sky(&mut pixmap, planet, horizon);
        stars(&mut pixmap, planet, horizon, scale, settings.stars.clamp(0.0, 2.0));
        glow(&mut pixmap, planet, horizon);
        hills(&mut pixmap, planet, settings, scale);
    } else {
        gradient(&mut pixmap, palette);
    }
    grain(&mut pixmap, settings.grain);
    Some(pixmap)
}

/// What a wallpaper depends on: when this changes, it's drawn again.
pub fn key(w: u32, h: u32, scale: f32, planet: &Planet, settings: &WallpaperSettings, palette: &crate::palette::Palette) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Only what the picture is drawn from: how often it changes, and how, don't touch it
    // (nor how a moving one plays: its first frame is what's painted)
    let still = WallpaperSettings::default();
    let drawn = WallpaperSettings {
        interval: 0,
        directory: String::new(),
        configured: String::new(),
        transition: String::new(),
        transition_ms: 0,
        live: true,
        live_fps: still.live_fps,
        live_scale: still.live_scale,
        live_battery: still.live_battery,
        live_idle: still.live_idle,
        ..settings.clone()
    };
    (w, h, (scale * 120.0) as u32, planet.id, format!("{drawn:?}"), palette.source.key()).hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(crate::config::expand_home(settings.image.trim())) {
        meta.modified().ok().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_a_whole_opaque_world() {
        let settings = WallpaperSettings::default();
        let planet = crate::theme::planet("aster");
        let settings = WallpaperSettings { world: true, ..settings };
        let palette = crate::palette::from_source(crate::palette::DEFAULT_SOURCE);
        let picture = render(320, 200, 1.0, planet, &settings, &palette).unwrap();
        assert_eq!((picture.width(), picture.height()), (320, 200));
        assert!(picture.data().chunks_exact(4).all(|p| p[3] == 255), "no hole in the sky");
        // Hills at the bottom are darker than the sky at the horizon
        let at = |x: u32, y: u32| {
            let p = &picture.data()[((y * 320 + x) * 4) as usize..][..3];
            p.iter().map(|&c| c as u32).sum::<u32>()
        };
        assert!(at(160, 198) < at(160, 100));
    }

    #[test]
    fn knows_when_to_paint_again() {
        let settings = WallpaperSettings::default();
        let p = crate::palette::from_source(crate::palette::DEFAULT_SOURCE);
        let (vesper, aster) = (crate::theme::planet("vesper"), crate::theme::planet("aster"));
        assert_eq!(key(1920, 1200, 1.0, vesper, &settings, &p), key(1920, 1200, 1.0, vesper, &settings, &p));
        assert_ne!(key(1920, 1200, 1.0, vesper, &settings, &p), key(1920, 1200, 1.0, aster, &settings, &p));
        assert_ne!(key(1920, 1200, 1.0, vesper, &settings, &p), key(1920, 1200, 1.25, vesper, &settings, &p));
        let flat = WallpaperSettings { relief: 0.0, ..WallpaperSettings::default() };
        assert_ne!(key(1920, 1200, 1.0, vesper, &settings, &p), key(1920, 1200, 1.0, vesper, &flat, &p));
        // How often and how it changes doesn't make it a different picture
        let animated = WallpaperSettings { interval: 15, transition: "ink".into(), transition_ms: 900, ..settings.clone() };
        assert_eq!(key(1920, 1200, 1.0, vesper, &settings, &p), key(1920, 1200, 1.0, vesper, &animated, &p));
    }
}
