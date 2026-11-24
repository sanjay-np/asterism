//! The bar's sources, each running on its own task on a small tokio runtime, away from the
//! drawing. They send `Update`s back to the Wayland loop and take `Command`s from it.

mod audio;
mod bluetooth;
pub mod capture;
mod brightness;
mod dbus;
mod hyprland;
mod login;
mod lyrics;
mod machine;
mod mpris;
mod network;
mod niri;
mod notifications;
mod polkit;
mod portal;
mod power;
mod privacy;
mod spectrum;
mod tray;
mod weather;
pub mod xkb;

use crate::model::{Command, CompositorKind, Update};
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Where services drop what they learn.
#[derive(Clone)]
pub struct Out(Sender<Update>);

impl Out {
    pub fn send(&self, update: Update) {
        let _ = self.0.send(update);
    }
}

pub fn detect_compositor() -> CompositorKind {
    if std::env::var_os("NIRI_SOCKET").is_some() {
        CompositorKind::Niri
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        CompositorKind::Hyprland
    } else {
        CompositorKind::Unknown
    }
}

/// What the services take on beyond reading: the shell runs them, a snapshot doesn't.
#[derive(Clone, Copy, Default)]
pub struct Roles {
    /// Be the notification daemon when no other one runs.
    pub notification_daemon: bool,
    /// Listen for `asterism msg`.
    pub control_socket: bool,
    /// Answer polkit's password requests for the session.
    pub polkit_agent: bool,
    /// Lock when logind asks, and before sleep.
    pub lock: bool,
    /// Answer BlueZ's pairing questions (be its agent).
    pub bluetooth_agent: bool,
}

/// Starts everything, returns where to send commands.
pub fn spawn(updates: Sender<Update>, kind: CompositorKind, roles: Roles) -> UnboundedSender<Command> {
    let (commands, receiver) = unbounded_channel();
    std::thread::Builder::new()
        .name("asterism-services".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
            runtime.block_on(run(Out(updates), receiver, kind, roles));
        })
        .expect("services thread");
    commands
}

/// Runs a shell command without waiting for it, detached from the bar.
pub fn run_detached(command: &str) {
    if command.trim().is_empty() {
        return;
    }
    let result = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{command} &"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if let Err(error) = result {
        log::warn!("couldn't run {command}: {error}");
    }
}

