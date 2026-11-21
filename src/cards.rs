//! Cards that aren't menus: notification pop-ups (and their entries in the centre), and the
//! little level that shows when the volume or brightness changes.

use crate::model::{Command, Notification};
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};

pub const TOAST_WIDTH: f32 = 380.0;
const PAD: f32 = 16.0;
const PICTURE: f32 = 44.0;

/// "now", "5 min", "2 h", then the time or the date.
pub fn ago(time: chrono::DateTime<chrono::Local>) -> String {
    let now = chrono::Local::now();
    let minutes = (now - time).num_minutes();
    if minutes < 1 {
        "now".into()
    } else if minutes < 60 {
        format!("{minutes} min")
    } else if minutes < 12 * 60 {
        format!("{} h", minutes / 60)
    } else if time.date_naive() == now.date_naive() {
        time.format("%H:%M").to_string()
    } else {
        time.format("%b %-d").to_string()
    }
}

fn default_action(n: &Notification) -> Option<Action> {
    n.actions.iter().any(|(key, _)| key == "default").then(|| Action::Send(Command::NotificationInvoke(n.id, "default".into())))
}

/// Lays one notification out at (x, y), `w` wide: picture, app and time, summary, body.
/// Returns the pieces and the height used.
#[allow(clippy::too_many_arguments)]
fn notification(theme: &Theme, text: &mut Text, n: &Notification, key: &str, x: f32, y: f32, w: f32, lines: usize) -> (Vec<Piece>, f32) {
    let t = theme;
    let small = Font::new(t.text - 2.0);
    let title = Font::new(t.text + 1.0).weight(500);
    let body = Font::new(t.text - 1.0);
    let has_picture = n.image.is_some();
    let left = if has_picture { PICTURE + 12.0 } else { 0.0 };
    let room = w - left - 26.0;
    let (_, sh) = text.measure("Ag", small);
    let (_, th) = text.measure("Ag", title);
    let (_, bh) = text.measure("Ag", body);
    let summary = text.wrap(&n.summary, title, room, 2);
    let body_lines = if n.body.is_empty() { Vec::new() } else { text.wrap(&n.body, body, w - left - 4.0, lines) };

    let mut piece = Piece::new(key, w, 0.0);
    if let Some(image) = &n.image {
        piece.prims.push(Prim::Image { x: 0.0, y: 2.0, w: PICTURE, h: PICTURE, image: image.clone(), tint: None, radius: 10.0 });
    }
    let app = if n.app.is_empty() { String::new() } else { format!("{} · ", n.app) };
    let mut cy = 0.0;
    piece.prims.push(Prim::Text { x: left, y: cy, text: format!("{app}{}", ago(n.time)), font: small, color: t.color_soft.alpha(0.75) });
    if n.urgency >= 2 {
        let (aw, _) = text.measure(&format!("{app}{}", ago(n.time)), small);
        piece.prims.push(Prim::Rect { x: left + aw + 6.0, y: cy + sh / 2.0 - 3.0, w: 6.0, h: 6.0, radius: 3.0, color: t.warn });
    }
    cy += sh;
    for line in summary {
        piece.prims.push(Prim::Text { x: left, y: cy, text: line, font: title, color: t.color_text });
        cy += th - 2.0;
    }
    cy += 2.0;
    for line in body_lines {
        piece.prims.push(Prim::Text { x: left, y: cy, text: line, font: body, color: t.color_soft });
        cy += bh - 3.0;
    }
    let h = cy.max(if has_picture { PICTURE + 4.0 } else { 0.0 }) + 2.0;
    piece.h = h;
    piece.x = x;
    piece.y = y;
    piece.pad = 8.0;

    let mut close = Piece::new(format!("{key}.close"), 18.0, 18.0);
    close.prims.push(Prim::Icon { x: 1.0, y: 1.0, size: 16.0, name: "close".into(), color: t.color_soft });
    close.x = x + w - 18.0;
    close.y = y - 1.0;
    close.lift = true;
    close.pad = 6.0;
    close.actions.left = Some(Action::Both(Box::new(Action::Send(Command::NotificationDismiss(n.id))), Box::new(Action::HideToast(n.id))));
    (vec![piece, close], h)
}

