//! What the speakers play, as a row of levels: a small FFT over the last few thousand samples,
//! summed into bands spaced like hearing is (more of them low down), each eased so it jumps up
//! at once and falls back gently. The capture itself is `services/spectrum.rs`.

/// Samples a second, as asked of the capture.
pub const RATE: u32 = 44_100;
/// Samples looked at for each frame: about 46 ms, enough to tell 22 Hz apart.
pub const WINDOW: usize = 2048;
/// New samples between frames: thirty frames a second.
pub const HOP: usize = RATE as usize / 30;
/// Bars on the widget.
pub const BARS: usize = 40;
/// How far below the loudest band of late a band still shows, in decibels.
const RANGE: f32 = 42.0;

/// The Fourier transform of `re` (with `im` zero, for real input), in place. Radix 2, so the
/// length must be a power of two.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    // Into bit-reversed order
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

pub struct Analyzer {
    /// The Hann window, so a band doesn't bleed into its neighbours.
    window: Vec<f32>,
    /// Where each band starts among the FFT's bins; one more than there are bands.
    edges: Vec<usize>,
    /// What each bar shows now, 0 to 1.
    levels: Vec<f32>,
    /// The loudest band of late, in decibels: the top of the scale, so quiet music still fills it.
    top: f32,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Analyzer {
    pub fn new(bars: usize) -> Self {
        let window = (0..WINDOW).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (WINDOW - 1) as f32).cos()).collect();
        // From 40 Hz to 14 kHz, evenly in log frequency; at least one bin each
        let bin = RATE as f32 / WINDOW as f32;
        let (low, high) = (40.0f32, 14_000.0f32);
        let mut edges = Vec::with_capacity(bars + 1);
        for i in 0..=bars {
            let freq = low * (high / low).powf(i as f32 / bars as f32);
            let at = (freq / bin).round() as usize;
            let at = match edges.last() {
                Some(&last) if at <= last => last + 1,
                _ => at.max(1),
            };
            edges.push(at.min(WINDOW / 2));
        }
        Self { window, edges, levels: vec![0.0; bars], top: -30.0, re: vec![0.0; WINDOW], im: vec![0.0; WINDOW] }
    }

    /// The bars for the last `WINDOW` samples (-1 to 1); fewer are taken as silence before them.
    pub fn analyze(&mut self, samples: &[f32]) -> &[f32] {
        let pad = WINDOW.saturating_sub(samples.len());
        let tail = &samples[samples.len().saturating_sub(WINDOW)..];
        for i in 0..WINDOW {
            self.re[i] = if i < pad { 0.0 } else { tail[i - pad] * self.window[i] };
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im);
        let mut loudest = f32::MIN;
        let mut db = Vec::with_capacity(self.levels.len());
        for band in self.edges.windows(2) {
            // The band's strongest bin: an average would flatten the high bands, which are wide
            let power = (band[0]..band[1]).map(|k| self.re[k] * self.re[k] + self.im[k] * self.im[k]).fold(0.0f32, f32::max);
            let level = 10.0 * (power.max(1e-12)).log10();
            loudest = loudest.max(level);
            db.push(level);
        }
        // The scale follows the music: up at once, down slowly, never so low that hiss fills it
        self.top = if loudest > self.top { loudest } else { (self.top - 0.15).max(loudest).max(-10.0) };
        for (level, band) in self.levels.iter_mut().zip(db) {
            let target = ((band - (self.top - RANGE)) / RANGE).clamp(0.0, 1.0);
            // Jumps up, falls back a little each frame
            *level = if target > *level { target } else { (*level - 0.05).max(target) };
        }
        &self.levels
    }

    /// Everything still: the bars sink back to dots.
    pub fn silent(&self) -> bool {
        self.levels.iter().all(|l| *l < 0.01)
    }
}

/// A made-up spectrum, for snapshots: a hump in the low end, a few peaks above.
pub fn sample_levels(bars: usize) -> Vec<f32> {
    (0..bars)
        .map(|i| {
            let x = i as f32 / bars as f32;
            (0.85 * (-((x - 0.12) / 0.14).powi(2)).exp() + 0.45 * (-((x - 0.45) / 0.08).powi(2)).exp() + 0.3 * (-((x - 0.7) / 0.1).powi(2)).exp() + 0.08 * ((i * 7 % 5) as f32 / 5.0)).min(1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_a_wave() {
        // A wave of 4 cycles in 64 samples lands in bin 4 (and its mirror)
        let mut re: Vec<f32> = (0..64).map(|i| (2.0 * std::f32::consts::PI * 4.0 * i as f32 / 64.0).cos()).collect();
        let mut im = vec![0.0; 64];
        fft(&mut re, &mut im);
        let power: Vec<f32> = re.iter().zip(&im).map(|(r, i)| r.hypot(*i)).collect();
        assert!((power[4] - 32.0).abs() < 0.01 && (power[60] - 32.0).abs() < 0.01, "{power:?}");
        assert!(power.iter().enumerate().filter(|(k, _)| *k != 4 && *k != 60).all(|(_, p)| *p < 0.01));
    }

    #[test]
    fn finds_a_tone_in_its_band() {
        let mut analyzer = Analyzer::new(BARS);
        let tone = |freq: f32| -> Vec<f32> { (0..WINDOW).map(|i| 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / RATE as f32).sin()).collect() };
        let levels = analyzer.analyze(&tone(1000.0)).to_vec();
        let peak = levels.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
        // 1 kHz sits a little past the middle of 40 Hz – 14 kHz on a log scale
        let at = ((1000.0f32 / 40.0).ln() / (14_000.0f32 / 40.0).ln() * BARS as f32) as usize;
        assert!(peak.abs_diff(at) <= 1, "peak in bar {peak}, expected about {at}: {levels:?}");
        assert!(levels[peak] > 0.9);
        // Far from it, nothing much
        assert!(levels[0] < 0.2 && levels[BARS - 1] < 0.2, "{levels:?}");
        // A higher tone moves the peak right
        let mut other = Analyzer::new(BARS);
        let high = other.analyze(&tone(6000.0)).iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
        assert!(high > peak + 5);
    }

    #[test]
    fn silence_sinks_to_dots() {
        let mut analyzer = Analyzer::new(BARS);
        let loud: Vec<f32> = (0..WINDOW).map(|i| (i as f32 * 0.3).sin() * 0.8).collect();
        analyzer.analyze(&loud);
        assert!(!analyzer.silent());
        for _ in 0..30 {
            analyzer.analyze(&vec![0.0; WINDOW]);
        }
        assert!(analyzer.silent());
    }

    #[test]
    fn bands_are_in_order() {
        let analyzer = Analyzer::new(64);
        assert_eq!(analyzer.edges.len(), 65);
        assert!(analyzer.edges.windows(2).all(|w| w[1] > w[0]));
        assert!(*analyzer.edges.last().unwrap() <= WINDOW / 2);
    }
}
