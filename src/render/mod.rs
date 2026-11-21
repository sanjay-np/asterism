//! Drawing, on the CPU with tiny-skia. Each piece of the bar is painted once into a picture of
//! its own (with its glow) and only painted again when it changes; a frame is those pictures
//! put together.

pub mod text;

use crate::model::Image;
use crate::theme::{Rgba, Theme};
use crate::ui::{Piece, Prim};
use std::collections::HashMap;
use std::rc::Rc;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

/// How far a glow reaches past the text, in logical pixels.
pub const GLOW_ROOM: f32 = 10.0;

pub struct Painter {
    pub text: text::Text,
    /// Glowing pictures that only move (sliding titles, their icon), drawn once.
    glowing: HashMap<u64, Rc<Pixmap>>,
    glow_color: Rgba,
    glow_strength: f32,
    icons: HashMap<(String, u32, u32), Rc<Pixmap>>,
    images: HashMap<(u64, u32, u32, Option<u32>), Rc<Pixmap>>,
    pieces: HashMap<(u32, String), (u64, Rc<Pixmap>)>,
    pub pack: String,
}

pub fn icon_svg(name: &str, pack: &str) -> Option<&'static [u8]> {
    let find = |table: &'static [(&'static str, &'static [u8])]| table.iter().find(|(n, _)| *n == name).map(|(_, d)| *d);
    match pack {
        "material" => find(embedded::MATERIAL).or_else(|| find(embedded::OUTLINE)),
        "phosphor" => find(embedded::PHOSPHOR).or_else(|| find(embedded::MATERIAL)),
        _ => find(embedded::OUTLINE).or_else(|| find(embedded::MATERIAL)),
    }
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = 0.552_284_8 * r;
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    p.close();
    p.finish()
}

fn solid(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.skia());
    paint.anti_alias = true;
    paint
}

