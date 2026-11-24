//! Draws the bar, or one of its menus, into a PNG with live data and no compositor: for
//! screenshots, and to see a change without restarting the bar.

use crate::app::{Surf, render_bar, render_popup};
use crate::config::Config;
use crate::model::Model;
use crate::popups::{self, Kind};
use crate::render::Painter;
use crate::theme::{Theme};
use smithay_client_toolkit::reexports::calloop;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Options {
    pub path: PathBuf,
    pub width: f32,
    pub scale: f32,
    pub menu: Option<String>,
    pub hover: Option<String>,
    /// The keyboard on this piece of a menu.
    pub focus: Option<String>,
    /// As if the screen left only this much height for a menu, so it scrolls.
    pub room: Option<f32>,
    /// How far a scrolling menu is scrolled.
    pub scroll: f32,
    /// A panel instead: launcher, toasts or osd.
    pub panel: Option<String>,
    /// What's typed in the launcher.
    pub query: String,
    pub wait: f32,
    /// Made-up workspaces and windows instead of yours: for screenshots to share.
    pub demo: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { path: PathBuf::from("asterism.png"), width: 1920.0, scale: 1.0, menu: None, hover: None, focus: None, room: None, scroll: 0.0, panel: None, query: String::new(), wait: 3.0, demo: false }
    }
}

fn kind(name: &str) -> Option<Kind> {
    Some(match name {
        "calendar" => Kind::Calendar,
        "media" => Kind::Media,
        "sound" => Kind::Sound,
        "network" => Kind::Network,
        "bluetooth" => Kind::Bluetooth,
        "power" => Kind::Power,
        "session" => Kind::Session,
        "machine" => Kind::Machine,
        "traffic" => Kind::Traffic,
        "keyboard" => Kind::Keyboard,
        "notifications" => Kind::Notifications,
        "timer" => Kind::Timer,
        "weather" => Kind::Weather,
        "capture" => Kind::Capture,
        "privacy" => Kind::Privacy,
        _ => return None,
    })
}

/// How light a picture (the screen's, at `scale`) is under the bar, as the shell measures it.
fn ground_of(theme: &Theme, picture: &tiny_skia::Pixmap, scale: f32) -> crate::theme::Ground {
    let band = (theme.reserve() * scale).ceil() as u32;
    let h = picture.height();
    let rows = if theme.on_top { 0..band } else { h.saturating_sub(band)..h };
    crate::theme::Ground::measure(picture, rows)
}

/// The wallpaper behind the picture, so things look as they would on the desktop.
fn sky(width: u32, height: u32, config: &Config) -> tiny_skia::Pixmap {
    let theme = Theme::new(config);
    crate::world::render(width, height, 1.0, crate::theme::planet(&config.planet), &config.wallpaper, &theme.palette)
        .unwrap_or_else(|| tiny_skia::Pixmap::new(width.max(1), height.max(1)).expect("size"))
}

