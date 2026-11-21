//! The desktop's widgets as the app runs them: a bottom-layer surface per output (above the
//! wallpaper, under every window), widgets placed by their saved shares of the screen, only
//! they take the pointer, and a drag moves one (saved when let go).

use super::*;

pub(super) struct Desk {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub output: wl_output::WlOutput,
}

/// A widget being moved: on which desk, which one, and where on it it was held.
pub(super) struct WidgetDrag {
    pub desk: u32,
    pub kind: String,
    pub grab: (f32, f32),
}

impl App {
    pub(super) fn add_desk(&mut self, output: wl_output::WlOutput) {
        if self.config.widgets.show.is_empty() || self.desks.iter().any(|d| d.output == output) {
            return;
        }
        let wl = self.compositor.create_surface(&self.qh);
        let surf = self.new_surf(wl.clone());
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Bottom, Some("asterism-desktop"), Some(&output));
        layer.set_anchor(Anchor::all());
        // The whole output: widgets may stand where the bar's room is, too
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        layer.commit();
        self.desks.push(Desk { surf, layer, output });
    }

    /// Widgets on or off with the config.
    pub(super) fn sync_desks(&mut self) {
        if self.config.widgets.show.is_empty() {
            self.desks.clear();
            return;
        }
        let outputs: Vec<_> = self.outputs.outputs().collect();
        for output in outputs {
            self.add_desk(output);
        }
        for desk in &mut self.desks {
            desk.surf.dirty = true;
            desk.surf.last = None;
        }
    }

    /// Where a widget stands on a desk of this size, kept fully on it.
    fn place(&self, kind: &str, w: f32, h: f32, size: (f32, f32)) -> (f32, f32) {
        let (fx, fy) = self.widget_places.get(kind).copied().unwrap_or_else(|| crate::widgets::default_place(kind));
        ((fx * size.0).clamp(16.0, (size.0 - w - 16.0).max(16.0)).round(), (fy * size.1).clamp(16.0, (size.1 - h - 16.0).max(16.0)).round())
    }

    pub(super) fn draw_desk(&mut self, index: usize) {
        let (w, h) = (self.desks[index].surf.w, self.desks[index].surf.h);
        let mut view = crate::widgets::View {
            model: &self.model,
            theme: &self.theme,
            text: &mut self.painter.text,
            now: chrono::Local::now(),
            lyrics: self.config.widgets.lyrics,
            requested: &self.requested,
            requests: Vec::new(),
        };
        let mut pieces = Vec::new();
        for kind in &self.config.widgets.show {
            if let Some(piece) = crate::widgets::build(kind, &mut view) {
                pieces.push(piece);
            }
        }
        for request in std::mem::take(&mut view.requests) {
            match &request {
                Command::FetchArt(url) => {
                    self.requested.insert(url.clone());
                }
                Command::FetchLyrics { artist, title, .. } => {
                    self.requested.insert(format!("lyrics:{}", crate::widgets::lyrics_key(artist, title)));
                }
                _ => {}
            }
            self.send(request);
        }
        for piece in &mut pieces {
            let kind = piece.key.trim_start_matches("widget.").to_string();
            let (x, y) = self.place(&kind, piece.w, piece.h, (w, h));
            piece.x = x;
            piece.y = y;
        }
        // Only the widgets take the pointer; the rest of the desktop lets it through
        if let Ok(region) = Region::new(&self.compositor) {
            for piece in &pieces {
                region.add(piece.x as i32, piece.y as i32, piece.w.ceil() as i32, piece.h.ceil() as i32);
            }
            if let Some(wl) = &self.desks[index].surf.wl {
                wl.set_input_region(Some(region.wl_region()));
            }
        }
        let desk = &mut self.desks[index];
        desk.surf.dirty = false;
        desk.surf.anim.begin();
        if let Some(damage) = shell::render_panel(&mut desk.surf, pieces, &mut self.painter, &self.theme, None, 0.0) {
            present(&mut desk.surf, damage, &mut self.pool, &self.qh);
        }
    }

    /// The pointer on a desk: press on a widget to hold it, move to carry it, let go to leave it.
    pub(super) fn desk_pointer(&mut self, index: usize, event: &PointerEvent) {
        let (x, y) = (event.position.0 as f32, event.position.1 as f32);
        match event.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_shape = None;
                self.set_cursor(if self.widget_drag.is_some() { Shape::Grabbing } else { Shape::Grab });
            }
            PointerEventKind::Press { button: BTN_LEFT, .. } => {
                let desk = &self.desks[index];
                if let Some(piece) = desk.surf.hit(x, y)
                    && let Some(kind) = piece.key.strip_prefix("widget.")
                {
                    self.widget_drag = Some(WidgetDrag { desk: desk.surf.id, kind: kind.to_string(), grab: (x - piece.x, y - piece.y) });
                    self.set_cursor(Shape::Grabbing);
                }
            }
            PointerEventKind::Motion { .. } => {
                let desk = &self.desks[index];
                let Some(drag) = self.widget_drag.as_ref().filter(|d| d.desk == desk.surf.id) else { return };
                let (w, h) = (desk.surf.w.max(1.0), desk.surf.h.max(1.0));
                let place = (((x - drag.grab.0) / w).clamp(0.0, 1.0), ((y - drag.grab.1) / h).clamp(0.0, 1.0));
                self.widget_places.insert(drag.kind.clone(), place);
                for desk in &mut self.desks {
                    desk.surf.dirty = true;
                }
            }
            PointerEventKind::Release { .. } => self.drop_widget(),
            _ => {}
        }
    }

    /// Let go of a widget being moved: where it is now is kept.
    fn drop_widget(&mut self) {
        if self.widget_drag.take().is_some() {
            crate::widgets::save_places(&self.widget_places);
            self.set_cursor(Shape::Default);
        }
    }
}
