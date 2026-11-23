//! The pet's voice, made here from nothing (no recordings): a fundamental and its harmonics,
//! shaped by vowel formants that move through the sound, the way a cat's "m-ee-ow" opens and
//! closes; a purr from a low rumble pulsing about 25 times a second; a fox's sharp yip; a bird's
//! quick chirps; a big cat's roar, growl and friendly chuff; a panda's bleat. Each comes in a few variants, written once as a WAV into the cache and played
//! with `pw-play` (or `paplay`) by the services.

use std::f32::consts::TAU;
use std::path::PathBuf;

pub const RATE: u32 = 22_050;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Voice {
    Meow,
    /// A rolled, rising "mrrp?": hello, thanks.
    Trill,
    Purr,
    /// A low, drawn-out "mrrow!": put me down.
    Complain,
    Yip,
    Chirp,
    /// A big cat's call: low, rough, swelling and falling away.
    Roar,
    /// A big cat's hello: two or three breathy puffs through the nose.
    Chuff,
    /// A low, rattling warning.
    Growl,
    /// A panda's call, wavering like a goat's.
    Bleat,
}

impl Voice {
    pub fn name(self) -> &'static str {
        match self {
            Voice::Meow => "meow",
            Voice::Trill => "trill",
            Voice::Purr => "purr",
            Voice::Complain => "complain",
            Voice::Yip => "yip",
            Voice::Chirp => "chirp",
            Voice::Roar => "roar",
            Voice::Chuff => "chuff",
            Voice::Growl => "growl",
            Voice::Bleat => "bleat",
        }
    }

    #[cfg(test)]
    pub const ALL: [Voice; 10] = [Voice::Meow, Voice::Trill, Voice::Purr, Voice::Complain, Voice::Yip, Voice::Chirp, Voice::Roar, Voice::Chuff, Voice::Growl, Voice::Bleat];
}

/// How many takes of each sound there are.
pub const VARIANTS: u32 = 3;

/// A value between keyframes `(at, value)` (sorted by `at`), linearly.
fn track(keys: &[(f32, f32)], t: f32) -> f32 {
    match keys.iter().position(|(at, _)| *at > t) {
        None => keys.last().map_or(0.0, |k| k.1),
        Some(0) => keys[0].1,
        Some(i) => {
            let (a, b) = (keys[i - 1], keys[i]);
            a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0).max(1e-6)
        }
    }
}

/// How much a vocal tract with these formants (frequency, width, strength) lets a harmonic at
/// `f` through.
fn formant_gain(f: f32, formants: &[(f32, f32, f32)]) -> f32 {
    formants.iter().map(|(centre, width, strength)| strength / (1.0 + ((f - centre) / width).powi(2))).sum::<f32>() + 0.04
}

/// A small, steady source of noise (xorshift).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// A voiced sound: pitch, vowel and loudness along normalised time `u` (0 to 1).
struct Voiced<'a> {
    seconds: f32,
    pitch: &'a dyn Fn(f32) -> f32,
    vowel: &'a dyn Fn(f32) -> [(f32, f32, f32); 3],
    loudness: &'a dyn Fn(f32) -> f32,
    /// Breath in it, and a rough edge (a subharmonic).
    breath: f32,
    rough: f32,
    /// How fast the harmonics fall away: lower is brighter.
    tilt: f32,
}

