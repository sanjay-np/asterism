//! The pet's picture: a rig (hips, shoulders, head, four feet, a tail) that the behaviour moves
//! smoothly between poses, drawn as filled outlines: a torso of haunch, belly and chest, legs
//! solved to bend at the elbow and the hock, a tapering tail that curls with its mood, a head
//! with ears, eyes that blink and follow the pointer, whiskers. The other four-legged animals
//! are the same rig built differently (`Build`): heavier, round-eared, with a tuft, a ringed tail
//! or a stub. Coats are the animals' own colours (a tabby, a ginger, a red fox, a Bengal tiger, a
//! giant panda, a sparrow…), or the wallpaper's.
//!
//! Coordinates in the rig are the animal's own: x forward, y down, its feet on y = 0, in pixels
//! at size 1. `Frame` turns them into the surface's, mirrored when it faces left.

use super::Species;
use crate::render::text::Font;
use crate::theme::{Icons, Rgba, Theme};
use crate::ui::Prim;

type P = (f32, f32);

/// A pose: where everything is. Moved between smoothly (`lerp`).
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    pub hip: P,
    pub shoulder: P,
    pub head: P,
    /// The head's turn, radians (down is positive).
    pub tilt: f32,
    /// Near front, far front, near hind, far hind (the bird uses the first two).
    pub feet: [P; 4],
    /// The tail's direction at its root (radians, y down: π is straight back, −π/2 up), how much
    /// it curls along its length, and how much it sways.
    pub tail: f32,
    pub curl: f32,
    pub sway: f32,
    /// Ears up (1) or flat (0); eyes open (1) or shut (0); mouth open (1, a yawn).
    pub ears: f32,
    pub eyes: f32,
    pub mouth: f32,
    /// Legs folded away: 0 none, 0.5 the hind ones, 1 all (only paws show).
    pub tuck: f32,
    /// Curled up asleep (above 0.5); for the bird, puffed up.
    pub ball: f32,
    /// The bird's wings: folded (0) to flying (1).
    pub wing: f32,
}

