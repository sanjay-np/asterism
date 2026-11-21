//! The desktop's widgets as the app runs them: a bottom-layer surface per output (above the
//! wallpaper, under every window), widgets placed by their saved shares of the screen, only
//! they take the pointer, and a drag moves one (saved when let go).
//!
//! Arranging them (`edit_widgets`) lifts the same surfaces above everything, dims the screen,
//! takes the whole pointer and the keyboard, and lays the editor over the widgets: a frame and
//! a remove button on each, a strip to add the others; the wheel on one makes it bigger or
//! smaller. Widgets with nothing to show stand in as placeholders there, so they can be placed
//! before they're needed.
//!
//! A right click on a widget, or on the wallpaper itself (the backdrop's surface), opens the
//! desktop's menu (`PanelKind::DeskMenu`) where it was clicked.

use super::*;

/// One small surface of the desktop: a widget (with, while arranging, its frame and its ×), or
/// the strip at the top while arranging. Placed on the output by its margins.
pub(super) struct Part {
    pub key: String,
    pub surf: Surf,
    pub layer: LayerSurface,
    /// Its top-left on the output, and the size asked of the compositor.
    pub at: (i32, i32),
    pub want: (u32, u32),
    /// The pointer's region as last given: None not given yet, then all of it (None) or these.
    pub input: Option<InputRegion>,
}

/// What takes the pointer on a part: all of it (None), or these rectangles.
type InputRegion = Option<Vec<(i32, i32, i32, i32)>>;

/// The desktop's widgets on one output: a small surface each (a whole-screen surface would hold
/// a full screen of pixels, two or three times over, to show a clock), and while arranging, a
/// dimmed layer under them that takes the keyboard.
pub(super) struct Desk {
    pub output: wl_output::WlOutput,
    pub dirty: bool,
    pub parts: Vec<Part>,
    pub dim: Option<Part>,
}

impl Desk {
    pub fn surfs(&self) -> impl Iterator<Item = &Surf> {
        self.parts.iter().map(|p| &p.surf).chain(self.dim.as_ref().map(|d| &d.surf))
    }

    pub fn has(&self, wl: &wl_surface::WlSurface) -> bool {
        self.surfs().any(|s| s.is(wl))
    }

    pub fn surf_mut(&mut self, wl: &wl_surface::WlSurface) -> Option<&mut Surf> {
        self.parts.iter_mut().map(|p| &mut p.surf).chain(self.dim.as_mut().map(|d| &mut d.surf)).find(|s| s.is(wl))
    }

    pub fn part_mut(&mut self, layer: &LayerSurface) -> Option<&mut Part> {
        self.parts.iter_mut().chain(self.dim.as_mut()).find(|p| &p.layer == layer)
    }

    /// Where a point on one of its surfaces is on the output.
    pub fn to_output(&self, wl: &wl_surface::WlSurface, x: f32, y: f32) -> Option<(f32, f32)> {
        self.parts.iter().chain(self.dim.as_ref()).find(|p| p.surf.is(wl)).map(|p| (x + p.at.0 as f32, y + p.at.1 as f32))
    }

    /// Everything drawn again (colours changed, say).
    pub fn repaint(&mut self) {
        self.dirty = true;
        for part in self.parts.iter_mut().chain(self.dim.as_mut()) {
            part.surf.last = None;
            part.input = None;
        }
    }

    fn ids(&self) -> Vec<u32> {
        self.surfs().map(|s| s.id).collect()
    }
}

/// A widget being moved: on which output, which one, where on it it was held, and where the
/// pointer is on the output. That last is kept from the press on by the pointer's own motion
/// (relative-pointer), not read off the widget's surface: the surface moves with the drag, and
/// positions on it lag behind its moves, which would make the widget run away.
pub(super) struct WidgetDrag {
    pub output: wl_output::WlOutput,
    pub kind: String,
    pub grab: (f32, f32),
    pub at: (f32, f32),
}

/// While arranging, widgets land on a grid this fine, so they line up easily.
const SNAP: f32 = 8.0;
/// Each click of the wheel makes a widget this much bigger or smaller.
const STEP: f32 = 0.1;
/// Room round a widget's surface for its shadow, its hover glow, its frame and ×.
const ROOM: f32 = 12.0;

