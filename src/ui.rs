//! The bar is rebuilt from the model whenever something changes, as a list of pieces: small
//! clickable units (a page, the clock, one status icon) made of drawing primitives. Pieces are
//! rendered once into their own little picture and kept until they look different.

use crate::model::{Command, Image};
use crate::render::text::Font;
use crate::theme::Rgba;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub enum Prim {
    /// A line of text, its box's top-left at (x, y).
    Text { x: f32, y: f32, text: String, font: Font, color: Rgba },
    /// Text sliding by inside `width`, twice in a row so it loops, its ends fading out.
    Marquee { x: f32, y: f32, width: f32, text: String, font: Font, color: Rgba, offset: f32, gap: f32 },
    /// One of the bar's own icons, tinted.
    Icon { x: f32, y: f32, size: f32, name: String, color: Rgba },
    /// An icon with its own glow, for pieces that don't glow as a whole.
    GlowIcon { x: f32, y: f32, size: f32, name: String, color: Rgba },
    /// A picture: a tray icon or an album cover. `tint` paints it in one colour.
    Image { x: f32, y: f32, w: f32, h: f32, image: Arc<Image>, tint: Option<Rgba>, radius: f32 },
    /// A ring filling up clockwise from the top.
    Ring { cx: f32, cy: f32, r: f32, width: f32, value: f32, color: Rgba },
    Line { points: Vec<(f32, f32)>, width: f32, color: Rgba },
    /// The space between a line and a level beneath it, filled: a graph's body.
    Area { points: Vec<(f32, f32)>, base: f32, color: Rgba },
    Rect { x: f32, y: f32, w: f32, h: f32, radius: f32, color: Rgba },
    /// A card: a rounded surface in the container colour, outlined with a hairline, on a soft
    /// shadow that reaches up to 12px below it.
    Card { x: f32, y: f32, w: f32, h: f32, radius: f32 },
    /// The inside of a rounded corner where two surfaces meet: a square of `r` with a quarter
    /// circle taken out. The solid corner sits at (x, y); `dx` and `dy` (±1) say which way the
    /// square reaches from it.
    Fillet { x: f32, y: f32, r: f32, dx: f32, dy: f32, color: Rgba },
    /// A surface hanging from one or more edges (bit 1 top, 2 right, 4 bottom, 8 left): its
    /// attached corners are square, and where it leaves an edge the edge curves into it.
    Drawer { x: f32, y: f32, w: f32, h: f32, radius: f32, attach: u8, color: Rgba },
}

fn f(h: &mut impl Hasher, value: f32) {
    ((value * 64.0).round() as i32).hash(h);
}

fn c(h: &mut impl Hasher, color: Rgba) {
    color.key().hash(h);
}

