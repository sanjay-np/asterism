//! Who is using the camera, and which apps take the screen.
//!
//! The camera: each `/dev/video*` node is watched (inotify) for being opened and closed, and
//! when that settles, the processes holding one are found in `/proc`. Opening it is what turns
//! a camera on, so nothing is polled. When PipeWire is the one holding it, the apps are the
//! ones its graph links to the camera; PipeWire alone (probing a new device) counts as no one.
//!
//! The screen: the compositor says when a cast starts or changes (niri's cast events,
//! Hyprland's `screencast`); `pw-dump` then says which app takes the cast's stream.

use super::Out;
use crate::model::Update;
use rustix::fs::inotify;
use serde_json::Value;
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::time::Duration;
use tokio::io::unix::AsyncFd;

/// What PipeWire's graph says about video: the nodes and who reads from whom.
#[derive(Debug, Default)]
pub struct Graph {
    nodes: HashMap<u64, Node>,
    /// (from node, to node)
    links: Vec<(u64, u64)>,
}

#[derive(Debug, Default)]
struct Node {
    class: String,
    app: String,
    /// A camera rather than a screen: it comes from a device.
    camera: bool,
}

impl Graph {
    /// Reads `pw-dump`'s array of objects.
    pub fn parse(dump: &Value) -> Graph {
        let mut graph = Graph::default();
        for object in dump.as_array().into_iter().flatten() {
            let Some(id) = object.get("id").and_then(Value::as_u64) else { continue };
            let info = object.get("info").unwrap_or(&Value::Null);
            match object.get("type").and_then(Value::as_str).unwrap_or("") {
                "PipeWire:Interface:Node" => {
                    let props = info.get("props").unwrap_or(&Value::Null);
                    let prop = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                    let app = [prop("application.name"), prop("application.process.binary")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
                    let api = prop("device.api");
                    let camera = matches!(api.as_str(), "v4l2" | "libcamera") || prop("object.path").starts_with("v4l2:") || prop("object.path").starts_with("libcamera:");
                    graph.nodes.insert(id, Node { class: prop("media.class"), app, camera });
                }
                "PipeWire:Interface:Link" => {
                    let end = |key: &str| info.get(key).and_then(Value::as_u64);
                    if let (Some(from), Some(to)) = (end("output-node-id"), end("input-node-id")) {
                        graph.links.push((from, to));
                    }
                }
                _ => {}
            }
        }
        graph
    }

    /// The apps reading from a node, each once.
    pub fn readers(&self, node: u64) -> Vec<String> {
        let mut apps: Vec<String> = Vec::new();
        for (_, to) in self.links.iter().filter(|(from, _)| *from == node) {
            if let Some(reader) = self.nodes.get(to)
                && !reader.app.is_empty()
                && !apps.contains(&reader.app)
            {
                apps.push(reader.app.clone());
            }
        }
        apps
    }

    /// The apps reading from a camera through PipeWire.
    pub fn camera_readers(&self) -> Vec<String> {
        let mut apps: Vec<String> = Vec::new();
        for (id, _) in self.nodes.iter().filter(|(_, n)| n.camera && n.class.starts_with("Video/Source")) {
            for app in self.readers(*id) {
                if !apps.contains(&app) {
                    apps.push(app);
                }
            }
        }
        apps
    }

    /// The apps reading from a screen being cast: video that doesn't come from a device.
    pub fn screen_readers(&self) -> Vec<String> {
        let mut apps: Vec<String> = Vec::new();
        for (id, _) in self.nodes.iter().filter(|(_, n)| !n.camera && n.class.starts_with("Video/Source")) {
            for app in self.readers(*id) {
                if !apps.contains(&app) {
                    apps.push(app);
                }
            }
        }
        apps
    }
}

/// PipeWire's graph now, or nothing when `pw-dump` isn't there.
pub async fn graph() -> Option<Graph> {
    let text = super::output("pw-dump", &["-N"]).await?;
    serde_json::from_str::<Value>(&text).ok().map(|dump| Graph::parse(&dump))
}

/// A process's name: the command it ran, else what the kernel calls it.
pub fn process_name(pid: u32) -> String {
    let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    if let Ok(cmdline) = std::fs::read(dir.join("cmdline"))
        && let Some(first) = cmdline.split(|b| *b == 0).next()
    {
        let name = String::from_utf8_lossy(first).rsplit('/').next().unwrap_or("").to_string();
        if !name.is_empty() {
            return name;
        }
    }
    std::fs::read_to_string(dir.join("comm")).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The processes with a camera open, by name, each once. Only the user's own processes can be
/// seen, which is everything that matters on a desktop.
fn camera_holders() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return names };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
        let holds = fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|to| to.to_string_lossy().starts_with("/dev/video")));
        if holds {
            let name = process_name(pid);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// PipeWire's own daemons: when they hold the camera, the apps are found in its graph.
fn is_pipewire(name: &str) -> bool {
    matches!(name, "pipewire" | "wireplumber" | "pipewire-media-session")
}

/// Who is using a camera now.
async fn camera_users() -> Vec<String> {
    let holders = tokio::task::spawn_blocking(camera_holders).await.unwrap_or_default();
    let mut users: Vec<String> = holders.iter().filter(|n| !is_pipewire(n)).cloned().collect();
    if holders.iter().any(|n| is_pipewire(n))
        && let Some(graph) = graph().await
    {
        for app in graph.camera_readers() {
            if !users.contains(&app) {
                users.push(app);
            }
        }
    }
    users.sort();
    users
}

const DEVICE_EVENTS: inotify::WatchFlags = inotify::WatchFlags::OPEN.union(inotify::WatchFlags::CLOSE);

/// Watches the video nodes there are, and `/dev` for new ones.
fn watch() -> std::io::Result<OwnedFd> {
    let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
    inotify::add_watch(&fd, "/dev", inotify::WatchFlags::CREATE)?;
    for entry in std::fs::read_dir("/dev")?.flatten() {
        if entry.file_name().to_string_lossy().starts_with("video") {
            let _ = inotify::add_watch(&fd, entry.path(), DEVICE_EVENTS);
        }
    }
    Ok(fd)
}

pub async fn run(out: Out) {
    let watched = watch().and_then(|fd| {
        // SAFETY: the inotify descriptor is owned here and moves into the AsyncFd, which keeps
        // it open and unchanged until it drops
        unsafe { AsyncFd::register(fd) }.map_err(|e| e.into())
    });
    let fd = match watched {
        Ok(fd) => fd,
        Err(error) => {
            log::warn!("privacy: can't watch the cameras: {error}");
            return;
        }
    };
    let mut users = camera_users().await;
    out.send(Update::Camera(users.clone()));
    let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
    let mut pending = false;
    loop {
        // Opening a camera comes in bursts (apps try each node, PipeWire probes): wait for quiet
        let settle = tokio::time::sleep(Duration::from_millis(if pending { 300 } else { 3_600_000 }));
        tokio::select! {
            ready = fd.readable() => {
                let Ok(mut guard) = ready else { return };
                let mut new_nodes = Vec::new();
                let mut reader = inotify::Reader::new(guard.get_inner(), &mut buffer);
                loop {
                    match reader.next() {
                        Ok(event) => {
                            pending = true;
                            if let Some(name) = event.file_name().map(|n| n.to_string_lossy().into_owned())
                                && name.starts_with("video")
                            {
                                new_nodes.push(name);
                            }
                        }
                        Err(rustix::io::Errno::AGAIN) => break,
                        Err(error) => {
                            log::warn!("privacy: {error}");
                            return;
                        }
                    }
                }
                guard.clear_ready();
                // A camera plugged in: watch its nodes too
                for name in new_nodes {
                    let _ = inotify::add_watch(guard.get_inner(), format!("/dev/{name}"), DEVICE_EVENTS);
                }
            }
            _ = settle, if pending => {
                pending = false;
                let now = camera_users().await;
                if now != users {
                    log::debug!("camera in use by {now:?}");
                    users = now;
                    out.send(Update::Camera(users.clone()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed `pw-dump`: a camera read by a browser, a screen cast read by a recorder, a
    /// microphone (not video) and a link with nothing on its other end.
    fn dump() -> Value {
        serde_json::json!([
            { "id": 63, "type": "PipeWire:Interface:Node", "info": { "state": "running", "props": {
                "media.class": "Video/Source", "device.api": "v4l2", "object.path": "v4l2:/dev/video0", "node.name": "v4l2_input.pci" } } },
            { "id": 80, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Stream/Input/Video", "application.name": "Firefox", "node.name": "webrtc" } } },
            { "id": 90, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Video/Source", "node.name": "niri" } } },
            { "id": 91, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Stream/Input/Video", "application.process.binary": "obs" } } },
            { "id": 54, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Audio/Source", "node.name": "alsa_input" } } },
            { "id": 100, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 63, "input-node-id": 80 } },
            { "id": 101, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 90, "input-node-id": 91 } },
            { "id": 102, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 90, "input-node-id": 91 } },
            { "id": 103, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 54, "input-node-id": 999 } },
            { "id": 104, "type": "PipeWire:Interface:Client", "info": { "props": { "application.name": "pw-dump" } } }
        ])
    }

    #[test]
    fn finds_who_reads_cameras_and_screens() {
        let graph = Graph::parse(&dump());
        assert_eq!(graph.camera_readers(), ["Firefox"]);
        assert_eq!(graph.screen_readers(), ["obs"], "the app's binary when it has no name, once for two links");
        assert_eq!(graph.readers(90), ["obs"]);
        assert!(graph.readers(54).is_empty(), "a link to nothing names no one");
    }

    #[test]
    fn a_camera_no_one_reads_is_not_in_use() {
        let mut dump = dump();
        dump.as_array_mut().unwrap().retain(|o| o["id"] != 100);
        assert!(Graph::parse(&dump).camera_readers().is_empty());
    }

    #[test]
    fn pipewire_itself_is_not_a_user() {
        assert!(is_pipewire("wireplumber") && is_pipewire("pipewire"));
        assert!(!is_pipewire("firefox"));
    }
}
