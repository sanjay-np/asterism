//! What the bar holds and how it looks, read from `~/.config/asterism/config.toml`.
//! Every field has a default, so an empty file (or none at all) gives ETEA's own bar.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BarStyle {
    /// Asterism's own: a slim solid strip along the edge with a hairline under it; menus and
    /// panels float just off it as outlined cards.
    #[default]
    #[serde(alias = "hairline")]
    Line,
    /// A thin frame around the screen with rounded inner corners; the bar is its top edge, and
    /// menus and panels hang from it like drawers.
    Frame,
    /// The bar as one rounded surface floating off the edge.
    #[serde(alias = "band")]
    Floating,
    /// Each end of the bar and its middle on a pill of their own.
    Islands,
    /// No surface at all: the bar straight on the wallpaper.
    #[serde(alias = "sky")]
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BarSize {
    Compact,
    #[default]
    Normal,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    #[default]
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PageStyle {
    /// A dot per workspace, the busy ones joined by a thin line.
    #[default]
    #[serde(alias = "constellation")]
    Dots,
    /// The workspace number, the one you're on in an accent pill.
    Numbers,
    /// The number in kanji (一 二 三…), the one you're on in the accent with a bōten dot over it.
    Kanji,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MediaTitle {
    /// Title and artist glide by when they don't fit.
    #[default]
    Scroll,
    /// Title only, cut short.
    Short,
    /// Just the note.
    Off,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            left: vec!["launcher".into(), "pages".into()],
            center: vec!["resources".into(), "clock".into(), "weather".into(), "media".into()],
            right: vec!["tools".into(), "tray".into(), "status".into()],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Clock {
    /// `true` for 24 hours, `false` for 12 hours with AM/PM.
    pub hours24: bool,
    pub seconds: bool,
    /// A chrono/strftime format; an empty string hides the date.
    pub date: String,
}

impl Default for Clock {
    fn default() -> Self {
        Self { hours24: true, seconds: false, date: "%A, %B %-d".into() }
    }
}

/// A little tool in the bar: an icon that runs a command, or one of the built-in toggles.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Tool {
    /// An icon name from `assets/icons` (like "capture", "picker", "clipboard").
    pub icon: String,
    /// Run through `sh -c`.
    pub command: String,
    pub label: String,
    /// "microphone" or "awake" for the built-in toggles, empty for a plain command.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Commands {
    pub settings: String,
    pub lock: String,
    pub suspend: String,
    /// Empty: the compositor's own way out (niri `quit`, Hyprland `exit`).
    pub logout: String,
    pub reboot: String,
    pub poweroff: String,
    pub network: String,
    pub bluetooth: String,
    pub monitor: String,
    pub notifications: String,
}

impl Default for Commands {
    fn default() -> Self {
        Self {
            settings: String::new(),
            lock: "loginctl lock-session".into(),
            suspend: "systemctl suspend".into(),
            logout: String::new(),
            reboot: "systemctl reboot".into(),
            poweroff: "systemctl poweroff".into(),
            network: String::new(),
            bluetooth: String::new(),
            monitor: String::new(),
            notifications: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    /// Be the notification daemon (when no other one is running).
    pub daemon: bool,
    /// How long a pop-up stays, in seconds, when the app doesn't say. Critical ones stay.
    pub timeout: u32,
    /// How many pop-ups show at once.
    pub max_popups: usize,
    /// Per app (its name, or its desktop file's name): "allow", "silence" (kept in the centre,
    /// no pop-up) or "discard" (dropped). Matched without caring about case.
    pub apps: HashMap<String, AppRule>,
    /// Battery level (percent) for a warning, and for an urgent one; 0 turns it off.
    pub battery_low: u32,
    pub battery_critical: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppRule {
    #[default]
    Allow,
    Silence,
    Discard,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { daemon: true, timeout: 6, max_popups: 4, apps: HashMap::new(), battery_low: 15, battery_critical: 5 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Fills the screen, cutting what overflows.
    #[default]
    Cover,
    /// Shows all of it, the planet's sky around.
    Contain,
    Stretch,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WallpaperSettings {
    /// Draw the wallpaper (turn off to keep another program's).
    pub enabled: bool,
    /// A picture instead of the planet's world.
    pub image: String,
    /// Where the wallpaper changer looks for pictures; empty: the picture's own folder, else
    /// ~/Pictures/Wallpapers.
    pub directory: String,
    /// Minutes between changing to another picture from that folder at random; 0 never does.
    pub interval: u32,
    /// The config's own `image`, before a picture chosen in the changer took its place.
    #[serde(skip)]
    pub configured: String,
    pub fit: Fit,
    /// Without a picture: the planet's hills (`true`) or a quiet gradient in the scheme's colours.
    pub world: bool,
    /// How much of the screen's height the hills take, 0 to 0.8.
    pub terrain: f32,
    /// 0 is smooth, 1 is normal, 2 gets wild.
    pub relief: f32,
    /// How many stars, 0 to 2.
    pub stars: f32,
    /// The film grain that keeps gradients from banding, 0 to 2.
    pub grain: f32,
    /// How one picture gives way to the next: random (a different one each time), none, or one
    /// of `transition::NAMES`; and how long it takes, in milliseconds.
    pub transition: String,
    pub transition_ms: u32,
}

impl Default for WallpaperSettings {
    fn default() -> Self {
        Self { enabled: true, image: String::new(), directory: String::new(), interval: 0, configured: String::new(), fit: Fit::Cover, world: false, terrain: 0.34, relief: 1.0, stars: 0.0, grain: 1.0, transition: "random".into(), transition_ms: 1200 }
    }
}

/// Where the weather is for, and in what units.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct WeatherSettings {
    pub enabled: bool,
    /// A place name; empty takes the city your time zone is named after.
    pub city: String,
    /// Or exact coordinates, which win over the name.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// "metric" (°C, km/h) or "imperial" (°F, mph).
    pub units: String,
}

impl Default for WeatherSettings {
    fn default() -> Self {
        Self { enabled: true, city: String::new(), latitude: None, longitude: None, units: "metric".into() }
    }
}

/// What stands on the desktop.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WidgetSettings {
    /// Any of `widgets::KINDS`; empty shows none.
    pub show: Vec<String>,
    /// Song words under what's playing, from LRCLIB (sends the song's name, nothing else).
    pub lyrics: bool,
    /// The photo frame's folder (empty: ~/Pictures), and the minutes each picture stays.
    pub photos: String,
    pub photo_minutes: u32,
    /// What the sticky note says.
    pub note: String,
}

impl Default for WidgetSettings {
    fn default() -> Self {
        Self { show: vec!["clock".into(), "weather".into(), "media".into()], lyrics: true, photos: String::new(), photo_minutes: 10, note: String::new() }
    }
}

impl WidgetSettings {
    /// The photo frame's folder, as a path.
    pub fn photo_dir(&self) -> PathBuf {
        let dir = self.photos.trim();
        if dir.is_empty() { home().join("Pictures") } else { expand_home(dir) }
    }
}

/// When the screen locks by itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LockSettings {
    /// Minutes without input before it locks; 0 never does. "Stay awake" holds it off.
    pub idle: u32,
    /// Lock before the machine sleeps (closing the lid, suspend from anywhere).
    pub before_sleep: bool,
    /// Unlock with a fingerprint too, when fprintd has a reader and your fingers.
    pub fingerprint: bool,
}

impl Default for LockSettings {
    fn default() -> Self {
        Self { idle: 10, before_sleep: true, fingerprint: true }
    }
}

/// Warmer colours in the evening.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NightLightSettings {
    /// "off", "on", or "schedule" (from `start` to `end`).
    pub mode: String,
    /// How warm at night, in kelvin (6500 is the screen as it is; 4000 is cosy, 3000 very warm).
    pub temperature: u32,
    /// "HH:MM", or "sunset" / "sunrise" from the weather's place.
    pub start: String,
    pub end: String,
}

impl Default for NightLightSettings {
    fn default() -> Self {
        Self { mode: "off".into(), temperature: 4000, start: "sunset".into(), end: "sunrise".into() }
    }
}

/// The focus timer's rhythm, in minutes.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TimerSettings {
    pub focus: u32,
    pub short_break: u32,
    pub long_break: u32,
    /// A long break after this many rounds of focus.
    pub rounds: u32,
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self { focus: 25, short_break: 5, long_break: 15, rounds: 4 }
    }
}

impl TimerSettings {
    pub fn focus_settings(&self) -> crate::timers::FocusSettings {
        let m = |v: u32| crate::timers::mins(v.clamp(1, 600) as u64);
        crate::timers::FocusSettings { work: m(self.focus), short_break: m(self.short_break), long_break: m(self.long_break), rounds: self.rounds.max(1) }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LauncherSettings {
    /// For apps that run in a terminal; empty finds one ($TERMINAL, kitty, foot, alacritty…).
    pub terminal: String,
    /// Where `?question` searches; `{}` is replaced by the question.
    pub search: String,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self { terminal: String::new(), search: "https://duckduckgo.com/?q={}".into() }
    }
}

/// Colours that override the scheme's, as `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Colors {
    pub text: Option<String>,
    pub text_soft: Option<String>,
    pub card: Option<String>,
    pub warn: Option<String>,
    /// The colour the whole scheme grows from; unset takes it from the wallpaper.
    pub accent: Option<String>,
    /// The bar's and the frame's colour.
    pub surface: Option<String>,
    /// Apps that get the scheme too, as files of their own to include: kitty, foot,
    /// alacritty, gtk.
    pub apps: Vec<String>,
    /// The bar's icons: tinted (empty too) leans toward the wallpaper's colour, accent is that
    /// colour, colourful gives each group of icons a colour of its own, plain is the text's.
    /// (Not `icons`: that's the icon set, at the top of the config.)
    pub bar_icons: String,
    /// The desktop's widgets: tinted takes the wallpaper's colour, plain doesn't.
    pub widgets: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The world the bar's light comes from: aster, nere, ocre or vesper.
    pub planet: String,
    pub style: BarStyle,
    pub size: BarSize,
    pub position: Position,
    pub page_style: PageStyle,
    /// Hyprland only: how many pages one group shows (niri shows its own workspaces).
    pub pages: u32,
    pub clock: Clock,
    /// Numbers beside the volume, the battery and the resources.
    pub percent: bool,
    pub media_title: MediaTitle,
    /// Background apps drawn in the bar's own colour.
    pub tray_tint: bool,
    /// The bar's top-right corner opens the control centre.
    pub hot_corner: bool,
    /// A card with the cover and the title when the song changes.
    pub song_popup: bool,
    /// Be the polkit authentication agent (asks your password when an app needs it).
    pub polkit_agent: bool,
    /// How strongly text glows off the wallpaper, 0 to 1 (0: none, the clean look).
    pub glow: f32,
    /// The frame around the screen, in pixels (0 for none), with the `frame` style.
    pub frame: u32,
    /// How round the frame's inner corners are.
    pub frame_radius: u32,
    /// The font family; "Google Sans Flex" and "Poppins" ship with the shell.
    pub font: String,
    /// The icon pack: "outline" (Material Symbols, outlined), "material" (filled) or "phosphor".
    pub icons: String,
    /// Only put a bar on these outputs (empty: every output).
    pub outputs: Vec<String>,
    pub layout: Layout,
    /// The status items, in order: notifications, keyboard, network, bluetooth, volume,
    /// battery, settings, session.
    pub status: Vec<String>,
    /// The resources: cpu, memory, temperature, network.
    pub resources: Vec<String>,
    /// Leave unset for tools that suit the compositor you're on.
    pub tools: Option<Vec<Tool>>,
    pub commands: Commands,
    pub colors: Colors,
    pub notifications: NotificationSettings,
    pub launcher: LauncherSettings,
    pub timers: TimerSettings,
    pub weather: WeatherSettings,
    pub night_light: NightLightSettings,
    pub lock: LockSettings,
    pub widgets: WidgetSettings,
    pub wallpaper: WallpaperSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            planet: "vesper".into(),
            style: BarStyle::Line,
            size: BarSize::Normal,
            position: Position::Top,
            page_style: PageStyle::Dots,
            pages: 8,
            clock: Clock::default(),
            percent: true,
            media_title: MediaTitle::Scroll,
            tray_tint: false,
            hot_corner: true,
            song_popup: true,
            polkit_agent: true,
            glow: 0.0,
            frame: 8,
            frame_radius: 22,
            font: "Google Sans Flex".into(),
            icons: "outline".into(),
            outputs: Vec::new(),
            layout: Layout::default(),
            status: ["timer", "notifications", "keyboard", "network", "bluetooth", "volume", "battery", "control", "session"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            resources: vec!["cpu".into(), "memory".into(), "network".into()],
            tools: None,
            commands: Commands::default(),
            colors: Colors::default(),
            notifications: NotificationSettings::default(),
            launcher: LauncherSettings::default(),
            timers: TimerSettings::default(),
            weather: WeatherSettings::default(),
            night_light: NightLightSettings::default(),
            lock: LockSettings::default(),
            widgets: WidgetSettings::default(),
            wallpaper: WallpaperSettings::default(),
        }
    }
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"));
    base.join("asterism").join("config.toml")
}

