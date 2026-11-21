//! The wallpaper surfaces: one background layer per output, painted with the planet's world
//! (or a picture). Painting happens on a thread of its own; the result comes back through a
//! channel and is sent to the compositor once.

use super::*;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

pub(super) struct Backdrop {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub output: wl_output::WlOutput,
    /// What's on screen now, and what's being painted.
    pub shown: Option<u64>,
    pub painting: Option<u64>,
}

/// A finished wallpaper: for which surface, painted from what, the picture.
pub(super) type Painted = (u32, u64, Pixmap);

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
        self.backdrops.push(Backdrop { surf, layer, output, shown: None, painting: None });
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
            let (id, settings, sender): (u32, _, Sender<Painted>) = (surf.id, self.config.wallpaper.clone(), self.painted.clone());
            let palette = self.theme.palette;
            std::thread::spawn(move || {
                let started = Instant::now();
                if let Some(picture) = crate::world::render(w, h, scale, planet, &settings, &palette) {
                    log::debug!("wallpaper {w}×{h} painted in {:?}", started.elapsed());
                    let _ = sender.send((id, key, picture));
                }
            });
        }
    }

    /// A wallpaper came back from its painter: on screen with it, if it's still the one wanted.
    pub(super) fn painted(&mut self, (id, key, picture): Painted) {
        let Some(backdrop) = self.backdrops.iter_mut().find(|b| b.surf.id == id) else { return };
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
        let full = (0, 0, picture.width() as i32, picture.height() as i32);
        surf.last = Some(LastFrame { canvas: picture, entries: Vec::new(), stars: Vec::new() });
        present(surf, full, &mut self.pool, &self.qh);
        // The compositor has its own copy now; the picture isn't needed until the next change
        surf.last = None;
        surf.buffers.clear();
        crate::images::give_back_memory();
        backdrop.shown = Some(key);
        if let Some(wl) = &surf.wl
            && let Ok(region) = Region::new(&self.compositor)
        {
            // Clicks on the desktop go to the compositor
            wl.set_input_region(Some(region.wl_region()));
            wl.commit();
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
