//! Open-Meteo backed weather snapshots for desktop cards.
//!
//! Network responses and the on-disk cache are both capped at one MiB. A failed
//! refresh may reuse the last valid response, but that response is always marked
//! stale in the returned snapshot.

use super::{Action, Page, PageSnapshot, Row, RowMeta};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Local, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    LazyLock, Mutex,
};
use std::time::{Duration as StdDuration, Instant};

pub const MAX_LOCATION_CHARS: usize = 120;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const CACHE_TTL: Duration = Duration::minutes(15);
const REQUEST_TIMEOUT: StdDuration = StdDuration::from_secs(8);
const RETRY_BACKOFF: StdDuration = StdDuration::from_secs(60);
const MAX_HOURLY_ROWS: usize = 12;
const MAX_DAILY_ROWS: usize = 7;
const CACHE_VERSION: u8 = 1;
const GEOCODING_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";

static CACHE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RECENT_FAILURES: LazyLock<Mutex<BTreeMap<String, (Instant, String)>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherConfig {
    pub location: String,
    pub latitude_e6: Option<i32>,
    pub longitude_e6: Option<i32>,
}

impl WeatherConfig {
    pub fn validate(&self) -> Result<()> {
        if self.location.chars().count() > MAX_LOCATION_CHARS {
            bail!("天气城市名称不能超过 {MAX_LOCATION_CHARS} 个字符");
        }
        if self.location.chars().any(char::is_control) {
            bail!("天气城市名称不能包含控制字符");
        }
        match (self.latitude_e6, self.longitude_e6) {
            (Some(latitude), Some(longitude)) => {
                if !(-90_000_000..=90_000_000).contains(&latitude)
                    || !(-180_000_000..=180_000_000).contains(&longitude)
                {
                    bail!("天气坐标超出经纬度范围");
                }
            }
            (None, None) => {
                if self.location.trim().is_empty() {
                    bail!("请先设置天气城市");
                }
            }
            _ => bail!("纬度和经度必须同时设置"),
        }
        Ok(())
    }