/// The pop-ups, newest on top, as one column of cards.
pub fn toasts(theme: &Theme, text: &mut Text, list: &[&Notification]) -> (f32, f32, Vec<Piece>) {
    let t = theme;
    let mut pieces = Vec::new();
    let mut y = 0.0;
    let inner = TOAST_WIDTH - PAD * 2.0;
    for n in list {
        let key = format!("toast.{}", n.id);
        let (mut parts, content_h) = notification(t, text, n, &format!("{key}.body"), PAD, y + PAD, inner, 3);
        let mut h = content_h + PAD * 2.0;

        // Its actions, as buttons under it
        let buttons: Vec<&(String, String)> = n.actions.iter().filter(|(k, label)| k != "default" && !label.is_empty()).collect();
        if !buttons.is_empty() {
            let font = Font::new(t.text - 1.0).weight(500);
            let mut bx = PAD;
            let by = y + PAD + content_h + 4.0;
            for (key_name, label) in buttons.into_iter().take(3) {
                let (lw, lh) = text.measure(label, font);
                let bw = lw + 24.0;
                let mut button = Piece::new(format!("{key}.action.{key_name}"), bw, 30.0);
                button.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: 30.0, radius: 15.0, color: t.secondary_container });
                button.prims.push(Prim::Text { x: 12.0, y: (30.0 - lh) / 2.0, text: label.clone(), font, color: t.on_secondary_container });
                button.x = bx;
                button.y = by;
                button.lift = true;
                button.actions.left = Some(Action::Both(Box::new(Action::Send(Command::NotificationInvoke(n.id, key_name.clone()))), Box::new(Action::HideToast(n.id))));
                bx += bw + 8.0;
                parts.push(button);
            }
            h += 38.0;
        }

        // A click on the card does its main thing, or just puts it away
        let mut card = Piece::new(format!("{key}.card"), TOAST_WIDTH, h);
        card.prims.push(Prim::Card { x: 0.0, y: 0.0, w: TOAST_WIDTH, h, radius: 18.0 });
        card.pad = 14.0;
        card.y = y;
        card.blocks = true;
        card.actions.left = Some(match default_action(n) {
            Some(action) => Action::Both(Box::new(action), Box::new(Action::HideToast(n.id))),
            None => Action::HideToast(n.id),
        });
        pieces.push(card);
        // The body sits on the card: clicking it is clicking the card
        if let Some(body) = parts.first_mut() {
            body.actions.left = pieces.last().and_then(|c| c.actions.left.clone());
        }
        pieces.extend(parts);
        y += h + 10.0;
    }
    (TOAST_WIDTH, (y - 10.0).max(0.0), pieces)
}

/// A notification in the centre: what the pop-up showed, with its × to let it go.
pub fn entry(theme: &Theme, text: &mut Text, n: &Notification, x: f32, y: f32, w: f32) -> (Vec<Piece>, f32) {
    let key = format!("centre.{}", n.id);
    let (mut parts, h) = notification(theme, text, n, &key, x + 6.0, y + 8.0, w - 12.0, 2);
    if let Some(first) = parts.first_mut() {
        first.highlight = true;
        first.actions.left = default_action(n).map(|action| Action::Both(Box::new(action), Box::new(Action::CloseMenu)));
        // The row lights up a little past its text
        first.y -= 6.0;
        first.h += 12.0;
        for prim in &mut first.prims {
            match prim {
                Prim::Text { y, .. } | Prim::Image { y, .. } | Prim::Rect { y, .. } => *y += 6.0,
                _ => {}
            }
        }
    }
    if let Some(close) = parts.get_mut(1) {
        close.actions.left = Some(Action::Send(Command::NotificationDismiss(n.id)));
    }
    (parts, h + 16.0)
}

/// What the little level shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Level {
    Volume { value: f32, muted: bool },
    Brightness(f32),
    Layout(String),
}

pub const PEEK_WIDTH: f32 = 272.0;

