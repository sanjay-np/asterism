//! The Wayland side: one layer-shell surface per output for the bar, an xdg popup for the open
//! menu, the pointer, and the loop that ties services, timers and drawing together.

use crate::bar::{self, Layout};
use crate::config::{self, BarStyle, Config, Tool};
use crate::model::{Command, Model, Notification, Update};
use crate::popups::{self, Kind};
use crate::render::Painter;
use crate::render::text::Font;
use crate::theme::Theme;
use crate::ui::{Action, Animator, Ease, Piece, Prim, Slider};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            self, EventLoop, LoopHandle,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Proxy, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        },
        protocols::wp::{
            cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
            fractional_scale::v1::client::{
                wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
                wp_fractional_scale_v1::{self, WpFractionalScaleV1},
            },
            idle_inhibit::zv1::client::{zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1},
            viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
        },
        protocols::xdg::shell::client::xdg_positioner,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, cursor_shape::CursorShapeManager},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        xdg::{
            XdgPositioner, XdgShell,
            popup::{Popup, PopupConfigure, PopupHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use tiny_skia::{Pixmap, PixmapPaint, Transform};
use tokio::sync::mpsc::UnboundedSender;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// The keys a menu answers to (sctk's key symbols are mapped onto these).
mod key {
    pub const ESC: u32 = 1;
    pub const BACKSPACE: u32 = 14;
    pub const TAB: u32 = 15;
    pub const ENTER: u32 = 28;
    pub const SPACE: u32 = 57;
    pub const KP_ENTER: u32 = 96;
    pub const HOME: u32 = 102;
    pub const UP: u32 = 103;
    pub const PAGE_UP: u32 = 104;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const END: u32 = 107;
    pub const DOWN: u32 = 108;
    pub const PAGE_DOWN: u32 = 109;
}

/// Room around a menu's card for the halos of its stars.
pub const POPUP_MARGIN: f32 = 14.0;

mod backdrop;
mod clipboard;
mod day;
mod desk;
mod dock;
mod idle;
mod night;
mod overview;
mod peek;
mod pet;
mod shell;
mod timer;
mod wallpaper;
mod wifi;
use backdrop::{Backdrop, Edge, Painted};
pub use shell::preview;

/// The desktop's widgets drawn into an offscreen surface, for snapshots.
pub fn render_desk_snapshot(surf: &mut Surf, pieces: Vec<Piece>, painter: &mut Painter, theme: &Theme, background: Option<crate::theme::Rgba>) {
    for _ in 0..2 {
        surf.begin_frame();
        shell::render_panel(surf, pieces.clone(), painter, theme, background, 0.0);
        // Past the fade-in
        std::thread::sleep(Duration::from_millis(350));
    }
}
use shell::{Panel, PanelKind, Toast};
/// Text sliding by moves this many pixels a second.
const MARQUEE_SPEED: f32 = 33.0;

/// For protocol objects that never send anything.
pub struct Silent;

impl<I: Proxy> Dispatch2<I, App> for Silent {
    fn event(&self, _: &mut App, _: &I, _: I::Event, _: &Connection, _: &QueueHandle<App>) {}
}

/// Tells a surface the scale its output prefers, in 120ths.
pub struct ScaleData(wl_surface::WlSurface);

impl Dispatch2<WpFractionalScaleV1, App> for ScaleData {
    fn event(&self, app: &mut App, _: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && let Some(surf) = app.surf_mut(&self.0)
            && surf.scale120 != scale
        {
            surf.scale120 = scale;
            surf.dirty = true;
        }
    }
}

/// What every surface the bar draws on has: its size and scale, its pieces, and the pointer on it.
pub struct Surf {
    id: u32,
    wl: Option<wl_surface::WlSurface>,
    viewport: Option<WpViewport>,
    _fractional: Option<WpFractionalScaleV1>,
    scale120: u32,
    int_scale: i32,
    w: f32,
    h: f32,
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    wants_frames: bool,
    anim: Animator,
    pieces: Vec<Piece>,
    hovered: Option<String>,
    pressed: Option<String>,
    pointer: Option<(f32, f32)>,
    /// A star under the pointer: where it is and whose window it is.
    target: Option<(f32, String, String)>,
    /// The piece the keyboard is on, in a menu.
    focus: Option<String>,
    /// In a menu that scrolls, the band of it that shows (top and bottom).
    visible: Option<(f32, f32)>,
    /// The last frame, so a frame where little changed only paints that little.
    last: Option<LastFrame>,
    /// Buffers sent before, each with what changed since it was last written: once the
    /// compositor lets go of one, it only needs that part brought up to date.
    buffers: Vec<(smithay_client_toolkit::shm::slot::Buffer, Option<Damage>)>,
    /// A big surface's own buffer pool (see `BIG_BUFFER`), and what makes one.
    own_pool: Option<SlotPool>,
    shm: Option<wl_shm::WlShm>,
}

/// What each piece looked like and where, last time.
#[derive(PartialEq)]
struct Entry {
    key: String,
    hash: u64,
    bounds: (i32, i32, i32, i32),
    look: (u32, u32),
}

struct LastFrame {
    canvas: Pixmap,
    entries: Vec<Entry>,
    stars: Vec<(i32, i32, i32)>,
}

/// A rectangle of pixels on a canvas: x, y, width, height.
type Damage = (i32, i32, i32, i32);

fn union(a: Option<Damage>, b: Damage) -> Damage {
    match a {
        None => b,
        Some(a) => {
            let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
            let (x1, y1) = ((a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
            (x0, y0, x1 - x0, y1 - y0)
        }
    }
}

fn overlaps(a: Damage, b: Damage) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Where a piece's picture lands on the canvas, glow and growth included.
fn bounds(piece: &Piece, lift: f32, shift_y: f32, scale: f32) -> Damage {
    let room = piece.pad + if piece.glow { crate::render::GLOW_ROOM } else { 0.0 };
    let (cx, cy) = (piece.x + piece.w / 2.0, piece.y + piece.h / 2.0 + shift_y);
    let (hw, hh) = ((piece.w / 2.0 + room) * lift.max(1.0), (piece.h / 2.0 + room) * lift.max(1.0));
    let x0 = ((cx - hw) * scale).floor() as i32 - 2;
    let y0 = ((cy - hh) * scale).floor() as i32 - 2;
    let x1 = ((cx + hw) * scale).ceil() as i32 + 2;
    let y1 = ((cy + hh) * scale).ceil() as i32 + 2;
    (x0, y0, x1 - x0, y1 - y0)
}

impl Surf {
    /// A surface that only exists in memory, for snapshots.
    pub fn offscreen(w: f32, h: f32, scale: f32) -> Surf {
        Surf {
            id: 0,
            wl: None,
            viewport: None,
            _fractional: None,
            scale120: (scale * 120.0).round() as u32,
            int_scale: 1,
            w,
            h,
            configured: true,
            dirty: true,
            frame_pending: false,
            wants_frames: false,
            anim: Animator::new(),
            pieces: Vec::new(),
            hovered: None,
            pressed: None,
            pointer: None,
            target: None,
            focus: None,
            visible: None,
            last: None,
            buffers: Vec::new(),
            own_pool: None,
            shm: None,
        }
    }

    pub fn canvas(&self) -> Option<Pixmap> {
        self.last.as_ref().map(|l| l.canvas.clone())
    }

    pub fn begin_frame(&mut self) {
        self.anim.begin();
    }

    pub fn focus_for_snapshot(&mut self, key: Option<String>) {
        self.focus = key;
    }

    pub fn hover_for_snapshot(&mut self, key: Option<String>) {
        self.hovered = key;
    }

    fn is(&self, wl: &wl_surface::WlSurface) -> bool {
        self.wl.as_ref() == Some(wl)
    }

    fn scale(&self) -> f32 {
        if self.scale120 > 0 { self.scale120 as f32 / 120.0 } else { self.int_scale.max(1) as f32 }
    }

    fn hit(&self, x: f32, y: f32) -> Option<&Piece> {
        // What has scrolled out of a menu can't be clicked
        if let Some((top, bottom)) = self.visible
            && (y < top || y > bottom)
        {
            return None;
        }
        self.pieces.iter().rev().find(|p| {
            let interactive = !p.actions.is_empty() || p.label.is_some() || !p.targets.is_empty() || p.blocks;
            interactive && p.contains(x, y, if p.highlight { 0.0 } else { 5.0 })
        })
    }
}

struct BarSurface {
    surf: Surf,
    layer: LayerSurface,
    output: wl_output::WlOutput,
    output_name: String,
    /// The output's height, logical pixels: how tall a menu can grow.
    output_height: f32,
    /// And its width, to find the frame's corners.
    output_width: f32,
}

/// Which surface a slider is being dragged on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragOn {
    Popup,
    Control,
}

struct Drag {
    key: String,
    slider: Slider,
    max: f32,
    left: f32,
    width: f32,
    sent: Instant,
    value: f32,
}

/// A piece of a menu the keyboard can reach, where it sits in the menu's content.
struct Focusable {
    key: String,
    y: f32,
    h: f32,
    left: Option<Action>,
    drag: Option<(Slider, f32)>,
}

/// How far a menu has scrolled, how tall its content is and how much of it shows.
#[derive(Clone, Copy)]
pub struct Scroll {
    pub offset: f32,
    pub content: f32,
    pub view: f32,
}

struct PopupSurface {
    surf: Surf,
    popup: Popup,
    kind: Kind,
    parent: u32,
    anchor: (i32, i32, i32, i32),
    state: popups::State,
    /// Where the content is scrolled to (the target; the drawing glides there).
    scroll: f32,
    content_h: f32,
    view_h: f32,
    focusables: Vec<Focusable>,
}

impl PopupSurface {
    fn max_scroll(&self) -> f32 {
        (self.content_h - self.view_h).max(0.0)
    }

    fn scroll_to(&mut self, offset: f32) {
        let offset = offset.clamp(0.0, self.max_scroll());
        if (offset - self.scroll).abs() > 0.1 {
            self.scroll = offset;
            self.surf.dirty = true;
        }
    }

    /// Scrolls just enough for a piece of the content to show whole.
    fn reveal(&mut self, y: f32, h: f32) {
        let margin = 10.0;
        if y - margin < self.scroll {
            self.scroll_to(y - margin);
        } else if y + h + margin > self.scroll + self.view_h {
            self.scroll_to(y + h + margin - self.view_h);
        }
    }
}

pub struct App {
    registry: RegistryState,
    seat_state: SeatState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    xdg: XdgShell,
    shm: Shm,
    pool: SlotPool,
    cursor_shapes: Option<CursorShapeManager>,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    qh: QueueHandle<App>,
    handle: LoopHandle<'static, App>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor: Option<WpCursorShapeDeviceV1>,
    /// The pointer's own motion, for drags (a widget, the pet): the manager, and ours.
    relative_manager: smithay_client_toolkit::seat::relative_pointer::RelativePointerState,
    relative: Option<smithay_client_toolkit::reexports::protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::ZwpRelativePointerV1>,
    cursor_shape: Option<Shape>,
    enter_serial: u32,
    scroll: f64,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    shift: bool,
    modifiers: smithay_client_toolkit::seat::keyboard::Modifiers,
    /// The launcher, the notification pop-ups and the level, when they show.
    panels: Vec<Panel>,
    toasts: Vec<Toast>,
    toasts_hovered: bool,
    osd: Option<(crate::cards::Level, u64)>,
    osd_generation: u64,
    launcher: crate::launcher::Launcher,
    /// The wallpaper changer, while it's open; when the picture last changed.
    picker: Option<crate::wallpapers::Picker>,
    /// The keyboard shortcuts sheet, while it's open.
    sheet: Option<crate::shortcuts::Sheet>,
    /// The dashboard, while it's open.
    dashboard: Option<crate::dashboard::Dashboard>,
    /// The settings app, while it's open.
    settings: Option<crate::settings::Settings>,
    /// A wifi network asking for its password.
    wifi: Option<wifi::WifiAsk>,
    /// A Bluetooth pairing question, and what's been typed for it.
    pair: Option<(crate::model::Pairing, String)>,
    /// The window overview, while it's open.
    overview: Option<crate::overview::Overview>,
    /// Desktop widgets: a surface per output, where each widget stands, and one being moved.
    desks: Vec<desk::Desk>,
    widget_places: HashMap<String, crate::widgets::Place>,
    widget_drag: Option<desk::WidgetDrag>,
    /// The widgets are being arranged (the desks raised above the windows, with the editor).
    desk_edit: bool,
    /// The visualizer's capture is running.
    spectrum_on: bool,
    /// The surface the keyboard is on, of ours.
    keyboard_on: Option<wl_surface::WlSurface>,
    /// The desktop's right-click menu, while it's open.
    desk_menu: Option<desk::DeskMenu>,
    /// Pictures the photo frame was asked to skip.
    photo_step: u64,
    /// The song last seen playing (player, artist, title), and the card for a new one.
    last_track: Option<String>,
    song_generation: u64,
    /// When the shell started: songs already playing then aren't news.
    started: Instant,
    /// The pet, its timer running, whether you've been away a minute (and the watch that
    /// says so), and where the pointer was last seen on its output.
    pets: Vec<pet::PetSurface>,
    /// The workspace card under a page dot, and the dot it's waiting on (workspace, bar, x).
    peek: Option<peek::Peek>,
    peek_wait: Option<(i64, usize, f32)>,
    dock: dock::DockState,
    /// Bluetooth devices already warned about running low (by path).
    bt_warned: HashSet<String>,
    pet_timer: bool,
    pet_idle: bool,
    pet_idle_watch: Option<smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::ExtIdleNotificationV1>,
    pet_pointer: Option<((f32, f32), Instant)>,
    /// The pet's bowl and ball, while they're out.
    props: Vec<pet::Prop>,
    /// The clipboard: the compositor's data-control, our device on the seat, the selection we
    /// offer (when one was chosen from the history), the picker, and where reads come back.
    clip_manager: Option<smithay_client_toolkit::reexports::protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1>,
    clip_device: Option<smithay_client_toolkit::reexports::protocols::ext::data_control::v1::client::ext_data_control_device_v1::ExtDataControlDeviceV1>,
    clip_source: Option<smithay_client_toolkit::reexports::protocols::ext::data_control::v1::client::ext_data_control_source_v1::ExtDataControlSourceV1>,
    clip_picker: Option<crate::clipboard::Picker>,
    clip_sender: calloop::channel::Sender<clipboard::ClipRead>,
    /// Idle: the compositor's notifier, and the notification for our timeout (with its minutes).
    idle_notifier: Option<smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1>,
    idle_watch: Option<(smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::ExtIdleNotificationV1, u32)>,
    /// Night light: the compositor's gamma controls, one per output; a hand on the switch, and
    /// what the schedule said last.
    gamma_manager: Option<smithay_client_toolkit::reexports::protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1>,
    gammas: Vec<night::Gamma>,
    /// Some screen's colours are held by another program: don't keep asking.
    gamma_refused: bool,
    night_forced: Option<bool>,
    night_scheduled: Option<bool>,
    wallpaper_changed: Instant,
    /// The slider being dragged, and where.
    drag: Option<(DragOn, Drag)>,
    /// What's typed in polkit's dialog, and for which request.
    password: crate::model::Secret,
    auth_cookie: Option<String>,
    /// What the control centre remembers while open (an armed button, a slider being dragged).
    control_state: popups::State,
    /// What the day panel remembers while open (the month shown, the field being typed in).
    day_state: popups::State,
    /// When the to-do list or notes last changed and haven't been written yet.
    day_dirty: Option<Instant>,
    idle_inhibit: Option<ZwpIdleInhibitManagerV1>,
    /// Holds the screen awake, on the bar with this id.
    inhibitor: Option<(ZwpIdleInhibitorV1, u32)>,

    bars: Vec<BarSurface>,
    /// The wallpapers, one per output.
    backdrops: Vec<Backdrop>,
    /// The frame's left, right and far edges, three per output.
    edges: Vec<Edge>,
    /// Where painted wallpapers come back.
    painted: calloop::channel::Sender<Painted>,
    popup: Option<PopupSurface>,
    next_id: u32,

    config_path: PathBuf,
    config_stamp: Option<SystemTime>,
    config: Config,
    theme: Theme,
    tools: Vec<Tool>,
    model: Model,
    painter: Painter,
    commands: UnboundedSender<Command>,
    requested: HashSet<String>,
    marquee: HashMap<String, f32>,
    marquee_at: Instant,
    marquee_timer: bool,
    last_minute: String,
    /// The wallpaper's last animation, so the next one is another.
    last_transition: Option<&'static str>,
}

fn stamp(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The tools for this config: its own, or ones that suit the compositor; those that can't run are left out.
pub fn tools_for(config: &Config, can_inhibit: bool) -> Vec<Tool> {
    let niri = crate::services::detect_compositor() == crate::model::CompositorKind::Niri;
    let tools = config.tools.clone().unwrap_or_else(|| config::default_tools(niri));
    tools
        .into_iter()
        .filter(|tool| !tool.kind.is_empty() || (config::command_available(&tool.command) && crate::render::icon_svg(&tool.icon, &config.icons).is_some()))
        .filter(|tool| tool.kind != "awake" || can_inhibit || config::command_available("systemd-inhibit"))
        .collect()
}

impl App {
    fn surf_mut(&mut self, wl: &wl_surface::WlSurface) -> Option<&mut Surf> {
        if let Some(popup) = self.popup.as_mut()
            && popup.surf.is(wl)
        {
            return Some(&mut popup.surf);
        }
        if let Some(panel) = self.panels.iter_mut().find(|p| p.surf.is(wl)) {
            return Some(&mut panel.surf);
        }
        if let Some(backdrop) = self.backdrops.iter_mut().find(|b| b.surf.is(wl)) {
            return Some(&mut backdrop.surf);
        }
        if let Some(edge) = self.edges.iter_mut().find(|e| e.surf.is(wl)) {
            return Some(&mut edge.surf);
        }
        if let Some(surf) = self.desks.iter_mut().find_map(|d| d.surf_mut(wl)) {
            return Some(surf);
        }
        if let Some(peek) = self.peek.as_mut().filter(|p| p.surf.is(wl)) {
            return Some(&mut peek.surf);
        }
        if let Some(pet) = self.pets.iter_mut().find(|p| p.surf.is(wl)) {
            return Some(&mut pet.surf);
        }
        if let Some(prop) = self.props.iter_mut().find(|p| p.surf.is(wl)) {
            return Some(&mut prop.surf);
        }
        self.bars.iter_mut().find(|b| b.surf.is(wl)).map(|b| &mut b.surf)
    }

    fn new_surf(&mut self, wl: wl_surface::WlSurface) -> Surf {
        self.next_id += 1;
        let viewport = self.viewporter.as_ref().map(|v| v.get_viewport(&wl, &self.qh, Silent));
        let fractional = match (&self.fractional, &viewport) {
            (Some(manager), Some(_)) => Some(manager.get_fractional_scale(&wl, &self.qh, ScaleData(wl.clone()))),
            _ => None,
        };
        Surf {
            id: self.next_id,
            wl: Some(wl),
            viewport,
            _fractional: fractional,
            scale120: 0,
            int_scale: 1,
            w: 0.0,
            h: 0.0,
            configured: false,
            dirty: true,
            frame_pending: false,
            wants_frames: false,
            anim: Animator::new(),
            pieces: Vec::new(),
            hovered: None,
            pressed: None,
            pointer: None,
            target: None,
            focus: None,
            visible: None,
            last: None,
            buffers: Vec::new(),
            own_pool: None,
            shm: Some(self.shm.wl_shm().clone()),
        }
    }

    fn wanted(&self, name: &str) -> bool {
        self.config.outputs.is_empty() || self.config.outputs.iter().any(|o| o == name)
    }

    fn place_layer(&self, layer: &LayerSurface) {
        let t = &self.theme;
        let edge = if t.on_top { Anchor::TOP } else { Anchor::BOTTOM };
        layer.set_anchor(edge | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, t.surface_height() as u32);
        layer.set_exclusive_zone(t.reserve() as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    }

    fn add_bar(&mut self, output: wl_output::WlOutput) {
        let Some(info) = self.outputs.info(&output) else { return };
        let name = info.name.clone().unwrap_or_default();
        if !self.wanted(&name) || self.bars.iter().any(|b| b.output == output) {
            return;
        }
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        surf.int_scale = info.scale_factor;
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Top, Some("asterism-bar"), Some(&output));
        self.place_layer(&layer);
        layer.commit();
        log::info!("bar on {name}");
        let output_height = info.logical_size.map_or(1080.0, |(_, h)| h as f32);
        let output_width = info.logical_size.map_or(0.0, |(w, _)| w as f32);
        self.bars.push(BarSurface { surf, layer, output, output_name: name, output_height, output_width });
    }

    fn set_input_region(&self, bar: &BarSurface) {
        // Only the bar's own strip takes the pointer: the halos' room below lets clicks through
        let t = &self.theme;
        if let Ok(region) = Region::new(&self.compositor) {
            let top = if t.on_top { 0 } else { t.glow_room as i32 };
            region.add(0, top, bar.surf.w as i32, t.reserve() as i32);
            if let Some(wl) = &bar.surf.wl {
                wl.set_input_region(Some(region.wl_region()));
            }
        }
    }

    pub fn update(&mut self, update: Update) {
        match update {
            Update::Toast(id) => return self.toast(id),
            Update::Request(request) => return self.request(request),
            Update::WallpaperPalette(image, palette) => return self.wallpaper_palette(image, palette),
            Update::Notify(note) => return self.send(Command::Notify(note)),
            Update::LockNow => return self.start_lock(),
            Update::WifiJoined(ssid, ok, why) => return self.wifi_joined(ssid, ok, why),
            Update::Pairing(pairing) => return self.pairing(pairing),
            Update::Spectrum(levels) => {
                // Thirty a second: only the desktop shows them, the bar needn't hear of it
                self.model.spectrum = levels;
                for desk in &mut self.desks {
                    desk.dirty = true;
                }
                if let Some(index) = self.panel_index(PanelKind::Song) {
                    self.panels[index].surf.dirty = true;
                }
                return;
            }
            _ => {}
        }
        self.notice(&update);
        let notifications = matches!(update, Update::Notifications(_));
        let auth = matches!(update, Update::Auth(_));
        let icons = matches!(update, Update::AppIcon(..));
        let dock = matches!(update, Update::Compositor(_) | Update::AppIcon(..) | Update::Trash(_) | Update::Apps(_));
        let thumbnails = matches!(update, Update::Thumbnail(..));
        let media = matches!(update, Update::Media(_));
        // With the picker closed, a thumbnail is the photo frame's: only the newest is kept
        if let Update::Thumbnail(path, _) = &update
            && self.picker.is_none()
        {
            let path = path.clone();
            self.model.thumbnails.retain(|p, _| *p == path);
            self.requested.retain(|key| !key.starts_with("thumb:") || key.strip_prefix("thumb:").is_some_and(|k| std::path::Path::new(k) == path));
        }
        // What the desktop's widgets show: other news leaves them be
        let widgets = match &update {
            Update::Machine(_) => self.config.widgets.show.iter().any(|k| k == "system"),
            Update::Media(_) | Update::Weather(_) | Update::Art(..) | Update::Lyrics(..) | Update::Photos(..) | Update::Thumbnail(..) => true,
            _ => false,
        };
        self.model.apply(update);
        if media {
            self.notice_song();
            self.watch_spectrum();
        }
        if let Some(index) = self.panel_index(PanelKind::Song) {
            self.panels[index].surf.dirty = true;
        }
        if notifications {
            self.sync_toasts();
        }
        if auth {
            self.sync_auth();
        }
        for bar in &mut self.bars {
            bar.surf.dirty = true;
        }
        if let Some(popup) = self.popup.as_mut() {
            popup.surf.dirty = true;
        }
        if let Some(peek) = self.peek.as_mut() {
            peek.surf.dirty = true;
        }
        if icons && let Some(index) = self.panel_index(PanelKind::Launcher) {
            self.panels[index].surf.dirty = true;
        }
        if thumbnails && let Some(index) = self.panel_index(PanelKind::Wallpapers) {
            self.panels[index].surf.dirty = true;
        }
        if let Some(index) = self.panel_index(PanelKind::Control) {
            self.panels[index].surf.dirty = true;
        }
        self.redraw_day();
        if dock {
            self.redraw_dock();
        }
        if let Some(index) = self.panel_index(PanelKind::Dashboard) {
            self.panels[index].surf.dirty = true;
        }
        if widgets {
            for desk in &mut self.desks {
                desk.dirty = true;
            }
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// The scheme's files for the apps that want them (written only where they changed).
    fn send_app_colors(&self) {
        if !self.config.colors.apps.is_empty() {
            self.send(Command::AppColors(Box::new(self.theme.palette), self.config.colors.apps.clone()));
        }
    }

    fn send_settings(&self) {
        self.send(Command::WeatherSettings(self.config.weather.clone()));
        self.send(Command::LockBeforeSleep(self.config.lock.before_sleep));
        self.send_notification_rules();
    }

    fn send_notification_rules(&self) {
        let rules = self.config.notifications.apps.iter().map(|(app, rule)| (app.to_lowercase(), *rule)).collect();
        self.send(Command::NotificationRules(rules));
    }

    /// Once a second: the clock, a playing track's progress, and the config file.
    fn tick(&mut self) {
        self.expire_toasts();
        self.rotate_wallpaper();
        self.tick_timers();
        self.save_day(false);
        self.apply_night_light();
        self.watch_idle();
        if self.model.recording.is_some() {
            for bar in &mut self.bars {
                bar.surf.dirty = true;
            }
        }
        // The desktop's clock, and the song's progress and words, move on
        let playing = self.model.media.player().is_some_and(|p| p.playing);
        for desk in &mut self.desks {
            if playing || desk.parts.is_empty() {
                desk.dirty = true;
            }
        }
        self.watch_clipboard();
        self.watch_spectrum();
        // The pet, when it's wanted and not up yet (or its idle watch couldn't be made before
        // the seat was known)
        if self.config.dock.enabled && self.panel_index(PanelKind::Dock).is_none() {
            self.sync_dock();
        }
        let wanted = [&self.config.pet.animal, &self.config.pet.friend].iter().filter(|a| crate::pet::Species::from_name(a).is_some()).count();
        if wanted > 0 && (self.pets.len() < wanted || self.pet_idle_watch.is_none()) {
            self.sync_pet();
        }
        let now = chrono::Local::now();
        let minute = now.format(if self.config.clock.seconds { "%H:%M:%S" } else { "%H:%M" }).to_string();
        if minute != self.last_minute {
            self.last_minute = minute;
            for bar in &mut self.bars {
                bar.surf.dirty = true;
            }
            for desk in &mut self.desks {
                desk.dirty = true;
            }
            if let Some(index) = self.panel_index(PanelKind::Control) {
                self.panels[index].surf.dirty = true;
            }
        }
        if let Some(popup) = self.popup.as_mut()
            && (popup.kind == Kind::Media && self.model.media.player().is_some_and(|p| p.playing) || popup.kind == Kind::Calendar)
        {
            popup.surf.dirty = true;
        }
        if now.timestamp() % 2 == 0 {
            let fresh = stamp(&self.config_path);
            if fresh != self.config_stamp {
                self.config_stamp = fresh;
                self.reload();
            }
        }
    }

    fn reload(&mut self) {
        log::info!("config changed, reading it again");
        self.config = Config::load(&self.config_path);
        self.send_settings();
        self.theme = Theme::new(&self.config);
        self.tools = tools_for(&self.config, self.idle_inhibit.is_some());
        // The same fonts, scanned once: only the family and the icons may change
        self.painter.configure(&self.config.font, &self.config.icons);
        let outputs: Vec<_> = self.outputs.outputs().collect();
        // Bars on outputs no longer wanted go, new ones come
        let keep: Vec<bool> = self.bars.iter().map(|b| self.wanted(&b.output_name)).collect();
        let mut index = 0;
        self.bars.retain(|_| {
            index += 1;
            keep[index - 1]
        });
        for output in outputs {
            self.add_bar(output);
        }
        self.keep_awake_somewhere();
        self.sync_backdrops();
        self.sync_desks();
        self.sync_edges();
        for bar in &self.bars {
            self.place_layer(&bar.layer);
            self.set_input_region(bar);
            bar.layer.commit();
        }
        // Colours and glow live in the theme, not in the pieces: paint everything again
        for bar in &mut self.bars {
            bar.surf.dirty = true;
            bar.surf.last = None;
        }
        self.close_menu();
        self.send_app_colors();
        self.sync_pet();
        self.sync_dock();
    }

    // ---------------------------------------------------------------- menus

    fn close_menu(&mut self) {
        if let Some(popup) = self.popup.take() {
            let parent = popup.parent;
            drop(popup);
            self.requested.remove("scan");
            self.requested.remove("btscan");
            if let Some(bar) = self.bars.iter_mut().find(|b| b.surf.id == parent) {
                bar.surf.dirty = true;
                // Back to never taking the keyboard: focus returns to the window
                bar.layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                bar.layer.commit();
            }
        }
    }

    /// How tall a menu may be under this bar before it scrolls.
    fn menu_room(&self, bar_id: u32) -> f32 {
        let height = self.bars.iter().find(|b| b.surf.id == bar_id).map_or(1080.0, |b| b.output_height);
        (height - self.theme.reserve() - 8.0 - 24.0).max(160.0)
    }

    fn positioner(&self, w: f32, h: f32, anchor: (i32, i32, i32, i32)) -> Option<XdgPositioner> {
        let positioner = XdgPositioner::new(&self.xdg).ok()?;
        positioner.set_size(w.ceil() as i32, h.ceil() as i32);
        positioner.set_anchor_rect(anchor.0, anchor.1, anchor.2.max(1), anchor.3.max(1));
        let below = self.theme.on_top;
        positioner.set_anchor(if below { xdg_positioner::Anchor::Bottom } else { xdg_positioner::Anchor::Top });
        positioner.set_gravity(if below { xdg_positioner::Gravity::Bottom } else { xdg_positioner::Gravity::Top });
        let gap = if self.theme.framed() { 0 } else { 8 };
        positioner.set_offset(0, if below { gap } else { -gap });
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::SlideX | xdg_positioner::ConstraintAdjustment::SlideY | xdg_positioner::ConstraintAdjustment::FlipY,
        );
        Some(positioner)
    }

    fn open_menu(&mut self, kind: Kind, bar_id: u32, piece: (f32, f32), serial: Option<u32>) {
        if self.popup.as_ref().is_some_and(|p| p.kind == kind) {
            self.close_menu();
            return;
        }
        self.close_menu();
        let Some(bar_index) = self.bars.iter().position(|b| b.surf.id == bar_id) else { return };
        let t = &self.theme;
        // Below the bar's band, centred on what was clicked
        let anchor = (piece.0 as i32, t.line() as i32, piece.1.max(1.0) as i32, t.bar_height as i32);
        let mut state = popups::State::default();
        let mut built = self.build_menu(&kind, &mut state);
        let content_h = built.fit(self.menu_room(bar_id));
        let (w, h) = (built.w, built.h);
        let Some(positioner) = self.positioner(w, h, anchor) else { return };
        let wl = self.compositor.create_surface(&self.qh);
        let surf = self.new_surf(wl.clone());
        let Ok(popup) = Popup::from_surface(None, &positioner, &self.qh, wl, &self.xdg) else {
            log::warn!("can't open a menu");
            return;
        };
        let m = POPUP_MARGIN as i32;
        popup.xdg_surface().set_window_geometry(m, m, w.ceil() as i32, h.ceil() as i32);
        // While a menu is open the bar may take the keyboard: the menu gets it through its grab
        // (and some compositors only honour a grab from a surface that can take the keyboard)
        // A click grabs: the menu gets the keyboard and closes when you click elsewhere. A keybind
        // has no click to grab with, so the bar takes the keyboard instead (Esc closes)
        let interactivity = if serial.is_some() { KeyboardInteractivity::OnDemand } else { KeyboardInteractivity::Exclusive };
        self.bars[bar_index].layer.set_keyboard_interactivity(interactivity);
        self.bars[bar_index].layer.commit();
        self.bars[bar_index].layer.get_popup(popup.xdg_popup());
        if let (Some(seat), Some(serial)) = (&self.seat, serial) {
            popup.xdg_popup().grab(seat, serial);
        }
        popup.wl_surface().commit();
        let mut surf = surf;
        surf.w = w + POPUP_MARGIN * 2.0;
        surf.h = h + POPUP_MARGIN * 2.0;
        surf.scale120 = self.bars[bar_index].surf.scale120;
        surf.int_scale = self.bars[bar_index].surf.int_scale;
        surf.pieces = built.pieces;
        if let Kind::Tray(_) = kind {
            state.tray_path.clear();
        }
        self.popup = Some(PopupSurface {
            surf,
            popup,
            kind,
            parent: bar_id,
            anchor,
            state,
            scroll: 0.0,
            content_h,
            view_h: h,
            focusables: Vec::new(),
        });
        self.bars[bar_index].surf.dirty = true;
    }

    fn build_menu(&mut self, kind: &Kind, state: &mut popups::State) -> popups::Built {
        let mut anim = Animator::new();
        anim.begin();
        self.build_menu_with(kind, state, &mut anim)
    }

    fn build_menu_with(&mut self, kind: &Kind, state: &mut popups::State, anim: &mut Animator) -> popups::Built {
        let mut ctx = popups::Ctx {
            model: &self.model,
            config: &self.config,
            theme: &self.theme,
            text: &mut self.painter.text,
            anim,
            state,
            now: chrono::Local::now(),
            requests: Vec::new(),
            requested: &self.requested,
        };
        let built = popups::build(kind, &mut ctx);
        let requests = std::mem::take(&mut ctx.requests);
        for request in requests {
            match &request {
                Command::FetchArt(url) => {
                    self.requested.insert(url.clone());
                }
                Command::ScanWifi => {
                    self.requested.insert("scan".into());
                }
                Command::ScanBluetooth => {
                    self.requested.insert("btscan".into());
                }
                _ => {}
            }
            self.send(request);
        }
        built
    }

    // ---------------------------------------------------------------- actions

    fn act(&mut self, action: Action, origin: Option<(u32, (f32, f32))>, serial: u32) {
        match action {
            Action::DockClick(_) | Action::DockLaunch(_) | Action::DockMenu(_) | Action::DockPin(..) | Action::DockClose(_) | Action::DockTrash | Action::DockTrashArm | Action::DockTrashEmpty => self.dock_act(action),
            Action::Send(Command::ToggleAwake) if self.idle_inhibit.is_some() => self.toggle_awake(),
            Action::Send(command) => self.send(command),
            Action::Menu(kind) => {
                if let Some((bar, piece)) = origin {
                    self.open_menu(kind, bar, piece, Some(serial));
                }
            }
            Action::CloseMenu => {
                self.close_menu();
                self.close_panel(PanelKind::Control);
                self.close_panel(PanelKind::DeskMenu);
                if self.panel_index(PanelKind::Day).is_some() {
                    self.close_day();
                }
            }
            Action::Month(month) => {
                if let Some(popup) = self.popup.as_mut() {
                    popup.state.month = month;
                    popup.surf.dirty = true;
                } else if self.panel_index(PanelKind::Day).is_some() {
                    self.day_state.month = month;
                    self.redraw_day();
                }
            }
            Action::Submenu(path) => {
                if let Some(popup) = self.popup.as_mut() {
                    popup.state.tray_path = path;
                    popup.surf.dirty = true;
                }
            }
            Action::Arm(key) => {
                if let Some(popup) = self.popup.as_mut() {
                    popup.state.armed = Some(key);
                    popup.surf.dirty = true;
                } else if let Some(index) = self.panel_index(PanelKind::Control) {
                    self.control_state.armed = Some(key);
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::ToggleControl => self.toggle_control(None),
            Action::ToggleDay => self.toggle_day(None),
            Action::DayFocus(field) => self.day_focus(field),
            Action::DayToggle(index) => self.day_toggle(index),
            Action::DayRemove(index) => self.day_remove(index),
            Action::DayClearDone => self.day_clear_done(),
            Action::AuthSubmit => self.auth_submit(),
            Action::AuthCancel => self.auth_cancel(),
            Action::Launch(index) => self.launch(index),
            Action::Move(by) => {
                self.launcher.step(by);
                if let Some(index) = self.panel_index(PanelKind::Launcher) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::ToggleLauncher => self.toggle_launcher(None),
            Action::ToggleWallpapers => self.toggle_wallpapers(None),
            Action::Timer(timer) => self.timer_action(timer),
            Action::ToggleNightLight => self.toggle_night_light(None),
            Action::ToggleClipboard => self.toggle_clipboard(None),
            Action::ToggleDashboard => self.toggle_dashboard(None),
            // A button for an app opens it; it doesn't close it again
            Action::ToggleSettings => self.toggle_settings(Some(true)),
            Action::CloseSettings => self.toggle_settings(Some(false)),
            Action::LockNow => self.start_lock(),
            Action::WifiAsk(ssid) => self.wifi_ask(ssid),
            Action::WifiSubmit => self.wifi_submit(),
            Action::WifiCancel => self.wifi_cancel(),
            Action::PairAnswer(answer) => self.pair_answer(answer),
            Action::ToggleOverview => self.toggle_overview(None),
            Action::OverviewWindow(_) | Action::OverviewWorkspace(_) | Action::OverviewClose(_) => self.overview_action(&action),
            Action::SetSetting(keys, value) => self.set_setting(&keys, &value),
            Action::MoveWindow => self.move_window(serial),
            Action::EditWidgets(on) => self.edit_widgets(on),
            Action::WidgetAdd(kind) => self.show_widget(&kind, true),
            Action::WidgetRemove(kind) => self.show_widget(&kind, false),
            Action::PetNext(slot) => self.pet_action(slot, true),
            Action::PetAway(slot) => self.pet_action(slot, false),
            Action::PetFeed => self.pet_feed(),
            Action::PetPlay => self.pet_play(),
            Action::PetYarn => self.pet_yarn(),
            Action::PetBed(put) => self.pet_bed(put),
            Action::NextPhoto => {
                self.photo_step += 1;
                for desk in &mut self.desks {
                    desk.dirty = true;
                }
            }
            Action::EditNote => {
                // The note's row in Settings, ready to type into
                self.toggle_settings(Some(true));
                if let Some(settings) = self.settings.as_mut() {
                    settings.page = 3;
                    settings.scroll = 0.0;
                    settings.editing = Some((vec!["widgets".into(), "note".into()], self.config.widgets.note.clone()));
                }
                if let Some(index) = self.panel_index(PanelKind::Settings) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::SettingsPage(page) => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.page = page;
                    settings.editing = None;
                    settings.scroll = 0.0;
                    settings.query.clear();
                }
                if let Some(index) = self.panel_index(PanelKind::Settings) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::SettingsEdit(keys) => {
                let current = crate::settings::text_value(&self.config, &keys);
                if let Some(settings) = self.settings.as_mut() {
                    settings.editing = Some((keys, current));
                }
                if let Some(index) = self.panel_index(PanelKind::Settings) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::DashSort(memory) => {
                if let Some(dashboard) = self.dashboard.as_mut() {
                    dashboard.sort = if memory { crate::dashboard::Sort::Memory } else { crate::dashboard::Sort::Cpu };
                    dashboard.armed = None;
                }
                if let Some(index) = self.panel_index(PanelKind::Dashboard) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::DashEnd(pid) => {
                if let Some(dashboard) = self.dashboard.as_mut() {
                    // The first click arms it, the second sends the end
                    if dashboard.armed == Some(pid) {
                        dashboard.armed = None;
                        self.send(Command::EndProcess(pid));
                    } else {
                        dashboard.armed = Some(pid);
                    }
                }
                if let Some(index) = self.panel_index(PanelKind::Dashboard) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::ClipCopy(_) | Action::ClipForget(_) | Action::ClipClear | Action::ClipMove(_) => self.clipboard_action(&action),
            Action::Wallpaper(index) => self.choose_wallpaper(index),
            Action::RandomWallpaper => self.step_wallpaper(0),
            Action::WallpaperMove(by) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.step(by);
                }
                if let Some(index) = self.panel_index(PanelKind::Wallpapers) {
                    self.panels[index].surf.dirty = true;
                }
            }
            Action::HideToast(id) => self.hide_toast(id),
            Action::Both(first, second) => {
                self.act(*first, origin, serial);
                self.act(*second, origin, serial);
            }
        }
    }

    /// Staying awake: an idle inhibitor on a bar, which is always on screen. The compositor
    /// then holds off idling whatever tool watches for it.
    fn toggle_awake(&mut self) {
        if let Some((inhibitor, _)) = self.inhibitor.take() {
            inhibitor.destroy();
            self.model.awake = false;
        } else {
            self.model.awake = true;
            self.keep_awake_somewhere();
        }
        for bar in &mut self.bars {
            bar.surf.dirty = true;
        }
    }

    /// Puts the inhibitor back on a bar that's still there, if staying awake.
    fn keep_awake_somewhere(&mut self) {
        if !self.model.awake {
            return;
        }
        if let Some((_, id)) = &self.inhibitor
            && self.bars.iter().any(|b| b.surf.id == *id)
        {
            return;
        }
        if let Some((old, _)) = self.inhibitor.take() {
            old.destroy();
        }
        let Some(manager) = &self.idle_inhibit else { return };
        if let Some(bar) = self.bars.first()
            && let Some(wl) = &bar.surf.wl
        {
            self.inhibitor = Some((manager.create_inhibitor(wl, &self.qh, Silent), bar.surf.id));
        }
    }

    // ---------------------------------------------------------------- keyboard

    /// A key pressed while a menu is open.
    fn key(&mut self, code: u32) {
        let Some(popup) = self.popup.as_mut() else { return };
        let in_submenu = !popup.state.tray_path.is_empty();
        match code {
            key::ESC | key::BACKSPACE if in_submenu => {
                popup.state.tray_path.pop();
                popup.surf.focus = None;
                popup.surf.dirty = true;
            }
            key::ESC => self.close_menu(),
            key::UP => self.move_focus(-1),
            key::DOWN => self.move_focus(1),
            key::TAB => self.move_focus(if self.shift { -1 } else { 1 }),
            key::HOME => self.move_focus(i32::MIN),
            key::END => self.move_focus(i32::MAX),
            key::PAGE_UP | key::PAGE_DOWN => {
                let page = popup.view_h * 0.8 * if code == key::PAGE_UP { -1.0 } else { 1.0 };
                if popup.max_scroll() > 0.0 {
                    popup.scroll_to(popup.scroll + page);
                } else {
                    self.move_focus(if code == key::PAGE_UP { i32::MIN } else { i32::MAX });
                }
            }
            key::LEFT | key::RIGHT => {
                let forward = code == key::RIGHT;
                let focused = popup.surf.focus.as_ref().and_then(|k| popup.focusables.iter().find(|f| &f.key == k));
                if let Some((slider, max)) = focused.and_then(|f| f.drag) {
                    self.nudge(slider, max, forward);
                } else if popup.kind == Kind::Calendar {
                    let month = popup.state.month + if forward { 1 } else { -1 };
                    self.act(Action::Month(month), None, 0);
                } else if let Some(Action::Submenu(path)) = focused.and_then(|f| f.left.clone()).filter(|_| forward) {
                    self.act(Action::Submenu(path), None, 0);
                    if let Some(popup) = self.popup.as_mut() {
                        popup.surf.focus = None;
                    }
                } else if !forward && in_submenu {
                    popup.state.tray_path.pop();
                    popup.surf.focus = None;
                    popup.surf.dirty = true;
                }
            }
            key::ENTER | key::KP_ENTER | key::SPACE => {
                let focused = popup.surf.focus.as_ref().and_then(|k| popup.focusables.iter().find(|f| &f.key == k));
                match focused.and_then(|f| f.left.clone()) {
                    Some(action) => {
                        let submenu = matches!(action, Action::Submenu(_));
                        self.act(action, None, 0);
                        if submenu && let Some(popup) = self.popup.as_mut() {
                            popup.surf.focus = None;
                        }
                    }
                    None if focused.is_none() => self.move_focus(1),
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Moves the keyboard to the next reachable piece (`i32::MIN` and `MAX` for the ends) and
    /// scrolls it into view.
    fn move_focus(&mut self, step: i32) {
        let Some(popup) = self.popup.as_mut() else { return };
        let count = popup.focusables.len() as i64;
        if count == 0 {
            return;
        }
        let current = popup.surf.focus.as_ref().and_then(|k| popup.focusables.iter().position(|f| &f.key == k));
        let next = match (current, step) {
            (_, i32::MIN) => 0,
            (_, i32::MAX) => count - 1,
            (None, s) if s < 0 => count - 1,
            (None, _) => 0,
            (Some(i), s) => (i as i64 + s as i64).clamp(0, count - 1),
        } as usize;
        let (y, h) = (popup.focusables[next].y, popup.focusables[next].h);
        popup.surf.focus = Some(popup.focusables[next].key.clone());
        popup.surf.dirty = true;
        popup.reveal(y, h);
    }

    /// Left and right on a slider: a twentieth of its range, five seconds for a track.
    fn nudge(&mut self, slider: Slider, max: f32, forward: bool) {
        let sign = if forward { 1.0 } else { -1.0 };
        let command = match slider {
            Slider::Volume => Some(Command::SetVolume((self.model.audio.volume + 0.05 * sign).clamp(0.0, max))),
            Slider::Brightness => Some(Command::SetBrightness((self.model.brightness.value + 0.05 * sign).clamp(0.01, max))),
            Slider::Stream(index) => self
                .model
                .audio
                .streams
                .iter()
                .find(|s| s.index == index)
                .map(|s| Command::SetStreamVolume(index, (s.volume + 0.05 * sign).clamp(0.0, max))),
            Slider::Seek => self
                .model
                .media
                .player()
                .map(|p| Command::MediaSeek(p.bus.clone(), (p.position_now() as f32 + 5_000_000.0 * sign).clamp(0.0, max) as i64)),
        };
        if let Some(command) = command {
            self.send(command);
        }
    }

    fn slide(&mut self, x: f32, done: bool) {
        let Some((on, drag)) = self.drag.as_mut() else { return };
        let fraction = ((x - drag.left) / drag.width).clamp(0.0, 1.0);
        drag.value = fraction * drag.max;
        let (on, key, value, slider) = (*on, drag.key.clone(), drag.value, drag.slider);
        let now = done || drag.sent.elapsed() >= Duration::from_millis(45);
        if now {
            drag.sent = Instant::now();
        }
        // The star follows the pointer right away, whatever the system says back
        let shown = (!done).then_some((key, value));
        match on {
            DragOn::Popup => {
                if let Some(popup) = self.popup.as_mut() {
                    popup.state.dragging = shown;
                    popup.surf.dirty = true;
                }
            }
            DragOn::Control => {
                self.control_state.dragging = shown;
                if let Some(index) = self.panel_index(PanelKind::Control) {
                    self.panels[index].surf.dirty = true;
                }
            }
        }
        if done {
            self.drag = None;
        }
        if !now {
            return;
        }
        let command = match slider {
            Slider::Volume => Some(Command::SetVolume(value)),
            Slider::Brightness => Some(Command::SetBrightness(value.max(0.01))),
            Slider::Stream(index) => Some(Command::SetStreamVolume(index, value)),
            Slider::Seek => self.model.media.player().map(|p| Command::MediaSeek(p.bus.clone(), value as i64)),
        };
        if let Some(command) = command {
            self.send(command);
        }
    }

    // ---------------------------------------------------------------- drawing

    pub fn flush(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.marquee_at).as_secs_f32();
        self.marquee_at = now;
        // The title slides on with time, once for every bar showing it
        if self.bars.iter().any(|b| b.surf.pieces.iter().any(|p| p.moving)) {
            let offset = self.marquee.entry("media".into()).or_insert(0.0);
            *offset += elapsed.min(0.1) * MARQUEE_SPEED;
        }
        let mut marquee_moving = false;
        for i in 0..self.bars.len() {
            if self.bars[i].surf.dirty && self.bars[i].surf.configured && !self.bars[i].surf.frame_pending {
                self.draw_bar(i);
            }
            marquee_moving |= self.bars[i].surf.pieces.iter().any(|p| p.moving);
        }
        if self.popup.as_ref().is_some_and(|p| p.surf.dirty && p.surf.configured && !p.surf.frame_pending) {
            self.draw_popup();
        }
        self.draw_peek();
        self.step_changes();
        self.release_backdrops();
        self.paint_backdrops();
        self.paint_edges();
        for index in 0..self.desks.len() {
            if self.desk_due(index) {
                self.draw_desk(index);
            }
        }
        for index in 0..self.panels.len() {
            let surf = &self.panels[index].surf;
            if surf.dirty && surf.configured && !surf.frame_pending {
                self.draw_panel(index);
            }
        }
        // Text sliding by doesn't need every frame: thirty a second is smooth at its pace
        if marquee_moving && !self.marquee_timer {
            self.marquee_timer = true;
            let _ = self.handle.insert_source(Timer::from_duration(Duration::from_millis(33)), |_, _, app| {
                app.marquee_timer = false;
                for bar in &mut app.bars {
                    if bar.surf.pieces.iter().any(|p| p.moving) {
                        bar.surf.dirty = true;
                    }
                }
                TimeoutAction::Drop
            });
        }
    }

    fn draw_bar(&mut self, index: usize) {
        let App { bars, painter, theme, model, config, tools, marquee, pool, qh, .. } = self;
        let bar = &mut bars[index];
        let start = Instant::now();
        let width = if bar.output_width > 0.0 { bar.output_width } else { bar.surf.w };
        let damage = render_bar(&mut bar.surf, painter, theme, model, config, tools, marquee, &bar.output_name, width);
        let drawn = start.elapsed();
        if let Some(damage) = damage {
            present(&mut bar.surf, damage, pool, qh);
        }
        log::trace!("bar frame: drawn in {drawn:?}, sent in {:?}, damage {damage:?}", start.elapsed() - drawn);
    }

    fn draw_popup(&mut self) {
        let Some(mut popup) = self.popup.take() else { return };
        popup.surf.dirty = false;
        popup.surf.anim.begin();
        let mut anim = std::mem::replace(&mut popup.surf.anim, Animator::new());
        let mut built = self.build_menu_with(&popup.kind, &mut popup.state, &mut anim);
        popup.surf.anim = anim;
        popup.content_h = built.fit(self.menu_room(popup.parent));
        popup.view_h = built.h;
        popup.scroll = popup.scroll.clamp(0.0, popup.max_scroll());
        popup.focusables = built
            .pieces
            .iter()
            .filter(|p| p.key != "card" && (p.actions.left.is_some() || p.actions.drag.is_some()))
            .map(|p| Focusable { key: p.key.clone(), y: p.y, h: p.h, left: p.actions.left.clone(), drag: p.actions.drag })
            .collect();
        if let Some(focus) = popup.surf.focus.clone()
            && !popup.focusables.iter().any(|f| f.key == focus)
        {
            popup.surf.focus = None;
        }
        let scroll = (popup.max_scroll() > 0.0).then_some(Scroll { offset: popup.scroll, content: popup.content_h, view: popup.view_h });
        let (w, h) = (built.w + POPUP_MARGIN * 2.0, built.h + POPUP_MARGIN * 2.0);
        // The content grew or shrank: the menu follows
        if (h - popup.surf.h).abs() > 0.5 || (w - popup.surf.w).abs() > 0.5 {
            if let Some(positioner) = self.positioner(built.w, built.h, popup.anchor) {
                let m = POPUP_MARGIN as i32;
                popup.popup.xdg_surface().set_window_geometry(m, m, built.w.ceil() as i32, built.h.ceil() as i32);
                popup.popup.reposition(&positioner, 1);
            }
            popup.surf.w = w;
            popup.surf.h = h;
        }
        if let Some(damage) = render_popup(&mut popup.surf, built.pieces, &mut self.painter, &self.theme, scroll) {
            present(&mut popup.surf, damage, &mut self.pool, &self.qh);
        }
        self.popup = Some(popup);
    }

    // ---------------------------------------------------------------- pointer

    fn set_cursor(&mut self, shape: Shape) {
        if self.cursor_shape == Some(shape) {
            return;
        }
        self.cursor_shape = Some(shape);
        if let Some(device) = &self.cursor {
            device.set_shape(self.enter_serial, shape);
        }
    }

    /// Which of the shell's surfaces this is.
    fn which(&self, wl: &wl_surface::WlSurface) -> Option<Which> {
        if self.popup.as_ref().is_some_and(|p| p.surf.is(wl)) {
            return Some(Which::Popup);
        }
        if let Some(i) = self.panels.iter().position(|p| p.surf.is(wl)) {
            return Some(Which::Panel(i));
        }
        self.bars.iter().position(|b| b.surf.is(wl)).map(Which::Bar)
    }

    fn surf_of(&mut self, which: Which) -> Option<&mut Surf> {
        match which {
            Which::Popup => self.popup.as_mut().map(|p| &mut p.surf),
            Which::Panel(i) => self.panels.get_mut(i).map(|p| &mut p.surf),
            Which::Bar(i) => self.bars.get_mut(i).map(|b| &mut b.surf),
        }
    }

    fn pointer_event(&mut self, event: &PointerEvent) {
        if let Some(index) = self.pets.iter().position(|p| p.surf.is(&event.surface)) {
            return self.pet_pointer_event(index, event);
        }
        if let Some(index) = self.props.iter().position(|p| p.surf.is(&event.surface)) {
            return self.prop_pointer_event(index, event);
        }
        if self.dock_pointer(event) {
            return;
        }
        // Where the pointer is, for the pet: on the desktop, its widgets or the bar
        if matches!(event.kind, PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. }) {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            let seen = if let Some(desk) = self.desks.iter().find(|d| d.has(&event.surface)) {
                desk.to_output(&event.surface, x, y).map(|(_, y)| (desk.output.clone(), y))
            } else if let Some(backdrop) = self.backdrops.iter().find(|b| b.surf.is(&event.surface)) {
                Some((backdrop.output.clone(), y))
            } else {
                self.bars.iter().find(|b| b.surf.is(&event.surface)).map(|b| (b.output.clone(), if self.theme.on_top { y } else { b.output_height - b.surf.h + y }))
            };
            if let Some((output, y)) = seen {
                self.saw_pointer(&output, x, y);
            }
        }
        if let Some(index) = self.desks.iter().position(|d| d.has(&event.surface)) {
            return self.desk_pointer(index, event);
        }
        if let Some(index) = self.backdrops.iter().position(|b| b.surf.is(&event.surface)) {
            return self.backdrop_pointer(index, event);
        }
        let (x, y) = (event.position.0 as f32, event.position.1 as f32);
        let Some(which) = self.which(&event.surface) else { return };
        let in_popup = which == Which::Popup;
        match event.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_shape = None;
                self.hover(which, Some((x, y)));
            }
            PointerEventKind::Leave { .. } => self.hover(which, None),
            PointerEventKind::Motion { .. } => {
                if self.overview.as_ref().is_some_and(|o| o.carry.is_some()) {
                    self.overview_motion(x, y);
                }
                self.hover(which, Some((x, y)));
                if self.drag.is_some() {
                    self.slide(x, false);
                }
            }
            PointerEventKind::Press { button, serial, .. } => self.press(which, x, y, button, serial),
            PointerEventKind::Release { .. } => {
                if self.drag.is_some() {
                    self.slide(x, true);
                }
                if self.overview.as_ref().is_some_and(|o| o.carry.is_some()) {
                    self.overview_drop(x, y);
                }
                if let Some(surf) = self.surf_of(which) {
                    surf.pressed = None;
                    surf.dirty = true;
                }
            }
            PointerEventKind::Axis { vertical, .. } => {
                // The settings window scrolls its page
                if let Which::Panel(i) = which
                    && self.panels.get(i).is_some_and(|p| p.kind == PanelKind::Settings)
                {
                    let pixels = if vertical.value120 != 0 {
                        vertical.value120 as f32 / 120.0 * 48.0
                    } else if vertical.discrete != 0 {
                        vertical.discrete as f32 * 48.0
                    } else {
                        vertical.absolute as f32
                    };
                    self.scroll_settings(pixels);
                    return;
                }
                // A long menu scrolls, unless what's under the pointer takes the wheel itself
                if in_popup
                    && let Some(popup) = self.popup.as_mut()
                    && popup.max_scroll() > 0.0
                    && !popup.surf.hit(x, y).is_some_and(|p| p.actions.up.is_some() || p.actions.down.is_some())
                {
                    let pixels = if vertical.value120 != 0 {
                        vertical.value120 as f32 / 120.0 * 48.0
                    } else if vertical.discrete != 0 {
                        vertical.discrete as f32 * 48.0
                    } else {
                        vertical.absolute as f32
                    };
                    popup.scroll_to(popup.scroll + pixels);
                    return;
                }
                let steps = self.wheel_steps(&vertical);
                if steps == 0 {
                    return;
                }
                let Some(piece) = self.surf_of(which).and_then(|s| s.hit(x, y)) else { return };
                let action = if steps < 0 { piece.actions.up.clone() } else { piece.actions.down.clone() };
                if let Some(action) = action {
                    for _ in 0..steps.unsigned_abs().min(3) {
                        self.act(action.clone(), None, 0);
                    }
                }
            }
        }
    }

    /// Wheels click in steps of 120; touchpads glide, so their pixels add up to steps.
    fn wheel_steps(&mut self, vertical: &smithay_client_toolkit::seat::pointer::AxisScroll) -> i32 {
        if vertical.value120 != 0 {
            self.scroll += vertical.value120 as f64 / 120.0;
        } else if vertical.discrete != 0 {
            return vertical.discrete;
        } else {
            self.scroll += vertical.absolute / 24.0;
        }
        let whole = self.scroll.trunc();
        self.scroll -= whole;
        whole as i32
    }

    fn hover(&mut self, which: Which, at: Option<(f32, f32)>) {
        if let Which::Panel(i) = which
            && self.panels.get(i).is_some_and(|p| p.kind == PanelKind::Toasts)
        {
            self.toasts_hovered = at.is_some();
        }
        let Some(surf) = self.surf_of(which) else { return };
        surf.pointer = at;
        let hit = at.and_then(|(x, y)| surf.hit(x, y));
        let key = hit.map(|p| p.key.clone());
        // Close to a star of a page: whose window that is
        let target = match (hit, at) {
            (Some(piece), Some((x, y))) if !piece.targets.is_empty() => piece
                .targets
                .iter()
                .map(|t| ((x - piece.x - t.x).hypot(y - piece.y - t.y), t))
                .filter(|(d, _)| *d < 18.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, t)| (piece.x + t.x, t.label.clone(), t.app.clone())),
            _ => None,
        };
        let clickable = hit.is_some_and(|p| !p.actions.is_empty() && !matches!(p.actions.left, Some(Action::MoveWindow)));
        let centre = hit.map_or(0.0, |p| p.x + p.w / 2.0);
        let page = key.clone();
        // A window's star: its app's icon goes beside its name, looked up once
        let wanted_icon = target.as_ref().map(|(_, _, app)| app.clone()).filter(|app| !app.is_empty());
        if key != surf.hovered || target != surf.target {
            if key.is_some() {
                surf.focus = None;
            }
            surf.hovered = key;
            surf.target = target;
            surf.dirty = true;
        }
        if at.is_some() {
            self.set_cursor(if clickable { Shape::Pointer } else { Shape::Default });
        }
        if let Some(app) = wanted_icon
            && !self.model.app_icons.contains_key(&app)
            && self.requested.insert(format!("icon:{app}"))
        {
            self.send(Command::AppIcon(app));
        }
        // On a bar, a page dot shows its workspace a moment later
        if let Which::Bar(bar) = which {
            self.peek_hover(bar, page.as_deref(), centre);
        }
    }

    fn press(&mut self, which: Which, x: f32, y: f32, button: u32, serial: u32) {
        let bar_id = match which {
            Which::Bar(i) => self.bars.get(i).map(|b| b.surf.id),
            _ => None,
        };
        let panel = match which {
            Which::Panel(i) => self.panels.get(i).map(|p| p.kind),
            _ => None,
        };
        let Some(surf) = self.surf_of(which) else { return };
        let Some(piece) = surf.hit(x, y).cloned() else {
            // Around the launcher or the control centre: a click there puts it away
            if let Some(kind @ (PanelKind::Launcher | PanelKind::Control | PanelKind::Wallpapers | PanelKind::Shortcuts | PanelKind::Clipboard | PanelKind::Dashboard | PanelKind::Overview | PanelKind::DeskMenu)) = panel {
                self.close_panel(kind);
            }
            if panel == Some(PanelKind::Day) {
                self.close_day();
            }
            return;
        };
        surf.pressed = Some(piece.key.clone());
        surf.dirty = true;
        let drag_on = match (which, panel) {
            (Which::Popup, _) => Some(DragOn::Popup),
            (_, Some(PanelKind::Control)) => Some(DragOn::Control),
            _ => None,
        };
        if let Some(on) = drag_on
            && button == BTN_LEFT
            && let Some((slider, max)) = piece.actions.drag
        {
            let drag = Drag { key: piece.key.clone(), slider, max, left: piece.x + 7.0, width: (piece.w - 14.0).max(1.0), sent: Instant::now() - Duration::from_secs(1), value: 0.0 };
            self.drag = Some((on, drag));
            self.slide(x, false);
            return;
        }
        // In the overview, a window is picked up, not clicked
        if panel == Some(PanelKind::Overview) && button == BTN_LEFT && self.overview_pick(&piece, x, y) {
            return;
        }
        let action = match button {
            BTN_LEFT => piece.actions.left.clone(),
            BTN_RIGHT => piece.actions.right.clone(),
            BTN_MIDDLE => piece.actions.middle.clone(),
            _ => None,
        };
        if let Some(action) = action {
            // A menu opened from the bar hangs under the piece that opened it
            let origin = bar_id.map(|id| (id, (piece.x, piece.w)));
            self.act(action, origin, serial);
        }
    }
}

/// One of the shell's surfaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Bar(usize),
    Popup,
    Panel(usize),
}

/// The bar for one output, drawn into a new canvas.
/// Returns the part of the canvas that changed, or nothing if nothing did.
#[allow(clippy::too_many_arguments)]
pub fn render_bar(surf: &mut Surf, painter: &mut Painter, t: &Theme, model: &Model, config: &Config, tools: &[Tool], marquee: &HashMap<String, f32>, output: &str, output_width: f32) -> Option<Damage> {
    surf.dirty = false;
    surf.anim.begin();
    let mut ctx = bar::Ctx {
        model,
        config,
        theme: t,
        tools,
        text: &mut painter.text,
        anim: &mut surf.anim,
        output,
        hovered: surf.hovered.as_deref(),
        now: chrono::Local::now(),
        width: surf.w,
        marquee,
        squeeze: 0.0,
    };
    let Layout { pieces, regions } = ctx.build();

    // What the bar stands on: the frame's top edge, a floating surface, pills, or nothing
    let mut all = Vec::with_capacity(pieces.len() + 4);
    let surface = |key: String, x: f32, y: f32, w: f32, h: f32, radius: f32| {
        let mut piece = Piece::new(key, w, h);
        piece.x = x;
        piece.y = y;
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius, color: t.surface });
        piece
    };
    match t.style {
        BarStyle::Line => {
            // A solid strip with a hairline along its inner edge
            let (y, h) = (t.line(), t.bar_height);
            let mut piece = Piece::new(format!("line.{}", surf.w), surf.w, h);
            piece.y = y;
            piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: surf.w, h, radius: 0.0, color: t.surface });
            let edge = if t.on_top { h - 1.0 } else { 0.0 };
            piece.prims.push(Prim::Rect { x: 0.0, y: edge, w: surf.w, h: 1.0, radius: 0.0, color: t.hairline() });
            all.push(piece);
        }
        BarStyle::Frame => {
            // The strip, and the frame's rounded inner corners under it at each end
            let r = t.frame_radius.min(t.glow_room);
            let strip = if t.on_top { (0.0, t.reserve()) } else { (t.line(), t.reserve()) };
            let mut piece = Piece::new(format!("frame.top.{}", surf.w), surf.w, t.surface_height());
            piece.prims.push(Prim::Rect { x: 0.0, y: strip.0, w: surf.w, h: strip.1, radius: 0.0, color: t.surface });
            let edge = if t.on_top { strip.1 } else { strip.0 };
            let dy = if t.on_top { 1.0 } else { -1.0 };
            let inset = (t.frame - (output_width - surf.w) / 2.0).max(0.0);
            piece.prims.push(Prim::Fillet { x: inset, y: edge, r, dx: 1.0, dy, color: t.surface });
            piece.prims.push(Prim::Fillet { x: surf.w - inset, y: edge, r, dx: -1.0, dy, color: t.surface });
            if inset > 0.0 {
                piece.prims.push(Prim::Rect { x: 0.0, y: if t.on_top { edge } else { edge - r }, w: inset, h: r, radius: 0.0, color: t.surface });
                piece.prims.push(Prim::Rect { x: surf.w - inset, y: if t.on_top { edge } else { edge - r }, w: inset, h: r, radius: 0.0, color: t.surface });
            }
            all.push(piece);
        }
        BarStyle::Islands => {
            let pad = (t.bar_height * 0.35).round();
            for (i, (x0, x1)) in regions.iter().enumerate() {
                all.push(surface(format!("island.{i}.{}", (x1 - x0).round()), x0 - pad, t.line(), x1 - x0 + pad * 2.0, t.bar_height, t.bar_height / 2.0));
            }
        }
        BarStyle::Floating => all.push(surface(format!("floating.{}", surf.w), 8.0, t.line(), surf.w - 16.0, t.bar_height, 14.0)),
        BarStyle::Transparent => {}
    }
    all.extend(pieces);

    // What the pointer is on gets its name written just off the bar
    let label = surf.target.as_ref().map(|(x, label, app)| (*x, label.clone(), model.app_icons.get(app).cloned().flatten())).or_else(|| {
        let hovered = surf.hovered.as_deref()?;
        let piece = all.iter().find(|p| p.key == hovered)?;
        Some((piece.x + piece.w / 2.0, piece.label.clone()?, None))
    });
    if let Some((cx, text, icon)) = label {
        let font = Font::new(t.text - 1.0);
        let (tw, th) = painter.text.measure(&text, font);
        let room = if icon.is_some() { 24.0 } else { 0.0 };
        // A small tooltip pill just off the bar
        let (w, h) = (tw + room + 20.0, th + 6.0);
        let mut piece = Piece::new("hover.label", w, h);
        piece.x = (cx - w / 2.0).clamp(t.frame + 4.0, (surf.w - w - t.frame - 4.0).max(4.0)).round();
        piece.y = if t.on_top { t.line() + t.bar_height + 4.0 } else { t.line() - h - 4.0 };
        piece.prims.push(Prim::Rect { x: 0.0, y: 0.0, w, h, radius: h / 2.0, color: t.container_highest });
        if let Some(image) = icon {
            piece.prims.push(Prim::Image { x: 10.0, y: (h - 16.0) / 2.0, w: 16.0, h: 16.0, image, tint: None, radius: 0.0 });
        }
        piece.prims.push(Prim::Text { x: 10.0 + room, y: 3.0, text, font, color: t.color_text });
        piece.opacity = surf.anim.value_from("hover.label", 0.0, 1.0, t.fast, Ease::OutCubic);
        all.push(piece);
    }

    // The bar settles in when it starts
    let seen = surf.anim.value_from("bar.seen", 0.0, 1.0, t.normal * 1.5, Ease::OutCubic);
    let lift_y = (1.0 - seen) * if t.on_top { -8.0 } else { 8.0 };
    let scale = surf.scale();
    let hovered = surf.hovered.clone();
    let pressed = surf.pressed.clone();
    // Under the pointer: a short accent underline that widens in, full width while pressed
    let stars: Vec<(f32, f32, f32)> = all
        .iter()
        .enumerate()
        .filter(|(_, p)| p.lift)
        .map(|(i, p)| {
            let target = if pressed.as_deref() == Some(p.key.as_str()) {
                1.6
            } else if hovered.as_deref() == Some(p.key.as_str()) {
                1.0
            } else {
                0.0
            };
            let alpha = surf.anim.value(&format!("{}.state", p.key), target, t.fast, Ease::OutCubic);
            (i as f32, 0.0, alpha)
        })
        .filter(|s| s.2 > 0.01)
        .collect();
    let lifts: Vec<f32> = all
        .iter()
        .map(|p| {
            if !p.lift {
                return 1.0;
            }
            let _ = (&pressed, &hovered);
            1.0
        })
        .collect();
    surf.anim.end();

    let (cw, ch) = ((surf.w * scale).ceil().max(1.0) as u32, (surf.h * scale).ceil().max(1.0) as u32);
    let entries: Vec<Entry> = all
        .iter()
        .zip(&lifts)
        .map(|(p, lift)| Entry {
            key: p.key.clone(),
            hash: p.content_hash(scale),
            bounds: {
                // Room for the hover underline under it
                let b = bounds(p, *lift, lift_y, scale);
                let grow = if p.lift { (11.0 * scale).ceil() as i32 } else { 0 };
                (b.0 - grow, b.1 - grow, b.2 + grow * 2, b.3 + grow * 2)
            },
            look: (lift.to_bits(), (p.opacity * seen).to_bits()),
        })
        .collect();
    let star_marks: Vec<(i32, i32, i32)> = stars.iter().map(|(x, y, a)| ((*x * 4.0) as i32, ((*y + lift_y) * 4.0) as i32, (*a * 255.0) as i32)).collect();
    surf.wants_frames = surf.anim.moving();

    // Only what changed gets painted again, when that's a small part of the bar
    let mut damage: Option<Damage> = None;
    let reuse = match &surf.last {
        Some(last)
            if last.canvas.width() == cw
                && last.canvas.height() == ch
                && last.stars == star_marks
                && last.entries.len() == entries.len()
                && last.entries.iter().zip(&entries).all(|(a, b)| a.key == b.key) =>
        {
            for (old, new) in last.entries.iter().zip(&entries) {
                if old != new {
                    damage = Some(union(damage, old.bounds));
                    damage = Some(union(damage, new.bounds));
                }
            }
            true
        }
        _ => false,
    };
    painter.forget(surf.id, &all);
    let full = (0, 0, cw as i32, ch as i32);
    let region = if reuse {
        match damage {
            // Nothing moved: nothing to send
            None => {
                surf.pieces = all;
                return None;
            }
            Some(d) if (d.2 as i64 * d.3 as i64) < (cw as i64 * ch as i64) * 2 / 5 => {
                let x0 = d.0.clamp(0, cw as i32);
                let y0 = d.1.clamp(0, ch as i32);
                let x1 = (d.0 + d.2).clamp(0, cw as i32);
                let y1 = (d.1 + d.3).clamp(0, ch as i32);
                (x0, y0, x1 - x0, y1 - y0)
            }
            Some(_) => full,
        }
    } else {
        full
    };
    if region.2 <= 0 || region.3 <= 0 {
        surf.pieces = all;
        return None;
    }

    let mut patch = Pixmap::new(region.2 as u32, region.3 as u32)?;
    let origin = (region.0 as f32, region.1 as f32);
    for (i, ((piece, lift), entry)) in all.iter().zip(&lifts).zip(&entries).enumerate() {
        if !overlaps(entry.bounds, region) {
            continue;
        }
        if let Some((_, _, amount)) = stars.iter().find(|s| s.0 as usize == i) {
            let prim = underline(t, piece, *amount, lift_y);
            painter.draw(&mut patch, t, &[prim], -origin.0 / scale, -origin.1 / scale, scale);
        }
        compose(&mut patch, painter, surf.id, piece, t, scale, *lift, seen, lift_y, origin);
    }
    let mut canvas = match surf.last.take() {
        Some(last) if region != full => last.canvas,
        _ => Pixmap::new(cw, ch)?,
    };
    // The patch replaces that part of the canvas
    let (row, stride) = (region.2 as usize * 4, cw as usize * 4);
    let (data, from) = (canvas.data_mut(), patch.data());
    for line in 0..region.3 as usize {
        let at = (region.1 as usize + line) * stride + region.0 as usize * 4;
        data[at..at + row].copy_from_slice(&from[line * row..(line + 1) * row]);
    }
    surf.last = Some(LastFrame { canvas, entries, stars: star_marks });
    surf.pieces = all;
    Some(region)
}

/// The mark under a bar item the pointer is on: it grows out from the middle as `amount` goes
/// 0 → 1, and spans the item at 1.6 (pressed). It sits on the bar's inner edge.
fn underline(t: &Theme, piece: &Piece, amount: f32, lift_y: f32) -> Prim {
    let span = (piece.w * (0.35 + 0.4 * amount.min(1.0)) + (amount - 1.0).max(0.0) * piece.w * 0.4).clamp(8.0, piece.w + 6.0);
    let edge = if t.on_top { t.line() + t.bar_height - 3.0 } else { t.line() + 1.0 };
    let y = edge.min(piece.y + piece.h + 8.0).max(piece.y - 10.0) + lift_y;
    Prim::Rect { x: piece.x + (piece.w - span) / 2.0, y, w: span, h: 2.0, radius: 1.0, color: t.primary.alpha(amount.min(1.0)) }
}

/// An open menu, drawn into a new canvas. Its pieces are placed on the card, the card
/// inside the room left for its halos. A menu taller than its card scrolls: the content slides
/// inside, fading where it's cut, with ETEA's thread-and-star scroll bar beside it.
pub fn render_popup(surf: &mut Surf, mut pieces: Vec<Piece>, painter: &mut Painter, t: &Theme, scroll: Option<Scroll>) -> Option<Damage> {
    let hovered = surf.hovered.clone();
    let focus = surf.focus.clone();
    let on = |key: &str| hovered.as_deref() == Some(key) || focus.as_deref() == Some(key);
    let offset = match scroll {
        Some(s) => surf.anim.value("popup.scroll", s.offset, t.fast, Ease::OutCubic),
        None => 0.0,
    };
    for piece in &mut pieces {
        piece.x += POPUP_MARGIN;
        piece.y += POPUP_MARGIN;
        if piece.key != "card" {
            piece.y -= offset;
        }
    }
    // Content stays clear of the card's ink edge
    surf.visible = scroll.map(|s| (POPUP_MARGIN + 9.0, POPUP_MARGIN + s.view - 9.0));
    let lifts: Vec<f32> = pieces
        .iter()
        .map(|p| {
            if !p.lift {
                return 1.0;
            }
            let target = 1.0;
            surf.anim.value(&format!("{}.lift", p.key), target, t.fast, Ease::Expressive)
        })
        .collect();
    let glows: Vec<f32> = pieces
        .iter()
        .map(|p| {
            // Rows light up; a reachable piece that isn't a row shows the keyboard with its star
            let lit = on(&p.key) && (p.highlight || p.lift || focus.as_deref() == Some(p.key.as_str()));
            surf.anim.value(&format!("{}.hl", p.key), if lit { 1.0 } else { 0.0 }, t.fast, Ease::OutCubic)
        })
        .collect();
    let opened = surf.anim.value_from("popup.open", 0.0, 1.0, t.normal, Ease::OutCubic);
    surf.anim.end();
    let scale = surf.scale();
    let (cw, ch) = ((surf.w * scale).ceil().max(1.0) as u32, (surf.h * scale).ceil().max(1.0) as u32);
    let mut canvas = Pixmap::new(cw, ch)?;
    // A drawer hangs still from the frame; a floating card drops in a little
    let rise = if t.framed() { 0.0 } else { (1.0 - opened) * if t.on_top { -6.0 } else { 6.0 } };
    // The card under everything; the content on a layer of its own when it has to be cut
    let mut content = if scroll.is_some() { Pixmap::new(cw, ch) } else { None };
    for ((piece, lift), glow) in pieces.iter().zip(&lifts).zip(&glows) {
        let layer = if piece.key == "card" { &mut canvas } else { content.as_mut().unwrap_or(&mut canvas) };
        if *glow > 0.01 {
            let prim = Prim::Rect { x: piece.x - 4.0, y: piece.y + rise, w: piece.w + 8.0, h: piece.h, radius: 10.0, color: t.state_layer(*glow) };
            painter.draw(layer, t, &[prim], 0.0, 0.0, scale);
        }
        compose(layer, painter, surf.id, piece, t, scale, *lift, opened, rise, (0.0, 0.0));
    }
    if let (Some(mut content), Some(s), Some((top, bottom))) = (content, scroll, surf.visible) {
        // Fade where the content is cut, only on a side with more to see
        let max = (s.content - s.view).max(0.0);
        let (fade_top, fade_bottom) = (offset.min(18.0), (max - offset).min(18.0));
        let row_bytes = cw as usize * 4;
        for (row, line) in content.data_mut().chunks_exact_mut(row_bytes).enumerate() {
            let y = (row as f32 + 0.5) / scale;
            let ramp = |distance: f32, fade: f32| if fade < 0.5 { if distance >= 0.0 { 1.0 } else { 0.0 } } else { (distance / fade).clamp(0.0, 1.0) };
            let alpha = ramp(y - top, fade_top).min(ramp(bottom - y, fade_bottom));
            if alpha >= 1.0 {
                continue;
            }
            let keep = (alpha * 256.0) as u32;
            for byte in line.iter_mut() {
                *byte = ((*byte as u32 * keep) >> 8) as u8;
            }
        }
        crate::render::blend_over(&mut canvas, &content, 0, 0, 1.0);

        // The scroll bar: a faint thread, a lit stroke for what shows, a star in its middle
        let x = surf.w - POPUP_MARGIN - 9.0;
        let (start, end) = (top + 8.0, bottom - 8.0);
        let track = (end - start).max(1.0);
        let thumb = (track * s.view / s.content).max(track * 0.08);
        let at = start + if max > 0.0 { offset / max } else { 0.0 } * (track - thumb);
        let prims = [
            Prim::Rect { x: x - 2.0, y: at + rise, w: 4.0, h: thumb, radius: 2.0, color: t.outline },
        ];
        painter.draw(&mut canvas, t, &prims, 0.0, 0.0, scale);
    }
    painter.forget(surf.id, &pieces);
    surf.wants_frames = surf.anim.moving();
    surf.pieces = pieces;
    let full = (0, 0, canvas.width() as i32, canvas.height() as i32);
    surf.last = Some(LastFrame { canvas, entries: Vec::new(), stars: Vec::new() });
    Some(full)
}

/// Puts a piece's picture on the canvas, grown around its middle by `lift`.
#[allow(clippy::too_many_arguments)]
fn compose(canvas: &mut Pixmap, painter: &mut Painter, surface: u32, piece: &Piece, theme: &Theme, scale: f32, lift: f32, opacity: f32, shift_y: f32, origin: (f32, f32)) {
    let Some((picture, room)) = painter.piece(surface, piece, theme, scale) else { return };
    let (cx, cy) = ((piece.x + piece.w / 2.0) * scale - origin.0, (piece.y + piece.h / 2.0 + shift_y) * scale - origin.1);
    let x = ((piece.x - room) * scale).round() - origin.0;
    let y = ((piece.y - room + shift_y) * scale).round() - origin.1;
    let opacity = (piece.opacity * opacity).clamp(0.0, 1.0);
    if (lift - 1.0).abs() < 0.001 {
        crate::render::blend_over(canvas, &picture, x as i32, y as i32, opacity);
        return;
    }
    let transform = Transform::from_translate(cx, cy).pre_scale(lift, lift).pre_translate(x - cx, y - cy);
    let paint = PixmapPaint {
        opacity,
        quality: tiny_skia::FilterQuality::Bilinear,
        ..Default::default()
    };
    canvas.draw_pixmap(0, 0, picture.as_ref().as_ref(), &paint, transform, None);
}

/// Swaps red and blue: tiny-skia is RGBA, wl_shm's ARGB8888 is BGRA in memory.
fn to_bgra(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
        let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let swapped = (v & 0xff00_ff00) | ((v & 0xff) << 16) | ((v >> 16) & 0xff);
        d.copy_from_slice(&swapped.to_le_bytes());
    }
}

/// A buffer this big or bigger comes from its surface's own pool, not the shared one: the shared
/// pool never shrinks, so one full-screen panel opened once would keep tens of megabytes for good.
const BIG_BUFFER: usize = 1 << 20;

/// Hands a finished canvas to the compositor. The previous buffer is used again when the
/// compositor is done with it: only what changed is copied in. Big surfaces draw from a pool of
/// their own (two buffers at most), which goes when they do.
fn present(surf: &mut Surf, damage: Damage, shared: &mut SlotPool, qh: &QueueHandle<App>) {
    let Some(canvas) = surf.last.as_ref().map(|l| &l.canvas) else { return };
    let bytes = canvas.width() as usize * canvas.height() as usize * 4;
    if bytes < BIG_BUFFER {
        surf.own_pool = None;
        return present_from(surf, damage, shared, qh, 3);
    }
    let own = surf.own_pool.take().or_else(|| {
        let wl_shm = surf.shm.clone()?;
        SlotPool::new(bytes * 2, &smithay_client_toolkit::shm::Shm::from(wl_shm)).ok()
    });
    match own {
        Some(mut pool) => {
            present_from(surf, damage, &mut pool, qh, 2);
            surf.own_pool = Some(pool);
        }
        None => present_from(surf, damage, shared, qh, 3),
    }
}

fn present_from(surf: &mut Surf, damage: Damage, pool: &mut SlotPool, qh: &QueueHandle<App>, keep: usize) {
    let Some(wl) = surf.wl.clone() else { return };
    let Some(canvas) = surf.last.as_ref().map(|l| &l.canvas) else { return };
    let (w, h) = (canvas.width() as i32, canvas.height() as i32);
    surf.buffers.retain(|(b, _)| b.height() == h && b.stride() == w * 4);
    for (_, stale) in &mut surf.buffers {
        *stale = Some(union(*stale, damage));
    }
    let stride = w as usize * 4;
    let mut chosen = None;
    for (index, (buffer, stale)) in surf.buffers.iter_mut().enumerate() {
        let Some(bytes) = buffer.canvas(pool) else { continue };
        if let Some(region) = stale.take() {
            let (x0, y0) = (region.0.clamp(0, w) as usize, region.1.clamp(0, h) as usize);
            let (x1, y1) = ((region.0 + region.2).clamp(0, w) as usize, (region.1 + region.3).clamp(0, h) as usize);
            for row in y0..y1 {
                let span = row * stride + x0 * 4..row * stride + x1 * 4;
                to_bgra(&mut bytes[span.clone()], &canvas.data()[span]);
            }
        }
        chosen = Some(index);
        break;
    }
    let chosen = match chosen {
        Some(index) => index,
        None => {
            // All still with the compositor: a new one, with the whole frame
            let Ok((buffer, bytes)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else {
                log::warn!("no buffer for a {w}×{h} surface");
                return;
            };
            to_bgra(bytes, canvas.data());
            if surf.buffers.len() >= keep {
                surf.buffers.remove(0);
            }
            surf.buffers.push((buffer, None));
            surf.buffers.len() - 1
        }
    };
    let buffer = &surf.buffers[chosen].0;
    if let Some(viewport) = &surf.viewport {
        wl.set_buffer_scale(1);
        viewport.set_destination(surf.w.round() as i32, surf.h.round() as i32);
    } else {
        wl.set_buffer_scale(surf.int_scale.max(1));
    }
    wl.damage_buffer(damage.0, damage.1, damage.2, damage.3);
    wl.frame(qh, FrameCallbackData(wl.clone()));
    surf.frame_pending = true;
    if buffer.attach_to(&wl).is_err() {
        return;
    }
    wl.commit();
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(surf) = self.surf_mut(surface)
            && surf.int_scale != factor
        {
            surf.int_scale = factor;
            surf.dirty = true;
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(surf) = self.surf_mut(surface) {
            surf.frame_pending = false;
            if surf.wants_frames {
                surf.dirty = true;
            }
        }
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_backdrop(output.clone());
        self.add_bar(output.clone());
        self.add_desk(output.clone());
        self.add_edges(output);
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let info = self.outputs.info(&output);
        let name = info.as_ref().and_then(|i| i.name.clone()).unwrap_or_default();
        let height = info.and_then(|i| i.logical_size).map(|(_, h)| h as f32);
        if let Some(bar) = self.bars.iter_mut().find(|b| b.output == output) {
            bar.output_name = name;
            if let Some(height) = height {
                bar.output_height = height;
            }
            bar.surf.dirty = true;
        } else {
            self.add_bar(output.clone());
        }
        self.add_backdrop(output);
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.backdrops.retain(|b| b.output != output);
        self.edges.retain(|e| e.output != output);
        self.gammas.retain(|g| g.output != output);
        self.desks.retain(|d| d.output != output);
        // Gone with its screen; it comes back on another at the next tick
        if self.pets.iter().any(|p| p.output == output) {
            self.pets.retain(|p| p.output != output);
            self.props.clear();
        }
        if let Some(index) = self.bars.iter().position(|b| b.output == output) {
            let id = self.bars[index].surf.id;
            if self.popup.as_ref().is_some_and(|p| p.parent == id) {
                self.close_menu();
            }
            self.bars.remove(index);
            self.keep_awake_somewhere();
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.pets.retain(|p| &p.layer != layer);
        if self.peek.as_ref().is_some_and(|p| &p.layer == layer) {
            self.peek = None;
        }
        self.props.retain(|p| &p.layer != layer);
        self.panels.retain(|p| !p.is_layer(layer));
        self.backdrops.retain(|b| &b.layer != layer);
        self.edges.retain(|e| &e.layer != layer);
        for desk in &mut self.desks {
            desk.parts.retain(|p| &p.layer != layer);
            if desk.dim.as_ref().is_some_and(|d| &d.layer == layer) {
                desk.dim = None;
            }
        }
        self.bars.retain(|b| &b.layer != layer);
        self.keep_awake_somewhere();
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if let Some(edge) = self.edges.iter_mut().find(|e| &e.layer == layer) {
            let (w, h) = configure.new_size;
            if w > 0 && h > 0 {
                edge.surf.w = w as f32;
                edge.surf.h = h as f32;
                edge.surf.configured = true;
                edge.surf.dirty = true;
            }
            return;
        }
        if let Some(index) = self.pets.iter().position(|p| &p.layer == layer) {
            return self.pet_configured(index);
        }
        if self.peek.as_ref().is_some_and(|p| &p.layer == layer) {
            return self.peek_configured();
        }
        if let Some(index) = self.props.iter().position(|p| &p.layer == layer) {
            return self.prop_configured(index);
        }
        for desk in &mut self.desks {
            let Some(part) = desk.part_mut(layer) else { continue };
            let (w, h) = configure.new_size;
            if w > 0 && h > 0 {
                part.surf.w = w as f32;
                part.surf.h = h as f32;
                part.surf.configured = true;
                part.surf.frame_pending = false;
                part.surf.last = None;
                part.surf.buffers.clear();
                desk.dirty = true;
            }
            return;
        }
        if let Some(backdrop) = self.backdrops.iter_mut().find(|b| &b.layer == layer) {
            let (w, h) = configure.new_size;
            if w > 0 && h > 0 {
                backdrop.surf.w = w as f32;
                backdrop.surf.h = h as f32;
                backdrop.surf.configured = true;
                backdrop.surf.dirty = true;
            }
            return;
        }
        if let Some(panel) = self.panels.iter_mut().find(|p| p.is_layer(layer)) {
            let (w, h) = configure.new_size;
            let surf = &mut panel.surf;
            surf.w = if w > 0 { w as f32 } else { panel.want.0 as f32 };
            surf.h = if h > 0 { h as f32 } else { panel.want.1 as f32 };
            surf.configured = true;
            surf.dirty = true;
            surf.frame_pending = false;
            // The dock's pointer region follows its size
            if panel.kind == PanelKind::Dock {
                self.dock_region();
            }
            return;
        }
        let Some(index) = self.bars.iter().position(|b| &b.layer == layer) else { return };
        let (w, h) = configure.new_size;
        let surf = &mut self.bars[index].surf;
        surf.w = if w > 0 { w as f32 } else { 1920.0 };
        surf.h = if h > 0 { h as f32 } else { self.theme.surface_height() };
        surf.configured = true;
        surf.dirty = true;
        surf.frame_pending = false;
        self.set_input_region(&self.bars[index]);
    }
}

impl PopupHandler for App {
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup, _: PopupConfigure) {
        if let Some(open) = self.popup.as_mut()
            && &open.popup == popup
        {
            open.surf.configured = true;
            open.surf.dirty = true;
            open.surf.frame_pending = false;
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        if self.popup.as_ref().is_some_and(|p| &p.popup == popup) {
            self.close_menu();
        }
    }
}

/// The settings app is a window of its own, like any app's.
impl smithay_client_toolkit::shell::xdg::window::WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &smithay_client_toolkit::shell::xdg::window::Window) {
        if let Some(kind) = self.panels.iter().find(|p| p.is_window(window)).map(|p| p.kind) {
            self.close_panel(kind);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &smithay_client_toolkit::shell::xdg::window::Window,
        configure: smithay_client_toolkit::shell::xdg::window::WindowConfigure,
        _: u32,
    ) {
        let Some(panel) = self.panels.iter_mut().find(|p| p.is_window(window)) else { return };
        // No size given: ours to choose, the size it had or the one it opened with
        let (w, h) = configure.new_size;
        let surf = &mut panel.surf;
        let w = w.map_or(if surf.configured { surf.w } else { panel.want.0 as f32 }, |w| w.get() as f32);
        let h = h.map_or(if surf.configured { surf.h } else { panel.want.1 as f32 }, |h| h.get() as f32);
        if (w - surf.w).abs() > 0.5 || (h - surf.h).abs() > 0.5 {
            surf.w = w;
            surf.h = h;
            surf.last = None;
        }
        surf.configured = true;
        surf.dirty = true;
        surf.frame_pending = false;
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.seat.get_or_insert(seat);
        self.watch_clipboard();
        self.watch_idle();
    }

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    self.cursor = self.cursor_shapes.as_ref().map(|m| m.get_shape_device(&pointer, qh));
                    // Without it (an older compositor), drags fall back to positions on surfaces
                    self.relative = self.relative_manager.get_relative_pointer(&pointer, qh).ok();
                    self.pointer = Some(pointer);
                }
                Err(error) => log::warn!("no pointer: {error}"),
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let repeat = Box::new(|app: &mut App, _: &wl_keyboard::WlKeyboard, event: KeyEvent| app.key_event(&event));
            match self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.handle.clone(), repeat) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(error) => log::warn!("no keyboard: {error}"),
            }
        }
        self.seat.get_or_insert(seat);
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
            && keyboard.version() >= 3
        {
            keyboard.release();
        }
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            self.cursor = None;
            pointer.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[smithay_client_toolkit::seat::keyboard::Keysym]) {
        self.keyboard_on = Some(surface.clone());
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32) {
        if self.keyboard_on.as_ref() == Some(surface) {
            self.keyboard_on = None;
        }
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key_event(&event);
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key_event(&event);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.shift = modifiers.shift;
        self.modifiers = modifiers;
    }
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            self.pointer_event(event);
        }
    }
}

