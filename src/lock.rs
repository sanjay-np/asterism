//! The lock screen: its own process (`asterism lock`), so nothing that happens to the bar can
//! leave the screen unlocked or you locked out. It takes the session lock (ext-session-lock),
//! covers every output, and asks for your password, which the system's own `unix_chkpwd`
//! checks (no PAM linked, /etc/shadow never read here).
//!
//! If it ever can't be unlocked by password: switch to a text console (Ctrl+Alt+F3), log in,
//! and run `asterism unlock`. It leaves a file the lock watches for; only you (or root) can
//! write to your runtime directory.

use crate::config::Config;
use crate::model::Secret;
use crate::render::Painter;
use crate::render::text::Font;
use crate::theme::Theme;
use crate::ui::Prim;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            self, EventLoop,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
    },
    session_lock::{SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface, SessionLockSurfaceConfigure},
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tiny_skia::Pixmap;

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// Written once the screen is locked, holding the lock's process id.
pub fn pid_file() -> PathBuf {
    runtime_dir().join("asterism-lock.pid")
}

/// Its appearance asks the lock to let go (`asterism unlock`, from a console).
fn unlock_file() -> PathBuf {
    runtime_dir().join("asterism-unlock")
}

/// Whether a lock is up right now.
pub fn running() -> bool {
    std::fs::read_to_string(pid_file())
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .and_then(rustix::process::Pid::from_raw)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

/// `asterism unlock`: asks a running lock to let go.
pub fn request_unlock() -> anyhow::Result<()> {
    if !running() {
        anyhow::bail!("the screen isn't locked by asterism");
    }
    std::fs::write(unlock_file(), b"")?;
    Ok(())
}

/// Checks a password with the system's helper: exit 0 is right.
fn check_password(user: &str, password: Secret) -> bool {
    let child = std::process::Command::new("unix_chkpwd")
        .args([user, "nonull"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        log::error!("lock: can't run unix_chkpwd");
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let mut bytes = password.bytes().to_vec();
        bytes.push(0);
        let _ = stdin.write_all(&bytes);
        bytes.iter_mut().for_each(|b| *b = 0);
    }
    child.wait().is_ok_and(|status| status.success())
}

/// Your name for the screen: the full one from /etc/passwd when there is one.
fn who() -> (String, String) {
    let user = std::env::var("USER").unwrap_or_default();
    let full = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|text| text.lines().find(|l| l.split(':').next() == Some(user.as_str())).map(|l| l.split(':').nth(4).unwrap_or("").split(',').next().unwrap_or("").to_string()))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| user.clone());
    (user, full)
}

