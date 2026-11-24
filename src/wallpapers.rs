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

/// Pictures the shell can draw, then the moving ones (`crate::live`).
const KINDS: [&str; 4] = ["png", "jpg", "jpeg", "gif"];

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
        .filter(|p| p.is_file() && (crate::live::is_video(p) || p.extension().and_then(|e| e.to_str()).is_some_and(|e| KINDS.contains(&e.to_lowercase().as_str()))))
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

// ---------------------------------------------------------------- folders

fn folders_path() -> PathBuf {
    state_path().with_file_name("wallpaper-folders")
}

/// Folders lately shown in the picker, the newest first, those still there.
pub fn recent_folders() -> Vec<PathBuf> {
    read_folders(&folders_path())
}

fn read_folders(file: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    text.lines().map(PathBuf::from).filter(|p| p.is_dir()).collect()
}

/// Puts a folder first among the recent ones (six are kept).
pub fn remember_folder(dir: &Path) {
    remember_folder_at(&folders_path(), dir);
}

fn remember_folder_at(file: &Path, dir: &Path) {
    let mut list = read_folders(file);
    list.retain(|p| p != dir);
    list.insert(0, dir.to_path_buf());
    list.truncate(6);
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    if let Err(error) = std::fs::write(file, text) {
        log::warn!("couldn't remember the wallpaper folder: {error}");
    }
}

/// The folders inside one, by name; hidden ones left out.
pub fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    found.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    found
}

/// A path as people write it: under the home folder, from "~".
pub fn short(path: &Path) -> String {
    let full = path.display().to_string();
    let home = home().display().to_string();
    match full.strip_prefix(&home) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => full,
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
    let big = crate::images::still(path, 1024)?;
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

/// Which pictures the picker shows: all of them, the still ones, or the moving ones (GIFs and
/// videos, `crate::live`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    Pictures,
    Moving,
}

pub struct Picker {
    pub list: Vec<PathBuf>,
    pub folder: PathBuf,
    /// The picture picked with the keys, by its place in `list`.
    pub selected: usize,
    /// How the grid last fell, for the keys.
    pub columns: usize,
    pub per_page: usize,
    /// Words typed to find a picture by its name.
    pub query: String,
    pub show: Show,
    /// The folder list under the folder's name is open.
    pub folders: bool,
    /// Folders to switch to: lately shown ones, and those beside this one.
    pub recent: Vec<PathBuf>,
    pub inside: Vec<PathBuf>,
}

pub struct View<'a> {
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub model: &'a Model,
    pub requested: &'a HashSet<String>,
    pub requests: Vec<Command>,
    /// The picture on screen now.
    pub current: &'a Path,
    /// What the options along the foot show as chosen.
    pub settings: &'a WallpaperSettings,
    pub w: f32,
    pub h: f32,
}

const SPAN: f32 = 0.8;
const MIN_CELL_W: f32 = 260.0;
const FIELD_H: f32 = 40.0;
const PILL_H: f32 = 32.0;

/// How often the picture can change by itself, in minutes (0 never), as the foot offers it.
const EVERY: [(&str, i64); 5] = [("Off", 0), ("5 min", 5), ("15 min", 15), ("30 min", 30), ("1 hour", 60)];

impl Picker {
    pub fn new(settings: &WallpaperSettings) -> Self {
        let folder = directory(settings);
        let list = list(&folder);
        let current = expand_home(settings.image.trim());
        let selected = list.iter().position(|p| *p == current).unwrap_or(0);
        let recent = recent_folders();
        let inside = subfolders(&folder);
        Self { list, folder, selected, columns: 4, per_page: 12, query: String::new(), show: Show::All, folders: false, recent, inside }
    }