/// The desktop's menu: where it was asked for, and for which widget.
pub(super) struct DeskMenu {
    pub at: (f32, f32),
    pub widget: Option<String>,
}

/// Which surface a piece goes on: its widget's, or the strip's.
fn group_of(key: &str) -> &str {
    key.strip_prefix("widget.").or_else(|| key.strip_prefix("edit.frame.")).or_else(|| key.strip_prefix("edit.remove.")).unwrap_or("strip")
}

/// A surface filled with one colour by a single pixel the compositor stretches (the viewporter),
/// instead of a whole screen of them.
fn present_fill(surf: &mut Surf, color: crate::theme::Rgba, pool: &mut SlotPool) -> bool {
    let (Some(wl), Some(viewport)) = (surf.wl.clone(), surf.viewport.as_ref()) else { return false };
    let Ok((buffer, bytes)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) else { return false };
    let a = color.a.clamp(0.0, 1.0);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0).round() as u8;
    // (The pool hands out slots rounded up to 64 bytes; the pixel is the first four)
    bytes[..4].copy_from_slice(&[byte(color.b), byte(color.g), byte(color.r), (a * 255.0).round() as u8]);
    wl.set_buffer_scale(1);
    viewport.set_destination(surf.w.round().max(1.0) as i32, surf.h.round().max(1.0) as i32);
    wl.damage_buffer(0, 0, 1, 1);
    if buffer.attach_to(&wl).is_err() {
        return false;
    }
    wl.commit();
    surf.buffers.clear();
    surf.buffers.push((buffer, None));
    true
}

impl App {
    /// Desks are wanted while there are widgets to show, or while they're being arranged.
    fn desks_wanted(&self) -> bool {
        !self.config.widgets.show.is_empty() || self.desk_edit
    }

    pub(super) fn add_desk(&mut self, output: wl_output::WlOutput) {
        if !self.desks_wanted() || self.desks.iter().any(|d| d.output == output) {
            return;
        }
        let dim = self.desk_edit.then(|| self.dim_part(&output));
        self.desks.push(Desk { output, dirty: true, parts: Vec::new(), dim });
    }

