//! Text, shaped by cosmic-text and drawn glyph by glyph into tiny-skia pixmaps. Poppins ships
//! inside the bar; system fonts fill in for what it lacks (other scripts, emoji).

use crate::theme::Rgba;
use cosmic_text::{Attrs, Buffer, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight};
use std::collections::HashMap;

static FONTS: [&[u8]; 4] = [
    include_bytes!("../../assets/fonts/GoogleSansFlex.ttf"),
    include_bytes!("../../assets/fonts/Poppins-Regular.ttf"),
    include_bytes!("../../assets/fonts/Poppins-Medium.ttf"),
    include_bytes!("../../assets/fonts/Poppins-SemiBold.ttf"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Font {
    pub size: f32,
    pub weight: u16,
    /// Tabular figures, so numbers don't make things jump.
    pub tnum: bool,
    /// Extra room between letters, in pixels.
    pub spacing: f32,
    /// Emoji in colour first: some have a plain glyph in text fonts that would win otherwise.
    pub emoji: bool,
}

impl Font {
    pub fn new(size: f32) -> Self {
        Self { size, weight: 400, tnum: false, spacing: 0.0, emoji: false }
    }
    pub fn weight(self, weight: u16) -> Self {
        Self { weight, ..self }
    }
    pub fn tnum(self) -> Self {
        Self { tnum: true, ..self }
    }
    pub fn spacing(self, spacing: f32) -> Self {
        Self { spacing, ..self }
    }
    pub fn emoji(self) -> Self {
        Self { emoji: true, ..self }
    }
    pub fn key(&self) -> (u32, u16, bool, i32) {
        // The emoji flag rides in the weight's top bit: no real weight comes near it
        ((self.size * 64.0) as u32, self.weight | if self.emoji { 0x8000 } else { 0 }, self.tnum, (self.spacing * 64.0) as i32)
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct Key {
    text: String,
    font: (u32, u16, bool, i32),
}

pub struct Text {
    fonts: FontSystem,
    swash: SwashCache,
    family: String,
    layouts: HashMap<Key, (Buffer, f32, u64)>,
    frame: u64,
}

/// Where a text may be drawn, and how its ends fade out (for text that slides by).
#[derive(Clone, Copy)]
pub struct Clip {
    pub left: f32,
    pub right: f32,
    pub fade: f32,
}

impl Text {
    pub fn new(family: &str) -> Self {
        let mut fonts = FontSystem::new();
        // Read where they are in the program, not copied (Google Sans Flex alone is 4 MB)
        for data in FONTS {
            fonts.db_mut().load_font_source(cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(data)));
        }
        Self { fonts, swash: SwashCache::new(), family: family.to_string(), layouts: HashMap::new(), frame: 0 }
    }

    /// Text in another family from now on; layouts made in the old one go.
    pub fn set_family(&mut self, family: &str) {
        if self.family != family {
            self.family = family.to_string();
            self.layouts.clear();
        }
    }

    /// Forgets layouts nobody asked for in a while.
    pub fn tidy(&mut self) {
        self.frame += 1;
        if self.layouts.len() > 400 {
            let frame = self.frame;
            self.layouts.retain(|_, (_, _, used)| frame - *used < 600);
        }
    }

    fn layout(&mut self, text: &str, font: Font) -> &(Buffer, f32, u64) {
        let key = Key { text: text.to_string(), font: font.key() };
        let frame = self.frame;
        if !self.layouts.contains_key(&key) {
            let metrics = Metrics::new(font.size, (font.size * 1.4).ceil());
            let mut buffer = Buffer::new(&mut self.fonts, metrics);
            let mut features = FontFeatures::new();
            if font.tnum {
                features.enable(FeatureTag::new(b"tnum"));
            }
            let attrs = Attrs::new()
                .family(if font.emoji { Family::Name("Noto Color Emoji") } else { Family::Name(&self.family) })
                .weight(Weight(font.weight))
                .letter_spacing(font.spacing / font.size.max(1.0))
                .font_features(features);
            buffer.set_size(None, None);
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut self.fonts, false);
            let width = buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max);
            self.layouts.insert(key.clone(), (buffer, width, frame));
        }
        let entry = self.layouts.get_mut(&key).unwrap();
        entry.2 = frame;
        entry
    }

    /// Width and height of a line of text, in logical pixels.
    pub fn measure(&mut self, text: &str, font: Font) -> (f32, f32) {
        let width = self.layout(text, font).1;
        (width.ceil(), (font.size * 1.4).ceil())
    }

    /// Breaks text into lines no wider than `width`, at most `max` of them; when there's more,
    /// the last line ends in "…".
    pub fn wrap(&mut self, text: &str, font: Font, width: f32, max: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut line = String::new();
        let mut index = 0;
        while index < words.len() && lines.len() < max {
            let word = words[index];
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.measure(&candidate, font).0 <= width {
                line = candidate;
                index += 1;
                continue;
            }
            if line.is_empty() {
                // A word longer than the line: cut it where it stops fitting
                let chars: Vec<char> = word.chars().collect();
                let mut fit = chars.len();
                while fit > 1 && self.measure(&chars[..fit].iter().collect::<String>(), font).0 > width {
                    fit -= 1;
                }
                lines.push(chars[..fit].iter().collect());
                let rest: String = chars[fit..].iter().collect();
                if rest.is_empty() {
                    index += 1;
                } else {
                    // Put the rest back in place of the word
                    return {
                        let mut tail = vec![rest.as_str()];
                        tail.extend_from_slice(&words[index + 1..]);
                        let more = self.wrap(&tail.join(" "), font, width, max - lines.len());
                        lines.extend(more);
                        lines
                    };
                }
            } else {
                lines.push(std::mem::take(&mut line));
            }
        }
        if !line.is_empty() && lines.len() < max {
            lines.push(line);
        }
        if index < words.len()
            && let Some(last) = lines.last_mut()
        {
            // More than fits: the last line says so
            let mut cut = format!("{last} {}", words[index..].join(" "));
            while !cut.is_empty() && self.measure(&format!("{cut}…"), font).0 > width {
                cut.pop();
            }
            *last = format!("{}…", cut.trim_end());
        }
        lines
    }

    /// Draws a line of text with its box's top-left at (x, y), logical pixels, at `scale`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, pixmap: &mut tiny_skia::Pixmap, text: &str, font: Font, x: f32, y: f32, color: Rgba, scale: f32, clip: Option<Clip>) {
        if text.is_empty() || color.a <= 0.0 {
            return;
        }
        self.layout(text, font);
        let key = Key { text: text.to_string(), font: font.key() };
        let (buffer, _, _) = &self.layouts[&key];
        let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
        let data = pixmap.data_mut();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((x * scale, (y + run.line_y) * scale), scale);
                let Some(image) = self.swash.get_image(&mut self.fonts, physical.cache_key) else { continue };
                let left = physical.x + image.placement.left;
                let top = physical.y - image.placement.top;
                let (w, h) = (image.placement.width as i32, image.placement.height as i32);
                for row in 0..h {
                    let py = top + row;
                    if py < 0 || py >= ph {
                        continue;
                    }
                    for col in 0..w {
                        let px = left + col;
                        if px < 0 || px >= pw {
                            continue;
                        }
                        let mut cover = 1.0;
                        if let Some(clip) = clip {
                            let lx = px as f32 / scale;
                            if lx < clip.left || lx >= clip.right {
                                continue;
                            }
                            if clip.fade > 0.0 {
                                cover = ((lx - clip.left) / clip.fade).min((clip.right - lx) / clip.fade).clamp(0.0, 1.0);
                            }
                        }
                        let i = (row * w + col) as usize;
                        let (sr, sg, sb, sa) = match image.content {
                            SwashContent::Mask => {
                                let a = image.data[i] as f32 / 255.0 * color.a * cover;
                                (color.r * a, color.g * a, color.b * a, a)
                            }
                            SwashContent::SubpixelMask => {
                                let a = image.data[i * 4 + 1] as f32 / 255.0 * color.a * cover;
                                (color.r * a, color.g * a, color.b * a, a)
                            }
                            // Emoji keep their colours, premultiplied here
                            SwashContent::Color => {
                                let a = image.data[i * 4 + 3] as f32 / 255.0 * color.a * cover;
                                let c = |v: u8| v as f32 / 255.0 * a;
                                (c(image.data[i * 4]), c(image.data[i * 4 + 1]), c(image.data[i * 4 + 2]), a)
                            }
                        };
                        if sa <= 0.0 {
                            continue;
                        }
                        let o = ((py * pw + px) * 4) as usize;
                        let keep = 1.0 - sa;
                        data[o] = (sr * 255.0 + data[o] as f32 * keep).round().min(255.0) as u8;
                        data[o + 1] = (sg * 255.0 + data[o + 1] as f32 * keep).round().min(255.0) as u8;
                        data[o + 2] = (sb * 255.0 + data[o + 2] as f32 * keep).round().min(255.0) as u8;
                        data[o + 3] = (sa * 255.0 + data[o + 3] as f32 * keep).round().min(255.0) as u8;
                    }
                }
            }
        }
    }
}
