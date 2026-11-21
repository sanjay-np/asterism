//! Night light, the arithmetic: how warm the screen is at a given moment, and the colour
//! curves that make it so. (Talking to the compositor lives in `app::night`.)

/// What the control centre shows for night light.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NightStatus {
    pub available: bool,
    pub on: bool,
    pub detail: String,
}

/// Daylight: the screen as it is.
pub const NEUTRAL: f32 = 6500.0;

/// How much of a channel a colour temperature keeps, red, green, blue, 0 to 1. Tanner Helland's
/// fit of blackbody colours, scaled so 6500 K is exactly white.
pub fn rgb(kelvin: f32) -> [f32; 3] {
    fn raw(kelvin: f32) -> [f32; 3] {
        let t = kelvin.clamp(1000.0, 40000.0) / 100.0;
        let r = if t <= 66.0 { 255.0 } else { 329.698_73 * (t - 60.0).powf(-0.133_204_76) };
        let g = if t <= 66.0 { 99.470_8 * t.ln() - 161.119_57 } else { 288.122_17 * (t - 60.0).powf(-0.075_514_85) };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_73 * (t - 10.0).ln() - 305.044_8
        };
        [r / 255.0, g / 255.0, b / 255.0].map(|c| c.clamp(0.0, 1.0))
    }
    let (warm, white) = (raw(kelvin), raw(NEUTRAL));
    [0, 1, 2].map(|i| (warm[i] / white[i]).clamp(0.0, 1.0))
}

/// The red, green and blue curves (each `size` long, one after the other) for the gamma table.
pub fn ramps(size: usize, kelvin: f32) -> Vec<u16> {
    let scale = rgb(kelvin);
    let mut table = Vec::with_capacity(size * 3);
    for channel in scale {
        for i in 0..size {
            let v = if size > 1 { i as f32 / (size - 1) as f32 } else { 1.0 };
            table.push((v * channel * 65535.0).round() as u16);
        }
    }
    table
}

/// How far into the night it is, 0 (day) to 1 (night), at `now` minutes after midnight. Night
/// runs from `start` to `end` (wrapping past midnight); it eases in over `fade` minutes after
/// `start`, and back out over the `fade` minutes before `end`.
pub fn night_level(now: f32, start: f32, end: f32, fade: f32) -> f32 {
    let day = 24.0 * 60.0;
    let since = |from: f32| (now - from).rem_euclid(day);
    let length = (end - start).rem_euclid(day);
    let into = since(start);
    if into >= length {
        return 0.0;
    }
    let fade = fade.min(length / 2.0).max(1.0);
    let before_end = length - into;
    (into / fade).min(before_end / fade).min(1.0)
}

/// "19:30" → 1170 minutes after midnight.
pub fn minutes(text: &str) -> Option<f32> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h * 60 + m) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_curves() {
        assert_eq!(rgb(NEUTRAL), [1.0, 1.0, 1.0]);
        let [r, g, b] = rgb(3500.0);
        assert!(r > 0.99 && g < 0.9 && b < g, "{r} {g} {b}");
        let table = ramps(256, 4000.0);
        assert_eq!(table.len(), 768);
        assert_eq!((table[0], table[255]), (0, 65535));
        assert!(table[511] < 65535 && table[767] < table[511]);
    }

    #[test]
    fn night_comes_and_goes() {
        let (start, end) = (minutes("19:00").unwrap(), minutes("07:00").unwrap());
        assert_eq!(night_level(minutes("12:00").unwrap(), start, end, 20.0), 0.0);
        assert_eq!(night_level(minutes("19:10").unwrap(), start, end, 20.0), 0.5);
        assert_eq!(night_level(minutes("23:59").unwrap(), start, end, 20.0), 1.0);
        assert_eq!(night_level(minutes("03:00").unwrap(), start, end, 20.0), 1.0);
        assert_eq!(night_level(minutes("06:50").unwrap(), start, end, 20.0), 0.5);
        assert_eq!(night_level(minutes("07:00").unwrap(), start, end, 20.0), 0.0);
        assert_eq!(minutes("25:00"), None);
    }
}