fn mix(a: P, b: P, k: f32) -> P {
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

fn num(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

impl Rig {
    /// The most any part of it is away from `other` (pixels or radians, alike).
    pub fn distance(&self, other: &Rig) -> f32 {
        let points = [(self.hip, other.hip), (self.shoulder, other.shoulder), (self.head, other.head)].into_iter().chain(self.feet.into_iter().zip(other.feet));
        let far = points.map(|(a, b)| (a.0 - b.0).abs().max((a.1 - b.1).abs())).fold(0.0, f32::max);
        [(self.tilt, other.tilt), (self.tail, other.tail), (self.curl, other.curl), (self.sway, other.sway), (self.ears, other.ears), (self.eyes, other.eyes), (self.mouth, other.mouth), (self.tuck, other.tuck), (self.ball, other.ball), (self.wing, other.wing)]
            .into_iter()
            .map(|(a, b)| (a - b).abs())
            .fold(far, f32::max)
    }

    /// Part of the way to `other` (0 stays, 1 arrives), each part at its own rate.
    pub fn lerp(&self, other: &Rig, k: &Rates) -> Rig {
        let mut feet = self.feet;
        for (f, o) in feet.iter_mut().zip(other.feet) {
            *f = mix(*f, o, k.feet);
        }
        Rig {
            hip: mix(self.hip, other.hip, k.body),
            shoulder: mix(self.shoulder, other.shoulder, k.body),
            head: mix(self.head, other.head, k.head),
            tilt: num(self.tilt, other.tilt, k.head),
            feet,
            tail: num(self.tail, other.tail, k.tail),
            curl: num(self.curl, other.curl, k.tail),
            sway: num(self.sway, other.sway, k.tail),
            ears: num(self.ears, other.ears, (k.head * 1.4).min(1.0)),
            eyes: num(self.eyes, other.eyes, k.head * 1.6).clamp(0.0, 1.0),
            mouth: num(self.mouth, other.mouth, k.head),
            tuck: num(self.tuck, other.tuck, k.body),
            ball: num(self.ball, other.ball, k.body),
            wing: num(self.wing, other.wing, k.body),
        }
    }
}

/// How far toward a pose each part goes in a moment: the head quickest, the tail last.
pub struct Rates {
    pub head: f32,
    pub body: f32,
    pub tail: f32,
    pub feet: f32,
}

/// Which way the colours go on the coat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pattern {
    Plain,
    /// Dark stripes across the back, rings on the tail, an M on the brow.
    Tabby,
    /// A dark face, ears, legs and tail (a Siamese).
    Points,
    /// Dark legs and ear backs, a white tail tip (a fox).
    Socks,
    /// Dark streaks on the back and wings (a sparrow).
    Streaks,
    /// A coloured breast (`light`): a robin, a bluebird.
    Breast,
    /// Bold dark stripes over the body, legs, face and tail rings (a tiger).
    Stripes,
    /// Dark legs, shoulders, ears and eye patches on a light coat (a panda).
    Panda,
}

#[derive(Clone, Copy, Debug)]
pub struct Coat {
    pub base: Rgba,
    pub dark: Rgba,
    pub light: Rgba,
    pub eye: Rgba,
    pub nose: Rgba,
    pub inner_ear: Rgba,
    pub pattern: Pattern,
    /// A light chest and belly, white paws, a light muzzle.
    pub chest: bool,
    pub paws: bool,
    pub muzzle: bool,
    /// A mane round the head and down the neck (a lion), in this colour.
    pub mane: Option<Rgba>,
}

/// How a four-legged animal is built, beside a cat.
#[derive(Clone, Copy)]
struct Build {
    /// Round ears (the big cats', the panda's) of this radius; else pointed, this tall.
    round_ears: bool,
    ear: f32,
    tail: Tail,
    /// How much thicker its body and legs are.
    heft: f32,
    /// The skull's half-width and half-height.
    skull: (f32, f32),
    whiskers: bool,
    /// Slit pupils (a cat's, a fox's), else round ones.
    slit: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Tail {
    /// A cat's, thinning to its tip; a tiger's is the same, thicker and ringed.
    Thin,
    /// A fox's bushy brush.
    Brush,
    /// A lion's, with a dark tuft at its end.
    Tuft,
    /// A panda's: barely there.
    Stub,
}

fn build(species: Species) -> Build {
    let cat = Build { round_ears: false, ear: 9.0, tail: Tail::Thin, heft: 1.0, skull: (9.5, 8.4), whiskers: true, slit: true };
    match species {
        Species::Fox => Build { ear: 12.5, tail: Tail::Brush, skull: (9.0, 7.6), whiskers: false, ..cat },
        Species::Tiger => Build { round_ears: true, ear: 3.2, heft: 1.12, skull: (10.5, 9.0), slit: false, ..cat },
        Species::Lion => Build { round_ears: true, ear: 3.0, tail: Tail::Tuft, heft: 1.1, skull: (10.5, 9.2), slit: false, ..cat },
        Species::Panda => Build { round_ears: true, ear: 4.3, tail: Tail::Stub, heft: 1.28, skull: (10.5, 9.6), whiskers: false, slit: false },
        _ => cat,
    }
}

/// The coats there are, by animal (the first is the one it has unless another is chosen).
pub fn coats(species: super::Species) -> &'static [&'static str] {
    match species {
        super::Species::Cat => &["tabby", "ginger", "black", "white", "tuxedo", "siamese", "wallpaper"],
        super::Species::Fox => &["red", "arctic", "wallpaper"],
        super::Species::Bird => &["sparrow", "robin", "bluebird", "wallpaper"],
        super::Species::Panda => &["giant", "brown", "wallpaper"],
        super::Species::Tiger => &["bengal", "white", "wallpaper"],
        super::Species::Lion => &["lion", "lioness", "wallpaper"],
    }
}

/// An animal's coat by name; an unknown one is the animal's first.
pub fn coat(species: super::Species, name: &str, t: &Theme) -> Coat {
    let rgb = Rgba::rgb;
    let name = name.trim().to_lowercase();
    let name = if coats(species).contains(&name.as_str()) { name.as_str() } else { coats(species)[0] };
    let pink = rgb(0xe8, 0x9a, 0xa0);
    let plain = |base: Rgba, dark: Rgba, light: Rgba, eye: Rgba| Coat { base, dark, light, eye, nose: pink, inner_ear: pink, pattern: Pattern::Plain, chest: false, paws: false, muzzle: false, mane: None };
    match (species, name) {
        // The wallpaper's colours, for every animal: the tint the bar's icons have, the accent
        (_, "wallpaper") => {
            let base = t.icon_tint(Icons::Machine);
            let dark = base.mix(t.surface, 0.45);
            let pattern = match species {
                Species::Bird => Pattern::Breast,
                Species::Tiger => Pattern::Stripes,
                Species::Panda => Pattern::Panda,
                _ => Pattern::Plain,
            };
            let dark = if species == Species::Panda { base.mix(t.surface, 0.75) } else { dark };
            let mane = (species == Species::Lion).then(|| base.mix(t.surface, 0.3));
            Coat { dark, light: t.color_text, eye: t.primary, nose: t.primary, inner_ear: t.primary.mix(base, 0.4), pattern, chest: species != Species::Panda, mane, ..plain(base, base, base, base) }
        }
        // A giant panda: white with black legs, shoulders, ears and eye patches; a Qinling
        // panda is brown and tan
        (Species::Panda, "brown") => Coat { pattern: Pattern::Panda, muzzle: true, nose: rgb(0x2e, 0x24, 0x20), inner_ear: rgb(0x4a, 0x38, 0x2c), ..plain(rgb(0xd8, 0xc6, 0xae), rgb(0x5e, 0x45, 0x34), rgb(0xf0, 0xe8, 0xdc), rgb(0x2a, 0x1e, 0x18)) },
        (Species::Panda, _) => Coat { pattern: Pattern::Panda, muzzle: true, nose: rgb(0x1a, 0x18, 0x1a), inner_ear: rgb(0x2a, 0x28, 0x2c), ..plain(rgb(0xf2, 0xf0, 0xea), rgb(0x1e, 0x1d, 0x21), rgb(0xff, 0xff, 0xff), rgb(0x30, 0x24, 0x1e)) },
        // A tiger: orange with black stripes and white cheeks, or the white one
        (Species::Tiger, "white") => Coat { pattern: Pattern::Stripes, chest: true, muzzle: true, nose: rgb(0xd8, 0x9a, 0x9a), inner_ear: rgb(0xf4, 0xf0, 0xea), ..plain(rgb(0xf1, 0xee, 0xe6), rgb(0x3a, 0x36, 0x36), rgb(0xff, 0xff, 0xff), rgb(0x6c, 0xa4, 0xdc)) },
        (Species::Tiger, _) => Coat { pattern: Pattern::Stripes, chest: true, muzzle: true, nose: rgb(0xc8, 0x7c, 0x70), inner_ear: rgb(0xf6, 0xee, 0xe2), ..plain(rgb(0xe0, 0x7c, 0x2c), rgb(0x1e, 0x16, 0x12), rgb(0xf8, 0xf1, 0xe6), rgb(0xe2, 0xb2, 0x34)) },
        // A lion (with his mane) or a lioness: tawny, a dark tuft on the tail
        (Species::Lion, "lioness") => Coat { chest: true, muzzle: true, nose: rgb(0xb8, 0x7a, 0x6a), inner_ear: rgb(0x8a, 0x66, 0x48), ..plain(rgb(0xcf, 0xa4, 0x68), rgb(0x5a, 0x3e, 0x26), rgb(0xf3, 0xe7, 0xcf), rgb(0xc8, 0x92, 0x34)) },
        (Species::Lion, _) => Coat { chest: true, muzzle: true, nose: rgb(0xa8, 0x6c, 0x5c), inner_ear: rgb(0x8a, 0x66, 0x48), mane: Some(rgb(0x8c, 0x5a, 0x2e)), ..plain(rgb(0xcd, 0xa0, 0x62), rgb(0x4e, 0x32, 0x1e), rgb(0xf3, 0xe7, 0xcf), rgb(0xc8, 0x92, 0x34)) },
        (_, "ginger") => Coat { pattern: Pattern::Tabby, chest: true, muzzle: true, ..plain(rgb(0xe0, 0x8c, 0x45), rgb(0xb0, 0x5a, 0x22), rgb(0xfb, 0xe6, 0xcc), rgb(0xd9, 0xa8, 0x2e)) },
        (_, "black") => Coat { nose: rgb(0x3a, 0x30, 0x30), inner_ear: rgb(0x4a, 0x38, 0x3a), ..plain(rgb(0x2a, 0x29, 0x2e), rgb(0x16, 0x16, 0x19), rgb(0x45, 0x44, 0x4c), rgb(0xe3, 0xc0, 0x3c)) },
        (_, "white") => Coat { ..plain(rgb(0xf3, 0xf1, 0xec), rgb(0xd3, 0xcf, 0xc7), rgb(0xff, 0xff, 0xff), rgb(0x6c, 0xa6, 0xe2)) },
        (_, "tuxedo") => Coat { chest: true, paws: true, muzzle: true, nose: rgb(0xd8, 0x8c, 0x92), ..plain(rgb(0x25, 0x24, 0x28), rgb(0x14, 0x14, 0x16), rgb(0xf4, 0xf2, 0xee), rgb(0x9c, 0xc0, 0x58)) },
        (_, "siamese") => Coat { pattern: Pattern::Points, nose: rgb(0x5a, 0x44, 0x3c), ..plain(rgb(0xef, 0xe3, 0xcf), rgb(0x5c, 0x45, 0x37), rgb(0xfa, 0xf4, 0xe8), rgb(0x5a, 0x9e, 0xe2)) },
        (_, "red") => Coat { pattern: Pattern::Socks, chest: true, muzzle: true, nose: rgb(0x1c, 0x16, 0x14), inner_ear: rgb(0xf0, 0xe4, 0xd8), ..plain(rgb(0xd6, 0x66, 0x2a), rgb(0x2c, 0x20, 0x1a), rgb(0xf7, 0xf0, 0xe6), rgb(0xd8, 0x9a, 0x2c)) },
        (_, "arctic") => Coat { pattern: Pattern::Socks, chest: true, muzzle: true, nose: rgb(0x22, 0x20, 0x22), inner_ear: rgb(0xe6, 0xe4, 0xe8), ..plain(rgb(0xec, 0xee, 0xf1), rgb(0xa9, 0xae, 0xb8), rgb(0xff, 0xff, 0xff), rgb(0xc9, 0x8e, 0x34)) },
        (_, "sparrow") => Coat { pattern: Pattern::Streaks, nose: rgb(0x6b, 0x5a, 0x48), ..plain(rgb(0x9c, 0x7b, 0x58), rgb(0x52, 0x3e, 0x2d), rgb(0xe6, 0xdc, 0xcc), rgb(0x18, 0x16, 0x16)) },
        (_, "robin") => Coat { pattern: Pattern::Breast, nose: rgb(0x3e, 0x34, 0x2c), ..plain(rgb(0x7e, 0x6c, 0x5c), rgb(0x4e, 0x42, 0x38), rgb(0xe2, 0x6c, 0x30), rgb(0x16, 0x14, 0x14)) },
        (_, "bluebird") => Coat { pattern: Pattern::Breast, nose: rgb(0x2e, 0x2c, 0x30), ..plain(rgb(0x4c, 0x7c, 0xd0), rgb(0x2c, 0x4c, 0x8c), rgb(0xde, 0x8a, 0x4a), rgb(0x14, 0x14, 0x18)) },
        // A grey-brown tabby: the cat's own coat
        _ => Coat { pattern: Pattern::Tabby, chest: true, muzzle: true, ..plain(rgb(0x92, 0x8a, 0x80), rgb(0x4e, 0x47, 0x40), rgb(0xea, 0xe4, 0xda), rgb(0x9a, 0xb8, 0x4e)) },
    }
}

/// From the animal's coordinates to the surface's: its feet at `feet`, facing `f` (±1), `s` big.
#[derive(Clone, Copy)]
pub struct Frame {
    pub feet: P,
    pub f: f32,
    pub s: f32,
}

impl Frame {
    fn at(&self, p: P) -> P {
        (self.feet.0 + p.0 * self.f * self.s, self.feet.1 + p.1 * self.s)
    }

    fn shape(&self, points: Vec<P>, color: Rgba) -> Prim {
        Prim::Shape { points: points.into_iter().map(|p| self.at(p)).collect(), color }
    }

    fn line(&self, points: Vec<P>, width: f32, color: Rgba) -> Prim {
        Prim::Line { points: points.into_iter().map(|p| self.at(p)).collect(), width: width * self.s, color }
    }
}

/// An ellipse round `c`, turned by `rot`.
fn ellipse(c: P, rx: f32, ry: f32, rot: f32) -> Vec<P> {
    let (sr, cr) = rot.sin_cos();
    // Twenty points: smooth at the pet's size, and a third less to fill than before
    (0..20)
        .map(|i| {
            let a = i as f32 / 20.0 * std::f32::consts::TAU;
            let (x, y) = (rx * a.cos(), ry * a.sin());
            (c.0 + x * cr - y * sr, c.1 + x * sr + y * cr)
        })
        .collect()
}

/// A limb from `a` (radius `ra`) to `b` (radius `rb`), rounded at both ends.
fn capsule(a: P, b: P, ra: f32, rb: f32) -> Vec<P> {
    let angle = (b.1 - a.1).atan2(b.0 - a.0);
    let mut points = Vec::with_capacity(20);
    for i in 0..=9 {
        let t = angle - std::f32::consts::FRAC_PI_2 + i as f32 / 9.0 * std::f32::consts::PI;
        points.push((b.0 + rb * t.cos(), b.1 + rb * t.sin()));
    }
    for i in 0..=9 {
        let t = angle + std::f32::consts::FRAC_PI_2 + i as f32 / 9.0 * std::f32::consts::PI;
        points.push((a.0 + ra * t.cos(), a.1 + ra * t.sin()));
    }
    points
}

/// Where the knee (or elbow) goes for a leg from `joint` to `foot`, bending backward.
fn knee(joint: P, foot: P, upper: f32, lower: f32) -> P {
    let (dx, dy) = (foot.0 - joint.0, foot.1 - joint.1);
    let d = dx.hypot(dy).clamp((upper - lower).abs() + 0.01, upper + lower - 0.01);
    let a = dy.atan2(dx);
    let b = ((upper * upper + d * d - lower * lower) / (2.0 * upper * d)).clamp(-1.0, 1.0).acos();
    (joint.0 + upper * (a + b).cos(), joint.1 + upper * (a + b).sin())
}

/// Turns a point of the head's own (x forward) by the head's tilt, round its centre.
fn turn(head: P, tilt: f32, p: P) -> P {
    let (s, c) = tilt.sin_cos();
    (head.0 + p.0 * c - p.1 * s, head.1 + p.0 * s + p.1 * c)
}

/// What's drawn round the animal: a heart, "!", z's, a speech bubble.
pub struct Extras<'a> {
    pub heart: f32,
    pub alert: bool,
    pub asleep: bool,
    pub happy: bool,
    pub bubble: Option<(&'a str, f32)>,
    /// How far below its feet the floor is (for its shadow).
    pub ground: f32,
    pub look: P,
}

fn darker(c: Rgba, by: f32) -> Rgba {
    c.mix(Rgba::BLACK, by)
}

/// Stripes across a stretch of body from `a` to `b` (`count` of them, reaching `reach` from the
/// back down its side, `width` wide).
#[allow(clippy::too_many_arguments)]
fn stripes(p: &mut Vec<Prim>, fr: &Frame, a: P, b: P, count: usize, reach: (f32, f32), width: f32, color: Rgba) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(1.0);
    let normal = (dy / len, -dx / len);
    for i in 0..count {
        let along = mix(a, b, (i as f32 + 0.6) / (count as f32 + 0.4));
        let top = (along.0 + normal.0 * reach.0, along.1 + normal.1 * reach.0);
        let low = (along.0 + normal.0 * reach.1 - dx / len * 1.5, along.1 + normal.1 * reach.1 - dy / len * 1.5);
        p.push(fr.line(vec![top, mix(top, low, 0.5), low], width, color));
    }
}

