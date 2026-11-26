//! What the shell does while you're away, and with the lid and the power button: the choices,
//! made from the settings, the battery and what logind is set to. `app/idle.rs` and
//! `app/power.rs` carry them out.

use crate::config::Config;
use crate::model::SystemPower;

/// A step taken after a while without input. Each has its own idle watch, so each comes back
/// undone (dim, off) the moment you touch something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Dim,
    Lock,
    Off,
    /// The screens off sooner while the lock is up.
    OffLocked,
    Sleep,
}

/// The steps to watch for and their wait in milliseconds, for plugged in or on battery. A step
/// at 0 minutes is left out.
pub fn steps(config: &Config, on_battery: bool) -> Vec<(Step, u32)> {
    let p = &config.power;
    let minutes = if on_battery {
        [(Step::Dim, p.dim_battery), (Step::Lock, config.lock.idle_battery), (Step::Off, p.off_battery), (Step::OffLocked, p.off_locked), (Step::Sleep, p.suspend_battery)]
    } else {
        [(Step::Dim, p.dim), (Step::Lock, config.lock.idle), (Step::Off, p.off), (Step::OffLocked, p.off_locked), (Step::Sleep, p.suspend)]
    };
    minutes.into_iter().filter(|(_, m)| *m > 0).map(|(step, m)| (step, m.saturating_mul(60_000))).collect()
}

/// What the lid or the power button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Left to logind: the shell doesn't hold the key.
    System,
    /// The session menu (lock, log out, restart, shut down).
    Menu,
    Sleep,
    Hibernate,
    PowerOff,
    Lock,
    Nothing,
}

impl Act {
    /// The config's word (unknown ones leave it to the system).
    pub fn of(word: &str) -> Act {
        match word.trim().to_ascii_lowercase().as_str() {
            "menu" | "session" => Act::Menu,
            "suspend" | "sleep" => Act::Sleep,
            "hibernate" => Act::Hibernate,
            "poweroff" | "power-off" | "shutdown" => Act::PowerOff,
            "lock" => Act::Lock,
            "nothing" | "ignore" => Act::Nothing,
            _ => Act::System,
        }
    }

    /// What logind does for one of its words (HandleLidSwitch=…), as near as the shell can do it.
    pub fn of_logind(word: &str) -> Act {
        match word.trim() {
            "suspend" | "suspend-then-hibernate" | "hybrid-sleep" | "sleep" => Act::Sleep,
            "hibernate" => Act::Hibernate,
            "poweroff" | "halt" | "reboot" | "kexec" => Act::PowerOff,
            "lock" => Act::Lock,
            _ => Act::Nothing,
        }
    }

    /// A short name, for Settings.
    pub fn name(self) -> &'static str {
        match self {
            Act::System => "As the system does",
            Act::Menu => "Session menu",
            Act::Sleep => "Sleep",
            Act::Hibernate => "Hibernate",
            Act::PowerOff => "Shut down",
            Act::Lock => "Lock",
            Act::Nothing => "Nothing",
        }
    }
}

/// The lid's setting that applies now: with another screen, else plugged in ("same" or empty:
/// as on battery), else on battery.
pub fn lid_word(config: &Config, on_battery: bool, docked: bool) -> &str {
    let p = &config.power;
    if docked {
        &p.lid_docked
    } else if !on_battery && !matches!(p.lid_plugged.trim(), "" | "same") {
        &p.lid_plugged
    } else {
        &p.lid
    }
}

/// What closing the lid does now. "system" is logind's own choice for the case, which the shell
/// does itself when it holds the lid for another case.
pub fn lid_act(config: &Config, system: &SystemPower, on_battery: bool, docked: bool) -> Act {
    match Act::of(lid_word(config, on_battery, docked)) {
        Act::System => {
            let word = if docked {
                &system.lid_docked
            } else if !on_battery && !system.lid_plugged.is_empty() {
                &system.lid_plugged
            } else {
                &system.lid
            };
            Act::of_logind(word)
        }
        // The session menu would open on a closed screen
        Act::Menu => Act::Lock,
        act => act,
    }
}

/// Whether the shell holds the lid from logind: when any case is set to something of its own.
pub fn takes_lid(config: &Config) -> bool {
    let p = &config.power;
    let plugged = if matches!(p.lid_plugged.trim(), "" | "same") { &p.lid } else { &p.lid_plugged };
    [&p.lid, plugged, &p.lid_docked].into_iter().any(|w| Act::of(w) != Act::System)
}

