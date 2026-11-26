//! The screen's brightness: read from /sys/class/backlight, set through logind (which lets the
//! logged-in user do it without root), with brightnessctl as a fallback.

use super::Out;
use crate::model::{Brightness, Command, Update};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;

/// The backlight to drive: firmware ones first (they know the panel best), then platform, then raw.
fn find() -> Option<PathBuf> {
    let mut found: Vec<(u8, PathBuf)> = std::fs::read_dir("/sys/class/backlight")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("max_brightness").exists())
        .map(|p| {
            let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
            let rank = match kind.trim() {
                "firmware" => 0,
                "platform" => 1,
                _ => 2,
            };
            (rank, p)
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, p)| p)
}

fn read(device: &std::path::Path) -> Option<(u32, u32)> {
    let number = |name: &str| std::fs::read_to_string(device.join(name)).ok()?.trim().parse::<u32>().ok();
    Some((number("brightness")?, number("max_brightness")?.max(1)))
}

async fn set(bus: Option<&Connection>, device: &std::path::Path, raw: u32) {
    let name = device.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    if let Some(bus) = bus {
        let ok = bus
            .call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1/session/auto", Some("org.freedesktop.login1.Session"), "SetBrightness", &("backlight", name.as_str(), raw))
            .await
            .is_ok();
        if ok {
            return;
        }
    }
    let _ = tokio::process::Command::new("brightnessctl").args(["-q", "-d", &name, "set", &raw.to_string()]).stdin(std::process::Stdio::null()).status().await;
}

/// How the dim fades in: steps, and the time between them. Coming back is at once.
const FADE_STEPS: u32 = 12;
const FADE_STEP: Duration = Duration::from_millis(60);

pub async fn run(out: Out, bus: Option<Connection>, mut commands: UnboundedReceiver<Command>) {
    let Some(device) = find() else {
        out.send(Update::Brightness(Brightness::default()));
        while commands.recv().await.is_some() {}
        return;
    };
    let output = std::fs::canonicalize(&device).ok().and_then(|p| crate::power::backlight_output(&p)).unwrap_or_default();
    let mut current = Brightness::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    // While dimmed: the brightness to go back to
    let mut saved: Option<u32> = None;
    // A command that came in the middle of a fade, handled next
    let mut next: Option<Command> = None;
    // Reads left that still count as dimmed, after coming back
    let mut settling = 0u8;
    loop {
        let command = match next.take() {
            Some(command) => Some(command),
            None => tokio::select! {
                _ = ticker.tick() => None,
                command = commands.recv() => match command {
                    Some(command) => Some(command),
                    None => return,
                },
            },
        };
        if let Some(command) = command
            && let Some((now, max)) = read(&device)
        {
            match command {
                Command::Dim(Some(keep)) => {
                    let back = *saved.get_or_insert(now);
                    let to = ((back as f32 * keep.clamp(0.05, 1.0)).round() as u32).clamp(1, max);
                    send_dimmed(&out, &mut current, &device, &output);
                    // Fades down (never up: a screen already darker stays so); any news (you're
                    // back) stops it
                    let steps = if to < now { FADE_STEPS } else { 0 };
                    for step in 1..=steps {
                        let raw = now as i64 + (to as i64 - now as i64) * step as i64 / FADE_STEPS as i64;
                        set(bus.as_ref(), &device, raw.max(1) as u32).await;
                        if let Ok(command) = tokio::time::timeout(FADE_STEP, commands.recv()).await {
                            match command {
                                Some(command) => next = Some(command),
                                None => return,
                            }
                            break;
                        }
                    }
                }
                Command::Dim(None) => {
                    if let Some(back) = saved.take() {
                        set(bus.as_ref(), &device, back.clamp(1, max)).await;
                        settling = 2;
                    }
                }
                Command::SetBrightness(v) => {
                    // Your own choice wins over the one put away
                    saved = None;
                    set(bus.as_ref(), &device, raw_of(v, max)).await;
                }
                Command::StepBrightness(step) => {
                    saved = None;
                    set(bus.as_ref(), &device, raw_of(now as f32 / max as f32 + step, max)).await;
                }
                _ => {}
            }
        }
        if let Some((now, max)) = read(&device) {
            // Coming back counts as dimmed for a moment too, so the OSD stays away for it
            let fresh = Brightness { available: true, value: now as f32 / max as f32, dimmed: saved.is_some() || settling > 0, output: output.clone() };
            settling = settling.saturating_sub(1);
            if fresh != current {
                current = fresh;
                out.send(Update::Brightness(current.clone()));
            }
        }
    }
}

/// Never quite black: the screen should still be there.
fn raw_of(target: f32, max: u32) -> u32 {
    ((target.clamp(0.01, 1.0) * max as f32).round() as u32).clamp(1, max)
}

/// Says it's dimming before the fade starts, so the first step shows no OSD.
fn send_dimmed(out: &Out, current: &mut Brightness, device: &std::path::Path, output: &str) {
    if current.dimmed {
        return;
    }
    let value = read(device).map(|(now, max)| now as f32 / max as f32).unwrap_or(current.value);
    *current = Brightness { available: true, value, dimmed: true, output: output.to_string() };
    out.send(Update::Brightness(current.clone()));
}
