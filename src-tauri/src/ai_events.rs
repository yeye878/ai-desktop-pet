//! 带 session_id 的 AI 事件广播。
//!
//! 多会话 / 三个 agent 窗口之后，前端靠事件 payload 里的 session_id 决定写到哪个
//! 窗口。所有 ai-* 事件都应该经过 `crate::emit_ai`。direct_api 与 collaboration
//! 模块内部原本直接调用 `app.emit(...)`，为了不再逐个改事件名，这里提供一个同名
//! 扩展 trait：模块里 `use crate::ai_events::AiEmit;` 并去掉 `use tauri::Emitter`
//! 之后，所有 `.emit(...)` 都会自动注入 session_id。

use crate::emit_ai;
use serde::Serialize;

/// 与 `tauri::Emitter` 同名，但会自动注入当前回合的 session_id。
/// 不要在同一个模块里同时使用 `tauri::Emitter`，否则方法解析会歧义。
pub trait AiEmit {
    fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), tauri::Error>;
}

impl AiEmit for tauri::AppHandle {
    fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), tauri::Error> {
        emit_ai(self, event, payload);
        Ok(())
    }
}
