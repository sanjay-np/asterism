//! Everything the bar knows about the machine and the compositor, gathered in one place.
//! Services send `Update`s, the bar reads the `Model`. The bar asks for changes with `Command`s.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// A picture the bar can draw: premultiplied RGBA pixels.
#[derive(Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Unique for the life of the program, so pictures can be cached by it.
    pub id: u64,
}

impl Image {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Arc<Image> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Arc::new(Image { width, height, data, id: NEXT.fetch_add(1, Ordering::Relaxed) })
    }
}

// ---------------------------------------------------------------- compositor

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositorKind {
    #[default]
    Unknown,
    Hyprland,
    Niri,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Window {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    /// Where it sits in its workspace and how big (logical pixels), when the compositor says.
    pub rect: Option<(f32, f32, f32, f32)>,
    pub floating: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    /// Hyprland: the workspace id. niri: its unique id.
    pub id: i64,
    /// The number shown and used to reach it: Hyprland's id, niri's index on its output.
    pub idx: i64,
    pub name: String,
    pub output: String,
    /// Shown on its output.
    pub active: bool,
    /// The one with keyboard focus.
    pub focused: bool,
    /// Ordered by where they sit, left to right.
    pub windows: Vec<Window>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompositorState {
    pub kind: CompositorKind,
    pub workspaces: Vec<Workspace>,
    /// Output name → id of its shown workspace.
    pub active_on: HashMap<String, i64>,
    pub focused_output: String,
    /// Keyboard layout codes, like ["us", "fr"].
    pub layouts: Vec<String>,
    pub layout_names: Vec<String>,
    pub layout: usize,
}

impl CompositorState {
    pub fn workspaces_on<'a>(&'a self, output: &'a str) -> impl Iterator<Item = &'a Workspace> + 'a {
        self.workspaces.iter().filter(move |w| w.output == output)
    }

    pub fn workspace(&self, id: i64) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }
}

// ---------------------------------------------------------------- machine

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Machine {
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub memory: f32,
    pub memory_used_kib: u64,
    pub memory_total_kib: u64,
    /// °C, 0 when no sensor says.
    pub temperature: f32,
    /// Bytes per second.
    pub download: f64,
    pub upload: f64,
    /// Since the bar started.
    pub downloaded: u64,
    pub uploaded: u64,
    /// How busy the graphics card is, 0 to 1, when it says.
    pub gpu: Option<f32>,
    /// Filled disks: where mounted, bytes used and in all.
    pub disks: Vec<(String, u64, u64)>,
    /// Seconds since boot, and the 1-minute load average.
    pub uptime: u64,
    pub load: f32,
    /// The last two minutes, a sample every two seconds, oldest first.
    pub history: MachineHistory,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MachineHistory {
    pub cpu: Vec<f32>,
    pub memory: Vec<f32>,
    pub gpu: Vec<f32>,
    /// Bytes per second.
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
}

/// A running program, for the dashboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    /// Share of the whole machine, 0 to 1.
    pub cpu: f32,
    pub memory_kib: u64,
    /// Yours (so you may end it).
    pub own: bool,
}