    /// A small layer surface for a part of the desktop: under the windows, or above everything
    /// while arranging.
    fn new_part(&mut self, output: &wl_output::WlOutput, key: &str, at: (i32, i32), size: (u32, u32)) -> Part {
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        if let Some(bar) = self.bars.iter().find(|b| &b.output == output) {
            surf.scale120 = bar.surf.scale120;
            surf.int_scale = bar.surf.int_scale;
        }
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, if self.desk_edit { Layer::Overlay } else { Layer::Bottom }, Some("asterism-desktop"), Some(output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_size(size.0, size.1);
        layer.set_margin(at.1, 0, 0, at.0);
        // Anywhere on the output, where the bar's room is too
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        Part { key: key.to_string(), surf, layer, at, want: size, input: None }
    }

    /// The dimmed screen while arranging: the whole output, the keyboard on the working one.
    fn dim_part(&mut self, output: &wl_output::WlOutput) -> Part {
        let wl = self.compositor.create_surface(&self.qh);
        let surf = self.new_surf(wl.clone());
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Overlay, Some("asterism-desktop"), Some(output));
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        let keys = if self.working_output().as_ref() == Some(output) { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
        layer.set_keyboard_interactivity(keys);
        layer.set_size(0, 0);
        layer.commit();
        Part { key: "dim".into(), surf, layer, at: (0, 0), want: (0, 0), input: None }
    }

    /// Widgets on or off with the config.
    pub(super) fn sync_desks(&mut self) {
        if !self.desks_wanted() {
            for desk in std::mem::take(&mut self.desks) {
                for id in desk.ids() {
                    self.painter.forget_surface(id);
                }
            }
            self.watch_spectrum();
            return;
        }
        let outputs: Vec<_> = self.outputs.outputs().collect();
        for output in outputs {
            self.add_desk(output);
        }
        for desk in &mut self.desks {
            desk.repaint();
        }
        self.watch_spectrum();
    }

    /// Starts or finishes arranging the widgets (None: the other way).
    pub(super) fn edit_widgets(&mut self, on: Option<bool>) {
        let on = on.unwrap_or(!self.desk_edit);
        if on == self.desk_edit {
            return;
        }
        self.desk_edit = on;
        self.widget_drag = None;
        if on {
            self.close_menu();
            self.close_panel(PanelKind::Control);
            self.close_panel(PanelKind::Launcher);
        }
        self.sync_desks();
        // Arranging: the dim goes up first, then the widgets come back above it (made anew, on the
        // Overlay layer); finished, they come back under the windows
        for index in 0..self.desks.len() {
            let ids = self.desks[index].ids();
            for id in ids {
                self.painter.forget_surface(id);
            }
            self.desks[index].parts.clear();
            self.desks[index].dim = None;
            if on {
                let output = self.desks[index].output.clone();
                self.desks[index].dim = Some(self.dim_part(&output));
            }
            self.desks[index].dirty = true;
        }
        if !on {
            self.set_cursor(Shape::Default);
        }
    }

    /// Puts a widget on the desktop, or takes one off: the config's list, written.
    pub(super) fn show_widget(&mut self, kind: &str, shown: bool) {
        let mut show = self.config.widgets.show.clone();
        show.retain(|k| k != kind);
        if shown {
            show.push(kind.to_string());
        }
        self.set_setting(&["widgets".into(), "show".into()], &crate::settings::Value::List(show));
    }

    /// Listens to the speakers only while a visualizer shows and something plays.
    pub(super) fn watch_spectrum(&mut self) {
        let playing = self.model.media.player().is_some_and(|p| p.playing);
        let shown = !self.desks.is_empty() && self.config.widgets.show.iter().any(|k| k == "visualizer" || k == "edge");
        let want = playing && (shown || self.panel_index(PanelKind::Song).is_some() || !self.pets.is_empty());
        if want != self.spectrum_on {
            self.spectrum_on = want;
            self.send(Command::WatchSpectrum(want));
        }
    }

    /// The desktop's widgets laid out on the whole output as before, then each drawn on its own
    /// small surface, placed where it stands.
    pub(super) fn draw_desk(&mut self, index: usize) {
        let output = self.desks[index].output.clone();
        let (w, h) = self.output_size(&output);
        let editing = self.desk_edit;
        let mut view = crate::widgets::View {
            model: &self.model,
            theme: &self.theme,
            text: &mut self.painter.text,
            now: chrono::Local::now(),
            lyrics: self.config.widgets.lyrics,
            requested: &self.requested,
            requests: Vec::new(),
            settings: &self.config.widgets,
            photo_step: self.photo_step,
        };
        let mut pieces = crate::widgets::arrange(&self.config.widgets.show, &mut view, &self.widget_places, (w, h), editing);
        for request in std::mem::take(&mut view.requests) {
            match &request {
                Command::FetchArt(url) => {
                    self.requested.insert(url.clone());
                }
                Command::FetchLyrics { artist, title, .. } => {
                    self.requested.insert(format!("lyrics:{}", crate::widgets::lyrics_key(artist, title)));
                }
                Command::Thumbnail(path) => {
                    self.requested.insert(format!("thumb:{}", path.display()));
                }
                Command::ListPhotos(dir) => {
                    self.requested.insert(format!("photos:{}", dir.display()));
                }
                _ => {}
            }
            self.send(request);
        }
        if editing {
            // The one being carried or pointed at, lit
            let active = self.widget_drag.as_ref().map(|d| d.kind.clone()).or_else(|| {
                let hovered = self.desks[index].parts.iter().find_map(|p| p.surf.hovered.clone())?;
                hovered.strip_prefix("widget.").or_else(|| hovered.strip_prefix("edit.remove.")).map(str::to_string)
            });
            let top = if self.theme.on_top { self.theme.reserve() + 20.0 } else { 28.0 };
            pieces = crate::widgets::editor(pieces, &self.config.widgets.show, &self.widget_places, active.as_deref(), &self.theme, &mut self.painter.text, w, top);
        }

        // Pieces by the surface they go on, in drawing order
        let mut groups: Vec<(String, Vec<Piece>)> = Vec::new();
        for piece in pieces {
            let key = group_of(&piece.key).to_string();
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(piece),
                None => groups.push((key, vec![piece])),
            }
        }
        self.desks[index].parts.retain(|p| groups.iter().any(|(k, _)| *k == p.key));
        for (key, mut list) in groups {
            // Its square: everything in it, with room for shadows and glow, kept on the output
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for p in &list {
                let room = p.pad + ROOM;
                x0 = x0.min(p.x - room);
                y0 = y0.min(p.y - room);
                x1 = x1.max(p.x + p.w + room);
                y1 = y1.max(p.y + p.h + room);
            }
            let (x0, y0) = (x0.max(0.0).floor(), y0.max(0.0).floor());
            let (x1, y1) = (x1.min(w).ceil(), y1.min(h).ceil());
            let at = (x0 as i32, y0 as i32);
            let size = ((x1 - x0).max(1.0) as u32, (y1 - y0).max(1.0) as u32);
            let found = self.desks[index].parts.iter().position(|p| p.key == key);
            let part_index = match found {
                Some(i) => i,
                None => {
                    let part = self.new_part(&output, &key, at, size);
                    self.desks[index].parts.push(part);
                    self.desks[index].parts.len() - 1
                }
            };
            let part = &mut self.desks[index].parts[part_index];
            if part.want != size {
                // A new size: asked for, drawn once the compositor agrees
                part.layer.set_size(size.0, size.1);
                part.want = size;
                part.layer.commit();
            }
            let moved = part.at != at;
            if moved {
                part.layer.set_margin(at.1, 0, 0, at.0);
                part.at = at;
            }
            let ready = part.surf.configured && part.surf.w.round() as u32 == size.0 && part.surf.h.round() as u32 == size.1;
            if !ready || part.surf.frame_pending {
                if moved {
                    part.layer.commit();
                }
                // Drawn when its frame (or configure) comes; until then nothing spins on it, which
                // matters: under the windows, a widget's frame may not come for a long while
                part.surf.dirty = true;
                continue;
            }
            for p in &mut list {
                p.x -= x0;
                p.y -= y0;
            }
            // Only the widget takes the pointer (all of it while arranging)
            let input = if editing { None } else { Some(list.iter().filter(|p| p.blocks).map(|p| (p.x as i32, p.y as i32, p.w.ceil() as i32, p.h.ceil() as i32)).collect::<Vec<_>>()) };
            if part.input.as_ref() != Some(&input)
                && let Some(wl) = &part.surf.wl
            {
                match &input {
                    None => wl.set_input_region(None),
                    Some(rects) => {
                        if let Ok(region) = Region::new(&self.compositor) {
                            for &(x, y, w, h) in rects {
                                region.add(x, y, w, h);
                            }
                            wl.set_input_region(Some(region.wl_region()));
                        }
                    }
                }
                part.input = Some(input);
            }
            part.surf.dirty = false;
            part.surf.anim.begin();
            match shell::render_panel(&mut part.surf, list, &mut self.painter, &self.theme, None, 0.0) {
                Some(damage) => present(&mut part.surf, damage, &mut self.pool, &self.qh),
                None => {
                    if moved && let Some(wl) = &part.surf.wl {
                        wl.commit();
                    }
                }
            }
        }
        // The dim behind it all while arranging: one pixel, stretched
        let dim_colour = self.theme.surface.alpha(0.6);
        if let Some(dim) = self.desks[index].dim.as_mut()
            && dim.surf.configured
            && dim.surf.buffers.is_empty()
            && !present_fill(&mut dim.surf, dim_colour, &mut self.pool)
        {
            // No viewporter: a whole screen of it, only while arranging
            dim.surf.anim.begin();
            if let Some(damage) = shell::render_panel(&mut dim.surf, Vec::new(), &mut self.painter, &self.theme, Some(dim_colour), 0.0) {
                present(&mut dim.surf, damage, &mut self.pool, &self.qh);
            }
        }
        self.desks[index].dirty = false;
    }