    fn cache_key(&self) -> String {
        let value = format!(
            "{}\0{}\0{}",
            self.location.trim(),
            self.latitude_e6.map_or_else(String::new, |v| v.to_string()),
            self.longitude_e6
                .map_or_else(String::new, |v| v.to_string())
        );
        hex_digest(Sha256::digest(value.as_bytes()).as_slice())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherSnapshot {
    pub location: String,
    pub source: String,
    pub fetched_at: String,
    pub stale: bool,
    pub error: Option<String>,
    pub current: WeatherCurrent,
    pub daily: Vec<WeatherDay>,
    pub hourly: Vec<WeatherHour>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherCurrent {
    pub time: String,
    pub temperature_c: f64,
    pub weather_code: i32,
    pub relative_humidity_percent: i32,
    pub wind_speed_kmh: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherDay {
    pub date: String,
    pub weather_code: Option<i32>,
    pub temperature_min_c: Option<f64>,
    pub temperature_max_c: Option<f64>,
    pub precipitation_probability_max: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherHour {
    pub time: String,
    pub temperature_c: Option<f64>,
    pub weather_code: Option<i32>,
    pub precipitation_probability: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WeatherCache {
    version: u8,
    config_key: String,
    snapshot: WeatherSnapshot,
}

#[derive(Debug, Deserialize)]
struct GeocodingResponse {
    results: Option<Vec<GeocodedPlace>>,
}

#[derive(Debug, Deserialize)]
struct GeocodedPlace {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    current: CurrentResponse,
    hourly: HourlyResponse,
    daily: DailyResponse,
}

#[derive(Debug, Deserialize)]
struct CurrentResponse {
    time: String,
    temperature_2m: f64,
    relative_humidity_2m: i32,
    weather_code: i32,
    wind_speed_10m: f64,
}

#[derive(Debug, Deserialize)]
struct HourlyResponse {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    precipitation_probability: Vec<Option<i32>>,
    weather_code: Vec<Option<i32>>,
}

#[derive(Debug, Deserialize)]
struct DailyResponse {
    time: Vec<String>,
    temperature_2m_min: Vec<Option<f64>>,
    temperature_2m_max: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<i32>>,
    weather_code: Vec<Option<i32>>,
}

/// Fetch current conditions plus a seven-day and twelve-hour forecast.
///
/// `cache_dir` is an application-owned cache directory. It contains only a
/// hashed configuration key and public forecast data, never credentials.
pub fn fetch_snapshot(config: &WeatherConfig, cache_dir: &Path) -> Result<WeatherSnapshot> {
    config.validate()?;
    let key = config.cache_key();
    let cached = read_cache(cache_dir, &key);
    let now = Utc::now();
    if let Some(snapshot) = cached
        .as_ref()
        .filter(|snapshot| cache_is_fresh(snapshot, now))
    {
        return Ok(snapshot.clone());
    }

    if let Some(message) = recent_failure(&key, Instant::now()) {
        let hint = format!("网络刷新退避中（60 秒内）：{message}");
        if let Some(snapshot) = cached {
            return Ok(mark_stale(snapshot, hint));
        }
        bail!("天气刷新退避中（60 秒内）：{message}");
    }

    let result = fetch_live(config, now);
    match result {
        Ok(mut snapshot) => {
            clear_failure(&key);
            let mut cache_value = snapshot.clone();
            cache_value.stale = false;
            cache_value.error = None;
            if let Err(error) = write_cache(cache_dir, &key, &cache_value) {
                snapshot.error = Some(format!(
                    "天气已更新，但无法保存缓存：{}",
                    short_error(&error)
                ));
            }
            Ok(snapshot)
        }
        Err(error) => {
            let message = short_error(&error);
            record_failure(&key, message.clone(), Instant::now());
            if let Some(snapshot) = cached {
                Ok(mark_stale(
                    snapshot,
                    format!("刷新失败，正在显示旧缓存：{message}"),
                ))
            } else {
                Err(error)
            }
        }
    }
}

/// Convert a weather page to rows. The persistent page field is a city string;
/// no default location or device geolocation is inferred.
pub fn to_page_snapshot(page: &Page, cache_dir: &Path) -> Result<PageSnapshot> {
    let location = page.utility.location.trim();
    if location.is_empty() {
        return Ok(PageSnapshot {
            title: page.title.clone(),
            subtitle: "数据来源：Open-Meteo".into(),
            rows: vec![Row {
                id: "weather-configure".into(),
                title: "尚未设置天气城市".into(),
                detail: "选择城市后查看当前天气和预报".into(),
                action: Some(Action::DesktopUtility("configure".into())),
                meta: RowMeta {
                    icon: "weather".into(),
                    ..RowMeta::default()
                },
                ..Row::default()
            }],
            empty_message: "请设置天气城市".into(),
        });
    }
    let config = WeatherConfig {
        location: location.to_owned(),
        ..WeatherConfig::default()
    };
    let weather = fetch_snapshot(&config, cache_dir)?;
    Ok(render_snapshot(page, &weather))
}

fn fetch_live(config: &WeatherConfig, now: DateTime<Utc>) -> Result<WeatherSnapshot> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();

    let (latitude, longitude, display_location) = match (config.latitude_e6, config.longitude_e6) {
        (Some(latitude), Some(longitude)) => {
            let label = if config.location.trim().is_empty() {
                format!(
                    "{:.3}, {:.3}",
                    latitude as f64 / 1_000_000.0,
                    longitude as f64 / 1_000_000.0
                )
            } else {
                config.location.trim().to_owned()
            };
            (
                latitude as f64 / 1_000_000.0,
                longitude as f64 / 1_000_000.0,
                label,
            )
        }
        (None, None) => geocode(&agent, config.location.trim())?,
        _ => unreachable!("WeatherConfig::validate checks coordinate pairs"),
    };

    let mut url = url::Url::parse(FORECAST_URL)?;
    url.query_pairs_mut()
        .append_pair("latitude", &format!("{latitude:.6}"))
        .append_pair("longitude", &format!("{longitude:.6}"))
        .append_pair(
            "current",
            "temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m",
        )
        .append_pair(
            "hourly",
            "temperature_2m,precipitation_probability,weather_code",
        )
        .append_pair(
            "daily",
            "weather_code,temperature_2m_min,temperature_2m_max,precipitation_probability_max",
        )
        .append_pair("forecast_days", "7")
        .append_pair("timezone", "auto");
    let forecast: ForecastResponse =
        request_json(&agent, url.as_str()).context("无法获取 Open-Meteo 天气预报")?;
    let mut snapshot = parse_forecast(forecast, display_location)?;
    snapshot.fetched_at = now.to_rfc3339();
    Ok(snapshot)
}

fn geocode(agent: &ureq::Agent, location: &str) -> Result<(f64, f64, String)> {
    let mut url = url::Url::parse(GEOCODING_URL)?;
    url.query_pairs_mut()
        .append_pair("name", location)
        .append_pair("count", "5")
        .append_pair("language", "en")
        .append_pair("format", "json");
    let response: GeocodingResponse =
        request_json(agent, url.as_str()).context("无法查询天气城市")?;
    parse_geocoding(response, location)
}

fn parse_geocoding(response: GeocodingResponse, query: &str) -> Result<(f64, f64, String)> {
    let place = response
        .results
        .and_then(|results| results.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("没有找到天气城市“{query}”"))?;
    if !place.latitude.is_finite()
        || !place.longitude.is_finite()
        || !(-90.0..=90.0).contains(&place.latitude)
        || !(-180.0..=180.0).contains(&place.longitude)
        || place.name.trim().is_empty()
        || place.name.chars().count() > MAX_LOCATION_CHARS
        || place.name.chars().any(char::is_control)
    {
        bail!("城市查询返回了无效坐标");
    }
    let mut parts = vec![place.name];
    if let Some(admin) = place.admin1.filter(|value| !value.is_empty()) {
        if !parts.iter().any(|part| part == &admin) {
            parts.push(admin);
        }
    }
    if let Some(country) = place.country.filter(|value| !value.is_empty()) {
        if !parts.iter().any(|part| part == &country) {
            parts.push(country);
        }
    }
    Ok((place.latitude, place.longitude, parts.join(" · ")))
}

fn request_json<T: DeserializeOwned>(agent: &ureq::Agent, address: &str) -> Result<T> {
    let mut response = agent.get(address).call()?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("读取 Open-Meteo 响应失败")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        bail!("Open-Meteo 响应超过 1 MiB 限制");
    }
    serde_json::from_slice(&bytes).context("Open-Meteo 返回了无效 JSON")
}

fn parse_forecast(response: ForecastResponse, location: String) -> Result<WeatherSnapshot> {
    let current = response.current;
    if !current.temperature_2m.is_finite()
        || !current.wind_speed_10m.is_finite()
        || !(0..=100).contains(&current.relative_humidity_2m)
    {
        bail!("Open-Meteo 当前天气数据不完整或无效");
    }
    let hourly = response.hourly;
    let hourly_len = hourly.time.len();
    if hourly_len < MAX_HOURLY_ROWS
        || hourly.temperature_2m.len() != hourly_len
        || hourly.precipitation_probability.len() != hourly_len
        || hourly.weather_code.len() != hourly_len
        || hourly_len > 24 * 16
    {
        bail!("Open-Meteo 逐小时天气数据不完整");
    }
    let hour_start = hourly
        .time
        .iter()
        .position(|time| time >= &current.time)
        .unwrap_or(0);
    let hours = (hour_start..hourly_len)
        .take(MAX_HOURLY_ROWS)
        .map(|index| {
            let temperature = hourly.temperature_2m[index];
            if temperature.is_some_and(|value| !value.is_finite()) {
                bail!("Open-Meteo 逐小时气温数据无效");
            }
            Ok(WeatherHour {
                time: hourly.time[index].clone(),
                temperature_c: temperature,
                weather_code: hourly.weather_code[index],
                precipitation_probability: hourly.precipitation_probability[index],
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if hours.len() < MAX_HOURLY_ROWS {
        bail!("Open-Meteo 未返回完整的未来 12 小时数据");
    }

    let daily = response.daily;
    let daily_len = daily.time.len();
    if daily_len < MAX_DAILY_ROWS
        || daily.temperature_2m_min.len() != daily_len
        || daily.temperature_2m_max.len() != daily_len
        || daily.precipitation_probability_max.len() != daily_len
        || daily.weather_code.len() != daily_len
        || daily_len > 16
    {
        bail!("Open-Meteo 七日天气数据不完整");
    }
    let days = (0..MAX_DAILY_ROWS)
        .map(|index| {
            for temperature in [
                daily.temperature_2m_min[index],
                daily.temperature_2m_max[index],
            ] {
                if temperature.is_some_and(|value| !value.is_finite()) {
                    bail!("Open-Meteo 每日气温数据无效");
                }
            }
            Ok(WeatherDay {
                date: daily.time[index].clone(),
                weather_code: daily.weather_code[index],
                temperature_min_c: daily.temperature_2m_min[index],
                temperature_max_c: daily.temperature_2m_max[index],
                precipitation_probability_max: daily.precipitation_probability_max[index],
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(WeatherSnapshot {
        location,
        source: "Open-Meteo".into(),
        fetched_at: Utc::now().to_rfc3339(),
        stale: false,
        error: None,
        current: WeatherCurrent {
            time: current.time,
            temperature_c: current.temperature_2m,
            weather_code: current.weather_code,
            relative_humidity_percent: current.relative_humidity_2m,
            wind_speed_kmh: current.wind_speed_10m,
        },
        daily: days,
        hourly: hours,
    })
}

fn render_snapshot(page: &Page, weather: &WeatherSnapshot) -> PageSnapshot {
    let fetched = DateTime::parse_from_rfc3339(&weather.fetched_at)
        .map(|time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| weather.fetched_at.clone());
    let freshness = if weather.stale {
        format!(
            "旧缓存 · {}",
            weather.error.as_deref().unwrap_or("更新失败")
        )
    } else if let Some(error) = weather.error.as_deref() {
        format!("{error} · 更新于 {fetched}")
    } else {
        format!("更新于 {fetched}")
    };
    let mut rows = Vec::with_capacity(1 + weather.hourly.len() + weather.daily.len());
    rows.push(Row {
        id: "weather-current".into(),
        title: format!("当前 · {}°C", format_temp(weather.current.temperature_c)),
        detail: format!(
            "{} · 湿度 {}% · 风速 {} km/h",
            weather_description(weather.current.weather_code),
            weather.current.relative_humidity_percent,
            format_temp(weather.current.wind_speed_kmh)
        ),
        meta: RowMeta {
            group: "当前天气".into(),
            always_detail: true,
            icon: weather_icon(weather.current.weather_code).into(),
            ..RowMeta::default()
        },
        ..Row::default()
    });
    for (index, hour) in weather.hourly.iter().enumerate() {
        let label = hour
            .time
            .split_once('T')
            .map(|(_, time)| time)
            .unwrap_or(&hour.time);
        rows.push(Row {
            id: format!("weather-hour-{index}"),
            title: label.into(),
            detail: format!(
                "{}°C · {} · 降水概率 {}%",
                hour.temperature_c.map_or_else(|| "—".into(), format_temp),
                hour.weather_code
                    .map(weather_description)
                    .unwrap_or_else(|| "天气未知".into()),
                hour.precipitation_probability
                    .map_or_else(|| "—".into(), |value| value.to_string())
            ),
            meta: RowMeta {
                group: "未来 12 小时".into(),
                always_detail: true,
                icon: weather_icon(hour.weather_code.unwrap_or(-1)).into(),
                date: hour.time.clone(),
                ..RowMeta::default()
            },
            ..Row::default()
        });
    }
    for (index, day) in weather.daily.iter().enumerate() {
        rows.push(Row {
            id: format!("weather-day-{index}"),
            title: day.date.clone(),
            detail: format!(
                "最低 {}°C · 最高 {}°C · 降水概率 {}% · {}",
                day.temperature_min_c
                    .map_or_else(|| "—".into(), format_temp),
                day.temperature_max_c
                    .map_or_else(|| "—".into(), format_temp),
                day.precipitation_probability_max
                    .map_or_else(|| "—".into(), |value| value.to_string()),
                day.weather_code
                    .map(weather_description)
                    .unwrap_or_else(|| "天气未知".into())
            ),
            meta: RowMeta {
                group: "未来 7 天".into(),
                always_detail: true,
                icon: weather_icon(day.weather_code.unwrap_or(-1)).into(),
                date: day.date.clone(),
                ..RowMeta::default()
            },
            ..Row::default()
        });
    }
    PageSnapshot {
        title: page.title.clone(),
        subtitle: format!("{} · {} · {freshness}", weather.location, weather.source),
        rows,
        empty_message: "暂无天气数据".into(),
    }
}

fn weather_description(code: i32) -> String {
    match code {
        0 => "晴朗",
        1 => "大致晴朗",
        2 => "局部多云",
        3 => "阴天",
        45 | 48 => "雾",
        51 | 53 | 55 => "毛毛雨",
        56 | 57 => "冻毛毛雨",
        61 | 63 | 65 => "降雨",
        66 | 67 => "冻雨",
        71 | 73 | 75 | 77 => "降雪",
        80 | 81 | 82 => "阵雨",
        85 | 86 => "阵雪",
        95 => "雷暴",
        96 | 99 => "雷暴伴冰雹",
        _ => "天气未知",
    }
    .into()
}

fn weather_icon(code: i32) -> &'static str {
    match code {
        0 | 1 => "Sun",
        2 | 3 | 45 | 48 => "☁",
        51..=67 | 80..=82 => "☂",
        71..=77 | 85..=86 => "❄",
        95..=99 => "ϟ",
        _ => "Sun",
    }
}

fn format_temp(value: f64) -> String {
    format!("{value:.1}")
}

fn read_cache(cache_dir: &Path, key: &str) -> Option<WeatherSnapshot> {
    let path = cache_path(cache_dir, key);
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return None;
    }
    let cached: WeatherCache = serde_json::from_slice(&bytes).ok()?;
    if cached.version != CACHE_VERSION
        || cached.config_key != key
        || !cache_shape_valid(&cached.snapshot)
    {
        return None;
    }
    Some(cached.snapshot)
}

fn cache_is_fresh(snapshot: &WeatherSnapshot, now: DateTime<Utc>) -> bool {
    let Ok(fetched) = DateTime::parse_from_rfc3339(&snapshot.fetched_at) else {
        return false;
    };
    let age = now.signed_duration_since(fetched.with_timezone(&Utc));
    age >= Duration::zero() && age <= CACHE_TTL
}

fn mark_stale(mut snapshot: WeatherSnapshot, message: String) -> WeatherSnapshot {
    snapshot.stale = true;
    snapshot.error = Some(message);
    snapshot
}

fn recent_failure(key: &str, now: Instant) -> Option<String> {
    let failures = RECENT_FAILURES.lock().ok()?;
    let (failed_at, message) = failures.get(key)?;
    now.checked_duration_since(*failed_at)
        .filter(|elapsed| *elapsed < RETRY_BACKOFF)
        .map(|_| message.clone())
}

fn record_failure(key: &str, message: String, now: Instant) {
    let Ok(mut failures) = RECENT_FAILURES.lock() else {
        return;
    };
    failures.retain(|_, (failed_at, _)| {
        now.checked_duration_since(*failed_at)
            .is_none_or(|elapsed| elapsed < RETRY_BACKOFF)
    });
    failures.insert(key.to_owned(), (now, message));
    while failures.len() > 64 {
        failures.pop_first();
    }
}

fn clear_failure(key: &str) {
    if let Ok(mut failures) = RECENT_FAILURES.lock() {
        failures.remove(key);
    }
}

fn write_cache(cache_dir: &Path, key: &str, snapshot: &WeatherSnapshot) -> Result<()> {
    let path = cache_path(cache_dir, key);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("天气缓存路径无效"))?;
    fs::create_dir_all(parent).context("无法创建天气缓存目录")?;
    let mut cache_snapshot = snapshot.clone();
    cache_snapshot.stale = false;
    cache_snapshot.error = None;
    let cached = WeatherCache {
        version: CACHE_VERSION,
        config_key: key.to_owned(),
        snapshot: cache_snapshot,
    };
    let bytes = serde_json::to_vec(&cached)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        bail!("天气缓存超过 1 MiB 限制");
    }
    let sequence = CACHE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".weather-cache-{}-{sequence}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .context("无法创建天气临时缓存文件")?;
    let write_result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        match fs::rename(&temp, &path) {
            Ok(()) => Ok(()),
            Err(error) if path.exists() => {
                fs::remove_file(&path).context("无法替换旧天气缓存")?;
                fs::rename(&temp, &path)
                    .with_context(|| format!("无法发布天气缓存（原子替换错误：{error}）"))
            }
            Err(error) => Err(error).context("无法发布天气缓存"),
        }
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(temp);
    }
    write_result
}

fn cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join("weather").join(format!("{key}.json"))
}

fn cache_shape_valid(snapshot: &WeatherSnapshot) -> bool {
    if snapshot.source != "Open-Meteo"
        || snapshot.stale
        || snapshot.error.is_some()
        || snapshot.location.chars().count() > 256
        || snapshot.location.chars().any(char::is_control)
        || snapshot.hourly.len() != MAX_HOURLY_ROWS
        || snapshot.daily.len() != MAX_DAILY_ROWS
        || snapshot.current.time.len() > 64
        || snapshot.fetched_at.len() > 64
        || !(0..=100).contains(&snapshot.current.relative_humidity_percent)
        || !snapshot.current.temperature_c.is_finite()
        || !snapshot.current.wind_speed_kmh.is_finite()
    {
        return false;
    }
    snapshot.hourly.iter().all(|hour| {
        hour.time.len() <= 64
            && hour.temperature_c.is_none_or(f64::is_finite)
            && hour
                .precipitation_probability
                .is_none_or(|value| (0..=100).contains(&value))
    }) && snapshot.daily.iter().all(|day| {
        day.date.len() <= 16
            && day.temperature_min_c.is_none_or(f64::is_finite)
            && day.temperature_max_c.is_none_or(f64::is_finite)
            && day
                .precipitation_probability_max
                .is_none_or(|value| (0..=100).contains(&value))
    })
}

fn short_error(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    text.chars().take(220).collect()
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forecast_fixture() -> ForecastResponse {
        let hourly_times = (0..24)
            .map(|hour| format!("2026-09-26T{hour:02}:00"))
            .collect::<Vec<_>>();
        let daily_times = [
            "2026-09-26",
            "2026-09-27",
            "2026-09-28",
            "2026-09-29",
            "2026-09-30",
            "2026-10-01",
            "2026-10-02",
        ]
        .map(str::to_owned);
        let json = serde_json::json!({
            "current": {
                "time": "2026-09-26T08:00",
                "temperature_2m": 19.3,
                "relative_humidity_2m": 64,
                "weather_code": 2,
                "wind_speed_10m": 12.4
            },
            "hourly": {
                "time": hourly_times,
                "temperature_2m": vec![20.0; 24],
                "precipitation_probability": vec![15; 24],
                "weather_code": vec![2; 24]
            },
            "daily": {
                "time": daily_times,
                "temperature_2m_min": vec![12.0; 7],
                "temperature_2m_max": vec![22.0; 7],
                "precipitation_probability_max": vec![30; 7],
                "weather_code": vec![2; 7]
            }
        });
        serde_json::from_value(json).unwrap()
    }

    fn sample_snapshot() -> WeatherSnapshot {
        parse_forecast(forecast_fixture(), "Test City".into()).unwrap()
    }

    #[test]
    fn fixture_parsing_produces_current_twelve_hours_and_seven_days() {
        let snapshot = sample_snapshot();
        assert_eq!(snapshot.source, "Open-Meteo");
        assert_eq!(snapshot.location, "Test City");
        assert_eq!(snapshot.current.temperature_c, 19.3);
        assert_eq!(snapshot.current.relative_humidity_percent, 64);
        assert_eq!(snapshot.hourly.len(), 12);
        assert_eq!(snapshot.hourly[0].time, "2026-09-26T08:00");
        assert_eq!(snapshot.daily.len(), 7);
        assert_eq!(weather_description(95), "雷暴");
        let page = Page::new(super::super::Module::Weather);
        let rendered = render_snapshot(&page, &snapshot);
        assert_eq!(rendered.rows.len(), 20);
        assert!(rendered.rows.iter().all(|row| row.meta.always_detail));
    }

    #[test]
    fn cache_roundtrips_and_expires_after_fifteen_minutes() {
        let root = std::env::temp_dir().join(format!(
            "mochi-weather-cache-test-{}-{}",
            std::process::id(),
            CACHE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let config = WeatherConfig {
            location: "Test City".into(),
            ..WeatherConfig::default()
        };
        let key = config.cache_key();
        let mut snapshot = sample_snapshot();
        let now = Utc::now();
        snapshot.fetched_at = now.to_rfc3339();
        write_cache(&root, &key, &snapshot).unwrap();
        let read = read_cache(&root, &key).unwrap();
        assert_eq!(read, snapshot);
        assert!(cache_is_fresh(&read, now + Duration::minutes(14)));
        assert!(!cache_is_fresh(&read, now + Duration::minutes(16)));
        assert!(read_cache(&root, "another-config").is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_reader_rejects_oversized_or_invalid_cache() {
        let root = std::env::temp_dir().join(format!(
            "mochi-weather-cache-invalid-{}-{}",
            std::process::id(),
            CACHE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let directory = root.join("weather");
        fs::create_dir_all(&directory).unwrap();
        let key = "bounded";
        fs::write(cache_path(&root, key), vec![b'x'; MAX_RESPONSE_BYTES + 1]).unwrap();
        assert!(read_cache(&root, key).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_requires_a_location_or_a_valid_coordinate_pair() {
        assert!(WeatherConfig::default().validate().is_err());
        assert!(WeatherConfig {
            location: String::new(),
            latitude_e6: Some(10),
            longitude_e6: None,
        }
        .validate()
        .is_err());
        assert!(WeatherConfig {
            location: String::new(),
            latitude_e6: Some(91_000_000),
            longitude_e6: Some(0),
        }
        .validate()
        .is_err());
        assert!(WeatherConfig {
            location: String::new(),
            latitude_e6: Some(31_230_000),
            longitude_e6: Some(121_470_000),
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn config_json_roundtrips_and_older_location_only_json_loads() {
        let config = WeatherConfig {
            location: "上海".into(),
            latitude_e6: Some(31_230_000),
            longitude_e6: Some(121_470_000),
        };
        let encoded = serde_json::to_value(&config).unwrap();
        assert_eq!(
            serde_json::from_value::<WeatherConfig>(encoded).unwrap(),
            config
        );
        let older: WeatherConfig =
            serde_json::from_value(serde_json::json!({ "location": "上海" })).unwrap();
        assert_eq!(older.location, "上海");
        assert_eq!(older.latitude_e6, None);
        assert_eq!(older.longitude_e6, None);
    }

    #[test]
    fn geocoding_fixture_parses_city_without_network() {
        let fixture: GeocodingResponse = serde_json::from_value(serde_json::json!({
            "results": [{
                "name": "Shanghai",
                "latitude": 31.23,
                "longitude": 121.47,
                "admin1": "Shanghai",
                "country": "China"
            }]
        }))
        .unwrap();
        let result = parse_geocoding(fixture, "Shanghai").unwrap();
        assert_eq!((result.0, result.1), (31.23, 121.47));
        assert_eq!(result.2, "Shanghai · China");
    }

    #[test]
    fn failed_refresh_marks_cached_snapshot_stale() {
        let stale = mark_stale(
            sample_snapshot(),
            "刷新失败，正在显示旧缓存：fixture network error".into(),
        );
        assert!(stale.stale);
        assert!(stale.error.as_deref().unwrap().contains("刷新失败"));
        assert_eq!(stale.current.temperature_c, 19.3);
    }

    #[test]
    fn failed_network_requests_back_off_for_sixty_seconds_by_config_key() {
        let key = format!("backoff-test-{}", std::process::id());
        let failed_at = Instant::now() - RETRY_BACKOFF / 2;
        record_failure(&key, "fixture timeout".into(), failed_at);
        assert_eq!(
            recent_failure(&key, Instant::now()).as_deref(),
            Some("fixture timeout")
        );
        assert_eq!(
            recent_failure(&key, Instant::now() + RETRY_BACKOFF).as_deref(),
            None
        );
        clear_failure(&key);
    }

    #[test]
    fn unconfigured_page_is_a_prompt_not_a_fake_forecast() {
        let mut page = Page::new(super::super::Module::Weather);
        page.utility.location.clear();
        let snapshot = to_page_snapshot(&page, Path::new("unused-cache")).unwrap();
        assert!(!snapshot.rows.is_empty());
        assert_eq!(snapshot.rows[0].title, "尚未设置天气城市");
        assert_eq!(
            snapshot.rows[0].action,
            Some(Action::DesktopUtility("configure".into()))
        );
    }

    #[test]
    #[ignore = "manual smoke test; requires internet access to Open-Meteo"]
    fn open_meteo_shenzhen_live_smoke_and_cache() {
        let cache_dir = std::env::temp_dir().join(format!(
            "mochi-weather-live-smoke-{}-{}",
            std::process::id(),
            CACHE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&cache_dir).unwrap();
        let config = WeatherConfig {
            location: "Shenzhen".into(),
            ..WeatherConfig::default()
        };
        let first = fetch_snapshot(&config, &cache_dir).unwrap();
        assert_eq!(first.source, "Open-Meteo");
        assert!(!first.location.is_empty());
        assert!(first.current.temperature_c.is_finite());
        assert!((0..=100).contains(&first.current.relative_humidity_percent));
        assert!(first.current.wind_speed_kmh.is_finite());
        assert_eq!(first.hourly.len(), MAX_HOURLY_ROWS);
        assert_eq!(first.daily.len(), MAX_DAILY_ROWS);
        println!(
            "Weather source: {}; location: {}; current: {:.1}°C",
            first.source, first.location, first.current.temperature_c
        );

        let second = fetch_snapshot(&config, &cache_dir).unwrap();
        assert_eq!(
            second.fetched_at, first.fetched_at,
            "second call should use fresh cache"
        );
        assert!(!second.stale);
        assert!(read_cache(&cache_dir, &config.cache_key()).is_some());
        let _ = std::fs::remove_dir_all(cache_dir);
    }
}
