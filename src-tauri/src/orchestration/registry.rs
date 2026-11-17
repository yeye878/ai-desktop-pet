//! 调用级隔离（第 10.2 节）：按 invocation 保存 PID 集合、取消令牌、事件序号。
//! 保留 session → run → invocation 索引；晚到事件按 invocation 关闭状态拒收。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct InvocationEntry {
    pub invocation_id: String,
    pub session_id: String,
    pub run_id: String,
    pub node_id: String,
    pub cancel: CancellationToken,
    pub pids: Mutex<Vec<u32>>,
    pub event_seq: AtomicU64,
    closed: Mutex<bool>,
}

impl InvocationEntry {
    pub fn add_pid(&self, pid: u32) {
        if let Ok(mut pids) = self.pids.lock() {
            pids.push(pid);
        }
    }

    pub fn next_seq(&self) -> u64 {
        self.event_seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// 关闭后晚到事件一律拒收，不得影响后续新运行。
    pub fn is_open(&self) -> bool {
        self.closed.lock().map(|closed| !*closed).unwrap_or(false)
    }

    fn close(&self) {
        if let Ok(mut closed) = self.closed.lock() {
            *closed = true;
        }
    }
}

#[derive(Default)]
pub struct InvocationRegistry {
    entries: Mutex<HashMap<String, std::sync::Arc<InvocationEntry>>>,
}

impl InvocationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        invocation_id: &str,
        session_id: &str,
        run_id: &str,
        node_id: &str,
    ) -> std::sync::Arc<InvocationEntry> {
        let entry = std::sync::Arc::new(InvocationEntry {
            invocation_id: invocation_id.to_string(),
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            node_id: node_id.to_string(),
            cancel: CancellationToken::new(),
            pids: Mutex::new(Vec::new()),
            event_seq: AtomicU64::new(0),
            closed: Mutex::new(false),
        });
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(invocation_id.to_string(), entry.clone());
        }
        entry
    }

    pub fn get(&self, invocation_id: &str) -> Option<std::sync::Arc<InvocationEntry>> {
        self.entries.lock().ok()?.get(invocation_id).cloned()
    }

    /// 停止单个节点调用：返回需要整树回收的 PID，调用方负责 kill。
    pub fn cancel_invocation(&self, invocation_id: &str) -> Vec<u32> {
        let Some(entry) = self.get(invocation_id) else { return Vec::new() };
        entry.close();
        entry.cancel.cancel();
        entry.pids.lock().map(|pids| pids.clone()).unwrap_or_default()
    }

    /// 停止整项任务的所有进行中调用。
    pub fn cancel_run(&self, run_id: &str) -> Vec<u32> {
        let entries: Vec<_> = self
            .entries
            .lock()
            .map(|entries| entries.values().cloned().collect())
            .unwrap_or_default();
        let mut pids = Vec::new();
        for entry in entries {
            if entry.run_id == run_id {
                entry.close();
                entry.cancel.cancel();
                if let Ok(list) = entry.pids.lock() {
                    pids.extend(list.iter().copied());
                }
            }
        }
        pids
    }

    pub fn finish(&self, invocation_id: &str) {
        if let Some(entry) = self.get(invocation_id) {
            entry.close();
        }
    }

    pub fn open_invocations(&self, run_id: &str) -> Vec<String> {
        self.entries
            .lock()
            .map(|entries| {
                entries
                    .values()
                    .filter(|entry| entry.run_id == run_id && entry.is_open())
                    .map(|entry| entry.invocation_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_run_isolates_other_runs_and_closes_late_events() {
        let registry = InvocationRegistry::new();
        let a = registry.register("inv-a", "s1", "run-1", "work-1");
        let b = registry.register("inv-b", "s1", "run-2", "work-1");
        a.add_pid(111);
        b.add_pid(222);
        let pids = registry.cancel_run("run-1");
        assert_eq!(pids, vec![111]);
        assert!(a.cancel.is_cancelled());
        assert!(!b.cancel.is_cancelled());
        assert!(!a.is_open(), "取消后晚到事件必须拒收");
        assert!(b.is_open());
        assert_eq!(registry.open_invocations("run-1"), Vec::<String>::new());
        assert_eq!(registry.open_invocations("run-2"), vec!["inv-b".to_string()]);
    }

    #[test]
    fn event_seq_is_per_invocation() {
        let registry = InvocationRegistry::new();
        let a = registry.register("inv-a", "s", "r", "n");
        let b = registry.register("inv-b", "s", "r", "n2");
        assert_eq!(a.next_seq(), 1);
        assert_eq!(a.next_seq(), 2);
        assert_eq!(b.next_seq(), 1);
    }

    #[test]
    fn user_stop_then_new_task_not_polluted() {
        let registry = InvocationRegistry::new();
        let old = registry.register("inv-old", "s", "run-old", "n");
        registry.cancel_invocation("inv-old");
        let new = registry.register("inv-new", "s", "run-new", "n");
        assert!(!old.is_open());
        assert!(new.is_open());
        assert!(!new.cancel.is_cancelled());
    }
}