//! The clipboard, through ext-data-control: every new selection is read (text, or a picture)
//! into the history, and choosing one from the picker offers it again as the selection.
//! Reading happens on a worker thread so a slow app never holds up the bar. Anything a
//! password manager marks as secret is never read.

use super::*;
use crate::clipboard::{Content, Picker};
use smithay_client_toolkit::reexports::protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use std::io::{Read, Write};
use std::sync::Mutex;

/// Text, in the order apps usually offer it.
const TEXT: [&str; 5] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING"];
/// Never more than this from one copy.
const LIMIT: usize = 16 << 20;

/// What an offer says it can be.
#[derive(Default)]
pub struct OfferData(Mutex<Vec<String>>);

impl Dispatch2<ExtDataControlOfferV1, App> for OfferData {
    fn event(&self, _: &mut App, _: &ExtDataControlOfferV1, event: ext_data_control_offer_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            self.0.lock().unwrap().push(mime_type);
        }
    }
}

pub struct DeviceData;

impl Dispatch2<ExtDataControlDeviceV1, App> for DeviceData {
    fn event(&self, app: &mut App, _: &ExtDataControlDeviceV1, event: ext_data_control_device_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        match event {
            ext_data_control_device_v1::Event::Selection { id: Some(offer) } => app.clipboard_offer(offer),
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            ext_data_control_device_v1::Event::Finished => app.clip_device = None,
            _ => {}
        }
    }

    fn event_created_child(_opcode: u16, qh: &QueueHandle<App>) -> std::sync::Arc<dyn smithay_client_toolkit::reexports::client::backend::ObjectData> {
        qh.make_data::<ExtDataControlOfferV1, OfferData>(OfferData::default())
    }
}

/// What a clip we offer is, for whoever pastes it.
pub struct SourceData {
    bytes: std::sync::Arc<Vec<u8>>,
}

impl Dispatch2<ExtDataControlSourceV1, App> for SourceData {
    fn event(&self, app: &mut App, source: &ExtDataControlSourceV1, event: ext_data_control_source_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        match event {
            ext_data_control_source_v1::Event::Send { fd, .. } => {
                let bytes = self.bytes.clone();
                std::thread::spawn(move || {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(&bytes);
                });
            }
            ext_data_control_source_v1::Event::Cancelled => {
                source.destroy();
                if app.clip_source.as_ref() == Some(source) {
                    app.clip_source = None;
                }
            }
            _ => {}
        }
    }
}

/// A selection read in: what it was offered as, and its bytes.
pub(super) type ClipRead = (String, Vec<u8>);

impl App {
    /// The device is per seat: made once the seat is known.
    pub(super) fn watch_clipboard(&mut self) {
        if self.clip_device.is_some() {
            return;
        }
        let (Some(manager), Some(seat)) = (&self.clip_manager, &self.seat) else { return };
        self.clip_device = Some(manager.get_data_device(seat, &self.qh, DeviceData));
    }

