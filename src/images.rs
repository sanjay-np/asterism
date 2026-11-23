//! Pictures from elsewhere: album covers, tray icons, app icons. Decoded to premultiplied
//! RGBA once, on the services thread, so drawing never waits on a decoder.

use crate::model::Image;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// PNG, JPEG or SVG bytes to a picture no bigger than `max` pixels on a side.
pub fn decode(data: &[u8], max: u32) -> Option<Arc<Image>> {
    let pixmap = if data.starts_with(b"\x89PNG") {
        tiny_skia::Pixmap::decode_png(data).ok()?
    } else if data.starts_with(&[0xff, 0xd8]) {
        jpeg(data)?
    } else if data.starts_with(b"GIF8") {
        // Its first frame
        crate::live::gif_still(data)?
    } else {
        svg(data, max)?
    };
    Some(shrink(pixmap, max))
}

pub fn load(path: &Path, max: u32) -> Option<Arc<Image>> {
    decode(&std::fs::read(path).ok()?, max)
}

/// A wallpaper as a still picture: a picture, or a video's or GIF's first frame (a video is
/// never read whole into memory).
pub fn still(path: &Path, max: u32) -> Option<Arc<Image>> {
    if crate::live::is_video(path) {
        return Some(shrink(crate::live::video_still(path, max)?, max));
    }
    load(path, max)
}

fn jpeg(data: &[u8]) -> Option<tiny_skia::Pixmap> {
    use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    // Opaque, so straight and premultiplied are the same
    tiny_skia::Pixmap::from_vec(pixels, tiny_skia::IntSize::from_wh(info.width as u32, info.height as u32)?)
}

pub fn svg(data: &[u8], size: u32) -> Option<tiny_skia::Pixmap> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let natural = tree.size();
    let scale = size as f32 / natural.width().max(natural.height());
    let width = (natural.width() * scale).ceil().max(1.0) as u32;
    let height = (natural.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(pixmap)
}

fn shrink(pixmap: tiny_skia::Pixmap, max: u32) -> Arc<Image> {
    let (w, h) = (pixmap.width(), pixmap.height());
    if w <= max && h <= max {
        return Image::new(w, h, pixmap.take());
    }
    let scale = max as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    let Some(mut small) = tiny_skia::Pixmap::new(nw, nh) else { return Image::new(w, h, pixmap.take()) };
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
    small.draw_pixmap(0, 0, pixmap.as_ref(), &paint, tiny_skia::Transform::from_scale(scale, scale), None);
    Image::new(nw, nh, small.take())
}

/// ARGB32 in network byte order, as tray items send their pixmaps.
pub fn from_argb(width: u32, height: u32, bytes: &[u8]) -> Option<Arc<Image>> {
    if width == 0 || height == 0 || bytes.len() < (width * height * 4) as usize {
        return None;
    }
    let mut data = Vec::with_capacity(bytes.len());
    for px in bytes.as_chunks::<4>().0.iter().take((width * height) as usize) {
        let (a, r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
        data.extend_from_slice(&[(r * a / 255) as u8, (g * a / 255) as u8, (b * a / 255) as u8, a as u8]);
    }
    Some(Image::new(width, height, data))
}

// ---------------------------------------------------------------- icon themes

/// The icon theme the desktop uses, as GTK or KDE settings name it.
fn theme_name() -> String {
    let home = crate::config::home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    for file in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Ok(text) = std::fs::read_to_string(config.join(file)) {
            for line in text.lines() {
                if let Some(value) = line.trim().strip_prefix("gtk-icon-theme-name") {
                    let value = value.trim_start_matches([' ', '=']).trim().trim_matches('"');
                    if !value.is_empty() {
                        return value.to_string();
                    }
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(config.join("kdeglobals")) {
        let mut icons = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                icons = line == "[Icons]";
            } else if icons && let Some(value) = line.strip_prefix("Theme=") {
                return value.trim().to_string();
            }
        }
    }
    "hicolor".into()
}

fn icon_roots() -> Vec<PathBuf> {
    let home = crate::config::home();
    let mut roots = vec![home.join(".icons")];
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    roots.push(data_home.join("icons"));
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for dir in dirs.split(':').filter(|d| !d.is_empty()) {
        roots.push(Path::new(dir).join("icons"));
    }
    roots
}

/// Every icon a theme has, by name, with the size of the folder it was found in (0 for scalable).
type ThemeIndex = HashMap<String, Vec<(u32, PathBuf)>>;

fn index_theme(theme: &str) -> (ThemeIndex, Vec<String>) {
    let mut index: ThemeIndex = HashMap::new();
    let mut inherits = Vec::new();
    for root in icon_roots() {
        let dir = root.join(theme);
        if !dir.is_dir() {
            continue;
        }
        if inherits.is_empty()
            && let Ok(text) = std::fs::read_to_string(dir.join("index.theme"))
        {
            for line in text.lines() {
                if let Some(value) = line.trim().strip_prefix("Inherits=") {
                    inherits = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
            }
        }
        walk(&dir, 0, &mut index);
    }
    (index, inherits)
}

fn walk(dir: &Path, size: u32, index: &mut ThemeIndex) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // "48x48", "48", "48@2x", "scalable", "symbolic": the folder names carry the size
            let here = name.split(['x', '@']).next().and_then(|n| n.parse::<u32>().ok());
            let size = match (here, name.as_str()) {
                (Some(n), _) => n,
                (None, "scalable") => 0,
                _ => size,
            };
            walk(&path, size, index);
        } else if let Some(stem) = name.strip_suffix(".svg").or_else(|| name.strip_suffix(".png")) {
            index.entry(stem.to_string()).or_default().push((size, path));
        }
    }
}

