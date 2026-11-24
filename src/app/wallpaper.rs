//! The wallpaper changer, as the app runs it: the picker panel, putting up a new picture (and
//! growing the colours from it, off this thread), and changing on its own every so often.

use super::*;
use crate::model::WallpaperRequest;
use crate::wallpapers::{self, Picker};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

impl App {
    pub(super) fn wallpaper_request(&mut self, request: WallpaperRequest) {
        match request {
            WallpaperRequest::Picker(open) => self.toggle_wallpapers(open),
            WallpaperRequest::Step(step) => self.step_wallpaper(step),
            WallpaperRequest::Set(path) => {
                let path = config::expand_home(path.trim());
                if path.is_file() {
                    self.set_wallpaper(&path);
                } else {
                    log::warn!("no picture at {}", path.display());
                }
            }
        }
    }

    pub(super) fn toggle_wallpapers(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Wallpapers).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Wallpapers),
            (_, false) => {
                self.close_menu();
                self.close_panel(PanelKind::Control);
                self.close_panel(PanelKind::Launcher);
                self.picker = Some(Picker::new(&self.config.wallpaper));
                // Like the launcher: the whole output, above everything, the keyboard its own
                self.open_panel(PanelKind::Wallpapers, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
            }
        }
    }

    /// The picker is gone: its thumbnails go too (they're on disk for next time).
    pub(super) fn forget_picker(&mut self) {
        self.picker = None;
        self.model.thumbnails.clear();
        self.requested.retain(|key| !key.starts_with("thumb:"));
        crate::images::give_back_memory();
    }

    /// Puts up the picker's picture with this index, and closes the picker.
    pub(super) fn choose_wallpaper(&mut self, index: usize) {
        let Some(path) = self.picker.as_ref().and_then(|p| p.list.get(index).cloned()) else { return };
        self.close_panel(PanelKind::Wallpapers);
        self.set_wallpaper(&path);
    }

    /// Another picture from the folder: the next, the previous, or (0) one at random.
    pub(super) fn step_wallpaper(&mut self, step: i32) {
        let folder = wallpapers::directory(&self.config.wallpaper);
        let current = config::expand_home(self.config.wallpaper.image.trim());
        match wallpapers::another(&wallpapers::list(&folder), &current, step) {
            Some(path) => {
                self.close_panel(PanelKind::Wallpapers);
                self.set_wallpaper(&path);
            }
            None => log::info!("no other pictures in {}", folder.display()),
        }
    }

    /// Puts up a picture and remembers it. The wallpaper paints on its own thread; the colours
    /// follow once they've grown from it (also off this thread), unless the config fixes them.
    pub(super) fn set_wallpaper(&mut self, path: &std::path::Path) {
        let image = path.display().to_string();
        log::info!("wallpaper: {image}");
        wallpapers::remember(path, &self.config.wallpaper.configured);
        self.config.wallpaper.image = image.clone();
        self.wallpaper_changed = Instant::now();
        if self.config.colors.accent.as_deref().and_then(crate::theme::Rgba::hex).is_some() {
            self.theme = Theme::new(&self.config);
            self.repaint_everything();
        } else {
            self.send(Command::WallpaperPalette(image));
        }
        self.sync_backdrops();
    }

    /// The colours grown from a picture came back: on with them, if it's still the one up.
    pub(super) fn wallpaper_palette(&mut self, image: String, palette: crate::palette::Palette) {
        if image != self.config.wallpaper.image {
            return;
        }
        self.theme = Theme::with_palette(&self.config, palette);
        self.repaint_everything();
        self.send_app_colors();
        self.sync_backdrops();
    }

    /// Colours live in the theme, not in the pieces: everything is painted again.
    pub(super) fn repaint_everything(&mut self) {
        self.own_icons();
        for bar in &mut self.bars {
            bar.surf.dirty = true;
            bar.surf.last = None;
        }
        for panel in &mut self.panels {
            panel.surf.dirty = true;
            panel.surf.last = None;
        }
        for edge in &mut self.edges {
            edge.surf.dirty = true;
        }
        // The widgets too: their colours aren't in what decides when they're drawn again
        for desk in &mut self.desks {
            desk.repaint();
        }
        if let Some(popup) = self.popup.as_mut() {
            popup.surf.dirty = true;
        }
    }

    /// Shows another folder's pictures from now on (the picker, the rotation): written to
    /// `[wallpaper] directory`, and remembered among the recent ones.
    pub(super) fn set_wallpaper_folder(&mut self, dir: &std::path::Path) {
        if !dir.is_dir() {
            return;
        }
        wallpapers::remember_folder(dir);
        if let Some(picker) = self.picker.as_mut() {
            picker.folders = false;
        }
        self.set_setting(&["wallpaper".to_string(), "directory".to_string()], &crate::settings::Value::Str(wallpapers::short(dir)));
    }

    /// The desktop's folder dialog, in place of the picker (which would cover it); the picker
    /// comes back when the dialog's answered.
    pub(super) fn choose_wallpaper_folder(&mut self) {
        let start = self.picker.as_ref().map(|p| p.folder.clone()).unwrap_or_else(|| wallpapers::directory(&self.config.wallpaper));
        let from_picker = self.panel_index(PanelKind::Wallpapers).is_some();
        self.close_panel(PanelKind::Wallpapers);
        self.folder_asked = Some(from_picker);
        self.send(Command::ChooseFolder(start));
    }

    pub(super) fn wallpaper_folder_chosen(&mut self, dir: Option<std::path::PathBuf>) {
        let Some(from_picker) = self.folder_asked.take() else { return };
        if let Some(dir) = &dir {
            self.set_wallpaper_folder(dir);
        }
        if from_picker {
            self.toggle_wallpapers(Some(true));
        }
    }

    /// Every `interval` minutes, another picture at random.
    pub(super) fn rotate_wallpaper(&mut self) {
        let minutes = self.config.wallpaper.interval;
        if minutes == 0 || self.wallpaper_changed.elapsed() < Duration::from_secs(minutes as u64 * 60) {
            return;
        }
        // Not while you're choosing one yourself
        if self.panel_index(PanelKind::Wallpapers).is_some() {
            return;
        }
        self.wallpaper_changed = Instant::now();
        self.step_wallpaper(0);
    }

    /// Typing finds pictures by name; the arrows, Tab and the page keys move through what's
    /// shown; Enter puts it up; Esc lets go of the words, then closes; Ctrl+R picks at random.
    pub(super) fn wallpapers_key(&mut self, event: &KeyEvent) {
        let ctrl = self.modifiers.ctrl;
        let Some(picker) = self.picker.as_mut() else { return };
        match event.keysym {
            Keysym::Escape if !picker.query.is_empty() => {
                picker.query.clear();
                picker.refilter();
            }
            Keysym::Escape => {
                self.close_panel(PanelKind::Wallpapers);
                return;
            }
            Keysym::Return | Keysym::KP_Enter => {
                let selected = picker.selected;
                if picker.shown().contains(&selected) {
                    self.choose_wallpaper(selected);
                }
                return;
            }
            Keysym::Left | Keysym::ISO_Left_Tab => picker.step(-1),
            Keysym::Right => picker.step(1),
            Keysym::Tab => picker.step(if self.modifiers.shift { -1 } else { 1 }),
            Keysym::Up => picker.step(-(picker.columns as i32)),
            Keysym::Down => picker.step(picker.columns as i32),
            Keysym::Page_Up => picker.step(-(picker.per_page as i32)),
            Keysym::Page_Down => picker.step(picker.per_page as i32),
            Keysym::Home => picker.jump_to_end(false),
            Keysym::End => picker.jump_to_end(true),
            Keysym::r | Keysym::R if ctrl => {
                self.step_wallpaper(0);
                return;
            }
            Keysym::BackSpace => picker.erase(ctrl),
            Keysym::u if ctrl => {
                picker.query.clear();
                picker.refilter();
            }
            _ => match &event.utf8 {
                Some(text) if !ctrl && !self.modifiers.alt => picker.type_text(text),
                _ => return,
            },
        }
        if let Some(index) = self.panel_index(PanelKind::Wallpapers) {
            self.panels[index].surf.dirty = true;
        }
    }
}
