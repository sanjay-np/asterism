//! Hyprland, through its two sockets: `.socket.sock` answers questions (opened and closed for
//! each one, as Hyprland asks), `.socket2.sock` tells what happens. Any event that changes
//! workspaces or windows has the state read again, a beat later so bursts count once.
//!
//! Since 0.55 Hyprland's config is Lua and `dispatch` takes a Lua call such as
//! `hl.dsp.focus({ workspace = 3 })`; before, it took `workspace 3`. The bar tries one, falls
//! back to the other, and remembers which one worked.

use super::{Out, privacy, xkb};
use crate::model::{Cast, CastTarget, Command, CompositorKind, CompositorState, Update, Window, Workspace, WorkspaceTarget};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Deserialize, Default)]
#[serde(default)]
struct WorkspaceRef {
    id: i64,
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Monitor {
    name: String,
    focused: bool,
    #[serde(rename = "activeWorkspace")]
    active_workspace: WorkspaceRef,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct HyprWorkspace {
    id: i64,
    name: String,
    monitor: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Client {
    address: String,
    mapped: bool,
    hidden: bool,
    at: (i32, i32),
    size: (i32, i32),
    floating: bool,
    workspace: WorkspaceRef,
    class: String,
    title: String,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: i32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Keyboard {
    layout: String,
    active_keymap: String,
    main: bool,
    active_layout_index: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Devices {
    keyboards: Vec<Keyboard>,
}

struct Hyprland {
    socket: PathBuf,
    events: PathBuf,
    /// Hyprland before the Lua config: dispatchers take their plain form.
    legacy: bool,
    state: CompositorState,
    /// Sharing the screen, as the last `screencast` event said.
    casts: Vec<Cast>,
}

fn socket_dir() -> Option<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    [runtime.map(|r| r.join("hypr").join(&signature)), Some(PathBuf::from("/tmp/hypr").join(&signature))]
        .into_iter()
        .flatten()
        .find(|dir| dir.join(".socket.sock").exists())
}

impl Hyprland {
    async fn request(&self, what: &str) -> Option<String> {
        let mut stream = UnixStream::connect(&self.socket).await.ok()?;
        stream.write_all(what.as_bytes()).await.ok()?;
        let mut reply = String::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut reply)).await.ok()?.ok()?;
        Some(reply)
    }

    async fn json<T: for<'de> Deserialize<'de> + Default>(&self, what: &str) -> T {
        match self.request(what).await {
            Some(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("hyprland {what}: {error}");
                T::default()
            }),
            None => T::default(),
        }
    }

    async fn refresh(&mut self) {
        let monitors: Vec<Monitor> = self.json("j/monitors").await;
        let workspaces: Vec<HyprWorkspace> = self.json("j/workspaces").await;
        let clients: Vec<Client> = self.json("j/clients").await;

        let mut state = CompositorState { kind: CompositorKind::Hyprland, ..Default::default() };
        for monitor in &monitors {
            state.active_on.insert(monitor.name.clone(), monitor.active_workspace.id);
            if monitor.focused {
                state.focused_output = monitor.name.clone();
            }
        }

        let mut windows: HashMap<i64, Vec<&Client>> = HashMap::new();
        for client in clients.iter().filter(|c| c.mapped && !c.hidden && c.workspace.id > 0) {
            windows.entry(client.workspace.id).or_default().push(client);
        }
        // Special workspaces (negative ids) aren't pages
        for workspace in workspaces.iter().filter(|w| w.id > 0) {
            let mut here = windows.remove(&workspace.id).unwrap_or_default();
            here.sort_by_key(|c| (c.at.0, c.at.1));
            let active = state.active_on.get(&workspace.monitor) == Some(&workspace.id);
            state.workspaces.push(Workspace {
                id: workspace.id,
                idx: workspace.id,
                name: workspace.name.clone(),
                output: workspace.monitor.clone(),
                active,
                focused: active && workspace.monitor == state.focused_output,
                windows: here
                    .into_iter()
                    .map(|c| Window {
                        id: u64::from_str_radix(c.address.trim_start_matches("0x"), 16).unwrap_or(0),
                        app_id: c.class.clone(),
                        title: c.title.clone(),
                        focused: c.focus_history_id == 0,
                        rect: (c.size.0 > 0).then_some((c.at.0 as f32, c.at.1 as f32, c.size.0 as f32, c.size.1 as f32)),
                        floating: c.floating,
                    })
                    .collect(),
            });
        }
        state.workspaces.sort_by_key(|w| w.id);
        self.read_keyboard(&mut state).await;
        state.casts = self.casts.clone();
        self.state = state;
    }

    async fn read_keyboard(&self, state: &mut CompositorState) {
        let devices: Devices = self.json("j/devices").await;
        let Some(main) = devices.keyboards.iter().find(|k| k.main).or(devices.keyboards.first()) else { return };
        state.layouts = main.layout.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
        state.layout_names = state.layouts.iter().map(|c| xkb::name_of(c)).collect();
        state.layout = main.active_layout_index.unwrap_or_else(|| {
            let code = xkb::code_of(&main.active_keymap);
            state.layouts.iter().position(|c| *c == code).unwrap_or(0)
        });
    }

    /// Sends a dispatcher in whichever form this Hyprland understands.
    async fn dispatch(&mut self, lua: &str, plain: &str) {
        let tries = if self.legacy { [plain, lua] } else { [lua, plain] };
        for (attempt, form) in tries.iter().enumerate() {
            let reply = self.request(&format!("dispatch {form}")).await.unwrap_or_default();
            if reply.trim() == "ok" {
                if attempt == 1 {
                    self.legacy = !self.legacy;
                }
                return;
            }
            log::debug!("hyprland: `dispatch {form}` said {reply:?}");
        }
        log::warn!("hyprland didn't take `{plain}` in either form");
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::FocusWorkspace(target) => {
                let id = match target {
                    WorkspaceTarget::Id(id) => id,
                    WorkspaceTarget::Step(output, delta) => {
                        let current = self.state.active_on.get(&output).copied().unwrap_or(1);
                        (current + delta as i64).max(1)
                    }
                };
                self.dispatch(&format!("hl.dsp.focus({{ workspace = {id} }})"), &format!("workspace {id}")).await;
            }
            Command::NextLayout => {
                self.request("switchxkblayout all next").await;
            }
            Command::SetLayout(index) => {
                self.request(&format!("switchxkblayout all {index}")).await;
            }
            Command::Quit => self.dispatch("hl.dsp.exit()", "exit").await,
            Command::FocusWindow(id) => self.dispatch(&format!("hl.dsp.focus({{ window = \"address:0x{id:x}\" }})"), &format!("focuswindow address:0x{id:x}")).await,
            Command::CloseWindow(id) => self.dispatch(&format!("hl.dsp.window.close({{ window = \"address:0x{id:x}\" }})"), &format!("closewindow address:0x{id:x}")).await,
            Command::MoveWindow(id, workspace) => {
                self.dispatch(
                    &format!("hl.dsp.window.move({{ window = \"address:0x{id:x}\", workspace = {workspace}, follow = false }})"),
                    &format!("movetoworkspacesilent {workspace},address:0x{id:x}"),
                )
                .await
            }
            _ => {}
        }
    }
}