struct Themes {
    indexes: HashMap<String, Arc<(ThemeIndex, Vec<String>)>>,
}

fn themes() -> &'static Mutex<Themes> {
    static THEMES: OnceLock<Mutex<Themes>> = OnceLock::new();
    THEMES.get_or_init(|| Mutex::new(Themes { indexes: HashMap::new() }))
}

fn theme(theme: &str) -> Arc<(ThemeIndex, Vec<String>)> {
    let mut themes = themes().lock().unwrap();
    themes.indexes.entry(theme.to_string()).or_insert_with(|| Arc::new(index_theme(theme))).clone()
}

/// The file for an icon name, closest to `size`: the desktop's theme, what it inherits from,
/// then hicolor, then /usr/share/pixmaps. `extra` is a folder an app ships its own icons in.
pub fn find_icon(name: &str, size: u32, extra: Option<&Path>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return Path::new(name).exists().then(|| PathBuf::from(name));
    }
    if let Some(extra) = extra.filter(|e| e.is_dir()) {
        let mut index = ThemeIndex::new();
        walk(extra, 0, &mut index);
        if let Some(found) = best(index.get(name), size) {
            return Some(found);
        }
    }
    let mut queue = vec![theme_name()];
    let mut seen = Vec::new();
    while let Some(current) = queue.first().cloned() {
        queue.remove(0);
        if seen.contains(&current) {
            continue;
        }
        seen.push(current.clone());
        let theme = theme(&current);
        if let Some(found) = best(theme.0.get(name), size) {
            return Some(found);
        }
        queue.extend(theme.1.iter().cloned());
        if queue.is_empty() && !seen.iter().any(|t| t == "hicolor") {
            queue.push("hicolor".into());
        }
    }
    ["png", "svg", "xpm"]
        .iter()
        .map(|ext| PathBuf::from(format!("/usr/share/pixmaps/{name}.{ext}")))
        .find(|p| p.exists() && !p.to_string_lossy().ends_with(".xpm"))
}

fn best(found: Option<&Vec<(u32, PathBuf)>>, size: u32) -> Option<PathBuf> {
    let found = found?;
    // Scalable wins, then the smallest one at least as big, then the biggest one smaller
    found
        .iter()
        .min_by_key(|(s, path)| {
            let svg = path.extension().is_some_and(|e| e == "svg");
            match *s {
                0 if svg => 0,
                s if s >= size => 1 + (s - size),
                s => 10_000 + (size - s),
            }
        })
        .map(|(_, p)| p.clone())
}

/// Desktop entries: an app id or window class to its name and icon.
pub struct App {
    pub name: String,
    pub icon: String,
}

fn apps() -> &'static HashMap<String, App> {
    static APPS: OnceLock<HashMap<String, App>> = OnceLock::new();
    APPS.get_or_init(|| {
        let mut apps = HashMap::new();
        let home = crate::config::home();
        let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
        let mut dirs = vec![data_home.join("applications")];
        let shared = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        dirs.extend(shared.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
        for dir in dirs.iter().rev() {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".desktop")) else { continue };
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                let (mut name, mut icon, mut class) = (String::new(), String::new(), String::new());
                let mut main = false;
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with('[') {
                        main = line == "[Desktop Entry]";
                        continue;
                    }
                    if !main {
                        continue;
                    }
                    if let Some(v) = line.strip_prefix("Name=") {
                        name = v.to_string();
                    } else if let Some(v) = line.strip_prefix("Icon=") {
                        icon = v.to_string();
                    } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
                        class = v.to_string();
                    }
                }
                if name.is_empty() {
                    continue;
                }
                let lower = id.to_lowercase();
                // "org.gnome.Nautilus" is also found as "nautilus"
                if let Some(last) = lower.rsplit('.').next() {
                    apps.insert(last.to_string(), App { name: name.clone(), icon: icon.clone() });
                }
                if !class.is_empty() {
                    apps.insert(class.to_lowercase(), App { name: name.clone(), icon: icon.clone() });
                }
                apps.insert(lower, App { name, icon });
            }
        }
        apps
    })
}

/// A window's app, from its app id or class, if a desktop entry knows it.
pub fn app(app_id: &str) -> Option<&'static App> {
    let lower = app_id.to_lowercase();
    let apps = apps();
    apps.get(&lower).or_else(|| lower.rsplit('.').next().and_then(|last| apps.get(last)))
}

/// Hands memory freed after decoding a big picture back to the system. glibc keeps what each
/// thread freed in that thread's pool, so a few wallpapers decoded on workers would otherwise
/// stay in the shell's footprint for good. (Part of the C library Rust already links.)
pub fn give_back_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: malloc_trim only returns free pages to the system; it touches nothing in use
        unsafe {
            malloc_trim(0);
        }
    }
}

/// Big buffers (decoded wallpapers run to tens of megabytes) get pages of their own, given back
/// the moment they're freed. glibc otherwise raises that bar after the first big free, and
/// later pictures land in pools it can't hand back. Call once, at start.
pub fn tune_allocator() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        const M_MMAP_THRESHOLD: i32 = -3;
        unsafe extern "C" {
            fn mallopt(param: i32, value: i32) -> i32;
        }
        // SAFETY: only changes where future allocations come from, before any threads start
        unsafe {
            mallopt(M_MMAP_THRESHOLD, 1 << 20);
        }
    }
}