    /// Whether a desk has something to draw now: news for it, or a part whose turn has come.
    pub(super) fn desk_due(&self, index: usize) -> bool {
        let desk = &self.desks[index];
        desk.dirty || desk.parts.iter().any(|p| p.surf.dirty && p.surf.configured && !p.surf.frame_pending)
    }

    /// The piece under a point on the output, among the desk's parts: which part, and the piece
    /// in the output's coordinates.
    fn desk_hit(&self, index: usize, x: f32, y: f32) -> Option<(usize, Piece)> {
        let desk = &self.desks[index];
        desk.parts.iter().enumerate().rev().find_map(|(i, part)| {
            let mut piece = part.surf.hit(x - part.at.0 as f32, y - part.at.1 as f32)?.clone();
            piece.x += part.at.0 as f32;
            piece.y += part.at.1 as f32;
            Some((i, piece))
        })
    }

    /// What's under the pointer on a desk: lit while arranging, and the cursor to match.
    fn desk_hover(&mut self, index: usize, x: f32, y: f32) {
        let hit = self.desk_hit(index, x, y);
        let shape = match &hit {
            Some((_, p)) if p.key.starts_with("widget.") => Shape::Grab,
            Some((_, p)) if p.actions.left.is_some() => Shape::Pointer,
            _ => Shape::Default,
        };
        let desk = &mut self.desks[index];
        for (i, part) in desk.parts.iter_mut().enumerate() {
            let key = hit.as_ref().filter(|(at, _)| *at == i).map(|(_, p)| p.key.clone());
            if part.surf.hovered != key {
                part.surf.hovered = key;
                desk.dirty = true;
            }
        }
        self.set_cursor(shape);
    }

