//! The dock as the shell runs it (`crate::dock` decides what's on it and draws it): a Top layer
//! on one edge of the working output (the foot unless `[dock] position` says otherwise), as
//! long as its apps, that takes no room from windows unless it's always out. In out of sight,
//! only a 2-pixel strip along the screen's edge by it takes the pointer; touching it brings
//! the dock out, and it goes in again a moment after the pointer leaves (not while an app is
//! being carried or its menu is open). A left press on an app waits: let
//! go and it's a click, move and the app is carried along the row (pinned apps change places,
//! an open one dropped among them is pinned, a pinned one lifted off is unpinned).

use super::*;
use crate::dock::{self, Carry, Click, Geometry};
use smithay_client_toolkit::seat::pointer::{BTN_LEFT, PointerEvent, PointerEventKind};

#[derive(Default)]
pub(super) struct DockState {
    /// Up (or on its way up) rather than down out of sight.
    pub shown: bool,
    /// The pointer is on it.
    pub inside: bool,
    /// Only the newest wait to go down counts.
    pub hide_gen: u64,
    /// A left press on an app that isn't a click or a carry yet: the app and where.
    pub press: Option<(String, f32, f32)>,
    /// The app being carried, by key (indices shift when windows come and go).
    pub carry: Option<(String, Carry)>,
    pub output: Option<wl_output::WlOutput>,
    /// The edge it was opened on (another one means a new surface).
    pub edge: Option<dock::Edge>,
    /// When the apps' desktop files were last read.
    pub apps_at: Option<Instant>,
}

/// How long it stays after `msg dock` brings it up untouched (after the pointer leaves, it's
/// `[dock] hide_delay`).
const SHOW_FOR: Duration = Duration::from_secs(3);

impl App {
    pub(super) fn dock_items(&self) -> Vec<dock::Item> {
        let mut items = dock::items(&self.config.dock.pinned, &self.model.compositor.workspaces, &self.model.apps);
        if !self.config.dock.running {
            items.retain(|i| i.pinned);
        }
        items
    }

    fn hide_after(&self) -> Duration {
        Duration::from_millis(self.config.dock.hide_delay.clamp(100, 5000) as u64)
    }

    /// Always up, it keeps windows clear of its card (the room above it for names stays free).
    fn dock_zone(&self) -> i32 {
        if self.config.dock.autohide {
            0
        } else {
            let g = self.dock_geometry();
            (g.card_h + g.foot).ceil() as i32
        }
    }

    fn dock_geometry(&self) -> Geometry {
        Geometry::of(&self.config.dock)
    }

    /// The dock comes up when it's wanted and there's a screen for it, and goes when it isn't.
    pub(super) fn sync_dock(&mut self) {
        let open = self.panel_index(PanelKind::Dock).is_some();
        if !self.config.dock.enabled {
            if open {
                self.close_panel(PanelKind::Dock);
            }
            return;
        }
        // Moved to another edge: a new surface, anchored there
        if open && self.dock.edge != Some(self.dock_geometry().edge) {
            self.close_panel(PanelKind::Dock);
            return self.sync_dock();
        }
        if open {
            // The settings may have changed: hiding or not, and the room kept for it
            let zone = self.dock_zone();
            if let Some(index) = self.panel_index(PanelKind::Dock) {
                self.panels[index].surf.dirty = true;
                self.panels[index].surf.last = None;
                if let shell::Role::Layer(layer) = &self.panels[index].role {
                    layer.set_exclusive_zone(zone);
                    layer.commit();
                }
            }
            if !self.config.dock.autohide {
                self.dock_show(true);
            } else if self.dock.shown && !self.dock_busy() {
                self.dock_hide_later(self.hide_after());
            }
            return self.dock_region();
        }
        let Some(output) = self.working_output() else { return };
        self.load_apps();
        let size = dock::surface_size(&self.dock_items(), &self.config.dock, &self.dock_geometry());
        let edge = self.dock_geometry().edge;
        self.dock = DockState { output: Some(output.clone()), apps_at: self.dock.apps_at, shown: !self.config.dock.autohide, edge: Some(edge), ..Default::default() };
        let zone = self.dock_zone();
        // Anchored to one edge only, so the compositor centres it along that edge
        let anchor = match edge {
            dock::Edge::Bottom => Anchor::BOTTOM,
            dock::Edge::Top => Anchor::TOP,
            dock::Edge::Left => Anchor::LEFT,
            dock::Edge::Right => Anchor::RIGHT,
        };
        self.open_panel_on(Some(output), PanelKind::Dock, Layer::Top, anchor, size, (0, 0, 0, 0), KeyboardInteractivity::None, zone);
    }

