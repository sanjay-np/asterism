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

pub async fn run(out: Out, bus: Option<Connection>, mut commands: UnboundedReceiver<Command>) {
    let Some(device) = find() else {
        out.send(Update::Brightness(Brightness::default()));
        while commands.recv().await.is_some() {}
        return;
    };
    let mut current = Brightness::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            Some(command) = commands.recv() => {
                let Some((now, max)) = read(&device) else { continue };
                let value = now as f32 / max as f32;
                let target = match command {
                    Command::SetBrightness(v) => v,
                    Command::StepBrightness(step) => value + step,
                    _ => continue,
                };
                // Never quite black: the screen should still be there
                let raw = ((target.clamp(0.01, 1.0) * max as f32).round() as u32).clamp(1, max);
                set(bus.as_ref(), &device, raw).await;
            }
        }
        if let Some((now, max)) = read(&device) {
            let fresh = Brightness { available: true, value: now as f32 / max as f32 };
            if fresh != current {
                current = fresh;
                out.send(Update::Brightness(current.clone()));
            }
        }
    }
}
