//! Live wallpapers as the app runs them (`crate::live` plays them): a player for each wallpaper
//! showing a video or GIF, asked for a frame each time the compositor wants one, and not asked
//! while something holds it still: on battery (unless allowed), the screen locked, or you away
//! for `live_idle` minutes. Held a minute, the player goes (and `ffmpeg` with it, and their
//! memory); it starts again from the beginning when the hold ends.
//!
//! A covered wallpaper needs nothing of its own: the compositor stops asking for its frames (niri
//! slows them to about one a second), so the player isn't asked either.

use super::*;
use crate::live::{Frame, Player, Spec};
use smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{self, ExtIdleNotificationV1};

/// How long a hold may last before the players go.
const PUT_AWAY_AFTER: Duration = Duration::from_secs(60);

pub struct LiveIdle;

impl Dispatch2<ExtIdleNotificationV1, App> for LiveIdle {
    fn event(&self, app: &mut App, _: &ExtIdleNotificationV1, event: ext_idle_notification_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        app.live_away = matches!(event, ext_idle_notification_v1::Event::Idled);
        log::debug!("live wallpaper: {}", if app.live_away { "you're away, still" } else { "you're back, playing" });
        app.hold_live();
        app.sync_live();
    }
}

impl App {
    /// Why the wallpapers shouldn't move now, if anything.
    fn live_held(&self) -> Option<&'static str> {
        let settings = &self.config.wallpaper;
        let battery = &self.model.battery;
        if self.live_locked {
            Some("locked")
        } else if self.live_away {
            Some("away")
        } else if battery.present && battery.on_battery && !settings.live_battery {
            Some("on battery")
        } else {
            None
        }
    }

    /// Notes when a hold began (or ended), so the players go after a minute of it.
    pub(super) fn hold_live(&mut self) {
        match (self.live_held(), self.live_held_since) {
            (Some(why), None) => {
                log::debug!("live wallpaper held: {why}");
                self.live_held_since = Some(Instant::now());
            }
            (None, Some(_)) => self.live_held_since = None,
            _ => {}
        }
    }

    /// What should play on a wallpaper now, if anything: the video or GIF on screen, unless
    /// moving wallpapers are off, it can't play here, it's being changed, or a long hold.
    fn live_spec(&self, index: usize) -> Option<Spec> {
        let settings = &self.config.wallpaper;
        let backdrop = &self.backdrops[index];
        let job = backdrop.job.as_ref()?;
        if !settings.enabled || !settings.live || backdrop.shown.is_none() || backdrop.change.is_some() || backdrop.painting.is_some() {
            return None;
        }
        if self.live_held_since.is_some_and(|since| since.elapsed() >= PUT_AWAY_AFTER) {
            return None;
        }
        let path = config::expand_home(job.settings.image.trim());
        if !crate::live::playable(&path) {
            return None;
        }
        let surface = job.palette.surface;
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(Spec {
            path,
            w: job.w,
            h: job.h,
            fit: job.settings.fit,
            fps: settings.live_fps,
            scale: settings.live_scale,
            scaled: backdrop.surf.viewport.is_some(),
            background: [byte(surface.r), byte(surface.g), byte(surface.b)],
        })
    }

    /// Players follow what's wanted: started, started again with new settings, or put away.
    pub(super) fn sync_live(&mut self) {
        for index in 0..self.backdrops.len() {
            let wanted = self.live_spec(index);
            let backdrop = &mut self.backdrops[index];
            if backdrop.live.as_ref().map(|p| &p.spec) == wanted.as_ref() {
                continue;
            }
            if let Some(old) = backdrop.live.take() {
                log::debug!("live wallpaper {}: put away after {} frames", old.id, old.frames);
                // The frame on screen stays; its buffers go once drawn
                backdrop.release = Some(Instant::now());
            }
            let Some(spec) = wanted else { continue };
            self.next_player += 1;
            let id = self.next_player;
            log::info!("live wallpaper {id}: {} ({}×{} screen, {} fps, {} %)", spec.path.display(), spec.w, spec.h, spec.fps, spec.scale);
            backdrop.live = Player::start(id, spec, self.live_frames.clone());
            if backdrop.live.is_some() {
                backdrop.release = None;
            }
        }
        // Away is watched only while something plays
        let minutes = self.config.wallpaper.live_idle;
        let playing = self.backdrops.iter().any(|b| b.live.is_some()) || self.live_away;
        let watched = self.live_idle_watch.as_ref().map(|(_, m)| *m);
        if !playing || minutes == 0 {
            if let Some((watch, _)) = self.live_idle_watch.take() {
                watch.destroy();
            }
            self.live_away = false;
        } else if watched != Some(minutes)
            && let (Some(notifier), Some(seat)) = (&self.idle_notifier, &self.seat)
        {
            if let Some((watch, _)) = self.live_idle_watch.take() {
                watch.destroy();
            }
            let watch = notifier.get_idle_notification(minutes.saturating_mul(60_000), seat, &self.qh, LiveIdle);
            self.live_idle_watch = Some((watch, minutes));
        }
    }

    /// Once a second: the lock and the battery looked at, players started or put away.
    pub(super) fn tick_live(&mut self) {
        let any = self.backdrops.iter().any(|b| b.live.is_some()) || self.live_held_since.is_some();
        self.live_locked = any && crate::lock::running();
        self.hold_live();
        self.sync_live();
    }

    /// Asks each player for its next frame, when the compositor wants one and nothing holds it.
    pub(super) fn step_live(&mut self) {
        if self.live_held().is_some() {
            return;
        }
        for backdrop in &mut self.backdrops {
            let Some(player) = backdrop.live.as_mut() else { continue };
            let surf = &backdrop.surf;
            if player.asked || backdrop.change.is_some() || surf.frame_pending || !surf.configured {
                continue;
            }
            player.ask();
        }
    }

    /// A frame came: on screen with it, if its player is still playing.
    pub(super) fn live_frame(&mut self, frame: Frame) {
        let Some(backdrop) = self.backdrops.iter_mut().find(|b| b.live.as_ref().is_some_and(|p| p.id == frame.player)) else { return };
        let Some(player) = backdrop.live.as_mut() else { return };
        player.asked = false;
        if player.frames == 0 {
            log::debug!("live wallpaper {}: first frame, {}×{}", player.id, frame.w, frame.h);
        }
        player.frames += 1;
        if backdrop.change.is_none() && backdrop.surf.configured {
            present_frame(&mut backdrop.surf, &frame, &self.qh);
        }
        player.spare = Some(frame);
    }
}