    /// The pictures shown, by their place in `list`: those of the kind chosen whose names hold
    /// every word typed (in any order, any case).
    pub fn shown(&self) -> Vec<usize> {
        let words: Vec<String> = self.query.to_lowercase().split_whitespace().map(str::to_string).collect();
        self.list
            .iter()
            .enumerate()
            .filter(|(_, p)| match self.show {
                Show::All => true,
                Show::Pictures => !crate::live::is_live(p),
                Show::Moving => crate::live::is_live(p),
            })
            .filter(|(_, p)| {
                let name = name(p).to_lowercase();
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// How many there are of each kind: all, still, moving.
    pub fn counts(&self) -> (usize, usize, usize) {
        let moving = self.list.iter().filter(|p| crate::live::is_live(p)).count();
        (self.list.len(), self.list.len() - moving, moving)
    }

    /// One step wraps round the ends; a row or a page stops there. Only through what's shown.
    pub fn step(&mut self, by: i32) {
        let shown = self.shown();
        if shown.is_empty() {
            return;
        }
        let count = shown.len() as i32;
        let at = shown.iter().position(|i| *i == self.selected).unwrap_or(0) as i32;
        let to = if by.abs() == 1 { (at + by).rem_euclid(count) } else { (at + by).clamp(0, count - 1) };
        self.selected = shown[to as usize];
    }

    /// The first or last of what's shown.
    pub fn jump_to_end(&mut self, last: bool) {
        let shown = self.shown();
        if let Some(i) = if last { shown.last() } else { shown.first() } {
            self.selected = *i;
        }
    }

    /// After the words or the kind changed: the picked picture stays if it's still shown, else
    /// the first one that is.
    pub fn refilter(&mut self) {
        let shown = self.shown();
        if !shown.contains(&self.selected)
            && let Some(first) = shown.first()
        {
            self.selected = *first;
        }
    }

    pub fn set_show(&mut self, show: Show) {
        self.show = show;
        self.refilter();
    }

    pub fn type_text(&mut self, text: &str) {
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        self.refilter();
    }

    /// Backspace: a letter, or with Ctrl the last word.
    pub fn erase(&mut self, word: bool) {
        if word {
            let kept = self.query.trim_end().rsplit_once(' ').map_or(String::new(), |(head, _)| format!("{head} "));
            self.query = kept;
        } else {
            self.query.pop();
        }
        self.refilter();
    }

    /// The whole screen: the title with the folder and a shuffle button; a search field and
    /// the kinds to show; the pictures as a grid of thumbnails (the one on screen ringed in the
    /// accent, moving ones marked); a dot per page; and along the foot, how pictures fill the
    /// screen, how often they change, whether moving ones play, and the rest of the settings.
    pub fn build(&mut self, view: &mut View<'_>) -> Vec<Piece> {
        let t = view.theme;
        let mut pieces = Vec::new();
        let span = (view.w * SPAN).round();
        let left = ((view.w - span) / 2.0).round();
        let top = (view.h * 0.06).round().max(28.0);
        let (all, still, moving) = self.counts();

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
        let folder = short(&self.folder);
        // The folder is a button: it opens the list of folders to switch to
        let mut about = if all == 1 { "1 picture".to_string() } else { format!("{all} pictures") };
        if moving > 0 {
            about.push_str(&format!(" · {moving} moving"));
        }
        let (aw, ah) = view.text.measure(&about, small);
        let mut button = pill(view, "wallpapers.folder", Some("image"), &format!("{folder}  ▾"), self.folders, Some(Action::WallpaperFolders(!self.folders)));
        button.label = Some("Show pictures from another folder".into());
        button.x = left;
        button.y = top + th + 6.0;
        let after = left + button.w + 10.0;
        let folder_bottom = button.y + button.h;
        pieces.push(button);
        let mut subtitle = Piece::new("wallpapers.count", aw, ah);
        subtitle.prims.push(Prim::Text { x: 0.0, y: 0.0, text: about, font: small, color: t.color_soft });
        subtitle.x = after;
        subtitle.y = (top + th + 6.0 + (PILL_H - ah) / 2.0).round();
        subtitle.blocks = true;
        pieces.push(subtitle);
        let ah = PILL_H + 6.0;
        if all > 1 {
            let mut shuffle = pill(view, "wallpapers.shuffle", Some("refresh"), "Shuffle", false, Some(Action::RandomWallpaper));
            shuffle.h = 38.0;
            shuffle.x = left + span - shuffle.w;
            shuffle.y = top + (th + ah - 38.0) / 2.0;
            pieces.push(shuffle);
        }

        // The search field, then the kinds: all, still, moving
        let bar_y = (top + th + ah + 20.0).round();
        let fw = 320f32.min(span * 0.4);
        let big = Font::new(t.text + 1.0);
        let (_, qh) = view.text.measure("Ag", big);
        let mut field = Piece::new("wallpapers.query", fw, FIELD_H);
        field.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: FIELD_H, radius: FIELD_H / 2.0, color: t.hairline() });
        field.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: FIELD_H - 2.0, radius: FIELD_H / 2.0 - 1.0, color: t.card_deep });
        field.prims.push(Prim::Icon { x: 14.0, y: (FIELD_H - 18.0) / 2.0, size: 18.0, name: "search".into(), color: t.color_soft });
        let ty = ((FIELD_H - qh) / 2.0).round();
        let shown_query = crate::bar::Ctx::ellipsize_with(view.text, &self.query, big, fw - 60.0);
        if self.query.is_empty() {
            field.prims.push(Prim::Text { x: 42.0, y: ty, text: "Type to find a picture".into(), font: big, color: t.color_soft.alpha(0.55) });
        } else {
            field.prims.push(Prim::Text { x: 42.0, y: ty, text: shown_query.clone(), font: big, color: t.color_text });
        }
        let caret = 42.0 + if self.query.is_empty() { 0.0 } else { view.text.measure(&shown_query, big).0 + 2.0 };
        field.prims.push(Prim::Rect { x: caret, y: (FIELD_H - 20.0) / 2.0, w: 2.0, h: 20.0, radius: 1.0, color: t.primary });
        field.x = left;
        field.y = bar_y;
        field.blocks = true;
        pieces.push(field);
        let mut x = left + fw + 16.0;
        for (show, icon, label, count) in [(Show::All, None, "All", all), (Show::Pictures, Some("image"), "Pictures", still), (Show::Moving, Some("play"), "Moving", moving)] {
            let key = format!("wallpapers.show.{}", label.to_lowercase());
            let mut chip = pill(view, &key, icon, &format!("{label}  {count}"), self.show == show, Some(Action::WallpaperShow(show)));
            chip.x = x;
            chip.y = bar_y + (FIELD_H - PILL_H) / 2.0;
            x += chip.w + 8.0;
            pieces.push(chip);
        }

