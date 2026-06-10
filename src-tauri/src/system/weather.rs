use serde::{Deserialize, Serialize};
use url::Url;

pub const DEFAULT_WEATHER_API_URL: &str = "http://wttr.in";
const LEGACY_HTTPS_WTTR_API_URL: &str = "https://wttr.in";
const WEATHER_REQUEST_TIMEOUT_SECS: u64 = 20;
const FALLBACK_WEATHER_API_URL: &str = "https://api.52vmy.cn/api/query/tian";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherInfo {
    pub city: String,
    pub temperature: f64,
    pub feels_like: f64,
    pub humidity: i32,
    pub description: String,
    pub wind_speed: f64,
    pub icon: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeatherConfig {
    pub enabled: bool,
    pub location: String,
    pub api_url: String,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            location: String::new(),
            api_url: DEFAULT_WEATHER_API_URL.to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WttrResponse {
    current_condition: Vec<CurrentCondition>,
    nearest_area: Vec<NearestArea>,
}

#[derive(Debug, Deserialize)]
struct CurrentCondition {
    #[serde(rename = "temp_C")]
    temp_c: String,
    #[serde(rename = "FeelsLikeC")]
    feels_like_c: String,
    humidity: String,
    #[serde(rename = "weatherDesc")]
    weather_desc: Vec<WeatherDesc>,
    #[serde(rename = "windspeedKmph")]
    windspeed_kmph: String,
    #[serde(rename = "weatherCode")]
    weather_code: String,
}

#[derive(Debug, Deserialize)]
struct WeatherDesc {
    value: String,
}

#[derive(Debug, Deserialize)]
struct NearestArea {
    #[serde(rename = "areaName")]
    area_name: Vec<AreaName>,
}

#[derive(Debug, Deserialize)]
struct AreaName {
    value: String,
}

#[derive(Debug, Deserialize)]
struct VmyWeatherResponse {
    code: i32,
    msg: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct VmyWeatherData {
    city: String,
    temp: String,
    weather: String,
    #[serde(rename = "windSpeed")]
    wind_speed: String,
    current: Option<VmyCurrentWeather>,
}

#[derive(Debug, Deserialize)]
struct VmyCurrentWeather {
    humidity: String,
    weather: String,
    temp: String,
    #[serde(rename = "windSpeed")]
    wind_speed: String,
}

/// 根据天气代码返回图标
fn weather_icon(code: &str) -> String {
    match code {
        "113" => "☀".to_string(),
        "116" => "⛅".to_string(),
        "119" | "122" => "☁".to_string(),
        "143" | "248" | "260" => "🌫".to_string(),
        "176" | "263" | "266" | "293" | "296" | "299" | "302" | "305" | "311" | "353" => {
            "🌧".to_string()
        }
        "179" | "182" | "185" | "227" | "230" | "320" | "323" | "326" | "329" | "332" | "335"
        | "338" | "350" | "362" | "365" | "374" | "377" => "🌨".to_string(),
        "200" | "386" | "389" | "392" | "395" => "⛈".to_string(),
        _ => "🌡".to_string(),
    }
}

fn weather_icon_from_description(description: &str) -> String {
    let desc = description.to_lowercase();
    if desc.contains("雷") || desc.contains("thunder") {
        "⛈".to_string()
    } else if desc.contains("雨") || desc.contains("rain") {
        "🌧".to_string()
    } else if desc.contains("雪") || desc.contains("snow") {
        "🌨".to_string()
    } else if desc.contains("雾")
        || desc.contains("霾")
        || desc.contains("fog")
        || desc.contains("haze")
    {
        "🌫".to_string()
    } else if desc.contains("云") || desc.contains("cloud") {
        "☁".to_string()
    } else if desc.contains("晴") || desc.contains("sunny") || desc.contains("clear") {
        "☀".to_string()
    } else {
        "🌡".to_string()
    }
}

pub fn normalize_weather_config(config: WeatherConfig) -> WeatherConfig {
    let mut api_url = config.api_url.trim().trim_end_matches('/').to_string();
    if api_url.eq_ignore_ascii_case(LEGACY_HTTPS_WTTR_API_URL) {
        api_url = DEFAULT_WEATHER_API_URL.to_string();
    }

    WeatherConfig {
        enabled: config.enabled,
        location: config.location.trim().to_string(),
        api_url: if api_url.is_empty() {
            DEFAULT_WEATHER_API_URL.to_string()
        } else {
            api_url
        },
    }
}

fn validate_weather_url(url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|e| format!("天气API地址无效: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("天气API地址必须以 http:// 或 https:// 开头".to_string());
    }
    if parsed.host_str().unwrap_or_default().is_empty() {
        return Err("天气API地址缺少主机名".to_string());
    }
    Ok(())
}

fn weather_api_label(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
        .unwrap_or_else(|| "配置的天气API".to_string())
}

pub fn build_weather_url(config: &WeatherConfig) -> Result<String, String> {
    let config = normalize_weather_config(config.clone());
    validate_weather_url(&config.api_url)?;

    let encoded_location = urlencoding::encode(&config.location);
    let url = if config.api_url.contains("{location}") {
        config
            .api_url
            .replace("{location}", encoded_location.as_ref())
    } else if config.location.is_empty() {
        format!("{}/?format=j1", config.api_url)
    } else {
        format!("{}/{}?format=j1", config.api_url, encoded_location)
    };

    validate_weather_url(&url)?;
    Ok(url)
}

fn parse_f64_field(value: &str, field: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("天气数据字段 {field} 不是有效数字: {value}"))
}

fn parse_i32_field(value: &str, field: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| format!("天气数据字段 {field} 不是有效整数: {value}"))
}

fn parse_percent_i32(value: &str, field: &str) -> Result<i32, String> {
    parse_i32_field(value.trim().trim_end_matches('%'), field)
}

fn parse_wind_speed_kmph(value: &str) -> f64 {
    let numeric = value
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>()
        .parse::<f64>()
        .unwrap_or(0.0);

    if value.contains('级') {
        match numeric as i32 {
            0 => 1.0,
            1 => 5.0,
            2 => 11.0,
            3 => 19.0,
            4 => 28.0,
            5 => 38.0,
            6 => 49.0,
            7 => 61.0,
            8 => 74.0,
            9 => 88.0,
            10 => 102.0,
            11 => 117.0,
            _ => numeric,
        }
    } else {
        numeric
    }
}

fn parse_wttr_response(wttr: WttrResponse) -> Result<WeatherInfo, String> {
    let current = wttr
        .current_condition
        .first()
        .ok_or_else(|| "天气数据为空".to_string())?;

    let city_name = wttr
        .nearest_area
        .first()
        .and_then(|area| area.area_name.first())
        .map(|area| area.value.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "未知城市".to_string());

    let temperature = parse_f64_field(&current.temp_c, "temp_C")?;
    let feels_like = parse_f64_field(&current.feels_like_c, "FeelsLikeC")?;
    let humidity = parse_i32_field(&current.humidity, "humidity")?;
    let wind_speed = parse_f64_field(&current.windspeed_kmph, "windspeedKmph")?;
    let description = current
        .weather_desc
        .first()
        .map(|desc| desc.value.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "未知".to_string());
    let icon = weather_icon(&current.weather_code);

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Ok(WeatherInfo {
        city: city_name,
        temperature,
        feels_like,
        humidity,
        description,
        wind_speed,
        icon,
        timestamp,
    })
}

fn parse_vmy_response(response: VmyWeatherResponse) -> Result<WeatherInfo, String> {
    if response.code != 200 {
        return Err(format!("52vmy 天气源返回失败: {}", response.msg));
    }

    let data: VmyWeatherData = serde_json::from_value(response.data)
        .map_err(|e| format!("解析 52vmy 天气数据失败: {e}"))?;
    let current = data.current.as_ref();
    let temperature = current
        .map(|item| parse_f64_field(&item.temp, "current.temp"))
        .unwrap_or_else(|| parse_f64_field(&data.temp, "temp"))?;
    let humidity = current
        .map(|item| parse_percent_i32(&item.humidity, "current.humidity"))
        .transpose()?
        .unwrap_or(0);
    let description = current
        .map(|item| item.weather.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| data.weather.clone());
    let wind_speed = current
        .map(|item| parse_wind_speed_kmph(&item.wind_speed))
        .unwrap_or_else(|| parse_wind_speed_kmph(&data.wind_speed));
    let icon = weather_icon_from_description(&description);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Ok(WeatherInfo {
        city: data.city,
        temperature,
        feels_like: temperature,
        humidity,
        description,
        wind_speed,
        icon,
        timestamp,
    })
}

pub async fn get_weather_with_client(
    client: &reqwest::Client,
    config: &WeatherConfig,
) -> Result<WeatherInfo, String> {
    let config = normalize_weather_config(config.clone());
    if !config.enabled {
        return Err("天气功能已关闭".to_string());
    }

    match get_wttr_weather_with_client(client, &config).await {
        Ok(weather) => Ok(weather),
        Err(primary_error) if should_try_fallback_weather(&config) => {
            match get_fallback_weather_with_client(client, &config.location).await {
                Ok(weather) => Ok(weather),
                Err(fallback_error) => Err(format!(
                    "{}\n兜底天气源也失败: {}",
                    primary_error, fallback_error
                )),
            }
        }
        Err(primary_error)
            if is_wttr_api_url(&config.api_url) && config.location.trim().is_empty() =>
        {
            Err(format!(
                "{}\nwttr.in 自动定位当前不可用；请在天气设置里填写城市/地区后再试。",
                primary_error
            ))
        }
        Err(primary_error) => Err(primary_error),
    }
}

async fn get_wttr_weather_with_client(
    client: &reqwest::Client,
    config: &WeatherConfig,
) -> Result<WeatherInfo, String> {
    let url = build_weather_url(&config)?;
    let api_label = weather_api_label(&url);

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(WEATHER_REQUEST_TIMEOUT_SECS),
        client
            .get(&url)
            .header("User-Agent", "ai-desktop-pet/1.0")
            .send(),
    )
    .await
    .map_err(|_| {
        format!(
            "请求天气API超时（{}秒，{}）",
            WEATHER_REQUEST_TIMEOUT_SECS, api_label
        )
    })?
    .map_err(|e| format!("请求天气API失败（{}）: {e}", api_label))?;

    if !response.status().is_success() {
        return Err(format!(
            "天气API返回错误（{}）: {}",
            api_label,
            response.status()
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("读取天气API响应失败（{}）: {e}", api_label))?;
    let wttr: WttrResponse = serde_json::from_str(&body).map_err(|e| {
        let preview = body
            .chars()
            .take(120)
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "解析天气数据失败（{}）: {e}。当前天气 API 需要返回 wttr.in format=j1 兼容 JSON；响应开头: {}",
            api_label, preview
        )
    })?;

    parse_wttr_response(wttr)
}

fn is_wttr_api_url(api_url: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|parsed| {
            parsed
                .host_str()
                .map(|host| host.eq_ignore_ascii_case("wttr.in"))
        })
        .unwrap_or(false)
}