/// A frame to the compositor, copied whole into a buffer from the surface's own pool (two at
/// most, made again when the frames' size changes), scaled by the viewporter to the screen.
fn present_frame(surf: &mut Surf, frame: &Frame, qh: &QueueHandle<App>) {
    let Some(wl) = surf.wl.clone() else { return };
    let (w, h) = (frame.w as i32, frame.h as i32);
    let bytes = frame.data.len();
    if bytes != w as usize * h as usize * 4 {
        return;
    }
    surf.buffers.retain(|(b, _)| b.height() == h && b.stride() == w * 4);
    if surf.buffers.is_empty() {
        // A new size: a pool just big enough for two
        surf.own_pool = surf.shm.clone().and_then(|shm| SlotPool::new(bytes * 2, &smithay_client_toolkit::shm::Shm::from(shm)).ok());
    }
    let Some(pool) = surf.own_pool.as_mut() else { return };
    let mut chosen = None;
    for (index, (buffer, stale)) in surf.buffers.iter_mut().enumerate() {
        // (the pool rounds a buffer's length up: only its start is the frame)
        if let Some(canvas) = buffer.canvas(pool) {
            canvas[..bytes].copy_from_slice(&frame.data);
            *stale = None;
            chosen = Some(index);
            break;
        }
    }
    let chosen = match chosen {
        Some(index) => index,
        None => {
            let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else {
                log::warn!("no buffer for a {w}×{h} wallpaper frame");
                return;
            };
            canvas[..bytes].copy_from_slice(&frame.data);
            if surf.buffers.len() >= 2 {
                surf.buffers.remove(0);
            }
            surf.buffers.push((buffer, None));
            surf.buffers.len() - 1
        }
    };
    if let Some(viewport) = &surf.viewport {
        wl.set_buffer_scale(1);
        viewport.set_destination(surf.w.round() as i32, surf.h.round() as i32);
    } else {
        wl.set_buffer_scale(surf.int_scale.max(1));
    }
    wl.damage_buffer(0, 0, w, h);
    wl.frame(qh, FrameCallbackData(wl.clone()));
    surf.frame_pending = true;
    if surf.buffers[chosen].0.attach_to(&wl).is_err() {
        return;
    }
    wl.commit();
}
