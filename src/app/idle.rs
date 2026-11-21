//! Locking: starting the lock screen as its own process, and locking after a while without
//! input (ext-idle-notify, which honours "Stay awake" and apps that keep the screen on).

use super::*;
use smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

pub struct IdleData;

impl Dispatch2<ExtIdleNotificationV1, App> for IdleData {
    fn event(&self, app: &mut App, _: &ExtIdleNotificationV1, event: ext_idle_notification_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        if let ext_idle_notification_v1::Event::Idled = event {
            log::info!("idle for {} min: locking", app.config.lock.idle);
            app.start_lock();
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

    /// Keeps the idle watch in step with the config's minutes.
    pub(super) fn watch_idle(&mut self) {
        let minutes = self.config.lock.idle;
        if self.idle_watch.as_ref().map(|(_, m)| *m) == Some(minutes).filter(|m| *m > 0) {
            return;
        }
        if let Some((old, _)) = self.idle_watch.take() {
            old.destroy();
        }
        let (Some(notifier), Some(seat)) = (&self.idle_notifier, &self.seat) else { return };
        if minutes == 0 {
            return;
        }
        let notification = notifier.get_idle_notification(minutes.saturating_mul(60_000), seat, &self.qh, IdleData);
        self.idle_watch = Some((notification, minutes));
    }
}

pub(super) fn bind_idle(globals: &smithay_client_toolkit::reexports::client::globals::GlobalList, qh: &QueueHandle<App>) -> Option<ExtIdleNotifierV1> {
    globals.bind::<ExtIdleNotifierV1, App, Silent>(qh, 1..=1, Silent).ok()
}
