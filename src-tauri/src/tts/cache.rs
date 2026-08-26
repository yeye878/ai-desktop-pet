use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

const AUDIO_EXTENSION: &str = "wav";

pub struct AudioCache {
    dir: PathBuf,
}

/// 把 TTS 请求文本归一化为缓存键的稳定形式：
/// - 去掉首尾空白
/// - 把任意空白序列（含 \n \r \t 多空格）压缩成单个空格
///
/// 不改变语音感知（Edge TTS 对中段空白也只产生短暂停顿或忽略），
/// 但能让 "你好" / "  你好  " / "你好\n" 共享同一条缓存项，
/// 显著提升相同语义不同空白排版的命中率。
pub fn canonical_text_for_key(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
        let path = self.dir.join(format!("{safe}.{AUDIO_EXTENSION}"));
        if path.exists() {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        }
    }

    pub fn put(&self, key: &str, data: &[u8]) -> Result<String, String> {
        let safe = Self::safe_key(key).ok_or_else(|| "TTS 缓存 key 不合法".to_string())?;
        let path = self.dir.join(format!("{safe}.{AUDIO_EXTENSION}"));
        fs::write(&path, data).map_err(|e| format!("缓存写入失败: {e}"))?;
        Ok(path.to_string_lossy().to_string())
    }

    /// 清理上一版本（v1：`edge` + `.mp3`）遗留的 `.mp3` 缓存项。
    /// 当前格式（v2：`edge-wav-v2` + `.wav`）的键命名空间与 v2 runes 不兼容，
    /// 旧 `.mp3` 永不会被命中，留着只是占盘。仅扫一次目录、忽略错误。
    /// 返回清理掉的文件数。仅当目录里同时存在 `.mp3` 与 `.wav` 时 `.wav` 必须保留。
    pub fn purge_legacy_mp3(&self) -> usize {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        let mut removed = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("mp3") {
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }

    /// 盘点 `.wav` 缓存项的数量与磁盘占用，供 `tts_cache_stats` 上报。
    pub fn inventory(&self) -> (usize, u64) {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return (0, 0),
        };
        let mut count = 0usize;
        let mut bytes = 0u64;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some(AUDIO_EXTENSION) {
                count += 1;
                if let Ok(meta) = entry.metadata() {
                    bytes += meta.len();
                }
            }
        }
        (count, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_text_collapses_whitespace_variants() {
        let a = canonical_text_for_key("你好");
        let b = canonical_text_for_key("  你好  ");
        let c = canonical_text_for_key("你好\n");
        let d = canonical_text_for_key("你\t好");
        assert_eq!(a, "你好");
        assert_eq!(a, b, "首尾空白应被消除");
        assert_eq!(a, c, "尾部换行应被消除");
        assert_eq!(d, "你 好", "中段空白压缩为单空格");
        // 多词句子也应当稳定
        let s1 = canonical_text_for_key("你好 世界  今天");
        let s2 = canonical_text_for_key("你好\n世界\t今天\n");
        assert_eq!(s1, "你好 世界 今天");
        assert_eq!(s1, s2, "不同空白排版的同义句应得到同一缓存键文本");
    }

    #[test]
    fn purge_legacy_mp3_only_removes_mp3_files() {
        let tmp = std::env::temp_dir().join(format!(
            "ai-desktop-pet-tts-cache-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let mp3_a = tmp.join("aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111.mp3");
        let mp3_b = tmp.join("bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222.mp3");
        let wav_c = tmp.join("cccc3333cccc3333cccc3333cccc3333cccc3333.wav");
        fs::write(&mp3_a, b"fake mp3").unwrap();
        fs::write(&mp3_b, b"fake mp3").unwrap();
        fs::write(&wav_c, b"fake wav").unwrap();

        let cache = AudioCache::new(tmp.clone());
        let removed = cache.purge_legacy_mp3();
        assert_eq!(removed, 2, "应仅清理 2 个 .mp3 文件");
        assert!(!mp3_a.exists(), "mp3_a 应被删");
        assert!(!mp3_b.exists(), "mp3_b 应被删");
        assert!(wav_c.exists(), ".wav 缓存项必须保留");

        let (count, bytes) = cache.inventory();
        assert_eq!(count, 1, "inventory 应只统计 .wav");
        assert_eq!(bytes, b"fake wav".len() as u64);

        fs::remove_dir_all(&tmp).ok();
    }
}
