//! Clipboard history: what you've copied (text and pictures), newest first, kept in memory only,
//! and the picker that puts one back. (Watching the clipboard lives in `app::clipboard`.)

use crate::model::Image;
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// How many it keeps, and how many of them may be pictures (they're big).
const KEEP: usize = 50;
const KEEP_IMAGES: usize = 8;

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    Image { png: Arc<Vec<u8>>, thumb: Arc<Image>, width: u32, height: u32 },
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub id: u64,
    pub content: Content,
    pub time: chrono::DateTime<chrono::Local>,
    hash: u64,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    pub clips: Vec<Clip>,
    next: u64,
}

fn hash_of(content: &Content) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match content {
        Content::Text(text) => text.hash(&mut h),
        Content::Image { png, .. } => png.hash(&mut h),
    }
    h.finish()
}

impl History {
    /// Something was copied: on top, or back on top if it was already here.
    pub fn add(&mut self, content: Content) {
        if let Content::Text(text) = &content
            && text.trim().is_empty()
        {
            return;
        }
        let hash = hash_of(&content);
        let time = chrono::Local::now();
        if let Some(at) = self.clips.iter().position(|c| c.hash == hash) {
            let mut clip = self.clips.remove(at);
            clip.time = time;
            self.clips.insert(0, clip);
            return;
        }
        self.next += 1;
        self.clips.insert(0, Clip { id: self.next, content, time, hash });
        // Old pictures go first, then the oldest of anything
        let mut images = 0;
        self.clips.retain(|c| {
            if matches!(c.content, Content::Image { .. }) {
                images += 1;
                images <= KEEP_IMAGES
            } else {
                true
            }
        });
        self.clips.truncate(KEEP);
    }

    pub fn forget(&mut self, id: u64) {
        self.clips.retain(|c| c.id != id);
    }

    pub fn get(&self, id: u64) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    /// The ones a search finds (all of them for an empty one): pictures only match "image".
    pub fn find(&self, query: &str) -> Vec<&Clip> {
        let wanted = query.trim().to_lowercase();
        self.clips
            .iter()
            .filter(|c| {
                wanted.is_empty()
                    || match &c.content {
                        Content::Text(text) => text.to_lowercase().contains(&wanted),
                        Content::Image { .. } => "image picture".contains(&wanted),
                    }
            })
            .collect()
    }
}

/// "3 min ago"
fn ago(time: chrono::DateTime<chrono::Local>) -> String {
    let seconds = (chrono::Local::now() - time).num_seconds().max(0);
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86400 => format!("{} h ago", seconds / 3600),
        _ => time.format("%b %-d").to_string(),
    }
}

// ---------------------------------------------------------------- the picker

pub struct Picker {
    pub query: String,
    pub selected: usize,
    /// The ids shown, in order, as last built.
    pub shown: Vec<u64>,
}

const WIDTH: f32 = 640.0;
const ROW: f32 = 58.0;
const VISIBLE: usize = 8;
const PAD: f32 = 20.0;

impl Picker {
    pub fn new() -> Self {
        Self { query: String::new(), selected: 0, shown: Vec::new() }
    }

    pub fn step(&mut self, by: i32) {
        if !self.shown.is_empty() {
            self.selected = (self.selected as i32 + by).clamp(0, self.shown.len() as i32 - 1) as usize;
        }
    }

