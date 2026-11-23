//! Live wallpapers: a video, played by an `ffmpeg` of its own, or an animated GIF, decoded here.
//! Each plays on a thread that hands over one frame at a time, and only when asked. The app asks
//! when the compositor wants a frame and nothing holds the wallpaper still (the battery, the
//! lock, you being away). Not asked, the thread waits, `ffmpeg` waits on its full pipe, and
//! nothing is spent.
//!
//! Frames come out the size the compositor is sent them: a video at a share of the screen's
//! pixels, a GIF at its own size cut to the screen's shape; the viewporter scales them up.

use crate::config::Fit;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Videos `ffmpeg` is asked to play.
pub const VIDEOS: [&str; 7] = ["mp4", "webm", "mkv", "mov", "m4v", "avi", "ogv"];

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

pub fn is_video(path: &Path) -> bool {
    VIDEOS.contains(&extension(path).as_str())
}

pub fn is_gif(path: &Path) -> bool {
    extension(path) == "gif"
}

/// A wallpaper that moves.
pub fn is_live(path: &Path) -> bool {
    is_gif(path) || is_video(path)
}

/// Whether `ffmpeg` can be run, asked once.
pub fn have_ffmpeg() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| std::process::Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()))
}

/// A moving wallpaper that can play here: GIFs always, videos with `ffmpeg` about.
pub fn playable(path: &Path) -> bool {
    is_gif(path) || (is_video(path) && have_ffmpeg())
}

/// A frame for the compositor: BGRA bytes (wl_shm's ARGB8888 as it lies in memory), opaque.
pub struct Frame {
    pub player: u64,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

/// What a playing wallpaper is made from: when any of it changes, it starts again.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub path: PathBuf,
    /// The screen, in pixels.
    pub w: u32,
    pub h: u32,
    pub fit: Fit,
    /// Frames a second for a video.
    pub fps: u32,
    /// A video is decoded at this share of the screen's width and height, in percent.
    pub scale: u32,
    /// The compositor scales what it's sent (viewporter); without that, frames are the screen's size.
    pub scaled: bool,
    /// Showing round what doesn't cover the screen, as RGB.
    pub background: [u8; 3],
}

impl Spec {
    /// The size of a video's frames.
    pub fn video_size(&self) -> (u32, u32) {
        if !self.scaled {
            return (self.w.max(1), self.h.max(1));
        }
        let k = self.scale.clamp(25, 100) as f32 / 100.0;
        (((self.w as f32 * k).round() as u32).max(2), ((self.h as f32 * k).round() as u32).max(2))
    }