fn should_try_fallback_weather(config: &WeatherConfig) -> bool {
    is_wttr_api_url(&config.api_url) && !config.location.trim().is_empty()
}

async fn get_fallback_weather_with_client(
    client: &reqwest::Client,
    location: &str,
) -> Result<WeatherInfo, String> {
    let encoded_location = urlencoding::encode(location.trim());
    let url = format!("{FALLBACK_WEATHER_API_URL}?city={encoded_location}");
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(WEATHER_REQUEST_TIMEOUT_SECS),
        client
            .get(&url)
            .header("User-Agent", "ai-desktop-pet/1.0")
            .send(),
    )
    .await
    .map_err(|_| {
        format!(
            "请求兜底天气源超时（{}秒，api.52vmy.cn）",
            WEATHER_REQUEST_TIMEOUT_SECS
        )
    })?
    .map_err(|e| format!("请求兜底天气源失败（api.52vmy.cn）: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("兜底天气源返回错误: {}", response.status()));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("读取兜底天气源响应失败: {e}"))?;
    let parsed: VmyWeatherResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析兜底天气源响应失败: {e}"))?;
    parse_vmy_response(parsed)
}

/// 根据天气状况生成暖心提醒
pub fn get_weather_advice(weather: &WeatherInfo) -> String {
    let temp = weather.temperature;
    let desc = weather.description.to_lowercase();
    let humidity = weather.humidity;

    let mut advice = String::new();

    // 温度建议
    if temp < 0.0 {
        advice.push_str("外面很冷，记得穿厚外套，注意保暖哦！🧣");
    } else if temp < 10.0 {
        advice.push_str("天气有点凉，出门记得加件外套。🧥");
    } else if temp < 20.0 {
        advice.push_str("温度适宜，适合外出活动。🌿");
    } else if temp < 30.0 {
        advice.push_str("天气不错，记得多喝水保持水分。💧");
    } else if temp < 35.0 {
        advice.push_str("天气炎热，注意防暑降温，多喝水。🍦");
    } else {
        advice.push_str("高温预警！尽量避免户外活动，做好防暑措施。☀");
    }

    // 天气状况建议
    if desc.contains("rain") || desc.contains("雨") {
        advice.push_str("\n记得带伞，小心路滑。🌂");
    } else if desc.contains("snow") || desc.contains("雪") {
        advice.push_str("\n下雪路滑，出行注意安全。⛄");
    } else if desc.contains("fog") || desc.contains("雾") || desc.contains("霾") {
        advice.push_str("\n能见度低，开车注意安全，建议戴口罩。😷");
    } else if desc.contains("wind") || desc.contains("风") {
        advice.push_str("\n风大注意安全，外出小心。🍃");
    }

    // 湿度建议
    if humidity > 80 {
        advice.push_str("\n湿度较高，注意防潮。💧");
    } else if humidity < 30 {
        advice.push_str("\n空气干燥，记得多喝水，注意皮肤保湿。🧴");
    }

    if advice.is_empty() {
        advice = "今天天气不错，祝你有美好的一天！😊".to_string();
    }

    advice
}

/// 格式化天气消息（含暖心提醒）
pub fn format_weather_message(weather: &WeatherInfo) -> String {
    let advice = get_weather_advice(weather);

    format!(
        "🌤 今日天气\n\n\
         📍 {}\n\
         🌡 温度: {}°C (体感 {}°C)\n\
         💧 湿度: {}%\n\
         🌬 风速: {} km/h\n\
         📝 天气: {} {}\n\n\
         {}",
        weather.city,
        weather.temperature,
        weather.feels_like,
        weather.humidity,
        weather.wind_speed,
        weather.icon,
        weather.description,
        advice
    )
}

/// 获取今天的日期字符串
pub fn get_today_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> WttrResponse {
        serde_json::from_value(serde_json::json!({
            "current_condition": [{
                "temp_C": "23",
                "FeelsLikeC": "25",
                "humidity": "61",
                "weatherDesc": [{ "value": "Partly cloudy" }],
                "windspeedKmph": "9",
                "weatherCode": "116"
            }],
            "nearest_area": [{
                "areaName": [{ "value": "Tokyo" }],
                "country": [{ "value": "Japan" }]
            }]
        }))
        .expect("sample response")
    }

    fn sample_vmy_response() -> VmyWeatherResponse {
        serde_json::from_value(serde_json::json!({
            "code": 200,
            "msg": "成功",
            "data": {
                "city": "上海",
                "temp": "29.7",
                "weather": "多云",
                "windSpeed": "<3级",
                "current": {
                    "humidity": "33%",
                    "weather": "晴",
                    "temp": "29.7",
                    "windSpeed": "1级"
                }
            }
        }))
        .expect("sample vmy response")
    }

    #[test]
    fn normalizes_weather_config_defaults_and_trims() {
        let config = normalize_weather_config(WeatherConfig {
            enabled: true,
            location: "  Tokyo  ".to_string(),
            api_url: " https://wttr.in/ ".to_string(),
        });

        assert!(config.enabled);
        assert_eq!(config.location, "Tokyo");
        assert_eq!(config.api_url, DEFAULT_WEATHER_API_URL);

        let defaulted = normalize_weather_config(WeatherConfig {
            enabled: false,
            location: " ".to_string(),
            api_url: " ".to_string(),
        });

        assert!(!defaulted.enabled);
        assert_eq!(defaulted.location, "");
        assert_eq!(defaulted.api_url, DEFAULT_WEATHER_API_URL);
    }

    #[test]
    fn builds_weather_urls_for_auto_city_and_template() {
        let auto = WeatherConfig {
            enabled: true,
            location: "".to_string(),
            api_url: DEFAULT_WEATHER_API_URL.to_string(),
        };
        assert_eq!(
            build_weather_url(&auto).expect("auto url"),
            "http://wttr.in/?format=j1"
        );

        let city = WeatherConfig {
            enabled: true,
            location: "上海".to_string(),
            api_url: DEFAULT_WEATHER_API_URL.to_string(),
        };
        assert_eq!(
            build_weather_url(&city).expect("city url"),
            "http://wttr.in/%E4%B8%8A%E6%B5%B7?format=j1"
        );

        let template = WeatherConfig {
            enabled: true,
            location: "New York".to_string(),
            api_url: "https://weather.example.test/{location}?format=j1".to_string(),
        };
        assert_eq!(
            build_weather_url(&template).expect("template url"),
            "https://weather.example.test/New%20York?format=j1"
        );
    }

    #[test]
    fn parses_wttr_response_into_weather_info() {
        let weather = parse_wttr_response(sample_response()).expect("weather");

        assert_eq!(weather.city, "Tokyo");
        assert_eq!(weather.temperature, 23.0);
        assert_eq!(weather.feels_like, 25.0);
        assert_eq!(weather.humidity, 61);
        assert_eq!(weather.description, "Partly cloudy");
        assert_eq!(weather.wind_speed, 9.0);
        assert_eq!(weather.icon, "⛅");
        assert!(weather.timestamp > 0);
    }

    #[test]
    fn parse_wttr_response_rejects_invalid_numbers() {
        let mut response = sample_response();
        response.current_condition[0].temp_c = "not-a-number".to_string();

        let error = parse_wttr_response(response).expect_err("invalid temperature should fail");
        assert!(error.contains("temp_C"));
    }

    #[test]
    fn parses_vmy_response_into_weather_info() {
        let weather = parse_vmy_response(sample_vmy_response()).expect("weather");

        assert_eq!(weather.city, "上海");
        assert_eq!(weather.temperature, 29.7);
        assert_eq!(weather.feels_like, 29.7);
        assert_eq!(weather.humidity, 33);
        assert_eq!(weather.description, "晴");
        assert_eq!(weather.wind_speed, 5.0);
        assert_eq!(weather.icon, "☀");
        assert!(weather.timestamp > 0);
    }
}