fn voiced(v: &Voiced, seed: u32) -> Vec<f32> {
    let n = (v.seconds * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    let mut noise = Noise(seed.max(1));
    let mut breath_low = 0.0f32;
    for i in 0..n {
        let u = i as f32 / n as f32;
        let f0 = (v.pitch)(u);
        phase = (phase + f0 / RATE as f32).fract();
        let formants = (v.vowel)(u);
        let mut sample = 0.0;
        let mut k = 1;
        while (k as f32) * f0 < 5_000.0 && k < 60 {
            let f = k as f32 * f0;
            sample += formant_gain(f, &formants) / (k as f32).powf(v.tilt) * (TAU * k as f32 * phase).sin();
            k += 1;
        }
        if v.rough > 0.0 {
            sample += v.rough * (TAU * 0.5 * phase).sin() * formant_gain(f0 * 0.5, &formants);
        }
        // Breath: noise, softened, through the same vowel roughly
        breath_low += (noise.next() - breath_low) * 0.35;
        sample += breath_low * v.breath;
        out.push(sample * (v.loudness)(u));
    }
    out
}

/// Attack, hold and release as shares of the sound.
fn envelope(u: f32, attack: f32, release: f32) -> f32 {
    let rise = (u / attack.max(1e-4)).min(1.0);
    let fall = ((1.0 - u) / release.max(1e-4)).min(1.0);
    (rise * fall).max(0.0).powf(0.8)
}

/// The sound, as samples from -1 to 1 at `RATE`.
pub fn synth(voice: Voice, variant: u32) -> Vec<f32> {
    let k = variant as f32;
    let mut samples = match voice {
        Voice::Meow => {
            // m → ee → a → ow, the pitch rising into the "ee" and falling away
            let top = 760.0 + 60.0 * k;
            let seconds = 0.62 + 0.12 * k;
            let pitch = move |u: f32| track(&[(0.0, 470.0), (0.32, top), (0.75, top * 0.82), (1.0, 520.0)], u) * (1.0 + 0.012 * (TAU * 5.5 * u * seconds).sin());
            let vowel = |u: f32| {
                let f1 = track(&[(0.0, 330.0), (0.16, 420.0), (0.45, 950.0), (0.8, 620.0), (1.0, 450.0)], u);
                let f2 = track(&[(0.0, 1200.0), (0.18, 2300.0), (0.45, 1550.0), (0.8, 1000.0), (1.0, 850.0)], u);
                [(f1, 140.0, 1.0), (f2, 220.0, 0.55), (3200.0, 400.0, 0.12)]
            };
            let loud = |u: f32| envelope(u, 0.12, 0.3) * track(&[(0.0, 0.35), (0.15, 1.0), (1.0, 0.85)], u);
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.08, rough: 0.0, tilt: 0.75 }, 11 + variant)
        }
        Voice::Trill => {
            // A rolled "rr" (a tremble in the loudness) opening into a rising vowel
            let seconds = 0.34 + 0.05 * k;
            let pitch = move |u: f32| track(&[(0.0, 520.0 + 30.0 * k), (0.6, 600.0), (1.0, 760.0 + 40.0 * k)], u);
            let vowel = |u: f32| {
                let f1 = track(&[(0.0, 420.0), (0.6, 520.0), (1.0, 820.0)], u);
                let f2 = track(&[(0.0, 1000.0), (0.6, 1200.0), (1.0, 1700.0)], u);
                [(f1, 150.0, 1.0), (f2, 250.0, 0.45), (3000.0, 400.0, 0.1)]
            };
            let loud = move |u: f32| {
                let roll = if u < 0.62 { 0.55 + 0.45 * (TAU * 30.0 * u * seconds).sin() } else { 1.0 };
                envelope(u, 0.08, 0.25) * roll
            };
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.06, rough: 0.0, tilt: 0.85 }, 23 + variant)
        }
        Voice::Complain => {
            let seconds = 0.85 + 0.1 * k;
            let pitch = move |u: f32| track(&[(0.0, 400.0), (0.35, 640.0 + 30.0 * k), (1.0, 360.0)], u);
            let vowel = |u: f32| {
                let f1 = track(&[(0.0, 330.0), (0.2, 480.0), (0.5, 900.0), (1.0, 480.0)], u);
                let f2 = track(&[(0.0, 1100.0), (0.2, 1300.0), (0.5, 1450.0), (1.0, 900.0)], u);
                [(f1, 160.0, 1.0), (f2, 260.0, 0.5), (2800.0, 400.0, 0.15)]
            };
            let loud = |u: f32| envelope(u, 0.15, 0.35);
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.12, rough: 0.35, tilt: 0.7 }, 37 + variant)
        }
        Voice::Purr => {
            // A low rumble pulsing ~25 times a second, in two breaths: in (softer), out
            let seconds = 1.9;
            let pitch = move |_: f32| 25.0 + 2.0 * k;
            let vowel = |_: f32| [(180.0, 160.0, 1.0), (520.0, 300.0, 0.25), (1200.0, 500.0, 0.05)];
            let loud = |u: f32| {
                let t = u * seconds;
                let breath = if t < 0.85 { envelope(t / 0.85, 0.2, 0.25) * 0.7 } else if t > 0.95 { envelope((t - 0.95) / 0.95, 0.2, 0.25) } else { 0.0 };
                breath * (0.35 + 0.65 * (TAU * 25.0 * t).sin().max(0.0))
            };
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.9, rough: 0.0, tilt: 0.3 }, 41 + variant)
        }
        Voice::Yip => {
            // One sharp, falling yip (or two)
            let one = |seed: u32| {
                let seconds = 0.2;
                let pitch = move |u: f32| 1000.0 * (1.0 - 0.38 * u) + 40.0 * k;
                let vowel = |_: f32| [(1100.0, 320.0, 1.0), (2100.0, 420.0, 0.7), (3300.0, 500.0, 0.3)];
                let loud = |u: f32| envelope(u, 0.04, 0.6);
                voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.25, rough: 0.2, tilt: 0.45 }, seed)
            };
            let mut out = one(53 + variant);
            if variant != 1 {
                out.extend(std::iter::repeat_n(0.0, (0.09 * RATE as f32) as usize));
                out.extend(one(59 + variant));
            }
            out
        }
        Voice::Chirp => {
            // Two to four quick whistles, sweeping up or down
            let mut out = Vec::new();
            let notes = 2 + variant as usize;
            for n in 0..notes {
                let seconds = 0.075 + 0.01 * (n % 2) as f32;
                let count = (seconds * RATE as f32) as usize;
                let (from, to) = if (n + variant as usize).is_multiple_of(2) { (3300.0, 4700.0) } else { (4900.0, 3500.0) };
                let mut phase = 0.0f32;
                for i in 0..count {
                    let u = i as f32 / count as f32;
                    phase = (phase + (from + (to - from) * u) / RATE as f32).fract();
                    let shape = (std::f32::consts::PI * u).sin().powf(0.7);
                    out.push(shape * ((TAU * phase).sin() + 0.15 * (TAU * 2.0 * phase).sin()));
                }
                out.extend(std::iter::repeat_n(0.0, (0.05 * RATE as f32) as usize));
            }
            out
        }
        Voice::Roar => {
            // An open "aa-oo" from a low, rough voice: up into it, then down and away
            let seconds = 1.25 + 0.15 * k;
            let pitch = move |u: f32| track(&[(0.0, 120.0), (0.25, 210.0 + 15.0 * k), (0.6, 170.0), (1.0, 95.0)], u) * (1.0 + 0.03 * (TAU * 7.0 * u * seconds).sin());
            let vowel = |u: f32| {
                let f1 = track(&[(0.0, 450.0), (0.3, 800.0), (0.7, 650.0), (1.0, 420.0)], u);
                let f2 = track(&[(0.0, 900.0), (0.3, 1250.0), (0.7, 1000.0), (1.0, 750.0)], u);
                [(f1, 220.0, 1.0), (f2, 300.0, 0.6), (2400.0, 500.0, 0.2)]
            };
            let loud = |u: f32| envelope(u, 0.18, 0.45) * track(&[(0.0, 0.5), (0.3, 1.0), (1.0, 0.7)], u);
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.35, rough: 0.7, tilt: 0.55 }, 61 + variant)
        }
        Voice::Growl => {
            // Low and steady, rattling about 18 times a second
            let seconds = 0.9 + 0.1 * k;
            let pitch = move |u: f32| 82.0 + 8.0 * k + 6.0 * (TAU * 1.5 * u).sin();
            let vowel = |_: f32| [(430.0, 180.0, 1.0), (880.0, 260.0, 0.5), (2200.0, 500.0, 0.15)];
            let loud = move |u: f32| envelope(u, 0.12, 0.3) * (0.55 + 0.45 * (TAU * 18.0 * u * seconds).sin().abs());
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.45, rough: 0.9, tilt: 0.5 }, 67 + variant)
        }
        Voice::Chuff => {
            // Puffs of breath with a little voice under them
            let mut out = Vec::new();
            let puffs = 2 + (variant % 2) as usize;
            for n in 0..puffs {
                let seconds = 0.13;
                let pitch = move |_: f32| 110.0 + 10.0 * n as f32;
                let vowel = |_: f32| [(700.0, 300.0, 1.0), (1250.0, 400.0, 0.5), (2600.0, 600.0, 0.2)];
                let loud = |u: f32| envelope(u, 0.1, 0.6);
                out.extend(voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 2.2, rough: 0.2, tilt: 0.9 }, 71 + variant * 3 + n as u32));
                if n + 1 < puffs {
                    out.extend(std::iter::repeat_n(0.0, (0.06 * RATE as f32) as usize));
                }
            }
            out
        }
        Voice::Bleat => {
            // "Mehh": a nasal "eh" whose pitch and loudness waver about 9 times a second
            let seconds = 0.55 + 0.1 * k;
            let pitch = move |u: f32| track(&[(0.0, 380.0 + 20.0 * k), (0.2, 460.0), (1.0, 400.0)], u) * (1.0 + 0.05 * (TAU * 9.0 * u * seconds).sin());
            let vowel = |u: f32| {
                let f1 = track(&[(0.0, 300.0), (0.15, 560.0), (1.0, 520.0)], u);
                [(f1, 150.0, 1.0), (1850.0, 240.0, 0.7), (2700.0, 400.0, 0.25)]
            };
            let loud = move |u: f32| envelope(u, 0.1, 0.35) * (0.7 + 0.3 * (TAU * 9.0 * u * seconds).sin());
            voiced(&Voiced { seconds, pitch: &pitch, vowel: &vowel, loudness: &loud, breath: 0.1, rough: 0.15, tilt: 0.65 }, 79 + variant)
        }
    };
    // As loud as it can be without clipping, then a touch under
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    for s in &mut samples {
        *s = *s / peak * 0.85;
    }
    samples
}

