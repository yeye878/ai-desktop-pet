pub mod cache;
pub mod edge_tts;
pub mod playback;

use base64::{engine::general_purpose, Engine as _};
use minimp3::{Decoder, Error as Mp3Error};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsVoice {
    pub id: String,
    pub name: String,
    pub language: String,
    pub gender: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub voice: String,
    pub rate: i32,
    pub pitch: i32,
    pub volume: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsResult {
    pub audio_path: String,
    pub cached: bool,
    pub audio_data_url: Option<String>,
    pub mime_type: Option<String>,
}

/// 后端汇报给前端的 TTS 缓存命中率指标。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub total_bytes: u64,
    /// 自进程启动以来累计的命中率（0..=1）。`hits + misses == 0` 时返回 0。
    pub hit_rate: f64,
    /// 上一次 `TtsManager::new` 时清理掉的旧版本 (.mp3) 缓存项数量。
    pub legacy_mp3_purged: u64,
}

pub struct TtsManager {
    pub cache: cache::AudioCache,
    hits: AtomicU64,
    misses: AtomicU64,
    legacy_mp3_purged: AtomicU64,
}

const EDGE_AUDIO_MIME: &str = "audio/wav";

impl TtsManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        let cache = cache::AudioCache::new(cache_dir);
        // 上线一次性清理上一版本（`edge` + .mp3）遗留的缓存项——它们键命名空间
        // 不再兼容、永不会被命中，留着只是占盘。
        let purged = cache.purge_legacy_mp3() as u64;
        Self {
            cache,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            legacy_mp3_purged: AtomicU64::new(purged),
        }
    }

    pub async fn synthesize_to_file(&self, req: &TtsRequest) -> Result<(String, bool), String> {
        // 把空白排版归一化进缓存键与合成请求：相同语义不同空白的请求共享缓存项，
        // 显著提升命中率；归一化只压缩/剔除空白，不改语音语义。
        let canonical_text = cache::canonical_text_for_key(&req.text);
        let canon_req = TtsRequest {
            text: canonical_text.clone(),
            voice: req.voice.clone(),
            rate: req.rate,
            pitch: req.pitch,
            volume: req.volume,
        };

        let cache_key = self.cache.make_key(
            "edge-wav-v2",
            &canon_req.voice,
            canon_req.rate,
            canon_req.pitch,
            canon_req.volume,
            &canonical_text,
        );
        if let Some(path) = self.cache.get(&cache_key) {
            // 验证缓存文件确实可读且非空，防止外部删除/截断导致播放失败被误判为缓存命中
            let is_valid = std::path::Path::new(&path)
                .metadata()
                .map(|m| m.len() > 0)
                .unwrap_or(false);
            if is_valid {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok((path, true));
            }
        }
        let mp3_audio = edge_tts::synthesize(&canon_req).await?;
        let audio = mp3_to_wav(&mp3_audio)?;
        let path = self.cache.put(&cache_key, &audio)?;
        self.misses.fetch_add(1, Ordering::Relaxed);
        Ok((path, false))
    }

    pub async fn synthesize(&self, req: &TtsRequest) -> Result<TtsResult, String> {
        let (path, cached) = self.synthesize_to_file(req).await?;
        let data = fs::read(&path).map_err(|e| format!("读取 TTS 缓存失败: {e}"))?;
        Ok(TtsResult {
            audio_data_url: Some(audio_data_url(&data, EDGE_AUDIO_MIME)),
            audio_path: path,
            cached,
            mime_type: Some(EDGE_AUDIO_MIME.to_string()),
        })
    }

    pub async fn list_voices() -> Result<Vec<TtsVoice>, String> {
        edge_tts::list_voices().await
    }

    /// 汇报自进程启动以来的累计命中率与盘上缓存项规模。
    pub fn cache_stats(&self) -> TtsCacheStats {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let (entries, total_bytes) = self.cache.inventory();
        let total = hits.saturating_add(misses);
        let hit_rate = if total == 0 { 0.0 } else { hits as f64 / total as f64 };
        TtsCacheStats {
            hits,
            misses,
            entries,
            total_bytes,
            hit_rate,
            legacy_mp3_purged: self.legacy_mp3_purged.load(Ordering::Relaxed),
        }
    }
}

fn audio_data_url(data: &[u8], mime_type: &str) -> String {
    format!(
        "data:{mime_type};base64,{}",
        general_purpose::STANDARD.encode(data)
    )
}

fn mp3_to_wav(mp3: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = Decoder::new(Cursor::new(mp3));
    let mut samples: Vec<i16> = Vec::new();
    let mut sample_rate: Option<u32> = None;
    let mut channels: Option<u16> = None;

    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                sample_rate.get_or_insert(frame.sample_rate as u32);
                channels.get_or_insert(frame.channels as u16);
                samples.extend(frame.data);
            }
            Err(Mp3Error::Eof) => break,
            Err(err) => return Err(format!("MP3 解码失败: {err}")),
        }
    }

    let sample_rate = sample_rate.ok_or_else(|| "MP3 解码失败: 没有音频帧".to_string())?;
    let channels = channels.unwrap_or(1).max(1);
    Ok(pcm_i16_to_wav(&samples, sample_rate, channels))
}

fn pcm_i16_to_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = samples.len().saturating_mul(2).min(u32::MAX as usize) as u32;
    let bits_per_sample = 16u16;
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let mut wav = Vec::with_capacity(44 + data_len as usize);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36u32.saturating_add(data_len)).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples.iter().take(data_len as usize / 2) {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn edge_synthesis_decodes_to_wav() {
        let req = TtsRequest {
            text: "你好，这是一次试听测试。".to_string(),
            voice: "zh-CN-XiaoxiaoNeural".to_string(),
            rate: 0,
            pitch: 0,
            volume: 100,
        };
        let mp3 = edge_tts::synthesize(&req).await.expect("edge synthesis");
        let wav = mp3_to_wav(&mp3).expect("mp3 to wav");
        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(wav.len() > 44);
    }
}
