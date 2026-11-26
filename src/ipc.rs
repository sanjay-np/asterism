//! `asterism msg …`: a small socket for keybinds and scripts to drive the running shell, and
//! the reason a second copy refuses to start.

use crate::model::{Capture, Request, TimerAction, Update, WallpaperRequest};
use crate::services::Out;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

pub const USAGE: &str = "\
asterism msg COMMAND

    launcher [open|close]          Toggle the app launcher
    control [open|close]           Toggle the control centre
    menu NAME                      Toggle a menu: calendar, media, sound, network, bluetooth,
                                   power, session, machine, traffic, keyboard, notifications,
                                   privacy
    notifications [clear]          Toggle the notification centre, or clear it
    dnd                            Toggle do not disturb
    volume up|down [PERCENT]       Change the volume (default 5) and show it
    volume mute                    Mute or unmute
    mic mute                       Mute or unmute the microphone
    brightness up|down [PERCENT]   Change the brightness (default 5) and show it
    osd volume|brightness          Show the level without changing it
    media play-pause|next|previous The player the bar follows
    awake                          Toggle staying awake
    wallpaper [open|close]         Toggle the wallpaper changer
    wallpaper next|previous|random Change to another picture from the folder
    wallpaper set PATH             Change to this picture
    night-light [on|off]           Turn night light the other way, or on, or off
    shortcuts [open|close]         Toggle the keyboard shortcuts sheet
    clipboard [open|close]         Toggle the clipboard history
    lock                           Lock the screen
    power button                   What the power button does ([power] button; bind XF86PowerOff to it)
    dashboard [open|close]         Toggle the dashboard (graphs, disks, processes)
    settings [open|close]          Toggle the settings app
    day [open|close]               Toggle the day panel (calendar, timers, to-do, notes)
    dock [show|hide]               Bring the dock up for a moment, or put it away
    bar arrange [on|off]           Drag the bar's items about (Done or this again to finish)
    overview [open|close]          Toggle the window overview
    widgets [edit|done]            Arrange the desktop's widgets (add, remove, move), or finish
    pet feed|play|yarn             Put down food, a ball or yarn for the pets
    pet bed|nobed                  Give them a bed, or take it away
    pet add [ANIMAL[:COAT]]        Another pet (cat, fox, bird, panda, tiger, lion), as many as you like
    pet [N|friend] next|away       Another animal for the first pet (or the Nth, or the second), or put it away
    screenshot [region|screen|window]  Take a screenshot (a region you choose, by default)
    screen-text                    Copy the text in a region you choose (grim, slurp, tesseract)
    record [audio]                 Start recording the screen (with sound), or stop
    timer MINUTES                  Start a countdown
    timer pause|cancel             Pause (or go on with) the countdown, or cancel it
    focus                          Start the focus timer, or stop it
    stopwatch [lap|reset]          Start or stop the stopwatch, take a lap, or reset it
    reload                         Read the config again
    ping                           Answers pong when the shell runs
";

pub fn socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    runtime.join(format!("asterism-{display}.sock"))
}

