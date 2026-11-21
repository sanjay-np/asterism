//! How one wallpaper gives way to the next: a fade, a wipe, a circle growing from somewhere, blinds,
//! ink spreading, pixels, or the new picture sliding in. Random by default, never the same twice
//! in a row.
//!
//! Most kinds are a map: for every pixel, a number from 0 to 255 saying when in the change it turns
//! over (a wipe's map is a ramp across the screen, a circle's the distance from its middle). The
//! map is made once, on the painting thread; each frame is then a lookup table from that number to
//! how much of the new picture shows, which keeps a frame of a whole screen to a few milliseconds.

/// One way of changing pictures, with what makes this time's different (a direction, a middle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Fade,
    /// Across the screen, at this angle (radians; 0 is left to right).
    Wipe(f32),
    /// Growing from this point (shares of the width and height).
    Circle(f32, f32),
    /// Strips opening one after another, down the screen (`true`: the strips stand upright).
    Blinds(bool),
    /// Soft blots spreading, like ink in water.
    Ink(u32),
    /// Squares turning over at random.
    Pixels(u32),
    /// The new picture slides in over the old, from this side (0 right, 1 left, 2 below, 3 above).
    Slide(u8),
}

/// The names the config takes, besides "random" and "none".
pub const NAMES: [&str; 7] = ["fade", "wipe", "circle", "blinds", "ink", "pixels", "slide"];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Fade => "fade",
            Kind::Wipe(_) => "wipe",
            Kind::Circle(..) => "circle",
            Kind::Blinds(_) => "blinds",
            Kind::Ink(_) => "ink",
            Kind::Pixels(_) => "pixels",
            Kind::Slide(_) => "slide",
        }
    }

    /// How wide the edge between old and new is, as a share of the change.
    fn soft(self) -> f32 {
        match self {
            Kind::Fade => 1.0,
            Kind::Wipe(_) => 0.15,
            Kind::Circle(..) => 0.12,
            Kind::Blinds(_) => 0.1,
            Kind::Ink(_) => 0.2,
            Kind::Pixels(_) => 0.04,
            Kind::Slide(_) => 0.0,
        }
    }
}

/// A small, good-enough source of chance (no crate for it): splitmix64.
pub struct Chance(u64);