async fn run(out: Out, mut commands: UnboundedReceiver<Command>, kind: CompositorKind, roles: Roles) {
    if roles.control_socket {
        tokio::spawn(crate::ipc::serve(out.clone()));
    }
    let compositor = match kind {
        CompositorKind::Niri => Some(start(out.clone(), niri::run)),
        CompositorKind::Hyprland => Some(start(out.clone(), hyprland::run)),
        CompositorKind::Unknown => {
            log::warn!("neither niri nor Hyprland found: workspaces stay empty");
            None
        }
    };
    let machine = start(out.clone(), machine::run);
    let weather = start(out.clone(), weather::run);
    let capture = start(out.clone(), capture::run);
    let lyrics = start(out.clone(), lyrics::run);
    let audio = start(out.clone(), audio::run);
    let spectrum = start(out.clone(), spectrum::run);
    tokio::spawn(privacy::run(out.clone()));

    let session = match zbus::Connection::session().await {
        Ok(connection) => Some(connection),
        Err(error) => {
            log::warn!("no session bus, so no tray and no music: {error}");
            None
        }
    };
    let system = match zbus::Connection::system().await {
        Ok(connection) => Some(connection),
        Err(error) => {
            log::warn!("no system bus, so no battery, network or bluetooth: {error}");
            None
        }
    };
    let media = session.clone().map(|bus| start_with(out.clone(), bus, mpris::run));
    let tray = session.clone().map(|bus| start_with(out.clone(), bus, tray::run));
    let power = system.clone().map(|bus| start_with(out.clone(), bus, power::run));
    let network = system.clone().map(|bus| start_with(out.clone(), bus, network::run));
    let agent = roles.bluetooth_agent;
    let bluetooth = system.clone().map(|bus| start_with(out.clone(), bus, move |out, bus, commands| bluetooth::run(out, bus, commands, agent)));
    let login = system.clone().filter(|_| roles.lock).map(|bus| start_with(out.clone(), bus, login::run));
    let polkit = match (&system, roles.polkit_agent) {
        (Some(bus), true) => Some(start_with(out.clone(), bus.clone(), polkit::run)),
        _ => None,
    };
    let notifications = {
        let (sender, receiver) = unbounded_channel();
        tokio::spawn(notifications::run(out.clone(), session.clone(), roles.notification_daemon, receiver));
        sender
    };
    let brightness = {
        let (sender, receiver) = unbounded_channel();
        tokio::spawn(brightness::run(out.clone(), system.clone(), receiver));
        sender
    };
    let mut awake: Option<tokio::process::Child> = None;
    // Big pictures take a lot of memory while they're decoded: a few thumbnails at a time
    let thumbnails = std::sync::Arc::new(tokio::sync::Semaphore::new(3));

    let forward = |to: &Option<UnboundedSender<Command>>, command: Command| {
        if let Some(to) = to {
            let _ = to.send(command);
        }
    };

    while let Some(command) = commands.recv().await {
        use Command::*;
        match command {
            FocusWorkspace(_) | NextLayout | SetLayout(_) | Quit | FocusWindow(_) | CloseWindow(_) | MoveWindow(..) | StopCast(_) => forward(&compositor, command),
            MediaPlayPause(_) | MediaNext(_) | MediaPrevious(_) | MediaSeek(..) | MediaChoose(_) | FetchArt(_) => {
                forward(&media, command)
            }
            SetVolume(_) | StepVolume(_) | ToggleMute | SetDefaultSink(_) | ToggleMicMute | SetStreamVolume(..) => {
                let _ = audio.send(command);
            }
            SetPowerProfile(_) => forward(&power, command),
            SetWifi(_) | ScanWifi | ConnectWifi(_) | ConnectWifiWith(..) | SetVpn(..) => forward(&network, command),
            SetBluetoothPower(_) | BluetoothConnect(..) | ScanBluetooth | BluetoothPair(_) | BluetoothForget(_) | PairAnswer(_) => forward(&bluetooth, command),
            TrayActivate(..) | TraySecondary(..) | TrayScroll(..) | TrayMenu(_) | TrayMenuClick(..) => forward(&tray, command),
            NotificationsToggle | NotificationsQuiet | NotificationInvoke(..) | NotificationDismiss(_) | NotificationExpired(_) | NotificationsClear => {
                let _ = notifications.send(command);
            }
            AuthRespond(..) | AuthCancel(_) => forward(&polkit, command),
            SetBrightness(_) | StepBrightness(_) => {
                let _ = brightness.send(command);
            }
            ToggleAwake => {
                // Staying awake: logind holds idle and sleep off while this runs
                if let Some(mut child) = awake.take() {
                    let _ = child.kill().await;
                    out.send(Update::Awake(false));
                } else {
                    match child("systemd-inhibit")
                        .args(["--what=idle:sleep", "--who=asterism", "--why=Staying awake", "--mode=block", "sleep", "infinity"])
                        .spawn()
                    {
                        Ok(child) => {
                            awake = Some(child);
                            out.send(Update::Awake(true));
                        }
                        Err(error) => log::warn!("systemd-inhibit: {error}"),
                    }
                }
            }
            Notify(note) => {
                if let Some(bus) = session.clone() {
                    tokio::spawn(async move {
                        let mut hints: std::collections::HashMap<&str, zbus::zvariant::Value> = std::collections::HashMap::new();
                        hints.insert("urgency", zbus::zvariant::Value::U8(note.urgency));
                        hints.insert("desktop-entry", zbus::zvariant::Value::from("asterism"));
                        let body = ("Asterism", 0u32, note.icon.as_str(), note.summary.as_str(), note.body.as_str(), Vec::<&str>::new(), hints, -1i32);
                        dbus::call(&bus, "org.freedesktop.Notifications", "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Notify", &body).await;
                    });
                }
            }
            LockBeforeSleep(_) => forward(&login, command),
            FetchLyrics { .. } => {
                let _ = lyrics.send(command);
            }
            PetSound(voice, variant, volume) => {
                tokio::task::spawn_blocking(move || {
                    // Made the first time it's wanted, then played by whichever player there is
                    let Some(path) = crate::pet::sound::file(voice, variant) else { return };
                    let played = std::process::Command::new("pw-play").arg("--volume").arg(format!("{volume:.2}")).arg(&path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
                    if played.is_err() {
                        let _ = std::process::Command::new("paplay").arg(format!("--volume={}", (volume * 65536.0) as u32)).arg(&path).stderr(std::process::Stdio::null()).status();
                    }
                });
            }
            ChooseFolder(start) => {
                let out = out.clone();
                match session.clone() {
                    Some(bus) => {
                        tokio::spawn(async move { out.send(Update::FolderChosen(portal::choose_folder(&bus, "Wallpaper folder", &start).await)) });
                    }
                    None => out.send(Update::FolderChosen(None)),
                }
            }
            DayLoad(dir) => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || out.send(Update::Day(crate::day::load(&dir))));
            }
            DaySave(dir, day) => {
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = crate::day::save(&dir, &day) {
                        log::warn!("couldn't save the day panel's files in {}: {error}", dir.display());
                    }
                });
            }
            LoadApps => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || out.send(Update::Apps(crate::launcher::Apps::load().entries)));
            }
            CountTrash => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || out.send(Update::Trash(crate::dock::trash_count())));
            }
            EmptyTrash => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || {
                    crate::dock::empty_trash();
                    out.send(Update::Trash(crate::dock::trash_count()));
                });
            }
            ListPhotos(dir) => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || {
                    let list = crate::wallpapers::list(&dir);
                    out.send(Update::Photos(dir, list));
                });
            }
            AppColors(palette, apps) => {
                tokio::task::spawn_blocking(move || {
                    let changed = crate::appcolors::write(&apps, &palette);
                    // kitty reads its config again on SIGUSR1; the others follow the file, or
                    // take it at their next start
                    if changed.iter().any(|a| a == "kitty") {
                        let _ = std::process::Command::new("pkill").args(["-USR1", "-x", "kitty"]).status();
                    }
                    if !changed.is_empty() {
                        log::info!("colours written for {}", changed.join(", "));
                    }
                });
            }
            WatchSpectrum(_) => {
                let _ = spectrum.send(command);
            }
            WatchProcesses(_) | EndProcess(_) => {
                let _ = machine.send(command);
            }
            Screenshot(_) | RecordStart { .. } | RecordStop | ScreenText => {
                let _ = capture.send(command);
            }
            WeatherSettings(_) | WeatherRefresh => {
                let _ = weather.send(command);
            }
            NotificationRules(_) => {
                let _ = notifications.send(command);
            }
            Thumbnail(path) => {
                let (out, thumbnails) = (out.clone(), thumbnails.clone());
                tokio::spawn(async move {
                    let Ok(_turn) = thumbnails.acquire_owned().await else { return };
                    if let Ok((path, image)) = tokio::task::spawn_blocking(move || {
                        let image = crate::wallpapers::thumbnail(&path);
                        crate::images::give_back_memory();
                        (path, image)
                    })
                    .await
                    {
                        out.send(Update::Thumbnail(path, image));
                    }
                });
            }
            WallpaperPalette(path) => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || {
                    let source = crate::palette::source_of(&crate::config::expand_home(&path)).unwrap_or(crate::palette::DEFAULT_SOURCE);
                    crate::images::give_back_memory();
                    out.send(Update::WallpaperPalette(path, crate::palette::from_source(source)));
                });
            }
            AppIcon(app) => {
                let out = out.clone();
                tokio::task::spawn_blocking(move || {
                    let name = crate::images::app(&app).map(|a| a.icon.clone()).filter(|i| !i.is_empty()).unwrap_or_else(|| app.to_lowercase());
                    // Big enough for the launcher's grid on a scaled screen
                    let icon = crate::images::find_icon(&name, 128, None).and_then(|path| crate::images::load(&path, 128));
                    out.send(Update::AppIcon(app, icon));
                });
            }
            Run(line) => run_detached(&line),
        }
    }
}