    /// The desktop files again, now and then (apps get installed): on the services thread.
    fn load_apps(&mut self) {
        if self.dock.apps_at.is_some_and(|at| at.elapsed() < Duration::from_secs(120)) {
            return;
        }
        self.dock.apps_at = Some(Instant::now());
        self.send(Command::LoadApps);
    }

    pub(super) fn redraw_dock(&mut self) {
        if let Some(index) = self.panel_index(PanelKind::Dock) {
            self.panels[index].surf.dirty = true;
        }
    }

    /// Where the pointer is taken: the card (and the room its icons grow into) while it's up,
    /// the strip along the edge while it's down.
    pub(super) fn dock_region(&mut self) {
        let Some(index) = self.panel_index(PanelKind::Dock) else { return };
        let g = self.dock_geometry();
        let (w, h) = (self.panels[index].surf.w, self.panels[index].surf.h);
        let (x0, card_w, _) = dock::layout(&dock::slots(&self.dock_items(), &self.config.dock, None), &g, g.along(w, h));
        let thick = g.thick();
        let (a, c, la, lc) = if self.dock.shown {
            let top = g.room - g.lift();
            (x0, top, card_w, thick - top)
        } else {
            (x0, thick - 2.0, card_w, 2.0)
        };
        self.panel_input(index, &[g.frame().rect(a, c, la, lc)]);
    }

    pub(super) fn dock_pieces(&mut self, index: usize, w: f32, h: f32) -> Vec<Piece> {
        let items = self.dock_items();
        let g = self.dock_geometry();
        let w = g.along(w, h);
        // Apps came or went: a new size, which shows once the compositor agrees
        let want = dock::surface_size(&items, &self.config.dock, &g);
        if want != self.panels[index].want
            && let shell::Role::Layer(layer) = &self.panels[index].role
        {
            layer.set_size(want.0, want.1);
            layer.commit();
            self.panels[index].want = want;
        }
        let carry = self.dock.carry.as_ref().and_then(|(key, c)| items.iter().position(|i| &i.key == key).map(|item| Carry { item, ..*c }));
        let hovered = self.panels[index].surf.hovered.clone();
        let pressed = self.panels[index].surf.pressed.clone();
        let mut anim = std::mem::replace(&mut self.panels[index].surf.anim, Animator::new());
        let mut view = dock::View {
            theme: &self.theme,
            text: &mut self.painter.text,
            anim: &mut anim,
            items: &items,
            settings: &self.config.dock,
            icons: &self.model.app_icons,
            hovered: hovered.as_deref(),
            pressed: pressed.as_deref(),
            shown: self.dock.shown,
            carry,
            trash: self.model.trash,
            w,
            wanted: Vec::new(),
        };
        let pieces = dock::build(&mut view);
        let wanted = std::mem::take(&mut view.wanted);
        self.panels[index].surf.anim = anim;
        for app in wanted {
            if self.requested.insert(format!("icon:{app}")) {
                self.send(Command::AppIcon(app));
            }
        }
        pieces
    }

    fn dock_show(&mut self, on: bool) {
        if self.dock.shown == on {
            return;
        }
        self.dock.shown = on;
        log::debug!("dock {}", if on { "up" } else { "down" });
        if on {
            self.send(Command::CountTrash);
            self.load_apps();
        }
        self.dock_region();
        self.redraw_dock();
    }

    /// Down a moment from now, unless it's in use by then.
    pub(super) fn dock_hide_later(&mut self, after: Duration) {
        self.dock.hide_gen += 1;
        if !self.config.dock.autohide {
            return;
        }
        let generation = self.dock.hide_gen;
        let _ = self.handle.insert_source(Timer::from_duration(after), move |_, _, app| {
            if app.dock.hide_gen == generation && !app.dock_busy() {
                app.dock_show(false);
            }
            TimeoutAction::Drop
        });
    }

    pub(super) fn dock_menu_closed(&mut self) {
        if self.dock.shown && !self.dock.inside {
            self.dock_hide_later(self.hide_after());
        }
    }

    fn dock_busy(&self) -> bool {
        self.dock.inside || self.dock.carry.is_some() || self.dock.press.is_some() || self.desk_menu.as_ref().is_some_and(|m| m.widget.as_deref().is_some_and(|w| w.starts_with("dock:")))
    }