/// What a line of words asks for.
pub fn parse(words: &[&str]) -> Result<Request, String> {
    let step = |w: Option<&&str>| -> Result<f32, String> {
        match w {
            None => Ok(0.05),
            Some(w) => w.trim_end_matches('%').parse::<f32>().map(|v| v / 100.0).map_err(|_| format!("not a percentage: {w}")),
        }
    };
    Ok(match words {
        ["launcher"] => Request::Launcher(None),
        ["launcher", "open"] => Request::Launcher(Some(true)),
        ["launcher", "close"] => Request::Launcher(Some(false)),
        ["control"] => Request::Control(None),
        ["control", "open"] => Request::Control(Some(true)),
        ["control", "close"] => Request::Control(Some(false)),
        ["menu", name] => Request::Menu(name.to_string()),
        ["notifications"] => Request::Notifications,
        ["notifications", "clear"] => Request::ClearNotifications,
        ["dnd"] | ["quiet"] => Request::Quiet,
        ["volume", "mute"] => Request::Mute,
        ["volume", "up", rest @ ..] => Request::Volume(step(rest.first())?),
        ["volume", "down", rest @ ..] => Request::Volume(-step(rest.first())?),
        ["mic", "mute"] => Request::MicMute,
        ["brightness", "up", rest @ ..] => Request::Brightness(step(rest.first())?),
        ["brightness", "down", rest @ ..] => Request::Brightness(-step(rest.first())?),
        ["osd", what @ ("volume" | "brightness")] => Request::Osd(what.to_string()),
        ["media", what @ ("play-pause" | "next" | "previous")] => Request::Media(what.to_string()),
        ["awake"] => Request::Awake,
        ["lock"] => Request::Lock,
        ["power", "button"] => Request::PowerButton,
        ["overview"] => Request::Overview(None),
        ["overview", "open"] => Request::Overview(Some(true)),
        ["overview", "close"] => Request::Overview(Some(false)),
        ["day"] => Request::Day(None),
        ["day", "open"] => Request::Day(Some(true)),
        ["day", "close"] => Request::Day(Some(false)),
        ["bar", "arrange"] => Request::ArrangeBar(None),
        ["bar", "arrange", "on"] => Request::ArrangeBar(Some(true)),
        ["bar", "arrange", "off"] => Request::ArrangeBar(Some(false)),
        ["dock"] => Request::Dock(None),
        ["dock", "show"] => Request::Dock(Some(true)),
        ["dock", "hide"] => Request::Dock(Some(false)),
        ["settings"] => Request::Settings(None),
        ["settings", "open"] => Request::Settings(Some(true)),
        ["settings", "close"] => Request::Settings(Some(false)),
        ["pet", what @ ("feed" | "play" | "yarn" | "bed" | "nobed" | "next" | "away")] => Request::Pet(what.to_string()),
        ["pet", "friend", what @ ("next" | "away")] => Request::Pet(format!("2 {what}")),
        ["pet", "add"] => Request::Pet("add".into()),
        ["pet", "add", animal] => Request::Pet(format!("add {animal}")),
        ["pet", n, what @ ("next" | "away")] if n.parse::<usize>().is_ok_and(|n| n > 0) => Request::Pet(format!("{n} {what}")),
        ["widgets"] => Request::Widgets(None),
        ["widgets", "edit"] => Request::Widgets(Some(true)),
        ["widgets", "done"] => Request::Widgets(Some(false)),
        ["dashboard"] => Request::Dashboard(None),
        ["dashboard", "open"] => Request::Dashboard(Some(true)),
        ["dashboard", "close"] => Request::Dashboard(Some(false)),
        ["screenshot"] | ["screenshot", "region"] => Request::Screenshot(Capture::Region),
        ["screenshot", "screen"] => Request::Screenshot(Capture::Screen),
        ["screenshot", "window"] => Request::Screenshot(Capture::Window),
        ["screen-text"] => Request::ScreenText,
        ["record"] => Request::Record { audio: false },
        ["record", "audio"] => Request::Record { audio: true },
        ["clipboard"] => Request::Clipboard(None),
        ["clipboard", "open"] => Request::Clipboard(Some(true)),
        ["clipboard", "close"] => Request::Clipboard(Some(false)),
        ["shortcuts"] => Request::Shortcuts(None),
        ["shortcuts", "open"] => Request::Shortcuts(Some(true)),
        ["shortcuts", "close"] => Request::Shortcuts(Some(false)),
        ["night-light"] => Request::NightLight(None),
        ["night-light", "on"] => Request::NightLight(Some(true)),
        ["night-light", "off"] => Request::NightLight(Some(false)),
        ["timer", "pause"] => Request::Timer(TimerAction::CountdownPause),
        ["timer", "cancel"] => Request::Timer(TimerAction::CountdownCancel),
        ["timer", minutes] => {
            let minutes: f64 = minutes.trim_end_matches(['m', 'M']).parse().map_err(|_| format!("not a number of minutes: {minutes}"))?;
            if !(minutes > 0.0 && minutes <= 24.0 * 60.0) {
                return Err(format!("{minutes} minutes is out of reach"));
            }
            Request::Timer(TimerAction::Countdown((minutes * 60.0).round() as u64))
        }
        ["focus"] => Request::Timer(TimerAction::Focus),
        ["stopwatch"] => Request::Timer(TimerAction::Stopwatch),
        ["stopwatch", "lap"] => Request::Timer(TimerAction::StopwatchLap),
        ["stopwatch", "reset"] => Request::Timer(TimerAction::StopwatchReset),
        ["wallpaper"] => Request::Wallpaper(WallpaperRequest::Picker(None)),
        ["wallpaper", "open"] => Request::Wallpaper(WallpaperRequest::Picker(Some(true))),
        ["wallpaper", "close"] => Request::Wallpaper(WallpaperRequest::Picker(Some(false))),
        ["wallpaper", "next"] => Request::Wallpaper(WallpaperRequest::Step(1)),
        ["wallpaper", "previous"] => Request::Wallpaper(WallpaperRequest::Step(-1)),
        ["wallpaper", "random"] => Request::Wallpaper(WallpaperRequest::Step(0)),
        ["wallpaper", "set", path @ ..] if !path.is_empty() => Request::Wallpaper(WallpaperRequest::Set(path.join(" "))),
        ["reload"] => Request::Reload,
        _ => return Err(format!("don't know {:?}", words.join(" "))),
    })
}

/// Sends a request to the running shell and returns its answer.
pub fn send(words: &[String]) -> anyhow::Result<String> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("{}\n", words.join(" ")).as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