/// A long-running helper (`pactl subscribe` and the like) that goes away with the bar,
/// even if the bar is killed.
pub fn child(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    // SAFETY: only an async-signal-safe prctl between fork and exec
    unsafe {
        command.pre_exec(|| {
            rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM)).map_err(std::io::Error::from)
        });
    }
    command
}

fn start<F, Fut>(out: Out, service: F) -> UnboundedSender<Command>
where
    F: FnOnce(Out, UnboundedReceiver<Command>) -> Fut,
    Fut: std::future::Future<Output = ()> + 'static + Send,
{
    let (sender, receiver) = unbounded_channel();
    tokio::spawn(service(out, receiver));
    sender
}

fn start_with<F, Fut>(out: Out, bus: zbus::Connection, service: F) -> UnboundedSender<Command>
where
    F: FnOnce(Out, zbus::Connection, UnboundedReceiver<Command>) -> Fut,
    Fut: std::future::Future<Output = ()> + 'static + Send,
{
    let (sender, receiver) = unbounded_channel();
    tokio::spawn(service(out, bus, receiver));
    sender
}

/// Reads a whole command's output, or nothing if it fails.
pub async fn output(program: &str, args: &[&str]) -> Option<String> {
    let result = tokio::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).output().await.ok()?;
    if !result.status.success() {
        return None;
    }
    String::from_utf8(result.stdout).ok()
}
