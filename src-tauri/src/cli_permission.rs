//! 本地 CLI 后端的权限档位。
//!
//! 三个 CLI 的权限词汇各不相同：Claude Code 用 `--permission-mode`，Codex 用
//! `--sandbox`，DeepSeek Harness 用 `DSH_PERMISSION_MODE` 环境变量。桌宠设置页把
//! 用户选择的档位保存在 SQLite 里，启动子进程时由这里归一化成各自的参数。
//!
//! 空值表示“跟随本机配置”：Claude Code 不传 `--permission-mode`；DSH 不覆写
//! `DSH_PERMISSION_MODE`。Codex 的 `exec` 模式没有审批通道，因此沙箱档位始终显式
//! 传入，默认仍是 `workspace-write`。

use tauri::Manager;

/// Claude Code 权限模式（`claude --permission-mode` 的可选值）。
pub const CLAUDE_PERMISSION_MODE_KEY: &str = "claude_permission_mode";
/// Codex 沙箱档位（`codex exec --sandbox` 的可选值）。
pub const CODEX_SANDBOX_MODE_KEY: &str = "codex_sandbox_mode";
/// DeepSeek Harness 权限档位（映射到 `DSH_PERMISSION_MODE`）。
pub const DSH_PERMISSION_MODE_KEY: &str = "dsh_permission_mode";

const CLAUDE_MODES: [&str; 6] = [
    "auto",
    "manual",
    "acceptEdits",
    "plan",
    "dontAsk",
    "bypassPermissions",
];

/// 归一化 Claude Code 的权限模式；空值 / 未知值返回 `None`（不传参数，跟随本机配置）。
pub fn claude_permission_mode(value: &str) -> Option<&'static str> {
    let value = value.trim();
    CLAUDE_MODES.iter().copied().find(|mode| *mode == value)
}

/// 归一化 Codex 的沙箱档位；空值 / 未知值回落到 `workspace-write`。
pub fn codex_sandbox_mode(value: &str) -> &'static str {
    match value.trim() {
        "read-only" => "read-only",
        "danger-full-access" => "danger-full-access",
        _ => "workspace-write",
    }
}

/// DSH 的桌宠档位：要覆写的沙箱模式（`None` = 跟随本机环境），以及是否由桌宠
/// 自动批准升权请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DshPermission {
    pub env_mode: Option<&'static str>,
    pub auto_approve: bool,
}

impl Default for DshPermission {
    fn default() -> Self {
        Self {
            env_mode: None,
            auto_approve: false,
        }
    }
}

/// DSH 档位名 → 桌宠行为。
///
/// `workspace-write-auto` 保留 DSH 的 workspace-write 沙箱，只把升权请求改成
/// 自动允许：模型仍然受沙箱约束，但不会再为每次越界弹确认窗口。
pub fn dsh_permission(value: &str) -> DshPermission {
    match value.trim() {
        "read-only" => DshPermission {
            env_mode: Some("read-only"),
            auto_approve: false,
        },
        "workspace-write" => DshPermission {
            env_mode: Some("workspace-write"),
            auto_approve: false,
        },
        "danger-full-access" => DshPermission {
            env_mode: Some("danger-full-access"),
            auto_approve: false,
        },
        "workspace-write-auto" => DshPermission {
            env_mode: Some("workspace-write"),
            auto_approve: true,
        },
        _ => DshPermission::default(),
    }
}

/// DSH 是否启用桌宠自动批准（只有显式的 `workspace-write-auto` 才算）。
pub fn dsh_auto_approve(value: &str) -> bool {
    value.trim() == "workspace-write-auto"
}

/// 会话级取值优先，其次设置页的全局默认。空串统一表示「跟随本机 CLI 配置」。
pub fn effective_value(session_value: &str, global_value: &str) -> String {
    let session = session_value.trim();
    if session.is_empty() {
        global_value.trim().to_string()
    } else {
        session.to_string()
    }
}

