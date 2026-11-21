//! The window overview: every workspace as a card, and in it a small map of its windows, in
//! their real columns and proportions, each with its app's icon and title. Click one to go to
//! it, middle-click to close it, drag it onto another card to move it there; type to find one.

use crate::model::{Command, Model, Window, Workspace};
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};
use std::collections::HashSet;

/// A window being carried: which, where it was picked up, where the pointer is now.
pub struct Carry {
    pub window: u64,
    pub app: String,
    pub from: (f32, f32),
    pub at: (f32, f32),
}

pub struct Overview {
    pub query: String,
    /// Which window the keys are on, by id.
    pub selected: Option<u64>,
    pub carry: Option<Carry>,
    /// The windows shown, in order, as last built (for the arrow keys).
    pub order: Vec<u64>,
}

pub struct View<'a> {
    pub model: &'a Model,
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub requested: &'a HashSet<String>,
    pub requests: Vec<Command>,
    pub output: &'a str,
    pub w: f32,
    pub h: f32,
}

/// Whether a window answers what's typed (its title or its app).
pub fn matches(window: &Window, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || window.title.to_lowercase().contains(&q) || window.app_id.to_lowercase().contains(&q) || crate::images::app(&window.app_id).is_some_and(|a| a.name.to_lowercase().contains(&q))
}

/// Where a workspace's windows go inside a box of `bw`×`bh`: their rects scaled together to fit,
/// centred. Windows the compositor gave no place get a row along the bottom.
pub fn layout(windows: &[Window], bw: f32, bh: f32) -> Vec<(f32, f32, f32, f32)> {
    let placed: Vec<(f32, f32, f32, f32)> = windows.iter().filter_map(|w| w.rect).collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y, w, h) in &placed {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(x + w);
        y1 = y1.max(y + h);
    }
    let scale = if placed.is_empty() { 1.0 } else { (bw / (x1 - x0).max(1.0)).min(bh / (y1 - y0).max(1.0)) };
    let (ox, oy) = if placed.is_empty() { (0.0, 0.0) } else { ((bw - (x1 - x0) * scale) / 2.0, (bh - (y1 - y0) * scale) / 2.0) };
    let unplaced = windows.iter().filter(|w| w.rect.is_none()).count().max(1) as f32;
    let mut next = 0.0;
    windows
        .iter()
        .map(|w| match w.rect {
            Some((x, y, ww, hh)) => (ox + (x - x0) * scale, oy + (y - y0) * scale, (ww * scale).max(6.0), (hh * scale).max(6.0)),
            None => {
                let slot = bw / unplaced;
                let r = (next * slot, bh * 0.25, slot - 6.0, bh * 0.5);
                next += 1.0;
                r
            }
        })
        .collect()
}

impl Overview {
    pub fn new(model: &Model) -> Self {
        let selected = model.compositor.workspaces.iter().flat_map(|w| &w.windows).find(|w| w.focused).map(|w| w.id);
        Self { query: String::new(), selected, carry: None, order: Vec::new() }
    }

    /// Moves the keys' choice along the windows shown.
    pub fn step(&mut self, by: i32) {
        if self.order.is_empty() {
            return;
        }
        let at = self.selected.and_then(|s| self.order.iter().position(|id| *id == s)).unwrap_or(0) as i32;
        let next = (at + by).rem_euclid(self.order.len() as i32) as usize;
        self.selected = Some(self.order[next]);
    }