/// Whether the shell holds the power button from logind.
pub fn takes_button(config: &Config) -> bool {
    Act::of(&config.power.button) != Act::System
}

/// The backlit screen's connector, from the backlight's place in sysfs
/// (…/drm/card1/card1-eDP-1/amdgpu_bl1 → eDP-1).
pub fn backlight_output(path: &std::path::Path) -> Option<String> {
    path.components().rev().filter_map(|c| c.as_os_str().to_str()).find_map(|part| {
        let rest = part.strip_prefix("card")?;
        let (number, connector) = rest.split_once('-')?;
        (number.chars().all(|c| c.is_ascii_digit()) && !connector.is_empty()).then(|| connector.to_string())
    })
}

/// Whether an output has the backlight: its connector when known, else the usual names of a
/// laptop's own panel.
pub fn backlit(output: &str, backlight: &str) -> bool {
    if !backlight.is_empty() {
        return output == backlight;
    }
    ["eDP", "LVDS", "DSI"].iter().any(|p| output.starts_with(p))
}

/// How dark the veil over a screen with no backlight is, for a dim level in percent: about as
/// dark as the backlit one looks, never black.
pub fn veil_alpha(level: u32) -> f32 {
    (1.0 - level.min(100) as f32 / 100.0) * 0.85
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> SystemPower {
        SystemPower { lid: "suspend".into(), lid_plugged: String::new(), lid_docked: "ignore".into(), button: "poweroff".into(), hibernate: false }
    }

    #[test]
    fn steps_follow_the_battery_and_leave_out_zeroes() {
        let mut c = Config::default();
        let plugged = steps(&c, false);
        assert_eq!(plugged, [(Step::Dim, 300_000), (Step::Lock, 600_000), (Step::Off, 900_000), (Step::OffLocked, 60_000), (Step::Sleep, 1_800_000)]);
        let battery = steps(&c, true);
        assert_eq!(battery[0], (Step::Dim, 120_000));
        assert_eq!(battery[1], (Step::Lock, 300_000));
        c.power.suspend = 0;
        c.lock.idle = 0;
        let kinds: Vec<Step> = steps(&c, false).into_iter().map(|(s, _)| s).collect();
        assert_eq!(kinds, [Step::Dim, Step::Off, Step::OffLocked]);
    }

    #[test]
    fn the_lid_takes_the_setting_for_the_case() {
        let mut c = Config::default();
        assert!(!takes_lid(&c));
        // All "system": logind's own words
        assert_eq!(lid_act(&c, &system(), true, false), Act::Sleep);
        assert_eq!(lid_act(&c, &system(), false, false), Act::Sleep);
        assert_eq!(lid_act(&c, &system(), false, true), Act::Nothing);
        c.power.lid = "lock".into();
        assert!(takes_lid(&c));
        // Plugged in is "same": as on battery
        assert_eq!(lid_act(&c, &system(), false, false), Act::Lock);
        c.power.lid_plugged = "nothing".into();
        assert_eq!(lid_act(&c, &system(), false, false), Act::Nothing);
        assert_eq!(lid_act(&c, &system(), true, false), Act::Lock);
        // Docked stays logind's
        assert_eq!(lid_act(&c, &system(), true, true), Act::Nothing);
        c.power.lid_docked = "menu".into();
        assert_eq!(lid_act(&c, &system(), true, true), Act::Lock);
    }

    #[test]
    fn the_button_is_held_only_for_a_choice_of_its_own() {
        let mut c = Config::default();
        assert!(!takes_button(&c));
        c.power.button = "menu".into();
        assert!(takes_button(&c));
        assert_eq!(Act::of("poweroff"), Act::PowerOff);
        assert_eq!(Act::of("whatever"), Act::System);
    }

    #[test]
    fn finds_the_backlit_screen() {
        let path = std::path::Path::new("/sys/devices/pci0000:00/0000:00:08.1/0000:03:00.0/drm/card1/card1-eDP-1/amdgpu_bl1");
        assert_eq!(backlight_output(path).as_deref(), Some("eDP-1"));
        assert_eq!(backlight_output(std::path::Path::new("/sys/devices/platform/acpi_video0")), None);
        assert!(backlit("eDP-1", "eDP-1"));
        assert!(!backlit("HDMI-A-1", "eDP-1"));
        assert!(backlit("eDP-1", ""));
        assert!(!backlit("DP-2", ""));
        assert!(veil_alpha(30) > 0.5 && veil_alpha(30) < 0.7);
        assert!(veil_alpha(0) < 1.0);
    }
}
