//! niri, through `$NIRI_SOCKET`: one connection carries the event stream (the whole state
//! first, then each change), and every action gets a connection of its own. Requests and
//! replies are one JSON line each.

use super::{Out, xkb};
use crate::model::{Command, CompositorKind, CompositorState, Update, Window, Workspace, WorkspaceTarget};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct NiriWorkspace {
    id: u64,
    idx: u8,
    name: Option<String>,
    output: Option<String>,
    is_active: bool,
    is_focused: bool,
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct NiriLayout {
    pos_in_scrolling_layout: Option<(usize, usize)>,
    tile_size: Option<(f64, f64)>,
    tile_pos_in_workspace_view: Option<(f64, f64)>,
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct NiriWindow {
    id: u64,
    title: Option<String>,
    app_id: Option<String>,
    workspace_id: Option<u64>,
    is_focused: bool,
    is_floating: bool,
    layout: NiriLayout,
}

/// Where each window sits in its workspace: columns side by side (as wide as their widest
/// tile), tiles stacked in a column; floating ones where the view puts them.
fn rects(windows: &[&NiriWindow]) -> Vec<Option<(f32, f32, f32, f32)>> {
    const GAP: f32 = 16.0;
    let mut columns: std::collections::BTreeMap<usize, f32> = std::collections::BTreeMap::new();
    for w in windows {
        if let (Some((column, _)), Some((tw, _))) = (w.layout.pos_in_scrolling_layout, w.layout.tile_size) {
            let width = columns.entry(column).or_insert(0.0);
            *width = width.max(tw as f32);
        }
    }
    let mut column_x = std::collections::HashMap::new();
    let mut x = 0.0;
    for (column, width) in &columns {
        column_x.insert(*column, x);
        x += width + GAP;
    }
    windows
        .iter()
        .map(|w| {
            let (tw, th) = w.layout.tile_size.map(|(a, b)| (a as f32, b as f32))?;
            match w.layout.pos_in_scrolling_layout {
                Some((column, row)) => {
                    // Tiles above this one in its column
                    let y: f32 = windows
                        .iter()
                        .filter_map(|o| match (o.layout.pos_in_scrolling_layout, o.layout.tile_size) {
                            (Some((c, r)), Some((_, h))) if c == column && r < row => Some(h as f32 + GAP),
                            _ => None,
                        })
                        .sum();
                    Some((column_x.get(&column).copied().unwrap_or(0.0), y, tw, th))
                }
                None => w.layout.tile_pos_in_workspace_view.map(|(px, py)| (px as f32, py as f32, tw, th)),
            }
        })
        .collect()
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct Layouts {
    names: Vec<String>,
    current_idx: u8,
}

#[derive(Default)]
struct Niri {
    workspaces: Vec<NiriWorkspace>,
    windows: HashMap<u64, NiriWindow>,
    layouts: Layouts,
}

impl Niri {
    /// One event from the stream, applied to what we know. Returns whether anything we show changed.
    fn apply(&mut self, event: Value) -> bool {
        let Some((kind, data)) = event.as_object().and_then(|o| o.iter().next()) else { return false };
        let parse = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
        match kind.as_str() {
            "WorkspacesChanged" => {
                self.workspaces = serde_json::from_value(parse("workspaces")).unwrap_or_default();
            }
            "WorkspaceActivated" => {
                let id = data.get("id").and_then(Value::as_u64).unwrap_or(0);
                let focused = data.get("focused").and_then(Value::as_bool).unwrap_or(false);
                let output = self.workspaces.iter().find(|w| w.id == id).and_then(|w| w.output.clone());
                for workspace in &mut self.workspaces {
                    if workspace.output == output {
                        workspace.is_active = workspace.id == id;
                    }
                    if focused {
                        workspace.is_focused = workspace.id == id;
                    }
                }
            }
            "WindowsChanged" => {
                let windows: Vec<NiriWindow> = serde_json::from_value(parse("windows")).unwrap_or_default();
                self.windows = windows.into_iter().map(|w| (w.id, w)).collect();
            }
            "WindowOpenedOrChanged" => {
                if let Ok(window) = serde_json::from_value::<NiriWindow>(parse("window")) {
                    if window.is_focused {
                        self.windows.values_mut().for_each(|w| w.is_focused = false);
                    }
                    self.windows.insert(window.id, window);
                }
            }
            "WindowClosed" => {
                let id = data.get("id").and_then(Value::as_u64).unwrap_or(0);
                self.windows.remove(&id);
            }
            "WindowFocusChanged" => {
                let id = data.get("id").and_then(Value::as_u64);
                self.windows.values_mut().for_each(|w| w.is_focused = Some(w.id) == id);
            }
            "WindowLayoutsChanged" => {
                // [[id, layout], …]: only the columns matter here, to keep the stars in order
                if let Some(changes) = data.get("changes").and_then(Value::as_array) {
                    for change in changes {
                        let (Some(id), Some(layout)) = (change.get(0).and_then(Value::as_u64), change.get(1)) else { continue };
                        if let (Some(window), Ok(layout)) = (self.windows.get_mut(&id), serde_json::from_value(layout.clone())) {
                            window.layout = layout;
                        }
                    }
                }
            }
            "KeyboardLayoutsChanged" => {
                self.layouts = serde_json::from_value(parse("keyboard_layouts")).unwrap_or_default();
            }
            "KeyboardLayoutSwitched" => {
                self.layouts.current_idx = data.get("idx").and_then(Value::as_u64).unwrap_or(0) as u8;
            }
            _ => return false,
        }
        true
    }

    fn state(&self) -> CompositorState {
        let mut state = CompositorState { kind: CompositorKind::Niri, ..Default::default() };
        let mut workspaces = self.workspaces.clone();
        workspaces.sort_by_key(|w| (w.output.clone(), w.idx));
        for workspace in &workspaces {
            let output = workspace.output.clone().unwrap_or_default();
            if workspace.is_active {
                state.active_on.insert(output.clone(), workspace.id as i64);
            }
            if workspace.is_focused {
                state.focused_output = output.clone();
            }
            let mut windows: Vec<&NiriWindow> = self.windows.values().filter(|w| w.workspace_id == Some(workspace.id)).collect();
            // Tiled windows by column then row, floating ones after
            windows.sort_by_key(|w| (w.layout.pos_in_scrolling_layout.unwrap_or((usize::MAX, 0)), w.id));
            let places = rects(&windows);
            state.workspaces.push(Workspace {
                id: workspace.id as i64,
                idx: workspace.idx as i64,
                name: workspace.name.clone().unwrap_or_default(),
                output,
                active: workspace.is_active,
                focused: workspace.is_focused,
                windows: windows
                    .into_iter()
                    .zip(places)
                    .map(|(w, rect)| Window {
                        id: w.id,
                        app_id: w.app_id.clone().unwrap_or_default(),
                        title: w.title.clone().unwrap_or_default(),
                        focused: w.is_focused,
                        rect,
                        floating: w.is_floating,
                    })
                    .collect(),
            });
        }
        state.layout_names = self.layouts.names.clone();
        state.layouts = self.layouts.names.iter().map(|n| xkb::code_of(n)).collect();
        state.layout = self.layouts.current_idx as usize;
        state
    }
}

/// Sends one request on a connection of its own and returns niri's answer.
async fn request(socket: &str, request: Value) -> Option<Value> {
    let mut stream = UnixStream::connect(socket).await.ok()?;
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).await.ok()?;
    let mut reply = String::new();
    let mut reader = BufReader::new(stream);
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut reply)).await.ok()?.ok()?;
    let value: Value = serde_json::from_str(&reply).ok()?;
    if let Some(error) = value.get("Err") {
        log::warn!("niri said no to {request}: {error}");
    }
    Some(value)
}