/// A rectangle with its own radius at each corner: top-left, top-right, bottom-right, bottom-left.
pub fn rounded_corners(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<tiny_skia::Path> {
    let cap = (w / 2.0).min(h / 2.0).max(0.0);
    let [tl, tr, br, bl] = r.map(|v| v.clamp(0.0, cap));
    let k = 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(x + tl, y);
    p.line_to(x + w - tr, y);
    p.cubic_to(x + w - tr + tr * k, y, x + w, y + tr - tr * k, x + w, y + tr);
    p.line_to(x + w, y + h - br);
    p.cubic_to(x + w, y + h - br + br * k, x + w - br + br * k, y + h, x + w - br, y + h);
    p.line_to(x + bl, y + h);
    p.cubic_to(x + bl - bl * k, y + h, x, y + h - bl + bl * k, x, y + h - bl);
    p.line_to(x, y + tl);
    p.cubic_to(x, y + tl - tl * k, x + tl - tl * k, y, x + tl, y);
    p.close();
    p.finish()
}

/// A concave corner: the square from (x, y) reaching `dx`, `dy` (±1) by `r`, minus the quarter
/// circle around its far corner.
pub fn fillet_path(x: f32, y: f32, r: f32, dx: f32, dy: f32) -> Option<tiny_skia::Path> {
    let k = 1.0 - 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(x, y);
    p.line_to(x + dx * r, y);
    p.cubic_to(x + dx * r * k, y, x, y + dy * r * k, x, y + dy * r);
    p.close();
    p.finish()
}

/// A drawer: square where it's attached, rounded elsewhere, and curving out into each edge it
/// leaves.
#[allow(clippy::too_many_arguments)]
pub fn draw_drawer(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, radius: f32, attach: u8, color: Rgba, t: Transform) {
    let (top, right, bottom, left) = (attach & 1 != 0, attach & 2 != 0, attach & 4 != 0, attach & 8 != 0);
    let corner = |a: bool, b: bool| if a || b { 0.0 } else { radius };
    if let Some(path) = rounded_corners(x, y, w, h, [corner(top, left), corner(top, right), corner(bottom, right), corner(bottom, left)]) {
        pixmap.fill_path(&path, &solid(color), FillRule::Winding, t, None);
    }
    let r = radius.min(w / 2.0).min(h / 2.0);
    let mut fillets = Vec::new();
    if top {
        if !left {
            fillets.push((x, y, -1.0, 1.0));
        }
        if !right {
            fillets.push((x + w, y, 1.0, 1.0));
        }
    }
    if bottom {
        if !left {
            fillets.push((x, y + h, -1.0, -1.0));
        }
        if !right {
            fillets.push((x + w, y + h, 1.0, -1.0));
        }
    }
    if right {
        if !top {
            fillets.push((x + w, y, -1.0, -1.0));
        }
        if !bottom {
            fillets.push((x + w, y + h, -1.0, 1.0));
        }
    }
    if left {
        if !top {
            fillets.push((x, y, 1.0, -1.0));
        }
        if !bottom {
            fillets.push((x, y + h, 1.0, 1.0));
        }
    }
    for (fx, fy, dx, dy) in fillets {
        if let Some(path) = fillet_path(fx, fy, r, dx, dy) {
            pixmap.fill_path(&path, &solid(color), FillRule::Winding, t, None);
        }
    }
}

/// The four-pointed star: four points curving in towards the middle, `size` from centre to tip.
pub fn star_path(cx: f32, cy: f32, size: f32) -> Option<tiny_skia::Path> {
    let s = size;
    let (x, y) = (cx - s, cy - s);
    let mut p = PathBuilder::new();
    p.move_to(x + s, y);
    p.quad_to(x + s * 1.15, y + s * 0.85, x + s * 2.0, y + s);
    p.quad_to(x + s * 1.15, y + s * 1.15, x + s, y + s * 2.0);
    p.quad_to(x + s * 0.85, y + s * 1.15, x, y + s);
    p.quad_to(x + s * 0.85, y + s * 0.85, x + s, y);
    p.close();
    p.finish()
}

/// A four-pointed star with its halo, for the `world` wallpaper's night sky.
#[allow(clippy::too_many_arguments)]
pub fn draw_star(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, fill: Rgba, halo: f32, glow: Rgba, t: Transform) {
    if size <= 0.05 {
        return;
    }
    if halo > 0.0 {
        let radius = size * 2.4;
        let stops = vec![
            tiny_skia::GradientStop::new(0.0, glow.alpha(0.8 * halo).skia()),
            tiny_skia::GradientStop::new(0.4, glow.alpha(0.24 * halo).skia()),
            tiny_skia::GradientStop::new(1.0, glow.alpha(0.0).skia()),
        ];
        if let Some(shader) = tiny_skia::RadialGradient::new(
            tiny_skia::Point::from_xy(cx, cy),
            0.0,
            tiny_skia::Point::from_xy(cx, cy),
            radius,
            stops,
            tiny_skia::SpreadMode::Pad,
            Transform::identity(),
        ) && let Some(circle) = PathBuilder::from_circle(cx, cy, radius)
        {
            let paint = Paint { shader, anti_alias: true, ..Default::default() };
            pixmap.fill_path(&circle, &paint, FillRule::Winding, t, None);
        }
    }
    if let Some(path) = star_path(cx, cy, size) {
        pixmap.fill_path(&path, &solid(fill), FillRule::Winding, t, None);
    }
}

fn arc(cx: f32, cy: f32, r: f32, start: f32, sweep: f32) -> Option<tiny_skia::Path> {
    let steps = ((sweep.abs() / std::f32::consts::TAU) * 72.0).ceil().max(2.0) as usize;
    let mut p = PathBuilder::new();
    for i in 0..=steps {
        let a = start + sweep * i as f32 / steps as f32;
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 { p.move_to(x, y) } else { p.line_to(x, y) }
    }
    p.finish()
}

/// A card: a soft shadow under it (a few widening layers, no blur needed at this size), the
/// container colour, and a hairline just inside its edge.
#[allow(clippy::too_many_arguments)]
pub fn draw_card(pixmap: &mut Pixmap, theme: &Theme, x: f32, y: f32, w: f32, h: f32, radius: f32, t: Transform) {
    for (grow, drop, alpha) in [(6.0, 6.0, 0.035), (3.5, 4.0, 0.06), (1.5, 2.0, 0.1)] {
        if let Some(shade) = rounded_rect(x - grow, y - grow + drop, w + grow * 2.0, h + grow * 2.0, radius + grow) {
            pixmap.fill_path(&shade, &solid(Rgba::BLACK.alpha(alpha)), FillRule::Winding, t, None);
        }
    }
    if let Some(shape) = rounded_rect(x, y, w, h, radius) {
        pixmap.fill_path(&shape, &solid(theme.card_deep), FillRule::Winding, t, None);
    }
    if let Some(edge) = rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, (radius - 0.5).max(0.0)) {
        pixmap.stroke_path(&edge, &solid(theme.hairline()), &Stroke { width: 1.0, ..Default::default() }, t, None);
    }
}