/// A cat, a fox, a panda, a tiger or a lion.
#[allow(clippy::too_many_arguments)]
pub fn quadruped(p: &mut Vec<Prim>, fr: &Frame, rig: &Rig, coat: &Coat, species: Species, clock: f32, x: &Extras) {
    let fox = species == Species::Fox;
    let b = build(species);
    let base = coat.base;
    let points = coat.pattern == Pattern::Points;
    let socks = coat.pattern == Pattern::Socks;
    let panda = coat.pattern == Pattern::Panda;
    let striped = coat.pattern == Pattern::Stripes;
    let breathe = 1.0 + 0.035 * (clock * 2.4).sin() * if x.asleep || rig.tuck > 0.5 { 1.0 } else { 0.4 };

    // A soft shadow on the floor
    if x.ground < 40.0 {
        let spread = (1.0 - x.ground / 40.0).max(0.2);
        let mid = ((rig.hip.0 + rig.shoulder.0) / 2.0, x.ground + 0.5);
        p.push(fr.shape(ellipse(mid, 26.0 * b.heft * spread, 3.4 * spread, 0.0), Rgba::BLACK.alpha(0.22 * spread)));
    }

    if rig.ball > 0.5 {
        return curled(p, fr, rig, coat, species, clock, breathe);
    }

    // The tail, behind everything (on the floor it lies along it; dangling it hangs free)
    tail(p, fr, rig, coat, species, clock, (x.ground < 2.0).then_some(0.0));

    let leg_len: (f32, f32) = if fox { (10.0, 10.5) } else { (9.0, 9.5) };
    let hind_joint = (rig.hip.0 + 1.0, rig.hip.1 + 3.0);
    let front_joint = (rig.shoulder.0, rig.shoulder.1 + 4.0);
    let leg_colour = |near: bool, lower: bool| -> Rgba {
        let c = if panda || (points || socks) && lower { coat.dark } else if points { base.mix(coat.dark, 0.35) } else { base };
        if near { c } else { darker(c, 0.22) }
    };
    let heft = b.heft;
    let leg = |p: &mut Vec<Prim>, joint: P, foot: P, near: bool, hind: bool| {
        let (upper, lower) = leg_len;
        let k = knee(joint, foot, upper, lower);
        let thick = if hind { 4.4 } else { 3.6 } * heft;
        p.push(fr.shape(capsule(joint, k, thick, 2.9 * heft), leg_colour(near, false)));
        p.push(fr.shape(capsule(k, foot, 2.9 * heft, 2.3 * heft), leg_colour(near, true)));
        if striped {
            // Two short bars across the leg
            for along in [0.35, 0.75] {
                let c = mix(joint, k, along);
                p.push(fr.line(vec![(c.0 - 2.6 * heft, c.1 - 0.6), (c.0 + 2.6 * heft, c.1 + 0.6)], 1.6, coat.dark.alpha(if near { 0.9 } else { 0.7 })));
            }
        }
        let paw = if coat.paws { coat.light } else { leg_colour(near, true) };
        p.push(fr.shape(ellipse((foot.0 + 1.3, foot.1 - 1.4), 3.2 * heft, 2.1 * heft.sqrt(), 0.0), if near { paw } else { darker(paw, 0.18) }));
    };

    // Far legs, then the body, then the near legs
    if rig.tuck < 0.5 {
        leg(p, (hind_joint.0 - 2.0, hind_joint.1), rig.feet[3], false, true);
    }
    if rig.tuck < 1.0 {
        leg(p, (front_joint.0 - 2.0, front_joint.1), rig.feet[1], false, false);
    }
    let sitting = rig.shoulder.1 < rig.hip.1 - 12.0;
    let haunch_r = if sitting { 13.0 } else { 11.0 } * breathe * b.heft;
    p.push(fr.shape(capsule(rig.hip, rig.shoulder, 9.5 * breathe * b.heft, 9.0 * breathe * b.heft), base));
    p.push(fr.shape(ellipse(rig.hip, haunch_r, haunch_r * 0.95, 0.0), base));
    // A panda's shoulders are black, a band running down into the front legs
    p.push(fr.shape(ellipse(rig.shoulder, 10.0 * breathe * b.heft, 10.5 * breathe * b.heft, 0.0), if panda { coat.dark } else { base }));
    if striped {
        stripes(p, fr, rig.shoulder, rig.hip, 6, (10.0 * b.heft, 1.0), 2.8, coat.dark.alpha(0.92));
        stripes(p, fr, (rig.hip.0 + 4.0, rig.hip.1), (rig.hip.0 - 8.0, rig.hip.1 + 2.0), 2, (11.0 * b.heft, 3.0), 2.6, coat.dark.alpha(0.88));
    }
    if coat.chest {
        // A light chest and belly
        let chest = mix(rig.shoulder, rig.hip, 0.15);
        let down = if sitting { (3.0, 2.0) } else { (1.5, 6.0) };
        p.push(fr.shape(ellipse((chest.0 + down.0, chest.1 + down.1), 7.5, if sitting { 10.0 } else { 6.0 }, 0.0), coat.light));
    }
    if coat.pattern == Pattern::Tabby {
        // Stripes across the back, from the shoulders to the haunch
        for i in 0..4 {
            let along = mix(rig.shoulder, rig.hip, 0.18 + 0.2 * i as f32);
            let (dx, dy) = (rig.hip.0 - rig.shoulder.0, rig.hip.1 - rig.shoulder.1);
            let len = dx.hypot(dy).max(1.0);
            let normal = (dy / len, -dx / len);
            let top = (along.0 + normal.0 * 9.0, along.1 + normal.1 * 9.0);
            let low = (along.0 + normal.0 * 3.0 - dx / len * 1.5, along.1 + normal.1 * 3.0 - dy / len * 1.5);
            p.push(fr.line(vec![top, mix(top, low, 0.5), low], 2.4, coat.dark.alpha(0.75)));
        }
        p.push(fr.line(vec![(rig.hip.0 - 6.0, rig.hip.1 - 8.0), (rig.hip.0 - 1.0, rig.hip.1 - 3.0)], 2.4, coat.dark.alpha(0.7)));
    }
    if rig.tuck < 0.5 {
        leg(p, hind_joint, rig.feet[2], true, true);
    }
    if rig.tuck < 1.0 {
        leg(p, front_joint, rig.feet[0], true, false);
    } else {
        // Loafing: only the paws peep out under the chest
        for dx in [4.0, 8.0] {
            p.push(fr.shape(ellipse((rig.shoulder.0 + dx, -1.8), 3.2, 2.0, 0.0), if coat.paws { coat.light } else { base }));
        }
    }

    // The neck, then the head
    let neck_base = (rig.shoulder.0 + 3.0, rig.shoulder.1 - 4.0);
    p.push(fr.shape(capsule(neck_base, rig.head, 7.0 * b.heft, 6.0 * b.heft), if panda { coat.dark } else { base }));
    head(p, fr, rig, coat, species, clock, x);
}

