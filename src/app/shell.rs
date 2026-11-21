//! The shell around the bar: the launcher, notification pop-ups and the volume/brightness level,
//! each a layer-shell panel of its own, and the requests keybinds send through `asterism msg`.

use super::*;
use crate::cards::{self, Level};
use crate::model::Request;
use crate::theme::Rgba;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PanelKind {
    Launcher,
    Toasts,
    Osd,
    Control,
    Auth,
    Wallpapers,
    Shortcuts,
    Clipboard,
    Dashboard,
    Settings,
    Wifi,
    Pair,
    Overview,
    DeskMenu,
    Song,
}

pub(super) struct Panel {
    pub surf: Surf,
    pub role: Role,
    pub kind: PanelKind,
    /// The size asked of the compositor, logical pixels (0 follows the output).
    pub want: (u32, u32),
}

/// What a panel is to the compositor: a layer of the shell, or a window like any app's.
pub(super) enum Role {
    Layer(LayerSurface),
    Window(smithay_client_toolkit::shell::xdg::window::Window),
}

impl Panel {
    pub fn is_layer(&self, layer: &LayerSurface) -> bool {
        matches!(&self.role, Role::Layer(l) if l == layer)
    }

    pub fn is_window(&self, window: &smithay_client_toolkit::shell::xdg::window::Window) -> bool {
        matches!(&self.role, Role::Window(w) if w == window)
    }
}

/// A notification showing as a pop-up, until when.
pub(super) struct Toast {
    pub id: u32,
    pub until: Option<Instant>,
}

/// The launcher's dimmed sky.
const SHADE: Rgba = Rgba { r: 0.02, g: 0.024, b: 0.055, a: 0.6 };
/// How long the level stays after the last change.
const OSD_FOR: Duration = Duration::from_millis(1400);
/// How long a new song's card stays.
const SONG_FOR: Duration = Duration::from_millis(4500);

impl App {
    // ---------------------------------------------------------------- panels

    /// The output being worked on: the compositor says which, else the first bar's.
    pub(super) fn working_output(&self) -> Option<wl_output::WlOutput> {
        let focused = &self.model.compositor.focused_output;
        self.bars.iter().find(|b| &b.output_name == focused).or(self.bars.first()).map(|b| b.output.clone())
    }

    pub(super) fn panel_index(&self, kind: PanelKind) -> Option<usize> {
        self.panels.iter().position(|p| p.kind == kind)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn open_panel(&mut self, kind: PanelKind, layer: Layer, anchor: Anchor, size: (u32, u32), margin: (i32, i32, i32, i32), keyboard: KeyboardInteractivity, exclusive: i32) {
        self.open_panel_on(None, kind, layer, anchor, size, margin, keyboard, exclusive);
    }

    /// A panel on this output (None: the one being worked on).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn open_panel_on(&mut self, output: Option<wl_output::WlOutput>, kind: PanelKind, layer: Layer, anchor: Anchor, size: (u32, u32), margin: (i32, i32, i32, i32), keyboard: KeyboardInteractivity, exclusive: i32) {
        let Some(output) = output.or_else(|| self.working_output()) else { return };
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        if let Some(bar) = self.bars.iter().find(|b| b.output == output) {
            surf.scale120 = bar.surf.scale120;
            surf.int_scale = bar.surf.int_scale;
        }
        let namespace = match kind {
            PanelKind::Launcher => "asterism-launcher",
            PanelKind::Toasts => "asterism-notifications",
            PanelKind::Osd => "asterism-osd",
            PanelKind::Control => "asterism-control",
            PanelKind::Auth => "asterism-polkit",
            PanelKind::Wallpapers => "asterism-wallpapers",
            PanelKind::Shortcuts => "asterism-shortcuts",
            PanelKind::Clipboard => "asterism-clipboard",
            PanelKind::Dashboard => "asterism-dashboard",
            PanelKind::Settings => "asterism-settings",
            PanelKind::Wifi => "asterism-wifi",
            PanelKind::Pair => "asterism-pairing",
            PanelKind::Overview => "asterism-overview",
            PanelKind::DeskMenu => "asterism-desktop-menu",
            PanelKind::Song => "asterism-song",
        };
        let surface = self.layer_shell.create_layer_surface(&self.qh, wl, layer, Some(namespace), Some(&output));
        surface.set_anchor(anchor);
        surface.set_size(size.0, size.1);
        surface.set_margin(margin.0, margin.1, margin.2, margin.3);
        surface.set_keyboard_interactivity(keyboard);
        surface.set_exclusive_zone(exclusive);
        surface.commit();
        self.panels.push(Panel { surf, role: Role::Layer(surface), kind, want: size });
    }

    /// A panel as an ordinary window: the compositor places it, sizes it and decorates it (or
    /// not) like any app's; `size` is what it starts at when the compositor leaves that to us.
    pub(super) fn open_window(&mut self, kind: PanelKind, title: &str, app_id: &str, size: (u32, u32), min: (u32, u32)) {
        use smithay_client_toolkit::shell::xdg::window::WindowDecorations;
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        // The scale of the screen it most likely opens on, until the compositor says
        if let Some(output) = self.working_output()
            && let Some(bar) = self.bars.iter().find(|b| b.output == output)
        {
            surf.scale120 = bar.surf.scale120;
            surf.int_scale = bar.surf.int_scale;
        }
        let window = self.xdg.create_window(wl, WindowDecorations::RequestServer, &self.qh);
        window.set_title(title);
        window.set_app_id(app_id);
        window.set_min_size(Some(min));
        window.commit();
        self.panels.push(Panel { surf, role: Role::Window(window), kind, want: size });
    }

    /// Lets the compositor move the window this press was on (its title area was held).
    pub(super) fn move_window(&mut self, serial: u32) {
        let Some(seat) = self.seat.clone() else { return };
        for panel in &self.panels {
            if let Role::Window(window) = &panel.role
                && panel.surf.pointer.is_some()
            {
                window.move_(&seat, serial);
            }
        }
    }

    /// The settings page follows the wheel.
    pub(super) fn scroll_settings(&mut self, pixels: f32) {
        let Some(settings) = self.settings.as_mut() else { return };
        let to = (settings.scroll + pixels).clamp(0.0, settings.max_scroll);
        if (to - settings.scroll).abs() > 0.1 {
            settings.scroll = to;
            if let Some(index) = self.panel_index(PanelKind::Settings) {
                self.panels[index].surf.dirty = true;
            }
        }
    }

