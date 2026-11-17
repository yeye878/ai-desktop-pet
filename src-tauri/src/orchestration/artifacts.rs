//! 产物快照、版本、哈希与证据（第 8/11 节）：不可变版本，文件区按 task/run 分目录，
//! 数据库存索引与哈希；哈希验证身份，快照可重读。

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::contracts::{ArtifactItem, ArtifactManifest};
use super::persistence::{new_id, unix_now, Store};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn run_dir(&self, task_id: &str, run_id: &str) -> PathBuf {
        self.root.join(task_id).join(run_id)
    }

    /// 发布一个文本产物版本（MVP 交付物以文本/文件快照为主）。
    pub fn publish_text(
        &self,
        store: &Store,
        task_id: &str,
        run_id: &str,
        task_revision: u32,
        artifact_set_id: &str,
        revision: u32,
        produced_by: &[String],
        items: &[(String, String)], // (id, content)
        known_limitations: &[String],
    ) -> rusqlite::Result<ArtifactManifest> {
        let dir = self.run_dir(task_id, run_id);
        std::fs::create_dir_all(&dir).map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
                Some(format!("产物目录创建失败：{e}")),
            )
        })?;
        let mut manifest_items = Vec::new();
        let mut hash_input = String::new();
        for (id, content) in items {
            let file_name = format!("v{revision}-{}-{id}.txt", artifact_set_id);
            let path = dir.join(&file_name);
            std::fs::write(&path, content).map_err(|e| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
                    Some(format!("产物写入失败：{e}")),
                )
            })?;
            let hash = sha256_hex(content.as_bytes());
            hash_input.push_str(&hash);
            manifest_items.push(ArtifactItem {
                id: id.clone(),
                uri: path.to_string_lossy().to_string(),
                media_type: "text/plain".into(),
                sha256: hash,
            });
        }
        let manifest = ArtifactManifest {
            artifact_set_id: artifact_set_id.to_string(),
            task_revision,
            revision,
            content_hash: sha256_hex(hash_input.as_bytes()),
            produced_by: produced_by.to_vec(),
            items: manifest_items,
            evidence_refs: vec![],
            known_limitations: known_limitations.to_vec(),
        };
        store.insert_artifact(run_id, &serde_json::to_value(&manifest).unwrap())?;
        Ok(manifest)
    }

    /// 审核快照重读：返回 (id, content)。文件缺失 ⇒ 审核环境受阻，不默认通过。
    pub fn read_snapshot(&self, manifest: &ArtifactManifest) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for item in &manifest.items {
            let content = std::fs::read_to_string(&item.uri)
                .map_err(|e| format!("产物 {} 不可读：{e}", item.id))?;
            if sha256_hex(content.as_bytes()) != item.sha256 {
                return Err(format!("产物 {} 哈希不匹配，快照被改动", item.id));
            }
            out.push((item.id.clone(), content));
        }
        Ok(out)
    }

    pub fn manifest_path_hint(&self, task_id: &str, run_id: &str) -> PathBuf {
        self.run_dir(task_id, run_id)
    }
}

pub fn new_artifact_set_id() -> String {
    new_id("art")
}

pub fn now() -> i64 {
    unix_now()
}

pub fn root_under(app_data: &Path) -> PathBuf {
    app_data.join("orchestration")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_detects_tampering_and_snapshot_rereads() {
        let dir = std::env::temp_dir().join(format!("orch-art-{}", uuid::Uuid::new_v4().simple()));
        let store_dir = dir.join("db");
        std::fs::create_dir_all(&store_dir).unwrap();
        let db = Store::new(store_dir.join("pet.db"));
        super::super::persistence::ensure_schema(db.path()).unwrap();
        let artifacts = ArtifactStore::new(dir.join("files"));
        let manifest = artifacts
            .publish_text(&db, "task", "run", 1, "set", 1, &["inv-1".into()], &[("reply".into(), "你好".into())], &[])
            .unwrap();
        let snapshot = artifacts.read_snapshot(&manifest).unwrap();
        assert_eq!(snapshot[0].1, "你好");
        // 篡改文件后哈希校验失败。
        std::fs::write(&manifest.items[0].uri, "篡改").unwrap();
        assert!(artifacts.read_snapshot(&manifest).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}