        // Along the foot: the options, centred, and the page dots over them
        let foot_y = (view.h - 26.0 - PILL_H).round();
        let mut foot = Vec::new();
        let label = |view: &mut View<'_>, key: &str, text: &str| {
            let font = Font::new(t.text - 1.0);
            let (w, h) = view.text.measure(text, font);
            let mut piece = Piece::new(key, w + 10.0, PILL_H);
            piece.prims.push(Prim::Text { x: 0.0, y: ((PILL_H - h) / 2.0).round(), text: text.into(), font, color: t.color_soft });
            piece.blocks = true;
            piece
        };
        let set = |keys: &[&str], value: crate::settings::Value| Some(Action::SetSetting(keys.iter().map(|k| k.to_string()).collect(), value));
        foot.push(label(view, "wallpapers.opt.fit", "Fill"));
        for (word, fit, value) in [("Cover", crate::config::Fit::Cover, "cover"), ("Whole", crate::config::Fit::Contain, "contain"), ("Stretch", crate::config::Fit::Stretch, "stretch")] {
            foot.push(pill(view, &format!("wallpapers.opt.fit.{value}"), None, word, view.settings.fit == fit, set(&["wallpaper", "fit"], crate::settings::Value::Str(value.into()))));
        }
        foot.push(divider(t, "wallpapers.opt.sep.1"));
        foot.push(label(view, "wallpapers.opt.every", "Change by itself"));
        for (word, minutes) in EVERY {
            foot.push(pill(view, &format!("wallpapers.opt.every.{minutes}"), None, word, view.settings.interval as i64 == minutes, set(&["wallpaper", "interval"], crate::settings::Value::Int(minutes))));
        }
        foot.push(divider(t, "wallpapers.opt.sep.2"));
        let live = view.settings.live;
        foot.push(switch(view, "wallpapers.opt.live", "Play moving ones", live, set(&["wallpaper", "live"], crate::settings::Value::Bool(!live))));
        foot.push(divider(t, "wallpapers.opt.sep.3"));
        foot.push(pill(view, "wallpapers.opt.more", Some("sliders"), "More settings", false, Some(Action::OpenSettings(2))));
        let gaps = 6.0;
        let width: f32 = foot.iter().map(|p| p.w).sum::<f32>() + gaps * (foot.len() - 1) as f32;
        let mut x = ((view.w - width) / 2.0).max(8.0).round();
        for mut piece in foot {
            piece.x = x;
            piece.y = foot_y;
            x += piece.w + gaps;
            pieces.push(piece);
        }
        let dots_y = foot_y - 34.0;

        // How the grid falls: places stretch to fill the span, thumbnails keep 16:10
        let grid_top = bar_y + FIELD_H + 20.0;
        let bottom = dots_y - 4.0;
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
        let shown = self.shown();
        let pages = shown.len().div_ceil(per_page).max(1);
        let at = shown.iter().position(|i| *i == self.selected).unwrap_or(0);
        let page = (at / per_page).min(pages - 1);

        // A cell is drawn 6px in from its place each side, so the hover layer (which reaches 6px
        // out) and the chosen one's tint fill the place exactly
        let (pw, ph) = (cell_w - 12.0, cell_h - 8.0);
        for (slot, index) in shown.iter().copied().skip(page * per_page).take(per_page).enumerate() {
            let path = &self.list[index];
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
            if crate::live::is_live(path) {
                // A moving one: a pill in its corner saying what it is
                let word = if crate::live::is_video(path) { "Video" } else { "GIF" };
                let font = Font::new(t.text - 2.0).weight(600);
                let (ww, wh) = view.text.measure(word, font);
                let (bw, bh) = (ww + 32.0, 24.0);
                let (bx, by) = (ix + 8.0, iy + thumb_h - bh - 8.0);
                cell.prims.push(Prim::Rect { x: bx, y: by, w: bw, h: bh, radius: bh / 2.0, color: t.surface.alpha(0.85) });
                cell.prims.push(Prim::Icon { x: bx + 8.0, y: by + (bh - 13.0) / 2.0, size: 13.0, name: "play".into(), color: t.primary });
                cell.prims.push(Prim::Text { x: bx + 24.0, y: by + ((bh - wh) / 2.0).round(), text: word.into(), font, color: t.color_text });
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
        if shown.is_empty() {
            let text = if all == 0 {
                format!("No pictures in {folder}. Choose another folder with the button under the title.")
            } else if !self.query.is_empty() {
                format!("No picture's name has “{}”.", self.query.trim())
            } else if self.show == Show::Moving {
                format!("No moving wallpapers in {folder}: GIFs and videos (mp4, webm, mkv…) put there show up here.")
            } else {
                "Nothing to show.".to_string()
            };
            let (w, h) = view.text.measure(&text, name_font);
            let mut none = Piece::new("wallpapers.none", w, h);
            none.prims.push(Prim::Text { x: 0.0, y: 0.0, text, font: name_font, color: t.color_soft });
            none.x = ((view.w - w) / 2.0).round();
            none.y = grid_top + 60.0;
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
                let jump = (p * per_page) as i32 - at as i32;
                dot.actions.left = (!here).then_some(Action::WallpaperMove(jump));
                dot.pad = 6.0;
                dot.x = x0 + p as f32 * slot;
                dot.y = dots_y;
                pieces.push(dot);
            }
        }
        if self.folders {
            self.folder_list(view, &mut pieces, left, folder_bottom + 8.0);
        }
        pieces
    }

    /// The folders to switch to, on a card under the folder's button: the system's dialog to
    /// choose any, the ones lately shown, then this one's own and the one above it. A click
    /// anywhere else closes it.
    fn folder_list(&self, view: &mut View<'_>, pieces: &mut Vec<Piece>, x: f32, y: f32) {
        let t = view.theme;
        let mut catcher = Piece::new("wallpapers.folders.away", view.w, view.h);
        catcher.actions.left = Some(Action::WallpaperFolders(false));
        pieces.push(catcher);

        let w = 420.0;
        let row_h = 40.0;
        let font = Font::new(t.text);
        let small = Font::new(t.text - 2.0);
        let label_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
        enum Line {
            Row(String, &'static str, String, String, Option<Action>, bool),
            Label(&'static str),
        }
        let mut lines = vec![Line::Row("wallpapers.folders.choose".into(), "add", "Choose a folder…".into(), "With your desktop's file dialog".into(), Some(Action::ChooseWallpaperFolder), false)];
        let row = |key: String, icon: &'static str, dir: &Path, word: Option<&str>, here: bool| {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| short(dir));
            let title = word.map_or(name.clone(), |w| format!("{w} {name}"));
            Line::Row(key, icon, title, short(dir), (!here).then(|| Action::WallpaperFolder(dir.to_path_buf())), here)
        };
        let recent: Vec<&PathBuf> = self.recent.iter().take(5).collect();
        if !recent.is_empty() {
            lines.push(Line::Label("Lately"));
            for (i, dir) in recent.into_iter().enumerate() {
                lines.push(row(format!("wallpapers.folders.recent.{i}"), "image", dir, None, *dir == self.folder));
            }
        }
        let parent = self.folder.parent().filter(|p| p.as_os_str().len() > 1);
        if parent.is_some() || !self.inside.is_empty() {
            lines.push(Line::Label("Around this one"));
        }
        if let Some(parent) = parent {
            lines.push(row("wallpapers.folders.up".into(), "up", parent, Some("Up to"), false));
        }
        for (i, dir) in self.inside.iter().take(8).enumerate() {
            lines.push(row(format!("wallpapers.folders.in.{i}"), "caret-right", dir, None, false));
        }

        let pad = 10.0;
        let label_h = 26.0;
        let h = pad * 2.0 + lines.iter().map(|l| if matches!(l, Line::Label(_)) { label_h } else { row_h }).sum::<f32>();
        let mut card = Piece::new("wallpapers.folders", w, h);
        card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: 14.0 });
        card.blocks = true;
        card.pad = 24.0;
        card.x = x;
        card.y = y;
        pieces.push(card);
        let mut at = y + pad;
        for line in lines {
            match line {
                Line::Label(text) => {
                    let text = text.to_uppercase();
                    let (_, lh) = view.text.measure(&text, label_font);
                    let mut piece = Piece::new(format!("wallpapers.folders.label.{at}"), w - 32.0, label_h);
                    piece.prims.push(Prim::Text { x: 0.0, y: (label_h - lh - 2.0).round(), text, font: label_font, color: t.primary });
                    piece.x = x + 16.0;
                    piece.y = at;
                    piece.blocks = true;
                    pieces.push(piece);
                    at += label_h;
                }
                Line::Row(key, icon, title, path, action, here) => {
                    let mut piece = Piece::new(key, w - 12.0, row_h);
                    piece.prims.push(Prim::Icon { x: 12.0, y: (row_h - 18.0) / 2.0, size: 18.0, name: icon.into(), color: t.primary });
                    let (_, fh) = view.text.measure("Ag", font);
                    let (_, sh) = view.text.measure("Ag", small);
                    let room = w - 12.0 - 44.0 - if here { 30.0 } else { 12.0 };
                    let title_room = (room * 0.55).max(120.0);
                    let title = crate::bar::Ctx::ellipsize_with(view.text, &title, font, title_room);
                    let (tw, _) = view.text.measure(&title, font);
                    piece.prims.push(Prim::Text { x: 42.0, y: ((row_h - fh) / 2.0).round(), text: title, font, color: t.color_text });
                    let path = crate::bar::Ctx::ellipsize_with(view.text, &path, small, (room - tw - 12.0).max(0.0));
                    if !path.is_empty() {
                        piece.prims.push(Prim::Text { x: 42.0 + tw + 10.0, y: ((row_h - sh) / 2.0).round(), text: path, font: small, color: t.color_soft });
                    }
                    if here {
                        piece.prims.push(Prim::Icon { x: w - 12.0 - 30.0, y: (row_h - 18.0) / 2.0, size: 18.0, name: "check".into(), color: t.primary });
                    }
                    piece.highlight = action.is_some();
                    piece.blocks = true;
                    piece.actions.left = action;
                    piece.x = x + 6.0;
                    piece.y = at;
                    pieces.push(piece);
                    at += row_h;
                }
            }
        }
    }
}