    /// What `ffmpeg` is told: loop the file for ever, no sound, at our pace and size, raw BGRA
    /// frames out of its standard output.
    pub fn ffmpeg_args(&self) -> Vec<String> {
        let (w, h) = self.video_size();
        let [r, g, b] = self.background;
        let fit = match self.fit {
            Fit::Cover => format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=bilinear,crop={w}:{h}"),
            Fit::Contain => format!("scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bilinear,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=0x{r:02x}{g:02x}{b:02x}"),
            Fit::Stretch => format!("scale={w}:{h}:flags=bilinear"),
        };
        let filter = format!("fps={},{fit},format=bgra", self.fps.clamp(5, 60));
        let path = self.path.to_string_lossy().into_owned();
        [
            "-nostdin", "-hide_banner", "-loglevel", "error",
            // Decoded by the graphics chip when it can, else on the processor; one thread each
            // for decoding and filtering, which measured both cheaper and smaller than many
            // (a frame comes only every 33 ms, there's no hurry)
            "-hwaccel", "auto", "-threads", "1",
            "-stream_loop", "-1", "-i", &path,
            "-an", "-sn", "-dn", "-filter_threads", "1", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "bgra", "-",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

/// Where frames come from: the next one into `data`, and how long it shows; None when there
/// are no more (an error, or a GIF of one frame, which stays as it is).
trait Source: Send {
    fn size(&self) -> (u32, u32);
    fn next(&mut self, data: &mut Vec<u8>) -> Option<Duration>;
}

struct Video {
    out: std::process::ChildStdout,
    size: (u32, u32),
    length: Duration,
}

impl Source for Video {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn next(&mut self, data: &mut Vec<u8>) -> Option<Duration> {
        data.resize(self.size.0 as usize * self.size.1 as usize * 4, 0);
        self.out.read_exact(data).ok()?;
        Some(self.length)
    }
}

/// A wallpaper playing.
pub struct Player {
    pub id: u64,
    pub spec: Spec,
    /// Asks for the next frame, handing back the last one's bytes to be filled again.
    ask: mpsc::Sender<Option<Vec<u8>>>,
    /// A frame was asked for and hasn't come yet.
    pub asked: bool,
    /// The last frame shown, its bytes going back with the next asking.
    pub spare: Option<Frame>,
    /// Frames shown so far.
    pub frames: u64,
    child: Option<Arc<Mutex<Option<Child>>>>,
}

impl Player {
    /// Starts playing: `ffmpeg` for a video, the decoder here for a GIF. Nothing comes until asked.
    pub fn start(id: u64, spec: Spec, frames: Sender<Frame>) -> Option<Player> {
        let mut child = None;
        let source: Box<dyn Source> = if is_video(&spec.path) {
            let mut command = std::process::Command::new("ffmpeg");
            command.args(spec.ffmpeg_args()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            // SAFETY: only an async-signal-safe prctl between fork and exec: it goes with the shell
            unsafe {
                use std::os::unix::process::CommandExt;
                command.pre_exec(|| rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM)).map_err(std::io::Error::from));
            }
            let mut process = command.spawn().map_err(|e| log::warn!("live wallpaper: can't start ffmpeg: {e}")).ok()?;
            let out = process.stdout.take()?;
            child = Some(Arc::new(Mutex::new(Some(process))));
            Box::new(Video { out, size: spec.video_size(), length: Duration::from_secs_f64(1.0 / spec.fps.clamp(5, 60) as f64) })
        } else {
            Box::new(Gif::open(&spec)?)
        };
        let (ask, asks) = mpsc::channel::<Option<Vec<u8>>>();
        let reaper = child.clone();
        std::thread::Builder::new()
            .name("asterism-live".into())
            .spawn(move || {
                play(id, source, asks, frames);
                // Its decoder goes with it
                if let Some(mut process) = reaper.and_then(|c| c.lock().ok()?.take()) {
                    let _ = process.kill();
                    let _ = process.wait();
                }
            })
            .ok()?;
        Some(Player { id, spec, ask, asked: false, spare: None, frames: 0, child })
    }

    /// Asks for the next frame.
    pub fn ask(&mut self) {
        self.asked = true;
        let _ = self.ask.send(self.spare.take().map(|f| f.data));
    }

    /// The last frame shown as a picture of `w`×`h`, to go from when the wallpaper changes.
    pub fn last_picture(&self, w: u32, h: u32) -> Option<tiny_skia::Pixmap> {
        let frame = self.spare.as_ref()?;
        let mut rgba = frame.data.clone();
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        let source = tiny_skia::Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(frame.w, frame.h)?)?;
        if (frame.w, frame.h) == (w, h) {
            return Some(source);
        }
        let mut picture = tiny_skia::Pixmap::new(w, h)?;
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        let transform = tiny_skia::Transform::from_scale(w as f32 / frame.w as f32, h as f32 / frame.h as f32);
        picture.draw_pixmap(0, 0, source.as_ref(), &paint, transform, None);
        Some(picture)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // The thread wakes on the closed channel, or on the pipe once `ffmpeg` is gone
        if let Some(child) = &self.child
            && let Ok(mut child) = child.lock()
            && let Some(process) = child.as_mut()
        {
            let _ = process.kill();
        }
    }
}

/// The player's thread: a frame each time it's asked, never before its time. After a pause it
/// goes on from now, not racing to catch up.
fn play(id: u64, mut source: Box<dyn Source>, asks: mpsc::Receiver<Option<Vec<u8>>>, frames: Sender<Frame>) {
    let (w, h) = source.size();
    let mut due = Instant::now();
    while let Ok(back) = asks.recv() {
        let mut data = back.unwrap_or_default();
        let Some(length) = source.next(&mut data) else { break };
        let now = Instant::now();
        if due + length < now {
            due = now;
        }
        if due > now {
            // Waiting on the channel, so a player put away doesn't sleep on
            match asks.recv_timeout(due - now) {
                Err(RecvTimeoutError::Timeout) | Ok(_) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        due += length;
        if frames.send(Frame { player: id, w, h, data }).is_err() {
            break;
        }
    }
    log::debug!("live wallpaper {id}: stopped");
}

// ---------------------------------------------------------------- GIFs

/// A GIF played from its file, frame by frame (only the picture so far is kept, not every frame),
/// starting again at its end.
struct Gif {
    path: PathBuf,
    decoder: Option<gif::Decoder<std::io::BufReader<std::fs::File>>>,
    /// The picture so far, RGBA, the GIF's own size.
    canvas: Vec<u8>,
    width: u32,
    height: u32,
    /// What the last frame asked to be done with it before the next, and where it was.
    dispose: Option<(gif::DisposalMethod, Rect, Vec<u8>)>,
    /// Frames in one round, once known: one alone doesn't play.
    frames: usize,
    counted: Option<usize>,
    out: (u32, u32),
    /// For each pixel out, the canvas pixel it shows (None: the background), by column and row.
    columns: Pixels,
    rows: Pixels,
    background: [u8; 3],
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

fn gif_decoder<R: Read>(reader: R) -> Option<gif::Decoder<R>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.read_info(reader).ok()
}

impl Gif {
    fn open(spec: &Spec) -> Option<Gif> {
        let decoder = gif_decoder(std::io::BufReader::new(std::fs::File::open(&spec.path).ok()?))?;
        let (width, height) = (decoder.width().max(1) as u32, decoder.height().max(1) as u32);
        let (out, columns, rows) = layout(width, height, spec);
        Some(Gif {
            path: spec.path.clone(),
            decoder: Some(decoder),
            canvas: vec![0; width as usize * height as usize * 4],
            width,
            height,
            dispose: None,
            frames: 0,
            counted: None,
            out,
            columns,
            rows,
            background: spec.background,
        })
    }

    /// The next frame drawn onto the canvas, and its delay; at the end, from the start again.
    fn advance(&mut self) -> Option<Duration> {
        for _ in 0..2 {
            if self.decoder.is_none() {
                self.decoder = gif_decoder(std::io::BufReader::new(std::fs::File::open(&self.path).ok()?));
                self.canvas.fill(0);
                self.dispose = None;
                self.frames = 0;
            }
            let decoder = self.decoder.as_mut()?;
            match decoder.read_next_frame() {
                Ok(Some(frame)) => {
                    let delay = frame.delay;
                    let rect = Rect { x: frame.left as u32, y: frame.top as u32, w: frame.width as u32, h: frame.height as u32 };
                    let (dispose, buffer) = (frame.dispose, frame.buffer.to_vec());
                    self.draw(rect, dispose, &buffer);
                    self.frames += 1;
                    // Browsers show quicker GIFs at ten frames a second, and so do we
                    let centis = if delay <= 1 { 10 } else { delay };
                    return Some(Duration::from_millis(centis as u64 * 10));
                }
                Ok(None) => {
                    self.counted = Some(self.frames);
                    if self.frames <= 1 {
                        return None;
                    }
                    self.decoder = None;
                }
                Err(error) => {
                    log::warn!("live wallpaper: {}: {error}", self.path.display());
                    return None;
                }
            }
        }
        None
    }

    /// The last frame's disposal done, then this one over the canvas (its clear pixels let
    /// what's under show).
    fn draw(&mut self, rect: Rect, dispose: gif::DisposalMethod, buffer: &[u8]) {
        let stride = self.width as usize * 4;
        let clip = |r: Rect, w: u32, h: u32| Rect { x: r.x.min(w), y: r.y.min(h), w: r.w.min(w.saturating_sub(r.x)), h: r.h.min(h.saturating_sub(r.y)) };
        if let Some((method, last, saved)) = self.dispose.take() {
            let last = clip(last, self.width, self.height);
            for row in 0..last.h as usize {
                let at = (last.y as usize + row) * stride + last.x as usize * 4;
                let span = &mut self.canvas[at..at + last.w as usize * 4];
                match method {
                    gif::DisposalMethod::Background => span.fill(0),
                    gif::DisposalMethod::Previous => span.copy_from_slice(&saved[row * last.w as usize * 4..][..last.w as usize * 4]),
                    _ => {}
                }
            }
        }
        let shown = clip(rect, self.width, self.height);
        let mut saved = Vec::new();
        if dispose == gif::DisposalMethod::Previous {
            for row in 0..shown.h as usize {
                let at = (shown.y as usize + row) * stride + shown.x as usize * 4;
                saved.extend_from_slice(&self.canvas[at..at + shown.w as usize * 4]);
            }
        }
        for row in 0..shown.h as usize {
            for col in 0..shown.w as usize {
                let from = (row * rect.w as usize + col) * 4;
                let Some(px) = buffer.get(from..from + 4) else { continue };
                if px[3] > 0 {
                    let at = (shown.y as usize + row) * stride + (shown.x as usize + col) * 4;
                    self.canvas[at..at + 4].copy_from_slice(px);
                }
            }
        }
        self.dispose = Some((dispose, shown, saved));
    }
}

impl Source for Gif {
    fn size(&self) -> (u32, u32) {
        self.out
    }

    fn next(&mut self, data: &mut Vec<u8>) -> Option<Duration> {
        // A GIF of one frame is a picture: it was painted already, and stays
        if self.counted.is_some_and(|n| n <= 1) {
            return None;
        }
        let length = self.advance()?;
        let (ow, oh) = self.out;
        data.resize(ow as usize * oh as usize * 4, 0);
        let [r, g, b] = self.background;
        let stride = self.width as usize * 4;
        for (y, line) in data.chunks_exact_mut(ow as usize * 4).enumerate() {
            let Some(row) = self.rows[y] else {
                for px in line.as_chunks_mut::<4>().0 {
                    *px = [b, g, r, 255];
                }
                continue;
            };
            let source = &self.canvas[row as usize * stride..][..stride];
            for (px, column) in line.as_chunks_mut::<4>().0.iter_mut().zip(&self.columns) {
                *px = match column.map(|c| &source[c as usize * 4..c as usize * 4 + 4]) {
                    Some(p) if p[3] > 0 => [p[2], p[1], p[0], 255],
                    _ => [b, g, r, 255],
                };
            }
        }
        Some(length)
    }
}

/// For each column (or row) sent, the GIF's own one it shows, if any.
type Pixels = Vec<Option<u32>>;

/// How a GIF of `gw`×`gh` lands in what's sent: the size sent, and for each column and row of
/// it the GIF's own (None: outside it). Scaled by the compositor, it's sent at its own size, cut
/// or widened to the screen's shape, and a big one made no bigger than a video would be (the
/// `scale` share of the screen); otherwise at the screen's size. Nearest pixel either way.
fn layout(gw: u32, gh: u32, spec: &Spec) -> ((u32, u32), Pixels, Pixels) {
    let (sw, sh) = (spec.w.max(1) as f32, spec.h.max(1) as f32);
    let (fw, fh) = (gw as f32, gh as f32);
    let screen = sw / sh;
    let ((ow, oh), (kx, ky)) = if spec.scaled {
        let size = match spec.fit {
            Fit::Stretch => (fw, fh),
            // Cut to the screen's shape
            Fit::Cover if fw / fh > screen => (fh * screen, fh),
            Fit::Cover => (fw, fw / screen),
            // Widened to it
            Fit::Contain if fw / fh > screen => (fw, fw / screen),
            Fit::Contain => (fh * screen, fh),
        };
        let most = spec.scale.clamp(25, 100) as f32 / 100.0;
        let (kx, ky) = ((sw * most / size.0).min(1.0), (sh * most / size.1).min(1.0));
        // Shrunk evenly, except stretched
        let (kx, ky) = if spec.fit == Fit::Stretch { (kx, ky) } else { (kx.min(ky), kx.min(ky)) };
        ((size.0 * kx, size.1 * ky), (kx, ky))
    } else {
        let k = match spec.fit {
            Fit::Stretch => (sw / fw, sh / fh),
            Fit::Cover => ((sw / fw).max(sh / fh), (sw / fw).max(sh / fh)),
            Fit::Contain => ((sw / fw).min(sh / fh), (sw / fw).min(sh / fh)),
        };
        ((sw, sh), k)
    };
    let (ow, oh) = ((ow.round() as u32).max(1), (oh.round() as u32).max(1));
    let map = |out: u32, k: f32, size: u32| -> Pixels {
        let offset = (out as f32 - size as f32 * k) / 2.0;
        (0..out)
            .map(|i| {
                let at = ((i as f32 + 0.5 - offset) / k).floor();
                (at >= 0.0 && at < size as f32).then_some(at as u32)
            })
            .collect()
    };
    ((ow, oh), map(ow, kx, gw), map(oh, ky, gh))
}

// ---------------------------------------------------------------- stills

/// A GIF's first frame, premultiplied (its clear pixels clear).
pub fn gif_still<R: Read>(reader: R) -> Option<tiny_skia::Pixmap> {
    let mut decoder = gif_decoder(reader)?;
    let (w, h) = (decoder.width().max(1) as u32, decoder.height().max(1) as u32);
    let frame = decoder.read_next_frame().ok()??;
    let mut canvas = vec![0u8; w as usize * h as usize * 4];
    for row in 0..frame.height as u32 {
        for col in 0..frame.width as u32 {
            let (x, y) = (frame.left as u32 + col, frame.top as u32 + row);
            let from = ((row * frame.width as u32 + col) * 4) as usize;
            if x < w && y < h
                && let Some(px) = frame.buffer.get(from..from + 4)
                && px[3] > 0
            {
                let at = ((y * w + x) * 4) as usize;
                canvas[at..at + 4].copy_from_slice(px);
            }
        }
    }
    tiny_skia::Pixmap::from_vec(canvas, tiny_skia::IntSize::from_wh(w, h)?)
}

/// A video's first frame, no bigger than `max` on a side, from `ffmpeg`.
pub fn video_still(path: &Path, max: u32) -> Option<tiny_skia::Pixmap> {
    if !have_ffmpeg() {
        return None;
    }
    let filter = format!("scale='min({max},iw)':'min({max},ih)':force_original_aspect_ratio=decrease");
    let output = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-an", "-vf", &filter, "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    tiny_skia::Pixmap::decode_png(&output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(w: u32, h: u32, fit: Fit, scaled: bool) -> Spec {
        Spec { path: "/x/sea.gif".into(), w, h, fit, fps: 30, scale: 100, scaled, background: [1, 2, 3] }
    }

    #[test]
    fn knows_what_moves() {
        assert!(is_live(Path::new("/a/b.MP4")) && is_live(Path::new("c.webm")) && is_live(Path::new("d.gif")));
        assert!(!is_live(Path::new("e.png")) && !is_live(Path::new("f")));
        assert!(is_gif(Path::new("g.GIF")) && !is_video(Path::new("g.gif")));
    }

    #[test]
    fn asks_ffmpeg_for_the_screens_shape() {
        let mut s = spec(1920, 1200, Fit::Cover, true);
        s.path = "/v/a b.mp4".into();
        s.scale = 50;
        assert_eq!(s.video_size(), (960, 600));
        let args = s.ffmpeg_args();
        let filter = &args[args.iter().position(|a| a == "-vf").unwrap() + 1];
        assert_eq!(filter, "fps=30,scale=960:600:force_original_aspect_ratio=increase:flags=bilinear,crop=960:600,format=bgra");
        // The path is one argument, spaces and all, and the file loops
        assert!(args.contains(&"/v/a b.mp4".to_string()) && args.windows(2).any(|w| w == ["-stream_loop", "-1"]));
        s.fit = Fit::Contain;
        assert!(s.ffmpeg_args().iter().any(|a| a.contains("pad=960:600:(ow-iw)/2:(oh-ih)/2:color=0x010203")));
        // Without the compositor scaling, the screen's whole size
        s.scaled = false;
        assert_eq!(s.video_size(), (1920, 1200));
    }

    #[test]
    fn a_gif_is_cut_or_widened_to_the_screen() {
        // 16:9 on 16:10, sent at its own size
        let ((w, h), columns, rows) = layout(480, 270, &spec(1920, 1200, Fit::Cover, true));
        assert_eq!((w, h), (432, 270));
        assert_eq!((columns[0], columns[431], rows[0]), (Some(24), Some(455), Some(0)));
        let ((w, h), columns, rows) = layout(480, 270, &spec(1920, 1200, Fit::Contain, true));
        assert_eq!((w, h), (480, 300));
        assert_eq!((rows[0], rows[15], rows[299], columns[0]), (None, Some(0), None, Some(0)));
        // Unscaled: the screen's size, nearest pixel
        let ((w, h), columns, _) = layout(100, 100, &spec(200, 100, Fit::Stretch, false));
        assert_eq!((w, h, columns[0], columns[1], columns[2], columns[199]), (200, 100, Some(0), Some(0), Some(1), Some(99)));
        // A big one no bigger than a video at half size
        let mut half = spec(1712, 1028, Fit::Cover, true);
        half.scale = 50;
        let ((w, h), columns, rows) = layout(1920, 1080, &half);
        assert_eq!((w, h), (856, 514));
        assert!(columns.iter().chain(&rows).all(Option::is_some) && columns[855] < Some(1920) && rows[513] < Some(1080));
    }

    /// A GIF of two 4×2 frames: red all over, then over its right half blue on a diagonal (the
    /// rest clear, so the red stays), the second asking to be cleared after.
    fn two_frames() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let palette = [255, 0, 0, 0, 0, 255, 0, 0, 0];
            let mut encoder = gif::Encoder::new(&mut bytes, 4, 2, &palette).unwrap();
            encoder.set_repeat(gif::Repeat::Infinite).unwrap();
            let mut red = gif::Frame { width: 4, height: 2, delay: 5, buffer: std::borrow::Cow::Owned(vec![0; 8]), ..Default::default() };
            red.dispose = gif::DisposalMethod::Keep;
            encoder.write_frame(&red).unwrap();
            let mut blue = gif::Frame { left: 2, width: 2, height: 2, delay: 0, buffer: std::borrow::Cow::Owned(vec![1, 2, 2, 1]), transparent: Some(2), ..Default::default() };
            blue.dispose = gif::DisposalMethod::Background;
            encoder.write_frame(&blue).unwrap();
        }
        bytes
    }

    #[test]
    fn plays_a_gif_round_and_round() {
        let dir = std::env::temp_dir().join(format!("asterism-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("two.gif");
        std::fs::write(&path, two_frames()).unwrap();
        let mut s = spec(4, 2, Fit::Stretch, true);
        s.path = path.clone();
        let mut gif = Gif::open(&s).unwrap();
        let mut data = Vec::new();
        let px = |data: &[u8], x: usize, y: usize| data[(y * 4 + x) * 4..][..4].to_vec();
        // Red, its own 50 ms
        assert_eq!(gif.next(&mut data), Some(Duration::from_millis(50)));
        assert_eq!((px(&data, 0, 0), px(&data, 3, 1)), (vec![0, 0, 255, 255], vec![0, 0, 255, 255]));
        // Blue over it where it isn't clear; a delay of 0 plays at ten a second
        assert_eq!(gif.next(&mut data), Some(Duration::from_millis(100)));
        assert_eq!((px(&data, 2, 0), px(&data, 2, 1), px(&data, 0, 0)), (vec![255, 0, 0, 255], vec![0, 0, 255, 255], vec![0, 0, 255, 255]));
        // And round again, from red
        assert_eq!(gif.next(&mut data), Some(Duration::from_millis(50)));
        assert_eq!(px(&data, 2, 0), vec![0, 0, 255, 255]);
        // Its first frame as a still
        let still = gif_still(std::io::Cursor::new(two_frames())).unwrap();
        assert_eq!((still.width(), still.height(), &still.data()[..4]), (4, 2, &[255u8, 0, 0, 255][..]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_still_gif_doesnt_play() {
        let mut bytes = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 2, 2, &[9, 9, 9]).unwrap();
            encoder.write_frame(&gif::Frame { width: 2, height: 2, buffer: std::borrow::Cow::Owned(vec![0; 4]), ..Default::default() }).unwrap();
        }
        let dir = std::env::temp_dir().join(format!("asterism-still-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = spec(2, 2, Fit::Cover, true);
        s.path = dir.join("one.gif");
        std::fs::write(&s.path, bytes).unwrap();
        let mut gif = Gif::open(&s).unwrap();
        let mut data = Vec::new();
        assert!(gif.next(&mut data).is_some());
        assert!(gif.next(&mut data).is_none() && gif.next(&mut data).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
