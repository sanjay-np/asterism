//! The weather as the bar shows it: read out of Open-Meteo's answer, with words and icons for
//! its WMO weather codes. (Fetching lives in `services::weather`.)

use chrono::NaiveDate;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Weather {
    pub available: bool,
    pub place: String,
    pub temperature: f32,
    pub feels_like: f32,
    pub code: u8,
    pub is_day: bool,
    /// Percent.
    pub humidity: f32,
    pub wind: f32,
    /// "°C" or "°F", "km/h" or "mph".
    pub temperature_unit: String,
    pub wind_unit: String,
    /// "06:41"
    pub sunrise: String,
    pub sunset: String,
    pub days: Vec<Day>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub code: u8,
    pub max: f32,
    pub min: f32,
    /// The chance of rain, percent.
    pub rain: u8,
}

/// Reads Open-Meteo's forecast answer.
pub fn parse(json: &serde_json::Value, place: &str) -> Option<Weather> {
    let current = json.get("current")?;
    let units = json.get("current_units");
    let number = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32);
    let unit = |key: &str, fallback: &str| units.and_then(|u| u.get(key)).and_then(|x| x.as_str()).unwrap_or(fallback).to_string();
    let daily = json.get("daily");
    let column = |key: &str| daily.and_then(|d| d.get(key)).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let (dates, codes, maxes, mins, rains) = (column("time"), column("weather_code"), column("temperature_2m_max"), column("temperature_2m_min"), column("precipitation_probability_max"));
    let days = dates
        .iter()
        .enumerate()
        .filter_map(|(i, date)| {
            Some(Day {
                date: NaiveDate::parse_from_str(date.as_str()?, "%Y-%m-%d").ok()?,
                code: codes.get(i)?.as_u64()? as u8,
                max: maxes.get(i)?.as_f64()? as f32,
                min: mins.get(i)?.as_f64()? as f32,
                rain: rains.get(i).and_then(|r| r.as_u64()).unwrap_or(0) as u8,
            })
        })
        .collect();
    // "2026-11-21T06:41" → "06:41"
    let first_time = |key: &str| column(key).first().and_then(|x| x.as_str()).and_then(|s| s.split('T').nth(1)).unwrap_or("").to_string();
    Some(Weather {
        available: true,
        place: place.to_string(),
        temperature: number(current, "temperature_2m")?,
        feels_like: number(current, "apparent_temperature").unwrap_or(0.0),
        code: number(current, "weather_code").unwrap_or(0.0) as u8,
        is_day: number(current, "is_day").unwrap_or(1.0) > 0.5,
        humidity: number(current, "relative_humidity_2m").unwrap_or(0.0),
        wind: number(current, "wind_speed_10m").unwrap_or(0.0),
        temperature_unit: unit("temperature_2m", "°C"),
        wind_unit: unit("wind_speed_10m", "km/h"),
        sunrise: first_time("sunrise"),
        sunset: first_time("sunset"),
        days,
    })
}

/// What a WMO weather code means, in a few words.
pub fn describe(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 | 63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 | 73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 | 81 => "Showers",
        82 => "Heavy showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

/// The bar's icon for a code, by day or by night.
pub fn icon(code: u8, day: bool) -> &'static str {
    match code {
        0 => {
            if day {
                "sun"
            } else {
                "moon"
            }
        }
        1 | 2 => {
            if day {
                "cloud-sun"
            } else {
                "cloud-moon"
            }
        }
        3 => "cloud",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 | 80..=82 => "rain",
        71..=77 | 85 | 86 => "snow",
        95..=99 => "storm",
        _ => "cloud",
    }
}

/// The city your time zone is named after: Asia/Kathmandu → Kathmandu, America/New_York →
/// New York. Read from the system, never looked up.
pub fn time_zone_city() -> Option<String> {
    let zone = std::fs::read_link("/etc/localtime").ok()?.to_string_lossy().into_owned();
    let city = zone.rsplit('/').next()?.replace('_', " ");
    (!city.is_empty() && city != "UTC" && city != "localtime").then_some(city)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_forecast() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"current_units":{"temperature_2m":"°C","wind_speed_10m":"km/h"},
                "current":{"temperature_2m":17.4,"apparent_temperature":16.1,"weather_code":2,"is_day":0,"relative_humidity_2m":63,"wind_speed_10m":5.8},
                "daily":{"time":["2026-11-21","2026-11-22"],"weather_code":[2,61],"temperature_2m_max":[21.3,19.0],"temperature_2m_min":[8.2,9.1],
                         "precipitation_probability_max":[5,70],"sunrise":["2026-11-21T06:41"],"sunset":["2026-11-21T17:08"]}}"#,
        )
        .unwrap();
        let w = parse(&json, "Kathmandu").unwrap();
        assert_eq!((w.temperature, w.code, w.is_day, w.sunrise.as_str(), w.sunset.as_str()), (17.4, 2, false, "06:41", "17:08"));
        assert_eq!(w.days.len(), 2);
        assert_eq!((w.days[1].code, w.days[1].rain), (61, 70));
        assert_eq!(icon(w.code, w.is_day), "cloud-moon");
        assert_eq!(describe(w.days[1].code), "Rain");
        assert!(parse(&serde_json::json!({}), "x").is_none());
    }
}