// ---------------------------------------------------------------- audio

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioStream {
    pub index: u32,
    pub app: String,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Audio {
    pub available: bool,
    /// 0 to 1.5
    pub volume: f32,
    pub muted: bool,
    pub sink: String,
    pub sinks: Vec<AudioDevice>,
    pub mic_muted: bool,
    pub mic_volume: f32,
    pub has_mic: bool,
    /// Apps recording from a microphone right now.
    pub listening: Vec<String>,
    pub streams: Vec<AudioStream>,
}

// ---------------------------------------------------------------- media

#[derive(Debug, Clone, Default)]
pub struct Player {
    pub bus: String,
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: String,
    pub playing: bool,
    pub paused: bool,
    /// Microseconds.
    pub length: i64,
    pub position: i64,
    pub position_at: Option<Instant>,
    pub rate: f64,
    pub can_next: bool,
    pub can_previous: bool,
}

impl Player {
    /// Where the track is now, moving on from the last time the player said.
    pub fn position_now(&self) -> i64 {
        match self.position_at {
            Some(at) if self.playing => {
                let moved = at.elapsed().as_secs_f64() * 1_000_000.0 * self.rate.max(0.0);
                (self.position + moved as i64).min(if self.length > 0 { self.length } else { i64::MAX })
            }
            _ => self.position,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Media {
    pub players: Vec<Player>,
    /// Index into `players` of the one everything follows.
    pub current: Option<usize>,
}

impl Media {
    pub fn player(&self) -> Option<&Player> {
        self.current.and_then(|i| self.players.get(i))
    }
}

// ---------------------------------------------------------------- power

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Battery {
    pub present: bool,
    /// 0 to 1
    pub level: f32,
    pub charging: bool,
    pub on_battery: bool,
    pub full: bool,
    /// Seconds, 0 when unknown.
    pub time_to_empty: i64,
    pub time_to_full: i64,
    pub profiles: Vec<String>,
    pub profile: String,
}

// ---------------------------------------------------------------- network

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessPoint {
    pub ssid: String,
    pub strength: u8,
    pub secure: bool,
    pub active: bool,
    /// Saved in NetworkManager: it joins without asking.
    pub known: bool,
    /// Needs more than a password (802.1X: a user name, certificates).
    pub enterprise: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Network {
    pub available: bool,
    pub wifi_enabled: bool,
    /// "wifi", "ethernet", "vpn", other NM types, or "" when offline.
    pub kind: String,
    pub name: String,
    /// 0 to 100, for wifi.
    pub strength: u8,
    pub address: String,
    pub access_points: Vec<AccessPoint>,
    /// The VPNs NetworkManager has saved (VPN plugins and WireGuard).
    pub vpns: Vec<Vpn>,
}

/// A saved VPN: its name, its NetworkManager settings path, and whether it's up (or coming up).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vpn {
    pub name: String,
    pub path: String,
    pub active: bool,
    pub connecting: bool,
}

// ---------------------------------------------------------------- bluetooth

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BluetoothDevice {
    pub path: String,
    pub name: String,
    pub icon: String,
    pub connected: bool,
    pub paired: bool,
    /// 0 to 100, -1 when it doesn't say.
    pub battery: i32,
    /// Signal strength while discovering (dBm), for devices nearby.
    pub rssi: Option<i16>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bluetooth {
    pub available: bool,
    pub adapter: String,
    pub powered: bool,
    /// Yours: paired (or connected).
    pub devices: Vec<BluetoothDevice>,
    /// Others in reach while looking, strongest first.
    pub nearby: Vec<BluetoothDevice>,
    pub discovering: bool,
}

/// What a device being paired asks of you.
#[derive(Debug, Clone, PartialEq)]
pub enum PairAsk {
    /// Does this number match the one on the device?
    Confirm(u32),
    /// Type the PIN (text) or passkey (digits) the device shows or expects.
    Pin,
    Passkey,
    /// Type this on the device.
    ShowPin(String),
    ShowPasskey(u32),
    /// May it pair (it asked, not you)?
    Authorize,
    /// May it use this service?
    Service(String),
}

/// A pairing question, for the card: which device, and what.
#[derive(Debug, Clone, PartialEq)]
pub struct Pairing {
    pub device: String,
    pub ask: PairAsk,
}

impl Bluetooth {
    pub fn connected(&self) -> impl Iterator<Item = &BluetoothDevice> {
        self.devices.iter().filter(|d| d.connected)
    }
}

// ---------------------------------------------------------------- tray

#[derive(Debug, Clone)]
pub struct TrayItem {
    /// "bus/path", unique.
    pub id: String,
    pub title: String,
    pub status: String,
    pub icon: Option<Arc<Image>>,
    pub item_is_menu: bool,
    pub has_menu: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    /// "checkmark", "radio" or "".
    pub toggle: String,
    pub checked: bool,
    pub children: Vec<MenuEntry>,
}

// ---------------------------------------------------------------- notifications

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notifications {
    pub available: bool,
    pub count: u32,
    pub quiet: bool,
    /// The bar is the notification daemon: `list` holds them, newest first.
    pub own: bool,
    pub list: Vec<Notification>,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    /// The app's icon, or the picture the notification brings (an avatar, a cover).
    pub image: Option<Arc<Image>>,
    /// (key, label); "default" is what a click on the notification does.
    pub actions: Vec<(String, String)>,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    pub time: chrono::DateTime<chrono::Local>,
    /// Milliseconds: -1 the daemon decides, 0 never goes by itself.
    pub timeout: i32,
    pub resident: bool,
    pub transient: bool,
}

impl PartialEq for Notification {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.summary == other.summary
            && self.body == other.body
            && self.actions == other.actions
            && self.image.as_ref().map(|i| i.id) == other.image.as_ref().map(|i| i.id)
    }
}

// ---------------------------------------------------------------- brightness

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Brightness {
    pub available: bool,
    /// 0 to 1
    pub value: f32,
}

// ---------------------------------------------------------------- authentication

/// A password, or anything as private: never printed, and overwritten with zeros when dropped.
#[derive(Default)]
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn push(&mut self, text: &str) {
        self.0.extend_from_slice(text.as_bytes());
    }

    /// Takes the last character away (UTF-8 aware), zeroing its bytes.
    pub fn pop(&mut self) {
        let mut start = self.0.len();
        while start > 0 {
            start -= 1;
            // A character starts at any byte that isn't a continuation byte (10xxxxxx)
            if self.0[start] & 0b1100_0000 != 0b1000_0000 {
                break;
            }
        }
        wipe(&mut self.0[start..]);
        self.0.truncate(start);
    }

    pub fn clear(&mut self) {
        wipe(&mut self.0);
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        String::from_utf8_lossy(&self.0).chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// Hands the secret over, leaving this one empty.
    pub fn take(&mut self) -> Secret {
        Secret(std::mem::take(&mut self.0))
    }
}

/// Overwrites memory in a way the compiler won't optimise away.
pub fn wipe(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: a plain write to a byte we own
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Commands are cloned rarely; a cloned secret is wiped on its own drop too.
impl Clone for Secret {
    fn clone(&self) -> Self {
        Secret(self.0.clone())
    }
}

/// Polkit asking who you are, for something that needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AuthRequest {
    pub cookie: String,
    /// What it's for, as the app says it ("Authentication is required to …").
    pub message: String,
    pub action: String,
    /// The account whose password is asked for.
    pub user: String,
    /// The last attempt didn't work, and why.
    pub error: String,
    /// A password is being checked.
    pub busy: bool,
    /// More requests waiting behind this one.
    pub waiting: usize,
}

// ---------------------------------------------------------------- asked from outside

/// What `asterism msg …` asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Launcher(Option<bool>),
    Control(Option<bool>),
    Menu(String),
    Notifications,
    Quiet,
    ClearNotifications,
    Volume(f32),
    Mute,
    MicMute,
    Brightness(f32),
    Osd(String),
    Media(String),
    Awake,
    Reload,
    Wallpaper(WallpaperRequest),
    Timer(TimerAction),
    /// Night light on, off, or (None) the other way.
    NightLight(Option<bool>),
    /// The keyboard shortcuts sheet, open or closed (None toggles).
    Shortcuts(Option<bool>),
    /// The clipboard history, open or closed (None toggles).
    Clipboard(Option<bool>),
    Screenshot(Capture),
    /// Start recording (with sound or not), or stop if it already is.
    Record { audio: bool },
    Lock,
    Dashboard(Option<bool>),
    Settings(Option<bool>),
    Overview(Option<bool>),
    /// Arranging the desktop's widgets: start, finish, or (None) the other way.
    Widgets(Option<bool>),
    /// Something for the pets: feed, play, yarn, bed, nobed, next, away, friend next, friend away.
    Pet(String),
    /// The text in a region of the screen onto the clipboard.
    ScreenText,
}

/// Something done to a timer, from its menu or from `asterism msg`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimerAction {
    /// A countdown of this many seconds.
    Countdown(u64),
    /// Adds (or with a negative number takes) seconds from the countdown.
    CountdownAdd(i64),
    CountdownPause,
    CountdownCancel,
    /// Starts the focus timer, or stops it.
    Focus,
    FocusPause,
    FocusSkip,
    Stopwatch,
    StopwatchLap,
    StopwatchReset,
}

/// A notification the shell sends about itself.
#[derive(Debug, Clone, Default)]
pub struct Note {
    pub summary: String,
    pub body: String,
    /// An icon name or a path.
    pub icon: String,
    /// 0 low, 1 normal, 2 critical (stays until answered).
    pub urgency: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WallpaperRequest {
    /// Open or close the changer (`None` toggles).
    Picker(Option<bool>),
    /// The next or previous picture in the folder, or (0) one at random.
    Step(i32),
    Set(String),
}

// ---------------------------------------------------------------- the lot

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub compositor: CompositorState,
    pub machine: Machine,
    pub audio: Audio,
    pub media: Media,
    pub battery: Battery,
    pub network: Network,
    pub bluetooth: Bluetooth,
    pub tray: Vec<TrayItem>,
    pub notifications: Notifications,
    pub brightness: Brightness,
    /// Polkit asking for a password.
    pub auth: Option<AuthRequest>,
    /// A tray item's menu, as last fetched.
    pub menus: HashMap<String, Vec<MenuEntry>>,
    /// Album covers by their URL.
    pub art: HashMap<String, Arc<Image>>,
    /// Apps' icons by app id, `None` when there isn't one.
    pub app_icons: HashMap<String, Option<Arc<Image>>>,
    /// The wallpaper changer's thumbnails, `None` when a picture couldn't be read.
    pub thumbnails: HashMap<std::path::PathBuf, Option<Arc<Image>>>,
    pub awake: bool,
    /// The countdown, the focus timer and the stopwatch (kept by the app, not a service).
    pub timers: crate::timers::Timers,
    pub weather: crate::weather::Weather,
    pub night: crate::night::NightStatus,
    /// What's been copied, newest first (in memory only).
    pub clipboard: crate::clipboard::History,
    /// The screen being recorded, since when.
    pub recording: Option<Recording>,
    /// What's running, while the dashboard wants to know.
    pub processes: Vec<Process>,
    /// Songs' words with their times, by `widgets::lyrics_key` (empty: none found).
    pub lyrics: HashMap<String, Vec<(f64, String)>>,
    /// What the speakers play, as levels 0 to 1 from low to high (empty: not listening).
    pub spectrum: Vec<f32>,
    /// The photo frame's folder, and the pictures in it.
    pub photos: Option<(std::path::PathBuf, Vec<std::path::PathBuf>)>,
}

/// What a screenshot takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Region,
    Screen,
    Window,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub since: chrono::DateTime<chrono::Local>,
    pub audio: bool,
    pub path: std::path::PathBuf,
}

pub enum Update {
    Compositor(CompositorState),
    Machine(Machine),
    Audio(Audio),
    Media(Media),
    Battery(Battery),
    Network(Network),
    Bluetooth(Bluetooth),
    Tray(Vec<TrayItem>),
    TrayMenu(String, Vec<MenuEntry>),
    Notifications(Notifications),
    /// A notification to show as it arrives (new, or replacing one).
    Toast(u32),
    Brightness(Brightness),
    Auth(Option<AuthRequest>),
    Request(Request),
    Art(String, Arc<Image>),
    AppIcon(String, Option<Arc<Image>>),
    Thumbnail(std::path::PathBuf, Option<Arc<Image>>),
    Weather(crate::weather::Weather),
    Recording(Option<Recording>),
    Processes(Vec<Process>),
    Lyrics(String, Vec<(f64, String)>),
    /// How joining a wifi network went: which, whether it did, and why not.
    WifiJoined(String, bool, String),
    /// Pairing asks something (or, None, stopped asking).
    Pairing(Option<Pairing>),
    /// Time to lock the screen (logind asked, or the machine's about to sleep).
    LockNow,
    /// A service has news for you: the app posts it as a notification.
    Notify(Note),
    /// The colours grown from this wallpaper.
    WallpaperPalette(String, crate::palette::Palette),
    Awake(bool),
    /// The visualizer's levels, about thirty times a second while it listens.
    Spectrum(Vec<f32>),
    /// The pictures in a folder, for the photo frame.
    Photos(std::path::PathBuf, Vec<std::path::PathBuf>),
}

impl Model {
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Compositor(state) => self.compositor = state,
            Update::Machine(machine) => self.machine = machine,
            Update::Audio(audio) => self.audio = audio,
            Update::Media(media) => self.media = media,
            Update::Battery(battery) => self.battery = battery,
            Update::Network(network) => self.network = network,
            Update::Bluetooth(bluetooth) => self.bluetooth = bluetooth,
            Update::Tray(items) => {
                self.menus.retain(|id, _| items.iter().any(|item| &item.id == id));
                self.tray = items;
            }
            Update::TrayMenu(id, menu) => {
                self.menus.insert(id, menu);
            }
            Update::Notifications(notifications) => self.notifications = notifications,
            Update::Brightness(brightness) => self.brightness = brightness,
            Update::Auth(auth) => self.auth = auth,
            // Handled by the app before the model sees them
            Update::Toast(_) | Update::Request(_) | Update::WallpaperPalette(..) | Update::Notify(_) | Update::LockNow | Update::WifiJoined(..) | Update::Pairing(_) => {}
            Update::Recording(recording) => self.recording = recording,
            Update::Processes(list) => self.processes = list,
            Update::Lyrics(key, lines) => {
                if self.lyrics.len() > 24 {
                    self.lyrics.clear();
                }
                self.lyrics.insert(key, lines);
            }
            Update::Weather(weather) => self.weather = weather,
            Update::Thumbnail(path, image) => {
                self.thumbnails.insert(path, image);
            }
            Update::Art(url, image) => {
                // A few covers are plenty: the playing one, and the ones just before
                if self.art.len() >= 6 {
                    self.art.clear();
                }
                self.art.insert(url, image);
            }
            Update::AppIcon(app, icon) => {
                self.app_icons.insert(app, icon);
            }
            Update::Awake(on) => self.awake = on,
            Update::Spectrum(levels) => self.spectrum = levels,
            Update::Photos(dir, list) => self.photos = Some((dir, list)),
        }
    }
}

