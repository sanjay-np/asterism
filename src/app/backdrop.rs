//! The wallpaper surfaces: one background layer per output, painted with the planet's world
//! (or a picture). Painting happens on a thread of its own; the result comes back through a
//! channel and is sent to the compositor once.
//!
//! When the picture changes, it changes with an animation (`crate::transition`): the painting
//! thread paints the picture on screen again too (nothing is kept between changes, so the
//! memory stays free) and makes the animation's map; then `step_changes` blends one into the
//! other a frame at a time, from a pool of its own that goes with it.

use super::*;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

pub(super) struct Backdrop {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub output: wl_output::WlOutput,
    /// What's on screen now, and what's being painted.
    pub shown: Option<u64>,
    pub painting: Option<u64>,
    /// What the picture on screen was painted from, to paint it again for the next change.
    pub job: Option<Job>,
    /// A change under way.
    pub change: Option<Change>,
}

/// Everything a wallpaper is painted from.
#[derive(Clone)]
pub(super) struct Job {
    w: u32,
    h: u32,
    scale: f32,
    planet: &'static crate::theme::Planet,
    settings: crate::config::WallpaperSettings,
    palette: crate::palette::Palette,
}

impl Job {
    fn paint(&self) -> Option<Pixmap> {
        crate::world::render(self.w, self.h, self.scale, self.planet, &self.settings, &self.palette)
    }
}

/// One picture giving way to the next.
pub(super) struct Change {
    old: Pixmap,
    new: Pixmap,
    map: Vec<u8>,
    kind: crate::transition::Kind,
    started: Instant,
    length: Duration,
    /// Its frames' buffers: a pool of its own, so the memory goes when the change is over.
    pool: Option<SlotPool>,
    frames: u32,
}

/// A finished wallpaper: for which surface, painted from what, the picture, and, when it should
/// change with an animation, the picture before it with the animation's kind and map.
pub(super) struct Painted {
    id: u32,
    key: u64,
    picture: Pixmap,
    job: Job,
    before: Option<(Pixmap, crate::transition::Kind, Vec<u8>)>,
}

