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

pub(super) struct Desk {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub output: wl_output::WlOutput,
    /// The rectangles that take the pointer, as last given to the compositor.
    pub input: Option<Vec<(i32, i32, i32, i32)>>,
}

/// A widget being moved: on which desk, which one, and where on it it was held.
pub(super) struct WidgetDrag {
    pub desk: u32,
    pub kind: String,
    pub grab: (f32, f32),
}

/// While arranging, widgets land on a grid this fine, so they line up easily.
const SNAP: f32 = 8.0;
/// Each click of the wheel makes a widget this much bigger or smaller.
const STEP: f32 = 0.1;

/// The desktop's menu: where it was asked for, and for which widget.
pub(super) struct DeskMenu {
    pub at: (f32, f32),
    pub widget: Option<String>,
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
        let wl = self.compositor.create_surface(&self.qh);
        let surf = self.new_surf(wl.clone());
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, if self.desk_edit { Layer::Overlay } else { Layer::Bottom }, Some("asterism-desktop"), Some(&output));
        layer.set_anchor(Anchor::all());
        // The whole output: widgets may stand where the bar's room is, too
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        layer.commit();
        self.desks.push(Desk { surf, layer, output, input: None });
    }

    /// Widgets on or off with the config.
    pub(super) fn sync_desks(&mut self) {
        if !self.desks_wanted() {
            for desk in &self.desks {
                self.painter.forget_surface(desk.surf.id);
            }
            self.desks.clear();
            self.watch_spectrum();
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
        // Above every window while arranging, the keyboard on the screen being worked on (for
        // Esc); back under the windows after
        let working = self.working_output();
        for desk in &mut self.desks {
            desk.layer.set_layer(if on { Layer::Overlay } else { Layer::Bottom });
            let keys = if on && working.as_ref() == Some(&desk.output) { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
            desk.layer.set_keyboard_interactivity(keys);
            desk.layer.commit();
            desk.surf.hovered = None;
            desk.surf.dirty = true;
            desk.surf.last = None;
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
        let want = playing && (shown || self.panel_index(PanelKind::Song).is_some());
        if want != self.spectrum_on {
            self.spectrum_on = want;
            self.send(Command::WatchSpectrum(want));
        }
    }

    pub(super) fn draw_desk(&mut self, index: usize) {
        let (w, h) = (self.desks[index].surf.w, self.desks[index].surf.h);
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
        // Only the widgets take the pointer, the rest of the desktop lets it through; while
        // arranging, all of it does
        let rects: Vec<(i32, i32, i32, i32)> =
            if editing { vec![(0, 0, w.ceil() as i32, h.ceil() as i32)] } else { pieces.iter().filter(|p| p.blocks).map(|p| (p.x as i32, p.y as i32, p.w.ceil() as i32, p.h.ceil() as i32)).collect() };
        if self.desks[index].input.as_ref() != Some(&rects)
            && let Ok(region) = Region::new(&self.compositor)
        {
            for &(x, y, w, h) in &rects {
                region.add(x, y, w, h);
            }
            if let Some(wl) = &self.desks[index].surf.wl {
                wl.set_input_region(Some(region.wl_region()));
            }
            self.desks[index].input = Some(rects);
        }
        let background = if editing {
            // The one being carried or pointed at, lit
            let surf = &self.desks[index].surf;
            let active = self.widget_drag.as_ref().map(|d| d.kind.clone()).or_else(|| {
                let hovered = surf.hovered.as_deref()?;
                hovered.strip_prefix("widget.").or_else(|| hovered.strip_prefix("edit.remove.")).map(str::to_string)
            });
            let top = if self.theme.on_top { self.theme.reserve() + 20.0 } else { 28.0 };
            pieces = crate::widgets::editor(pieces, &self.config.widgets.show, &self.widget_places, active.as_deref(), &self.theme, &mut self.painter.text, w, top);
            Some(self.theme.surface.alpha(0.6))
        } else {
            None
        };
        let desk = &mut self.desks[index];
        desk.surf.dirty = false;
        desk.surf.anim.begin();
        if let Some(damage) = shell::render_panel(&mut desk.surf, pieces, &mut self.painter, &self.theme, background, 0.0) {
            present(&mut desk.surf, damage, &mut self.pool, &self.qh);
        }
    }

    /// What's under the pointer on a desk: lit while arranging, and the cursor to match.
    fn desk_hover(&mut self, index: usize, x: f32, y: f32) {
        let surf = &mut self.desks[index].surf;
        let hit = surf.hit(x, y);
        let key = hit.map(|p| p.key.clone());
        let shape = match hit {
            Some(p) if p.key.starts_with("widget.") => Shape::Grab,
            Some(p) if p.actions.left.is_some() => Shape::Pointer,
            _ => Shape::Default,
        };
        if surf.hovered != key {
            surf.hovered = key;
            surf.dirty = true;
        }
        self.set_cursor(shape);
    }

    /// The pointer on a desk: press on a widget to hold it, move to carry it, let go to leave it.
    pub(super) fn desk_pointer(&mut self, index: usize, event: &PointerEvent) {
        let (x, y) = (event.position.0 as f32, event.position.1 as f32);
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
                let surf = &mut self.desks[index].surf;
                if surf.hovered.take().is_some() {
                    surf.dirty = true;
                }
            }
            PointerEventKind::Press { button, serial, .. } => {
                let desk = &self.desks[index];
                let Some(piece) = desk.surf.hit(x, y).cloned() else { return };
                let widget = piece.key.strip_prefix("widget.").map(str::to_string);
                // The edge visualizer stays where it is
                if button == BTN_LEFT
                    && let Some(kind) = widget.as_deref().filter(|k| *k != "edge")
                {
                    self.widget_drag = Some(WidgetDrag { desk: desk.surf.id, kind: kind.to_string(), grab: (x - piece.x, y - piece.y) });
                    self.set_cursor(Shape::Grabbing);
                    return;
                }
                if button == BTN_RIGHT && widget.is_some() && !self.desk_edit {
                    let output = desk.output.clone();
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
                let desk = &self.desks[index];
                let Some(kind) = desk.surf.hit(x, y).and_then(|p| p.key.strip_prefix("widget.")).filter(|k| *k != "edge").map(str::to_string) else { return };
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
                        desk.surf.dirty = true;
                    }
                }
            }
            PointerEventKind::Motion { .. } => {
                let desk = &self.desks[index];
                let Some(drag) = self.widget_drag.as_ref().filter(|d| d.desk == desk.surf.id) else { return self.desk_hover(index, x, y) };
                let (w, h) = (desk.surf.w.max(1.0), desk.surf.h.max(1.0));
                let (mut left, mut top) = (x - drag.grab.0, y - drag.grab.1);
                if self.desk_edit {
                    left = (left / SNAP).round() * SNAP;
                    top = (top / SNAP).round() * SNAP;
                }
                let scale = crate::widgets::place_of(&self.widget_places, &drag.kind).scale;
                let place = crate::widgets::Place { x: (left / w).clamp(0.0, 1.0), y: (top / h).clamp(0.0, 1.0), scale };
                self.widget_places.insert(drag.kind.clone(), place);
                for desk in &mut self.desks {
                    desk.surf.dirty = true;
                }
            }
            PointerEventKind::Release { .. } => {
                self.drop_widget();
                self.desk_hover(index, x, y);
            }
            _ => {}
        }
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
                desk.surf.dirty = true;
            }
        }
    }
}