    pub(super) fn close_panel(&mut self, kind: PanelKind) {
        for panel in self.panels.iter().filter(|p| p.kind == kind) {
            self.painter.forget_surface(panel.surf.id);
        }
        self.panels.retain(|p| p.kind != kind);
        if kind == PanelKind::Control {
            self.control_state = popups::State::default();
            self.drag = None;
        }
        if kind == PanelKind::Toasts {
            self.toasts_hovered = false;
        }
        if kind == PanelKind::Wallpapers {
            self.forget_picker();
        }
        if kind == PanelKind::Shortcuts {
            self.sheet = None;
        }
        if kind == PanelKind::Clipboard {
            self.clip_picker = None;
        }
        if kind == PanelKind::Settings {
            self.settings = None;
        }
        if kind == PanelKind::Overview {
            self.overview = None;
        }
        if kind == PanelKind::Launcher {
            // Every app's icon was loaded for the grid: only those with windows open stay (the
            // overview and the bar's labels use them); the rest come back with the launcher
            let open: HashSet<String> = self.model.compositor.workspaces.iter().flat_map(|w| &w.windows).map(|w| w.app_id.clone()).collect();
            self.model.app_icons.retain(|app, _| open.contains(app));
            self.requested.retain(|key| key.strip_prefix("icon:").is_none_or(|app| open.contains(app)));
            crate::images::give_back_memory();
        }
        if kind == PanelKind::DeskMenu {
            self.desk_menu = None;
        }
        if kind == PanelKind::Song {
            self.watch_spectrum();
        }
        if kind == PanelKind::Dashboard && self.dashboard.take().is_some() {
            self.send(Command::WatchProcesses(false));
        }
    }

    /// Lets only these rectangles of a panel take the pointer (none: clicks go through).
    fn panel_input(&self, index: usize, rects: &[(f32, f32, f32, f32)]) {
        let panel = &self.panels[index];
        let Some(wl) = &panel.surf.wl else { return };
        if let Ok(region) = Region::new(&self.compositor) {
            for &(x, y, w, h) in rects {
                region.add(x as i32, y as i32, w.ceil() as i32, h.ceil() as i32);
            }
            wl.set_input_region(Some(region.wl_region()));
        }
    }

    pub(super) fn draw_panel(&mut self, index: usize) {
        let kind = self.panels[index].kind;
        self.panels[index].surf.dirty = false;
        self.panels[index].surf.anim.begin();
        let (w, h) = (self.panels[index].surf.w, self.panels[index].surf.h);
        let m = POPUP_MARGIN;
        let (pieces, background, rise) = match kind {
            PanelKind::Launcher => {
                let mut view = crate::launcher::View {
                    theme: &self.theme,
                    text: &mut self.painter.text,
                    model: &self.model,
                    requested: &self.requested,
                    requests: Vec::new(),
                    w,
                    h,
                };
                let pieces = self.launcher.build(&mut view);
                for request in std::mem::take(&mut view.requests) {
                    if let Command::AppIcon(id) = &request {
                        self.requested.insert(format!("icon:{id}"));
                    }
                    self.send(request);
                }
                // The whole screen, the wallpaper only just showing through
                (pieces, Some(self.theme.surface.alpha(0.9)), -10.0)
            }
            PanelKind::Wifi => {
                let Some(pieces) = self.wifi_pieces(w, h) else { return };
                (pieces, Some(SHADE), -10.0)
            }
            PanelKind::Overview => {
                let Some(pieces) = self.overview_pieces(w, h) else { return };
                (pieces, Some(self.theme.surface.alpha(0.93)), -10.0)
            }
            PanelKind::DeskMenu => {
                let Some(pieces) = self.desk_menu_pieces(index, w, h) else { return };
                (pieces, None, -6.0)
            }
            PanelKind::Song => {
                let Some(player) = self.model.media.player() else { return };
                let art = self.model.art.get(&player.art_url).cloned();
                if art.is_none() && !player.art_url.is_empty() && self.requested.insert(player.art_url.clone()) {
                    self.send(Command::FetchArt(player.art_url.clone()));
                }
                let player = player.clone();
                let mut pieces = cards::song(&self.theme, &mut self.painter.text, &player, art, &self.model.spectrum);
                for piece in &mut pieces {
                    piece.x += m;
                    piece.y += m;
                }
                (pieces, None, if self.theme.on_top { -10.0 } else { 10.0 })
            }
            PanelKind::Pair => {
                let Some((pairing, typed)) = self.pair.as_ref() else { return };
                (cards::pair_card(&self.theme, &mut self.painter.text, pairing, typed, w, h), Some(SHADE), -10.0)
            }
            PanelKind::Settings => {
                let Some(settings) = self.settings.as_mut() else { return };
                (settings.build(&self.config, &self.model, &self.theme, &mut self.painter.text, w, h), Some(self.theme.surface), 0.0)
            }
            PanelKind::Dashboard => {
                let Some(dashboard) = self.dashboard.as_ref() else { return };
                (dashboard.build(&self.model, &self.theme, &mut self.painter.text, w, 8.0), None, if self.theme.on_top { -10.0 } else { 10.0 })
            }
            PanelKind::Clipboard => {
                let Some(picker) = self.clip_picker.as_mut() else { return };
                (picker.build(&self.model.clipboard, &self.theme, &mut self.painter.text, w, h), Some(SHADE), -10.0)
            }
            PanelKind::Shortcuts => {
                let Some(sheet) = self.sheet.as_ref() else { return };
                (sheet.build(&self.theme, &mut self.painter.text, w, h), Some(self.theme.surface.alpha(0.94)), -10.0)
            }
            PanelKind::Wallpapers => {
                let Some(picker) = self.picker.as_mut() else { return };
                let current = crate::config::expand_home(self.config.wallpaper.image.trim());
                let mut view = crate::wallpapers::View {
                    theme: &self.theme,
                    text: &mut self.painter.text,
                    model: &self.model,
                    requested: &self.requested,
                    requests: Vec::new(),
                    current: &current,
                    w,
                    h,
                };
                let pieces = picker.build(&mut view);
                for request in std::mem::take(&mut view.requests) {
                    if let Command::Thumbnail(path) = &request {
                        self.requested.insert(format!("thumb:{}", path.display()));
                    }
                    self.send(request);
                }
                (pieces, Some(self.theme.surface.alpha(0.92)), -10.0)
            }
            PanelKind::Toasts => {
                let list: Vec<&Notification> = self.toasts.iter().filter_map(|toast| self.model.notifications.list.iter().find(|n| n.id == toast.id)).collect();
                let (_, _, mut pieces) = cards::toasts(&self.theme, &mut self.painter.text, &list);
                for piece in &mut pieces {
                    piece.x += m;
                    piece.y += m;
                }
                (pieces, None, if self.theme.on_top { -8.0 } else { 8.0 })
            }
            PanelKind::Auth => {
                let Some(request) = self.model.auth.clone() else { return };
                let pieces = cards::auth(&self.theme, &mut self.painter.text, &request, self.password.len(), w, h);
                (pieces, Some(SHADE), -10.0)
            }
            PanelKind::Control => {
                let mut anim = std::mem::replace(&mut self.panels[index].surf.anim, Animator::new());
                let mut ctx = popups::Ctx {
                    model: &self.model,
                    config: &self.config,
                    theme: &self.theme,
                    text: &mut self.painter.text,
                    anim: &mut anim,
                    state: &self.control_state,
                    now: chrono::Local::now(),
                    requests: Vec::new(),
                    requested: &self.requested,
                };
                let framed = self.theme.framed();
                let mut built = popups::control(&mut ctx, if framed { h } else { h - 24.0 });
                let requests = std::mem::take(&mut ctx.requests);
                self.panels[index].surf.anim = anim;
                for request in requests {
                    if let Command::FetchArt(url) = &request {
                        self.requested.insert(url.clone());
                    }
                    self.send(request);
                }
                // At the right edge, under the bar (the bar's space is already left out): with
                // the frame it runs the whole height, attached
                if framed {
                    built.stretch(h);
                }
                let (x0, y0) = if framed { (w - built.w, 0.0) } else { ((w - built.w - 12.0).round(), 10.0) };
                let mut pieces = built.pieces;
                for piece in &mut pieces {
                    piece.x += x0;
                    piece.y += y0;
                }
                (pieces, None, if framed { 0.0 } else if self.theme.on_top { -10.0 } else { 10.0 })
            }
            PanelKind::Osd => {
                let Some(level) = self.osd.as_ref().map(|(level, _)| level.clone()) else { return };
                let mut pieces = cards::osd(&self.theme, &mut self.painter.text, &level);
                for piece in &mut pieces {
                    piece.x += m;
                    piece.y += m;
                }
                (pieces, None, 8.0)
            }
        };
        let panel = &mut self.panels[index];
        if let Some(damage) = render_panel(&mut panel.surf, pieces, &mut self.painter, &self.theme, background, rise) {
            present(&mut panel.surf, damage, &mut self.pool, &self.qh);
        }
    }