    /// The pointer on a desk: press on a widget to hold it, move to carry it, let go to leave it.
    pub(super) fn desk_pointer(&mut self, index: usize, event: &PointerEvent) {
        let Some((x, y)) = self.desks[index].to_output(&event.surface, event.position.0 as f32, event.position.1 as f32) else { return };
        match event.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_shape = None;
                if self.widget_drag.is_some() {
                    self.set_cursor(Shape::Grabbing);
                } else {
                    self.desk_hover(index, x, y);
                }
            }
            PointerEventKind::Leave { .. } => {
                let desk = &mut self.desks[index];
                for part in &mut desk.parts {
                    if part.surf.is(&event.surface) && part.surf.hovered.take().is_some() {
                        desk.dirty = true;
                    }
                }
            }
            PointerEventKind::Press { button, serial, .. } => {
                let Some((_, piece)) = self.desk_hit(index, x, y) else { return };
                let widget = piece.key.strip_prefix("widget.").map(str::to_string);
                // The edge visualizer stays where it is
                if button == BTN_LEFT
                    && let Some(kind) = widget.as_deref().filter(|k| *k != "edge")
                {
                    let output = self.desks[index].output.clone();
                    self.widget_drag = Some(WidgetDrag { output, kind: kind.to_string(), grab: (x - piece.x, y - piece.y), at: (x, y) });
                    self.set_cursor(Shape::Grabbing);
                    return;
                }
                if button == BTN_RIGHT && widget.is_some() && !self.desk_edit {
                    let output = self.desks[index].output.clone();
                    return self.open_desk_menu(output, (x, y), widget);
                }
                if button == BTN_LEFT
                    && let Some(action) = piece.actions.left
                {
                    self.act(action, None, serial);
                }
            }
            PointerEventKind::Axis { vertical, .. } if self.desk_edit => {
                // The wheel on a widget, while arranging: bigger (up) or smaller (down)
                let steps = self.wheel_steps(&vertical);
                let Some(kind) = self.desk_hit(index, x, y).and_then(|(_, p)| p.key.strip_prefix("widget.").filter(|k| *k != "edge").map(str::to_string)) else { return };
                if steps == 0 {
                    return;
                }
                let mut place = crate::widgets::place_of(&self.widget_places, &kind);
                let scale = (place.scale - steps as f32 * STEP).clamp(crate::widgets::SMALLEST, crate::widgets::BIGGEST);
                if (scale - place.scale).abs() > 0.001 {
                    place.scale = (scale * 10.0).round() / 10.0;
                    self.widget_places.insert(kind, place);
                    crate::widgets::save_places(&self.widget_places);
                    for desk in &mut self.desks {
                        desk.dirty = true;
                    }
                }
            }
            PointerEventKind::Motion { .. } => {
                let output = self.desks[index].output.clone();
                if !self.widget_drag.as_ref().is_some_and(|d| d.output == output) {
                    return self.desk_hover(index, x, y);
                }
                // With relative motion, the drag follows that instead (see `WidgetDrag`)
                if self.relative.is_none() {
                    self.move_widget(x, y);
                }
            }
            PointerEventKind::Release { .. } => {
                self.drop_widget();
                self.desk_hover(index, x, y);
            }
            _ => {}
        }
    }

    /// The widget being dragged, to where the pointer is now (on its output).
    pub(super) fn move_widget(&mut self, x: f32, y: f32) {
        let Some(drag) = self.widget_drag.as_mut() else { return };
        drag.at = (x, y);
        let (output, kind, grab) = (drag.output.clone(), drag.kind.clone(), drag.grab);
        let (w, h) = self.output_size(&output);
        let (mut left, mut top) = (x - grab.0, y - grab.1);
        if self.desk_edit {
            left = (left / SNAP).round() * SNAP;
            top = (top / SNAP).round() * SNAP;
        }
        let scale = crate::widgets::place_of(&self.widget_places, &kind).scale;
        let place = crate::widgets::Place { x: (left / w.max(1.0)).clamp(0.0, 1.0), y: (top / h.max(1.0)).clamp(0.0, 1.0), scale };
        self.widget_places.insert(kind, place);
        for desk in &mut self.desks {
            desk.dirty = true;
        }
    }

    /// The pointer moved by (dx, dy), whatever surface it's on: drags follow it from here.
    pub(super) fn relative_motion(&mut self, dx: f32, dy: f32) {
        if let Some(drag) = self.widget_drag.as_ref() {
            let (w, h) = self.output_size(&drag.output);
            let at = ((drag.at.0 + dx).clamp(0.0, w), (drag.at.1 + dy).clamp(0.0, h));
            self.move_widget(at.0, at.1);
        }
        self.pet_relative_motion(dx, dy);
    }

    /// The wallpaper under the pointer: a right click there opens the desktop's menu.
    pub(super) fn backdrop_pointer(&mut self, index: usize, event: &PointerEvent) {
        match event.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_shape = None;
                self.set_cursor(Shape::Default);
            }
            PointerEventKind::Press { button: BTN_RIGHT, .. } => {
                let output = self.backdrops[index].output.clone();
                self.open_desk_menu(output, (event.position.0 as f32, event.position.1 as f32), None);
            }
            _ => {}
        }
    }

    /// The desktop's menu where the right click was, on that output.
    pub(super) fn open_desk_menu(&mut self, output: wl_output::WlOutput, at: (f32, f32), widget: Option<String>) {
        self.close_menu();
        self.close_panel(PanelKind::Control);
        self.close_panel(PanelKind::DeskMenu);
        self.desk_menu = Some(DeskMenu { at, widget });
        // The whole output, so a click beside the menu puts it away; Esc too
        self.open_panel_on(Some(output), PanelKind::DeskMenu, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
    }

    /// The desktop's menu, drawn where it was asked for and kept on the screen.
    pub(super) fn desk_menu_pieces(&mut self, index: usize, w: f32, h: f32) -> Option<Vec<Piece>> {
        let menu = self.desk_menu.as_ref()?;
        let mut anim = std::mem::replace(&mut self.panels[index].surf.anim, Animator::new());
        let state = popups::State::default();
        let mut ctx = popups::Ctx {
            model: &self.model,
            config: &self.config,
            theme: &self.theme,
            text: &mut self.painter.text,
            anim: &mut anim,
            state: &state,
            now: chrono::Local::now(),
            requests: Vec::new(),
            requested: &self.requested,
        };
        let built = popups::desktop(&mut ctx, menu.widget.as_deref());
        self.panels[index].surf.anim = anim;
        let x = menu.at.0.min(w - built.w - 8.0).max(8.0).round();
        let y = if menu.at.1 + built.h + 8.0 > h { (menu.at.1 - built.h).max(8.0) } else { menu.at.1 }.round();
        let mut pieces = built.pieces;
        for piece in &mut pieces {
            piece.x += x;
            piece.y += y;
        }
        Some(pieces)
    }

    /// Esc or Enter finishes arranging.
    pub(super) fn desk_key(&mut self, event: &smithay_client_toolkit::seat::keyboard::KeyEvent) {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        if matches!(event.keysym, Keysym::Escape | Keysym::Return | Keysym::KP_Enter) {
            self.edit_widgets(Some(false));
        }
    }

    /// Let go of a widget being moved: where it is now is kept.
    fn drop_widget(&mut self) {
        if self.widget_drag.take().is_some() {
            crate::widgets::save_places(&self.widget_places);
            for desk in &mut self.desks {
                desk.dirty = true;
            }
        }
    }
}