/// A workspace at a glance, for the bar's page dot under the pointer: its windows where they
/// sit (scaled into the card, the way the overview draws them), each with its app's icon, the
/// focused one outlined in the accent. Returns the card's size and its pieces.
pub fn peek(t: &Theme, text: &mut Text, ws: &crate::model::Workspace, icons: &std::collections::HashMap<String, Option<std::sync::Arc<crate::model::Image>>>) -> (f32, f32, Vec<Piece>) {
    let (w, pad, title_h, map_h) = (PEEK_WIDTH, 14.0, 26.0, 136.0);
    let h = pad + title_h + map_h + pad;
    let mut card = Piece::new("peek.card", w, h);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: t.radius });
    card.pad = 14.0;
    let font = Font::new(t.text - 1.0).weight(600).spacing(0.6);
    let count = ws.windows.len();
    let title = format!("WORKSPACE {} · {} WINDOW{}", ws.idx, count, if count == 1 { "" } else { "S" });
    let title = crate::bar::Ctx::ellipsize_with(text, &title, font, w - pad * 2.0);
    card.prims.push(Prim::Text { x: pad, y: pad, text: title, font, color: t.primary });
    let (mx, my, mw) = (pad, pad + title_h, w - pad * 2.0);
    for (window, (x, y, ww, wh)) in ws.windows.iter().zip(crate::overview::layout(&ws.windows, mw, map_h)) {
        let (x, y) = (mx + x, my + y);
        let edge = if window.focused { t.primary } else { t.hairline() };
        card.prims.push(Prim::Rect { x, y, w: ww, h: wh, radius: 6.0, color: edge });
        card.prims.push(Prim::Rect { x: x + 1.5, y: y + 1.5, w: (ww - 3.0).max(1.0), h: (wh - 3.0).max(1.0), radius: 5.0, color: t.container_high });
        let size = (ww.min(wh) * 0.45).clamp(10.0, 28.0);
        match icons.get(&window.app_id).cloned().flatten() {
            Some(image) => card.prims.push(Prim::Image { x: x + (ww - size) / 2.0, y: y + (wh - size) / 2.0, w: size, h: size, image, tint: None, radius: 0.0 }),
            None => {
                let letter: String = window.app_id.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                let font = Font::new(size * 0.7).weight(600);
                let (lw, lh) = text.measure(&letter, font);
                card.prims.push(Prim::Text { x: x + (ww - lw) / 2.0, y: y + (wh - lh) / 2.0, text: letter, font, color: t.color_soft });
            }
        }
    }
    (w, h, vec![card])
}

pub const SONG_WIDTH: f32 = 380.0;
pub const SONG_HEIGHT: f32 = 92.0;

/// The song that just started: its cover, its name and who sings it, and the music moving
/// under them (while the visualizer hears it).
pub fn song(t: &Theme, text: &mut Text, player: &crate::model::Player, art: Option<std::sync::Arc<crate::model::Image>>, levels: &[f32]) -> Vec<Piece> {
    let (w, h) = (SONG_WIDTH, SONG_HEIGHT);
    let mut card = Piece::new("song.card", w, h);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: t.radius });
    card.pad = 14.0;
    let side = h - 32.0;
    match art {
        Some(image) => card.prims.push(Prim::Image { x: 16.0, y: 16.0, w: side, h: side, image, tint: None, radius: 12.0 }),
        None => {
            card.prims.push(Prim::Rect { x: 16.0, y: 16.0, w: side, h: side, radius: 12.0, color: t.container_high });
            card.prims.push(Prim::Icon { x: 16.0 + (side - 26.0) / 2.0, y: 16.0 + (side - 26.0) / 2.0, size: 26.0, name: "music".into(), color: t.color_soft });
        }
    }
    let x = 16.0 + side + 14.0;
    let room = w - x - 18.0;
    let title_font = Font::new(t.text + 1.0).weight(500);
    let small = Font::new(t.text - 1.0);
    let title = crate::bar::Ctx::ellipsize_with(text, &player.title, title_font, room);
    let artist = crate::bar::Ctx::ellipsize_with(text, &player.artist, small, room);
    card.prims.push(Prim::Text { x, y: 16.0, text: title, font: title_font, color: t.color_text });
    card.prims.push(Prim::Text { x, y: 38.0, text: artist, font: small, color: t.color_soft });
    crate::widgets::bars(&mut card, t, levels, x - 2.0, room + 4.0, h - 22.0, 8.0);
    vec![card]
}