impl Prim {
    pub fn hash(&self, h: &mut impl Hasher) {
        std::mem::discriminant(self).hash(h);
        match self {
            Prim::Text { x, y, text, font, color } => {
                f(h, *x);
                f(h, *y);
                text.hash(h);
                font.key().hash(h);
                c(h, *color);
            }
            Prim::Marquee { x, y, width, text, font, color, offset, gap } => {
                for v in [*x, *y, *width, *offset, *gap] {
                    f(h, v);
                }
                text.hash(h);
                font.key().hash(h);
                c(h, *color);
            }
            Prim::Icon { x, y, size, name, color } | Prim::GlowIcon { x, y, size, name, color } => {
                for v in [*x, *y, *size] {
                    f(h, v);
                }
                name.hash(h);
                c(h, *color);
            }
            Prim::Image { x, y, w, h: height, image, tint, radius } => {
                for v in [*x, *y, *w, *height, *radius] {
                    f(h, v);
                }
                image.id.hash(h);
                tint.map(|t| t.key()).hash(h);
            }
            Prim::Ring { cx, cy, r, width, value, color } => {
                for v in [*cx, *cy, *r, *width, *value * 4.0] {
                    f(h, v);
                }
                c(h, *color);
            }
            Prim::Area { points, base, color } => {
                for (x, y) in points {
                    f(h, *x);
                    f(h, *y);
                }
                f(h, *base);
                c(h, *color);
            }
            Prim::Line { points, width, color } => {
                for (x, y) in points {
                    f(h, *x);
                    f(h, *y);
                }
                f(h, *width);
                c(h, *color);
            }
            Prim::Rect { x, y, w, h: height, radius, color } => {
                for v in [*x, *y, *w, *height, *radius] {
                    f(h, v);
                }
                c(h, *color);
            }
            Prim::Fillet { x, y, r, dx, dy, color } => {
                for v in [*x, *y, *r, *dx, *dy] {
                    f(h, v);
                }
                c(h, *color);
            }
            Prim::Drawer { x, y, w, h: height, radius, attach, color } => {
                for v in [*x, *y, *w, *height, *radius] {
                    f(h, v);
                }
                attach.hash(h);
                c(h, *color);
            }
            Prim::Card { x, y, w, h: height, radius } => {
                for v in [*x, *y, *w, *height, *radius] {
                    f(h, v);
                }
            }
        }
    }
}

/// What a click, a scroll or a drag does.
#[derive(Clone, Debug)]
pub enum Action {
    Send(Command),
    /// Opens this menu, or closes it if it's the one open.
    Menu(crate::popups::Kind),
    CloseMenu,
    /// Steps the calendar by months; 0 comes back to this month.
    Month(i32),
    /// Shows this submenu of a tray menu (the path of entry ids from the top).
    Submenu(Vec<i32>),
    /// The first click on something drastic arms it, the second one does it.
    Arm(String),
    /// Opens the launcher's result with this index.
    Launch(usize),
    /// Moves the launcher's choice up or down.
    Move(i32),
    ToggleLauncher,
    /// Opens or closes the control centre.
    ToggleControl,
    /// Sends the typed password to polkit.
    AuthSubmit,
    AuthCancel,
    /// Takes a notification's pop-up away (it stays in the centre).
    HideToast(u32),
    /// Several at once.
    Both(Box<Action>, Box<Action>),
    /// The wallpaper changer: open or close it, put up its picture with this index, move its
    /// choice, or put up one at random.
    ToggleWallpapers,
    /// Night light the other way.
    ToggleNightLight,
    /// The clipboard history: open or close it, copy an entry again, forget one or all, move.
    ToggleClipboard,
    /// The dashboard: open or close it, sort its processes (by memory, or not), end one.
    ToggleDashboard,
    /// The settings app: open or close it, show a page, start typing a value, write one.
    ToggleSettings,
    CloseSettings,
    SettingsPage(usize),
    SettingsEdit(Vec<String>),
    SetSetting(Vec<String>, crate::settings::Value),
    /// The window this piece is in follows the pointer (a window's title area).
    MoveWindow,
    /// Arranging the desktop's widgets: start or finish (None: the other way), put one on the
    /// desktop, take one off.
    EditWidgets(Option<bool>),
    WidgetAdd(String),
    WidgetRemove(String),
    /// The sticky note, to be written (in Settings); the photo frame's next picture.
    EditNote,
    NextPhoto,
    LockNow,
    /// The wifi password prompt: ask for this network's password, send it, or let it be.
    WifiAsk(String),
    WifiSubmit,
    WifiCancel,
    /// The overview: open or close it, go to a window or a workspace, close a window.
    ToggleOverview,
    OverviewWindow(u64),
    OverviewWorkspace(i64),
    OverviewClose(u64),
    /// The answer to a Bluetooth pairing question (None: no).
    PairAnswer(Option<String>),
    DashSort(bool),
    DashEnd(u32),
    ClipCopy(u64),
    ClipForget(u64),
    ClipClear,
    ClipMove(i32),
    /// Something done to a timer.
    Timer(crate::model::TimerAction),
    Wallpaper(usize),
    WallpaperMove(i32),
    RandomWallpaper,
}

