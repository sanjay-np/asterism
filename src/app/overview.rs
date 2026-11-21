//! The window overview as the app runs it: opening it, its keys, and carrying a window from
//! one workspace card to another.

use super::*;
use crate::overview::{Carry, Overview};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

impl App {
    pub(super) fn toggle_overview(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Overview).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Overview),
            (_, false) => {
                self.close_menu();
                for kind in [PanelKind::Control, PanelKind::Launcher, PanelKind::Dashboard] {
                    self.close_panel(kind);
                }
                self.overview = Some(Overview::new(&self.model));
                self.open_panel(PanelKind::Overview, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
            }
        }
    }

    pub(super) fn overview_pieces(&mut self, w: f32, h: f32) -> Option<Vec<Piece>> {
        let output = self.model.compositor.focused_output.clone();
        let overview = self.overview.as_mut()?;
        let mut view = crate::overview::View { model: &self.model, theme: &self.theme, text: &mut self.painter.text, requested: &self.requested, requests: Vec::new(), output: &output, w, h };
        let pieces = overview.build(&mut view);
        for request in std::mem::take(&mut view.requests) {
            if let Command::AppIcon(id) = &request {
                self.requested.insert(format!("icon:{id}"));
            }
            self.send(request);
        }
        Some(pieces)
    }

    fn overview_dirty(&mut self) {
        if let Some(index) = self.panel_index(PanelKind::Overview) {
            self.panels[index].surf.dirty = true;
        }
    }

    pub(super) fn overview_action(&mut self, action: &Action) {
        match action {
            Action::OverviewWindow(id) => {
                self.send(Command::FocusWindow(*id));
                self.close_panel(PanelKind::Overview);
            }
            Action::OverviewWorkspace(id) => {
                self.send(Command::FocusWorkspace(crate::model::WorkspaceTarget::Id(*id)));
                self.close_panel(PanelKind::Overview);
            }
            Action::OverviewClose(id) => {
                self.send(Command::CloseWindow(*id));
                self.overview_dirty();
            }
            _ => {}
        }
    }

    /// A press on a window in the overview picks it up (a click is a pick-up that doesn't move).
    pub(super) fn overview_pick(&mut self, piece: &Piece, x: f32, y: f32) -> bool {
        let Some(id) = piece.key.strip_prefix("ov.win.").and_then(|i| i.parse::<u64>().ok()) else { return false };
        let app = self.model.compositor.workspaces.iter().flat_map(|w| &w.windows).find(|w| w.id == id).map(|w| w.app_id.clone()).unwrap_or_default();
        if let Some(overview) = self.overview.as_mut() {
            overview.carry = Some(Carry { window: id, app, from: (x, y), at: (x, y) });
        }
        true
    }

    pub(super) fn overview_motion(&mut self, x: f32, y: f32) {
        if let Some(carry) = self.overview.as_mut().and_then(|o| o.carry.as_mut()) {
            carry.at = (x, y);
            self.overview_dirty();
        }
    }

    /// Let go: barely moved is a click (go to it); over another workspace's card, it moves there.
    pub(super) fn overview_drop(&mut self, x: f32, y: f32) {
        let Some(carry) = self.overview.as_mut().and_then(|o| o.carry.take()) else { return };
        let moved = (x - carry.from.0).hypot(y - carry.from.1);
        if moved < 8.0 {
            return self.overview_action(&Action::OverviewWindow(carry.window));
        }
        let Some(index) = self.panel_index(PanelKind::Overview) else { return };
        let target = self.panels[index]
            .surf
            .pieces
            .iter()
            .filter(|p| x >= p.x && x <= p.x + p.w && y >= p.y && y <= p.y + p.h)
            .find_map(|p| p.key.strip_prefix("ov.ws.").and_then(|id| id.parse::<i64>().ok()));
        let home = self.model.compositor.workspaces.iter().find(|w| w.windows.iter().any(|win| win.id == carry.window)).map(|w| w.id);
        if let Some(target) = target
            && Some(target) != home
        {
            self.send(Command::MoveWindow(carry.window, target));
        }
        self.overview_dirty();
    }

    pub(super) fn overview_key(&mut self, event: &KeyEvent) {
        let Some(overview) = self.overview.as_mut() else { return };
        match event.keysym {
            Keysym::Escape if !overview.query.is_empty() => overview.query.clear(),
            Keysym::Escape => return self.close_panel(PanelKind::Overview),
            Keysym::Return | Keysym::KP_Enter => {
                if let Some(id) = overview.selected {
                    self.overview_action(&Action::OverviewWindow(id));
                }
                return;
            }
            Keysym::Left | Keysym::Up | Keysym::ISO_Left_Tab => overview.step(-1),
            Keysym::Right | Keysym::Down | Keysym::Tab => overview.step(1),
            Keysym::Delete => {
                if let Some(id) = overview.selected {
                    self.send(Command::CloseWindow(id));
                }
            }
            Keysym::BackSpace => {
                overview.query.pop();
            }
            _ => match &event.utf8 {
                Some(text) if !self.modifiers.ctrl && !self.modifiers.alt && text.chars().all(|c| !c.is_control()) => {
                    overview.query.push_str(text);
                    overview.selected = None;
                }
                _ => return,
            },
        }
        self.overview_dirty();
    }
}