/// A pill to press: outlined when off, filled in the secondary container when on (a choice
/// made), with an optional icon before its words.
fn pill(view: &mut View<'_>, key: &str, icon: Option<&str>, label: &str, on: bool, action: Option<Action>) -> Piece {
    let t = view.theme;
    let font = Font::new(t.text - 1.0).weight(500);
    let (lw, lh) = view.text.measure(label, font);
    let lead = if icon.is_some() { 24.0 } else { 0.0 };
    let (w, h) = ((lw + lead + 28.0).round(), PILL_H);
    let mut piece = Piece::new(key, w, h);
    if on {
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.secondary_container });
    } else {
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.hairline() });
        piece.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0, radius: h / 2.0 - 1.0, color: t.card_deep });
    }
    let ink = if on { t.on_secondary_container } else { t.color_text };
    if let Some(icon) = icon {
        piece.prims.push(Prim::Icon { x: 13.0, y: (h - 16.0) / 2.0, size: 16.0, name: icon.into(), color: if on { ink } else { t.primary } });
    }
    piece.prims.push(Prim::Text { x: 14.0 + lead, y: ((h - lh) / 2.0).round(), text: label.into(), font, color: ink });
    piece.highlight = true;
    piece.actions.left = action;
    piece
}

/// Words and a compact switch after them, the whole of it pressable.
fn switch(view: &mut View<'_>, key: &str, label: &str, on: bool, action: Option<Action>) -> Piece {
    let t = view.theme;
    let font = Font::new(t.text - 1.0);
    let (lw, lh) = view.text.measure(label, font);
    let (sw, sh) = (36.0, 20.0);
    let w = (lw + 12.0 + sw + 8.0).round();
    let mut piece = Piece::new(key, w, PILL_H);
    piece.prims.push(Prim::Text { x: 4.0, y: ((PILL_H - lh) / 2.0).round(), text: label.into(), font, color: t.color_soft });
    let (sx, sy) = (w - sw - 4.0, (PILL_H - sh) / 2.0);
    piece.prims.push(Prim::Rect { x: sx, y: sy, w: sw, h: sh, radius: sh / 2.0, color: if on { t.primary } else { t.outline_variant } });
    let thumb = 14.0;
    let cx = if on { sx + sw - sh / 2.0 } else { sx + sh / 2.0 };
    piece.prims.push(Prim::Rect { x: cx - thumb / 2.0, y: PILL_H / 2.0 - thumb / 2.0, w: thumb, h: thumb, radius: thumb / 2.0, color: if on { t.on_primary } else { t.color_soft } });
    piece.highlight = true;
    piece.actions.left = action;
    piece
}

