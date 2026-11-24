//! Asterism, a quiet desktop shell in Rust: workspaces as joined dots, the clock, the music, the machine's
//! load, the tray and the system's state, for Hyprland and niri.

mod app;
mod appcolors;
mod bar;
mod cards;
mod clipboard;
mod config;
mod dashboard;
mod day;
mod dock;
mod images;
mod ipc;
mod launcher;
mod live;
mod lock;
mod model;
mod night;
mod overview;
mod popups;
mod render;
mod services;
mod settings;
mod shortcuts;
mod snapshot;
mod spectrum;
mod theme;
mod transition;
mod timers;
mod palette;
mod pet;
mod privacy;
mod ui;
mod wallpapers;
mod weather;
mod widgets;
mod world;

use std::path::PathBuf;

const HELP: &str = "\
asterism: a quiet desktop shell for niri and Hyprland

USAGE:
    asterism [--config FILE]
    asterism msg COMMAND…           (see `asterism msg help`)
    asterism --snapshot FILE.png [--width PX] [--scale N] [--menu NAME] [--hover KEY] [--wait SECONDS]

OPTIONS:
    -c, --config FILE   Read this config instead of ~/.config/asterism/config.toml
    --snapshot FILE     Draw the bar with live data into a PNG instead of showing it
    --width PX          Snapshot width in logical pixels (default 1920)
    --scale N           Snapshot scale (default 1)
    --menu NAME         Snapshot a menu instead: calendar, media, sound, network, bluetooth,
                        power, session, machine, traffic, keyboard
    --hover KEY         Snapshot as if the pointer were on this piece (like clock, page.1)
    --focus KEY         Snapshot a menu with the keyboard on this piece
    --panel NAME        Snapshot launcher, toasts or osd instead
    --query TEXT        What's typed in the launcher snapshot
    --room PX           Snapshot a menu as if only this much height were left, so it scrolls
    --scroll PX         How far that menu is scrolled
    --wait SECONDS      How long to gather data before a snapshot (default 3)
    --demo              Made-up workspaces and windows instead of yours, for shared screenshots
    -h, --help          Show this
    -V, --version       Show the version

The config is read again whenever it changes. RUST_LOG=debug says more about what happens.
";

fn main() {
    images::tune_allocator();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,zbus=warn,tracing=warn")).init();

    // Talking to the running shell
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.first().is_some_and(|a| a == "msg") {
        let words = &all[1..];
        if words.is_empty() || words[0] == "help" || words[0] == "--help" {
            print!("{}", ipc::USAGE);
            return;
        }
        match ipc::send(words) {
            Ok(reply) if reply.starts_with("error") => {
                eprintln!("{reply}");
                std::process::exit(1);
            }
            Ok(reply) => {
                if reply != "ok" {
                    println!("{reply}");
                }
            }
            Err(error) => {
                eprintln!("asterism isn't running ({error})");
                std::process::exit(1);
            }
        }
        return;
    }

    // The lock screen runs on its own, apart from the shell
    if all.first().is_some_and(|a| a == "unlock") {
        if let Err(error) = lock::request_unlock() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if all.first().is_some_and(|a| a == "lock") {
        // Its own name, so stopping the shell (pkill -x asterism) can never stop the lock
        let _ = rustix::thread::set_name(c"asterism-lock");
        let config_path = all.iter().position(|a| a == "-c" || a == "--config").and_then(|i| all.get(i + 1)).map(PathBuf::from).unwrap_or_else(config::default_path);
        if let Err(error) = lock::run(config_path) {
            eprintln!("asterism lock: {error:#}");
            std::process::exit(1);
        }
        return;
    }

    let mut args = std::env::args().skip(1);
    let mut config_path = config::default_path();
    let mut snapshot: Option<snapshot::Options> = None;
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("{name} needs a value");
                std::process::exit(2);
            })
        };
        match arg.as_str() {
            "-c" | "--config" => config_path = PathBuf::from(value("--config")),
            "--snapshot" => snapshot.get_or_insert_with(snapshot::Options::default).path = PathBuf::from(value("--snapshot")),
            "--width" => snapshot.get_or_insert_with(snapshot::Options::default).width = value("--width").parse().unwrap_or(1920.0),
            "--scale" => snapshot.get_or_insert_with(snapshot::Options::default).scale = value("--scale").parse().unwrap_or(1.0),
            "--menu" => snapshot.get_or_insert_with(snapshot::Options::default).menu = Some(value("--menu")),
            "--hover" => snapshot.get_or_insert_with(snapshot::Options::default).hover = Some(value("--hover")),
            "--focus" => snapshot.get_or_insert_with(snapshot::Options::default).focus = Some(value("--focus")),
            "--room" => snapshot.get_or_insert_with(snapshot::Options::default).room = value("--room").parse().ok(),
            "--scroll" => snapshot.get_or_insert_with(snapshot::Options::default).scroll = value("--scroll").parse().unwrap_or(0.0),
            "--panel" => snapshot.get_or_insert_with(snapshot::Options::default).panel = Some(value("--panel")),
            "--query" => snapshot.get_or_insert_with(snapshot::Options::default).query = value("--query"),
            "--demo" => snapshot.get_or_insert_with(snapshot::Options::default).demo = true,
            "--wait" => snapshot.get_or_insert_with(snapshot::Options::default).wait = value("--wait").parse().unwrap_or(3.0),
            "-h" | "--help" => {
                print!("{HELP}");
                return;
            }
            "-V" | "--version" => {
                println!("asterism {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("unknown option {other:?}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    let result = match snapshot {
        Some(options) => snapshot::run(&config_path, options),
        None => {
            if ipc::running() {
                eprintln!("asterism is already running on this display");
                std::process::exit(1);
            }
            // A socket left by a shell that crashed is replaced as this one starts
            app::run(config_path)
        }
    };
    if let Err(error) = result {
        log::error!("{error:#}");
        std::process::exit(1);
    }
}