pub fn run(config_path: &Path, options: Options) -> anyhow::Result<()> {
    let config = Config::load(config_path);
    let theme = Theme::new(&config);
    let tools = crate::app::tools_for(&config, true);

    // Gather what the services say for a moment
    let (sender, receiver) = calloop::channel::channel();
    let commands = crate::services::spawn(sender, crate::services::detect_compositor(), crate::services::Roles::default());
    let mut event_loop: calloop::EventLoop<Model> = calloop::EventLoop::try_new()?;
    event_loop
        .handle()
        .insert_source(receiver, |event, _, model: &mut Model| {
            if let calloop::channel::Event::Msg(update) = event {
                model.apply(update);
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let _ = commands.send(crate::model::Command::WeatherSettings(config.weather.clone()));
    let mut model = Model::default();
    let until = Instant::now() + Duration::from_secs_f32(options.wait);
    let mut asked = false;
    while Instant::now() < until {
        event_loop.dispatch(Duration::from_millis(100), &mut model)?;
        // A menu may want something fetched first (a cover, the wifi networks)
        if !asked && Instant::now() + Duration::from_secs_f32(options.wait / 2.0) > until {
            asked = true;
            if let Some(player) = model.media.player()
                && !player.art_url.is_empty()
            {
                let _ = commands.send(crate::model::Command::FetchArt(player.art_url.clone()));
            }
            let _ = commands.send(crate::model::Command::ScanWifi);
            if let Some(item) = model.tray.first() {
                let _ = commands.send(crate::model::Command::TrayMenu(item.id.clone()));
            }
            if let Some(player) = model.media.player()
                && config.widgets.lyrics
            {
                let _ = commands.send(crate::model::Command::FetchLyrics { artist: player.artist.clone(), title: player.title.clone(), album: player.album.clone(), seconds: (player.length / 1_000_000) as u32 });
            }
            if matches!(options.panel.as_deref(), Some("overview" | "peek")) {
                let apps: HashSet<String> = model.compositor.workspaces.iter().flat_map(|w| &w.windows).map(|w| w.app_id.clone()).chain(["firefox", "kitty", "code", "org.gnome.Nautilus", "spotify"].map(String::from)).collect();
                for app in apps {
                    let _ = commands.send(crate::model::Command::AppIcon(app));
                }
            }
            // The dock: the apps' names, and the icons of what's on it
            if options.panel.as_deref() == Some("deskmenu") {
                let _ = commands.send(crate::model::Command::CountTrash);
            }
            if options.panel.as_deref() == Some("dock") {
                let _ = commands.send(crate::model::Command::LoadApps);
                let _ = commands.send(crate::model::Command::CountTrash);
                let apps = crate::launcher::Apps::load().entries;
                for item in crate::dock::items(&crate::dock::sample_pins(&config.dock.pinned, &options.query), &model.compositor.workspaces, &apps) {
                    let _ = commands.send(crate::model::Command::AppIcon(item.key));
                }
            }
            if options.panel.as_deref() == Some("dashboard") {
                let _ = commands.send(crate::model::Command::WatchProcesses(true));
            }
            // The wallpaper changer wants its thumbnails
            if options.panel.as_deref() == Some("wallpapers") {
                for path in crate::wallpapers::list(&crate::wallpapers::directory(&config.wallpaper)) {
                    let _ = commands.send(crate::model::Command::Thumbnail(path));
                }
            }
            // The launcher's grid wants every app's picture
            if options.panel.as_deref() == Some("launcher") {
                for entry in crate::launcher::Apps::load().entries {
                    let _ = commands.send(crate::model::Command::AppIcon(entry.id));
                }
            }
        }
    }

    // The shell's own windows wear its mark, as the app draws it
    if let Some(icon) = crate::logo::icon(&theme, 128) {
        model.app_icons.insert(crate::logo::SETTINGS_APP.to_string(), Some(icon));
    }
    // The privacy pill and its menu with something in use, unless something really is
    if options.menu.as_deref() == Some("privacy") || options.hover.as_deref() == Some("status.privacy") {
        crate::privacy::sample(&mut model);
    }
    if options.demo {
        let output = model.compositor.focused_output.clone();
        // Side by side in columns, the second of a pair half as wide; the first one focused
        let window = |id: u64, n: usize, app: &str, title: &str| crate::model::Window {
            id,
            app_id: app.into(),
            title: title.into(),
            focused: id == 10,
            rect: Some((n as f32 * 976.0, 0.0, if n % 2 == 1 { 640.0 } else { 960.0 }, 1150.0)),
            floating: false,
        };
        let open: [&[(&str, &str)]; 6] = [
            &[("firefox", "Firefox"), ("kitty", "Terminal")],
            &[("code", "main.rs")],
            &[(crate::logo::SETTINGS_APP, "Settings")],
            &[("org.gnome.Nautilus", "Files"), ("kitty", "Terminal"), ("kitty", "Terminal")],
            &[("spotify", "Spotify")],
            &[],
        ];
        model.compositor.workspaces = open
            .iter()
            .enumerate()
            .map(|(i, apps)| crate::model::Workspace {
                id: 100 + i as i64,
                idx: i as i64 + 1,
                name: String::new(),
                output: output.clone(),
                active: i == 1,
                focused: i == 1,
                windows: apps.iter().enumerate().map(|(n, (app, title))| window((i * 10 + n) as u64, n, app, title)).collect(),
            })
            .collect();
        model.compositor.active_on.insert(output, 101);
        // Nor your to-do list
        model.day = crate::day::sample();
        // Nor what you're running
        let running = [("firefox", 0.062, 1_240_000), ("code", 0.031, 820_000), ("spotify", 0.018, 410_000), ("kitty", 0.004, 160_000), ("pipewire", 0.003, 42_000), ("nautilus", 0.0, 210_000), ("asterism", 0.004, 34_000), ("niri", 0.006, 96_000)];
        model.processes = running.iter().enumerate().map(|(i, (name, cpu, kib))| crate::model::Process { pid: 1200 + i as u32 * 37, name: (*name).into(), cpu: *cpu, memory_kib: *kib, own: !matches!(*name, "niri" | "pipewire" | "asterism") }).collect();
        // Nor your screen's brightness right now
        if model.brightness.available {
            model.brightness.value = 0.6;
        }
        // Nor your networks
        let network = &mut model.network;
        if !network.name.is_empty() {
            network.name = "Home".into();
        }
        if !network.address.is_empty() {
            network.address = "192.168.1.20".into();
        }
        let names = ["Home", "Neighbours", "Café", "Studio-5G", "Guest", "Library"];
        network.access_points.truncate(names.len());
        for (n, ap) in network.access_points.iter_mut().enumerate() {
            ap.ssid = names.get(n).map_or_else(|| format!("Network {n}"), |s| s.to_string());
        }
        for (n, vpn) in network.vpns.iter_mut().enumerate() {
            vpn.name = ["Office", "Travel"].get(n).map_or_else(|| format!("VPN {n}"), |s| s.to_string());
        }
    }
    // The day panel's files; a made-up day when there's nothing in them yet
    if !model.day.loaded {
        model.day = crate::day::load(&crate::day::folder(&config.day.folder));
        if model.day.tasks.is_empty() && model.day.notes.is_empty() {
            model.day = crate::day::sample();
        }
    }
    // Notifications to show, when the bar isn't the daemon here
    if model.notifications.list.is_empty() {
        let n = |id: u32, app: &str, summary: &str, body: &str, minutes: i64, urgency: u8, actions: Vec<(String, String)>| crate::model::Notification {
            id,
            app: app.into(),
            summary: summary.into(),
            body: body.into(),
            image: None,
            actions,
            urgency,
            time: chrono::Local::now() - chrono::Duration::minutes(minutes),
            timeout: -1,
            resident: false,
            transient: false,
        };
        model.notifications = crate::model::Notifications {
            available: true,
            own: true,
            count: 3,
            quiet: false,
            list: vec![
                n(3, "Signal", "Maya", "Are we still on for the hike tomorrow morning? I found a trail that ends at a lake, we could watch the sun come up over it.", 0, 1, vec![("default".into(), "Open".into()), ("reply".into(), "Reply".into())]),
                n(2, "Battery", "Battery low", "12% left, about 25 minutes. Plug in soon.", 4, 2, vec![]),
                n(1, "Firefox", "Download finished", "etea-shell-main.zip", 62, 0, vec![]),
            ],
        };
    }
    let output = model.compositor.focused_output.clone();
    let mut painter = Painter::new(&config.font, &config.icons);
    let marquee = HashMap::new();
    let scale = options.scale;

    // The visualizer can't hear anything here: a made-up row of levels while music plays
    if model.spectrum.is_empty() && model.media.player().is_some_and(|p| p.playing) {
        model.spectrum = crate::spectrum::sample_levels(crate::spectrum::BARS);
    }
    // A frame of the wallpaper changing: from yours to the next one in the folder, by the kind
    // asked ("ink", or "ink@0.3" for a moment other than halfway; else one at random)
    if options.panel.as_deref() == Some("transition") {
        let (w, h) = ((options.width * scale) as u32, (options.width * 0.625 * scale) as u32);
        let planet = crate::theme::planet(&config.planet);
        let old = crate::world::render(w, h, scale, planet, &config.wallpaper, &theme.palette).ok_or_else(|| anyhow::anyhow!("nothing drawn"))?;
        let list = crate::wallpapers::list(&crate::wallpapers::directory(&config.wallpaper));
        let current = crate::config::expand_home(config.wallpaper.image.trim());
        let next = list.iter().position(|p| *p == current).map(|i| list[(i + 1) % list.len()].clone()).or_else(|| list.first().cloned()).ok_or_else(|| anyhow::anyhow!("no other picture in the folder"))?;
        let settings = crate::config::WallpaperSettings { image: next.display().to_string(), ..config.wallpaper.clone() };
        let new = crate::world::render(w, h, scale, planet, &settings, &theme.palette).ok_or_else(|| anyhow::anyhow!("nothing drawn"))?;
        let (name, at) = options.query.split_once('@').map_or((options.query.as_str(), 0.5), |(n, t)| (n, t.parse().unwrap_or(0.5)));
        let mut chance = crate::transition::Chance::from_clock();
        let kind = crate::transition::pick(if name.is_empty() { "random" } else { name }, None, &mut chance).unwrap_or(crate::transition::Kind::Fade);
        let map = crate::transition::map(kind, w, h, scale);
        let mut canvas = old.clone();
        crate::transition::compose(canvas.data_mut(), old.data(), new.data(), &map, kind, at, w, h);
        canvas.save_png(&options.path)?;
        println!("{} ({kind:?} at {at})", options.path.display());
        std::process::exit(0);
    }
    let editing = options.panel.as_deref() == Some("widgets");
    if options.panel.as_deref() == Some("wallpaper") || editing {
        // A screen with the screen's proportions, 16:10
        let (w, h) = ((options.width * scale) as u32, (options.width * 0.625 * scale) as u32);
        let mut picture = crate::world::render(w, h, scale, crate::theme::planet(&config.planet), &config.wallpaper, &theme.palette).ok_or_else(|| anyhow::anyhow!("nothing drawn"))?;
        let ground = ground_of(&theme, &picture, scale);
        // The desktop's widgets, where they'd stand
        let mut desk_canvas = None;
        if !config.widgets.show.is_empty() || editing {
            let (dw, dh) = (options.width, options.width * 0.625);
            let requested = HashSet::new();
            let places = crate::widgets::load_places();
            // Twice: what the first pass asks for (the photo frame's folder and picture) is
            // fetched here, then shown
            let mut pieces = Vec::new();
            for _ in 0..3 {
                let mut view = crate::widgets::View { model: &model, theme: &theme, text: &mut painter.text, now: chrono::Local::now(), lyrics: config.widgets.lyrics, requested: &requested, requests: Vec::new(), settings: &config.widgets, photo_step: 0 };
                pieces = crate::widgets::arrange(&config.widgets.show, &mut view, &places, (dw, dh), editing, &mut HashMap::new());
                let requests = std::mem::take(&mut view.requests);
                for request in requests {
                    match request {
                        crate::model::Command::ListPhotos(dir) => model.photos = Some((dir.clone(), crate::wallpapers::list(&dir))),
                        crate::model::Command::Thumbnail(path) => {
                            let image = crate::wallpapers::thumbnail(&path);
                            model.thumbnails.insert(path, image);
                        }
                        _ => {}
                    }
                }
            }
            let mut background = None;
            if editing {
                let top = if theme.on_top { theme.reserve() + 20.0 } else { 28.0 };
                let active = options.hover.as_deref().and_then(|k| k.strip_prefix("widget.")).map(str::to_string);
                pieces = crate::widgets::editor(pieces, &config.widgets.show, None, active.as_deref(), &theme, &mut painter.text, dw, top);
                background = Some(theme.surface.alpha(0.6));
            }
            let mut desk = Surf::offscreen(dw, dh, scale);
            desk.begin_frame();
            crate::app::render_desk_snapshot(&mut desk, pieces, &mut painter, &theme, background);
            desk_canvas = desk.canvas();
        }
        // The widgets lie under the bar; while arranging, over everything
        if !editing && let Some(canvas) = desk_canvas.take() {
            picture.draw_pixmap(0, 0, canvas.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
        // The bar on it, as it would be
        let mut surf = Surf::offscreen(options.width, theme.surface_height(), scale);
        let marquee = HashMap::new();
        let output = model.compositor.focused_output.clone();
        let mut placed = Default::default();
        render_bar(&mut surf, &mut painter, &theme, &model, &config, &tools, &marquee, &output, options.width, None, &mut placed, Some(&ground));
        std::thread::sleep(Duration::from_millis(500));
        render_bar(&mut surf, &mut painter, &theme, &model, &config, &tools, &marquee, &output, options.width, None, &mut placed, Some(&ground));
        if let Some(bar) = surf.canvas() {
            let y = if theme.on_top { 0 } else { (h as f32 - theme.surface_height() * scale) as i32 };
            picture.draw_pixmap(0, y, bar.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
        if let Some(canvas) = desk_canvas.take() {
            picture.draw_pixmap(0, 0, canvas.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
        // The frame's other edges, as the shell draws them
        if theme.framed() && theme.frame > 0.0 {
            let (fw, fh) = (options.width, options.width * 0.625);
            let (f, r) = (theme.frame, theme.frame_radius);
            let reserve = theme.reserve();
            let (top, bottom) = if theme.on_top { (reserve, fh - f) } else { (f, fh - reserve) };
            let c = theme.surface;
            let prims = vec![
                crate::ui::Prim::Rect { x: 0.0, y: top, w: f, h: bottom - top, radius: 0.0, color: c },
                crate::ui::Prim::Rect { x: fw - f, y: top, w: f, h: bottom - top, radius: 0.0, color: c },
                if theme.on_top { crate::ui::Prim::Rect { x: 0.0, y: bottom, w: fw, h: f, radius: 0.0, color: c } } else { crate::ui::Prim::Rect { x: 0.0, y: 0.0, w: fw, h: f, radius: 0.0, color: c } },
                crate::ui::Prim::Fillet { x: f, y: if theme.on_top { bottom } else { top }, r, dx: 1.0, dy: if theme.on_top { -1.0 } else { 1.0 }, color: c },
                crate::ui::Prim::Fillet { x: fw - f, y: if theme.on_top { bottom } else { top }, r, dx: -1.0, dy: if theme.on_top { -1.0 } else { 1.0 }, color: c },
            ];
            painter.draw(&mut picture, &theme, &prims, 0.0, 0.0, scale);
        }
        // An open menu, or the control centre, where it would be
        if let Some(name) = options.menu.as_deref() {
            let (fw, fh) = (options.width, options.width * 0.625);
            let f = if theme.framed() { theme.frame } else { 0.0 };
            let reserve = theme.reserve();
            let state = popups::State::default();
            let requested = HashSet::new();
            let mut anim = crate::ui::Animator::new();
            anim.begin();
            let mut ctx = popups::Ctx { model: &model, config: &config, theme: &theme, text: &mut painter.text, anim: &mut anim, state: &state, now: chrono::Local::now(), requests: Vec::new(), requested: &requested };
            let room = fh - reserve - f.max(8.0);
            let (built, x, y) = if name == "control" {
                let mut built = popups::control(&mut ctx, if theme.framed() { room } else { room - 24.0 });
                if theme.framed() {
                    built.stretch(room);
                }
                let x = if theme.framed() { fw - f - built.w } else { fw - built.w - 12.0 };
                let y = if theme.framed() { reserve } else { reserve + 10.0 };
                (built, x, y)
            } else if name == "day" {
                // In the middle, by the bar
                let built = popups::day(&mut ctx, if theme.framed() { room } else { room - 24.0 });
                let x = ((fw - built.w) / 2.0).round();
                let y = if theme.framed() { reserve } else { reserve + 10.0 };
                (built, x, y)
            } else {
                let kind = if name == "tray" {
                    model.tray.first().map(|i| Kind::Tray(i.id.clone())).ok_or_else(|| anyhow::anyhow!("no tray item"))?
                } else {
                    kind(name).ok_or_else(|| anyhow::anyhow!("no menu called {name}"))?
                };
                let mut built = popups::build(&kind, &mut ctx);
                built.fit(room - 24.0);
                let gap = if theme.framed() { 0.0 } else { 8.0 };
                let x = fw - f - 120.0 - built.w;
                (built, x, reserve + gap)
            };
            let margin = crate::app::POPUP_MARGIN;
            let mut surf = Surf::offscreen(built.w + margin * 2.0, built.h + margin * 2.0, scale);
            for _ in 0..2 {
                surf.begin_frame();
                render_popup(&mut surf, built.pieces.clone(), &mut painter, &theme, None);
                std::thread::sleep(Duration::from_millis(400));
            }
            let menu = surf.canvas().ok_or_else(|| anyhow::anyhow!("nothing drawn"))?;
            let at = |v: f32| ((v - margin) * scale).round() as i32;
            let y = if theme.on_top { y } else { fh - y - built.h };
            picture.draw_pixmap(at(x), at(y), menu.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
        picture.save_png(&options.path)?;
        println!("{}", options.path.display());
        std::process::exit(0);
    }
    if let Some(panel) = options.panel.as_deref() {
        let h = if matches!(panel, "launcher" | "wallpapers" | "shortcuts" | "clipboard" | "dashboard" | "settings" | "overview") { (options.width * 0.625).round() } else if matches!(panel, "control" | "auth" | "wifi" | "pair") { 700.0 } else { 0.0 };
        let shown = crate::app::preview(panel, &options.query, &theme, &mut painter, &model, &config, options.width, h, scale).ok_or_else(|| anyhow::anyhow!("no panel called {panel}"))?;
        let mut picture = sky(shown.width(), shown.height(), &config);
        picture.draw_pixmap(0, 0, shown.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        picture.save_png(&options.path)?;
        println!("{}", options.path.display());
        std::process::exit(0);
    }
    let picture = match options.menu.as_deref() {
        None => {
            let mut surf = Surf::offscreen(options.width, theme.surface_height(), scale);
            surf.hover_for_snapshot(options.hover.clone());
            // `--query arrange` draws arrange mode; `arrange:ITEM@X` (or `carry:ITEM@X`, without
            // the mode) an item carried with the pointer at X, `…@X!` pulled off the bar
            let arrange = arranging(&options.query);
            let mut placed = crate::bar::Placed::default();
            // The bar alone stands on the screen's picture squeezed to its size: measured as such
            let ground = ground_of(&theme, &sky((options.width * scale).ceil() as u32, (theme.surface_height() * scale).ceil() as u32, &config), scale);
            // Twice: the second time, everything has settled in (and the item's place is known)
            render_bar(&mut surf, &mut painter, &theme, &model, &config, &tools, &marquee, &output, options.width, Some(&crate::bar::Arranging { carry: None, ..arrange.clone() }), &mut placed, Some(&ground));
            std::thread::sleep(Duration::from_millis(500));
            let arrange = crate::bar::Arranging {
                carry: arrange.carry.map(|mut c| {
                    let x0 = placed.spans.iter().find(|s| s.name == c.item).map_or(c.x, |s| s.x0);
                    c.grab = 12.0_f32.min(c.x - x0 + 12.0).max(0.0);
                    c
                }),
                ..arrange
            };
            render_bar(&mut surf, &mut painter, &theme, &model, &config, &tools, &marquee, &output, options.width, Some(&arrange), &mut placed, Some(&ground));
            let bar = surf.canvas().ok_or_else(|| anyhow::anyhow!("nothing drawn"))?;
            let mut picture = sky(bar.width(), bar.height(), &config);
            picture.draw_pixmap(0, 0, bar.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
            picture
        }
        Some(name) => {
            let kind = if name == "tray" {
                model.tray.first().map(|i| Kind::Tray(i.id.clone())).ok_or_else(|| anyhow::anyhow!("no tray item"))?
            } else {
                kind(name).ok_or_else(|| anyhow::anyhow!("no menu called {name}"))?
            };
            let state = popups::State::default();
            let requested = HashSet::new();
            let mut anim = crate::ui::Animator::new();
            anim.begin();
            let mut ctx = popups::Ctx {
                model: &model,
                config: &config,
                theme: &theme,
                text: &mut painter.text,
                anim: &mut anim,
                state: &state,
                now: chrono::Local::now(),
                requests: Vec::new(),
                requested: &requested,
            };
            let mut built = popups::build(&kind, &mut ctx);
            let content = built.fit(options.room.unwrap_or(f32::MAX));
            let scroll = (content > built.h).then(|| crate::app::Scroll { offset: options.scroll.clamp(0.0, content - built.h), content, view: built.h });
            let margin = crate::app::POPUP_MARGIN;
            let mut surf = Surf::offscreen(built.w + margin * 2.0, built.h + margin * 2.0, scale);
            surf.hover_for_snapshot(options.hover.clone());
            surf.focus_for_snapshot(options.focus.clone());
            surf.begin_frame();
            render_popup(&mut surf, built.pieces.clone(), &mut painter, &theme, scroll);
            std::thread::sleep(Duration::from_millis(400));
            surf.begin_frame();
            render_popup(&mut surf, built.pieces, &mut painter, &theme, scroll);
            let menu = surf.canvas().ok_or_else(|| anyhow::anyhow!("nothing drawn"))?;
            let mut picture = sky(menu.width(), menu.height(), &config);
            picture.draw_pixmap(0, 0, menu.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
            picture
        }
    };
    picture.save_png(&options.path)?;
    println!("{}", options.path.display());
    // Leave without waiting for the services
    std::process::exit(0);
}

/// The bar's arrange mode and carried item a snapshot asks for (see `--query`).
fn arranging(query: &str) -> crate::bar::Arranging {
    let (mode, rest) = match query.split_once(':') {
        Some(("arrange", rest)) => (true, Some(rest)),
        Some(("carry", rest)) => (false, Some(rest)),
        _ => (query == "arrange", None),
    };
    let carry = rest.and_then(|rest| rest.split_once('@')).map(|(item, x)| {
        let off = x.ends_with('!');
        crate::bar::Carry { item: item.to_string(), x: x.trim_end_matches('!').parse().unwrap_or(0.0), grab: 12.0, off, moved: true }
    });
    crate::bar::Arranging { mode, carry }
}
