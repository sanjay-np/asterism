//! Changing the wallpaper: the pictures in a folder, the one you chose (kept apart from the
//! config, so choosing never rewrites your file), thumbnails cached on disk, and the picker.

use crate::config::{WallpaperSettings, expand_home, home};
use crate::model::{Command, Image, Model};
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Pictures the shell can draw.
const KINDS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Thumbnails are cut to the screen's usual shape, so a grid of them lines up.
pub const THUMB_W: u32 = 384;
pub const THUMB_H: u32 = 240;

/// Where the pictures are: the configured folder, else the current picture's, else
/// ~/Pictures/Wallpapers, else ~/Pictures.
pub fn directory(settings: &WallpaperSettings) -> PathBuf {
    if !settings.directory.trim().is_empty() {
        return expand_home(settings.directory.trim());
    }
    let image = settings.image.trim();
    if !image.is_empty()
        && let Some(parent) = expand_home(image).parent()
        && parent.is_dir()
    {
        return parent.to_path_buf();
    }
    let wallpapers = home().join("Pictures/Wallpapers");
    if wallpapers.is_dir() { wallpapers } else { home().join("Pictures") }
}

/// The pictures in a folder (not its subfolders), by name.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| KINDS.contains(&e.to_lowercase().as_str())))
        .collect();
    found.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    found
}

/// A picture's name for people: its file name without the extension.
pub fn name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default()
}

// ---------------------------------------------------------------- the choice

fn state_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/state"));
    state.join("asterism").join("wallpaper")
}

/// Keeps the chosen picture, with the config's own `image` as it was then: if you later edit
/// that line, your edit wins over the old choice.
pub fn remember(chosen: &Path, configured: &str) {
    remember_at(&state_path(), chosen, configured);
}

fn remember_at(file: &Path, chosen: &Path, configured: &str) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(error) = std::fs::write(file, format!("{}\n{}\n", chosen.display(), configured)) {
        log::warn!("couldn't remember the wallpaper: {error}");
    }
}

/// The chosen picture replaces the config's, while it exists and the config's line hasn't
/// changed since. `settings.configured` must already hold the config's own value.
pub fn apply_chosen(settings: &mut WallpaperSettings) {
    apply_chosen_from(&state_path(), settings);
}

fn apply_chosen_from(file: &Path, settings: &mut WallpaperSettings) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    let mut lines = text.lines();
    let (Some(chosen), configured) = (lines.next(), lines.next().unwrap_or("")) else { return };
    if configured == settings.configured && Path::new(chosen).is_file() {
        settings.image = chosen.to_string();
    }
}

/// Another picture than `current`: the next in the list, or one picked at random.
pub fn another(list: &[PathBuf], current: &Path, step: i32) -> Option<PathBuf> {
    if list.is_empty() {
        return None;
    }
    let at = list.iter().position(|p| p == current);
    let count = list.len() as i64;
    let index = match (step, at) {
        (0, _) => {
            // At random, never the same one twice running
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut hasher);
            let mut pick = (hasher.finish() % count as u64) as i64;
            if Some(pick as usize) == at && count > 1 {
                pick = (pick + 1) % count;
            }
            pick
        }
        (_, Some(at)) => (at as i64 + step as i64).rem_euclid(count),
        (_, None) => if step > 0 { 0 } else { count - 1 },
    };
    list.get(index as usize).cloned()
}

// ---------------------------------------------------------------- thumbnails

/// A small copy of the picture, cut to 16:10 from its middle. Made once and kept in
/// ~/.cache/asterism/thumbnails, so the picker opens fast the next time.
pub fn thumbnail(path: &Path) -> Option<Arc<Image>> {
    let meta = std::fs::metadata(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, meta.len(), meta.modified().ok(), THUMB_W, THUMB_H).hash(&mut hasher);
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache"));
    let file = cache.join("asterism/thumbnails").join(format!("{:016x}.png", hasher.finish()));
    if let Some(image) = crate::images::load(&file, THUMB_W.max(THUMB_H)) {
        return Some(image);
    }
    let big = crate::images::load(path, 1024)?;
    let source = tiny_skia::Pixmap::from_vec(big.data.clone(), tiny_skia::IntSize::from_wh(big.width, big.height)?)?;
    let mut small = tiny_skia::Pixmap::new(THUMB_W, THUMB_H)?;
    let scale = (THUMB_W as f32 / big.width as f32).max(THUMB_H as f32 / big.height as f32);
    let (dx, dy) = ((THUMB_W as f32 - big.width as f32 * scale) / 2.0, (THUMB_H as f32 - big.height as f32 * scale) / 2.0);
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
    small.draw_pixmap(0, 0, source.as_ref(), &paint, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), None);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = small.save_png(&file);
    Some(Image::new(THUMB_W, THUMB_H, small.take()))
}