/// A thin upright hairline between groups of options.
fn divider(t: &Theme, key: &str) -> Piece {
    let mut piece = Piece::new(key, 13.0, PILL_H);
    piece.prims.push(Prim::Rect { x: 6.0, y: 7.0, w: 1.0, h: PILL_H - 14.0, radius: 0.0, color: t.outline_variant });
    piece.blocks = true;
    piece
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker(names: &[&str]) -> Picker {
        let list = names.iter().map(|n| PathBuf::from(format!("/w/{n}"))).collect();
        Picker { list, folder: PathBuf::from("/w"), selected: 0, columns: 4, per_page: 12, query: String::new(), show: Show::All, folders: false, recent: Vec::new(), inside: Vec::new() }
    }

    #[test]
    fn remembers_recent_folders() {
        let dir = scratch("folders");
        let file = dir.join("state/wallpaper-folders");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        remember_folder_at(&file, &a);
        remember_folder_at(&file, &b);
        remember_folder_at(&file, &a);
        assert_eq!(read_folders(&file), [a.clone(), b.clone()], "the newest first, each once");
        std::fs::remove_dir(&b).unwrap();
        assert_eq!(read_folders(&file), [a.clone()], "gone folders drop out");
        for i in 0..8 {
            let d = dir.join(format!("d{i}"));
            std::fs::create_dir_all(&d).unwrap();
            remember_folder_at(&file, &d);
        }
        assert_eq!(read_folders(&file).len(), 6, "six kept");
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        assert!(subfolders(&dir).iter().all(|p| !p.ends_with(".hidden")) && subfolders(&dir).contains(&a));
    }

    #[test]
    fn finds_pictures_by_kind_and_words() {
        let mut p = picker(&["mountain-lake.jpg", "neon_car.png", "street2.gif", "rain.mp4", "Mountain Sunrise.png"]);
        assert_eq!(p.counts(), (5, 3, 2));
        assert_eq!(p.shown(), [0, 1, 2, 3, 4]);
        p.set_show(Show::Moving);
        assert_eq!(p.shown(), [2, 3], "GIFs and videos");
        assert_eq!(p.selected, 2, "the pick moves to what's shown");
        p.set_show(Show::Pictures);
        assert_eq!(p.shown(), [0, 1, 4]);
        p.set_show(Show::All);
        p.type_text("SUNRISE mount");
        assert_eq!(p.shown(), [4], "every word, any order, any case");
        assert_eq!(p.selected, 4);
        p.erase(true);
        assert_eq!(p.query, "SUNRISE ");
        p.erase(false);
        p.erase(false);
        assert_eq!(p.query, "SUNRIS");
        p.query.clear();
        p.type_text("lake");
        assert_eq!(p.shown(), [0]);
    }

    #[test]
    fn steps_only_through_what_is_shown() {
        let mut p = picker(&["a.png", "b.gif", "c.png", "d.webm", "e.png"]);
        p.set_show(Show::Moving);
        assert_eq!(p.selected, 1);
        p.step(1);
        assert_eq!(p.selected, 3);
        p.step(1);
        assert_eq!(p.selected, 1, "a step wraps round");
        p.step(12);
        assert_eq!(p.selected, 3, "a page stops at the end");
        p.set_show(Show::All);
        assert_eq!(p.selected, 3, "still shown: it stays");
        p.jump_to_end(false);
        assert_eq!(p.selected, 0);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("asterism-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_pictures_by_name() {
        let dir = scratch("list");
        for file in ["b.JPG", "a.png", "notes.txt", "c.jpeg", "d.webp", "e.gif", "f.MP4"] {
            std::fs::write(dir.join(file), b"x").unwrap();
        }
        std::fs::create_dir(dir.join("sub.png")).unwrap();
        let names: Vec<String> = list(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        // Moving ones too (`crate::live`)
        assert_eq!(names, ["a.png", "b.JPG", "c.jpeg", "e.gif", "f.MP4"]);
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