/// The events that change what the bar shows.
fn matters(event: &str) -> bool {
    matches!(
        event,
        "workspace"
            | "workspacev2"
            | "focusedmon"
            | "focusedmonv2"
            | "createworkspace"
            | "createworkspacev2"
            | "destroyworkspace"
            | "destroyworkspacev2"
            | "moveworkspace"
            | "moveworkspacev2"
            | "renameworkspace"
            | "openwindow"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
            | "activewindowv2"
            | "windowtitlev2"
            | "monitoradded"
            | "monitorremoved"
            | "activelayout"
            | "configreloaded"
            | "changefloatingmode"
            | "minimized"
    )
}

/// What `screencast>>STATE,OWNER` says (owner 0 a screen, 1 a window), with the apps taking
/// it from PipeWire's graph. Hyprland names neither the screen nor the window, and has no way
/// to stop a cast from outside.
fn cast_of(data: &str) -> Option<CastTarget> {
    let (state, owner) = data.split_once(',').unwrap_or((data, "0"));
    (state.trim() == "1").then(|| if owner.trim() == "1" { CastTarget::Window(String::new()) } else { CastTarget::Output(String::new()) })
}

async fn screencast(data: &str) -> Vec<Cast> {
    let Some(target) = cast_of(data) else { return Vec::new() };
    let apps = privacy::graph().await.map(|g| g.screen_readers()).unwrap_or_default();
    let cast = |stream: usize, app: String| Cast { stream: stream as u64, session: None, app, target: target.clone(), active: true };
    if apps.is_empty() { vec![cast(0, String::new())] } else { apps.into_iter().enumerate().map(|(i, app)| cast(i, app)).collect() }
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    let Some(dir) = socket_dir() else {
        log::error!("hyprland: can't find its sockets");
        return;
    };
    let mut hyprland = Hyprland {
        socket: dir.join(".socket.sock"),
        events: dir.join(".socket2.sock"),
        legacy: false,
        state: CompositorState::default(),
        casts: Vec::new(),
    };

    loop {
        let stream = match UnixStream::connect(&hyprland.events).await {
            Ok(stream) => stream,
            Err(error) => {
                log::warn!("hyprland events: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        hyprland.refresh().await;
        out.send(Update::Compositor(hyprland.state.clone()));

        let mut lines = BufReader::new(stream).lines();
        let mut pending = false;
        loop {
            let settle = tokio::time::sleep(Duration::from_millis(if pending { 25 } else { 3_600_000 }));
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        let (event, data) = line.split_once(">>").unwrap_or((line.as_str(), ""));
                        if event == "screencast" {
                            hyprland.casts = screencast(data).await;
                            pending = true;
                        } else if matters(event) {
                            pending = true;
                        }
                    }
                    _ => break,
                },
                Some(command) = commands.recv() => hyprland.command(command).await,
                _ = settle, if pending => {
                    pending = false;
                    let before = hyprland.state.clone();
                    hyprland.refresh().await;
                    if hyprland.state != before {
                        out.send(Update::Compositor(hyprland.state.clone()));
                    }
                }
            }
        }
        log::warn!("hyprland: event socket closed, reconnecting");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::net::UnixListener;

    #[test]
    fn reads_screencast_events() {
        assert_eq!(cast_of("1,0"), Some(CastTarget::Output(String::new())));
        assert_eq!(cast_of("1,1"), Some(CastTarget::Window(String::new())));
        assert_eq!(cast_of("0,0"), None);
        assert_eq!(cast_of("0"), None);
    }

    /// A pretend Hyprland: answers the questions the bar asks, and takes dispatchers in
    /// one form only, like a real one would.
    async fn fake(dir: &std::path::Path, lua: bool, seen: Arc<Mutex<Vec<String>>>) {
        let listener = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let mut buffer = vec![0u8; 4096];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                seen.lock().unwrap().push(request.clone());
                let reply = match request.as_str() {
                    "j/monitors" => r#"[{"id":0,"name":"DP-1","focused":true,"activeWorkspace":{"id":2,"name":"2"}},
                                         {"id":1,"name":"HDMI-A-1","focused":false,"activeWorkspace":{"id":9,"name":"9"}}]"#.to_string(),
                    "j/workspaces" => r#"[{"id":2,"name":"2","monitor":"DP-1","windows":2},{"id":1,"name":"1","monitor":"DP-1","windows":0},
                                           {"id":9,"name":"9","monitor":"HDMI-A-1","windows":1},{"id":-98,"name":"special:magic","monitor":"DP-1"}]"#.to_string(),
                    "j/clients" => r#"[{"address":"0x10","mapped":true,"hidden":false,"at":[900,10],"workspace":{"id":2,"name":"2"},"class":"firefox","title":"Web","focusHistoryID":1},
                                        {"address":"0x11","mapped":true,"hidden":false,"at":[10,10],"workspace":{"id":2,"name":"2"},"class":"kitty","title":"Shell","focusHistoryID":0},
                                        {"address":"0x12","mapped":true,"hidden":false,"at":[0,0],"workspace":{"id":9,"name":"9"},"class":"mpv","title":"Film","focusHistoryID":2},
                                        {"address":"0x13","mapped":true,"hidden":false,"at":[0,0],"workspace":{"id":-98,"name":"special:magic"},"class":"notes","title":"Notes","focusHistoryID":3},
                                        {"address":"0x14","mapped":false,"hidden":false,"at":[0,0],"workspace":{"id":2,"name":"2"},"class":"ghost","title":"","focusHistoryID":4}]"#.to_string(),
                    "j/devices" => r#"{"mice":[],"keyboards":[{"name":"at-translated","layout":"us,fr","active_keymap":"French","main":true}]}"#.to_string(),
                    other if other.starts_with("dispatch hl.") => if lua { "ok".into() } else { "Invalid dispatcher".into() },
                    other if other.starts_with("dispatch ") => if lua { "error: lua syntax".into() } else { "ok".into() },
                    _ => "ok".into(),
                };
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, reply.as_bytes()).await;
            }
        });
    }

    fn hyprland(dir: &std::path::Path) -> Hyprland {
        Hyprland { socket: dir.join(".socket.sock"), events: dir.join(".socket2.sock"), legacy: false, state: CompositorState::default(), casts: Vec::new() }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("asterism-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn reads_the_state() {
        let dir = temp("state");
        fake(&dir, true, Arc::default()).await;
        let mut h = hyprland(&dir);
        h.refresh().await;
        let s = &h.state;
        assert_eq!(s.kind, CompositorKind::Hyprland);
        assert_eq!(s.focused_output, "DP-1");
        assert_eq!(s.active_on.get("HDMI-A-1"), Some(&9));
        // Sorted by id, special workspaces left out
        assert_eq!(s.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(), vec![1, 2, 9]);
        let two = s.workspace(2).unwrap();
        assert!(two.active && two.focused);
        // Left to right, unmapped windows left out, the focused one known
        assert_eq!(two.windows.iter().map(|w| w.app_id.as_str()).collect::<Vec<_>>(), vec!["kitty", "firefox"]);
        assert!(two.windows[0].focused);
        assert!(s.workspace(9).unwrap().active && !s.workspace(9).unwrap().focused);
        assert_eq!(s.layouts, vec!["us", "fr"]);
        assert_eq!(s.layout, 1, "French is the second layout");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dispatches_in_lua_then_remembers() {
        let dir = temp("lua");
        let seen = Arc::new(Mutex::new(Vec::new()));
        fake(&dir, true, seen.clone()).await;
        let mut h = hyprland(&dir);
        h.command(Command::FocusWorkspace(WorkspaceTarget::Id(3))).await;
        assert!(!h.legacy);
        assert_eq!(seen.lock().unwrap().as_slice(), ["dispatch hl.dsp.focus({ workspace = 3 })"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn falls_back_to_the_plain_form() {
        let dir = temp("legacy");
        let seen = Arc::new(Mutex::new(Vec::new()));
        fake(&dir, false, seen.clone()).await;
        let mut h = hyprland(&dir);
        h.command(Command::FocusWorkspace(WorkspaceTarget::Id(4))).await;
        assert!(h.legacy, "an older Hyprland is remembered");
        h.command(Command::FocusWorkspace(WorkspaceTarget::Id(5))).await;
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["dispatch hl.dsp.focus({ workspace = 4 })", "dispatch workspace 4", "dispatch workspace 5"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn steps_from_the_workspace_shown_on_that_output() {
        let dir = temp("step");
        let seen = Arc::new(Mutex::new(Vec::new()));
        fake(&dir, true, seen.clone()).await;
        let mut h = hyprland(&dir);
        h.refresh().await;
        seen.lock().unwrap().clear();
        h.command(Command::FocusWorkspace(WorkspaceTarget::Step("HDMI-A-1".into(), 1))).await;
        h.command(Command::FocusWorkspace(WorkspaceTarget::Step("DP-1".into(), -5))).await;
        assert_eq!(seen.lock().unwrap().as_slice(), ["dispatch hl.dsp.focus({ workspace = 10 })", "dispatch hl.dsp.focus({ workspace = 1 })"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
