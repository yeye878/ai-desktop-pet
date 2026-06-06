use serde::{Deserialize, Serialize};
use std::time::Duration;

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

#[derive(Debug, Deserialize)]
struct WttrResponse {
    current_condition: Vec<CurrentCondition>,
    nearest_area: Vec<NearestArea>,
}

#[derive(Debug, Deserialize)]
struct CurrentCondition {
    temp_C: String,
    FeelsLikeC: String,
    humidity: String,
    weatherDesc: Vec<WeatherDesc>,
    windspeedKmph: String,
    #[serde(rename = "weatherCode")]
    weather_code: String,
}

#[derive(Debug, Deserialize)]
struct WeatherDesc {
    value: String,
}

#[derive(Debug, Deserialize)]
struct NearestArea {
    areaName: Vec<AreaName>,
    country: Vec<AreaName>,
}

#[derive(Debug, Deserialize)]
struct AreaName {
    value: String,
}

/// 根据天气代码返回图标
fn weather_icon(code: &str) -> String {
    match code {
        "113" => "☀".to_string(),
        "116" => "⛅".to_string(),
        "119" | "122" => "☁".to_string(),
        "143" | "248" | "260" => "🌫".to_string(),
        "176" | "263" | "266" | "293" | "296" | "299" | "302" | "305" | "311" | "353" => "🌧".to_string(),
        "179" | "182" | "185" | "227" | "230" | "320" | "323" | "326" | "329" | "332" | "335" | "338" | "350" | "362" | "365" | "374" | "377" => "🌨".to_string(),
        "200" | "386" | "389" | "392" | "395" => "⛈".to_string(),
        _ => "🌡".to_string(),
    }
}

/// 获取天气信息（根据城市名）
pub async fn get_weather(city: &str) -> Result<WeatherInfo, String> {
    let url = if city.is_empty() {
        "https://wttr.in/?format=j1".to_string()
    } else {
        format!("https://wttr.in/{}?format=j1", urlencoding::encode(city))
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建HTTP客户端失败: {e}"))?;

    let response = client
        .get(&url)
        .header("User-Agent", "ai-desktop-pet/1.0")
        .send()
        .await
        .map_err(|e| format!("请求天气API失败: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("天气API返回错误: {}", response.status()));
    }

    let wttr: WttrResponse = response
        .json()
        .await
        .map_err(|e| format!("解析天气数据失败: {e}"))?;

    let current = wttr
        .current_condition
        .first()
        .ok_or("天气数据为空")?;

    let area = wttr.nearest_area.first();

    let city_name = area
        .and_then(|a| a.areaName.first())
        .map(|a| a.value.clone())
        .unwrap_or_else(|| "未知城市".to_string());

    let temperature = current.temp_C.parse::<f64>().unwrap_or(0.0);
    let feels_like = current.FeelsLikeC.parse::<f64>().unwrap_or(0.0);
    let humidity = current.humidity.parse::<i32>().unwrap_or(0);
    let wind_speed = current.windspeedKmph.parse::<f64>().unwrap_or(0.0);
    let description = current
        .weatherDesc
        .first()
        .map(|d| d.value.clone())
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

/// 获取天气信息（自动根据IP定位）
pub async fn get_weather_auto() -> Result<WeatherInfo, String> {
    get_weather("").await
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