    /// `msg dock [show|hide]`, else up or down.
    pub(super) fn toggle_dock(&mut self, show: Option<bool>) {
        if self.panel_index(PanelKind::Dock).is_none() {
            return;
        }
        // Always up: nothing to bring up or put away
        if !self.config.dock.autohide {
            return;
        }
        let on = show.unwrap_or(!self.dock.shown);
        self.dock_show(on);
        if on && !self.dock.inside {
            self.dock_hide_later(SHOW_FOR);
        }
    }

    /// The dock's own part of the pointer: coming up and going down, presses that may become
    /// carries. True when the event is used up here; else the usual hover and clicks follow.
    pub(super) fn dock_pointer(&mut self, event: &PointerEvent) -> bool {
        let Some(index) = self.panels.iter().position(|p| p.kind == PanelKind::Dock && p.surf.is(&event.surface)) else { return false };
        let (x, y) = (event.position.0 as f32, event.position.1 as f32);
        match event.kind {
            PointerEventKind::Enter { .. } => {
                self.dock.inside = true;
                self.dock.hide_gen += 1;
                self.dock_show(true);
                false
            }
            PointerEventKind::Leave { .. } => {
                self.dock.inside = false;
                if self.dock.carry.is_none() {
                    self.dock.press = None;
                    self.panels[index].surf.pressed = None;
                }
                self.dock_hide_later(self.hide_after());
                false
            }
            PointerEventKind::Motion { .. } => {
                if let Some((key, px, py)) = self.dock.press.clone()
                    && (x - px).hypot(y - py) > 6.0
                {
                    self.dock.press = None;
                    self.panels[index].surf.pressed = None;
                    let (a, c) = self.dock_geometry().frame().point(x, y);
                    self.dock.carry = Some((key, Carry { item: 0, hole: None, x: a, y: c }));
                    self.set_cursor(Shape::Grabbing);
                }
                if self.dock.carry.is_none() {
                    return false;
                }
                // Carrying works along and across the dock, whichever edge it's on
                let items = self.dock_items();
                let g = self.dock_geometry();
                let along = g.along(self.panels[index].surf.w, self.panels[index].surf.h);
                let (a, c) = g.frame().point(x, y);
                if let Some((key, carry)) = self.dock.carry.as_mut() {
                    carry.x = a;
                    carry.y = c;
                    carry.hole = items.iter().position(|i| &i.key == key).and_then(|item| dock::hole(&items, &self.config.dock, &g, along, item, a, c));
                }
                self.panels[index].surf.dirty = true;
                true
            }
            PointerEventKind::Press { button: BTN_LEFT, .. } => {
                let Some(key) = self.panels[index].surf.hit(x, y).map(|p| p.key.clone()) else { return false };
                let Some(app) = key.strip_prefix("dock.app.") else { return false };
                self.dock.press = Some((app.to_string(), x, y));
                self.panels[index].surf.pressed = Some(key);
                self.panels[index].surf.dirty = true;
                true
            }
            PointerEventKind::Release { button: BTN_LEFT, .. } => {
                if let Some((key, carry)) = self.dock.carry.take() {
                    self.set_cursor(Shape::Default);
                    self.dock_drop(&key, carry);
                } else if let Some((key, ..)) = self.dock.press.take() {
                    self.panels[index].surf.pressed = None;
                    self.panels[index].surf.dirty = true;
                    self.dock_click(&key);
                } else {
                    return false;
                }
                if !self.dock.inside {
                    self.dock_hide_later(self.hide_after());
                }
                true
            }
            _ => false,
        }
    }

    /// An app let go of: its new place among the pinned, or off them.
    fn dock_drop(&mut self, key: &str, carry: Carry) {
        self.redraw_dock();
        let items = self.dock_items();
        let Some(item) = items.iter().find(|i| i.key == key) else { return };
        let pinned: Vec<String> = items.iter().filter(|i| i.pinned).map(|i| i.key.clone()).collect();
        let next = match carry.hole {
            Some(at) => dock::repin(&pinned, key, Some(at)),
            None if item.pinned => dock::repin(&pinned, key, None),
            None => return,
        };
        if next != pinned {
            self.set_setting(&["dock".into(), "pinned".into()], &crate::settings::Value::List(next));
        }
    }