pub const OSD_WIDTH: f32 = 300.0;
pub const OSD_HEIGHT: f32 = 56.0;

pub fn osd(theme: &Theme, text: &mut Text, level: &Level) -> Vec<Piece> {
    let t = theme;
    let (w, h) = (OSD_WIDTH, OSD_HEIGHT);
    let mut card = Piece::new("osd.card", w, h);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w, h, radius: h / 2.0 });
    card.pad = 14.0;
    let mut content = Piece::new("osd.level", w - 40.0, 24.0);
    content.x = 20.0;
    content.y = (h - 24.0) / 2.0;
    let font = Font::new(t.text).tnum();
    match level {
        Level::Layout(name) => {
            content.prims.push(Prim::Icon { x: 0.0, y: 0.0, size: 22.0, name: "keyboard".into(), color: t.color_text });
            let (_, th) = text.measure(name, Font::new(t.text + 1.0));
            content.prims.push(Prim::Text { x: 36.0, y: (24.0 - th) / 2.0, text: name.clone(), font: Font::new(t.text + 1.0).weight(500), color: t.color_text });
        }
        Level::Volume { value, .. } | Level::Brightness(value) => {
            let (icon, shown) = match level {
                Level::Volume { muted: true, .. } => ("speaker-x", 0.0),
                Level::Volume { value, .. } if *value > 0.5 => ("speaker-high", *value),
                Level::Volume { value, .. } => ("speaker-low", *value),
                _ => ("brightness", *value),
            };
            let _ = value;
            content.prims.push(Prim::Icon { x: 0.0, y: 1.0, size: 22.0, name: icon.into(), color: t.color_text });
            let label = format!("{}%", (shown * 100.0).round());
            let (lw, lh) = text.measure("100%", font);
            let track = content.w - 36.0 - lw - 14.0;
            let fill = shown.clamp(0.0, 1.0);
            content.prims.push(Prim::Rect { x: 36.0, y: 6.0, w: track, h: 12.0, radius: 6.0, color: t.secondary_container });
            content.prims.push(Prim::Rect { x: 36.0, y: 6.0, w: (track * fill).max(12.0), h: 12.0, radius: 6.0, color: t.primary });
            content.prims.push(Prim::Text { x: content.w - lw, y: (24.0 - lh) / 2.0, text: label, font, color: t.color_soft });
        }
    }
    content.pad = 16.0;
    vec![card, content]
}

pub const AUTH_WIDTH: f32 = 420.0;

/// A question with a password field: polkit's, or a wifi network's.
pub struct Ask<'a> {
    pub key: &'a str,
    pub icon: &'a str,
    pub title: &'a str,
    pub message: &'a str,
    /// A quiet line under the message (whose password it is).
    pub note: &'a str,
    pub busy: bool,
    pub error: &'a str,
    pub ok: &'a str,
    pub ok_action: Action,
    pub cancel_action: Action,
}

/// Polkit's question: what it's for, whose password, a field where each character shows as a
/// small dot (never the character), and the two answers. Centred on a `w`×`h` surface.
pub fn auth(theme: &Theme, text: &mut Text, request: &crate::model::AuthRequest, typed: usize, w: f32, h: f32) -> Vec<Piece> {
    let mut who = format!("Password for {}", request.user);
    if request.waiting > 0 {
        who.push_str(&format!(" · {} more waiting", request.waiting));
    }
    let ask = Ask {
        key: "auth",
        icon: "lock",
        title: "Authentication required",
        message: &request.message,
        note: &who,
        busy: request.busy,
        error: &request.error,
        ok: "Authenticate",
        ok_action: Action::AuthSubmit,
        cancel_action: Action::AuthCancel,
    };
    password_card(theme, text, &ask, typed, w, h)
}