/// Asleep, curled round itself, the tail over its front, breathing.
fn curled(p: &mut Vec<Prim>, fr: &Frame, rig: &Rig, coat: &Coat, species: Species, clock: f32, breathe: f32) {
    let b = build(species);
    let base = coat.base;
    let mid = (0.0, -9.0 * b.heft);
    p.push(fr.shape(ellipse((-11.0, -11.0 * breathe * b.heft), 11.0 * breathe * b.heft, 10.5 * breathe * b.heft, 0.0), base));
    p.push(fr.shape(ellipse(mid, 25.0 * b.heft, 10.5 * breathe * b.heft, 0.0), base));
    if coat.pattern == Pattern::Panda {
        // Its black shoulders and the front legs tucked under the chin
        p.push(fr.shape(ellipse((9.0, -10.0 * b.heft), 9.5 * b.heft, 9.5 * breathe * b.heft, 0.0), coat.dark));
    }
    if coat.pattern == Pattern::Tabby || coat.pattern == Pattern::Stripes {
        let (count, width) = if coat.pattern == Pattern::Stripes { (6, 2.8) } else { (4, 2.3) };
        let step = 32.0 / count as f32;
        for i in 0..count {
            let x = -18.0 + step * i as f32;
            p.push(fr.line(vec![(x, -18.5 * breathe * b.heft), (x + 1.5, -11.0)], width, coat.dark.alpha(0.75)));
        }
    }
    // The head resting on its front, eyes shut
    let h = (15.0, -10.0 * b.heft);
    let rest = Rig { head: h, tilt: 0.55, ears: 0.45, eyes: 0.0, mouth: 0.0, ..*rig };
    head(p, fr, &rest, coat, species, clock, &Extras { heart: 0.0, alert: false, asleep: true, happy: false, bubble: None, ground: 0.0, look: (0.0, 0.0) });
    // The tail wrapped round the front, over the paws (a panda's is too short to)
    if b.tail == Tail::Stub {
        return;
    }
    let tip = if matches!(coat.pattern, Pattern::Points | Pattern::Socks | Pattern::Stripes) || b.tail == Tail::Tuft { coat.dark } else { base };
    let r: &[f32] = match b.tail {
        Tail::Brush => &[6.0, 7.5, 8.0, 7.0, 5.0],
        Tail::Tuft => &[3.4, 3.2, 3.0, 3.4, 4.2],
        _ => &[3.4, 3.2, 3.0, 2.8, 2.4],
    };
    let path = [(-22.0, -4.0), (-12.0, -1.5), (0.0, -0.5), (10.0, -1.0), (19.0, -3.0), (24.0, -6.5)];
    for (i, w) in path.windows(2).enumerate() {
        let ra = r[i.min(r.len() - 1)];
        let rb = r[(i + 1).min(r.len() - 1)];
        let colour = match b.tail {
            Tail::Brush if i >= 3 => coat.light,
            Tail::Tuft if i >= 4 => tip,
            Tail::Tuft => base,
            _ if coat.pattern == Pattern::Stripes => if i % 2 == 1 || i == 4 { coat.dark } else { base },
            _ if i >= 3 => tip,
            _ => base,
        };
        p.push(fr.shape(capsule(w[0], w[1], ra, rb), colour));
    }
}