    pub fn build(&mut self, v: &mut View<'_>) -> Vec<Piece> {
        let t = v.theme;
        let mut pieces = Vec::new();
        let span = (v.w * 0.86).round();
        let left = ((v.w - span) / 2.0).round();
        let top = (v.h * 0.06).round().max(24.0);

        // The question, in the middle
        let big = Font::new(t.text + 3.0);
        let (fw, fh) = (480f32.min(v.w - 48.0), 44.0);
        let mut field = Piece::new("ov.query", fw, fh);
        field.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: fh, radius: fh / 2.0, color: t.hairline() });
        field.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: fh - 2.0, radius: fh / 2.0 - 1.0, color: t.card_deep });
        field.prims.push(Prim::Icon { x: 16.0, y: (fh - 18.0) / 2.0, size: 18.0, name: "search".into(), color: t.color_soft });
        let (_, qh) = v.text.measure("Ag", big);
        if self.query.is_empty() {
            field.prims.push(Prim::Text { x: 44.0, y: (fh - qh) / 2.0, text: "Type to find a window".into(), font: big, color: t.color_soft.alpha(0.55) });
        } else {
            let (qw, _) = v.text.measure(&self.query, big);
            field.prims.push(Prim::Text { x: 44.0, y: (fh - qh) / 2.0, text: self.query.clone(), font: big, color: t.color_text });
            field.prims.push(Prim::Rect { x: 46.0 + qw, y: 11.0, w: 2.0, h: fh - 22.0, radius: 1.0, color: t.primary });
        }
        field.x = ((v.w - fw) / 2.0).round();
        field.y = top;
        field.blocks = true;
        pieces.push(field);

        // This screen's workspaces first, then the others', each screen its own row(s)
        let mut outputs: Vec<&str> = Vec::new();
        for w in &v.model.compositor.workspaces {
            if !outputs.contains(&w.output.as_str()) {
                outputs.push(&w.output);
            }
        }
        outputs.sort_by_key(|o| *o != v.output);
        let several = outputs.len() > 1;
        let mut y = top + fh + 30.0;
        let columns = 4usize;
        let gap = 18.0;
        let card_w = ((span - gap * (columns as f32 - 1.0)) / columns as f32).floor();
        let head_h = 34.0;
        let body_h = (card_w * 0.56).round();
        let card_h = head_h + body_h + 14.0;
        self.order.clear();
        let label_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
        let head_font = Font::new(t.text).weight(500);
        let title_font = Font::new(t.text - 2.0);
        for output in outputs {
            let spaces: Vec<&Workspace> = v.model.compositor.workspaces_on(output).collect();
            if several {
                let mut label = Piece::new(format!("ov.output.{output}"), span, 20.0);
                label.prims.push(Prim::Text { x: 0.0, y: 0.0, text: output.to_uppercase(), font: label_font, color: t.primary });
                label.x = left;
                label.y = y;
                pieces.push(label);
                y += 24.0;
            }
            for (i, ws) in spaces.iter().enumerate() {
                if i > 0 && i % columns == 0 {
                    y += card_h + gap;
                }
                let (cx, cy) = (left + (i % columns) as f32 * (card_w + gap), y);
                let current = v.model.compositor.active_on.get(output) == Some(&ws.id);
                let mut card = Piece::new(format!("ov.ws.{}", ws.id), card_w, card_h);
                card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: card_w, h: card_h, radius: 16.0, color: if current { t.primary } else { t.hairline() } });
                card.prims.push(Prim::Rect { x: 1.5, y: 1.5, w: card_w - 3.0, h: card_h - 3.0, radius: 14.5, color: t.card_deep });
                let name = if ws.name.is_empty() { format!("Workspace {}", ws.idx) } else { format!("{} · {}", ws.idx, ws.name) };
                card.prims.push(Prim::Text { x: 16.0, y: 10.0, text: name, font: head_font, color: if current { t.primary } else { t.color_text } });
                let count = match ws.windows.len() {
                    0 => "empty".to_string(),
                    1 => "1 window".to_string(),
                    n => format!("{n} windows"),
                };
                let (nw, _) = v.text.measure(&count, title_font);
                card.prims.push(Prim::Text { x: card_w - 16.0 - nw, y: 12.0, text: count, font: title_font, color: t.color_soft });
                card.highlight = true;
                card.actions.left = Some(Action::OverviewWorkspace(ws.id));
                card.x = cx;
                card.y = cy;
                pieces.push(card);

                // The map of its windows
                let (bx, by, bw, bh) = (cx + 12.0, cy + head_h + 2.0, card_w - 24.0, body_h);
                for (window, (wx, wy, ww, wh)) in ws.windows.iter().zip(layout(&ws.windows, bw, bh)) {
                    let found = matches(window, &self.query);
                    if found {
                        self.order.push(window.id);
                    }
                    let carried = self.carry.as_ref().is_some_and(|c| c.window == window.id);
                    let chosen = self.selected == Some(window.id);
                    let mut tile = Piece::new(format!("ov.win.{}", window.id), ww, wh);
                    let fill = if window.focused { t.primary_container } else { t.container_high };
                    let edge = if chosen { t.primary } else { t.hairline() };
                    tile.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: ww, h: wh, radius: 8.0, color: edge });
                    tile.prims.push(Prim::Rect { x: if chosen { 2.0 } else { 1.0 }, y: if chosen { 2.0 } else { 1.0 }, w: ww - if chosen { 4.0 } else { 2.0 }, h: wh - if chosen { 4.0 } else { 2.0 }, radius: 7.0, color: fill });
                    let icon = (ww.min(wh) * 0.42).clamp(10.0, 40.0);
                    let title_room = ww - 10.0;
                    let with_title = wh > icon + 30.0 && title_room > 40.0;
                    let iy = if with_title { (wh - icon - 18.0) / 2.0 } else { (wh - icon) / 2.0 };
                    match v.model.app_icons.get(&window.app_id).cloned().flatten() {
                        Some(image) => tile.prims.push(Prim::Image { x: (ww - icon) / 2.0, y: iy, w: icon, h: icon, image, tint: None, radius: 0.0 }),
                        None => {
                            if !v.model.app_icons.contains_key(&window.app_id) && !v.requested.contains(&format!("icon:{}", window.app_id)) {
                                v.requests.push(Command::AppIcon(window.app_id.clone()));
                            }
                            tile.prims.push(Prim::Icon { x: (ww - icon) / 2.0, y: iy, size: icon, name: "apps".into(), color: t.color_soft });
                        }
                    }
                    if with_title {
                        let title = crate::bar::Ctx::ellipsize_with(v.text, if window.title.is_empty() { &window.app_id } else { &window.title }, title_font, title_room);
                        let (tw, _) = v.text.measure(&title, title_font);
                        tile.prims.push(Prim::Text { x: ((ww - tw) / 2.0).max(5.0), y: iy + icon + 4.0, text: title, font: title_font, color: t.color_text });
                    }
                    tile.opacity = if carried { 0.3 } else if found { 1.0 } else { 0.25 };
                    tile.highlight = true;
                    tile.label = Some(window.title.clone());
                    tile.actions.left = Some(Action::OverviewWindow(window.id));
                    tile.actions.middle = Some(Action::OverviewClose(window.id));
                    tile.x = (bx + wx).round();
                    tile.y = (by + wy).round();
                    pieces.push(tile);
                }
                if ws.windows.is_empty() {
                    let (ew, eh) = v.text.measure("Nothing here", title_font);
                    let mut empty = Piece::new(format!("ov.empty.{}", ws.id), ew, eh);
                    empty.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Nothing here".into(), font: title_font, color: t.color_soft.alpha(0.6) });
                    empty.x = (bx + (bw - ew) / 2.0).round();
                    empty.y = (by + (bh - eh) / 2.0).round();
                    pieces.push(empty);
                }
            }
            y += card_h + gap + 6.0;
        }
        // Keys land on a window that's shown
        if self.selected.is_none_or(|s| !self.order.contains(&s)) {
            self.selected = self.order.first().copied();
        }

        // The window being carried follows the pointer
        if let Some(carry) = &self.carry {
            let (gw, gh) = (120.0, 76.0);
            let mut ghost = Piece::new("ov.ghost", gw, gh);
            ghost.prims.push(Prim::Card { x: 0.0, y: 0.0, w: gw, h: gh, radius: 12.0 });
            match v.model.app_icons.get(&carry.app).cloned().flatten() {
                Some(image) => ghost.prims.push(Prim::Image { x: (gw - 36.0) / 2.0, y: (gh - 36.0) / 2.0, w: 36.0, h: 36.0, image, tint: None, radius: 0.0 }),
                None => ghost.prims.push(Prim::Icon { x: (gw - 32.0) / 2.0, y: (gh - 32.0) / 2.0, size: 32.0, name: "apps".into(), color: t.color_soft }),
            }
            ghost.pad = 14.0;
            ghost.x = (carry.at.0 - gw / 2.0).round();
            ghost.y = (carry.at.1 - gh / 2.0).round();
            pieces.push(ghost);
        }

        let hint = "Click goes there · drag moves a window · middle-click closes · ←→ choose · Enter goes · Esc closes";
        let font = Font::new(t.text - 2.0);
        let (hw, hh) = v.text.measure(hint, font);
        let mut footer = Piece::new("ov.footer", hw, hh);
        footer.prims.push(Prim::Text { x: 0.0, y: 0.0, text: hint.into(), font, color: t.color_soft.alpha(0.7) });
        footer.x = ((v.w - hw) / 2.0).round();
        footer.y = (v.h - 22.0 - hh).round();
        pieces.push(footer);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, rect: Option<(f32, f32, f32, f32)>) -> Window {
        Window { id, app_id: "kitty".into(), title: format!("Shell {id}"), focused: false, rect, floating: false }
    }

    #[test]
    fn maps_windows_into_the_card() {
        // Two columns, 1000 and 500 wide with a gap, into a box 200×100
        let windows = vec![window(1, Some((0.0, 0.0, 1000.0, 1000.0))), window(2, Some((1016.0, 0.0, 500.0, 1000.0)))];
        let rects = layout(&windows, 200.0, 100.0);
        let scale = 100.0 / 1000.0;
        assert!((rects[0].2 - 1000.0 * scale).abs() < 0.01 && (rects[1].0 - rects[0].0 - 1016.0 * scale).abs() < 0.01);
        assert!(rects.iter().all(|(x, y, w, h)| *x >= 0.0 && *y >= 0.0 && x + w <= 200.01 && y + h <= 100.01));
        // A window with no place still gets one
        let rects = layout(&[window(3, None)], 200.0, 100.0);
        assert!(rects[0].2 > 0.0);
        assert!(matches(&windows[0], "shell 1") && !matches(&windows[0], "firefox"));
    }
}