// ---------------------------------------------------------------- the picker

pub struct Picker {
    pub list: Vec<PathBuf>,
    pub folder: PathBuf,
    pub selected: usize,
    /// How the grid last fell, for the keys.
    pub columns: usize,
    pub per_page: usize,
}

pub struct View<'a> {
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub model: &'a Model,
    pub requested: &'a HashSet<String>,
    pub requests: Vec<Command>,
    /// The picture on screen now.
    pub current: &'a Path,
    pub w: f32,
    pub h: f32,
}

const SPAN: f32 = 0.8;
const MIN_CELL_W: f32 = 260.0;

impl Picker {
    pub fn new(settings: &WallpaperSettings) -> Self {
        let folder = directory(settings);
        let list = list(&folder);
        let current = expand_home(settings.image.trim());
        let selected = list.iter().position(|p| *p == current).unwrap_or(0);
        Self { list, folder, selected, columns: 4, per_page: 12 }
    }

    /// One step wraps round the ends; a row or a page stops there.
    pub fn step(&mut self, by: i32) {
        if self.list.is_empty() {
            return;
        }
        let count = self.list.len() as i32;
        self.selected = if by.abs() == 1 { (self.selected as i32 + by).rem_euclid(count) } else { (self.selected as i32 + by).clamp(0, count - 1) } as usize;
    }