#[derive(Debug, Clone)]
pub enum WorkspaceTarget {
    /// Hyprland id or niri unique id.
    Id(i64),
    /// One up or down from where you are, on this output.
    Step(String, i32),
}

#[derive(Debug, Clone)]
pub enum Command {
    FocusWorkspace(WorkspaceTarget),
    NextLayout,
    /// A window: go to it, close it, or move it to a workspace (by id) without following it.
    FocusWindow(u64),
    CloseWindow(u64),
    MoveWindow(u64, i64),
    SetLayout(usize),
    Quit,
    MediaPlayPause(String),
    MediaNext(String),
    MediaPrevious(String),
    /// Microseconds.
    MediaSeek(String, i64),
    MediaChoose(String),
    FetchArt(String),
    /// Look up an app's icon in the icon theme, by its app id.
    AppIcon(String),
    /// A wallpaper's thumbnail, from the cache or made.
    Thumbnail(std::path::PathBuf),
    /// Grow the colours from this picture.
    WallpaperPalette(String),
    /// The shell's own news, through whichever notification daemon runs (often itself).
    Notify(Note),
    Screenshot(Capture),
    RecordStart { audio: bool },
    RecordStop,
    /// Hold sleep back to lock first, or don't.
    LockBeforeSleep(bool),
    /// Look at what's running (every couple of seconds), or stop looking.
    WatchProcesses(bool),
    /// Listen to what the speakers play, for the visualizer, or stop.
    WatchSpectrum(bool),
    /// A saved VPN (by its settings path) up or down.
    SetVpn(String, bool),
    /// The text in a region of the screen, chosen now, onto the clipboard.
    ScreenText,
    /// The pet speaks: this sound, this take of it, this loud (0 to 1).
    PetSound(crate::pet::Voice, u32, f32),
    /// List a folder's pictures, for the photo frame.
    ListPhotos(std::path::PathBuf),
    /// Write the scheme's files for these apps (`appcolors`).
    AppColors(Box<crate::palette::Palette>, Vec<String>),
    /// Asks a program to end (SIGTERM).
    EndProcess(u32),
    /// Joins a wifi network with its password (saved only if it works).
    ConnectWifiWith(String, Secret),
    /// A song's words, from LRCLIB.
    FetchLyrics { artist: String, title: String, album: String, seconds: u32 },
    /// Where the weather is wanted, and how; then fetch it again now.
    WeatherSettings(crate::config::WeatherSettings),
    WeatherRefresh,
    /// Which apps' notifications pop up, stay quiet, or are dropped (names in lower case).
    NotificationRules(std::collections::HashMap<String, crate::config::AppRule>),
    SetVolume(f32),
    StepVolume(f32),
    ToggleMute,
    SetDefaultSink(String),
    ToggleMicMute,
    SetStreamVolume(u32, f32),
    SetPowerProfile(String),
    SetWifi(bool),
    ScanWifi,
    ConnectWifi(String),
    SetBluetoothPower(bool),
    /// Look for devices nearby for a while.
    ScanBluetooth,
    /// Pair with this device, then trust and connect it.
    BluetoothPair(String),
    /// Forget a paired device.
    BluetoothForget(String),
    /// The answer to a pairing question: None says no; for a yes/no question, Some("").
    PairAnswer(Option<String>),
    BluetoothConnect(String, bool),
    TrayActivate(String, i32, i32),
    TraySecondary(String, i32, i32),
    TrayScroll(String, i32),
    TrayMenu(String),
    TrayMenuClick(String, i32),
    ToggleAwake,
    NotificationsToggle,
    NotificationsQuiet,
    /// Runs a notification's action ("default" for a click on it).
    NotificationInvoke(u32, String),
    /// Closes a notification for good.
    NotificationDismiss(u32),
    /// Its pop-up went by; a transient notification goes with it.
    NotificationExpired(u32),
    NotificationsClear,
    SetBrightness(f32),
    StepBrightness(f32),
    /// The password for this request.
    AuthRespond(String, Secret),
    AuthCancel(String),
    Run(String),
}