/// The card a password is typed into, centred on a `w`×`h` surface.
pub fn password_card(theme: &Theme, text: &mut Text, ask: &Ask<'_>, typed: usize, w: f32, h: f32) -> Vec<Piece> {
    let t = theme;
    let pad = 24.0;
    let inner = AUTH_WIDTH - pad * 2.0;
    let mut pieces = Vec::new();
    let mut y = pad;
    let k = ask.key;

    let mut head = Piece::new(format!("{k}.head"), inner, 48.0);
    head.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: 48.0, h: 48.0, radius: 24.0, color: t.primary_container });
    head.prims.push(Prim::Icon { x: 13.0, y: 13.0, size: 22.0, name: ask.icon.into(), color: t.on_primary_container });
    let title = Font::new(t.text + 5.0).weight(500);
    let (_, th) = text.measure("Ag", title);
    head.prims.push(Prim::Text { x: 62.0, y: (48.0 - th) / 2.0, text: crate::bar::Ctx::ellipsize_with(text, ask.title, title, inner - 62.0), font: title, color: t.color_text });
    head.pad = 14.0;
    head.x = pad;
    head.y = y;
    pieces.push(head);
    y += 60.0;

    let body = Font::new(t.text);
    let small = Font::new(t.text - 2.0);
    let (_, bh) = text.measure("Ag", body);
    let (_, sh) = text.measure("Ag", small);
    let lines = text.wrap(ask.message, body, inner, 4);
    let mut message = Piece::new(format!("{k}.message"), inner, lines.len() as f32 * (bh - 2.0) + sh + 4.0);
    let mut ly = 0.0;
    for line in lines {
        message.prims.push(Prim::Text { x: 0.0, y: ly, text: line, font: body, color: t.color_soft });
        ly += bh - 2.0;
    }
    message.prims.push(Prim::Text { x: 0.0, y: ly + 4.0, text: ask.note.to_string(), font: small, color: t.color_soft.alpha(0.7) });
    message.x = pad;
    message.y = y;
    y += message.h + 12.0;
    pieces.push(message);

    // The field: a star per character typed, a caret after them
    let field_h = 46.0;
    let mut field = Piece::new(format!("{k}.field"), inner, field_h);
    field.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: inner, h: field_h, radius: 14.0, color: t.container_highest });
    if ask.busy {
        let (_, ch) = text.measure("Ag", body);
        field.prims.push(Prim::Text { x: 16.0, y: (field_h - ch) / 2.0, text: "Checking…".into(), font: body, color: t.color_soft });
    } else {
        let room = ((inner - 40.0) / 16.0) as usize;
        for i in 0..typed.min(room) {
            field.prims.push(Prim::Rect { x: 18.0 + i as f32 * 16.0, y: field_h / 2.0 - 4.0, w: 8.0, h: 8.0, radius: 4.0, color: t.color_text });
        }
        let caret = 16.0 + typed.min(room) as f32 * 16.0 + if typed > 0 { 2.0 } else { 0.0 };
        field.prims.push(Prim::Rect { x: caret, y: 12.0, w: 2.0, h: field_h - 24.0, radius: 1.0, color: t.lit.alpha(0.9) });
    }
    field.pad = 12.0;
    field.x = pad;
    field.y = y;
    y += field_h + 8.0;
    pieces.push(field);

    if !ask.error.is_empty() {
        let mut error = Piece::new(format!("{k}.error"), inner, sh);
        error.prims.push(Prim::Text { x: 2.0, y: 0.0, text: ask.error.to_string(), font: small, color: t.warn });
        error.x = pad;
        error.y = y;
        y += sh + 6.0;
        pieces.push(error);
    }
    y += 6.0;

    // Cancel, then the way forward
    let font = Font::new(t.text).weight(500);
    let mut bx = pad + inner;
    for (key, label, strong, action) in [(format!("{k}.ok"), ask.ok, true, ask.ok_action.clone()), (format!("{k}.cancel"), "Cancel", false, ask.cancel_action.clone())] {
        let (lw, lh) = text.measure(label, font);
        let bw = lw + 32.0;
        bx -= bw;
        let mut button = Piece::new(key, bw, 38.0);
        button.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: 38.0, radius: 19.0, color: if strong { t.primary } else { t.secondary_container } });
        button.prims.push(Prim::Text { x: 16.0, y: (38.0 - lh) / 2.0, text: label.into(), font, color: if strong { t.on_primary } else { t.on_secondary_container } });
        button.lift = true;
        button.x = bx;
        button.y = y;
        button.actions.left = Some(action);
        pieces.push(button);
        bx -= 10.0;
    }
    y += 38.0 + pad;

    let mut card = Piece::new(format!("{k}.card"), AUTH_WIDTH, y);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w: AUTH_WIDTH, h: y, radius: 22.0 });
    card.pad = 14.0;
    card.blocks = true;
    pieces.insert(0, card);
    // Centred, a little above the middle
    let (x0, y0) = (((w - AUTH_WIDTH) / 2.0).round(), ((h - y) * 0.42).round().max(8.0));
    for piece in &mut pieces {
        piece.x += x0;
        piece.y += y0;
    }
    pieces
}