fn tail(p: &mut Vec<Prim>, fr: &Frame, rig: &Rig, coat: &Coat, species: Species, clock: f32, floor: Option<f32>) {
    let b = build(species);
    let fox = b.tail == Tail::Brush;
    if b.tail == Tail::Stub {
        // A panda's: a little round tuft at the back
        let root = (rig.hip.0 - 10.0 * b.heft, rig.hip.1 - 3.0);
        p.push(fr.shape(ellipse(root, 3.6, 3.2, 0.0), coat.base));
        return;
    }
    let segments = 9;
    let length = match b.tail {
        Tail::Brush => 4.3,
        Tail::Tuft => 4.4,
        _ => 4.0 * b.heft,
    };
    let root = (rig.hip.0 - 8.0, rig.hip.1 - 3.0);
    let mut points = vec![root];
    let mut angle = rig.tail;
    for i in 0..segments {
        let k = i as f32 / segments as f32;
        // Two slow swings that never quite line up, so it doesn't tick like a pendulum
        let swing = 0.7 * (clock * 2.2 - i as f32 * 0.5).sin() + 0.45 * (clock * 0.83 + 1.3 - i as f32 * 0.35).sin();
        angle += rig.curl + rig.sway * swing * k * 0.5;
        let last = points[points.len() - 1];
        points.push((last.0 + angle.cos() * length, last.1 + angle.sin() * length));
    }
    let heft = b.heft;
    let tufted = b.tail == Tail::Tuft;
    let radius = |i: usize| -> f32 {
        let k = i as f32 / segments as f32;
        if fox {
            4.0 + 6.0 * (k * std::f32::consts::PI * 0.9).sin()
        } else if tufted && i >= segments - 1 {
            4.2
        } else {
            (3.4 - 1.1 * k) * heft
        }
    };
    if let Some(floor) = floor {
        for (i, point) in points.iter_mut().enumerate() {
            point.1 = point.1.min(floor - radius(i));
        }
    }
    for i in 0..segments {
        let colour = match coat.pattern {
            Pattern::Socks if i >= segments - 2 => coat.light,
            Pattern::Points if i >= 3 => coat.dark,
            Pattern::Tabby if i % 2 == 1 => coat.dark.mix(coat.base, 0.25),
            // A tiger's rings, and its black tip
            Pattern::Stripes if i % 2 == 1 || i == segments - 1 => coat.dark,
            _ if tufted && i >= segments - 1 => coat.dark,
            _ => coat.base,
        };
        let colour = if fox && coat.pattern != Pattern::Socks && i >= segments - 2 { coat.light } else { colour };
        p.push(fr.shape(capsule(points[i], points[i + 1], radius(i), radius(i + 1)), colour));
    }
}