    fn dock_click(&mut self, key: &str) {
        let items = self.dock_items();
        let Some(item) = items.iter().find(|i| i.key == key) else { return };
        match dock::click(item) {
            Click::Launch => self.dock_launch(key),
            Click::Focus(id) => self.send(Command::FocusWindow(id)),
        }
    }

    /// Starts the app the way its desktop file says (in a terminal if it asks for one).
    fn dock_launch(&mut self, key: &str) {
        let command = match dock::entry(&self.model.apps, key) {
            Some(entry) => {
                let command = crate::launcher::clean_exec(&entry.exec);
                if entry.terminal { format!("{} -e {command}", crate::launcher::terminal_command(&self.config.launcher.terminal)) } else { command }
            }
            // Pinned by hand as a command name: run it as one, never through a shell's hands
            None if key.chars().all(|c| c.is_alphanumeric() || "._-".contains(c)) && crate::config::command_available(key) => key.to_string(),
            None => {
                self.send(Command::Notify(crate::model::Note { summary: "Couldn't start it".into(), body: format!("No app called “{key}” was found."), ..Default::default() }));
                return;
            }
        };
        crate::launcher::spawn_app(&command);
    }

    /// The app's (or the trash's) menu, over its icon.
    fn dock_menu(&mut self, key: &str) {
        let Some(index) = self.panel_index(PanelKind::Dock) else { return };
        let Some(output) = self.dock.output.clone() else { return };
        let piece_key = if key == dock::TRASH { "dock.trash".to_string() } else { format!("dock.app.{key}") };
        let surf = &self.panels[index].surf;
        let Some(piece) = surf.pieces.iter().find(|p| p.key == piece_key) else { return };
        let Some(card) = surf.pieces.iter().find(|p| p.key == "dock.card") else { return };
        let (cx, cy) = (piece.x + piece.w / 2.0, piece.y + piece.h / 2.0);
        // The surface sits in the middle of its edge, in the room the bar leaves
        let (ow, oh) = self.bars.iter().find(|b| b.output == output).map_or((surf.w, surf.h), |b| (b.output_width.max(b.surf.w), b.output_height));
        let reserve = self.theme.reserve();
        let (above, below) = if self.theme.on_top { (reserve, 0.0) } else { (0.0, reserve) };
        let middle = above + ((oh - above - below - surf.h) / 2.0).round();
        let menu_w = crate::popups::DESKTOP_WIDTH;
        // The menu goes on the card's inner side, by the icon (the desktop menu turns it up
        // when it would run off the foot)
        let at = match self.dock_geometry().edge {
            dock::Edge::Bottom => (((ow - surf.w) / 2.0).round() + cx - menu_w / 2.0, oh - below - surf.h + card.y - 8.0),
            dock::Edge::Top => (((ow - surf.w) / 2.0).round() + cx - menu_w / 2.0, above + card.y + card.h + 8.0),
            dock::Edge::Left => (card.x + card.w + 8.0, middle + cy - 24.0),
            dock::Edge::Right => (ow - surf.w + card.x - menu_w - 8.0, middle + cy - 24.0),
        };
        self.open_desk_menu(output, at, Some(format!("dock:{key}")));
    }

    pub(super) fn dock_act(&mut self, action: Action) {
        match action {
            Action::DockClick(key) => self.dock_click(&key),
            Action::DockLaunch(key) => self.dock_launch(&key),
            Action::DockMenu(key) => self.dock_menu(&key),
            Action::DockPin(key, on) => {
                let mut pinned: Vec<String> = self.dock_items().iter().filter(|i| i.pinned).map(|i| i.key.clone()).collect();
                pinned.retain(|k| *k != key);
                if on {
                    pinned.push(key);
                }
                self.set_setting(&["dock".into(), "pinned".into()], &crate::settings::Value::List(pinned));
            }
            Action::DockClose(key) => {
                let ids: Vec<u64> = self.dock_items().into_iter().find(|i| i.key == key).map(|i| i.windows.iter().map(|w| w.id).collect()).unwrap_or_default();
                for id in ids {
                    self.send(Command::CloseWindow(id));
                }
            }
            Action::DockTrash => dock::open_trash(),
            Action::DockTrashArm => {
                if let Some(menu) = self.desk_menu.as_mut() {
                    menu.widget = Some(format!("dock:{}!", dock::TRASH));
                }
                if let Some(index) = self.panel_index(PanelKind::DeskMenu) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::DockTrashEmpty => self.send(Command::EmptyTrash),
            _ => {}
        }
    }
}
