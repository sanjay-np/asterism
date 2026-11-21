//! Night light, as the app runs it: a gamma control per output (wlr-gamma-control), warm
//! curves written to them as the evening comes, and the switch in the control centre. The
//! controls are only held while the screen is warmed: letting go gives each screen its own
//! colours back (any calibration included), and leaves them free for other tools.

use super::*;
use crate::night::{NEUTRAL, NightStatus, minutes, night_level, ramps};
use smithay_client_toolkit::reexports::protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};
use std::io::{Seek, Write};

/// An output's gamma control, and what it was last given.
pub(super) struct Gamma {
    pub output: wl_output::WlOutput,
    pub control: ZwlrGammaControlV1,
    /// How many steps each curve has, once the compositor has said.
    pub size: u32,
    pub applied: Option<u32>,
}

/// Which output a control is for.
pub struct GammaData(wl_output::WlOutput);

impl Dispatch2<ZwlrGammaControlV1, App> for GammaData {
    fn event(&self, app: &mut App, _: &ZwlrGammaControlV1, event: zwlr_gamma_control_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(gamma) = app.gammas.iter_mut().find(|g| g.output == self.0) {
                    gamma.size = size;
                    gamma.applied = None;
                }
                app.apply_night_light();
            }
            zwlr_gamma_control_v1::Event::Failed => {
                // Another program holds this screen's colours (or it can't do curves): leave it
                // be until night light is next turned off and on
                log::info!("night light: this screen's colours are someone else's");
                app.gammas.retain(|g| g.output != self.0);
                app.gamma_refused = true;
            }
            _ => {}
        }
    }
}

impl App {
    pub(super) fn add_gamma(&mut self, output: wl_output::WlOutput) {
        let Some(manager) = &self.gamma_manager else { return };
        if self.gammas.iter().any(|g| g.output == output) {
            return;
        }
        let control = manager.get_gamma_control(&output, &self.qh, GammaData(output.clone()));
        self.gammas.push(Gamma { output, control, size: 0, applied: None });
    }

    /// Evening and morning: the schedule's times, from the weather's sun or the config.
    fn night_times(&self) -> (f32, f32) {
        let n = &self.config.night_light;
        let w = &self.model.weather;
        let at = |text: &str, sun: &str, fallback: &str| {
            let text = text.trim();
            if text.eq_ignore_ascii_case("sunset") || text.eq_ignore_ascii_case("sunrise") { minutes(sun).or_else(|| minutes(fallback)) } else { minutes(text).or_else(|| minutes(fallback)) }.unwrap_or(0.0)
        };
        (at(&n.start, &w.sunset, "19:00"), at(&n.end, &w.sunrise, "07:00"))
    }

    /// How warm the screen should be right now, and whether the schedule says it's night.
    fn night_target(&mut self) -> (f32, bool) {
        let n = &self.config.night_light;
        let warm = (n.temperature as f32).clamp(1000.0, NEUTRAL);
        let now = chrono::Local::now();
        let minute = now.format("%H").to_string().parse::<f32>().unwrap_or(0.0) * 60.0 + now.format("%M").to_string().parse::<f32>().unwrap_or(0.0);
        let (start, end) = self.night_times();
        let level = match n.mode.as_str() {
            "on" => 1.0,
            "schedule" => night_level(minute, start, end, 20.0),
            _ => 0.0,
        };
        let scheduled = level > 0.5;
        // A hand on the switch holds until the schedule itself turns over
        if self.night_scheduled != Some(scheduled) {
            self.night_scheduled = Some(scheduled);
            self.night_forced = None;
        }
        let level = match self.night_forced {
            Some(true) => 1.0,
            Some(false) => 0.0,
            None => level,
        };
        (NEUTRAL - (NEUTRAL - warm) * level, scheduled)
    }

    /// Writes new curves to the screens whose warmth changed. Runs every tick; cheap when
    /// nothing moves.
    pub(super) fn apply_night_light(&mut self) {
        if self.gamma_manager.is_none() {
            self.model.night = NightStatus::default();
            return;
        }
        let (kelvin, scheduled) = self.night_target();
        let on = kelvin < NEUTRAL - 1.0;
        let (start, end) = self.night_times();
        let clock = |m: f32| format!("{:02}:{:02}", (m / 60.0) as u32, (m % 60.0) as u32);
        let detail = match (self.config.night_light.mode.as_str(), on, self.night_forced) {
            (_, true, Some(true)) | ("on", true, _) => "On".to_string(),
            ("schedule", true, None) => format!("Until {}", clock(end)),
            ("schedule", false, _) if !scheduled => format!("From {}", clock(start)),
            _ => "Off".to_string(),
        };
        self.model.night = NightStatus { available: true, on, detail };
        if !on {
            // Back to the screens' own colours: let the controls go
            for gamma in self.gammas.drain(..) {
                gamma.control.destroy();
            }
            self.gamma_refused = false;
            return;
        }
        if !self.gamma_refused {
            let outputs: Vec<_> = self.outputs.outputs().collect();
            for output in outputs {
                self.add_gamma(output);
            }
        }
        let rounded = (kelvin / 10.0).round() as u32 * 10;
        for gamma in &mut self.gammas {
            if gamma.size == 0 || gamma.applied == Some(rounded) {
                continue;
            }
            match gamma_file(gamma.size as usize, rounded as f32) {
                Ok(file) => {
                    use std::os::fd::AsFd;
                    gamma.control.set_gamma(file.as_fd());
                    gamma.applied = Some(rounded);
                }
                Err(error) => log::warn!("night light: {error}"),
            }
        }
    }

    /// The switch: whatever it is now, the other way, until the schedule next turns over.
    pub(super) fn toggle_night_light(&mut self, on: Option<bool>) {
        let now_on = self.model.night.on;
        self.night_forced = Some(on.unwrap_or(!now_on));
        self.apply_night_light();
        if let Some(index) = self.panel_index(PanelKind::Control) {
            self.panels[index].surf.dirty = true;
        }
    }
}

/// The three curves in a sealed memory file, as the protocol takes them.
fn gamma_file(size: usize, kelvin: f32) -> std::io::Result<std::fs::File> {
    let fd = rustix::fs::memfd_create("asterism-gamma", rustix::fs::MemfdFlags::CLOEXEC)?;
    let mut file = std::fs::File::from(fd);
    let bytes: Vec<u8> = ramps(size, kelvin).iter().flat_map(|v| v.to_ne_bytes()).collect();
    file.write_all(&bytes)?;
    file.rewind()?;
    Ok(file)
}

pub(super) fn bind_gamma(globals: &smithay_client_toolkit::reexports::client::globals::GlobalList, qh: &QueueHandle<App>) -> Option<ZwlrGammaControlManagerV1> {
    globals.bind::<ZwlrGammaControlManagerV1, App, Silent>(qh, 1..=1, Silent).ok()
}
