use crate::tts::{TtsRequest, TtsVoice};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, tungstenite};
use uuid::Uuid;

const TRUSTED_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const VOICES_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list?trustedclienttoken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const WSS_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const OUTPUT_FORMAT: &str = "audio-24khz-96kbitrate-mono-mp3";
const EDGE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";
const EDGE_ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";
const SEC_MS_GEC_VERSION: &str = "1-143.0.3650.75";
const WIN_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;

fn compute_sec_ms_gec() -> String {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ticks = unix_secs + WIN_EPOCH_OFFSET_SECS;
    let rounded = ticks - (ticks % 300);
    let final_ticks = rounded * 10_000_000;
    let input = format!("{final_ticks}{TRUSTED_TOKEN}");
    let hash = Sha256::digest(input.as_bytes());
    hex::encode_upper(hash)
}

pub async fn list_voices() -> Result<Vec<TtsVoice>, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(VOICES_URL)
        .header("User-Agent", EDGE_UA)
        .send()
        .await
        .map_err(|e| format!("获取声音列表失败: {e}"))?;
    let items: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| format!("解析声音列表失败: {e}"))?;
    let voices = items
        .iter()
        .filter_map(|v| {
            Some(TtsVoice {
                id: v.get("ShortName")?.as_str()?.to_string(),
                name: v.get("FriendlyName")?.as_str()?.to_string(),
                language: v.get("Locale")?.as_str()?.to_string(),
                gender: v.get("Gender")?.as_str()?.to_string(),
            })
        })
        .collect();
    Ok(voices)
}

pub async fn synthesize(req: &TtsRequest) -> Result<Vec<u8>, String> {
    let conn_id = Uuid::new_v4().to_string().replace('-', "");
    let sec_ms_gec = compute_sec_ms_gec();
    let muid = hex::encode_upper(Uuid::new_v4().as_bytes());
    let url = format!(
        "{WSS_URL}?TrustedClientToken={TRUSTED_TOKEN}&ConnectionId={conn_id}&Sec-MS-GEC={sec_ms_gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}"
    );

    let request = Request::builder()
        .uri(&url)
        .header("User-Agent", EDGE_UA)
        .header("Origin", EDGE_ORIGIN)
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        .header("Sec-MS-GEC", &sec_ms_gec)
        .header("Sec-MS-GEC-Version", SEC_MS_GEC_VERSION)
        .header("Host", "speech.platform.bing.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .header("Cookie", format!("muid={muid};"))
        .body(())
        .map_err(|e| format!("构建请求失败: {e}"))?;

    let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(15), connect_async(request))
        .await
        .map_err(|_| "WebSocket 连接超时".to_string())?
        .map_err(|e| format!("WebSocket 连接失败: {e}"))?;

    let (mut write, mut read) = ws_stream.split();

    let config_msg = format!(
        "Content-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
        {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\
        \"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\
        \"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}"
    );
    write
        .send(Message::Text(config_msg))
        .await
        .map_err(|e| format!("发送配置失败: {e}"))?;

    let ssml = build_ssml(&req.voice, req.rate, req.pitch, req.volume, &req.text);
    let request_id = Uuid::new_v4().to_string().replace('-', "");
    let ssml_msg = format!(
        "X-RequestId:{request_id}\r\nContent-Type:application/ssml+xml\r\n\
        Path:ssml\r\n\r\n{ssml}"
    );
    write
        .send(Message::Text(ssml_msg))
        .await
        .map_err(|e| format!("发送 SSML 失败: {e}"))?;

    let mut audio_data: Vec<u8> = Vec::new();

    let read_result = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    if let Some(payload) = extract_audio_payload(&data) {
                        audio_data.extend_from_slice(payload);
                    }
                }
                Ok(Message::Text(text)) => {
                    if text.contains("turn.end") {
                        break;
                    }
                }
                Err(e) => return Err(format!("WebSocket 读取错误: {e}")),
                _ => {}
            }
        }
        Ok(())
    })
    .await;

    // 优雅关闭 WebSocket
    write.send(Message::Close(None)).await.ok();

    match read_result {
        Err(_) => return Err("TTS 合成超时".to_string()),
        Ok(Err(e)) => return Err(e),
        Ok(Ok(())) => {}
    }

    if audio_data.is_empty() {
        return Err("未收到音频数据".to_string());
    }
    if !looks_like_mp3(&audio_data) {
        return Err("Edge TTS 返回的音频数据不是有效 MP3".to_string());
    }
    Ok(audio_data)
}

fn build_ssml(voice: &str, rate: i32, pitch: i32, volume: i32, text: &str) -> String {
    let rate_str = if rate >= 0 {
        format!("+{rate}%")
    } else {
        format!("{rate}%")
    };
    let pitch_str = if pitch >= 0 {
        format!("+{pitch}Hz")
    } else {
        format!("{pitch}Hz")
    };
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        // 过滤 XML 非法控制字符（U+0000-U+001F 中除 \t \n \r 外）
        .chars()
        .filter(|c| !c.is_control() || *c == '\t' || *c == '\n' || *c == '\r')
        .collect::<String>();
    let safe_voice = voice
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;");
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='zh-CN'>\
        <voice name='{safe_voice}'>\
        <prosody rate='{rate_str}' pitch='{pitch_str}' volume='{volume}'>\
        {escaped}\
        </prosody></voice></speak>"
    )
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn looks_like_mp3(data: &[u8]) -> bool {
    data.starts_with(b"ID3")
        || matches!(data.get(0..2), Some([0xFF, second]) if second & 0xE0 == 0xE0)
}

fn extract_audio_payload(data: &[u8]) -> Option<&[u8]> {
    if data.len() >= 2 {
        let header_len = u16::from_be_bytes([data[0], data[1]]) as usize;
        let payload_start = 2 + header_len;
        if payload_start <= data.len() {
            let header = &data[2..payload_start];
            if find_subsequence(header, b"Path:audio").is_some() {
                return Some(&data[payload_start..]);
            }
        }
    }

    if find_subsequence(data, b"Path:audio").is_none() {
        return None;
    }

    find_subsequence(data, b"\r\n\r\n").and_then(|pos| data.get(pos + 4..))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_voices() {
        match list_voices().await {
            Ok(v) => println!("Successfully listed {} voices", v.len()),
            Err(e) => panic!("list_voices failed: {}", e),
        }
    }

    #[tokio::test]
    async fn test_synthesize() {
        let req = TtsRequest {
            text: "你好，这是一次测试。".to_string(),
            voice: "zh-CN-XiaoxiaoNeural".to_string(),
            rate: 0,
            pitch: 0,
            volume: 100,
        };
        match synthesize(&req).await {
            Ok(audio) => println!("Successfully synthesized {} bytes of audio", audio.len()),
            Err(e) => panic!("synthesize failed: {}", e),
        }
    }
}