/// A friendly name for a Bluetooth service, from its UUID.
fn service_name(uuid: &str) -> &'static str {
    let short = uuid.get(4..8).unwrap_or("").to_ascii_lowercase();
    match short.as_str() {
        "110a" | "110b" | "110d" => "audio",
        "110c" | "110e" | "110f" => "media controls",
        "1108" | "1112" | "111e" | "111f" => "calls",
        "1124" | "1812" => "input",
        "1105" | "1106" => "file transfer",
        "1115" | "1116" => "networking",
        "112f" | "1132" => "contacts and messages",
        _ => "a service",
    }
}

/// A Bluetooth pairing question: a number to check, a code to type here or there, or whether to
/// let a device in. `typed` is what's been typed for a PIN or passkey. Centred on `w`×`h`.
pub fn pair_card(theme: &Theme, text: &mut Text, pairing: &crate::model::Pairing, typed: &str, w: f32, h: f32) -> Vec<Piece> {
    use crate::model::PairAsk;
    let t = theme;
    let pad = 24.0;
    let inner = AUTH_WIDTH - pad * 2.0;
    let device = pairing.device.as_str();
    let (title, message, code, field, ok): (String, String, Option<String>, bool, Option<&str>) = match &pairing.ask {
        PairAsk::Confirm(n) => (format!("Pair with {device}?"), format!("Check that {device} shows this number too."), Some(format!("{:03} {:03}", n / 1000, n % 1000)), false, Some("Pair")),
        PairAsk::Pin => (format!("Pair with {device}"), format!("Type the PIN {device} shows, or the one it expects (often 0000)."), None, true, Some("Pair")),
        PairAsk::Passkey => (format!("Pair with {device}"), format!("Type the number {device} shows."), None, true, Some("Pair")),
        PairAsk::ShowPin(pin) => (format!("Pair with {device}"), format!("Type this on {device}, then Enter there."), Some(pin.clone()), false, None),
        PairAsk::ShowPasskey(n) => (format!("Pair with {device}"), format!("Type this on {device}, then Enter there."), Some(format!("{n:06}")), false, None),
        PairAsk::Authorize => (format!("{device} wants to pair"), "Let it pair with this computer?".into(), None, false, Some("Allow")),
        PairAsk::Service(uuid) => (format!("{device} wants {}", service_name(uuid)), "Let it use that?".into(), None, false, Some("Allow")),
    };
    let mut pieces = Vec::new();
    let mut y = pad;
    let mut head = Piece::new("pair.head", inner, 48.0);
    head.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: 48.0, h: 48.0, radius: 24.0, color: t.primary_container });
    head.prims.push(Prim::Icon { x: 13.0, y: 13.0, size: 22.0, name: "bluetooth".into(), color: t.on_primary_container });
    let title_font = Font::new(t.text + 5.0).weight(500);
    let (_, th) = text.measure("Ag", title_font);
    let title = crate::bar::Ctx::ellipsize_with(text, &title, title_font, inner - 62.0);
    head.prims.push(Prim::Text { x: 62.0, y: (48.0 - th) / 2.0, text: title, font: title_font, color: t.color_text });
    head.x = pad;
    head.y = y;
    pieces.push(head);
    y += 60.0;
    let body = Font::new(t.text);
    let (_, bh) = text.measure("Ag", body);
    let lines = text.wrap(&message, body, inner, 3);
    let mut words = Piece::new("pair.message", inner, lines.len() as f32 * (bh - 2.0));
    for (i, line) in lines.into_iter().enumerate() {
        words.prims.push(Prim::Text { x: 0.0, y: i as f32 * (bh - 2.0), text: line, font: body, color: t.color_soft });
    }
    words.x = pad;
    words.y = y;
    y += words.h + 14.0;
    pieces.push(words);
    if let Some(code) = code {
        // The number, big and spaced, to read across the room
        let font = Font::new(t.text + 22.0).weight(500).tnum().spacing(4.0);
        let (cw, ch) = text.measure(&code, font);
        let mut big = Piece::new("pair.code", inner, ch + 8.0);
        big.prims.push(Prim::Text { x: ((inner - cw) / 2.0).round(), y: 4.0, text: code, font, color: t.primary });
        big.x = pad;
        big.y = y;
        y += ch + 20.0;
        pieces.push(big);
    }
    if field {
        let fh = 46.0;
        let mut entry = Piece::new("pair.field", inner, fh);
        entry.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: inner, h: fh, radius: 14.0, color: t.container_highest });
        let font = Font::new(t.text + 4.0).weight(500).tnum().spacing(2.0);
        let (tw, tht) = text.measure(typed, font);
        entry.prims.push(Prim::Text { x: 16.0, y: ((fh - tht) / 2.0).round(), text: typed.to_string(), font, color: t.color_text });
        entry.prims.push(Prim::Rect { x: 18.0 + tw, y: 12.0, w: 2.0, h: fh - 24.0, radius: 1.0, color: t.primary });
        entry.x = pad;
        entry.y = y;
        y += fh + 14.0;
        pieces.push(entry);
    }
    // No, then yes
    let font = Font::new(t.text).weight(500);
    let mut bx = pad + inner;
    let answer = if field { Some(typed.to_string()) } else { Some(String::new()) };
    let mut buttons: Vec<(&str, &str, bool, Action)> = Vec::new();
    if let Some(ok) = ok {
        buttons.push(("pair.ok", ok, true, Action::PairAnswer(answer)));
    }
    buttons.push(("pair.no", if ok.is_some() { "Don't" } else { "Close" }, false, Action::PairAnswer(None)));
    for (key, label, strong, action) in buttons {
        let (lw, lh) = text.measure(label, font);
        let bw = lw + 32.0;
        bx -= bw;
        let mut button = Piece::new(key, bw, 38.0);
        button.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: 38.0, radius: 19.0, color: if strong { t.primary } else { t.secondary_container } });
        button.prims.push(Prim::Text { x: 16.0, y: (38.0 - lh) / 2.0, text: label.into(), font, color: if strong { t.on_primary } else { t.on_secondary_container } });
        button.lift = true;
        button.actions.left = Some(action);
        button.x = bx;
        button.y = y;
        pieces.push(button);
        bx -= 10.0;
    }
    y += 38.0 + pad;
    let mut card = Piece::new("pair.card", AUTH_WIDTH, y);
    card.prims.push(Prim::Card { x: 0.0, y: 0.0, w: AUTH_WIDTH, h: y, radius: 22.0 });
    card.pad = 14.0;
    card.blocks = true;
    pieces.insert(0, card);
    let (x0, y0) = (((w - AUTH_WIDTH) / 2.0).round(), ((h - y) * 0.42).round().max(8.0));
    for piece in &mut pieces {
        piece.x += x0;
        piece.y += y0;
    }
    pieces
}
