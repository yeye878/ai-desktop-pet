use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

pub struct AudioCache {
    dir: PathBuf,
}

impl AudioCache {
    pub fn new(dir: PathBuf) -> Self {
        fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    pub fn make_key(
        &self,
        provider: &str,
        voice: &str,
        rate: i32,
        pitch: i32,
        volume: i32,
        text: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(provider.as_bytes());
        hasher.update(voice.as_bytes());
        hasher.update(rate.to_le_bytes());
        hasher.update(pitch.to_le_bytes());
        hasher.update(volume.to_le_bytes());
        hasher.update(text.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// 把任意输入限制为 64 个十六进制字符以内的安全文件名组件
    fn safe_key(key: &str) -> Option<String> {
        if key.is_empty() || key.len() > 64 {
            return None;
        }
        if key.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(key.to_ascii_lowercase())
        } else {
            None
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let safe = Self::safe_key(key)?;
        let path = self.dir.join(format!("{safe}.mp3"));
        if path.exists() {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        }
    }

    pub fn put(&self, key: &str, data: &[u8]) -> Result<String, String> {
        let safe = Self::safe_key(key).ok_or_else(|| "TTS 缓存 key 不合法".to_string())?;
        let path = self.dir.join(format!("{safe}.mp3"));
        fs::write(&path, data).map_err(|e| format!("缓存写入失败: {e}"))?;
        Ok(path.to_string_lossy().to_string())
    }
}