    /// The whole screen: a title with the folder and a shuffle button, the pictures as a grid of
    /// thumbnails (the one on screen ringed in the accent), and a dot per page.
    pub fn build(&mut self, view: &mut View<'_>) -> Vec<Piece> {
        let t = view.theme;
        let mut pieces = Vec::new();
        let span = (view.w * SPAN).round();
        let left = ((view.w - span) / 2.0).round();
        let top = (view.h * 0.07).round().max(28.0);

        // The title, where the pictures come from, and shuffle
        let title_font = Font::new(t.text + 9.0).weight(500);
        let small = Font::new(t.text - 1.0);
        let (tw, th) = view.text.measure("Wallpapers", title_font);
        let mut title = Piece::new("wallpapers.title", tw, th);
        title.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Wallpapers".into(), font: title_font, color: t.color_text });
        title.x = left;
        title.y = top;
        title.blocks = true;
        pieces.push(title);
        let folder = self.folder.display().to_string().replacen(&home().display().to_string(), "~", 1);
        let count = match self.list.len() {
            1 => "1 picture".to_string(),
            n => format!("{n} pictures"),
        };
        let about = format!("{folder} · {count}");
        let (aw, ah) = view.text.measure(&about, small);
        let mut subtitle = Piece::new("wallpapers.folder", aw, ah);
        subtitle.prims.push(Prim::Text { x: 0.0, y: 0.0, text: about, font: small, color: t.color_soft });
        subtitle.x = left;
        subtitle.y = top + th;
        subtitle.blocks = true;
        pieces.push(subtitle);
        if self.list.len() > 1 {
            let label = "Shuffle";
            let font = Font::new(t.text).weight(500);
            let (lw, lh) = view.text.measure(label, font);
            let (bw, bh) = (lw + 52.0, 38.0);
            let mut shuffle = Piece::new("wallpapers.shuffle", bw, bh);
            shuffle.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.hairline() });
            shuffle.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: bw - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
            shuffle.prims.push(Prim::Icon { x: 16.0, y: (bh - 18.0) / 2.0, size: 18.0, name: "refresh".into(), color: t.primary });
            shuffle.prims.push(Prim::Text { x: 40.0, y: (bh - lh) / 2.0, text: label.into(), font, color: t.color_text });
            shuffle.highlight = true;
            shuffle.actions.left = Some(Action::RandomWallpaper);
            shuffle.x = left + span - bw;
            shuffle.y = top + (th + ah - bh) / 2.0;
            pieces.push(shuffle);
        }

        // How the grid falls: places stretch to fill the span, thumbnails keep 16:10
        let grid_top = top + th + ah + 32.0;
        let bottom = view.h - 72.0;
        let columns = ((span / MIN_CELL_W).floor() as usize).max(1);
        let cell_w = span / columns as f32;
        let thumb_w = cell_w - 28.0;
        let thumb_h = (thumb_w * THUMB_H as f32 / THUMB_W as f32).round();
        let name_font = Font::new(t.text - 1.0);
        let (_, nh) = view.text.measure("Ag", name_font);
        let cell_h = 14.0 + thumb_h + 8.0 + nh + 12.0;
        let rows = (((bottom - grid_top) / cell_h).floor() as usize).max(1);
        self.columns = columns;
        self.per_page = columns * rows;
        let per_page = self.per_page;
        let pages = self.list.len().div_ceil(per_page).max(1);
        let page = (self.selected / per_page).min(pages - 1);

        // A cell is drawn 6px in from its place each side, so the hover layer (which reaches 6px
        // out) and the chosen one's tint fill the place exactly
        let (pw, ph) = (cell_w - 12.0, cell_h - 8.0);
        for (slot, (index, path)) in self.list.iter().enumerate().skip(page * per_page).take(per_page).enumerate() {
            let mut cell = Piece::new(format!("wallpapers.cell.{slot}"), pw, ph);
            if index == self.selected {
                cell.prims.push(Prim::Rect { x: -6.0, y: 0.0, w: pw + 12.0, h: ph, radius: 12.0, color: t.secondary_container.alpha(0.75) });
            }
            let (ix, iy) = ((pw - thumb_w) / 2.0, 14.0);
            let on_screen = path == view.current;
            if on_screen {
                // The one on screen: ringed in the accent
                cell.prims.push(Prim::Rect { x: ix - 3.0, y: iy - 3.0, w: thumb_w + 6.0, h: thumb_h + 6.0, radius: 15.0, color: t.primary });
            }
            match view.model.thumbnails.get(path).cloned() {
                Some(Some(image)) => cell.prims.push(Prim::Image { x: ix, y: iy, w: thumb_w, h: thumb_h, image, tint: None, radius: 12.0 }),
                found => {
                    let key = format!("thumb:{}", path.display());
                    if found.is_none() && !view.requested.contains(&key) {
                        view.requests.push(Command::Thumbnail(path.clone()));
                    }
                    cell.prims.push(Prim::Rect { x: ix, y: iy, w: thumb_w, h: thumb_h, radius: 12.0, color: t.container_high });
                    cell.prims.push(Prim::Icon { x: ix + (thumb_w - 28.0) / 2.0, y: iy + (thumb_h - 28.0) / 2.0, size: 28.0, name: "image".into(), color: t.outline });
                }
            }
            if on_screen {
                let r = 13.0;
                let (cx, cy) = (ix + thumb_w - r - 8.0, iy + r + 8.0);
                cell.prims.push(Prim::Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0, radius: r, color: t.primary });
                cell.prims.push(Prim::Icon { x: cx - 9.0, y: cy - 9.0, size: 18.0, name: "check".into(), color: t.on_primary });
            }
            let label = crate::bar::Ctx::ellipsize_with(view.text, &name(path), name_font, thumb_w);
            let (lw, _) = view.text.measure(&label, name_font);
            cell.prims.push(Prim::Text { x: ((pw - lw) / 2.0).max(0.0), y: iy + thumb_h + 8.0, text: label, font: name_font, color: if on_screen { t.color_text } else { t.color_soft } });
            cell.pad = 8.0;
            cell.highlight = true;
            cell.actions.left = Some(Action::Wallpaper(index));
            cell.actions.up = Some(Action::WallpaperMove(-(per_page as i32)));
            cell.actions.down = Some(Action::WallpaperMove(per_page as i32));
            cell.x = (left + (slot % columns) as f32 * cell_w + 6.0).round();
            cell.y = grid_top + (slot / columns) as f32 * cell_h + 4.0;
            pieces.push(cell);
        }
        if self.list.is_empty() {
            let text = format!("No pictures in {folder}. Set [wallpaper] directory to another folder.");
            let (w, h) = view.text.measure(&text, name_font);
            let mut none = Piece::new("wallpapers.none", w, h);
            none.prims.push(Prim::Text { x: 0.0, y: 0.0, text, font: name_font, color: t.color_soft });
            none.x = ((view.w - w) / 2.0).round();
            none.y = grid_top + 40.0;
            pieces.push(none);
        }

        // A dot per page, joined, the one shown in the accent and ringed (as in the launcher)
        if pages > 1 {
            let slot = 22.0;
            let x0 = ((view.w - pages as f32 * slot) / 2.0).round();
            for p in 0..pages {
                let mut dot = Piece::new(format!("wallpapers.page.{p}"), slot, slot);
                let (cx, cy) = (slot / 2.0, slot / 2.0);
                let here = p == page;
                let r = if here { 4.5 } else { 3.0 };
                let thread = t.outline.alpha(0.55);
                if p > 0 {
                    dot.prims.push(Prim::Line { points: vec![(-0.5, cy), (cx - r - 3.0, cy)], width: 1.2, color: thread });
                }
                if p + 1 < pages {
                    dot.prims.push(Prim::Line { points: vec![(cx + r + 3.0, cy), (slot + 0.5, cy)], width: 1.2, color: thread });
                }
                dot.prims.push(Prim::Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0, radius: r, color: if here { t.primary } else { t.color_soft } });
                if here {
                    dot.prims.push(Prim::Ring { cx, cy, r: r + 3.2, width: 1.2, value: 1.0, color: t.primary.alpha(0.5) });
                }
                let jump = (p * per_page) as i32 - self.selected as i32;
                dot.actions.left = (!here).then_some(Action::WallpaperMove(jump));
                dot.pad = 6.0;
                dot.x = x0 + p as f32 * slot;
                dot.y = (bottom + 8.0).round();
                pieces.push(dot);
            }
        }

        let hint = "Click or Enter sets it · arrows choose · Esc closes";
        let font = Font::new(t.text - 2.0);
        let (hw, hh) = view.text.measure(hint, font);
        let mut footer = Piece::new("wallpapers.footer", hw, hh);
        footer.prims.push(Prim::Text { x: 0.0, y: 0.0, text: hint.into(), font, color: t.color_soft.alpha(0.6) });
        footer.x = ((view.w - hw) / 2.0).round();
        footer.y = (view.h - 22.0 - hh).round();
        pieces.push(footer);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("asterism-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_pictures_by_name() {
        let dir = scratch("list");
        for file in ["b.JPG", "a.png", "notes.txt", "c.jpeg", "d.webp"] {
            std::fs::write(dir.join(file), b"x").unwrap();
        }
        std::fs::create_dir(dir.join("sub.png")).unwrap();
        let names: Vec<String> = list(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.png", "b.JPG", "c.jpeg"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_choice_holds_until_the_config_changes() {
        let dir = scratch("choice");
        let (chosen, state) = (dir.join("sea.png"), dir.join("state"));
        std::fs::write(&chosen, b"x").unwrap();
        remember_at(&state, &chosen, "~/old.png");
        let mut settings = WallpaperSettings { image: "~/old.png".into(), configured: "~/old.png".into(), ..Default::default() };
        apply_chosen_from(&state, &mut settings);
        assert_eq!(settings.image, chosen.display().to_string());
        // You edited the config since: your edit wins
        let mut edited = WallpaperSettings { image: "~/new.png".into(), configured: "~/new.png".into(), ..Default::default() };
        apply_chosen_from(&state, &mut edited);
        assert_eq!(edited.image, "~/new.png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steps_and_shuffles() {
        let list: Vec<PathBuf> = ["a", "b", "c"].iter().map(PathBuf::from).collect();
        assert_eq!(another(&list, Path::new("c"), 1), Some(PathBuf::from("a")));
        assert_eq!(another(&list, Path::new("a"), -1), Some(PathBuf::from("c")));
        assert_eq!(another(&list, Path::new("elsewhere"), 1), Some(PathBuf::from("a")));
        for _ in 0..20 {
            assert_ne!(another(&list, Path::new("b"), 0), Some(PathBuf::from("b")));
        }
        assert_eq!(another(&[], Path::new("a"), 1), None);
    }
}
