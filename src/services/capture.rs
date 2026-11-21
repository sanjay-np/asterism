//! Screenshots and screen recording. Screenshots go through the compositor where it has its own
//! (niri: region, screen, window, saved and copied by niri), else grim and slurp; either way
//! the new picture is then shown in a notification. Recording runs wf-recorder until stopped.

use super::Out;
use crate::model::{Capture, Command, Recording, Update};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::UnboundedReceiver;

/// Where niri puts screenshots: its `screenshot-path`'s folder, or its default. `None` when
/// it's set to `null` (niri then only copies them).
fn niri_folder() -> Option<PathBuf> {
    let home = crate::config::home();
    let config = std::fs::read_to_string(home.join(".config/niri/config.kdl")).unwrap_or_default();
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with("//") || !line.starts_with("screenshot-path") {
            continue;
        }
        let value = line.trim_start_matches("screenshot-path").trim();
        if value == "null" {
            return None;
        }
        let path = value.trim_matches('"');
        return crate::config::expand_home(path).parent().map(Path::to_path_buf);
    }
    Some(home.join("Pictures/Screenshots"))
}

/// The newest picture in a folder made after `since`, waiting up to `patience` for it.
async fn new_picture(folder: &Path, since: SystemTime, patience: Duration) -> Option<PathBuf> {
    let until = tokio::time::Instant::now() + patience;
    while tokio::time::Instant::now() < until {
        let newest = std::fs::read_dir(folder).ok().and_then(|entries| {
            entries
                .flatten()
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .filter(|(time, path)| *time > since && path.extension().is_some_and(|x| x == "png"))
                .max_by_key(|(time, _)| *time)
                .map(|(_, path)| path)
        });
        if newest.is_some() {
            // Give the writer a moment to finish the file
            tokio::time::sleep(Duration::from_millis(300)).await;
            return newest;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    None
}

fn note(summary: &str, body: String, picture: &Path) -> crate::model::Note {
    crate::model::Note { summary: summary.into(), body, icon: picture.display().to_string(), urgency: 0 }
}

async fn screenshot(out: &Out, kind: Capture) {
    let niri = crate::services::detect_compositor() == crate::model::CompositorKind::Niri;
    let started = SystemTime::now();
    if niri {
        let action = match kind {
            Capture::Region => "screenshot",
            Capture::Screen => "screenshot-screen",
            Capture::Window => "screenshot-window",
        };
        let _ = super::output("niri", &["msg", "action", action]).await;
        let Some(folder) = niri_folder() else { return };
        // The region one waits for you to choose; the others are done at once
        let patience = if kind == Capture::Region { Duration::from_secs(120) } else { Duration::from_secs(5) };
        if let Some(path) = new_picture(&folder, started, patience).await {
            out.send(Update::Notify(note("Screenshot saved", format!("Copied, and kept in {}", folder.display()), &path)));
        }
        return;
    }
    // Elsewhere: grim, and slurp to choose a region
    if !crate::config::command_available("grim") {
        out.send(Update::Notify(crate::model::Note { summary: "Can't take screenshots".into(), body: "Install grim (and slurp for a region).".into(), ..Default::default() }));
        return;
    }
    let folder = crate::config::home().join("Pictures/Screenshots");
    let _ = std::fs::create_dir_all(&folder);
    let path = folder.join(chrono::Local::now().format("Screenshot from %Y-%m-%d %H-%M-%S.png").to_string());
    let mut grim = tokio::process::Command::new("grim");
    if kind == Capture::Region {
        let Some(region) = super::output("slurp", &[]).await.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()) else { return };
        grim.args(["-g", &region]);
    }
    if grim.arg(&path).status().await.is_ok_and(|s| s.success()) {
        let _ = tokio::process::Command::new("sh").arg("-c").arg(format!("wl-copy --type image/png < '{}'", path.display().to_string().replace('\'', "'\\''"))).status().await;
        out.send(Update::Notify(note("Screenshot saved", format!("Copied, and kept in {}", folder.display()), &path)));
    }
}

/// The tools reading text off the screen needs: a region chosen (slurp), captured (grim), read
/// (tesseract), copied (wl-copy).
pub const TEXT_TOOLS: [&str; 4] = ["slurp", "grim", "tesseract", "wl-copy"];

/// Reads the text in a region of the screen you choose and copies it; a notification says what
/// it found (or what's missing). The picture never touches the disk: grim writes it to
/// tesseract through a pipe.
async fn screen_text(out: &Out) {
    let missing: Vec<&str> = TEXT_TOOLS.iter().copied().filter(|t| !crate::config::command_available(t)).collect();
    if !missing.is_empty() {
        out.send(Update::Notify(crate::model::Note { summary: "Can't read text off the screen".into(), body: format!("Install {}.", missing.join(", ")), ..Default::default() }));
        return;
    }
    // Esc in slurp: nothing chosen, nothing done
    let Some(region) = super::output("slurp", &[]).await else { return };
    let region = region.trim().to_string();
    if region.is_empty() {
        return;
    }
    let script = "grim -g \"$1\" - | tesseract stdin stdout 2>/dev/null";
    let Some(text) = super::output("sh", &["-c", script, "sh", &region]).await else { return };
    let text = text.trim().to_string();
    if text.is_empty() {
        out.send(Update::Notify(crate::model::Note { summary: "No text found there".into(), ..Default::default() }));
        return;
    }
    let copied = super::child("wl-copy").stdin(std::process::Stdio::piped()).spawn();
    if let Ok(mut child) = copied {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(text.as_bytes()).await;
        }
        let _ = child.wait().await;
    }
    let words = text.split_whitespace().count();
    let preview: String = text.chars().take(120).collect();
    out.send(Update::Notify(crate::model::Note { summary: format!("Copied {words} word{}", if words == 1 { "" } else { "s" }), body: preview, icon: "edit-copy".into(), urgency: 0 }));
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    let mut recorder: Option<(tokio::process::Child, PathBuf)> = None;
    while let Some(command) = commands.recv().await {
        match command {
            Command::ScreenText => {
                let out = out.clone();
                tokio::spawn(async move { screen_text(&out).await });
            }
            Command::Screenshot(kind) => {
                let out = out.clone();
                tokio::spawn(async move { screenshot(&out, kind).await });
            }
            Command::RecordStart { audio } => {
                if recorder.is_some() {
                    continue;
                }
                if !crate::config::command_available("wf-recorder") {
                    out.send(Update::Notify(crate::model::Note { summary: "Can't record the screen".into(), body: "Install wf-recorder to record.".into(), ..Default::default() }));
                    continue;
                }
                let folder = crate::config::home().join("Videos/Recordings");
                let _ = std::fs::create_dir_all(&folder);
                let path = folder.join(chrono::Local::now().format("Recording %Y-%m-%d %H-%M-%S.mp4").to_string());
                let mut wf = super::child("wf-recorder");
                wf.arg("-f").arg(&path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if audio {
                    wf.arg("--audio");
                }
                match wf.spawn() {
                    Ok(child) => {
                        out.send(Update::Recording(Some(Recording { since: chrono::Local::now(), audio, path: path.clone() })));
                        recorder = Some((child, path));
                    }
                    Err(error) => log::warn!("wf-recorder: {error}"),
                }
            }
            Command::RecordStop => {
                if let Some((mut child, path)) = recorder.take() {
                    // SIGINT lets it finish the file properly
                    if let Some(pid) = child.id().and_then(|p| rustix::process::Pid::from_raw(p as i32)) {
                        let _ = rustix::process::kill_process(pid, rustix::process::Signal::INT);
                    }
                    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                    let _ = child.kill().await;
                    out.send(Update::Recording(None));
                    out.send(Update::Notify(crate::model::Note { summary: "Recording saved".into(), body: path.display().to_string(), icon: "video-x-generic".into(), urgency: 0 }));
                }
            }
            _ => {}
        }
    }
}