async fn action(socket: &str, action: Value) {
    request(socket, json!({ "Action": action })).await;
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    let Ok(socket) = std::env::var("NIRI_SOCKET") else { return };
    loop {
        let mut stream = match UnixStream::connect(&socket).await {
            Ok(stream) => stream,
            Err(error) => {
                log::warn!("niri: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        if stream.write_all(b"\"EventStream\"\n").await.is_err() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        let mut lines = BufReader::new(stream).lines();
        let mut niri = Niri::default();
        let mut pending = false;
        loop {
            let settle = tokio::time::sleep(Duration::from_millis(if pending { 12 } else { 3_600_000 }));
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        // The first line is the reply, {"Ok":"Handled"}, the rest are events
                        if let Ok(event) = serde_json::from_str::<Value>(&line)
                            && event.get("Ok").is_none() && event.get("Err").is_none() && niri.apply(event) {
                                pending = true;
                            }
                    }
                    _ => break,
                },
                Some(command) = commands.recv() => match command {
                    Command::FocusWorkspace(WorkspaceTarget::Id(id)) => {
                        action(&socket, json!({ "FocusWorkspace": { "reference": { "Id": id } } })).await;
                    }
                    Command::FocusWorkspace(WorkspaceTarget::Step(_, delta)) => {
                        let name = if delta > 0 { "FocusWorkspaceDown" } else { "FocusWorkspaceUp" };
                        action(&socket, json!({ name: {} })).await;
                    }
                    Command::NextLayout => action(&socket, json!({ "SwitchLayout": { "layout": "Next" } })).await,
                    Command::FocusWindow(id) => action(&socket, json!({ "FocusWindow": { "id": id } })).await,
                    Command::CloseWindow(id) => action(&socket, json!({ "CloseWindow": { "id": id } })).await,
                    Command::MoveWindow(id, workspace) => {
                        action(&socket, json!({ "MoveWindowToWorkspace": { "window_id": id, "reference": { "Id": workspace }, "focus": false } })).await;
                    }
                    Command::SetLayout(index) => action(&socket, json!({ "SwitchLayout": { "layout": { "Index": index } } })).await,
                    Command::Quit => action(&socket, json!({ "Quit": { "skip_confirmation": false } })).await,
                    _ => {}
                },
                _ = settle, if pending => {
                    pending = false;
                    out.send(Update::Compositor(niri.state()));
                }
            }
        }
        log::warn!("niri: event stream closed, reconnecting");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// Events as niri 26.04 sends them.
    fn started() -> Niri {
        let mut niri = Niri::default();
        niri.apply(event(r#"{"WorkspacesChanged":{"workspaces":[
            {"id":2,"idx":2,"name":null,"output":"eDP-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":null},
            {"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":11},
            {"id":5,"idx":1,"name":"chat","output":"HDMI-A-1","is_urgent":false,"is_active":true,"is_focused":false,"active_window_id":null}]}}"#));
        niri.apply(event(r#"{"WindowsChanged":{"windows":[
            {"id":14,"title":"Code","app_id":"com.microsoft.VSCode","pid":1,"workspace_id":1,"is_focused":false,"is_floating":false,"is_urgent":false,
             "layout":{"pos_in_scrolling_layout":[2,1],"tile_size":[1712.0,1184.0],"window_size":[1712,1184],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}},
            {"id":11,"title":"Shell","app_id":"kitty","pid":2,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,
             "layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[900.0,1184.0],"window_size":[900,1184],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}},
            {"id":20,"title":"Float","app_id":"pavucontrol","pid":3,"workspace_id":1,"is_focused":false,"is_floating":true,"is_urgent":false,
             "layout":{"pos_in_scrolling_layout":null,"tile_size":[400.0,300.0],"window_size":[400,300],"tile_pos_in_workspace_view":[10.0,10.0],"window_offset_in_tile":[0.0,0.0]}}]}}"#));
        niri.apply(event(r#"{"KeyboardLayoutsChanged":{"keyboard_layouts":{"names":["English (US)","French"],"current_idx":0}}}"#));
        niri
    }

    #[test]
    fn reads_the_stream() {
        let state = started().state();
        assert_eq!(state.kind, CompositorKind::Niri);
        assert_eq!(state.focused_output, "eDP-1");
        let mine: Vec<_> = state.workspaces_on("eDP-1").map(|w| w.idx).collect();
        assert_eq!(mine, vec![1, 2]);
        let first = state.workspace(1).unwrap();
        // Columns left to right, floating windows last
        assert_eq!(first.windows.iter().map(|w| w.app_id.as_str()).collect::<Vec<_>>(), vec!["kitty", "com.microsoft.VSCode", "pavucontrol"]);
        // Columns side by side (900 wide, a gap, then the next), the floating one where it floats
        let rects: Vec<_> = first.windows.iter().map(|w| w.rect).collect();
        assert_eq!(rects, vec![Some((0.0, 0.0, 900.0, 1184.0)), Some((916.0, 0.0, 1712.0, 1184.0)), Some((10.0, 10.0, 400.0, 300.0))]);
        assert!(first.windows[2].floating && !first.windows[0].floating);
        assert_eq!(state.active_on.get("HDMI-A-1"), Some(&5));
        assert_eq!(state.workspace(5).unwrap().name, "chat");
        assert_eq!(state.layouts, vec!["us", "fr"]);
    }

    #[test]
    fn follows_changes() {
        let mut niri = started();
        assert!(niri.apply(event(r#"{"WorkspaceActivated":{"id":2,"focused":true}}"#)));
        assert!(niri.apply(event(r#"{"WindowClosed":{"id":14}}"#)));
        assert!(niri.apply(event(r#"{"WindowOpenedOrChanged":{"window":{"id":30,"title":"New","app_id":"firefox","pid":4,"workspace_id":2,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{"pos_in_scrolling_layout":[1,1]}}}}"#)));
        assert!(niri.apply(event(r#"{"KeyboardLayoutSwitched":{"idx":1}}"#)));
        assert!(!niri.apply(event(r#"{"OverviewOpenedOrClosed":{"is_open":true}}"#)), "events the bar doesn't show change nothing");
        let state = niri.state();
        assert!(state.workspace(2).unwrap().active && state.workspace(2).unwrap().focused);
        assert!(!state.workspace(1).unwrap().active);
        // Another output's shown workspace stays shown
        assert!(state.workspace(5).unwrap().active);
        assert_eq!(state.workspace(1).unwrap().windows.len(), 2);
        let two = &state.workspace(2).unwrap().windows;
        assert_eq!(two.len(), 1);
        assert!(two[0].focused);
        assert!(state.workspace(1).unwrap().windows.iter().all(|w| !w.focused), "focus moved to the new window");
        assert_eq!(state.layout, 1);
    }
}
