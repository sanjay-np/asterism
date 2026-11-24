//! What the bar's privacy pill says: which apps use the microphone, a camera or the screen
//! now, by readable names. The services find them (`services::privacy`, the audio service,
//! the compositor's casts); this decides what's shown of them.

use crate::config::PrivacySettings;
use crate::launcher::Entry;
use crate::model::{Cast, CastTarget, Model};

/// The senses in use now, as the settings let them show.
#[derive(Debug, Default, PartialEq)]
pub struct InUse {
    pub microphone: Vec<String>,
    pub camera: Vec<String>,
    pub screen: Vec<Cast>,
}

impl InUse {
    pub fn of(model: &Model, settings: &PrivacySettings) -> InUse {
        let names = |list: &[String]| {
            let mut out: Vec<String> = Vec::new();
            for raw in list {
                let name = app_name(&model.apps, raw);
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            out
        };
        InUse {
            microphone: if settings.microphone { names(&model.audio.listening) } else { Vec::new() },
            camera: if settings.camera { names(&model.camera) } else { Vec::new() },
            screen: if settings.screen {
                model.compositor.casts.iter().map(|c| Cast { app: if c.app.is_empty() { String::new() } else { app_name(&model.apps, &c.app) }, ..c.clone() }).collect()
            } else {
                Vec::new()
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.microphone.is_empty() && self.camera.is_empty() && self.screen.is_empty()
    }

    /// One line for the pill's tooltip: "Microphone: Firefox · Camera: Firefox · Screen: OBS".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.microphone.is_empty() {
            parts.push(format!("Microphone: {}", self.microphone.join(", ")));
        }
        if !self.camera.is_empty() {
            parts.push(format!("Camera: {}", self.camera.join(", ")));
        }
        if !self.screen.is_empty() {
            let mut apps: Vec<String> = Vec::new();
            for cast in &self.screen {
                let app = cast_app(cast);
                if !apps.contains(&app) {
                    apps.push(app);
                }
            }
            let paused = if self.screen.iter().all(|c| !c.active) { " (paused)" } else { "" };
            parts.push(format!("Screen: {}{paused}", apps.join(", ")));
        }
        parts.join(" · ")
    }
}

/// The app taking a cast, or a word for one not known.
pub fn cast_app(cast: &Cast) -> String {
    if cast.app.is_empty() { "An app".into() } else { cast.app.clone() }
}

/// What a cast takes, in a few words.
fn target_name(cast: &Cast) -> String {
    match &cast.target {
        CastTarget::Output(name) if !name.is_empty() => name.clone(),
        CastTarget::Output(_) => "The screen".into(),
        CastTarget::Window(title) if !title.is_empty() => title.clone(),
        CastTarget::Window(_) => "A window".into(),
        CastTarget::Nothing => "Nothing chosen yet".into(),
    }
}

/// What a cast takes, and whether it's paused.
pub fn cast_target(cast: &Cast) -> String {
    let what = target_name(cast);
    if cast.active { what } else { format!("{what} · paused") }
}

/// The word on a cast's stop button beside others: its app, else what it takes.
pub fn cast_stop(cast: &Cast) -> String {
    if cast.app.is_empty() { format!("Stop {}", target_name(cast)) } else { format!("Stop {}", cast.app) }
}

/// A readable name for what a process or stream calls itself: its desktop file's name when
/// there is one ("firefox" → "Firefox"), else the word with a capital ("obs" → "Obs").
pub fn app_name(apps: &[Entry], raw: &str) -> String {
    let raw = raw.trim();
    if let Some(entry) = crate::dock::entry(apps, raw) {
        return entry.name.clone();
    }
    if raw.chars().any(char::is_uppercase) {
        return raw.to_string();
    }
    crate::dock::pretty(raw)
}

/// Made-up use, for snapshots when nothing is using anything.
pub fn sample(model: &mut Model) {
    if model.audio.listening.is_empty() && model.camera.is_empty() && model.compositor.casts.is_empty() {
        model.audio.listening = vec!["Firefox".into()];
        model.camera = vec!["firefox".into()];
        model.compositor.casts = vec![
            Cast { stream: 1, session: Some(1), app: "OBS Studio".into(), target: CastTarget::Output("eDP-1".into()), active: true },
            Cast { stream: 2, session: Some(2), app: String::new(), target: CastTarget::Window("Notes".into()), active: false },
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> Entry {
        Entry { id: id.into(), name: name.into(), ..Default::default() }
    }

    #[test]
    fn names_apps_readably() {
        let apps = [entry("org.mozilla.firefox", "Firefox"), entry("com.obsproject.Studio", "OBS Studio")];
        assert_eq!(app_name(&apps, "firefox"), "Firefox", "by the desktop file");
        assert_eq!(app_name(&apps, "zoom"), "Zoom", "a capital when there's none");
        assert_eq!(app_name(&apps, "WEBRTC VoiceEngine"), "WEBRTC VoiceEngine", "a name with capitals is kept");
    }

    #[test]
    fn shows_what_the_settings_let_through() {
        let mut model = Model::default();
        sample(&mut model);
        let all = InUse::of(&model, &PrivacySettings::default());
        assert_eq!((all.microphone.len(), all.camera.len(), all.screen.len()), (1, 1, 2));
        assert_eq!(all.camera, ["Firefox"]);
        assert_eq!(all.summary(), "Microphone: Firefox · Camera: Firefox · Screen: OBS Studio, An app");
        let no_camera = InUse::of(&model, &PrivacySettings { camera: false, ..Default::default() });
        assert!(no_camera.camera.is_empty() && !no_camera.is_empty());
        let none = InUse::of(&model, &PrivacySettings { microphone: false, camera: false, screen: false });
        assert!(none.is_empty());
        // Sample data never covers something real
        model.camera = vec!["zoom".into()];
        let mut again = model.clone();
        sample(&mut again);
        assert_eq!(again.camera, ["zoom"]);
    }

    #[test]
    fn says_what_a_cast_takes() {
        let cast = |target, active| Cast { target, active, ..Default::default() };
        assert_eq!(cast_target(&cast(CastTarget::Output("HDMI-A-1".into()), true)), "HDMI-A-1");
        assert_eq!(cast_target(&cast(CastTarget::Output(String::new()), true)), "The screen");
        assert_eq!(cast_target(&cast(CastTarget::Window(String::new()), false)), "A window · paused");
        assert_eq!(cast_app(&cast(CastTarget::Nothing, true)), "An app");
        assert_eq!(cast_stop(&cast(CastTarget::Window("Notes".into()), false)), "Stop Notes", "what it takes when the app isn't known");
        assert_eq!(cast_stop(&Cast { app: "OBS".into(), ..cast(CastTarget::Nothing, true) }), "Stop OBS");
        let paused = InUse { screen: vec![cast(CastTarget::Nothing, false)], ..Default::default() };
        assert_eq!(paused.summary(), "Screen: An app (paused)");
    }
}
