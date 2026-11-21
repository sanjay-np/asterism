//! The weather, from Open-Meteo: no account, no key. Where: the coordinates or city in the
//! config, else the city your time zone is named after. Only that place (or its name, to find
//! it) is ever sent. Fetched every 20 minutes, sooner again after a failure.

use super::Out;
use crate::config::WeatherSettings;
use crate::model::{Command, Update};
use crate::weather::Weather;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct Place {
    latitude: f64,
    longitude: f64,
    name: String,
}

async fn get_json(url: &str) -> Option<serde_json::Value> {
    let output = super::output("curl", &["-sSf", "-m", "15", "-A", "asterism", url]).await?;
    serde_json::from_str(&output).ok()
}

fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn locate(settings: &WeatherSettings) -> Option<Place> {
    if let (Some(latitude), Some(longitude)) = (settings.latitude, settings.longitude) {
        return Some(Place { latitude, longitude, name: settings.city.trim().to_string() });
    }
    let city = Some(settings.city.trim().to_string()).filter(|c| !c.is_empty()).or_else(crate::weather::time_zone_city)?;
    let json = get_json(&format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json", encode(&city))).await?;
    let found = json.get("results")?.get(0)?;
    Some(Place { latitude: found.get("latitude")?.as_f64()?, longitude: found.get("longitude")?.as_f64()?, name: found.get("name").and_then(|n| n.as_str()).unwrap_or(&city).to_string() })
}

async fn fetch(place: &Place, settings: &WeatherSettings) -> Option<Weather> {
    let imperial = settings.units.eq_ignore_ascii_case("imperial");
    let units = if imperial { "&temperature_unit=fahrenheit&wind_speed_unit=mph" } else { "" };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.3}&longitude={:.3}&current=temperature_2m,apparent_temperature,weather_code,is_day,relative_humidity_2m,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&timezone=auto&forecast_days=5{units}",
        place.latitude, place.longitude
    );
    crate::weather::parse(&get_json(&url).await?, &place.name)
}

pub async fn run(out: Out, mut commands: UnboundedReceiver<Command>) {
    let mut settings: Option<WeatherSettings> = None;
    let mut place: Option<Place> = None;
    let mut next = Instant::now() + Duration::from_secs(3600 * 24);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(next) => {}
            command = commands.recv() => match command {
                Some(Command::WeatherSettings(new)) => {
                    if settings.as_ref() == Some(&new) {
                        continue;
                    }
                    settings = Some(new);
                    place = None;
                }
                Some(Command::WeatherRefresh) => {}
                Some(_) => continue,
                None => return,
            },
        }
        let Some(wanted) = settings.clone() else { continue };
        if !wanted.enabled {
            out.send(Update::Weather(Weather::default()));
            next = Instant::now() + Duration::from_secs(3600 * 24);
            continue;
        }
        if place.is_none() {
            place = locate(&wanted).await;
        }
        let weather = match &place {
            Some(place) => fetch(place, &wanted).await,
            None => None,
        };
        next = Instant::now()
            + match weather {
                Some(weather) => {
                    out.send(Update::Weather(weather));
                    Duration::from_secs(20 * 60)
                }
                None => {
                    log::debug!("weather: nothing this time, trying again soon");
                    Duration::from_secs(2 * 60)
                }
            };
    }
}
