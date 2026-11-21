//! What the speakers play, for the visualizer: `parec` (or PipeWire's `pw-record`) listening
//! to the default output's monitor, only while the widget shows and something plays. The
//! samples never leave this process; each frame becomes a row of levels (`crate::spectrum`).

use super::Out;
use crate::model::{Command, Update};
use crate::spectrum::{Analyzer, BARS, HOP, RATE, WINDOW};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::UnboundedReceiver;

/// The capture: mono, 16-bit, little-endian, from whatever the speakers play.
fn capture() -> Option<tokio::process::Child> {
    let rate = RATE.to_string();
    // Small fragments, so the bars keep up with the music
    let parec = super::child("parec")
        .args(["--device=@DEFAULT_MONITOR@", "--format=s16le", &format!("--rate={rate}"), "--channels=1", "--latency-msec=20", "--client-name=asterism", "--stream-name=Visualizer"])
        .stdout(std::process::Stdio::piped())
        .spawn();
    match parec {
        Ok(child) => Some(child),
        Err(_) => super::child("pw-record")
            .args(["-P", "{ stream.capture.sink=true node.name=asterism-visualizer }", "--rate", &rate, "--channels", "1", "--format", "s16", "-"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| log::warn!("visualizer: neither parec nor pw-record would start: {error}"))
            .ok(),
    }
}

async fn listen(out: Out) {
    let Some(mut child) = capture() else { return };
    let Some(mut stdout) = child.stdout.take() else { return };
    let mut analyzer = Analyzer::new(BARS);
    let mut samples: Vec<f32> = Vec::with_capacity(WINDOW + HOP);
    let mut bytes = vec![0u8; HOP * 2];
    let mut was_silent = false;
    loop {
        if stdout.read_exact(&mut bytes).await.is_err() {
            log::info!("visualizer: the capture ended");
            break;
        }
        samples.extend(bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0));
        if samples.len() > WINDOW {
            samples.drain(..samples.len() - WINDOW);
        }
        let levels = analyzer.analyze(&samples).to_vec();
        // Silence once is news; silence again isn't, so a quiet song costs no drawing
        let silent = analyzer.silent();
        if silent && was_silent {
            continue;
        }
        was_silent = silent;
        out.send(Update::Spectrum(levels));
    }
    out.send(Update::Spectrum(Vec::new()));
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    let mut task: Option<tokio::task::JoinHandle<()>> = None;
    while let Some(command) = commands.recv().await {
        let Command::WatchSpectrum(on) = command else { continue };
        let running = task.as_ref().is_some_and(|t| !t.is_finished());
        match (on, running) {
            (true, false) => task = Some(tokio::spawn(listen(out.clone()))),
            (false, true) => {
                // Dropping the task drops the child, which is killed with it
                if let Some(task) = task.take() {
                    task.abort();
                }
                out.send(Update::Spectrum(Vec::new()));
            }
            _ => {}
        }
    }
}