/// "~/Pictures/sky.jpg" → "/home/you/Pictures/sky.jpg".
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

impl Config {
    /// Reads the file; a missing file is the defaults, a broken one is reported and the defaults used.
    pub fn load(path: &Path) -> Self {
        let mut config = Self::read(path);
        // A picture chosen in the wallpaper changer stands in for the config's
        config.wallpaper.configured = config.wallpaper.image.clone();
        crate::wallpapers::apply_chosen(&mut config.wallpaper);
        config
    }

    fn read(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(config) => config.sanitized(),
                Err(error) => {
                    log::error!("{}: {error}", path.display());
                    Config::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(error) => {
                log::error!("{}: {error}", path.display());
                Config::default()
            }
        }
    }

    fn sanitized(mut self) -> Self {
        self.pages = self.pages.clamp(4, 10);
        self.glow = self.glow.clamp(0.0, 1.0);
        self
    }
}

/// The tools that work out of the box where you are, when the config doesn't list its own.
pub fn default_tools(niri: bool) -> Vec<Tool> {
    let mut tools = Vec::new();
    if niri {
        tools.push(Tool { icon: "capture".into(), command: String::new(), label: "Screenshot and recording".into(), kind: "capture".into() });
        tools.push(Tool {
            icon: "picker".into(),
            command: "niri msg pick-color | awk '/Hex/ {print $2}' | wl-copy -n".into(),
            label: "Pick a colour".into(),
            kind: String::new(),
        });
    } else {
        tools.push(Tool { icon: "capture".into(), command: String::new(), label: "Screenshot and recording".into(), kind: "capture".into() });
        tools.push(Tool {
            icon: "picker".into(),
            command: "hyprpicker -a".into(),
            label: "Pick a colour".into(),
            kind: String::new(),
        });
    }
    tools.push(Tool { icon: "clipboard".into(), command: String::new(), label: "Clipboard history".into(), kind: "clipboard".into() });
    tools.push(Tool { icon: "microphone".into(), command: String::new(), label: "Microphone".into(), kind: "microphone".into() });
    tools.push(Tool { icon: "coffee".into(), command: String::new(), label: "Stay awake".into(), kind: "awake".into() });
    tools
}

/// Whether every program a shell command starts with can be found, so a tool that
/// can't work doesn't sit in the bar.
pub fn command_available(command: &str) -> bool {
    let programs = command
        .split(['|', ';', '&'])
        .filter_map(|part| part.split_whitespace().find(|word| !word.contains('=')))
        .filter(|word| !word.starts_with('"') && !word.starts_with('$') && !word.starts_with('('));
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    programs.into_iter().all(|program| {
        if program.contains('/') {
            return Path::new(program).exists();
        }
        // Shell builtins and helpers every system has
        if matches!(program, "awk" | "sed" | "echo" | "printf" | "test" | "[") {
            return true;
        }
        dirs.iter().any(|dir| dir.join(program).is_file())
    })
}
