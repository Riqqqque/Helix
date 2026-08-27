use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use thiserror::Error;

const GEOCODING_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const USER_AGENT: &str = "Helix/0.1 (+https://github.com/Riqqqque/Helix)";
const RESPONSE_LIMIT_BYTES: u64 = 128 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_CACHE_ENTRIES: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TemperatureUnit {
    Celsius,
    Fahrenheit,
}

impl TemperatureUnit {
    fn query_value(self) -> &'static str {
        match self {
            Self::Celsius => "celsius",
            Self::Fahrenheit => "fahrenheit",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WeatherQuery {
    pub(crate) location: String,
    pub(crate) unit: TemperatureUnit,
}

impl WeatherQuery {
    pub(crate) fn validate(mut self) -> Result<Self, WeatherError> {
        self.location = self.location.trim().to_owned();
        let length = self.location.chars().count();
        if !(2..=120).contains(&length)
            || self
                .location
                .chars()
                .any(|character| character.is_control())
        {
            return Err(WeatherError::InvalidLocation);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WeatherForecast {
    provider: &'static str,
    location: WeatherLocation,
    current: CurrentWeather,
    daily: Vec<DailyWeather>,
    fetched_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WeatherLocation {
    name: String,
    admin_area: Option<String>,
    country: String,
    timezone: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CurrentWeather {
    observed_at: String,
    temperature: f64,
    apparent_temperature: f64,
    relative_humidity_percent: f64,
    precipitation: f64,
    weather_code: u16,
    wind_speed: f64,
    temperature_unit: String,
    precipitation_unit: String,
    wind_speed_unit: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DailyWeather {
    date: String,
    weather_code: u16,
    temperature_max: f64,
    temperature_min: f64,
    precipitation_probability_max: Option<f64>,
}

#[derive(Debug, Error)]
pub(crate) enum WeatherError {
    #[error("Enter a city, postal code, or city and region between 2 and 120 characters.")]
    InvalidLocation,
    #[error("No matching location was found.")]
    LocationNotFound,
    #[error("The weather provider is temporarily unavailable.")]
    ProviderUnavailable,
    #[error("The weather provider returned an invalid response.")]
    InvalidProviderResponse,
}

#[derive(Clone, Debug, Deserialize)]
struct GeocodingResponse {
    #[serde(default)]
    results: Vec<GeocodingResult>,
}

#[derive(Clone, Debug, Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    timezone: String,
    country: String,
    admin1: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ForecastResponse {
    current: ProviderCurrent,
    current_units: ProviderCurrentUnits,
    daily: ProviderDaily,
}

#[derive(Clone, Debug, Deserialize)]
struct ProviderCurrent {
    time: String,
    temperature_2m: f64,
    apparent_temperature: f64,
    relative_humidity_2m: f64,
    precipitation: f64,
    weather_code: u16,
    wind_speed_10m: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct ProviderCurrentUnits {
    temperature_2m: String,
    precipitation: String,
    wind_speed_10m: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ProviderDaily {
    time: Vec<String>,
    weather_code: Vec<u16>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    precipitation_probability_max: Vec<Option<f64>>,
}

#[derive(Clone)]
struct CachedForecast {
    inserted_at: Instant,
    forecast: WeatherForecast,
}

static CACHE: OnceLock<Mutex<HashMap<(String, TemperatureUnit), CachedForecast>>> = OnceLock::new();
static HTTP_AGENT: OnceLock<ureq::Agent> = OnceLock::new();

pub(crate) fn forecast(query: WeatherQuery) -> Result<WeatherForecast, WeatherError> {
    let query = query.validate()?;
    let key = (query.location.to_lowercase(), query.unit);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache
        .lock()
        .map_err(|_| WeatherError::ProviderUnavailable)?
        .get(&key)
        .filter(|entry| entry.inserted_at.elapsed() < CACHE_TTL)
        .cloned()
    {
        return Ok(cached.forecast);
    }

    let location = geocode(&query.location)?;
    let forecast = fetch_forecast(&location, query.unit)?;
    let mut entries = cache
        .lock()
        .map_err(|_| WeatherError::ProviderUnavailable)?;
    entries.retain(|_, value| value.inserted_at.elapsed() < CACHE_TTL);
    if entries.len() >= MAX_CACHE_ENTRIES
        && let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, value)| value.inserted_at)
            .map(|(entry_key, _)| entry_key.clone())
    {
        entries.remove(&oldest);
    }
    entries.insert(
        key,
        CachedForecast {
            inserted_at: Instant::now(),
            forecast: forecast.clone(),
        },
    );
    Ok(forecast)
}

fn geocode(location: &str) -> Result<GeocodingResult, WeatherError> {
    let response: GeocodingResponse = fetch_json(
        GEOCODING_URL,
        &[
            ("name", location),
            ("count", "1"),
            ("language", "en"),
            ("format", "json"),
        ],
    )?;
    response
        .results
        .into_iter()
        .next()
        .ok_or(WeatherError::LocationNotFound)
}

fn fetch_forecast(
    location: &GeocodingResult,
    unit: TemperatureUnit,
) -> Result<WeatherForecast, WeatherError> {
    let latitude = location.latitude.to_string();
    let longitude = location.longitude.to_string();
    let response: ForecastResponse = fetch_json(
        FORECAST_URL,
        &[
            ("latitude", latitude.as_str()),
            ("longitude", longitude.as_str()),
            (
                "current",
                "temperature_2m,apparent_temperature,relative_humidity_2m,precipitation,weather_code,wind_speed_10m",
            ),
            (
                "daily",
                "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max",
            ),
            ("temperature_unit", unit.query_value()),
            ("wind_speed_unit", "mph"),
            ("precipitation_unit", "inch"),
            ("forecast_days", "5"),
            ("timezone", "auto"),
        ],
    )?;
    let count = response.daily.time.len();
    if count == 0
        || count > 5
        || response.daily.weather_code.len() != count
        || response.daily.temperature_2m_max.len() != count
        || response.daily.temperature_2m_min.len() != count
        || response.daily.precipitation_probability_max.len() != count
        || !provider_numbers_are_finite(&response)
    {
        return Err(WeatherError::InvalidProviderResponse);
    }
    let daily = (0..count)
        .map(|index| DailyWeather {
            date: response.daily.time[index].clone(),
            weather_code: response.daily.weather_code[index],
            temperature_max: response.daily.temperature_2m_max[index],
            temperature_min: response.daily.temperature_2m_min[index],
            precipitation_probability_max: response.daily.precipitation_probability_max[index],
        })
        .collect();
    Ok(WeatherForecast {
        provider: "Open-Meteo",
        location: WeatherLocation {
            name: location.name.clone(),
            admin_area: location.admin1.clone(),
            country: location.country.clone(),
            timezone: location.timezone.clone(),
        },
        current: CurrentWeather {
            observed_at: response.current.time,
            temperature: response.current.temperature_2m,
            apparent_temperature: response.current.apparent_temperature,
            relative_humidity_percent: response.current.relative_humidity_2m,
            precipitation: response.current.precipitation,
            weather_code: response.current.weather_code,
            wind_speed: response.current.wind_speed_10m,
            temperature_unit: response.current_units.temperature_2m,
            precipitation_unit: response.current_units.precipitation,
            wind_speed_unit: response.current_units.wind_speed_10m,
        },
        daily,
        fetched_at_unix_ms: helix_core::unix_timestamp_ms(),
    })
}

fn provider_numbers_are_finite(response: &ForecastResponse) -> bool {
    [
        response.current.temperature_2m,
        response.current.apparent_temperature,
        response.current.relative_humidity_2m,
        response.current.precipitation,
        response.current.wind_speed_10m,
    ]
    .into_iter()
    .chain(response.daily.temperature_2m_max.iter().copied())
    .chain(response.daily.temperature_2m_min.iter().copied())
    .chain(
        response
            .daily
            .precipitation_probability_max
            .iter()
            .flatten()
            .copied(),
    )
    .all(f64::is_finite)
}

fn fetch_json<T: for<'de> Deserialize<'de>>(
    url: &str,
    query: &[(&str, &str)],
) -> Result<T, WeatherError> {
    let agent = HTTP_AGENT.get_or_init(|| {
        ureq::Agent::from(
            ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(8)))
                .user_agent(USER_AGENT)
                .build(),
        )
    });
    let mut request = agent.get(url);
    for (name, value) in query {
        request = request.query(name, value);
    }
    let mut response = request
        .call()
        .map_err(|_| WeatherError::ProviderUnavailable)?;
    let body = response
        .body_mut()
        .with_config()
        .limit(RESPONSE_LIMIT_BYTES)
        .read_to_string()
        .map_err(|_| WeatherError::ProviderUnavailable)?;
    serde_json::from_str(&body).map_err(|_| WeatherError::InvalidProviderResponse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_query_rejects_controls_and_invalid_lengths() {
        assert!(
            WeatherQuery {
                location: " D".to_owned(),
                unit: TemperatureUnit::Fahrenheit,
            }
            .validate()
            .is_err()
        );
        assert!(
            WeatherQuery {
                location: "Denver\nColorado".to_owned(),
                unit: TemperatureUnit::Fahrenheit,
            }
            .validate()
            .is_err()
        );
        assert_eq!(
            WeatherQuery {
                location: " Denver, Colorado ".to_owned(),
                unit: TemperatureUnit::Fahrenheit,
            }
            .validate()
            .expect("valid query")
            .location,
            "Denver, Colorado"
        );
    }

    #[test]
    fn provider_shape_requires_aligned_finite_daily_data() {
        let response: ForecastResponse = serde_json::from_value(serde_json::json!({
            "current": {
                "time": "2026-08-26T12:00",
                "temperature_2m": 75.0,
                "apparent_temperature": 74.0,
                "relative_humidity_2m": 32.0,
                "precipitation": 0.0,
                "weather_code": 0,
                "wind_speed_10m": 4.0
            },
            "current_units": {
                "temperature_2m": "°F",
                "precipitation": "inch",
                "wind_speed_10m": "mp/h"
            },
            "daily": {
                "time": ["2026-08-26"],
                "weather_code": [0],
                "temperature_2m_max": [80.0],
                "temperature_2m_min": [55.0],
                "precipitation_probability_max": [10.0]
            }
        }))
        .expect("provider shape");
        assert!(provider_numbers_are_finite(&response));
    }
}
