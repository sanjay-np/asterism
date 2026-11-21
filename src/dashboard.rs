//! The dashboard: how hard the machine is working, as graphs over the last two minutes, how
//! full the disks are, and what's running, with a way to end it. A wide card under the bar.

use crate::model::{Model, Process};
use crate::render::text::{Font, Text};
use crate::theme::{Rgba, Theme};
use crate::ui::{Action, Piece, Prim};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Cpu,
    Memory,
}

pub struct Dashboard {
    pub sort: Sort,
    /// A process whose End was clicked once: the next click ends it.
    pub armed: Option<u32>,
}

const PAD: f32 = 22.0;

/// What holds the session up: ending one of these ends everything, so no End button.
const ESSENTIAL: [&str; 15] = [
    "systemd", "(sd-pam)", "niri", "Hyprland", "asterism", "asterism-lock", "dbus-broker", "dbus-broker-launch", "dbus-daemon", "pipewire", "pipewire-pulse", "wireplumber", "Xwayland", "xwayland-satellite", "xdg-desktop-portal",
];

/// Whether the dashboard offers to end this one: yours, and not part of the session itself.
pub fn endable(p: &Process) -> bool {
    p.own && !ESSENTIAL.contains(&p.name.as_str()) && !p.name.starts_with("xdg-desktop-portal")
}
const ROWS: usize = 10;

/// "1.4 GB", "512 MB"
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e12 {
        format!("{:.1} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else {
        format!("{:.0} MB", b / 1e6)
    }
}

fn uptime(seconds: u64) -> String {
    let (d, h, m) = (seconds / 86400, seconds / 3600 % 24, seconds / 60 % 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        _ => format!("{d} d {h} h"),
    }
}

/// The processes to show, biggest first by the chosen measure.
pub fn ranked(list: &[Process], sort: Sort) -> Vec<&Process> {
    let mut list: Vec<&Process> = list.iter().collect();
    match sort {
        Sort::Cpu => list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.memory_kib.cmp(&a.memory_kib))),
        Sort::Memory => list.sort_by_key(|p| std::cmp::Reverse(p.memory_kib)),
    }
    list.truncate(ROWS);
    list
}

impl Dashboard {
    pub fn new() -> Self {
        Self { sort: Sort::Cpu, armed: None }
    }