/// 本回合给 Claude Code 的 `--permission-mode` 取值（空串 = 不传参数）。
pub fn claude_effective(session_value: &str, global_value: &str) -> Option<&'static str> {
    claude_permission_mode(&effective_value(session_value, global_value))
}

/// 本回合给 Codex 的 `--sandbox` 取值（总是具体的沙箱档位）。
pub fn codex_effective(session_value: &str, global_value: &str) -> &'static str {
    codex_sandbox_mode(&effective_value(session_value, global_value))
}

/// 本回合给 DSH 的权限设置（沙箱环境变量 + 升权自动批准）。
pub fn dsh_effective(
    session_value: &str,
    global_value: &str,
) -> DshPermission {
    dsh_permission(&effective_value(session_value, global_value))
}

/// 从桌宠设置库读取一个权限档位；读取失败按空值处理。
pub async fn read_setting(app: &tauri::AppHandle, key: &str) -> String {
    let state = app.state::<crate::AppState>();
    let db = state.db.lock().await;
    db.get_setting(key).ok().flatten().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_permission_wins_over_the_global_default() {
        assert_eq!(effective_value("", "auto"), "auto");
        assert_eq!(effective_value("  ", "manual"), "manual");
        assert_eq!(effective_value("dnontAsk", "auto"), "dnontAsk");
        assert_eq!(effective_value("", ""), "");

        assert_eq!(claude_effective("manual", "auto"), Some("manual"));
        assert_eq!(claude_effective("", "bypassPermissions"), Some("bypassPermissions"));
        assert_eq!(claude_effective("", ""), None, "两层都空 = 跟随本机");

        assert_eq!(codex_effective("danger-full-access", "read-only"), "danger-full-access");
        assert_eq!(codex_effective("", ""), "workspace-write", "默认工作区可写");

        assert_eq!(
            dsh_effective("", "workspace-write-auto"),
            DshPermission {
                env_mode: Some("workspace-write"),
                auto_approve: true
            }
        );
        assert!(!dsh_effective("workspace-write", "workspace-write-auto").auto_approve,
            "会话显式档位优先于全局自动批准"
        );
    }

    #[test]
    fn claude_modes_are_normalized_and_unknown_values_follow_the_cli() {
        assert_eq!(claude_permission_mode("auto"), Some("auto"));
        assert_eq!(
            claude_permission_mode("  bypassPermissions \n"),
            Some("bypassPermissions")
        );
        for unknown in ["", "  ", "default", "AUTO", "danger-full-access"] {
            assert_eq!(claude_permission_mode(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn codex_sandbox_defaults_to_workspace_write() {
        assert_eq!(codex_sandbox_mode("read-only"), "read-only");
        assert_eq!(codex_sandbox_mode("danger-full-access"), "danger-full-access");
        assert_eq!(codex_sandbox_mode("workspace-write"), "workspace-write");
        for unknown in ["", "  ", "full-access", "DANGER-FULL-ACCESS"] {
            assert_eq!(codex_sandbox_mode(unknown), "workspace-write", "{unknown:?}");
        }
    }

    #[test]
    fn dsh_modes_map_to_sandbox_and_auto_approval() {
        assert_eq!(
            dsh_permission(""),
            DshPermission {
                env_mode: None,
                auto_approve: false
            }
        );
        assert_eq!(
            dsh_permission("workspace-write-auto"),
            DshPermission {
                env_mode: Some("workspace-write"),
                auto_approve: true
            }
        );
        assert_eq!(
            dsh_permission("danger-full-access"),
            DshPermission {
                env_mode: Some("danger-full-access"),
                auto_approve: false
            }
        );
        assert_eq!(
            dsh_permission("read-only").env_mode,
            Some("read-only")
        );
        assert!(!dsh_auto_approve("workspace-write"));
        assert!(dsh_auto_approve("workspace-write-auto"));
    }
}