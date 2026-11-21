//! A workspace at a glance: pointing at a page dot on the bar a moment brings up a small card
//! under it with that workspace's windows, where they sit (`cards::peek`). It's a surface of its
//! own that takes no pointer, so the pointer stays on the bar; moving off the dot puts it away.

use super::*;

pub(super) struct Peek {
    pub workspace: i64,
    pub surf: Surf,
    pub layer: LayerSurface,
}

/// How long the pointer rests on a dot before its workspace shows (passing over doesn't).
const PEEK_AFTER: Duration = Duration::from_millis(350);

impl App {
    /// The pointer is on this bar's piece (or none): a page dot starts the wait for its card,
    /// anything else puts the card away.
    pub(super) fn peek_hover(&mut self, bar: usize, key: Option<&str>, centre: f32) {
        let workspace = key.and_then(|k| k.strip_prefix("page.")).and_then(|id| id.parse::<i64>().ok());
        if workspace.is_some() && workspace == self.peek.as_ref().map(|p| p.workspace) {
            return;
        }
        self.close_peek();
        let Some(workspace) = workspace else {
            self.peek_wait = None;
            return;
        };
        // Only a workspace with windows: an empty one has nothing to show but its name
        if !self.model.compositor.workspaces.iter().any(|w| w.id == workspace && !w.windows.is_empty()) {
            self.peek_wait = None;
            return;
        }
        if self.peek_wait.as_ref().is_some_and(|(w, ..)| *w == workspace) {
            return;
        }
        self.peek_wait = Some((workspace, bar, centre));
        let _ = self.handle.insert_source(Timer::from_duration(PEEK_AFTER), move |_, _, app| {
            if app.peek_wait.as_ref().is_some_and(|(w, ..)| *w == workspace) {
                app.open_peek();
            }
            TimeoutAction::Drop
        });
    }

    fn open_peek(&mut self) {
        let Some((workspace, bar, centre)) = self.peek_wait.take() else { return };
        let Some(output) = self.bars.get(bar).map(|b| b.output.clone()) else { return };
        let Some(ws) = self.model.compositor.workspaces.iter().find(|w| w.id == workspace) else { return };
        // The apps' icons, asked for once
        let wanted: Vec<String> = ws.windows.iter().map(|w| w.app_id.clone()).filter(|a| !a.is_empty() && !self.model.app_icons.contains_key(a)).collect();
        for app in wanted {
            if self.requested.insert(format!("icon:{app}")) {
                self.send(Command::AppIcon(app));
            }
        }
        let m = POPUP_MARGIN;
        let (w, h) = (crate::cards::PEEK_WIDTH + m * 2.0, 14.0 + 26.0 + 136.0 + 14.0 + m * 2.0);
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        if let Some(bar) = self.bars.get(bar) {
            surf.scale120 = bar.surf.scale120;
            surf.int_scale = bar.surf.int_scale;
        }
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Overlay, Some("asterism-peek"), Some(&output));
        let output_width = self.bars.get(bar).map_or(1920.0, |b| b.output_width.max(b.surf.w));
        let left = (centre - w / 2.0).clamp(0.0, (output_width - w).max(0.0)).round() as i32;
        // Just off the bar, under its hover label (exclusive zone 0 keeps it clear of the bar)
        if self.theme.on_top {
            layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            layer.set_margin(22, 0, 0, left);
        } else {
            layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
            layer.set_margin(0, 0, 22, left);
        }
        layer.set_size(w as u32, h as u32);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.peek = Some(Peek { workspace, surf, layer });
    }

    pub(super) fn close_peek(&mut self) {
        if let Some(peek) = self.peek.take() {
            self.painter.forget_surface(peek.surf.id);
        }
    }

    pub(super) fn peek_configured(&mut self) {
        let Some(peek) = self.peek.as_mut() else { return };
        peek.surf.w = crate::cards::PEEK_WIDTH + POPUP_MARGIN * 2.0;
        peek.surf.h = 14.0 + 26.0 + 136.0 + 14.0 + POPUP_MARGIN * 2.0;
        peek.surf.configured = true;
        peek.surf.dirty = true;
        peek.surf.frame_pending = false;
        // Clicks go through: the pointer belongs to the bar
        if let Some(wl) = &peek.surf.wl
            && let Ok(region) = Region::new(&self.compositor)
        {
            wl.set_input_region(Some(region.wl_region()));
        }
    }

    pub(super) fn draw_peek(&mut self) {
        let Some(peek) = self.peek.as_mut() else { return };
        if !peek.surf.dirty || !peek.surf.configured || peek.surf.frame_pending {
            return;
        }
        let Some(ws) = self.model.compositor.workspaces.iter().find(|w| w.id == peek.workspace) else { return self.close_peek() };
        let (_, _, mut pieces) = crate::cards::peek(&self.theme, &mut self.painter.text, ws, &self.model.app_icons);
        for piece in &mut pieces {
            piece.x += POPUP_MARGIN;
            piece.y += POPUP_MARGIN;
        }
        peek.surf.dirty = false;
        peek.surf.anim.begin();
        if let Some(damage) = shell::render_panel(&mut peek.surf, pieces, &mut self.painter, &self.theme, None, if self.theme.on_top { -6.0 } else { 6.0 }) {
            present(&mut peek.surf, damage, &mut self.pool, &self.qh);
        }
    }
}