pub fn running() -> bool {
    send(&["ping".into()]).is_ok_and(|r| r == "pong")
}

/// Listens for requests and hands them to the shell.
pub async fn serve(out: Out) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let path = socket_path();
    // A socket nobody answers on is left over from a crash
    let _ = std::fs::remove_file(&path);
    let listener = match tokio::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("no control socket at {}: {error}", path.display());
            return;
        }
    };
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let out = out.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            let mut reader = tokio::io::BufReader::new(read);
            if tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await.is_err() {
                return;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            let reply = if words == ["ping"] {
                "pong".to_string()
            } else {
                match parse(&words) {
                    Ok(request) => {
                        out.send(Update::Request(request));
                        "ok".to_string()
                    }
                    Err(error) => format!("error: {error}"),
                }
            };
            let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_requests() {
        assert_eq!(parse(&["volume", "up"]), Ok(Request::Volume(0.05)));
        assert_eq!(parse(&["volume", "down", "10%"]), Ok(Request::Volume(-0.1)));
        assert_eq!(parse(&["brightness", "up", "2"]), Ok(Request::Brightness(0.02)));
        assert_eq!(parse(&["launcher"]), Ok(Request::Launcher(None)));
        assert_eq!(parse(&["control", "close"]), Ok(Request::Control(Some(false))));
        assert_eq!(parse(&["menu", "sound"]), Ok(Request::Menu("sound".into())));
        assert_eq!(parse(&["osd", "volume"]), Ok(Request::Osd("volume".into())));
        assert_eq!(parse(&["wallpaper"]), Ok(Request::Wallpaper(WallpaperRequest::Picker(None))));
        assert_eq!(parse(&["wallpaper", "random"]), Ok(Request::Wallpaper(WallpaperRequest::Step(0))));
        assert_eq!(parse(&["wallpaper", "set", "~/My", "Sea.png"]), Ok(Request::Wallpaper(WallpaperRequest::Set("~/My Sea.png".into()))));
        assert!(parse(&["wallpaper", "set"]).is_err());
        assert_eq!(parse(&["timer", "25"]), Ok(Request::Timer(TimerAction::Countdown(1500))));
        assert_eq!(parse(&["timer", "1.5m"]), Ok(Request::Timer(TimerAction::Countdown(90))));
        assert!(parse(&["timer", "soon"]).is_err());
        assert!(parse(&["timer", "0"]).is_err());
        assert_eq!(parse(&["screenshot"]), Ok(Request::Screenshot(Capture::Region)));
        assert_eq!(parse(&["screenshot", "window"]), Ok(Request::Screenshot(Capture::Window)));
        assert_eq!(parse(&["record", "audio"]), Ok(Request::Record { audio: true }));
        assert_eq!(parse(&["screen-text"]), Ok(Request::ScreenText));
        assert_eq!(parse(&["day", "open"]), Ok(Request::Day(Some(true))));
        assert_eq!(parse(&["dock"]), Ok(Request::Dock(None)));
        assert_eq!(parse(&["power", "button"]), Ok(Request::PowerButton));
        assert_eq!(parse(&["bar", "arrange"]), Ok(Request::ArrangeBar(None)));
        assert_eq!(parse(&["bar", "arrange", "off"]), Ok(Request::ArrangeBar(Some(false))));
        assert!(parse(&["bar", "shuffle"]).is_err());
        assert_eq!(parse(&["dock", "hide"]), Ok(Request::Dock(Some(false))));
        assert_eq!(parse(&["clipboard"]), Ok(Request::Clipboard(None)));
        assert_eq!(parse(&["shortcuts", "open"]), Ok(Request::Shortcuts(Some(true))));
        assert_eq!(parse(&["night-light", "off"]), Ok(Request::NightLight(Some(false))));
        assert_eq!(parse(&["widgets"]), Ok(Request::Widgets(None)));
        assert_eq!(parse(&["pet", "feed"]), Ok(Request::Pet("feed".into())));
        assert!(parse(&["pet", "bite"]).is_err());
        assert_eq!(parse(&["pet", "friend", "away"]), Ok(Request::Pet("2 away".into())));
        assert_eq!(parse(&["pet", "add"]), Ok(Request::Pet("add".into())));
        assert_eq!(parse(&["pet", "add", "panda:brown"]), Ok(Request::Pet("add panda:brown".into())));
        assert_eq!(parse(&["pet", "7", "next"]), Ok(Request::Pet("7 next".into())));
        assert!(parse(&["pet", "0", "away"]).is_err());
        assert_eq!(parse(&["widgets", "edit"]), Ok(Request::Widgets(Some(true))));
        assert_eq!(parse(&["widgets", "done"]), Ok(Request::Widgets(Some(false))));
        assert!(parse(&["volume", "up", "lots"]).is_err());
        assert!(parse(&["launch", "missiles"]).is_err());
    }
}