/// The head: ears, skull, muzzle (a fox's snout), eyes, nose, mouth, whiskers; a lion's mane
/// behind it, a tiger's stripes, a panda's eye patches.
fn head(p: &mut Vec<Prim>, fr: &Frame, rig: &Rig, coat: &Coat, species: Species, clock: f32, x: &Extras) {
    let fox = species == Species::Fox;
    let build = build(species);
    let h = rig.head;
    let at = |q: P| turn(h, rig.tilt, q);
    let base = coat.base;
    let points = coat.pattern == Pattern::Points;
    let panda = coat.pattern == Pattern::Panda;
    let ear_colour = if points || panda || coat.pattern == Pattern::Socks { coat.dark } else { base };
    let perk = rig.ears.clamp(0.0, 1.0);
    if let Some(mane) = coat.mane {
        // A shaggy ring round the face, fuller at the back, and down the neck
        let centre = at((-3.0, 1.5));
        let ring: Vec<P> = (0..28)
            .map(|i| {
                let a = i as f32 / 28.0 * std::f32::consts::TAU;
                let shag = if i % 2 == 0 { 15.5 } else { 13.0 };
                let back = 1.0 + 0.18 * (-a.cos()).max(0.0);
                (centre.0 + shag * back * a.cos(), centre.1 + shag * 1.05 * a.sin())
            })
            .collect();
        p.push(fr.shape(ring, mane));
        p.push(fr.shape(ellipse(at((-2.0, 1.0)), 12.5, 12.0, rig.tilt), mane.mix(base, 0.25)));
    }
    // Pointed ears: the far one behind, the near one in front, up or laid back, behind the skull
    if !build.round_ears {
        let tall = build.ear;
        for (i, (a, b)) in [((-0.5, -7.0), (5.5, -5.5)), ((-6.5, -4.5), (-0.5, -7.5))].into_iter().enumerate() {
            let mid = mix(a, b, 0.5);
            let apex = (mid.0 - 4.0 * (1.0 - perk) - if i == 1 { 1.0 } else { -0.5 }, mid.1 - (tall * (0.45 + 0.55 * perk)));
            let colour = if i == 0 { darker(ear_colour, 0.2) } else { ear_colour };
            p.push(fr.shape(vec![at(a), at(apex), at(b)], colour));
            if i == 1 {
                let inner = vec![at(mix(a, apex, 0.25)), at(mix(mid, apex, 0.75)), at(mix(b, apex, 0.25))];
                p.push(fr.shape(inner, coat.inner_ear));
            }
        }
    }
    // The skull
    p.push(fr.shape(ellipse(h, build.skull.0, build.skull.1, rig.tilt).into_iter().collect(), base));
    // Round ears sit on the head, in front of the skull (a panda's black ones show so)
    if build.round_ears {
        // On the skull's edge, so about half of each shows over it
        let r = build.ear;
        let (sw, sh) = build.skull;
        for (i, spot) in [(sw * 0.38, -sh * 0.92), (-sw * 0.42, -sh * 0.98)].into_iter().enumerate() {
            let spot = (spot.0 - 2.5 * (1.0 - perk), spot.1 + 2.0 * (1.0 - perk));
            let colour = if i == 0 { darker(ear_colour, 0.2) } else { ear_colour };
            p.push(fr.shape(ellipse(at(spot), r, r * 0.95, rig.tilt), colour));
            if i == 1 {
                p.push(fr.shape(ellipse(at((spot.0 + 0.3, spot.1 + 0.6)), r * 0.5, r * 0.45, rig.tilt), coat.inner_ear));
            }
        }
    }
    if coat.pattern == Pattern::Stripes {
        // Stripes over the brow and down the cheek
        for dx in [-3.5f32, -0.5, 2.5] {
            p.push(fr.line(vec![at((dx, -8.6)), at((dx + 0.6, -5.4))], 1.5, coat.dark.alpha(0.9)));
        }
        for dy in [0.5f32, 3.5] {
            p.push(fr.line(vec![at((-8.6, dy - 1.0)), at((-4.5, dy)), at((-2.8, dy + 0.8))], 1.5, coat.dark.alpha(0.85)));
        }
    }
    if panda {
        // The eye patches, drooping outward
        p.push(fr.shape(ellipse(at((3.4, -0.6)), 3.6, 2.7, rig.tilt + 0.55), coat.dark));
    }
    if fox {
        // A long snout: dark-tipped, light underneath
        p.push(fr.shape(vec![at((2.5, -4.0)), at((15.5, 1.0)), at((14.0, 3.2)), at((2.0, 5.5))], base));
        p.push(fr.shape(vec![at((3.0, 2.0)), at((14.5, 2.6)), at((3.0, 6.0)), at((-4.0, 5.0))], coat.light));
        p.push(fr.shape(ellipse(at((15.0, 1.4)), 1.9, 1.5, 0.0), coat.nose));
    } else {
        let muzzle = if coat.muzzle { coat.light } else if points { coat.dark } else { base.mix(coat.light, 0.2) };
        if points {
            // The Siamese mask
            p.push(fr.shape(ellipse(at((4.0, 1.5)), 7.0, 6.0, rig.tilt), coat.dark.mix(base, 0.2)));
        }
        if build.round_ears && !panda {
            // A big cat's broad muzzle and wide nose
            p.push(fr.shape(ellipse(at((7.0, 3.0)), 6.0, 4.6, rig.tilt), muzzle));
            p.push(fr.shape(vec![at((9.6, 0.0)), at((12.8, 0.4)), at((11.0, 2.4))], coat.nose));
        } else if panda {
            p.push(fr.shape(ellipse(at((7.0, 3.0)), 5.0, 4.0, rig.tilt), muzzle));
            p.push(fr.shape(ellipse(at((10.6, 1.4)), 1.9, 1.3, rig.tilt), coat.nose));
        } else {
            p.push(fr.shape(ellipse(at((6.5, 2.4)), 4.6, 3.6, rig.tilt), muzzle));
            p.push(fr.shape(vec![at((9.8, 0.2)), at((11.6, 0.6)), at((10.2, 1.9))], coat.nose));
        }
        if coat.pattern == Pattern::Tabby {
            // The M on the brow
            for dx in [-1.5f32, 1.5, 4.0] {
                p.push(fr.line(vec![at((dx, -7.6)), at((dx + 0.4, -4.6))], 1.4, coat.dark.alpha(0.75)));
            }
        }
    }
    // The mouth: a line, or open in a yawn
    if rig.mouth > 0.05 {
        let jaw = if fox { (10.0, 3.6) } else { (8.0, 4.0) };
        p.push(fr.shape(ellipse(at(jaw), 3.0, 3.4 * rig.mouth, rig.tilt), Rgba::rgb(0x3a, 0x1c, 0x22)));
        p.push(fr.shape(ellipse(at((jaw.0, jaw.1 + 1.6 * rig.mouth)), 1.8, 1.2 * rig.mouth, rig.tilt), Rgba::rgb(0xe0, 0x7c, 0x88)));
    } else if !fox {
        p.push(fr.line(vec![at((10.2, 2.2)), at((9.6, 3.4)), at((8.0, 3.8))], 0.7, coat.dark.alpha(0.6)));
    }
    // The eye: an almond with a slit that follows what it watches; shut, a curve
    let eye = if fox { (3.5, -1.8) } else { (3.2, -1.4) };
    let open = rig.eyes.clamp(0.0, 1.0);
    // A panda's eye is small, in its patch
    let size = if panda { 0.6 } else { 1.0 };
    if open > 0.2 && !x.happy {
        p.push(fr.shape(ellipse(at(eye), 2.5 * size, 2.6 * open * size, rig.tilt), coat.eye));
        let pupil = (eye.0 + x.look.0 * 0.8 * size, eye.1 + x.look.1 * 0.5 * size);
        // A slit (cats and foxes) or a round pupil (the big cats, the panda)
        let (pw, ph) = if build.slit { (if rig.ears > 0.95 { 1.3 } else { 0.85 }, 2.1) } else { (1.3, 1.3) };
        p.push(fr.shape(ellipse(at(pupil), pw * size, ph * open * size, rig.tilt), Rgba::rgb(0x0c, 0x0a, 0x0a)));
        p.push(fr.shape(ellipse(at((eye.0 + 0.8 * size, eye.1 - 1.0 * open * size)), 0.55, 0.55, 0.0), Rgba::WHITE.alpha(0.9)));
    } else {
        // Shut: asleep a soft downward curve, happy an upward one
        let lift = if x.happy { -0.8 } else { 0.7 };
        let line = vec![at((eye.0 - 2.4 * size, eye.1)), at((eye.0, eye.1 + lift)), at((eye.0 + 2.4 * size, eye.1))];
        let lid = if panda { coat.light.alpha(0.8) } else { darker(coat.dark, 0.3) };
        p.push(fr.line(line, 1.1, lid));
    }
    if build.whiskers {
        // Whiskers
        for (dy, to) in [(-0.4, (18.0, -1.6)), (1.0, (18.5, 1.2)), (2.2, (17.2, 3.9))] {
            p.push(fr.line(vec![at((9.5, dy)), at(to)], 0.5, coat.light.alpha(0.6)));
        }
    }
    let _ = clock;
}

