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

    pub fn make_key(&self, provider: &str, voice: &str, rate: i32, pitch: i32, volume: i32, text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(provider.as_bytes());
        hasher.update(voice.as_bytes());
        hasher.update(rate.to_le_bytes());
        hasher.update(pitch.to_le_bytes());
        hasher.update(volume.to_le_bytes());
        hasher.update(text.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let path = self.dir.join(format!("{key}.mp3"));
        if path.exists() {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        }
    }

    pub fn put(&self, key: &str, data: &[u8]) -> Result<String, String> {
        let path = self.dir.join(format!("{key}.mp3"));
        fs::write(&path, data).map_err(|e| format!("缓存写入失败: {e}"))?;
        Ok(path.to_string_lossy().to_string())
    }
}