/// Blurs an alpha channel in place, three box passes each way: close to a gaussian. The
/// vertical pass slides whole rows at once, so it runs on wide registers.
fn blur(alpha: &mut [f32], w: usize, h: usize, radius: usize) {
    if radius == 0 || w == 0 || h == 0 {
        return;
    }
    let inv = 1.0 / (radius * 2 + 1) as f32;
    let mut across = vec![0.0f32; w * h];
    let mut sums = vec![0.0f32; w];
    for _ in 0..3 {
        for (row, out) in alpha.chunks_exact(w).zip(across.chunks_exact_mut(w)) {
            let mut sum: f32 = row[..=radius.min(w - 1)].iter().sum();
            for x in 0..w {
                out[x] = sum * inv;
                if x + radius + 1 < w {
                    sum += row[x + radius + 1];
                }
                if x >= radius {
                    sum -= row[x - radius];
                }
            }
        }
        sums.iter_mut().for_each(|s| *s = 0.0);
        for line in across.chunks_exact(w).take(radius + 1) {
            sums.iter_mut().zip(line).for_each(|(s, v)| *s += v);
        }
        for y in 0..h {
            alpha[y * w..(y + 1) * w].iter_mut().zip(&sums).for_each(|(a, s)| *a = s * inv);
            if y + radius + 1 < h {
                sums.iter_mut().zip(&across[(y + radius + 1) * w..(y + radius + 2) * w]).for_each(|(s, v)| *s += v);
            }
            if y >= radius {
                sums.iter_mut().zip(&across[(y - radius) * w..(y - radius + 1) * w]).for_each(|(s, v)| *s -= v);
            }
        }
    }
}

impl Painter {
    pub fn new(font: &str, pack: &str) -> Self {
        Self {
            text: text::Text::new(font),
            glowing: HashMap::new(),
            glow_color: Rgba::WHITE,
            glow_strength: 0.0,
            icons: HashMap::new(),
            images: HashMap::new(),
            pieces: HashMap::new(),
            pack: pack.to_string(),
        }
    }

    /// Forgets pieces a surface no longer shows.
    /// A surface went away: nothing drawn for it is wanted any more, nor pictures sized for it.
    pub fn forget_surface(&mut self, surface: u32) {
        self.pieces.retain(|(s, _), _| *s != surface);
        self.images.clear();
    }

    pub fn forget(&mut self, surface: u32, keep: &[Piece]) {
        self.pieces.retain(|(s, key), _| *s != surface || keep.iter().any(|p| &p.key == key));
        if self.icons.len() > 300 {
            self.icons.clear();
        }
        if self.images.len() > 100 {
            self.images.clear();
        }
        if self.glowing.len() > 8 {
            self.glowing.clear();
        }
        self.text.tidy();
    }

    fn icon(&mut self, name: &str, px: u32, color: Rgba) -> Option<Rc<Pixmap>> {
        let key = (name.to_string(), px, color.key());
        if let Some(found) = self.icons.get(&key) {
            return Some(found.clone());
        }
        let data = icon_svg(name, &self.pack)?;
        let mut pixmap = crate::images::svg(data, px)?;
        tint(&mut pixmap, color);
        let pixmap = Rc::new(pixmap);
        self.icons.insert(key, pixmap.clone());
        Some(pixmap)
    }

    fn image(&mut self, image: &Image, w: u32, h: u32, tint_with: Option<Rgba>) -> Option<Rc<Pixmap>> {
        let key = (image.id, w, h, tint_with.map(|t| t.key()));
        if let Some(found) = self.images.get(&key) {
            return Some(found.clone());
        }
        let source = Pixmap::from_vec(image.data.clone(), tiny_skia::IntSize::from_wh(image.width, image.height)?)?;
        let mut sized = Pixmap::new(w.max(1), h.max(1))?;
        let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
        sized.draw_pixmap(0, 0, source.as_ref(), &paint, Transform::from_scale(w as f32 / image.width as f32, h as f32 / image.height as f32), None);
        if let Some(color) = tint_with {
            tint(&mut sized, color);
        }
        let sized = Rc::new(sized);
        self.images.insert(key, sized.clone());
        Some(sized)
    }

