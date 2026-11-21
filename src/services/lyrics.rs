//! Song lyrics, line by line with their times, from LRCLIB (no account, no key). Only the song
//! is sent: its artist, title, album and length. Asked once per song; "none" is remembered too.

use super::Out;
use crate::model::{Command, Update};
use tokio::sync::mpsc::UnboundedReceiver;

fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn fetch(artist: &str, title: &str, album: &str, seconds: u32) -> Vec<(f64, String)> {
    let mut url = format!("https://lrclib.net/api/get?artist_name={}&track_name={}", encode(artist), encode(title));
    if !album.is_empty() {
        url.push_str(&format!("&album_name={}", encode(album)));
    }
    if seconds > 0 {
        url.push_str(&format!("&duration={seconds}"));
    }
    let Some(text) = super::output("curl", &["-sSf", "-m", "15", "-A", "asterism", &url]).await else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    json.get("syncedLyrics").and_then(|v| v.as_str()).map(crate::widgets::parse_lyrics).unwrap_or_default()
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    while let Some(command) = commands.recv().await {
        if let Command::FetchLyrics { artist, title, album, seconds } = command {
            let out = out.clone();
            tokio::spawn(async move {
                let lines = fetch(&artist, &title, &album, seconds).await;
                out.send(Update::Lyrics(crate::widgets::lyrics_key(&artist, &title), lines));
            });
        }
    }
}