/// The pointer's motion as such, apart from any surface: what drags follow.
impl smithay_client_toolkit::seat::relative_pointer::RelativePointerHandler for App {
    fn relative_pointer_motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &smithay_client_toolkit::reexports::protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::ZwpRelativePointerV1,
        _: &wl_pointer::WlPointer,
        event: smithay_client_toolkit::seat::relative_pointer::RelativeMotionEvent,
    ) {
        self.relative_motion(event.delta.0 as f32, event.delta.1 as f32);
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);

pub fn run(config_path: PathBuf) -> anyhow::Result<()> {
    let config = Config::load(&config_path);
    let theme = Theme::new(&config);
    let conn = Connection::connect_to_env().map_err(|e| anyhow::anyhow!("can't reach the Wayland compositor: {e}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn)?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<'static, App> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue).insert(handle.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| anyhow::anyhow!("this compositor has no layer shell (wlr-layer-shell)"))?;
    let xdg = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new(1920 * 80 * 4, &shm)?;
    let cursor_shapes = CursorShapeManager::bind(&globals, &qh).ok();
    let fractional = globals.bind::<WpFractionalScaleManagerV1, App, Silent>(&qh, 1..=1, Silent).ok();
    let viewporter = globals.bind::<WpViewporter, App, Silent>(&qh, 1..=1, Silent).ok();
    let idle_inhibit = globals.bind::<ZwpIdleInhibitManagerV1, App, Silent>(&qh, 1..=1, Silent).ok();
    if idle_inhibit.is_none() {
        log::info!("no idle-inhibit protocol here: staying awake goes through systemd-inhibit");
    }

    let (clip_sender, clip_reads) = calloop::channel::channel::<clipboard::ClipRead>();
    handle
        .insert_source(clip_reads, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(read) = event {
                app.clipboard_read(read);
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (painted, finished) = calloop::channel::channel::<Painted>();
    handle
        .insert_source(finished, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(picture) = event {
                app.painted(picture);
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (sender, receiver) = calloop::channel::channel();
    let roles = crate::services::Roles { notification_daemon: config.notifications.daemon, control_socket: true, polkit_agent: config.polkit_agent, lock: true, bluetooth_agent: true };
    let commands = crate::services::spawn(sender, crate::services::detect_compositor(), roles);
    handle
        .insert_source(receiver, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(update) = event {
                app.update(update);
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    handle
        .insert_source(Timer::immediate(), |_, _, app: &mut App| {
            app.tick();
            let millis = 1000 - chrono::Local::now().timestamp_subsec_millis().min(999) as u64;
            TimeoutAction::ToDuration(Duration::from_millis(millis + 5))
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut app = App {
        registry: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        xdg,
        shm,
        pool,
        cursor_shapes,
        fractional,
        viewporter,
        qh: qh.clone(),
        handle: handle.clone(),
        seat: None,
        pointer: None,
        cursor: None,
        relative_manager: smithay_client_toolkit::seat::relative_pointer::RelativePointerState::bind(&globals, &qh),
        relative: None,
        cursor_shape: None,
        enter_serial: 0,
        scroll: 0.0,
        keyboard: None,
        shift: false,
        modifiers: Default::default(),
        panels: Vec::new(),
        toasts: Vec::new(),
        toasts_hovered: false,
        osd: None,
        osd_generation: 0,
        launcher: crate::launcher::Launcher::new(),
        picker: None,
        sheet: None,
        dashboard: None,
        settings: None,
        wifi: None,
        pair: None,
        overview: None,
        desks: Vec::new(),
        widget_places: crate::widgets::load_places(),
        widget_drag: None,
        desk_edit: false,
        spectrum_on: false,
        keyboard_on: None,
        desk_menu: None,
        photo_step: 0,
        last_track: None,
        song_generation: 0,
        started: Instant::now(),
        pets: Vec::new(),
        peek: None,
        peek_wait: None,
        dock: dock::DockState::default(),
        bt_warned: HashSet::new(),
        pet_timer: false,
        pet_idle: false,
        pet_idle_watch: None,
        pet_pointer: None,
        props: Vec::new(),
        clip_manager: clipboard::bind_clipboard(&globals, &qh),
        clip_device: None,
        clip_source: None,
        clip_picker: None,
        clip_sender,
        idle_notifier: idle::bind_idle(&globals, &qh),
        idle_watch: None,
        gamma_manager: night::bind_gamma(&globals, &qh),
        gammas: Vec::new(),
        gamma_refused: false,
        night_forced: None,
        night_scheduled: None,
        wallpaper_changed: Instant::now(),
        drag: None,
        password: Default::default(),
        auth_cookie: None,
        control_state: popups::State::default(),
        day_state: popups::State::default(),
        day_dirty: None,
        inhibitor: None,
        bars: Vec::new(),
        backdrops: Vec::new(),
        edges: Vec::new(),
        painted,
        popup: None,
        next_id: 0,
        config_stamp: stamp(&config_path),
        config_path,
        tools: tools_for(&config, idle_inhibit.is_some()),
        idle_inhibit,
        painter: Painter::new(&config.font, &config.icons),
        theme,
        config,
        model: Model::default(),
        commands,
        requested: HashSet::new(),
        marquee: HashMap::new(),
        marquee_at: Instant::now(),
        marquee_timer: false,
        last_minute: String::new(),
        last_transition: None,
    };
    app.send_settings();
    app.send_app_colors();

    loop {
        event_loop.dispatch(None, &mut app)?;
        app.flush();
        let _ = conn.flush();
    }
}
