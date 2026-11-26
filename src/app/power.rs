//! Power, as the app does it: the dim (the backlight through the brightness service, and a dark
//! veil over screens that have none), the screens off and on, and what the lid and the power
//! button do while the shell holds them (`crate::power` says what).

use super::*;
use crate::power::Act;

/// A see-through dark layer over one screen with no backlight, faded in. A 1-pixel buffer
/// stretched by the viewporter; it takes no input, so the first touch goes where it's aimed.
pub(super) struct Veil {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub from: Instant,
    pub alpha: f32,
}

/// How long the veil takes to come, and how often it's redrawn meanwhile.
const VEIL_FADE: Duration = Duration::from_millis(720);
const VEIL_STEP: Duration = Duration::from_millis(40);

impl App {
    /// Dims every screen (the backlit one by its backlight, the others by a veil), or puts them back.
    pub(super) fn dim(&mut self, on: bool) {
        if on == self.dimmed {
            return;
        }
        self.dimmed = on;
        let level = self.config.power.dim_level.clamp(5, 100);
        // The backlight only when its screen is one of ours (a shell nested in another
        // compositor has none, and mustn't dim the screen it runs on)
        let names: Vec<String> = self.outputs.outputs().filter_map(|o| self.outputs.info(&o).and_then(|i| i.name)).collect();
        let backlight = (self.model.brightness.available).then(|| self.model.brightness.output.clone()).filter(|b| names.iter().any(|n| crate::power::backlit(n, b)));
        if backlight.is_some() {
            self.send(Command::Dim(on.then_some(level as f32 / 100.0)));
        }
        if !on {
            self.veils.clear();
            return;
        }
        let outputs: Vec<_> = self.outputs.outputs().collect();
        for output in outputs {
            let name = self.outputs.info(&output).and_then(|i| i.name).unwrap_or_default();
            if backlight.as_deref().is_some_and(|b| crate::power::backlit(&name, b)) {
                continue;
            }
            self.add_veil(output);
        }
        if !self.veils.is_empty() {
            self.step_veils();
        }
    }

    fn add_veil(&mut self, output: wl_output::WlOutput) {
        let wl = self.compositor.create_surface(&self.qh);
        let surf = self.new_surf(wl.clone());
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl.clone(), Layer::Overlay, Some("asterism-veil"), Some(&output));
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        if let Ok(region) = Region::new(&self.compositor) {
            wl.set_input_region(Some(region.wl_region()));
        }
        layer.commit();
        self.veils.push(Veil { surf, layer, from: Instant::now(), alpha: -1.0 });
    }

    /// Draws each veil at its place in the fade, and comes back until they're all there.
    fn step_veils(&mut self) {
        let full = crate::power::veil_alpha(self.config.power.dim_level);
        let mut fading = false;
        for veil in &mut self.veils {
            let k = (veil.from.elapsed().as_secs_f32() / VEIL_FADE.as_secs_f32()).min(1.0);
            fading |= k < 1.0;
            let alpha = full * crate::transition::ease(k);
            if !veil.surf.configured || (alpha - veil.alpha).abs() < 0.004 {
                continue;
            }
            if desk::present_fill(&mut veil.surf, crate::theme::Rgba { r: 0.0, g: 0.0, b: 0.0, a: alpha }, &mut self.pool) {
                veil.alpha = alpha;
            }
        }
        if fading || self.veils.iter().any(|v| !v.surf.configured) {
            let _ = self.handle.insert_source(Timer::from_duration(VEIL_STEP), |_, _, app| {
                if !app.veils.is_empty() {
                    app.step_veils();
                }
                TimeoutAction::Drop
            });
        }
    }

    pub(super) fn veil_configured(&mut self, index: usize, w: u32, h: u32) {
        let veil = &mut self.veils[index];
        veil.surf.w = w.max(1) as f32;
        veil.surf.h = h.max(1) as f32;
        veil.surf.configured = true;
        veil.alpha = -1.0;
        self.step_veils();
    }

    /// The screens off (DPMS), or on again.
    pub(super) fn screens(&mut self, on: bool) {
        if on == !self.screens_off {
            return;
        }
        self.screens_off = !on;
        self.send(Command::Monitors(on));
    }

    /// Tells the logind service which keys to hold, when that changes.
    pub(super) fn sync_power_keys(&mut self) {
        let keys = (crate::power::takes_lid(&self.config), crate::power::takes_button(&self.config));
        if self.power_keys != Some(keys) {
            self.power_keys = Some(keys);
            self.send(Command::TakePowerKeys { lid: keys.0, button: keys.1 });
        }
    }

    /// Carries out the lid's or the button's choice.
    fn power_act(&mut self, act: Act) {
        log::info!("power: {act:?}");
        match act {
            Act::Sleep => self.send(Command::Suspend),
            Act::Hibernate => self.send(Command::Hibernate),
            Act::PowerOff => self.send(Command::PowerOff),
            Act::Lock => self.start_lock(),
            Act::Menu => self.menu_from_keys("session"),
            Act::System | Act::Nothing => {}
        }
    }

    /// The lid closed or opened while the shell holds it.
    pub(super) fn lid(&mut self, closed: bool, docked: bool) {
        if !closed {
            return;
        }
        let act = crate::power::lid_act(&self.config, &self.model.system_power, self.model.battery.on_battery, docked);
        self.power_act(act);
    }

    /// The power button (the compositor runs `asterism msg power button` for it).
    pub(super) fn power_button(&mut self) {
        let act = match Act::of(&self.config.power.button) {
            // Left to logind: it has already done its part
            Act::System => return,
            // The menu can't show over the lock: the screens off instead
            Act::Menu if crate::lock::running() => return self.screens(false),
            act => act,
        };
        self.power_act(act);
    }
}
