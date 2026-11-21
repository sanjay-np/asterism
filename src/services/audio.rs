//! Sound, through `pactl`, which talks to PipeWire's pulse server: `pactl subscribe` says when
//! something changes, the JSON listings say what things are now.

use super::{Out, output};
use crate::model::{Audio, AudioDevice, AudioStream, Command, Update};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::UnboundedReceiver;

async fn json(args: &[&str]) -> Value {
    let mut all = vec!["-f", "json"];
    all.extend_from_slice(args);
    output("pactl", &all).await.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null)
}

/// The average of a device's channels, 0 to 1.5.
fn volume(device: &Value) -> f32 {
    let Some(channels) = device.get("volume").and_then(Value::as_object) else { return 0.0 };
    let values: Vec<f32> = channels
        .values()
        .filter_map(|c| c.get("value_percent").and_then(Value::as_str))
        .filter_map(|p| p.trim_end_matches('%').trim().parse::<f32>().ok())
        .collect();
    if values.is_empty() { 0.0 } else { values.iter().sum::<f32>() / values.len() as f32 / 100.0 }
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn property(value: &Value, key: &str) -> String {
    value.get("properties").map(|p| text(p, key)).unwrap_or_default()
}

async fn read() -> Audio {
    let info = json(&["info"]).await;
    if info.is_null() {
        return Audio::default();
    }
    let default_sink = text(&info, "default_sink_name");
    let default_source = text(&info, "default_source_name");
    let sinks = json(&["list", "sinks"]).await;
    let sources = json(&["list", "sources"]).await;
    let outputs = json(&["list", "source-outputs"]).await;
    let inputs = json(&["list", "sink-inputs"]).await;

    let mut audio = Audio { available: true, sink: default_sink.clone(), ..Default::default() };
    for sink in sinks.as_array().into_iter().flatten() {
        let name = text(sink, "name");
        if name == default_sink {
            audio.volume = volume(sink);
            audio.muted = sink.get("mute").and_then(Value::as_bool).unwrap_or(false);
        }
        audio.sinks.push(AudioDevice { name, description: text(sink, "description") });
    }

    let sources = sources.as_array().cloned().unwrap_or_default();
    if let Some(mic) = sources.iter().find(|s| text(s, "name") == default_source) {
        audio.has_mic = !text(mic, "name").ends_with(".monitor");
        audio.mic_muted = mic.get("mute").and_then(Value::as_bool).unwrap_or(false);
        audio.mic_volume = volume(mic);
    }

    // Someone is listening to a real microphone. Captures of what the speakers play (a
    // visualiser, a recording with the computer's sound) don't count, nor paused ones.
    let is_monitor = |index: u64| {
        sources.iter().find(|s| s.get("index").and_then(Value::as_u64) == Some(index)).is_none_or(|s| {
            text(s, "name").ends_with(".monitor") || s.get("monitor_of_sink").and_then(Value::as_u64).is_some()
        })
    };
    for stream in outputs.as_array().into_iter().flatten() {
        let corked = stream.get("corked").and_then(Value::as_bool).unwrap_or(false);
        let source = stream.get("source").and_then(Value::as_u64).unwrap_or(u64::MAX);
        if corked || is_monitor(source) || property(stream, "stream.capture.sink") == "true" {
            continue;
        }
        let app = property(stream, "application.name");
        audio.listening.push(if app.is_empty() { "Something".into() } else { app });
    }

    for input in inputs.as_array().into_iter().flatten() {
        let mut app = property(input, "application.name");
        if app.is_empty() {
            app = property(input, "media.name");
        }
        audio.streams.push(AudioStream {
            index: input.get("index").and_then(Value::as_u64).unwrap_or(0) as u32,
            app,
            volume: volume(input),
            muted: input.get("mute").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    audio
}

async fn pactl(args: &[&str]) {
    let _ = tokio::process::Command::new("pactl").args(args).stdin(std::process::Stdio::null()).status().await;
}

async fn command(command: Command, current: &Audio) {
    match command {
        Command::SetVolume(v) => pactl(&["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", (v.clamp(0.0, 1.5) * 100.0).round())]).await,
        Command::StepVolume(step) => {
            let target = (current.volume + step).clamp(0.0, 1.5);
            pactl(&["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", (target * 100.0).round())]).await
        }
        Command::ToggleMute => pactl(&["set-sink-mute", "@DEFAULT_SINK@", "toggle"]).await,
        Command::SetDefaultSink(name) => pactl(&["set-default-sink", &name]).await,
        Command::ToggleMicMute => pactl(&["set-source-mute", "@DEFAULT_SOURCE@", "toggle"]).await,
        Command::SetStreamVolume(index, v) => {
            pactl(&["set-sink-input-volume", &index.to_string(), &format!("{}%", (v.clamp(0.0, 1.5) * 100.0).round())]).await
        }
        _ => {}
    }
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    let mut current = read().await;
    out.send(Update::Audio(current.clone()));
    loop {
        let child = super::child("pactl").arg("subscribe").stdout(std::process::Stdio::piped()).spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                log::warn!("pactl: {error}; no sound in the bar");
                // Still take commands, so nothing queues up
                while commands.recv().await.is_some() {}
                return;
            }
        };
        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut pending = false;
        loop {
            let settle = tokio::time::sleep(Duration::from_millis(if pending { 40 } else { 3_600_000 }));
            tokio::select! {
                line = lines.next_line() => match line {
                    // "Event 'change' on sink #55"
                    Ok(Some(line)) => {
                        if ["sink", "source", "server", "source-output", "sink-input"].iter().any(|k| line.contains(k)) {
                            pending = true;
                        }
                    }
                    _ => break,
                },
                Some(c) = commands.recv() => command(c, &current).await,
                _ = settle, if pending => {
                    pending = false;
                    let now = read().await;
                    if now != current {
                        current = now;
                        out.send(Update::Audio(current.clone()));
                    }
                }
            }
        }
        let _ = child.kill().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        current = read().await;
        out.send(Update::Audio(current.clone()));
    }
}
