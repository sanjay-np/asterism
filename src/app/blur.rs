//! Blur behind the shell's surfaces, done by the compositor (ext-background-effect-v1): the
//! desktop's widgets, the dock, the full-screen launcher, overview and wallpaper picker, the
//! control centre, and notifications (pop-ups and their menu). Cards over a blur are drawn
//! see-through and without a shadow (`Theme::frosted`). Each
//! surface asks with the rectangles of what should be frosted (a rounded card is made of rows,
//! `widgets::rounded_rows`), only when they change.

use super::*;
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

/// Hears whether the compositor can blur behind surfaces (and if that changes).
pub struct BlurCaps;

impl Dispatch2<ExtBackgroundEffectManagerV1, App> for BlurCaps {
    fn event(&self, app: &mut App, _: &ExtBackgroundEffectManagerV1, event: ext_background_effect_manager_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            let can = matches!(flags, smithay_client_toolkit::reexports::client::WEnum::Value(f) if f.contains(Capability::Blur));
            log::debug!("compositor blur behind surfaces: {can}");
            if app.blur_capable != can {
                app.blur_capable = can;
                for desk in &mut app.desks {
                    desk.repaint();
                }
                for panel in &mut app.panels {
                    panel.surf.dirty = true;
                }
            }
        }
    }
}

pub(super) fn bind_blur(globals: &smithay_client_toolkit::reexports::client::globals::GlobalList, qh: &QueueHandle<App>) -> Option<ExtBackgroundEffectManagerV1> {
    globals.bind::<ExtBackgroundEffectManagerV1, App, BlurCaps>(qh, 1..=1, BlurCaps).ok()
}

/// The blur asked for behind one surface: the protocol object (one per surface, ever), and the
/// rectangles last given. Declare it before the surface's layer or window in its struct, so it
/// goes first (fields drop in order).
#[derive(Default)]
pub(super) struct Blur {
    effect: Option<ExtBackgroundEffectSurfaceV1>,
    region: Vec<(i32, i32, i32, i32)>,
}

impl Drop for Blur {
    fn drop(&mut self) {
        if let Some(effect) = self.effect.take() {
            effect.destroy();
        }
    }
}

impl Blur {
    /// Asks for blur behind `wanted` (none: no blur), if that's new; true when it was asked, so
    /// the surface needs a commit. Nothing happens without the compositor's help.
    pub fn set(&mut self, wl: &wl_surface::WlSurface, wanted: Vec<(i32, i32, i32, i32)>, manager: Option<&ExtBackgroundEffectManagerV1>, compositor: &CompositorState, qh: &QueueHandle<App>) -> bool {
        if self.region == wanted {
            return false;
        }
        if self.effect.is_none()
            && !wanted.is_empty()
            && let Some(manager) = manager
        {
            self.effect = Some(manager.get_background_effect(wl, qh, Silent));
        }
        let mut asked = false;
        if let Some(effect) = &self.effect {
            if wanted.is_empty() {
                effect.set_blur_region(None);
                asked = true;
            } else if let Ok(region) = Region::new(compositor) {
                for &(x, y, w, h) in &wanted {
                    region.add(x, y, w, h);
                }
                effect.set_blur_region(Some(region.wl_region()));
                asked = true;
            }
        }
        self.region = wanted;
        asked
    }
}

/// The cards among `pieces` (moved by `dx`, `dy`), as rows to blur behind.
pub(super) fn card_rows(pieces: &[Piece], dx: f32, dy: f32) -> Vec<(i32, i32, i32, i32)> {
    let mut rows = Vec::new();
    for piece in pieces {
        for prim in &piece.prims {
            if let Prim::Card { x, y, w, h, radius } = prim {
                rows.extend(crate::widgets::rounded_rows(piece.x + x + dx, piece.y + y + dy, *w, *h, *radius));
            }
        }
    }
    rows
}

impl App {
    /// Whether blur can be had at all, and is wanted for `what`.
    pub(super) fn blurs(&self, wanted: bool) -> bool {
        wanted && self.blur_capable && self.blur_effects.is_some()
    }

    /// What should be frosted behind a panel: the whole of a full-screen one, the dock's card.
    pub(super) fn panel_blur(&self, kind: PanelKind, pieces: &[Piece], w: f32, h: f32) -> Vec<(i32, i32, i32, i32)> {
        let b = &self.config.blur;
        let whole = vec![(0, 0, w.ceil() as i32, h.ceil() as i32)];
        match kind {
            PanelKind::Launcher if self.blurs(b.launcher) => whole,
            PanelKind::Overview if self.blurs(b.overview) => whole,
            PanelKind::Wallpapers if self.blurs(b.wallpapers) => whole,
            // Their cards (with the frame, the control centre is a drawer joined to it: no blur)
            PanelKind::Control if self.blurs(b.control) => card_rows(pieces, 0.0, 0.0),
            PanelKind::Toasts if self.blurs(b.notifications) => card_rows(pieces, 0.0, 0.0),
            PanelKind::Dock if self.blurs(b.dock) && self.config.dock.background != "none" => {
                let radius = self.dock_geometry().radius(&self.config.dock, self.theme.radius);
                // Only while it's mostly out: it fades as it slides
                pieces.iter().filter(|p| p.key == "dock.card" && p.opacity > 0.5).flat_map(|p| crate::widgets::rounded_rows(p.x, p.y, p.w, p.h, radius)).collect()
            }
            _ => Vec::new(),
        }
    }
}