#[derive(Clone, Copy, Debug)]
pub enum Slider {
    Volume,
    Brightness,
    Stream(u32),
    /// Seeks the player, `max` is the track length in microseconds.
    Seek,
}

#[derive(Clone, Default, Debug)]
pub struct Actions {
    pub left: Option<Action>,
    pub right: Option<Action>,
    pub middle: Option<Action>,
    pub up: Option<Action>,
    pub down: Option<Action>,
    /// Dragging along x sends this slider's value, `max` at the right end.
    pub drag: Option<(Slider, f32)>,
}

impl Actions {
    pub fn is_empty(&self) -> bool {
        self.left.is_none() && self.right.is_none() && self.middle.is_none() && self.up.is_none() && self.down.is_none() && self.drag.is_none()
    }
}

/// A point that shows a label when the pointer comes close: the stars of a page.
#[derive(Clone)]
pub struct Target {
    pub x: f32,
    pub y: f32,
    pub label: String,
    pub app: String,
}

#[derive(Clone)]
pub struct Piece {
    pub key: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Room around it for what spills out (halos, glow).
    pub pad: f32,
    pub prims: Vec<Prim>,
    /// Lifted off the sky by a soft glow in the planet's light.
    pub glow: bool,
    pub opacity: f32,
    /// Grows a touch under the pointer, a small star lights up beside it.
    pub lift: bool,
    /// Shown just off the bar while the pointer is on it.
    pub label: Option<String>,
    pub targets: Vec<Target>,
    pub actions: Actions,
    /// Needs a new frame even without changes (text sliding by).
    pub moving: bool,
    /// Lights up under the pointer (popup rows).
    pub highlight: bool,
    /// Catches the pointer even with nothing to do, so clicks don't fall through it.
    pub blocks: bool,
}

impl Piece {
    pub fn new(key: impl Into<String>, w: f32, h: f32) -> Self {
        Piece {
            key: key.into(),
            x: 0.0,
            y: 0.0,
            w,
            h,
            pad: 0.0,
            prims: Vec::new(),
            glow: false,
            opacity: 1.0,
            lift: false,
            label: None,
            targets: Vec::new(),
            actions: Actions::default(),
            moving: false,
            highlight: false,
            blocks: false,
        }
    }