    pub fn build(&self, model: &Model, t: &Theme, text: &mut Text, w: f32, top: f32) -> Vec<Piece> {
        let m = &model.machine;
        let cw = 1000f32.min(w - 32.0);
        let x0 = ((w - cw) / 2.0).round();
        let inner = cw - PAD * 2.0;
        let mut pieces = Vec::new();
        let mut y = top + PAD;
        let label_font = Font::new(t.text - 2.0).weight(600).spacing(1.0);
        let small = Font::new(t.text - 1.0);

        // Title, and the machine in a line
        let title_font = Font::new(t.text + 6.0).weight(500);
        let (_, th) = text.measure("Ag", title_font);
        let mut title = Piece::new("dash.title", inner, th);
        title.prims.push(Prim::Text { x: 0.0, y: 0.0, text: "Dashboard".into(), font: title_font, color: t.color_text });
        let mut about = format!("up {} · load {:.2}", uptime(m.uptime), m.load);
        if m.temperature > 0.0 {
            about.push_str(&format!(" · {:.0}°C", m.temperature));
        }
        let (aw, ah) = text.measure(&about, small);
        title.prims.push(Prim::Text { x: inner - aw, y: th - ah - 3.0, text: about, font: small, color: t.color_soft });
        title.x = x0 + PAD;
        title.y = y;
        pieces.push(title);
        y += th + 14.0;

        // A graph per measure: its name, what it reads now, its lines and their colours
        type Graph<'n> = (&'n str, String, Vec<Vec<f32>>, Vec<Rgba>);
        let mut graphs: Vec<Graph> = vec![
            ("CPU", format!("{:.0}%", m.cpu * 100.0), vec![m.history.cpu.clone()], vec![t.primary]),
            ("Memory", format!("{} of {}", size(m.memory_used_kib * 1024), size(m.memory_total_kib * 1024)), vec![m.history.memory.clone()], vec![t.primary]),
        ];
        if let Some(gpu) = m.gpu {
            graphs.push(("Graphics", format!("{:.0}%", gpu * 100.0), vec![m.history.gpu.clone()], vec![t.primary]));
        }
        let top_rate = m.history.download.iter().chain(&m.history.upload).fold(64.0 * 1024.0, |a: f64, b| a.max(*b));
        let scale = |v: &Vec<f64>| v.iter().map(|x| (*x / top_rate) as f32).collect::<Vec<f32>>();
        graphs.push(("Network", format!("↓ {}  ↑ {}", crate::bar::rate(m.download), crate::bar::rate(m.upload)), vec![scale(&m.history.download), scale(&m.history.upload)], vec![t.primary, t.palette.tertiary]));
        let gap = 12.0;
        let n = graphs.len() as f32;
        let gw = ((inner - gap * (n - 1.0)) / n).floor();
        let gh = 132.0;
        for (i, (name, value, series, colors)) in graphs.into_iter().enumerate() {
            let mut card = Piece::new(format!("dash.graph.{i}"), gw, gh);
            card.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: gw, h: gh, radius: 14.0, color: t.hairline() });
            card.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: gw - 2.0, h: gh - 2.0, radius: 13.0, color: t.container_high.alpha(0.5) });
            card.prims.push(Prim::Text { x: 14.0, y: 12.0, text: name.to_uppercase(), font: label_font, color: t.primary });
            let big = Font::new(t.text + 4.0).weight(500).tnum();
            let value = crate::bar::Ctx::ellipsize_with(text, &value, big, gw - 28.0);
            card.prims.push(Prim::Text { x: 14.0, y: 30.0, text: value, font: big, color: t.color_text });
            // The graph: oldest on the left, now on the right
            let (gx, gy, ww, hh) = (1.0, 66.0, gw - 2.0, gh - 67.0);
            for (k, (values, color)) in series.iter().zip(&colors).enumerate() {
                if values.len() < 2 {
                    continue;
                }
                let step = ww / 59.0;
                let start = gx + ww - step * (values.len() - 1) as f32;
                let points: Vec<(f32, f32)> = values.iter().enumerate().map(|(j, v)| (start + j as f32 * step, gy + hh - v.clamp(0.0, 1.0) * (hh - 6.0))).collect();
                if k == 0 {
                    card.prims.push(Prim::Area { points: points.clone(), base: gy + hh, color: color.alpha(0.18) });
                }
                card.prims.push(Prim::Line { points, width: 1.6, color: *color });
            }
            card.x = x0 + PAD + i as f32 * (gw + gap);
            card.y = y;
            pieces.push(card);
        }
        y += gh + 18.0;

        // The disks, side by side
        if !m.disks.is_empty() {
            let n = m.disks.len() as f32;
            let dw = ((inner - gap * (n - 1.0)) / n).floor();
            for (i, (point, used, total)) in m.disks.iter().enumerate() {
                let mut disk = Piece::new(format!("dash.disk.{i}"), dw, 40.0);
                let fill = *used as f32 / (*total).max(1) as f32;
                disk.prims.push(Prim::Text { x: 0.0, y: 0.0, text: point.clone(), font: Font::new(t.text - 1.0).weight(500), color: t.color_text });
                let words = format!("{} of {}", size(*used), size(*total));
                let (ww, _) = text.measure(&words, small);
                disk.prims.push(Prim::Text { x: dw - ww, y: 0.0, text: words, font: small, color: t.color_soft });
                disk.prims.push(Prim::Rect { x: 0.0, y: 26.0, w: dw, h: 5.0, radius: 2.5, color: t.outline_variant });
                disk.prims.push(Prim::Rect { x: 0.0, y: 26.0, w: (dw * fill).max(5.0), h: 5.0, radius: 2.5, color: if fill > 0.9 { t.warn } else { t.primary } });
                disk.x = x0 + PAD + i as f32 * (dw + gap);
                disk.y = y;
                pieces.push(disk);
            }
            y += 40.0 + 14.0;
        }

        // What's running
        let mut header = Piece::new("dash.procs.head", inner, 30.0);
        header.prims.push(Prim::Text { x: 0.0, y: 8.0, text: "WHAT'S RUNNING".into(), font: label_font, color: t.primary });
        header.x = x0 + PAD;
        header.y = y;
        pieces.push(header);
        // Sort: two small pills at the right
        let mut px = x0 + PAD + inner;
        for (key, label, sort) in [("dash.sort.memory", "Memory", Sort::Memory), ("dash.sort.cpu", "CPU", Sort::Cpu)] {
            let font = Font::new(t.text - 2.0).weight(500);
            let (lw, lh) = text.measure(label, font);
            let (bw, bh) = (lw + 22.0, 26.0);
            px -= bw;
            let mut pill = Piece::new(key, bw, bh);
            let on = self.sort == sort;
            if on {
                pill.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.secondary_container });
            } else {
                pill.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.hairline() });
                pill.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: bw - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
            }
            pill.prims.push(Prim::Text { x: 11.0, y: ((bh - lh) / 2.0).round(), text: label.into(), font, color: if on { t.on_secondary_container } else { t.color_text } });
            pill.highlight = true;
            pill.actions.left = Some(Action::DashSort(sort == Sort::Memory));
            pill.x = px;
            pill.y = y + 2.0;
            pieces.push(pill);
            px -= 8.0;
        }
        y += 36.0;
        let list = ranked(&model.processes, self.sort);
        if list.is_empty() {
            let mut wait = Piece::new("dash.procs.wait", inner, 30.0);
            wait.prims.push(Prim::Text { x: 0.0, y: 6.0, text: "Looking…".into(), font: small, color: t.color_soft });
            wait.x = x0 + PAD;
            wait.y = y;
            pieces.push(wait);
            y += 30.0;
        }
        let row_h = 34.0;
        let name_font = Font::new(t.text - 1.0);
        let figure = Font::new(t.text - 1.0).tnum();
        for (i, p) in list.iter().enumerate() {
            let mut row = Piece::new(format!("dash.proc.{i}"), inner, row_h);
            if i % 2 == 0 {
                row.prims.push(Prim::Rect { x: -8.0, y: 0.0, w: inner + 16.0, h: row_h, radius: 8.0, color: t.container_high.alpha(0.35) });
            }
            let (_, nh) = text.measure("Ag", name_font);
            let ty = ((row_h - nh) / 2.0).round();
            let name = crate::bar::Ctx::ellipsize_with(text, &p.name, name_font, inner * 0.45);
            row.prims.push(Prim::Text { x: 0.0, y: ty, text: name, font: name_font, color: t.color_text });
            let cpu = format!("{:.1}%", p.cpu * 100.0);
            let memory = size(p.memory_kib * 1024);
            let (cw1, _) = text.measure(&cpu, figure);
            let (mw, _) = text.measure(&memory, figure);
            let col_cpu = inner * 0.62;
            let col_mem = inner * 0.80;
            row.prims.push(Prim::Text { x: col_cpu - cw1, y: ty, text: cpu, font: figure, color: if self.sort == Sort::Cpu { t.color_text } else { t.color_soft } });
            row.prims.push(Prim::Text { x: col_mem - mw, y: ty, text: memory, font: figure, color: if self.sort == Sort::Memory { t.color_text } else { t.color_soft } });
            row.highlight = true;
            row.x = x0 + PAD;
            row.y = y;
            pieces.push(row);
            // End: only yours; a first click arms it, a second ends it
            if endable(p) {
                let armed = self.armed == Some(p.pid);
                let label = if armed { "End it?" } else { "End" };
                let font = Font::new(t.text - 2.0).weight(500);
                let (lw, lh) = text.measure(label, font);
                let (bw, bh) = (lw + 20.0, 24.0);
                let mut end = Piece::new(format!("dash.end.{i}"), bw, bh);
                if armed {
                    end.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.error_container });
                } else {
                    end.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: bw, h: bh, radius: bh / 2.0, color: t.hairline() });
                    end.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: bw - 2.0, h: bh - 2.0, radius: bh / 2.0 - 1.0, color: t.card_deep });
                }
                end.prims.push(Prim::Text { x: 10.0, y: ((bh - lh) / 2.0).round(), text: label.into(), font, color: if armed { t.palette.on_error_container } else { t.color_text } });
                end.highlight = true;
                end.actions.left = Some(Action::DashEnd(p.pid));
                end.x = x0 + PAD + inner - bw;
                end.y = y + (row_h - bh) / 2.0;
                pieces.push(end);
            }
            y += row_h;
        }
        let ch = y + PAD - top;
        let mut card = crate::popups::card(t, cw, ch, 1);
        card.key = "dash.card".into();
        card.x = x0;
        card.y = top;
        pieces.insert(0, card);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_sizes() {
        let p = |pid, cpu, memory_kib| Process { pid, name: format!("p{pid}"), cpu, memory_kib, own: true };
        let list = vec![p(1, 0.1, 500), p(2, 0.5, 100), p(3, 0.0, 9000)];
        assert_eq!(ranked(&list, Sort::Cpu).iter().map(|p| p.pid).collect::<Vec<_>>(), [2, 1, 3]);
        assert_eq!(ranked(&list, Sort::Memory).iter().map(|p| p.pid).collect::<Vec<_>>(), [3, 1, 2]);
        assert_eq!(size(1_500_000_000), "1.5 GB");
        assert!(endable(&p(9, 0.0, 1)) && !endable(&Process { name: "niri".into(), ..p(10, 0.0, 1) }) && !endable(&Process { own: false, ..p(11, 0.0, 1) }));
        assert_eq!(uptime(3 * 3600 + 120), "3 h 2 min");
    }
}