    /// A new selection: read it, unless it's secret or something we can't keep.
    fn clipboard_offer(&mut self, offer: ExtDataControlOfferV1) {
        let mimes = offer.data::<OfferData>().map(|d| d.0.lock().unwrap().clone()).unwrap_or_default();
        if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
            offer.destroy();
            return;
        }
        let mime = mimes.iter().find(|m| m.as_str() == "image/png").or_else(|| TEXT.iter().find_map(|t| mimes.iter().find(|m| m.as_str() == *t))).cloned();
        let Some(mime) = mime else {
            offer.destroy();
            return;
        };
        let Ok((reader, writer)) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC) else {
            offer.destroy();
            return;
        };
        {
            use std::os::fd::AsFd;
            offer.receive(mime.clone(), writer.as_fd());
        }
        drop(writer);
        offer.destroy();
        let sender = self.clip_sender.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = std::fs::File::from(reader).take(LIMIT as u64).read_to_end(&mut bytes);
            if !bytes.is_empty() {
                let _ = sender.send((mime, bytes));
            }
        });
    }

    /// Bytes from a selection: into the history.
    pub(super) fn clipboard_read(&mut self, (mime, bytes): ClipRead) {
        let content = if mime.starts_with("image/") {
            let Some(thumb) = crate::images::decode(&bytes, 160) else { return };
            let (width, height) = png_size(&bytes).unwrap_or((thumb.width, thumb.height));
            crate::images::give_back_memory();
            Content::Image { png: std::sync::Arc::new(bytes), thumb, width, height }
        } else {
            Content::Text(String::from_utf8_lossy(&bytes).into_owned())
        };
        self.model.clipboard.add(content);
        if let Some(index) = self.panel_index(PanelKind::Clipboard) {
            self.panels[index].surf.dirty = true;
        }
    }

    /// Offers a clip from the history as the selection again.
    pub(super) fn clipboard_copy(&mut self, id: u64) {
        let (Some(manager), Some(device)) = (&self.clip_manager, &self.clip_device) else { return };
        let Some(clip) = self.model.clipboard.get(id) else { return };
        let (bytes, mimes): (Vec<u8>, Vec<&str>) = match &clip.content {
            Content::Text(text) => (text.clone().into_bytes(), TEXT.to_vec()),
            Content::Image { png, .. } => ((**png).clone(), vec!["image/png"]),
        };
        let source = manager.create_data_source(&self.qh, SourceData { bytes: std::sync::Arc::new(bytes) });
        for mime in mimes {
            source.offer(mime.to_string());
        }
        device.set_selection(Some(&source));
        if let Some(old) = self.clip_source.replace(source) {
            old.destroy();
        }
        self.close_panel(PanelKind::Clipboard);
    }

    pub(super) fn toggle_clipboard(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Clipboard).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_panel(PanelKind::Clipboard),
            (_, false) => {
                if self.clip_manager.is_none() {
                    log::warn!("this compositor has no data-control: no clipboard history");
                    return;
                }
                self.close_menu();
                self.clip_picker = Some(Picker::new());
                self.open_panel(PanelKind::Clipboard, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
            }
        }
    }

    pub(super) fn clipboard_action(&mut self, action: &Action) {
        match action {
            Action::ClipCopy(id) => return self.clipboard_copy(*id),
            Action::ClipForget(id) => self.model.clipboard.forget(*id),
            Action::ClipClear => self.model.clipboard = Default::default(),
            Action::ClipMove(by) => {
                if let Some(picker) = self.clip_picker.as_mut() {
                    picker.step(*by);
                }
            }
            _ => return,
        }
        if let Some(index) = self.panel_index(PanelKind::Clipboard) {
            self.panels[index].surf.dirty = true;
        }
    }

    pub(super) fn clipboard_key(&mut self, event: &KeyEvent) {
        let Some(picker) = self.clip_picker.as_mut() else { return };
        match event.keysym {
            Keysym::Escape => return self.close_panel(PanelKind::Clipboard),
            Keysym::Return | Keysym::KP_Enter => {
                if let Some(id) = picker.shown.get(picker.selected).copied() {
                    self.clipboard_copy(id);
                }
                return;
            }
            Keysym::Up | Keysym::ISO_Left_Tab => picker.step(-1),
            Keysym::Down | Keysym::Tab => picker.step(1),
            Keysym::Page_Up => picker.step(-8),
            Keysym::Page_Down => picker.step(8),
            Keysym::Delete => {
                if let Some(id) = picker.shown.get(picker.selected).copied() {
                    self.model.clipboard.forget(id);
                }
            }
            Keysym::BackSpace => {
                picker.query.pop();
                picker.selected = 0;
            }
            _ => match &event.utf8 {
                Some(text) if !self.modifiers.ctrl && !self.modifiers.alt && text.chars().all(|c| !c.is_control()) => {
                    picker.query.push_str(text);
                    picker.selected = 0;
                }
                _ => return,
            },
        }
        if let Some(index) = self.panel_index(PanelKind::Clipboard) {
            self.panels[index].surf.dirty = true;
        }
    }
}

/// A PNG's size, from its header.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG") {
        return None;
    }
    Some((u32::from_be_bytes(bytes[16..20].try_into().ok()?), u32::from_be_bytes(bytes[20..24].try_into().ok()?)))
}

pub(super) fn bind_clipboard(globals: &smithay_client_toolkit::reexports::client::globals::GlobalList, qh: &QueueHandle<App>) -> Option<ExtDataControlManagerV1> {
    globals.bind::<ExtDataControlManagerV1, App, Silent>(qh, 1..=1, Silent).ok()
}
