use crate::tts::{TtsRequest, TtsVoice};
use tokio_tungstenite::{connect_async, tungstenite};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::http::Request;
use futures_util::{SinkExt, StreamExt};
use sha2::{Sha256, Digest};
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};

const TRUSTED_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const VOICES_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list?trustedclienttoken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const WSS_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const OUTPUT_FORMAT: &str = "audio-24khz-96kbitrate-mono-mp3";
const EDGE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";
const EDGE_ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";
const WIN_EPOCH_OFFSET: u64 = 621_355_968_000_000_000;
const TICKS_PER_SEC: u64 = 10_000_000;
const ROUND_INTERVAL: u64 = 3_000_000_000;

fn compute_sec_ms_gec() -> String {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ticks = unix_secs * TICKS_PER_SEC + WIN_EPOCH_OFFSET;
    let rounded = ticks - (ticks % ROUND_INTERVAL);
    let input = format!("{rounded}{TRUSTED_TOKEN}");
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
    let items: Vec<serde_json::Value> = resp.json().await.map_err(|e| format!("解析声音列表失败: {e}"))?;
    let voices = items.iter().filter_map(|v| {
        Some(TtsVoice {
            id: v.get("ShortName")?.as_str()?.to_string(),
            name: v.get("FriendlyName")?.as_str()?.to_string(),
            language: v.get("Locale")?.as_str()?.to_string(),
            gender: v.get("Gender")?.as_str()?.to_string(),
        })
    }).collect();
    Ok(voices)
}

pub async fn synthesize(req: &TtsRequest) -> Result<Vec<u8>, String> {
    let conn_id = Uuid::new_v4().to_string().replace('-', "");
    let sec_ms_gec = compute_sec_ms_gec();
    let url = format!(
        "{WSS_URL}?TrustedClientToken={TRUSTED_TOKEN}&ConnectionId={conn_id}&Sec-MS-GEC={sec_ms_gec}"
    );

    let request = Request::builder()
        .uri(&url)
        .header("User-Agent", EDGE_UA)
        .header("Origin", EDGE_ORIGIN)
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        .header("Sec-MS-GEC", &sec_ms_gec)
        .header("Sec-MS-GEC-Version", "1-130.0.2849.68")
        .header("Host", "speech.platform.bing.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .body(())
        .map_err(|e| format!("构建请求失败: {e}"))?;

    let (ws_stream, _) = connect_async(request)
        .await
        .map_err(|e| format!("WebSocket 连接失败: {e}"))?;

    let (mut write, mut read) = ws_stream.split();

    let config_msg = format!(
        "Content-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
        {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\
        \"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\
        \"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}"
    );
    write.send(Message::Text(config_msg)).await.map_err(|e| format!("发送配置失败: {e}"))?;

    let ssml = build_ssml(&req.voice, req.rate, req.pitch, req.volume, &req.text);
    let request_id = Uuid::new_v4().to_string().replace('-', "");
    let ssml_msg = format!(
        "X-RequestId:{request_id}\r\nContent-Type:application/ssml+xml\r\n\
        Path:ssml\r\n\r\n{ssml}"
    );
    write.send(Message::Text(ssml_msg)).await.map_err(|e| format!("发送 SSML 失败: {e}"))?;

    let mut audio_data: Vec<u8> = Vec::new();
    let header_separator = b"Path:audio\r\n";

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                if let Some(pos) = find_subsequence(&data, header_separator) {
                    let audio_start = pos + header_separator.len();
                    if audio_start < data.len() {
                        audio_data.extend_from_slice(&data[audio_start..]);
                    }
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

    if audio_data.is_empty() {
        return Err("未收到音频数据".to_string());
    }
    Ok(audio_data)
}

fn build_ssml(voice: &str, rate: i32, pitch: i32, volume: i32, text: &str) -> String {
    let rate_str = if rate >= 0 { format!("+{rate}%") } else { format!("{rate}%") };
    let pitch_str = if pitch >= 0 { format!("+{pitch}Hz") } else { format!("{pitch}Hz") };
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='zh-CN'>\
        <voice name='{voice}'>\
        <prosody rate='{rate_str}' pitch='{pitch_str}' volume='{volume}'>\
        {escaped}\
        </prosody></voice></speak>"
    )
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