impl Chance {
    #[cfg(test)]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn from_clock() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Self(nanos)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// From 0 up to (not including) `n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// From 0 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// This time's change, from the setting ("random", "none" or one of `NAMES`); a random one is
/// never the kind used `last` time.
pub fn pick(setting: &str, last: Option<&str>, chance: &mut Chance) -> Option<Kind> {
    let setting = setting.trim().to_lowercase();
    let name = match setting.as_str() {
        "none" | "off" => return None,
        "" | "random" => {
            let choices: Vec<&str> = NAMES.iter().copied().filter(|n| Some(*n) != last).collect();
            choices[chance.below(choices.len() as u64) as usize]
        }
        name if NAMES.contains(&name) => name,
        _ => "fade",
    };
    Some(match name {
        "wipe" => Kind::Wipe(chance.below(8) as f32 * std::f32::consts::FRAC_PI_4),
        "circle" => {
            // From the middle, a corner, or somewhere of its own
            match chance.below(3) {
                0 => Kind::Circle(0.5, 0.5),
                1 => Kind::Circle(chance.below(2) as f32, chance.below(2) as f32),
                _ => Kind::Circle(0.15 + 0.7 * chance.unit(), 0.15 + 0.7 * chance.unit()),
            }
        }
        "blinds" => Kind::Blinds(chance.below(2) == 0),
        "ink" => Kind::Ink(chance.next() as u32),
        "pixels" => Kind::Pixels(chance.next() as u32),
        "slide" => Kind::Slide(chance.below(4) as u8),
        _ => Kind::Fade,
    })
}

fn hash(x: i64, y: i64, seed: u32) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f) ^ (seed as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth noise: random heights on a grid `cell` apart, eased between.
fn value_noise(x: f32, y: f32, cell: f32, seed: u32) -> f32 {
    let (gx, gy) = (x / cell, y / cell);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - x0, gy - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = (x0 as i64, y0 as i64);
    let a = hash(x0, y0, seed);
    let b = hash(x0 + 1, y0, seed);
    let c = hash(x0, y0 + 1, seed);
    let d = hash(x0 + 1, y0 + 1, seed);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// When each pixel turns over, 0 to 255, for a canvas `w` × `h` (empty for the kinds without
/// one). `scale` keeps squares and blots the same size on any screen.
pub fn map(kind: Kind, w: u32, h: u32, scale: f32) -> Vec<u8> {
    let (wf, hf) = (w as f32, h as f32);
    let n = (w * h) as usize;
    let mut out = Vec::with_capacity(n);
    let mut fill = |f: &dyn Fn(f32, f32) -> f32| {
        for y in 0..h {
            for x in 0..w {
                out.push((f(x as f32, y as f32).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    };
    match kind {
        Kind::Fade | Kind::Slide(_) => return Vec::new(),
        Kind::Wipe(angle) => {
            let (c, s) = (angle.cos(), angle.sin());
            // The ramp runs from the first corner pixel the edge meets to the last
            let (last_x, last_y) = ((wf - 1.0).max(1.0), (hf - 1.0).max(1.0));
            let corners = [0.0, last_x * c, last_y * s, last_x * c + last_y * s];
            let low = corners.iter().copied().fold(f32::MAX, f32::min);
            let high = corners.iter().copied().fold(f32::MIN, f32::max);
            fill(&|x, y| (x * c + y * s - low) / (high - low).max(1.0));
        }
        Kind::Circle(fx, fy) => {
            let (cx, cy) = (fx * wf, fy * hf);
            let far = [(0.0, 0.0), (wf, 0.0), (0.0, hf), (wf, hf)].iter().map(|(x, y)| (x - cx).hypot(y - cy)).fold(1.0, f32::max);
            fill(&|x, y| (x - cx).hypot(y - cy) / far);
        }
        Kind::Blinds(upright) => {
            let strips = 10.0;
            fill(&|x, y| {
                let along = if upright { x / wf } else { y / hf };
                let k = (along * strips).floor().min(strips - 1.0);
                let within = along * strips - k;
                k / strips * 0.55 + within * 0.45
            });
        }
        Kind::Ink(seed) => {
            let cell = 180.0 * scale;
            let mut raw = Vec::with_capacity(n);
            for y in 0..h {
                for x in 0..w {
                    let (x, y) = (x as f32, y as f32);
                    raw.push(value_noise(x, y, cell, seed) * 0.7 + value_noise(x, y, cell / 2.7, seed.wrapping_add(1)) * 0.3);
                }
            }
            // Stretched so the first blot starts at 0 and the last corner turns at 1
            let low = raw.iter().copied().fold(f32::MAX, f32::min);
            let high = raw.iter().copied().fold(f32::MIN, f32::max);
            let span = (high - low).max(1e-4);
            return raw.into_iter().map(|v| (((v - low) / span) * 255.0).round() as u8).collect();
        }
        Kind::Pixels(seed) => {
            let size = (14.0 * scale).max(4.0);
            fill(&|x, y| hash((x / size) as i64, (y / size) as i64, seed));
        }
    }
    out
}

/// The change eased in and out, so it starts and ends gently.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// One frame of the change, `progress` from 0 (all old) to 1 (all new), into `canvas`. All three
/// are RGBA of `w` × `h`; `map` is `map(kind, …)`.
#[allow(clippy::too_many_arguments)]
pub fn compose(canvas: &mut [u8], old: &[u8], new: &[u8], map: &[u8], kind: Kind, progress: f32, w: u32, h: u32) {
    let progress = progress.clamp(0.0, 1.0);
    if let Kind::Slide(side) = kind {
        return slide(canvas, old, new, side, progress, w as usize, h as usize);
    }
    // How much of the new picture shows (0 to 256) for each place in the map
    let mut lut = [0u16; 256];
    if kind == Kind::Fade || map.len() != canvas.len() / 4 {
        lut = [(progress * 256.0).round() as u16; 256];
    } else {
        let soft = kind.soft();
        let edge = progress * (1.0 + soft);
        for (at, a) in lut.iter_mut().enumerate() {
            *a = (((edge - at as f32 / 255.0) / soft).clamp(0.0, 1.0) * 256.0).round() as u16;
        }
    }
    let uniform = kind == Kind::Fade || map.len() != canvas.len() / 4;
    for (i, ((out, o), n)) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(old.as_chunks::<4>().0).zip(new.as_chunks::<4>().0).enumerate() {
        let a = if uniform { lut[0] } else { lut[map[i] as usize] };
        match a {
            0 => out.copy_from_slice(o),
            256 => out.copy_from_slice(n),
            a => {
                for c in 0..4 {
                    out[c] = ((o[c] as u16 * (256 - a) + n[c] as u16 * a) >> 8) as u8;
                }
            }
        }
    }
}

/// The new picture coming in over the old one from a side, row by row.
fn slide(canvas: &mut [u8], old: &[u8], new: &[u8], side: u8, progress: f32, w: usize, h: usize) {
    canvas.copy_from_slice(old);
    let stride = w * 4;
    match side {
        // From the right, or from the left: each row, part of the new one
        0 | 1 => {
            let shown = ((w as f32 * progress).round() as usize).min(w);
            for y in 0..h {
                let row = y * stride;
                if side == 0 {
                    canvas[row + (w - shown) * 4..row + stride].copy_from_slice(&new[row..row + shown * 4]);
                } else {
                    canvas[row..row + shown * 4].copy_from_slice(&new[row + (w - shown) * 4..row + stride]);
                }
            }
        }
        // From below, or from above: whole rows of the new one
        _ => {
            let shown = ((h as f32 * progress).round() as usize).min(h);
            if side == 2 {
                canvas[(h - shown) * stride..].copy_from_slice(&new[..shown * stride]);
            } else {
                canvas[..shown * stride].copy_from_slice(&new[(h - shown) * stride..]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pictures(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
        let n = (w * h * 4) as usize;
        let old: Vec<u8> = (0..n).map(|i| if i % 4 == 3 { 255 } else { 20 }).collect();
        let new: Vec<u8> = (0..n).map(|i| if i % 4 == 3 { 255 } else { 220 }).collect();
        (old, new)
    }

    #[test]
    fn every_kind_starts_old_and_ends_new() {
        let (w, h) = (64, 40);
        let (old, new) = pictures(w, h);
        let mut chance = Chance::new(7);
        for name in NAMES {
            let kind = pick(name, None, &mut chance).unwrap();
            let map = map(kind, w, h, 1.0);
            let mut canvas = vec![0u8; old.len()];
            compose(&mut canvas, &old, &new, &map, kind, 0.0, w, h);
            assert_eq!(canvas, old, "{name} at the start");
            compose(&mut canvas, &old, &new, &map, kind, 1.0, w, h);
            assert_eq!(canvas, new, "{name} at the end");
            // Halfway, some of each (or, for a fade, between them)
            compose(&mut canvas, &old, &new, &map, kind, 0.5, w, h);
            assert!(canvas != old && canvas != new, "{name} halfway");
        }
    }

    #[test]
    fn a_wipe_goes_its_way() {
        let (w, h) = (100, 10);
        let right = map(Kind::Wipe(0.0), w, h, 1.0);
        assert!(right[0] < right[50] && right[50] < right[99]);
        assert_eq!((right[0], right[99]), (0, 255));
        let left = map(Kind::Wipe(std::f32::consts::PI), w, h, 1.0);
        assert!(left[0] > left[99]);
        let circle = map(Kind::Circle(0.5, 0.5), w, h, 1.0);
        assert!(circle[5 * w as usize + 50] < circle[0]);
    }

    #[test]
    fn random_never_repeats_and_none_is_none() {
        let mut chance = Chance::new(1);
        let mut last: Option<&str> = None;
        for _ in 0..200 {
            let kind = pick("random", last, &mut chance).unwrap();
            assert_ne!(Some(kind.name()), last);
            last = Some(kind.name());
        }
        assert_eq!(pick("none", None, &mut chance), None);
        assert_eq!(pick("Circle", None, &mut chance).map(Kind::name), Some("circle"));
        assert_eq!(pick("sparkles", None, &mut chance), Some(Kind::Fade));
    }

    /// How long a frame of a whole 1920×1200 screen takes (run with --ignored --nocapture).
    #[test]
    #[ignore]
    fn frame_cost() {
        let (w, h) = (1920, 1200);
        let (old, new) = pictures(w, h);
        let mut canvas = vec![0u8; old.len()];
        for name in NAMES {
            let kind = pick(name, None, &mut Chance::new(3)).unwrap();
            let started = std::time::Instant::now();
            let map = map(kind, w, h, 1.0);
            let made = started.elapsed();
            let started = std::time::Instant::now();
            for i in 0..30 {
                compose(&mut canvas, &old, &new, &map, kind, i as f32 / 30.0, w, h);
            }
            println!("{name:7} map {made:>10.1?}  frame {:>8.2?}", started.elapsed() / 30);
        }
    }

    #[test]
    fn maps_fit_their_canvas() {
        for kind in [Kind::Ink(3), Kind::Pixels(3), Kind::Blinds(true), Kind::Wipe(1.0)] {
            assert_eq!(map(kind, 33, 17, 1.0).len(), 33 * 17);
        }
        let ink = map(Kind::Ink(3), 200, 120, 1.0);
        assert_eq!((ink.iter().min(), ink.iter().max()), (Some(&0), Some(&255)));
    }
}