/// The samples as a 16-bit mono WAV file.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    out
}

/// Where a sound's file is kept (made the first time it's wanted). The version in the name
/// makes a changed voice write itself again.
pub fn file(voice: Voice, variant: u32) -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".cache"));
    file_in(&cache.join("asterism").join("pet"), voice, variant)
}

fn file_in(dir: &std::path::Path, voice: Voice, variant: u32) -> Option<PathBuf> {
    let path = dir.join(format!("{}-{}-v1.wav", voice.name(), variant % VARIANTS));
    if !path.exists() {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, wav(&synth(voice, variant % VARIANTS))).ok()?;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_voice_sounds() {
        for voice in Voice::ALL {
            for variant in 0..VARIANTS {
                let samples = synth(voice, variant);
                let seconds = samples.len() as f32 / RATE as f32;
                assert!((0.15..2.5).contains(&seconds), "{voice:?} lasts {seconds}s");
                let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                assert!((0.8..=0.86).contains(&peak), "{voice:?} peaks at {peak}");
                // Not a click: plenty of it is sound
                let loud = samples.iter().filter(|s| s.abs() > 0.05).count();
                assert!(loud > samples.len() / 8, "{voice:?} is mostly silence");
            }
        }
    }

    #[test]
    fn a_meow_rises_then_falls() {
        // The pitch is highest about a third of the way in: count zero crossings in thirds
        let samples = synth(Voice::Meow, 0);
        let third = samples.len() / 3;
        let crossings = |part: &[f32]| part.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        let (a, b, c) = (crossings(&samples[..third]), crossings(&samples[third..2 * third]), crossings(&samples[2 * third..]));
        assert!(b > c, "{a} {b} {c}");
    }

    #[test]
    fn makes_its_files_once() {
        let dir = std::env::temp_dir().join(format!("asterism-pet-sounds-{}", std::process::id()));
        let path = file_in(&dir, Voice::Meow, 4).expect("written");
        assert!(path.ends_with("meow-1-v1.wav"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        let made = std::fs::metadata(&path).unwrap().modified().unwrap();
        file_in(&dir, Voice::Meow, 1).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), made, "not written twice");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every voice and take as WAVs in $ASTERISM_SOUNDS, to listen to (run with --ignored).
    #[test]
    #[ignore]
    fn write_samples_to_listen_to() {
        let Some(dir) = std::env::var_os("ASTERISM_SOUNDS") else { return };
        let dir = PathBuf::from(dir);
        for voice in Voice::ALL {
            for variant in 0..VARIANTS {
                file_in(&dir, voice, variant).expect("written");
            }
        }
    }

    #[test]
    fn writes_a_wav() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 16_383);
    }
}