impl App {
    pub(super) fn add_backdrop(&mut self, output: wl_output::WlOutput) {
        if !self.config.wallpaper.enabled || self.backdrops.iter().any(|b| b.output == output) {
            return;
        }
        let Some(info) = self.outputs.info(&output) else { return };
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        surf.int_scale = info.scale_factor;
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Background, Some("asterism-wallpaper"), Some(&output));
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        layer.commit();
        self.backdrops.push(Backdrop { surf, layer, output, shown: None, painting: None, job: None, change: None });
    }

    /// Wallpapers follow the config: on, off, or painted again with new settings.
    pub(super) fn sync_backdrops(&mut self) {
        if !self.config.wallpaper.enabled {
            self.backdrops.clear();
            return;
        }
        let outputs: Vec<_> = self.outputs.outputs().collect();
        for output in outputs {
            self.add_backdrop(output);
        }
        for backdrop in &mut self.backdrops {
            backdrop.surf.dirty = true;
        }
    }

    /// Starts painting the wallpapers whose look changed.
    pub(super) fn paint_backdrops(&mut self) {
        let planet = crate::theme::planet(&self.config.planet);
        // One animation for every screen changing now, a different one from last time
        let mut kind = None;
        let mut picked = false;
        for backdrop in &mut self.backdrops {
            let surf = &mut backdrop.surf;
            if !surf.dirty || !surf.configured || surf.w < 1.0 {
                continue;
            }
            surf.dirty = false;
            let scale = surf.scale();
            let (w, h) = ((surf.w * scale).ceil() as u32, (surf.h * scale).ceil() as u32);
            let key = crate::world::key(w, h, scale, planet, &self.config.wallpaper, &self.theme.palette);
            if backdrop.shown == Some(key) || backdrop.painting == Some(key) {
                continue;
            }
            backdrop.painting = Some(key);
            let job = Job { w, h, scale, planet, settings: self.config.wallpaper.clone(), palette: self.theme.palette };
            // A change of picture on a screen already showing one, at the same size, animates
            let before = backdrop.job.clone().filter(|old| backdrop.shown.is_some() && old.w == w && old.h == h);
            if before.is_some() && !picked {
                picked = true;
                let mut chance = crate::transition::Chance::from_clock();
                kind = crate::transition::pick(&self.config.wallpaper.transition, self.last_transition, &mut chance);
                self.last_transition = kind.map(crate::transition::Kind::name);
            }
            let before = before.zip(kind);
            let (id, sender): (u32, Sender<Painted>) = (surf.id, self.painted.clone());
            std::thread::spawn(move || {
                let started = Instant::now();
                // The picture going away is painted beside the new one, not after it
                let (picture, before) = std::thread::scope(|scope| {
                    let old = scope.spawn(|| before.and_then(|(old, kind)| Some((old.paint()?, kind, crate::transition::map(kind, w, h, scale)))));
                    (job.paint(), old.join().ok().flatten())
                });
                let Some(picture) = picture else { return };
                log::debug!("wallpaper {w}×{h} painted in {:?}", started.elapsed());
                let _ = sender.send(Painted { id, key, picture, job, before });
            });
        }
    }

    /// A wallpaper came back from its painter: on screen with it, if it's still the one wanted
    /// (by way of its animation, when it has one).
    pub(super) fn painted(&mut self, painted: Painted) {
        let Painted { id, key, picture, job, before } = painted;
        let Some(index) = self.backdrops.iter().position(|b| b.surf.id == id) else { return };
        let backdrop = &mut self.backdrops[index];
        if backdrop.painting != Some(key) {
            return;
        }
        backdrop.painting = None;
        let surf = &mut backdrop.surf;
        let scale = surf.scale();
        if picture.width() != (surf.w * scale).ceil() as u32 || picture.height() != (surf.h * scale).ceil() as u32 {
            // The output changed while it was being painted: paint again
            surf.dirty = true;
            return;
        }
        backdrop.shown = Some(key);
        backdrop.job = Some(job);
        match before {
            Some((old, kind, map)) if old.width() == picture.width() && old.height() == picture.height() => {
                log::debug!("wallpaper changes by {}", kind.name());
                let bytes = old.data().len();
                surf.last = Some(LastFrame { canvas: old.clone(), entries: Vec::new(), stars: Vec::new() });
                let length = Duration::from_millis(self.config.wallpaper.transition_ms.clamp(200, 5000) as u64);
                backdrop.change = Some(Change { old, new: picture, map, kind, started: Instant::now(), length, pool: SlotPool::new(bytes * 2, &self.shm).ok(), frames: 0 });
                // The frames come from `step_changes`, in `flush`
            }
            _ => {
                backdrop.change = None;
                self.show_backdrop(index, picture);
            }
        }
    }

    /// A picture on the wallpaper for good: sent once, then let go of (the compositor keeps its
    /// own copy).
    fn show_backdrop(&mut self, index: usize, picture: Pixmap) {
        let surf = &mut self.backdrops[index].surf;
        let full = (0, 0, picture.width() as i32, picture.height() as i32);
        // Buffers from a change's own pool go with it
        surf.buffers.clear();
        surf.last = Some(LastFrame { canvas: picture, entries: Vec::new(), stars: Vec::new() });
        present(surf, full, &mut self.pool, &self.qh);
        surf.last = None;
        surf.buffers.clear();
        crate::images::give_back_memory();
        if let Some(wl) = &surf.wl {
            // The whole wallpaper takes the pointer: a right click on it opens the desktop's
            // menu (`desk::backdrop_pointer`); nothing else is done with clicks there
            wl.set_input_region(None);
            wl.commit();
        }
    }

    /// The next frame of every change under way, as the compositor asks for them. A change
    /// whose screen isn't drawn (the compositor sends no frames for it) still ends on time.
    pub(super) fn step_changes(&mut self) {
        for index in 0..self.backdrops.len() {
            let backdrop = &mut self.backdrops[index];
            let Some(change) = backdrop.change.as_mut() else { continue };
            let t = change.started.elapsed().as_secs_f32() / change.length.as_secs_f32().max(0.001);
            if t >= 1.0 {
                if let Some(change) = backdrop.change.take() {
                    let Change { new, frames, length, kind, .. } = change;
                    log::debug!("wallpaper changed by {} in {frames} frames over {length:?}", kind.name());
                    self.show_backdrop(index, new);
                }
                continue;
            }
            let surf = &mut backdrop.surf;
            if surf.frame_pending || !surf.configured {
                continue;
            }
            let Some(last) = surf.last.as_mut() else { continue };
            let (w, h) = (last.canvas.width(), last.canvas.height());
            crate::transition::compose(last.canvas.data_mut(), change.old.data(), change.new.data(), &change.map, change.kind, crate::transition::ease(t), w, h);
            let pool = change.pool.as_mut().unwrap_or(&mut self.pool);
            present(surf, (0, 0, w as i32, h as i32), pool, &self.qh);
            change.frames += 1;
        }
    }
}

/// One side of the frame around the screen: left, right, or the edge facing the bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Side {
    Left,
    Right,
    /// The bottom, or the top when the bar is at the bottom.
    Far,
}

pub(super) struct Edge {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub output: wl_output::WlOutput,
    pub side: Side,
    pub output_width: f32,
    pub shown: Option<u64>,
}

