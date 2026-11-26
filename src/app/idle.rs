//! Locking: starting the lock screen as its own process; and the steps taken after a while
//! without input (dim, lock, screens off, sleep: `crate::power::steps`), each its own
//! ext-idle-notify watch, which honours "Stay awake" and apps that keep the screen on, and says
//! when you're back so a step can be undone.

use super::*;
use crate::power::Step;
use smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

pub struct IdleData(pub Step);

impl Dispatch2<ExtIdleNotificationV1, App> for IdleData {
    fn event(&self, app: &mut App, _: &ExtIdleNotificationV1, event: ext_idle_notification_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        match event {
            ext_idle_notification_v1::Event::Idled => app.idled(self.0),
            ext_idle_notification_v1::Event::Resumed => app.resumed(self.0),
            _ => {}
        }
    }
}

impl App {
    /// Starts `asterism lock`, unless one is up. It's left to run on its own: nothing the shell
    /// does afterwards (restarting, crashing) touches it.
    pub(super) fn start_lock(&mut self) {
        if crate::lock::running() {
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("asterism"));
        let child = std::process::Command::new(exe)
            .arg("lock")
            .arg("-c")
            .arg(&self.config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn();
        match child {
            // Reaped when it's done, so it doesn't linger as a zombie
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(error) => log::error!("couldn't start the lock screen: {error}"),
        }
        self.close_menu();
        for kind in [PanelKind::Launcher, PanelKind::Control, PanelKind::Day, PanelKind::Wallpapers, PanelKind::Shortcuts, PanelKind::Clipboard] {
            self.close_panel(kind);
        }
    }

    /// Keeps the idle watches in step with the config and the battery: only the steps that
    /// changed are made again (a new watch starts counting from now).
    pub(super) fn watch_idle(&mut self) {
        let wanted = crate::power::steps(&self.config, self.model.battery.on_battery);
        let have: Vec<(Step, u32)> = self.idle_watches.iter().map(|(step, ms, _)| (*step, *ms)).collect();
        if have == wanted {
            return;
        }
        let (Some(notifier), Some(seat)) = (self.idle_notifier.clone(), self.seat.clone()) else { return };
        let mut undim = false;
        self.idle_watches.retain(|(step, ms, watch)| {
            let keep = wanted.contains(&(*step, *ms));
            if !keep {
                watch.destroy();
                // Its "you're back" won't come now
                undim |= *step == Step::Dim;
            }
            keep
        });
        if undim {
            self.dim(false);
        }
        for (step, ms) in wanted {
            if !self.idle_watches.iter().any(|(s, m, _)| *s == step && *m == ms) {
                let watch = notifier.get_idle_notification(ms, &seat, &self.qh, IdleData(step));
                self.idle_watches.push((step, ms, watch));
            }
        }
        self.idle_watches.sort_by_key(|(_, ms, _)| *ms);
        log::debug!("idle steps: {:?}", self.idle_watches.iter().map(|(s, ms, _)| (*s, ms / 1000)).collect::<Vec<_>>());
    }

    /// A step's time without input has come.
    fn idled(&mut self, step: Step) {
        log::info!("idle: {step:?}");
        match step {
            Step::Dim => self.dim(true),
            Step::Lock => self.start_lock(),
            Step::Off => self.screens(false),
            Step::OffLocked => {
                if crate::lock::running() {
                    self.screens(false);
                }
            }
            Step::Sleep => {
                // Not while music plays (players don't hold idle off as video apps do); the
                // watch is made again, so it asks once more after the same time
                if self.model.media.players.iter().any(|p| p.playing) {
                    log::info!("idle: not sleeping while music plays");
                    if let Some(at) = self.idle_watches.iter().position(|(s, _, _)| *s == Step::Sleep) {
                        self.idle_watches.remove(at).2.destroy();
                    }
                    return;
                }
                self.send(Command::Suspend);
            }
        }
    }

    /// You're back: undo what can be.
    fn resumed(&mut self, step: Step) {
        match step {
            Step::Dim => self.dim(false),
            Step::Off | Step::OffLocked => {
                if self.screens_off {
                    self.screens(true);
                }
            }
            Step::Lock | Step::Sleep => {}
        }
    }
}

pub(super) fn bind_idle(globals: &smithay_client_toolkit::reexports::client::globals::GlobalList, qh: &QueueHandle<App>) -> Option<ExtIdleNotifierV1> {
    globals.bind::<ExtIdleNotifierV1, App, Silent>(qh, 1..=1, Silent).ok()
}
