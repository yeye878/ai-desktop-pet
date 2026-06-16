pub mod cache;
pub mod edge_tts;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

pub struct TtsManager {
    pub cache: cache::AudioCache,
}

impl TtsManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache: cache::AudioCache::new(cache_dir),
        }
    }

    pub async fn synthesize(&self, req: &TtsRequest) -> Result<TtsResult, String> {
        let cache_key = self.cache.make_key(
            "edge", &req.voice, req.rate, req.pitch, req.volume, &req.text,
        );
        if let Some(path) = self.cache.get(&cache_key) {
            // 验证缓存文件确实可读，防止外部删除导致播放失败
            if std::path::Path::new(&path)
                .metadata()
                .map(|m| m.len() > 0)
                .unwrap_or(false)
            {
                return Ok(TtsResult {
                    audio_path: path,
                    cached: true,
                });
            }
        }
        let audio = edge_tts::synthesize(req).await?;
        let path = self.cache.put(&cache_key, &audio)?;
        Ok(TtsResult {
            audio_path: path,
            cached: false,
        })
    }

    pub async fn list_voices() -> Result<Vec<TtsVoice>, String> {
        edge_tts::list_voices().await
    }
}