/// What the fingerprint reader says.
#[derive(Debug, Clone, PartialEq)]
enum Finger {
    /// A reader, your fingers, and it's listening.
    Ready,
    Matched,
    NoMatch,
    /// Read badly: try again (why).
    Retry(&'static str),
    /// Gone (unplugged, or fprintd stopped).
    Gone,
}

const FPRINT: &str = "net.reactivated.Fprint";
const FPRINT_DEVICE: &str = "net.reactivated.Fprint.Device";

/// Listens on the fingerprint reader through fprintd, for as long as the lock is up. Says
/// nothing at all when there's no reader or no enrolled finger.
async fn fingerprint(user: String, tell: calloop::channel::Sender<Finger>) -> Option<()> {
    use futures_lite::StreamExt;
    let bus = zbus::Connection::system().await.ok()?;
    let reply = bus.call_method(Some(FPRINT), "/net/reactivated/Fprint/Manager", Some("net.reactivated.Fprint.Manager"), "GetDefaultDevice", &()).await.ok()?;
    let device: zbus::zvariant::OwnedObjectPath = reply.body().deserialize().ok()?;
    let call = |method: &'static str, user: Option<&str>| {
        let (bus, device) = (bus.clone(), device.clone());
        let user = user.map(str::to_string);
        async move {
            match user {
                Some(u) => bus.call_method(Some(FPRINT), device.as_str(), Some(FPRINT_DEVICE), method, &(u.as_str(),)).await,
                None => bus.call_method(Some(FPRINT), device.as_str(), Some(FPRINT_DEVICE), method, &()).await,
            }
        }
    };
    let fingers: Vec<String> = call("ListEnrolledFingers", Some(&user)).await.ok()?.body().deserialize().ok()?;
    if fingers.is_empty() {
        return None;
    }
    call("Claim", Some(&user)).await.ok()?;
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(FPRINT).ok()?.interface(FPRINT_DEVICE).ok()?.member("VerifyStatus").ok()?.path(device.as_str().to_string()).ok()?.build();
    let mut statuses = zbus::MessageStream::for_match_rule(rule, &bus, Some(16)).await.ok()?;
    let _ = tell.send(Finger::Ready);
    loop {
        if bus.call_method(Some(FPRINT), device.as_str(), Some(FPRINT_DEVICE), "VerifyStart", &("any",)).await.is_err() {
            let _ = tell.send(Finger::Gone);
            return None;
        }
        while let Some(Ok(message)) = statuses.next().await {
            let Ok((result, done)) = message.body().deserialize::<(String, bool)>() else { continue };
            match result.as_str() {
                "verify-match" => {
                    let _ = call("VerifyStop", None).await;
                    let _ = call("Release", None).await;
                    let _ = tell.send(Finger::Matched);
                    return Some(());
                }
                "verify-no-match" => {
                    let _ = tell.send(Finger::NoMatch);
                }
                "verify-retry-scan" => {
                    let _ = tell.send(Finger::Retry("Try that again"));
                }
                "verify-swipe-too-short" => {
                    let _ = tell.send(Finger::Retry("A little longer"));
                }
                "verify-finger-not-centered" => {
                    let _ = tell.send(Finger::Retry("Centre your finger"));
                }
                "verify-remove-and-retry" => {
                    let _ = tell.send(Finger::Retry("Lift your finger, then try again"));
                }
                "verify-disconnected" => {
                    let _ = tell.send(Finger::Gone);
                    return None;
                }
                _ => {}
            }
            if done {
                // fprintd ends a round after a result: start the next one
                let _ = call("VerifyStop", None).await;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

struct LockSurf {
    surface: SessionLockSurface,
    output: wl_output::WlOutput,
    size: (u32, u32),
    scale: i32,
}

struct Lock {
    registry: RegistryState,
    outputs: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    lock_state: SessionLockState,
    lock: Option<SessionLock>,
    surfaces: Vec<LockSurf>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    modifiers: Modifiers,
    password: Secret,
    message: String,
    checking: bool,
    config: Config,
    theme: Theme,
    painter: Painter,
    /// Wallpapers, by size, painted once.
    backgrounds: HashMap<(u32, u32), Pixmap>,
    user: String,
    name: String,
    checked: calloop::channel::Sender<bool>,
    /// The fingerprint reader is listening.
    finger: bool,
    finger_to: calloop::channel::Sender<Finger>,
    done: bool,
}

impl Lock {
    fn add_surface(&mut self, output: wl_output::WlOutput, qh: &QueueHandle<Self>) {
        let Some(lock) = self.lock.as_ref().filter(|l| l.is_locked()) else { return };
        if self.surfaces.iter().any(|s| s.output == output) {
            return;
        }
        let wl = self.compositor.create_surface(qh);
        let scale = self.outputs.info(&output).map_or(1, |i| i.scale_factor.max(1));
        let surface = lock.create_lock_surface(wl, &output, qh);
        self.surfaces.push(LockSurf { surface, output, size: (0, 0), scale });
    }

    fn redraw_all(&mut self) {
        for i in 0..self.surfaces.len() {
            self.draw(i);
        }
    }

    /// One output's lock: the wallpaper, dimmed; the time; your name; the password field.
    fn draw(&mut self, index: usize) {
        let (w, h, scale) = {
            let s = &self.surfaces[index];
            (s.size.0, s.size.1, s.scale)
        };
        if w == 0 || h == 0 {
            return;
        }
        let (pw, ph) = (w * scale as u32, h * scale as u32);
        let background = self.backgrounds.entry((pw, ph)).or_insert_with(|| {
            let t = &self.theme;
            crate::world::render(pw, ph, scale as f32, crate::theme::planet(&self.config.planet), &self.config.wallpaper, &t.palette).unwrap_or_else(|| Pixmap::new(pw, ph).expect("size"))
        });
        let mut pixmap = background.clone();
        let t = &self.theme;
        let (wf, hf) = (w as f32, h as f32);
        let mut prims = vec![Prim::Rect { x: 0.0, y: 0.0, w: wf, h: hf, radius: 0.0, color: t.surface.alpha(0.6) }];
        let text = &mut self.painter.text;
        let now = chrono::Local::now();
        let center = |text: &mut crate::render::text::Text, s: &str, font: Font, y: f32, color| {
            let (tw, _) = text.measure(s, font);
            Prim::Text { x: ((wf - tw) / 2.0).round(), y, text: s.to_string(), font, color }
        };
        let clock_font = Font::new(96.0).weight(300).tnum();
        let (_, ch) = text.measure("12:00", clock_font);
        let top = (hf * 0.22).round();
        prims.push(center(text, &now.format("%H:%M").to_string(), clock_font, top, t.color_text));
        prims.push(center(text, &now.format("%A, %B %-d").to_string(), Font::new(t.text + 5.0), top + ch, t.color_soft));

        // You, and the field
        let fy = (hf * 0.58).round();
        let initial: String = self.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        let r = 34.0;
        prims.push(Prim::Rect { x: wf / 2.0 - r, y: fy - 2.0 * r - 44.0, w: r * 2.0, h: r * 2.0, radius: r, color: t.primary_container });
        let (iw, ih) = text.measure(&initial, Font::new(30.0).weight(500));
        prims.push(Prim::Text { x: wf / 2.0 - iw / 2.0, y: fy - r - 44.0 - ih / 2.0, text: initial, font: Font::new(30.0).weight(500), color: t.on_primary_container });
        prims.push(center(text, &self.name, Font::new(t.text + 3.0).weight(500), fy - 36.0, t.color_text));
        let (fw, fh) = (340.0f32.min(wf - 48.0), 48.0);
        let fx = ((wf - fw) / 2.0).round();
        prims.push(Prim::Rect { x: fx, y: fy, w: fw, h: fh, radius: fh / 2.0, color: t.hairline() });
        prims.push(Prim::Rect { x: fx + 1.0, y: fy + 1.0, w: fw - 2.0, h: fh - 2.0, radius: fh / 2.0 - 1.0, color: t.card_deep });
        if self.password.is_empty() {
            prims.push(center(text, if self.checking { "Checking…" } else { "Password" }, Font::new(t.text + 1.0), fy + 14.0, t.color_soft.alpha(0.6)));
        } else {
            // A dot a character, never the characters
            let n = self.password.len().min(28);
            let (d, gap) = (8.0, 6.0);
            let total = n as f32 * d + (n as f32 - 1.0) * gap;
            for i in 0..n {
                let x = wf / 2.0 - total / 2.0 + i as f32 * (d + gap);
                prims.push(Prim::Rect { x, y: fy + fh / 2.0 - d / 2.0, w: d, h: d, radius: d / 2.0, color: if self.checking { t.color_soft } else { t.color_text } });
            }
        }
        let (message, color) = if !self.message.is_empty() {
            (self.message.clone(), t.warn)
        } else if self.modifiers.caps_lock {
            ("Caps Lock is on".to_string(), t.warn)
        } else if self.finger {
            ("Touch the fingerprint reader, or type your password".to_string(), t.color_soft.alpha(0.7))
        } else {
            ("Type your password, then Enter".to_string(), t.color_soft.alpha(0.7))
        };
        if self.finger {
            prims.push(Prim::Icon { x: fx + 16.0, y: fy + (fh - 20.0) / 2.0, size: 20.0, name: "fingerprint".into(), color: t.primary });
        }
        prims.push(center(text, &message, Font::new(t.text - 1.0), fy + fh + 14.0, color));
        self.painter.draw(&mut pixmap, &self.theme, &prims, 0.0, 0.0, scale as f32);

        // Into a buffer, as the compositor reads it (BGRA in memory)
        let stride = pw as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(pw as i32, ph as i32, stride, wl_shm::Format::Argb8888) else { return };
        for (dst, src) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(pixmap.data().as_chunks::<4>().0) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        let s = &self.surfaces[index];
        let wl = s.surface.wl_surface();
        wl.set_buffer_scale(scale);
        wl.damage_buffer(0, 0, pw as i32, ph as i32);
        let _ = buffer.attach_to(wl);
        wl.commit();
    }

    fn submit(&mut self) {
        if self.checking || self.password.is_empty() {
            return;
        }
        self.checking = true;
        self.message.clear();
        let (user, password, sender) = (self.user.clone(), self.password.take(), self.checked.clone());
        std::thread::spawn(move || {
            let right = check_password(&user, password);
            // A wrong guess costs a moment, a right one doesn't
            if !right {
                std::thread::sleep(Duration::from_millis(800));
            }
            let _ = sender.send(right);
        });
        self.redraw_all();
    }

    fn finger(&mut self, said: Finger) {
        match said {
            Finger::Ready => self.finger = true,
            Finger::Matched => return self.unlock(),
            Finger::NoMatch => self.message = "That fingerprint isn't one of yours".into(),
            Finger::Retry(why) => self.message = why.into(),
            Finger::Gone => self.finger = false,
        }
        self.redraw_all();
    }

    fn checked(&mut self, right: bool) {
        self.checking = false;
        if right {
            self.unlock();
        } else {
            self.message = "That's not it. Try again.".into();
            self.redraw_all();
        }
    }

    fn unlock(&mut self) {
        if let Some(lock) = self.lock.take() {
            lock.unlock();
        }
        self.surfaces.clear();
        let _ = std::fs::remove_file(pid_file());
        let _ = std::fs::remove_file(unlock_file());
        self.done = true;
    }

    fn key(&mut self, event: &KeyEvent) {
        if self.checking {
            return;
        }
        match event.keysym {
            Keysym::Return | Keysym::KP_Enter => return self.submit(),
            Keysym::BackSpace => self.password.pop(),
            Keysym::Escape => self.password.clear(),
            Keysym::u if self.modifiers.ctrl => self.password.clear(),
            _ => match &event.utf8 {
                Some(text) if !self.modifiers.ctrl && !self.modifiers.alt && text.chars().all(|c| !c.is_control()) => self.password.push(text),
                _ => return,
            },
        }
        self.message.clear();
        self.redraw_all();
    }
}

impl SessionLockHandler for Lock {
    fn locked(&mut self, _: &Connection, qh: &QueueHandle<Self>, lock: SessionLock) {
        log::info!("lock: the screen is locked");
        self.lock = Some(lock);
        if self.config.lock.fingerprint {
            let (user, tell) = (self.user.clone(), self.finger_to.clone());
            std::thread::spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    runtime.block_on(fingerprint(user, tell));
                }
            });
        }
        let _ = std::fs::write(pid_file(), std::process::id().to_string());
        let outputs: Vec<_> = self.outputs.outputs().collect();
        for output in outputs {
            self.add_surface(output, qh);
        }
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        // Refused (another locker holds it), or taken away
        log::warn!("lock: the compositor ended the lock");
        let _ = std::fs::remove_file(pid_file());
        self.done = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: SessionLockSurface, configure: SessionLockSurfaceConfigure, _: u32) {
        if let Some(index) = self.surfaces.iter().position(|s| s.surface.wl_surface() == surface.wl_surface()) {
            self.surfaces[index].size = configure.new_size;
            self.draw(index);
        }
    }
}

impl CompositorHandler for Lock {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(index) = self.surfaces.iter().position(|s| s.surface.wl_surface() == surface) {
            self.surfaces[index].scale = factor.max(1);
            self.draw(index);
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Lock {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // A screen plugged in while locked gets covered too
        self.add_surface(output, qh);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.surfaces.retain(|s| s.output != output);
    }
}

impl SeatHandler for Lock {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(error) => log::warn!("lock: no keyboard: {error}"),
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            self.keyboard = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Lock {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(&event);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(&event);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        let caps_changed = modifiers.caps_lock != self.modifiers.caps_lock;
        self.modifiers = modifiers;
        if caps_changed {
            self.redraw_all();
        }
    }
}

impl ShmHandler for Lock {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Lock {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Lock);
smithay_client_toolkit::delegate_dispatch2!(Lock);

/// `asterism lock`: locks, and returns once unlocked.
pub fn run(config_path: PathBuf) -> anyhow::Result<()> {
    if running() {
        log::info!("lock: already locked");
        return Ok(());
    }
    let _ = std::fs::remove_file(unlock_file());
    let config = Config::load(&config_path);
    let theme = Theme::new(&config);
    let painter = Painter::new(&config.font, &config.icons);
    let conn = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<Lock>(&conn)?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<Lock> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue).insert(handle.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (finger_to, fingers) = calloop::channel::channel::<Finger>();
    handle
        .insert_source(fingers, |event, _, lock: &mut Lock| {
            if let calloop::channel::Event::Msg(said) = event {
                lock.finger(said);
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (checked, results) = calloop::channel::channel::<bool>();
    handle
        .insert_source(results, |event, _, lock: &mut Lock| {
            if let calloop::channel::Event::Msg(right) = event {
                lock.checked(right);
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    // Twice a second: the clock, and whether `asterism unlock` asked to let go
    let mut minute = String::new();
    handle
        .insert_source(Timer::from_duration(Duration::from_millis(500)), move |_, _, lock: &mut Lock| {
            if unlock_file().exists() {
                log::warn!("lock: unlocked from outside (asterism unlock)");
                lock.unlock();
            }
            let now = chrono::Local::now().format("%H:%M").to_string();
            if now != minute {
                minute = now;
                lock.redraw_all();
            }
            TimeoutAction::ToDuration(Duration::from_millis(500))
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new(1920 * 1200 * 4, &shm)?;
    let lock_state = SessionLockState::new(&globals, &qh);
    let (user, name) = who();
    let mut lock = Lock {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)?,
        shm,
        pool,
        lock: None,
        lock_state,
        surfaces: Vec::new(),
        keyboard: None,
        modifiers: Modifiers::default(),
        password: Secret::default(),
        message: String::new(),
        checking: false,
        config,
        theme,
        painter,
        backgrounds: HashMap::new(),
        user,
        name,
        checked,
        finger: false,
        finger_to,
        done: false,
    };
    // Held from the start: the lock must live until the compositor says locked (or refuses)
    lock.lock = Some(lock.lock_state.lock(&qh).map_err(|e| anyhow::anyhow!("no session lock here: {e}"))?);
    while !lock.done {
        event_loop.dispatch(Duration::from_millis(500), &mut lock)?;
        let _ = conn.flush();
    }
    // Let the compositor hear the unlock before going
    let _ = conn.roundtrip();
    log::info!("lock: unlocked");
    Ok(())
}