    pub fn content_hash(&self, scale: f32) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for prim in &self.prims {
            prim.hash(&mut h);
        }
        f(&mut h, self.w);
        f(&mut h, self.h);
        f(&mut h, self.pad);
        f(&mut h, scale);
        self.glow.hash(&mut h);
        h.finish()
    }

    pub fn contains(&self, x: f32, y: f32, margin: f32) -> bool {
        x >= self.x - margin && x < self.x + self.w + margin && y >= self.y - margin && y < self.y + self.h + margin
    }

    /// The same piece drawn `k` times as big, text and all (a widget made bigger or smaller).
    pub fn scale(&mut self, k: f32) {
        if (k - 1.0).abs() < 0.001 {
            return;
        }
        self.w *= k;
        self.h *= k;
        self.pad *= k;
        for prim in &mut self.prims {
            match prim {
                Prim::Text { x, y, font, .. } => {
                    *x *= k;
                    *y *= k;
                    font.size *= k;
                    font.spacing *= k;
                }
                Prim::Marquee { x, y, width, font, offset, gap, .. } => {
                    for v in [x, y, width, offset, gap] {
                        *v *= k;
                    }
                    font.size *= k;
                    font.spacing *= k;
                }
                Prim::Icon { x, y, size, .. } | Prim::GlowIcon { x, y, size, .. } => {
                    for v in [x, y, size] {
                        *v *= k;
                    }
                }
                Prim::Image { x, y, w, h, radius, .. } => {
                    for v in [x, y, w, h, radius] {
                        *v *= k;
                    }
                }
                Prim::Ring { cx, cy, r, width, .. } => {
                    for v in [cx, cy, r, width] {
                        *v *= k;
                    }
                }
                Prim::Line { points, width, .. } => {
                    for (x, y) in points.iter_mut() {
                        *x *= k;
                        *y *= k;
                    }
                    *width *= k;
                }
                Prim::Area { points, base, .. } => {
                    for (x, y) in points.iter_mut() {
                        *x *= k;
                        *y *= k;
                    }
                    *base *= k;
                }
                Prim::Rect { x, y, w, h, radius, .. } | Prim::Card { x, y, w, h, radius } | Prim::Drawer { x, y, w, h, radius, .. } => {
                    for v in [x, y, w, h, radius] {
                        *v *= k;
                    }
                }
                Prim::Fillet { x, y, r, .. } => {
                    for v in [x, y, r] {
                        *v *= k;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- motion

#[derive(Clone, Copy, Debug)]
pub enum Ease {
    OutCubic,
    /// Overshoots a touch, then settles: ETEA's `Theme.expressive`.
    Expressive,
}

fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    // Solve x(s) = t for s, then return y(s)
    let at = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
    let mut s = t;
    for _ in 0..8 {
        let x = at(x1, x2, s) - t;
        let dx = 3.0 * x1 * (1.0 - s) * (1.0 - 3.0 * s) + 3.0 * x2 * s * (2.0 - 3.0 * s) + 3.0 * s * s;
        if dx.abs() < 1e-5 {
            break;
        }
        s = (s - x / dx).clamp(0.0, 1.0);
    }
    at(y1, y2, s)
}

impl Ease {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::Expressive => bezier(0.38, 1.21, 0.22, 1.0, t),
        }
    }
}

struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: f32,
    ease: Ease,
    used: u64,
}

/// Values that glide to where they're asked to be. Asked for every frame; a value nobody
/// asks for any more is forgotten.
pub struct Animator {
    tweens: HashMap<String, Tween>,
    now: Instant,
    frame: u64,
    moving: bool,
}

impl Animator {
    pub fn new() -> Self {
        Self { tweens: HashMap::new(), now: Instant::now(), frame: 0, moving: false }
    }

    pub fn begin(&mut self) {
        self.now = Instant::now();
        self.frame += 1;
        self.moving = false;
    }

    pub fn end(&mut self) {
        let frame = self.frame;
        self.tweens.retain(|_, t| frame - t.used <= 2);
    }

    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Glides towards `target`; a value seen for the first time starts there.
    pub fn value(&mut self, key: &str, target: f32, duration: f32, ease: Ease) -> f32 {
        self.value_from(key, target, target, duration, ease)
    }

    /// Glides towards `target`; a value seen for the first time starts at `initial`.
    pub fn value_from(&mut self, key: &str, initial: f32, target: f32, duration: f32, ease: Ease) -> f32 {
        let (now, frame) = (self.now, self.frame);
        let tween = self.tweens.entry(key.to_string()).or_insert_with(|| Tween { from: initial, to: target, start: now, duration, ease, used: frame });
        tween.used = frame;
        let progress = |t: &Tween| if t.duration <= 0.0 { 1.0 } else { now.duration_since(t.start).as_secs_f32() * 1000.0 / t.duration };
        if (tween.to - target).abs() > 1e-4 {
            // Start again from wherever it is now
            let p = progress(tween).min(1.0);
            let current = tween.from + (tween.to - tween.from) * tween.ease.apply(p);
            *tween = Tween { from: current, to: target, start: now, duration, ease, used: frame };
        }
        let p = progress(tween);
        if p >= 1.0 {
            return tween.to;
        }
        self.moving = true;
        tween.from + (tween.to - tween.from) * tween.ease.apply(p)
    }
}