/// A small songbird: perched, hopping, pecking or flying.
pub fn bird(p: &mut Vec<Prim>, fr: &Frame, rig: &Rig, coat: &Coat, clock: f32, x: &Extras) {
    let body = rig.hip;
    let puff = if rig.ball > 0.5 { 1.5 } else { 0.0 };
    if x.ground < 60.0 {
        let spread = (1.0 - x.ground / 60.0).max(0.2);
        p.push(fr.shape(ellipse((body.0, x.ground + 0.5), 9.0 * spread, 1.8 * spread, 0.0), Rgba::BLACK.alpha(0.2 * spread)));
    }
    let flap = (clock * 17.0).sin();
    let wing_up = rig.wing > 0.5;
    // The far wing, behind, while flying
    if wing_up {
        let root = (body.0 + 1.0, body.1 - 3.0);
        let tip = (root.0 - 6.0, root.1 - 13.0 * flap);
        p.push(fr.shape(vec![root, tip, (root.0 - 9.0, root.1 + 1.0)], darker(coat.base, 0.3)));
    }
    // Legs and toes, perched
    if rig.wing < 0.5 {
        for (i, foot) in rig.feet[..2].iter().enumerate() {
            let top = (body.0 - 1.0 + i as f32 * 2.5, body.1 + 4.0);
            p.push(fr.line(vec![top, *foot], 1.1, coat.nose));
            p.push(fr.line(vec![(foot.0 - 1.5, foot.1), (foot.0 + 2.2, foot.1)], 0.9, coat.nose));
        }
    }
    // The tail feathers
    let root = (body.0 - 7.0, body.1 - 1.0);
    let tail_tip = (root.0 + rig.tail.cos() * 11.0, root.1 + rig.tail.sin() * 11.0);
    let side = (-(rig.tail.sin()) * 2.6, rig.tail.cos() * 2.6);
    p.push(fr.shape(vec![(root.0 + side.0, root.1 + side.1), (tail_tip.0 + side.0 * 1.3, tail_tip.1 + side.1 * 1.3), (tail_tip.0 - side.0 * 1.3, tail_tip.1 - side.1 * 1.3), (root.0 - side.0, root.1 - side.1)], coat.dark));
    // The body, its breast, the head
    p.push(fr.shape(ellipse(body, 9.5 + puff, 7.0 + puff, -0.15), coat.base));
    let breast = if coat.pattern == Pattern::Breast { coat.light } else { coat.light.mix(coat.base, 0.15) };
    p.push(fr.shape(ellipse((body.0 + 3.5, body.1 + 1.5), 6.0 + puff * 0.6, 5.0 + puff * 0.6, 0.3), breast));
    if coat.pattern == Pattern::Streaks {
        for i in 0..3 {
            let sx = body.0 - 5.0 + i as f32 * 3.0;
            p.push(fr.line(vec![(sx, body.1 - 5.0), (sx - 1.6, body.1 - 2.0)], 1.0, coat.dark.alpha(0.8)));
        }
    }
    let h = rig.head;
    let at = |q: P| turn(h, rig.tilt, q);
    p.push(fr.shape(ellipse(h, 5.3, 5.0, 0.0), coat.base));
    if coat.pattern == Pattern::Streaks {
        // A sparrow's grey cheek and brown cap
        p.push(fr.shape(ellipse(at((1.0, 1.5)), 3.4, 2.4, rig.tilt), coat.light.mix(Rgba::rgb(0x9a, 0x98, 0x96), 0.5)));
    }
    if coat.pattern == Pattern::Breast {
        p.push(fr.shape(ellipse(at((2.0, 2.6)), 3.6, 2.6, rig.tilt), coat.light));
    }
    p.push(fr.shape(vec![at((4.2, -1.3)), at((9.0, 0.3)), at((4.2, 1.6))], Rgba::rgb(0x3c, 0x30, 0x26).mix(coat.nose, 0.4)));
    let eye = (1.9, -1.2);
    if rig.eyes > 0.3 && !x.happy {
        p.push(fr.shape(ellipse(at((eye.0 + x.look.0 * 0.3, eye.1)), 1.25, 1.3, 0.0), coat.eye));
        p.push(fr.shape(ellipse(at((eye.0 + 0.4, eye.1 - 0.5)), 0.4, 0.4, 0.0), Rgba::WHITE.alpha(0.9)));
    } else {
        let lift = if x.happy { -0.6 } else { 0.5 };
        p.push(fr.line(vec![at((eye.0 - 1.4, eye.1)), at((eye.0, eye.1 + lift)), at((eye.0 + 1.4, eye.1))], 0.9, coat.dark));
    }
    // The near wing: folded along the side, or beating
    if wing_up {
        let root = (body.0 + 1.0, body.1 - 3.5);
        let tip = (root.0 - 4.0, root.1 - 15.0 * flap);
        p.push(fr.shape(vec![root, tip, (root.0 - 11.0, root.1 + 1.0)], coat.dark.mix(coat.base, 0.3)));
    } else {
        p.push(fr.shape(ellipse((body.0 - 2.5, body.1 - 1.5), 7.0, 4.0, -0.2), coat.dark.mix(coat.base, 0.35)));
        for i in 0..3 {
            let fx = body.0 - 6.5 + i as f32 * 2.2;
            p.push(fr.line(vec![(fx, body.1 + 0.5), (fx - 2.4, body.1 + 1.2)], 0.8, coat.dark.alpha(0.9)));
        }
    }
}