    /// Draws primitives with (ox, oy) as their origin, logical pixels, at `scale`.
    pub fn draw(&mut self, pixmap: &mut Pixmap, theme: &Theme, prims: &[Prim], ox: f32, oy: f32, scale: f32) {
        let t = Transform::from_scale(scale, scale).pre_translate(ox, oy);
        for prim in prims {
            match prim {
                Prim::Text { x, y, text, font, color } => self.text.draw(pixmap, text, *font, ox + x, oy + y, *color, scale, None),
                Prim::Marquee { x, y, width, text, font, color, offset, gap } => {
                    // The strip (the title twice, with its glow) is drawn once; each frame shows a window of it
                    let (tw, th) = self.text.measure(text, *font);
                    let room = GLOW_ROOM;
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(&(text, font.key(), color.key(), (scale * 64.0) as u32, (*gap * 64.0) as u32), &mut h);
                    let key = std::hash::Hasher::finish(&h);
                    let strip = match self.glowing.get(&key) {
                        Some(strip) => strip.clone(),
                        None => {
                            let w = ((tw * 2.0 + gap + room * 2.0) * scale).ceil() as u32;
                            let h = ((th + room * 2.0) * scale).ceil() as u32;
                            let Some(mut strip) = Pixmap::new(w.max(1), h.max(1)) else { continue };
                            for copy in [0.0, tw + gap] {
                                self.text.draw(&mut strip, text, *font, room + copy, room, *color, scale, None);
                            }
                            if self.glow_strength > 0.0 {
                                strip = glow(strip, self.glow_color, self.glow_strength, (4.0 * scale).round() as usize);
                            }
                            let strip = Rc::new(strip);
                            self.glowing.insert(key, strip.clone());
                            strip
                        }
                    };
                    let left = ((ox + x) * scale).round() as i32;
                    let top = ((oy + y - room) * scale).round() as i32;
                    let source = ((offset + room) * scale).round() as i32;
                    let cols = (width * scale).round() as i32;
                    blend_window(pixmap, &strip, left, top, source, cols, (width * 0.07 * scale).max(1.0));
                }
                Prim::GlowIcon { x, y, size, name, color } => {
                    let px = (size * scale).round() as u32;
                    let room = GLOW_ROOM;
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(&(name, px, color.key()), &mut h);
                    let key = std::hash::Hasher::finish(&h);
                    let picture = match self.glowing.get(&key) {
                        Some(picture) => picture.clone(),
                        None => {
                            let pad = (room * scale).round() as i32;
                            let Some(icon) = self.icon(name, px.max(1), *color) else { continue };
                            let Some(mut picture) = Pixmap::new(px + pad as u32 * 2, px + pad as u32 * 2) else { continue };
                            blend_over(&mut picture, &icon, pad, pad, 1.0);
                            if self.glow_strength > 0.0 {
                                picture = glow(picture, self.glow_color, self.glow_strength, (4.0 * scale).round() as usize);
                            }
                            let picture = Rc::new(picture);
                            self.glowing.insert(key, picture.clone());
                            picture
                        }
                    };
                    let (dx, dy) = (((ox + x - room) * scale).round() as i32, ((oy + y - room) * scale).round() as i32);
                    blend_over(pixmap, &picture, dx, dy, 1.0);
                }
                Prim::Icon { x, y, size, name, color } => {
                    let px = (size * scale).round() as u32;
                    if let Some(icon) = self.icon(name, px.max(1), *color) {
                        let (dx, dy) = (((ox + x) * scale).round() as i32, ((oy + y) * scale).round() as i32);
                        pixmap.draw_pixmap(dx, dy, icon.as_ref().as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                    }
                }
                Prim::Image { x, y, w, h, image, tint: tint_with, radius } => {
                    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
                    if let Some(sized) = self.image(image, pw, ph, *tint_with) {
                        let (dx, dy) = ((ox + x) * scale, (oy + y) * scale);
                        if *radius > 0.0 {
                            let pattern = tiny_skia::Pattern::new(
                                sized.as_ref().as_ref(),
                                tiny_skia::SpreadMode::Pad,
                                tiny_skia::FilterQuality::Nearest,
                                1.0,
                                Transform::from_translate(dx.round(), dy.round()),
                            );
                            if let Some(path) = rounded_rect(dx.round(), dy.round(), pw as f32, ph as f32, radius * scale) {
                                let paint = Paint { shader: pattern, anti_alias: true, ..Default::default() };
                                pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                            }
                        } else {
                            pixmap.draw_pixmap(dx.round() as i32, dy.round() as i32, sized.as_ref().as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                        }
                    }
                }
                Prim::Ring { cx, cy, r, width, value, color } => {
                    let stroke = Stroke { width: *width, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                    if let Some(circle) = PathBuilder::from_circle(*cx, *cy, *r) {
                        pixmap.stroke_path(&circle, &solid(color.fade(0.22)), &stroke, t, None);
                    }
                    let sweep = value.clamp(0.02, 1.0) * std::f32::consts::TAU;
                    if let Some(path) = arc(*cx, *cy, *r, -std::f32::consts::FRAC_PI_2, sweep) {
                        pixmap.stroke_path(&path, &solid(*color), &stroke, t, None);
                    }
                }
                Prim::Area { points, base, color } => {
                    if let (Some(first), Some(last)) = (points.first(), points.last()) {
                        let mut path = PathBuilder::new();
                        path.move_to(first.0, *base);
                        for (x, y) in points {
                            path.line_to(*x, *y);
                        }
                        path.line_to(last.0, *base);
                        path.close();
                        if let Some(path) = path.finish() {
                            pixmap.fill_path(&path, &solid(*color), FillRule::Winding, t, None);
                        }
                    }
                }
                Prim::Line { points, width, color } => {
                    let mut path = PathBuilder::new();
                    for (i, (x, y)) in points.iter().enumerate() {
                        if i == 0 { path.move_to(*x, *y) } else { path.line_to(*x, *y) }
                    }
                    if let Some(path) = path.finish() {
                        let stroke = Stroke { width: *width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                        pixmap.stroke_path(&path, &solid(*color), &stroke, t, None);
                    }
                }
                Prim::Rect { x, y, w, h, radius, color } => {
                    if *w <= 0.01 || *h <= 0.01 {
                        continue;
                    }
                    if let Some(path) = rounded_rect(*x, *y, *w, *h, *radius) {
                        pixmap.fill_path(&path, &solid(*color), FillRule::Winding, t, None);
                    }
                }
                Prim::Fillet { x, y, r, dx, dy, color } => {
                    if let Some(path) = fillet_path(*x, *y, *r, *dx, *dy) {
                        pixmap.fill_path(&path, &solid(*color), FillRule::Winding, t, None);
                    }
                }
                Prim::Drawer { x, y, w, h, radius, attach, color } => draw_drawer(pixmap, *x, *y, *w, *h, *radius, *attach, *color, t),
                Prim::Card { x, y, w, h, radius } => draw_card(pixmap, theme, *x, *y, *w, *h, *radius, t),
            }
        }
    }

    /// A piece's own picture, painted again only if it changed.
    pub fn piece(&mut self, surface: u32, piece: &Piece, theme: &Theme, scale: f32) -> Option<(Rc<Pixmap>, f32)> {
        if self.glow_color != theme.glow || self.glow_strength != theme.glow_strength {
            self.glow_color = theme.glow;
            self.glow_strength = theme.glow_strength;
            self.glowing.clear();
        }
        let hash = piece.content_hash(scale) ^ theme.glow.key() as u64 ^ (theme.glow_strength * 100.0) as u64;
        let room = piece.pad + if piece.glow { GLOW_ROOM } else { 0.0 };
        let key = (surface, piece.key.clone());
        if let Some((h, pixmap)) = self.pieces.get(&key)
            && *h == hash
        {
            return Some((pixmap.clone(), room));
        }
        let w = ((piece.w + room * 2.0) * scale).ceil() as u32;
        let h = ((piece.h + room * 2.0) * scale).ceil() as u32;
        let mut pixmap = Pixmap::new(w.max(1), h.max(1))?;
        self.draw(&mut pixmap, theme, &piece.prims, room, room, scale);
        if piece.glow && theme.glow_strength > 0.0 {
            pixmap = glow(pixmap, theme.glow, theme.glow_strength, (4.0 * scale).round() as usize);
        }
        let pixmap = Rc::new(pixmap);
        self.pieces.insert(key, (hash, pixmap.clone()));
        Some((pixmap, room))
    }
}

/// Lays `src` over `dst` with its top-left at (x, y), premultiplied source-over, faded by
/// `opacity`. Much quicker than a general draw for the common case of whole-pixel offsets.
pub fn blend_over(dst: &mut Pixmap, src: &Pixmap, x: i32, y: i32, opacity: f32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + sw).min(dw), (y + sh).min(dh));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let fade = (opacity.clamp(0.0, 1.0) * 256.0).round() as u32;
    let (from, to) = (src.data(), dst.data_mut());
    for row in y0..y1 {
        let s = ((row - y) * sw + (x0 - x)) as usize * 4;
        let d = (row * dw + x0) as usize * 4;
        let n = (x1 - x0) as usize * 4;
        for (sp, dp) in from[s..s + n].as_chunks::<4>().0.iter().zip(to[d..d + n].as_chunks_mut::<4>().0) {
            let sa = (sp[3] as u32 * fade) >> 8;
            if sa == 0 {
                continue;
            }
            let keep = 255 - sa;
            for c in 0..3 {
                dp[c] = (((sp[c] as u32 * fade) >> 8) + (dp[c] as u32 * keep + 127) / 255).min(255) as u8;
            }
            dp[3] = (sa + (dp[3] as u32 * keep + 127) / 255).min(255) as u8;
        }
    }
}

/// Lays columns `source .. source + cols` of `strip` at (x, y), fading to nothing over `fade`
/// pixels at both ends of the window.
fn blend_window(dst: &mut Pixmap, strip: &Pixmap, x: i32, y: i32, source: i32, cols: i32, fade: f32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (strip.width() as i32, strip.height() as i32);
    let (from, to) = (strip.data(), dst.data_mut());
    for col in 0..cols {
        let (sx, dx) = (source + col, x + col);
        if sx < 0 || sx >= sw || dx < 0 || dx >= dw {
            continue;
        }
        let along = col as f32 + 0.5;
        let alpha = (along / fade).min((cols as f32 - along) / fade).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            continue;
        }
        let fade = (alpha * 256.0) as u32;
        for row in 0..sh {
            let dy = y + row;
            if dy < 0 || dy >= dh {
                continue;
            }
            let s = ((row * sw + sx) * 4) as usize;
            let d = ((dy * dw + dx) * 4) as usize;
            let sa = (from[s + 3] as u32 * fade) >> 8;
            if sa == 0 {
                continue;
            }
            let keep = 255 - sa;
            for c in 0..3 {
                to[d + c] = (((from[s + c] as u32 * fade) >> 8) + (to[d + c] as u32 * keep + 127) / 255).min(255) as u8;
            }
            to[d + 3] = (sa + (to[d + 3] as u32 * keep + 127) / 255).min(255) as u8;
        }
    }
}

/// Paints every pixel in one colour, keeping its coverage.
pub fn tint(pixmap: &mut Pixmap, color: Rgba) {
    for px in pixmap.data_mut().as_chunks_mut::<4>().0 {
        let a = px[3] as f32 / 255.0 * color.a;
        px[0] = (color.r * a * 255.0).round() as u8;
        px[1] = (color.g * a * 255.0).round() as u8;
        px[2] = (color.b * a * 255.0).round() as u8;
        px[3] = (a * 255.0).round() as u8;
    }
}

/// The content with a soft glow in `color` behind it, lifting it off whatever sky is below.
fn glow(content: Pixmap, color: Rgba, strength: f32, radius: usize) -> Pixmap {
    let (w, h) = (content.width() as usize, content.height() as usize);
    let mut alpha: Vec<f32> = content.data().as_chunks::<4>().0.iter().map(|p| p[3] as f32 / 255.0).collect();
    blur(&mut alpha, w, h, radius);
    let mut out = Pixmap::new(w as u32, h as u32).expect("same size");
    for (px, a) in out.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(&alpha) {
        // A little boost so thin text still gets a visible glow
        let a = (a * 1.8).min(1.0) * strength * color.a;
        px[0] = (color.r * a * 255.0) as u8;
        px[1] = (color.g * a * 255.0) as u8;
        px[2] = (color.b * a * 255.0) as u8;
        px[3] = (a * 255.0) as u8;
    }
    blend_over(&mut out, &content, 0, 0, 1.0);
    out
}