    /// A card in the middle: the search, the clips (newest first), a word on the keys.
    pub fn build(&mut self, history: &History, t: &Theme, text: &mut Text, w: f32, h: f32) -> Vec<Piece> {
        let found = history.find(&self.query);
        self.shown = found.iter().map(|c| c.id).collect();
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
        let cw = WIDTH.min(w - 32.0);
        let inner = cw - PAD * 2.0;
        let x0 = ((w - cw) / 2.0).round();
        let y0 = (h * 0.14).round();
        let mut pieces = Vec::new();

        let big = Font::new(t.text + 3.0);
        let (_, qh) = text.measure("Ag", big);
        let mut field = Piece::new("clip.query", inner, 44.0);
        field.prims.push(Prim::Icon { x: 0.0, y: 12.0, size: 20.0, name: "clipboard".into(), color: t.color_soft });
        if self.query.is_empty() {
            field.prims.push(Prim::Text { x: 32.0, y: (44.0 - qh) / 2.0, text: "Clipboard history".into(), font: big, color: t.color_soft.alpha(0.55) });
        } else {
            let shown = crate::bar::Ctx::ellipsize_with(text, &self.query, big, inner - 40.0);
            let (sw, _) = text.measure(&shown, big);
            field.prims.push(Prim::Text { x: 32.0, y: (44.0 - qh) / 2.0, text: shown, font: big, color: t.color_text });
            field.prims.push(Prim::Rect { x: 34.0 + sw, y: 11.0, w: 2.0, h: 22.0, radius: 1.0, color: t.primary });
        }
        field.x = x0 + PAD;
        field.y = y0 + PAD - 4.0;
        field.blocks = true;
        pieces.push(field);
        let mut y = y0 + PAD + 44.0;
        let mut line = Piece::new("clip.line", inner, 9.0);
        line.prims.push(Prim::Line { points: vec![(0.0, 4.5), (inner, 4.5)], width: 1.0, color: t.outline_variant });
        line.x = x0 + PAD;
        line.y = y;
        pieces.push(line);
        y += 13.0;

        let first = self.selected.saturating_sub(VISIBLE - 1).min(found.len().saturating_sub(VISIBLE));
        let main = Font::new(t.text);
        let small = Font::new(t.text - 2.0);
        let (_, mh) = text.measure("Ag", main);
        let (_, sh) = text.measure("Ag", small);
        for (index, clip) in found.iter().enumerate().skip(first).take(VISIBLE) {
            let mut row = Piece::new(format!("clip.row.{}", index - first), inner, ROW);
            if index == self.selected {
                row.prims.push(Prim::Rect { x: -6.0, y: 2.0, w: inner + 12.0, h: ROW - 4.0, radius: 14.0, color: t.secondary_container });
            }
            let (title, detail, left) = match &clip.content {
                Content::Text(body) => {
                    let one_line: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
                    let lines = body.lines().count();
                    let size = if lines > 1 { format!("{lines} lines") } else { format!("{} characters", body.chars().count()) };
                    row.prims.push(Prim::Icon { x: 8.0, y: (ROW - 20.0) / 2.0, size: 20.0, name: "clipboard".into(), color: t.color_soft });
                    (one_line, format!("{} · {size}", ago(clip.time)), 44.0)
                }
                Content::Image { thumb, width, height, .. } => {
                    let (tw, th) = (72.0, 44.0);
                    row.prims.push(Prim::Image { x: 0.0, y: (ROW - th) / 2.0, w: tw, h: th, image: thumb.clone(), tint: None, radius: 6.0 });
                    ("Picture".to_string(), format!("{} · {width}×{height}", ago(clip.time)), 86.0)
                }
            };
            let room = inner - left - 8.0;
            let title = crate::bar::Ctx::ellipsize_with(text, &title, main, room);
            let top = (ROW - mh - sh) / 2.0;
            row.prims.push(Prim::Text { x: left, y: top, text: title, font: main, color: t.color_text });
            row.prims.push(Prim::Text { x: left, y: top + mh - 1.0, text: detail, font: small, color: t.color_soft });
            row.highlight = true;
            row.actions.left = Some(Action::ClipCopy(clip.id));
            row.actions.middle = Some(Action::ClipForget(clip.id));
            row.actions.up = Some(Action::ClipMove(-1));
            row.actions.down = Some(Action::ClipMove(1));
            row.x = x0 + PAD;
            row.y = y;
            pieces.push(row);
            y += ROW;
        }
        if found.is_empty() {
            let words = if history.clips.is_empty() { "Nothing copied yet. What you copy from now on shows up here." } else { "Nothing like that in the history" };
            let mut none = Piece::new("clip.none", inner, 40.0);
            none.prims.push(Prim::Text { x: 4.0, y: 10.0, text: words.into(), font: small, color: t.color_soft });
            none.x = x0 + PAD;
            none.y = y;
            pieces.push(none);
            y += 40.0;
        }

        // The keys, and clear all
        let hint = "↑↓ choose · Enter copies · Delete forgets · Esc closes";
        let mut footer = Piece::new("clip.footer", inner - 90.0, 28.0);
        footer.prims.push(Prim::Text { x: 4.0, y: 6.0, text: hint.into(), font: Font::new(t.text - 3.0), color: t.color_soft.alpha(0.7) });
        footer.x = x0 + PAD;
        footer.y = y + 6.0;
        pieces.push(footer);
        if !history.clips.is_empty() {
            let label = "Clear all";
            let font = Font::new(t.text - 2.0).weight(500);
            let (lw, lh) = text.measure(label, font);
            let (bw, bh) = (lw + 22.0, 28.0);
            let mut clear = Piece::new("clip.clear", bw, bh);
            clear.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.hairline() });
            clear.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: bw - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
            clear.prims.push(Prim::Text { x: 11.0, y: ((bh - lh) / 2.0).round(), text: label.into(), font, color: t.color_text });
            clear.highlight = true;
            clear.actions.left = Some(Action::ClipClear);
            clear.x = x0 + cw - PAD - bw;
            clear.y = y + 6.0;
            pieces.push(clear);
        }
        let ch = y + 6.0 + 28.0 + PAD - y0;
        let mut card = crate::popups::card(t, cw, ch, 4);
        card.key = "clip.card".into();
        card.x = x0;
        card.y = y0;
        pieces.insert(0, card);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_newest_first_without_repeats() {
        let mut history = History::default();
        for word in ["one", "two", "three", "two", "   "] {
            history.add(Content::Text(word.into()));
        }
        let texts: Vec<&str> = history.clips.iter().map(|c| if let Content::Text(t) = &c.content { t.as_str() } else { "" }).collect();
        assert_eq!(texts, ["two", "three", "one"]);
        for n in 0..80 {
            history.add(Content::Text(format!("entry {n}")));
        }
        assert_eq!(history.clips.len(), KEEP);
        assert_eq!(history.find("entry 79").len(), 1);
        let id = history.clips[0].id;
        history.forget(id);
        assert!(history.get(id).is_none());
    }
}