/// A heart, "!", z's and a speech bubble round the animal (`top`: the top of its head, in the
/// surface's coordinates).
pub fn extras(p: &mut Vec<Prim>, t: &Theme, top: P, size: (f32, f32), x: &Extras, clock: f32) {
    if x.asleep {
        for i in 0..2 {
            let rise = (clock * 0.6 + i as f32 * 0.5).fract();
            let font = Font::new(10.0 + rise * 5.0).weight(600);
            p.push(Prim::Text { x: top.0 + 8.0 + rise * 10.0, y: top.1 - 14.0 - rise * 24.0, text: "z".into(), font, color: t.color_text.alpha(0.85 * (1.0 - rise)) });
        }
    }
    if x.heart > 0.0 {
        let k = 1.0 - x.heart / 1.8;
        let r = 7.5 * (k * std::f32::consts::PI).sin().max(0.0).sqrt();
        let (hx, hy) = (top.0 - 14.0, top.1 - 8.0 - k * 18.0);
        let red = Rgba::rgb(0xe8, 0x5a, 0x6e);
        p.push(Prim::Shape { points: ellipse((hx - r * 0.5, hy), r * 0.55, r * 0.55, 0.0), color: red });
        p.push(Prim::Shape { points: ellipse((hx + r * 0.5, hy), r * 0.55, r * 0.55, 0.0), color: red });
        p.push(Prim::Shape { points: vec![(hx - r * 1.04, hy + r * 0.12), (hx, hy + r * 1.2), (hx + r * 1.04, hy + r * 0.12)], color: red });
    }
    if x.alert && x.bubble.is_none() {
        let font = Font::new(17.0).weight(800);
        p.push(Prim::Text { x: top.0 - 3.0, y: top.1 - 30.0, text: "!".into(), font, color: t.primary });
    }
    if let Some((words, alpha)) = x.bubble {
        // A small card with the words, its tail pointing down at the head, kept on the surface
        let font = Font::new(12.0).weight(600);
        let w = 12.0 + words.chars().count() as f32 * 7.0;
        let h = 22.0;
        let bx = (top.0 - w / 2.0 + 10.0).clamp(2.0, size.0 - w - 2.0);
        let by = (top.1 - h - 12.0).max(2.0);
        let card = t.card_deep.alpha(0.95 * alpha);
        p.push(Prim::Rect { x: bx - 1.0, y: by - 1.0, w: w + 2.0, h: h + 2.0, radius: 12.0, color: t.hairline().fade(alpha) });
        p.push(Prim::Rect { x: bx, y: by, w, h, radius: 11.0, color: card });
        let tip = (top.0.clamp(bx + 8.0, bx + w - 8.0), by + h + 6.0);
        p.push(Prim::Shape { points: vec![(tip.0 - 5.0, by + h - 0.5), tip, (tip.0 + 3.0, by + h - 0.5)], color: card });
        p.push(Prim::Text { x: bx + 6.0, y: by + 3.0, text: words.into(), font, color: t.color_text.alpha(alpha) });
    }
}