impl App {
    /// The frame's three other edges for an output, when the style has a frame.
    pub(super) fn add_edges(&mut self, output: wl_output::WlOutput) {
        if !self.theme.framed() || self.theme.frame < 1.0 || self.edges.iter().any(|e| e.output == output) {
            return;
        }
        let Some(info) = self.outputs.info(&output) else { return };
        if !self.config.outputs.is_empty() && !self.config.outputs.iter().any(|o| Some(o) == info.name.as_ref()) {
            return;
        }
        let thickness = self.theme.frame.round() as u32;
        let corners = self.theme.frame_radius.round() as u32;
        let far = if self.theme.on_top { Anchor::BOTTOM } else { Anchor::TOP };
        for side in [Side::Left, Side::Right, Side::Far] {
            let wl = self.compositor.create_surface(&self.qh);
            let mut surf = self.new_surf(wl.clone());
            surf.int_scale = info.scale_factor;
            let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Top, Some("asterism-frame"), Some(&output));
            let (anchor, size) = match side {
                Side::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (thickness, 0)),
                Side::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (thickness, 0)),
                Side::Far => (far | Anchor::LEFT | Anchor::RIGHT, (0, thickness + corners)),
            };
            layer.set_anchor(anchor);
            layer.set_size(size.0, size.1);
            layer.set_exclusive_zone(thickness as i32);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            let output_width = info.logical_size.map_or(0.0, |(w, _)| w as f32);
            self.edges.push(Edge { surf, layer, output: output.clone(), side, output_width, shown: None });
        }
    }

    /// Edges follow the config: there with the frame style, gone without it.
    pub(super) fn sync_edges(&mut self) {
        if !self.theme.framed() || self.theme.frame < 1.0 {
            self.edges.clear();
            return;
        }
        // A new thickness or position: build them again
        self.edges.clear();
        let outputs: Vec<_> = self.outputs.outputs().collect();
        for output in outputs {
            self.add_edges(output);
        }
    }

    /// Paints the edges whose size or colour changed: a solid strip, and for the far edge the
    /// frame's rounded inner corners.
    pub(super) fn paint_edges(&mut self) {
        let t = &self.theme;
        for edge in &mut self.edges {
            let surf = &mut edge.surf;
            if !surf.dirty || !surf.configured || surf.w < 1.0 || surf.h < 1.0 {
                continue;
            }
            surf.dirty = false;
            let scale = surf.scale();
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                ((surf.w * 8.0) as u32, (surf.h * 8.0) as u32, (scale * 120.0) as u32, t.surface.key(), (t.frame_radius * 8.0) as u32, t.on_top).hash(&mut h);
                h.finish()
            };
            if edge.shown == Some(key) {
                continue;
            }
            let Some(mut canvas) = Pixmap::new((surf.w * scale).ceil() as u32, (surf.h * scale).ceil() as u32) else { continue };
            let thickness = t.frame;
            let prims = match edge.side {
                Side::Left | Side::Right => vec![Prim::Rect { x: 0.0, y: 0.0, w: surf.w, h: surf.h, radius: 0.0, color: t.surface }],
                Side::Far => {
                    let r = t.frame_radius.min(surf.h - thickness).max(0.0);
                    // Spanning the whole output, the corners sit inside the side edges; between them, at the ends
                    let inset = if edge.output_width > 0.0 && surf.w >= edge.output_width - 0.5 { thickness } else { 0.0 };
                    let (strip_y, corner_y, dy) = if t.on_top { (surf.h - thickness, surf.h - thickness, -1.0) } else { (0.0, thickness, 1.0) };
                    let mut prims = vec![
                        Prim::Rect { x: 0.0, y: strip_y, w: surf.w, h: thickness, radius: 0.0, color: t.surface },
                        Prim::Fillet { x: inset, y: corner_y, r, dx: 1.0, dy, color: t.surface },
                        Prim::Fillet { x: surf.w - inset, y: corner_y, r, dx: -1.0, dy, color: t.surface },
                    ];
                    if inset > 0.0 {
                        let top = if t.on_top { corner_y - r } else { corner_y };
                        prims.push(Prim::Rect { x: 0.0, y: top, w: inset, h: r, radius: 0.0, color: t.surface });
                        prims.push(Prim::Rect { x: surf.w - inset, y: top, w: inset, h: r, radius: 0.0, color: t.surface });
                    }
                    prims
                }
            };
            self.painter.draw(&mut canvas, t, &prims, 0.0, 0.0, scale);
            let full = (0, 0, canvas.width() as i32, canvas.height() as i32);
            surf.last = Some(LastFrame { canvas, entries: Vec::new(), stars: Vec::new() });
            present(surf, full, &mut self.pool, &self.qh);
            surf.last = None;
            edge.shown = Some(key);
            if let Some(wl) = &surf.wl
                && let Ok(region) = Region::new(&self.compositor)
            {
                // Only the strip takes the pointer, not the corners' room
                match edge.side {
                    Side::Far => {
                        let y = if t.on_top { surf.h - thickness } else { 0.0 };
                        region.add(0, y as i32, surf.w as i32, thickness as i32);
                    }
                    _ => region.add(0, 0, surf.w as i32, surf.h as i32),
                }
                wl.set_input_region(Some(region.wl_region()));
                wl.commit();
            }
        }
    }
}