    // ---------------------------------------------------------------- launcher

    pub(super) fn toggle_shortcuts(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Shortcuts).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Shortcuts),
            (_, false) => {
                self.close_menu();
                self.sheet = Some(crate::shortcuts::Sheet::new());
                self.open_panel(PanelKind::Shortcuts, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
            }
        }
    }

    /// Typing narrows the sheet; Backspace widens it; Esc closes it.
    fn shortcuts_key(&mut self, event: &KeyEvent) {
        let Some(sheet) = self.sheet.as_mut() else { return };
        match event.keysym {
            Keysym::Escape => {
                self.close_panel(PanelKind::Shortcuts);
                return;
            }
            Keysym::BackSpace => {
                sheet.query.pop();
            }
            _ => match &event.utf8 {
                Some(text) if !self.modifiers.ctrl && !self.modifiers.alt && text.chars().all(|c| !c.is_control()) => sheet.query.push_str(text),
                _ => return,
            },
        }
        if let Some(index) = self.panel_index(PanelKind::Shortcuts) {
            self.panels[index].surf.dirty = true;
        }
    }

    pub(super) fn toggle_launcher(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Launcher).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Launcher),
            (_, false) => {
                self.close_menu();
                self.launcher.open();
                // The whole output, dimmed, above everything; the keyboard all its own
                self.open_panel(PanelKind::Launcher, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
            }
        }
    }

    // ---------------------------------------------------------------- authentication

    /// Polkit's request came, changed or went: the dialog follows it.
    pub(super) fn sync_auth(&mut self) {
        match self.model.auth.as_ref().map(|r| r.cookie.clone()) {
            Some(cookie) => {
                if self.auth_cookie.as_ref() != Some(&cookie) {
                    // A new request starts with an empty field
                    self.password.clear();
                    self.auth_cookie = Some(cookie);
                }
                if self.panel_index(PanelKind::Auth).is_none() {
                    self.close_menu();
                    self.close_panel(PanelKind::Launcher);
                    self.close_panel(PanelKind::Control);
                    // Above everything, the keyboard all its own
                    self.open_panel(PanelKind::Auth, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
                }
                if let Some(index) = self.panel_index(PanelKind::Auth) {
                    self.panels[index].surf.dirty = true;
                }
            }
            None => {
                self.password.clear();
                self.auth_cookie = None;
                self.close_panel(PanelKind::Auth);
            }
        }
    }

    pub(super) fn auth_submit(&mut self) {
        let Some(request) = self.model.auth.as_ref() else { return };
        if request.busy || self.password.is_empty() {
            return;
        }
        let cookie = request.cookie.clone();
        let password = self.password.take();
        self.send(Command::AuthRespond(cookie, password));
    }

    pub(super) fn auth_cancel(&mut self) {
        if let Some(request) = self.model.auth.as_ref() {
            self.send(Command::AuthCancel(request.cookie.clone()));
        }
        self.password.clear();
    }

    fn auth_key(&mut self, event: &KeyEvent) {
        let busy = self.model.auth.as_ref().is_some_and(|r| r.busy);
        let ctrl = self.modifiers.ctrl;
        match event.keysym {
            Keysym::Escape => self.auth_cancel(),
            Keysym::Return | Keysym::KP_Enter => self.auth_submit(),
            _ if busy => return,
            Keysym::BackSpace if ctrl => self.password.clear(),
            Keysym::BackSpace => self.password.pop(),
            Keysym::u if ctrl => self.password.clear(),
            _ => match &event.utf8 {
                Some(text) if !ctrl && !self.modifiers.alt && !text.chars().any(char::is_control) => self.password.push(text),
                _ => return,
            },
        }
        if let Some(index) = self.panel_index(PanelKind::Auth) {
            self.panels[index].surf.dirty = true;
        }
    }

    // ---------------------------------------------------------------- control centre

    pub(super) fn toggle_settings(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Settings).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Settings),
            (_, false) => {
                self.close_menu();
                self.close_panel(PanelKind::Control);
                self.settings = Some(crate::settings::Settings::new());
                // A window like any app's: tiled, moved and closed the compositor's way
                self.open_window(PanelKind::Settings, "Settings", "asterism-settings", (1000, 720), (720, 480));
            }
        }
    }

    /// Writes a setting to the config, then reads the config again so it shows at once.
    pub(super) fn set_setting(&mut self, keys: &[String], value: &crate::settings::Value) {
        match crate::settings::write(&self.config_path, keys, value) {
            Ok(()) => {
                self.reload();
                self.config_stamp = stamp(&self.config_path);
            }
            Err(error) => {
                log::warn!("couldn't save {}: {error}", keys.join("."));
                self.send(Command::Notify(crate::model::Note { summary: "Couldn't save that setting".into(), body: format!("{error}"), ..Default::default() }));
            }
        }
        if let Some(index) = self.panel_index(PanelKind::Settings) {
            // Colours may have changed with it, and they aren't in the pieces' hashes
            self.panels[index].surf.dirty = true;
            self.panels[index].surf.last = None;
        }
    }

    /// Ctrl+W or Ctrl+Q closes; arrows go through the pages, Page Up and Down scroll one;
    /// typing goes to the value being edited (Esc lets it be).
    fn settings_key(&mut self, event: &KeyEvent) {
        if self.modifiers.ctrl && matches!(event.keysym, Keysym::w | Keysym::q) {
            return self.close_panel(PanelKind::Settings);
        }
        match event.keysym {
            Keysym::Page_Up => return self.scroll_settings(-240.0),
            Keysym::Page_Down => return self.scroll_settings(240.0),
            _ => {}
        }
        let Some(settings) = self.settings.as_mut() else { return };
        match (&mut settings.editing, event.keysym) {
            (None, Keysym::Up) => {
                settings.page = settings.page.saturating_sub(1);
                settings.scroll = 0.0;
            }
            (None, Keysym::Down) => {
                settings.page = (settings.page + 1).min(crate::settings::PAGES.len() - 1);
                settings.scroll = 0.0;
            }
            (None, _) => return,
            (Some(_), Keysym::Escape) => settings.editing = None,
            (Some(_), Keysym::Return | Keysym::KP_Enter) => {
                if let Some((keys, typed)) = settings.editing.take() {
                    let value = if typed.trim().is_empty() { crate::settings::Value::Remove } else { crate::settings::Value::Str(typed.trim().to_string()) };
                    return self.set_setting(&keys, &value);
                }
            }
            (Some((_, typed)), Keysym::BackSpace) => {
                typed.pop();
            }
            (Some((_, typed)), _) => match &event.utf8 {
                Some(text) if !self.modifiers.ctrl && text.chars().all(|c| !c.is_control()) => typed.push_str(text),
                _ => return,
            },
        }
        if let Some(index) = self.panel_index(PanelKind::Settings) {
            self.panels[index].surf.dirty = true;
        }
    }

    pub(super) fn toggle_dashboard(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Dashboard).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Dashboard),
            (_, false) => {
                self.close_menu();
                self.close_panel(PanelKind::Control);
                self.dashboard = Some(crate::dashboard::Dashboard::new());
                self.send(Command::WatchProcesses(true));
                self.open_panel(PanelKind::Dashboard, Layer::Top, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, 0);
            }
        }
    }

    pub(super) fn toggle_control(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Control).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Control),
            (_, false) => {
                self.close_menu();
                self.close_panel(PanelKind::Launcher);
                self.control_state = popups::State::default();
                // The whole output below the bar: a click beside the card closes it
                self.open_panel(PanelKind::Control, Layer::Top, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, 0);
            }
        }
    }

    pub(super) fn launch(&mut self, index: usize) {
        let terminal = self.config.launcher.terminal.clone();
        let search = self.config.launcher.search.clone();
        if self.launcher.activate(index, &terminal, &search) {
            self.close_panel(PanelKind::Launcher);
        }
    }

    fn launcher_key(&mut self, event: &KeyEvent) {
        let ctrl = self.modifiers.ctrl;
        match event.keysym {
            Keysym::Escape => {
                self.close_panel(PanelKind::Launcher);
                return;
            }
            Keysym::Return | Keysym::KP_Enter => {
                let selected = self.launcher.selected;
                self.launch(selected);
                return;
            }
            // Round the grid: a column or a row at a time, or a page
            Keysym::Left | Keysym::ISO_Left_Tab => self.launcher.step(-1),
            Keysym::Right => self.launcher.step(1),
            Keysym::Tab => self.launcher.step(if self.modifiers.shift { -1 } else { 1 }),
            Keysym::Up => self.launcher.step(-(self.launcher.columns as i32)),
            Keysym::Down => self.launcher.step(self.launcher.columns as i32),
            Keysym::Page_Up => self.launcher.step(-(self.launcher.per_page as i32)),
            Keysym::Page_Down => self.launcher.step(self.launcher.per_page as i32),
            Keysym::BackSpace => self.launcher.erase(ctrl),
            Keysym::u if ctrl => self.launcher.clear(),
            Keysym::w if ctrl => self.launcher.erase(true),
            _ => match &event.utf8 {
                Some(text) if !ctrl && !self.modifiers.alt => self.launcher.type_text(text),
                _ => return,
            },
        }
        if let Some(index) = self.panel_index(PanelKind::Launcher) {
            self.panels[index].surf.dirty = true;
        }
    }

    /// A key, from whichever of the shell's surfaces has the keyboard.
    pub(super) fn key_event(&mut self, event: &KeyEvent) {
        if self.panel_index(PanelKind::Auth).is_some() {
            self.auth_key(event);
            return;
        }
        if self.panel_index(PanelKind::Pair).is_some() {
            self.pair_key(event);
            return;
        }
        // The settings window only hears keys while it has them, like any window
        let on = self.keyboard_on.clone();
        let on = |surf: &Surf| on.as_ref().is_some_and(|wl| surf.is(wl));
        if self.panel_index(PanelKind::Settings).is_some_and(|i| on(&self.panels[i].surf)) {
            self.settings_key(event);
            return;
        }
        if self.desk_edit && self.desks.iter().any(|d| d.surfs().any(on)) {
            self.desk_key(event);
            return;
        }
        if self.panel_index(PanelKind::Overview).is_some() {
            self.overview_key(event);
            return;
        }
        if self.panel_index(PanelKind::Wifi).is_some() {
            self.wifi_key(event);
            return;
        }
        if self.panel_index(PanelKind::Launcher).is_some() {
            self.launcher_key(event);
            return;
        }
        if self.panel_index(PanelKind::Wallpapers).is_some() {
            self.wallpapers_key(event);
            return;
        }
        if self.panel_index(PanelKind::Shortcuts).is_some() {
            self.shortcuts_key(event);
            return;
        }
        if self.panel_index(PanelKind::Clipboard).is_some() {
            self.clipboard_key(event);
            return;
        }
        if self.panel_index(PanelKind::DeskMenu).is_some() {
            if event.keysym == Keysym::Escape {
                self.close_panel(PanelKind::DeskMenu);
            }
            return;
        }
        if self.panel_index(PanelKind::Control).is_some() {
            if event.keysym == Keysym::Escape {
                self.close_panel(PanelKind::Control);
            }
            return;
        }
        if self.panel_index(PanelKind::Dashboard).is_some() {
            if event.keysym == Keysym::Escape {
                self.close_panel(PanelKind::Dashboard);
            }
            return;
        }
        if self.popup.is_none() {
            return;
        }
        let code = match event.keysym {
            Keysym::Escape => key::ESC,
            Keysym::BackSpace => key::BACKSPACE,
            Keysym::Tab => key::TAB,
            Keysym::ISO_Left_Tab => {
                self.shift = true;
                key::TAB
            }
            Keysym::Return => key::ENTER,
            Keysym::KP_Enter => key::KP_ENTER,
            Keysym::space => key::SPACE,
            Keysym::Home => key::HOME,
            Keysym::End => key::END,
            Keysym::Up => key::UP,
            Keysym::Down => key::DOWN,
            Keysym::Left => key::LEFT,
            Keysym::Right => key::RIGHT,
            Keysym::Page_Up => key::PAGE_UP,
            Keysym::Page_Down => key::PAGE_DOWN,
            _ => return,
        };
        self.key(code);
    }

    // ---------------------------------------------------------------- notifications

    /// A notification arrived (or was replaced): it pops up, unless it's quiet time.
    pub(super) fn toast(&mut self, id: u32) {
        let Some(n) = self.model.notifications.list.iter().find(|n| n.id == id) else { return };
        if let Some(shown) = self.pet.as_mut() {
            shown.pet.notice();
        }
        if self.model.notifications.quiet && n.urgency < 2 {
            return;
        }
        let timeout = match n.timeout {
            0 => None,
            t if t > 0 => Some(Duration::from_millis(t as u64)),
            _ if n.urgency >= 2 => None,
            _ => Some(Duration::from_secs(self.config.notifications.timeout.max(1) as u64)),
        };
        self.toasts.retain(|t| t.id != id);
        self.toasts.insert(0, Toast { id, until: timeout.map(|d| Instant::now() + d) });
        // Too many: the oldest leave
        while self.toasts.len() > self.config.notifications.max_popups.max(1) {
            if let Some(old) = self.toasts.pop() {
                self.send(Command::NotificationExpired(old.id));
            }
        }
        self.sync_toasts();
    }

    pub(super) fn hide_toast(&mut self, id: u32) {
        let before = self.toasts.len();
        self.toasts.retain(|t| t.id != id);
        if self.toasts.len() != before {
            self.sync_toasts();
        }
    }

    /// Once a second: pop-ups whose time is up go, unless the pointer is on them.
    pub(super) fn expire_toasts(&mut self) {
        if self.toasts_hovered {
            // Their time starts again from the moment the pointer leaves
            let fresh = Instant::now() + Duration::from_secs(2);
            for toast in &mut self.toasts {
                if let Some(until) = &mut toast.until {
                    *until = (*until).max(fresh);
                }
            }
            return;
        }
        let now = Instant::now();
        let gone: Vec<u32> = self.toasts.iter().filter(|t| t.until.is_some_and(|u| u <= now)).map(|t| t.id).collect();
        if gone.is_empty() {
            return;
        }
        self.toasts.retain(|t| !gone.contains(&t.id));
        for id in gone {
            self.send(Command::NotificationExpired(id));
        }
        self.sync_toasts();
    }

    /// Opens, resizes or closes the pop-up column to fit what shows.
    pub(super) fn sync_toasts(&mut self) {
        // Ones the app or the centre closed don't pop up any more
        let list = &self.model.notifications.list;
        self.toasts.retain(|t| list.iter().any(|n| n.id == t.id));
        if self.toasts.is_empty() {
            self.close_panel(PanelKind::Toasts);
            return;
        }
        let shown: Vec<&Notification> = self.toasts.iter().filter_map(|t| list.iter().find(|n| n.id == t.id)).collect();
        let (w, h, _) = cards::toasts(&self.theme, &mut self.painter.text, &shown);
        let m = POPUP_MARGIN;
        let size = ((w + m * 2.0).ceil() as u32, (h + m * 2.0).ceil() as u32);
        let index = match self.panel_index(PanelKind::Toasts) {
            Some(index) => index,
            None => {
                let anchor = if self.theme.on_top { Anchor::TOP | Anchor::RIGHT } else { Anchor::BOTTOM | Anchor::RIGHT };
                self.open_panel(PanelKind::Toasts, Layer::Top, anchor, size, (0, 0, 0, 0), KeyboardInteractivity::None, 0);
                match self.panel_index(PanelKind::Toasts) {
                    Some(index) => index,
                    None => return,
                }
            }
        };
        let panel = &mut self.panels[index];
        if panel.want != size {
            panel.want = size;
            if let Role::Layer(layer) = &panel.role {
                layer.set_size(size.0, size.1);
                layer.commit();
            }
        }
        panel.surf.dirty = true;
        self.panel_input(index, &[(m, m, w, h)]);
    }

    // ---------------------------------------------------------------- the level

    pub(super) fn show_osd(&mut self, level: Level) {
        self.osd_generation += 1;
        let generation = self.osd_generation;
        self.osd = Some((level, generation));
        let m = POPUP_MARGIN;
        let index = match self.panel_index(PanelKind::Osd) {
            Some(index) => index,
            None => {
                let size = ((cards::OSD_WIDTH + m * 2.0) as u32, (cards::OSD_HEIGHT + m * 2.0) as u32);
                let anchor = if self.theme.on_top { Anchor::BOTTOM } else { Anchor::TOP };
                self.open_panel(PanelKind::Osd, Layer::Overlay, anchor, size, (64, 0, 64, 0), KeyboardInteractivity::None, 0);
                match self.panel_index(PanelKind::Osd) {
                    Some(index) => index,
                    None => return,
                }
            }
        };
        // Clicks go straight through it
        self.panel_input(index, &[]);
        self.panels[index].surf.dirty = true;
        let _ = self.handle.insert_source(Timer::from_duration(OSD_FOR), move |_, _, app| {
            if app.osd.as_ref().is_some_and(|(_, g)| *g == generation) {
                app.osd = None;
                app.close_panel(PanelKind::Osd);
            }
            TimeoutAction::Drop
        });
    }

    /// A new song: its card for a few seconds, under the bar in the middle; clicks go through.
    pub(super) fn show_song(&mut self) {
        self.song_generation += 1;
        let generation = self.song_generation;
        let m = POPUP_MARGIN;
        let size = ((cards::SONG_WIDTH + m * 2.0) as u32, (cards::SONG_HEIGHT + m * 2.0) as u32);
        let index = match self.panel_index(PanelKind::Song) {
            Some(index) => index,
            None => {
                let anchor = if self.theme.on_top { Anchor::TOP } else { Anchor::BOTTOM };
                // Exclusive zone 0: the compositor keeps it clear of the bar
                self.open_panel(PanelKind::Song, Layer::Overlay, anchor, size, (0, 0, 0, 0), KeyboardInteractivity::None, 0);
                match self.panel_index(PanelKind::Song) {
                    Some(index) => index,
                    None => return,
                }
            }
        };
        self.panel_input(index, &[]);
        self.panels[index].surf.dirty = true;
        self.watch_spectrum();
        let _ = self.handle.insert_source(Timer::from_duration(SONG_FOR), move |_, _, app| {
            if app.song_generation == generation {
                app.close_panel(PanelKind::Song);
            }
            TimeoutAction::Drop
        });
    }

    /// The song playing changed: its card shows, unless the music's own menu is open, the
    /// shell just started, or it's turned off.
    pub(super) fn notice_song(&mut self) {
        let track = self.model.media.player().filter(|p| p.playing && !p.title.is_empty()).map(|p| format!("{}\u{1f}{}\u{1f}{}", p.bus, p.artist, p.title));
        let Some(track) = track else { return };
        if self.last_track.as_ref() == Some(&track) {
            return;
        }
        let fresh = self.last_track.is_some() || self.started.elapsed() > Duration::from_secs(5);
        self.last_track = Some(track);
        let media_open = self.popup.as_ref().is_some_and(|p| p.kind == Kind::Media);
        if fresh && self.config.song_popup && !media_open && self.panel_index(PanelKind::Control).is_none() {
            self.show_song();
        }
    }

    /// The model is about to change: changes made outside the shell show the level.
    pub(super) fn notice(&mut self, update: &Update) {
        let menu = self.popup.as_ref().map(|p| p.kind.clone());
        match update {
            Update::Audio(audio) => {
                let old = &self.model.audio;
                if old.available && audio.available && (old.volume != audio.volume || old.muted != audio.muted) && menu != Some(Kind::Sound) {
                    self.show_osd(Level::Volume { value: audio.volume, muted: audio.muted });
                }
            }
            Update::Battery(battery) => {
                // Running low: a word as the level crosses each line on the way down
                let old = &self.model.battery;
                let settings = &self.config.notifications;
                if battery.present && old.present && battery.on_battery && !battery.charging {
                    let (was, now) = (old.level * 100.0, battery.level * 100.0);
                    let crossed = |line: u32| line > 0 && was > line as f32 && now <= line as f32;
                    let left = crate::popups::battery_line(battery);
                    let note = if crossed(settings.battery_critical) {
                        Some(crate::model::Note { summary: "Battery critically low".into(), body: format!("{left}. Plug in now, or save your work."), icon: "battery-caution".into(), urgency: 2 })
                    } else if crossed(settings.battery_low) {
                        Some(crate::model::Note { summary: "Battery low".into(), body: format!("{left}. Plug in soon."), icon: "battery-low".into(), urgency: 1 })
                    } else {
                        None
                    };
                    if let Some(note) = note {
                        self.send(Command::Notify(note));
                    }
                }
            }
            Update::Brightness(brightness) => {
                let old = &self.model.brightness;
                if old.available && brightness.available && (old.value - brightness.value).abs() > 0.004 {
                    self.show_osd(Level::Brightness(brightness.value));
                }
            }
            Update::Compositor(state) => {
                let old = &self.model.compositor;
                if old.layouts.len() > 1 && state.layouts == old.layouts && state.layout != old.layout {
                    let name = state.layout_names.get(state.layout).cloned().unwrap_or_default();
                    self.show_osd(Level::Layout(name));
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- requests

    pub(super) fn request(&mut self, request: Request) {
        match request {
            Request::Launcher(open) => self.toggle_launcher(open),
            Request::Wallpaper(request) => self.wallpaper_request(request),
            Request::Timer(action) => self.timer_action(action),
            Request::NightLight(on) => self.toggle_night_light(on),
            Request::Shortcuts(open) => self.toggle_shortcuts(open),
            Request::Clipboard(open) => self.toggle_clipboard(open),
            Request::Screenshot(kind) => self.send(Command::Screenshot(kind)),
            Request::Lock => self.start_lock(),
            Request::Dashboard(open) => self.toggle_dashboard(open),
            Request::Settings(open) => self.toggle_settings(open),
            Request::Overview(open) => self.toggle_overview(open),
            Request::Widgets(on) => self.edit_widgets(on),
            Request::Pet(what) => match what.as_str() {
                "feed" => self.pet_feed(),
                "play" => self.pet_play(),
                "next" => self.pet_action(true),
                _ => self.pet_action(false),
            },
            Request::Record { audio } => self.send(if self.model.recording.is_some() { Command::RecordStop } else { Command::RecordStart { audio } }),
            Request::Control(open) => self.toggle_control(open),
            Request::Menu(name) => self.menu_from_keys(&name),
            Request::Notifications => self.menu_from_keys("notifications"),
            Request::Quiet => self.send(Command::NotificationsQuiet),
            Request::ClearNotifications => self.send(Command::NotificationsClear),
            Request::Volume(step) => {
                let audio = &self.model.audio;
                let value = (audio.volume + step).clamp(0.0, 1.5);
                self.show_osd(Level::Volume { value, muted: audio.muted && step <= 0.0 });
                self.send(Command::StepVolume(step));
            }
            Request::Mute => {
                let audio = &self.model.audio;
                self.show_osd(Level::Volume { value: audio.volume, muted: !audio.muted });
                self.send(Command::ToggleMute);
            }
            Request::MicMute => self.send(Command::ToggleMicMute),
            Request::Brightness(step) => {
                if self.model.brightness.available {
                    self.show_osd(Level::Brightness((self.model.brightness.value + step).clamp(0.01, 1.0)));
                }
                self.send(Command::StepBrightness(step));
            }
            Request::Osd(what) => {
                let level = if what == "brightness" {
                    Level::Brightness(self.model.brightness.value)
                } else {
                    Level::Volume { value: self.model.audio.volume, muted: self.model.audio.muted }
                };
                self.show_osd(level);
            }
            Request::Media(what) => {
                if let Some(player) = self.model.media.player() {
                    let bus = player.bus.clone();
                    self.send(match what.as_str() {
                        "next" => Command::MediaNext(bus),
                        "previous" => Command::MediaPrevious(bus),
                        _ => Command::MediaPlayPause(bus),
                    });
                }
            }
            Request::Awake => self.act(Action::Send(Command::ToggleAwake), None, 0),
            Request::Reload => self.reload(),
        }
    }

    /// A menu opened by a keybind: under the piece it belongs to when the bar shows it, with
    /// the keyboard (there's no click to grab with, so Esc or the keybind closes it).
    fn menu_from_keys(&mut self, name: &str) {
        let (kind, piece_key) = match name {
            "calendar" | "clock" => (Kind::Calendar, "clock"),
            "media" | "music" => (Kind::Media, "media"),
            "sound" | "volume" => (Kind::Sound, "status.volume"),
            "network" | "wifi" => (Kind::Network, "status.network"),
            "bluetooth" => (Kind::Bluetooth, "status.bluetooth"),
            "power" | "battery" => (Kind::Power, "status.battery"),
            "session" => (Kind::Session, "status.session"),
            "machine" | "cpu" => (Kind::Machine, "res.cpu"),
            "traffic" => (Kind::Traffic, "res.network"),
            "keyboard" => (Kind::Keyboard, "status.keyboard"),
            "notifications" => {
                if !self.model.notifications.own {
                    self.send(Command::NotificationsToggle);
                    return;
                }
                (Kind::Notifications, "status.notifications")
            }
            other => {
                log::warn!("no menu called {other}");
                return;
            }
        };
        let focused = &self.model.compositor.focused_output;
        let Some(bar) = self.bars.iter().find(|b| &b.output_name == focused).or(self.bars.first()) else { return };
        let anchor = bar.surf.pieces.iter().find(|p| p.key == piece_key).map(|p| (p.x, p.w)).unwrap_or((bar.surf.w / 2.0 - 1.0, 2.0));
        let id = bar.surf.id;
        self.open_menu(kind, id, anchor, None);
    }
}

/// A panel's pieces, painted with only what changed since last time painted again; the same as
/// the bar, over an optional background.
pub(super) fn render_panel(surf: &mut Surf, pieces: Vec<Piece>, painter: &mut Painter, t: &Theme, background: Option<Rgba>, rise: f32) -> Option<Damage> {
    let hovered = surf.hovered.clone();
    let pressed = surf.pressed.clone();
    let appear = surf.anim.value_from("panel.appear", 0.0, 1.0, t.normal, Ease::OutCubic);
    let shift = (1.0 - appear) * rise;
    let looks: Vec<(f32, f32)> = pieces
        .iter()
        .map(|p| {
            let on = hovered.as_deref() == Some(p.key.as_str());
            let lift = if p.lift {
                let target = 1.0;
                surf.anim.value(&format!("{}.lift", p.key), target, t.fast, Ease::Expressive)
            } else {
                1.0
            };
            let pressing = pressed.as_deref() == Some(p.key.as_str());
            let glow = if p.highlight || p.lift { surf.anim.value(&format!("{}.hl", p.key), if pressing { 1.6 } else if on { 1.0 } else { 0.0 }, t.fast, Ease::OutCubic) } else { 0.0 };
            (lift, glow)
        })
        .collect();
    surf.anim.end();
    surf.wants_frames = surf.anim.moving();
    let scale = surf.scale();
    let (cw, ch) = ((surf.w * scale).ceil().max(1.0) as u32, (surf.h * scale).ceil().max(1.0) as u32);
    let entries: Vec<Entry> = pieces
        .iter()
        .zip(&looks)
        .map(|(p, (lift, glow))| {
            let mut bounds = bounds(p, *lift, shift, scale);
            if p.highlight {
                let grow = (8.0 * scale).ceil() as i32;
                bounds = (bounds.0 - grow, bounds.1 - grow, bounds.2 + grow * 2, bounds.3 + grow * 2);
            }
            Entry { key: p.key.clone(), hash: p.content_hash(scale) ^ (glow * 64.0) as u64, bounds, look: (lift.to_bits(), (p.opacity * appear).to_bits()) }
        })
        .collect();

    let full = (0, 0, cw as i32, ch as i32);
    let mut damage: Option<Damage> = None;
    let reuse = match &surf.last {
        Some(last) if last.canvas.width() == cw && last.canvas.height() == ch => {
            // Pieces that came, went or changed
            for new in &entries {
                match last.entries.iter().find(|old| old.key == new.key) {
                    Some(old) if old == new => {}
                    Some(old) => {
                        damage = Some(union(damage, old.bounds));
                        damage = Some(union(damage, new.bounds));
                    }
                    None => damage = Some(union(damage, new.bounds)),
                }
            }
            for old in &last.entries {
                if !entries.iter().any(|new| new.key == old.key) {
                    damage = Some(union(damage, old.bounds));
                }
            }
            true
        }
        _ => false,
    };
    painter.forget(surf.id, &pieces);
    let region = match (reuse, damage) {
        (true, None) => {
            surf.pieces = pieces;
            return None;
        }
        (true, Some(d)) if (d.2 as i64 * d.3 as i64) < (cw as i64 * ch as i64) / 2 => {
            let x0 = d.0.clamp(0, cw as i32);
            let y0 = d.1.clamp(0, ch as i32);
            (x0, y0, (d.0 + d.2).clamp(0, cw as i32) - x0, (d.1 + d.3).clamp(0, ch as i32) - y0)
        }
        _ => full,
    };
    if region.2 <= 0 || region.3 <= 0 {
        surf.pieces = pieces;
        return None;
    }
    let mut patch = Pixmap::new(region.2 as u32, region.3 as u32)?;
    if let Some(color) = background {
        patch.fill(color.skia());
    }
    let origin = (region.0 as f32, region.1 as f32);
    for ((piece, (lift, glow)), entry) in pieces.iter().zip(&looks).zip(&entries) {
        if !overlaps(entry.bounds, region) {
            continue;
        }
        if *glow > 0.01 {
            let prim = Prim::Rect { x: piece.x - 6.0, y: piece.y + shift, w: piece.w + 12.0, h: piece.h, radius: 12.0, color: t.state_layer(*glow) };
            painter.draw(&mut patch, t, &[prim], -origin.0 / scale, -origin.1 / scale, scale);
        }
        compose(&mut patch, painter, surf.id, piece, t, scale, *lift, appear, shift, origin);
    }
    let mut canvas = match surf.last.take() {
        Some(last) if region != full => last.canvas,
        _ => Pixmap::new(cw, ch)?,
    };
    let (row, stride) = (region.2 as usize * 4, cw as usize * 4);
    let (data, from) = (canvas.data_mut(), patch.data());
    for line in 0..region.3 as usize {
        let at = (region.1 as usize + line) * stride + region.0 as usize * 4;
        data[at..at + row].copy_from_slice(&from[line * row..(line + 1) * row]);
    }
    surf.last = Some(LastFrame { canvas, entries, stars: Vec::new() });
    surf.pieces = pieces;
    Some(region)
}

/// A panel drawn into a picture with no compositor, for `--snapshot --panel`.
#[allow(clippy::too_many_arguments)]
pub fn preview(what: &str, query: &str, theme: &Theme, painter: &mut Painter, model: &Model, config: &Config, w: f32, h: f32, scale: f32) -> Option<Pixmap> {
    let m = POPUP_MARGIN;
    let (mut surf, pieces, background) = match what {
        "launcher" => {
            let mut launcher = crate::launcher::Launcher::new();
            launcher.open();
            if !query.is_empty() {
                launcher.type_text(query);
            }
            let requested = HashSet::new();
            let mut view = crate::launcher::View { theme, text: &mut painter.text, model, requested: &requested, requests: Vec::new(), w, h };
            let pieces = launcher.build(&mut view);
            (Surf::offscreen(w, h, scale), pieces, Some(theme.surface.alpha(0.9)))
        }
        "overview" => {
            let mut overview = crate::overview::Overview::new(model);
            overview.query = query.to_string();
            let requested = HashSet::new();
            let output = model.compositor.focused_output.clone();
            let mut view = crate::overview::View { model, theme, text: &mut painter.text, requested: &requested, requests: Vec::new(), output: &output, w, h };
            let pieces = overview.build(&mut view);
            (Surf::offscreen(w, h, scale), pieces, Some(theme.surface.alpha(0.93)))
        }
        "pair" => {
            let ask = match query {
                "pin" => crate::model::PairAsk::Pin,
                "show" => crate::model::PairAsk::ShowPasskey(48_213),
                "allow" => crate::model::PairAsk::Service("0000110b-0000-1000-8000-00805f9b34fb".into()),
                _ => crate::model::PairAsk::Confirm(714_062),
            };
            let pairing = crate::model::Pairing { device: "Pixel Buds".into(), ask };
            let pieces = cards::pair_card(theme, &mut painter.text, &pairing, if query == "pin" { "0000" } else { "" }, w, h);
            (Surf::offscreen(w, h, scale), pieces, Some(SHADE))
        }
        "wifi" => {
            let card = cards::Ask {
                key: "wifi",
                icon: "wifi-high",
                title: "Connect to “Home”",
                message: "This network is secured. Type its password to join it.",
                note: "Kept by NetworkManager once it works",
                busy: false,
                error: if query.is_empty() { "" } else { "That password didn't work." },
                ok: "Connect",
                ok_action: crate::ui::Action::WifiSubmit,
                cancel_action: crate::ui::Action::WifiCancel,
            };
            let pieces = cards::password_card(theme, &mut painter.text, &card, 9, w, h);
            (Surf::offscreen(w, h, scale), pieces, Some(SHADE))
        }
        "settings" => {
            let mut settings = crate::settings::Settings::new();
            settings.page = query.parse().unwrap_or(0);
            let pieces = settings.build(config, model, theme, &mut painter.text, w, h);
            (Surf::offscreen(w, h, scale), pieces, Some(theme.surface))
        }
        "dashboard" => {
            let dashboard = crate::dashboard::Dashboard::new();
            let pieces = dashboard.build(model, theme, &mut painter.text, w, 8.0);
            (Surf::offscreen(w, h, scale), pieces, None)
        }
        "clipboard" => {
            let mut history = crate::clipboard::History::default();
            for text in ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK… you@laptop", "https://github.com/cykler01/cykler-caelestia", "Meeting moved to 4 pm, same room.", "cargo build --release && install -Dm755 target/release/asterism ~/.local/bin/"] {
                history.add(crate::clipboard::Content::Text(text.to_string()));
            }
            let mut picker = crate::clipboard::Picker::new();
            picker.query = query.to_string();
            let pieces = picker.build(&history, theme, &mut painter.text, w, h);
            (Surf::offscreen(w, h, scale), pieces, Some(SHADE))
        }
        "shortcuts" => {
            let mut sheet = crate::shortcuts::Sheet::new();
            sheet.query = query.to_string();
            let pieces = sheet.build(theme, &mut painter.text, w, h);
            (Surf::offscreen(w, h, scale), pieces, Some(theme.surface.alpha(0.94)))
        }
        "wallpapers" => {
            let mut picker = crate::wallpapers::Picker::new(&config.wallpaper);
            let current = crate::config::expand_home(config.wallpaper.image.trim());
            let requested = HashSet::new();
            let mut view = crate::wallpapers::View { theme, text: &mut painter.text, model, requested: &requested, requests: Vec::new(), current: &current, w, h };
            let pieces = picker.build(&mut view);
            (Surf::offscreen(w, h, scale), pieces, Some(theme.surface.alpha(0.92)))
        }
        "toasts" => {
            let list: Vec<&Notification> = model.notifications.list.iter().take(3).collect();
            let (cw, ch, mut pieces) = cards::toasts(theme, &mut painter.text, &list);
            for piece in &mut pieces {
                piece.x += m;
                piece.y += m;
            }
            (Surf::offscreen(cw + m * 2.0, ch + m * 2.0, scale), pieces, None)
        }
        "auth" => {
            let request = crate::model::AuthRequest {
                cookie: "preview".into(),
                message: "Authentication is required to install updated software from the system's repositories.".into(),
                action: "org.freedesktop.packagekit.system-update".into(),
                user: std::env::var("USER").unwrap_or_else(|_| "you".into()),
                error: if query.is_empty() { String::new() } else { "That wasn't it. Try again.".into() },
                busy: false,
                waiting: 0,
            };
            let pieces = cards::auth(theme, &mut painter.text, &request, 7, w, h);
            (Surf::offscreen(w, h, scale), pieces, Some(SHADE))
        }
        "control" => {
            let state = popups::State::default();
            let requested = HashSet::new();
            let mut anim = Animator::new();
            anim.begin();
            let mut ctx = popups::Ctx { model, config, theme, text: &mut painter.text, anim: &mut anim, state: &state, now: chrono::Local::now(), requests: Vec::new(), requested: &requested };
            let built = popups::control(&mut ctx, h - 24.0);
            let mut pieces = built.pieces;
            for piece in &mut pieces {
                piece.x += m;
                piece.y += m;
            }
            (Surf::offscreen(built.w + m * 2.0, built.h + m * 2.0, scale), pieces, None)
        }
        "deskmenu" => {
            let state = popups::State::default();
            let requested = HashSet::new();
            let mut anim = Animator::new();
            anim.begin();
            let mut ctx = popups::Ctx { model, config, theme, text: &mut painter.text, anim: &mut anim, state: &state, now: chrono::Local::now(), requests: Vec::new(), requested: &requested };
            let built = popups::desktop(&mut ctx, (!query.is_empty()).then_some(query));
            let mut pieces = built.pieces;
            for piece in &mut pieces {
                piece.x += m;
                piece.y += m;
            }
            (Surf::offscreen(built.w + m * 2.0, built.h + m * 2.0, scale), pieces, None)
        }
        "pet" => {
            // Every pose in a row, on the wallpaper's colours
            // "cat" or "cat:ginger"; drawn in a grid, a few to a row
            let (animal, coat) = query.split_once(':').unwrap_or((query, ""));
            let species = crate::pet::Species::from_name(animal).unwrap_or(crate::pet::Species::Cat);
            let coat = crate::pet::draw::coat(species, coat, theme);
            let poses = crate::pet::Pet::poses(species);
            let (cell_w, cell_h, across) = (130.0, 110.0, 4);
            let mut pieces = Vec::new();
            for (i, pet) in poses.iter().enumerate() {
                let mut piece = pet.draw(theme, &coat, 1.0, 0.0);
                piece.key = format!("pet.{i}");
                piece.x = (i % across) as f32 * cell_w - (crate::pet::W - cell_w) / 2.0;
                piece.y = (i / across) as f32 * cell_h - (crate::pet::H - cell_h) + 4.0;
                pieces.push(piece);
            }
            let rows = poses.len().div_ceil(across);
            (Surf::offscreen(cell_w * across as f32, cell_h * rows as f32, scale), pieces, Some(theme.surface))
        }
        "song" => {
            let player = model.media.player()?.clone();
            let art = model.art.get(&player.art_url).cloned();
            let levels = if model.spectrum.is_empty() { crate::spectrum::sample_levels(crate::spectrum::BARS) } else { model.spectrum.clone() };
            let mut pieces = cards::song(theme, &mut painter.text, &player, art, &levels);
            for piece in &mut pieces {
                piece.x += m;
                piece.y += m;
            }
            (Surf::offscreen(cards::SONG_WIDTH + m * 2.0, cards::SONG_HEIGHT + m * 2.0, scale), pieces, None)
        }
        "osd" => {
            let mut pieces = cards::osd(theme, &mut painter.text, &Level::Volume { value: model.audio.volume.max(0.4), muted: false });
            for piece in &mut pieces {
                piece.x += m;
                piece.y += m;
            }
            (Surf::offscreen(cards::OSD_WIDTH + m * 2.0, cards::OSD_HEIGHT + m * 2.0, scale), pieces, None)
        }
        _ => return None,
    };
    for _ in 0..2 {
        surf.begin_frame();
        render_panel(&mut surf, pieces.clone(), painter, theme, background, 0.0);
        std::thread::sleep(Duration::from_millis(350));
    }
    surf.canvas()
}
