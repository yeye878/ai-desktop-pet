/**
 * 天气服务 - 前端
 */

export interface WeatherInfo {
  city: string;
  temperature: number;
  feels_like: number;
  humidity: number;
  description: string;
  wind_speed: number;
  icon: string;
  timestamp: number;
}

export interface WeatherUpdateEvent {
  message: string;
  sent_today: boolean;
}

/**
 * 根据天气状况生成暖心建议
 */
export function getWeatherAdvice(weather: WeatherInfo): string {
  const temp = weather.temperature;
  const desc = weather.description.toLowerCase();
  const humidity = weather.humidity;

  let advice = '';

  // 温度建议
  if (temp < 0) {
    advice += '外面很冷，记得穿厚外套，注意保暖哦！🧣';
  } else if (temp < 10) {
    advice += '天气有点凉，出门记得加件外套。🧥';
  } else if (temp < 20) {
    advice += '温度适宜，适合外出活动。🌿';
  } else if (temp < 30) {
    advice += '天气不错，记得多喝水保持水分。💧';
  } else if (temp < 35) {
    advice += '天气炎热，注意防暑降温，多喝水。🍦';
  } else {
    advice += '高温预警！尽量避免户外活动，做好防暑措施。☀';
  }

  // 天气状况建议
  if (desc.includes('rain') || desc.includes('雨')) {
    advice += '\n记得带伞，小心路滑。🌂';
  } else if (desc.includes('snow') || desc.includes('雪')) {
    advice += '\n下雪路滑，出行注意安全。⛄';
  } else if (desc.includes('fog') || desc.includes('雾') || desc.includes('霾')) {
    advice += '\n能见度低，开车注意安全，建议戴口罩。😷';
  } else if (desc.includes('wind') || desc.includes('风')) {
    advice += '\n风大注意安全，外出小心。🍃';
  }

  // 湿度建议
  if (humidity > 80) {
    advice += '\n湿度较高，注意防潮。💧';
  } else if (humidity < 30) {
    advice += '\n空气干燥，记得多喝水，注意皮肤保湿。🧴';
  }

  if (!advice) {
    advice = '今天天气不错，祝你有美好的一天！😊';
  }

  return advice;
}

/**
 * 格式化天气信息为简洁的显示文本
 */
export function formatWeatherDisplay(weather: WeatherInfo): string {
  return `${weather.icon} ${weather.temperature}°C ${weather.city}`;
}

/**
 * 格式化天气信息为详细的消息（用于系统消息）
 */
export function formatWeatherMessage(weather: WeatherInfo): string {
  const advice = getWeatherAdvice(weather);

  return `🌤 今日天气

📍 ${weather.city}
🌡 温度: ${weather.temperature}°C (体感 ${weather.feels_like}°C)
💧 湿度: ${weather.humidity}%
🌬 风速: ${weather.wind_speed} km/h
📝 天气: ${weather.icon} ${weather.description}

${advice}`;
}
