//! 本机 DeepSeek Harness（dsh）CLI 接入。
//!
//! dsh 启动的是一个 profile：由若干 bundle 补丁层按顺序叠加而成的插件树。桌宠
//! 拥有 $DSH_HOME/profiles/dsh-pet，它挂载随包发布的 dsh-base 与 dsh-headless，
//! 再用自带的 plugins/pet-runner.mjs 替换随包的 headless 应用，从而拿到与
//! Codex/Claude Code 同等的效果：
//!
//!   * 推理增量与正文增量实时流式返回（官方 headless 只打印最后一条文本）；
//!   * 工具调用与工具结果作为事件上报，进入桌宠的工具轨迹面板；
//!   * 支持 --resume 续接同一会话，对话连续性不再依赖每轮重放历史；
//!   * 任务文本走提示词文件，不再受 Windows 命令行长度限制。
//!
//! runner 是桌宠 profile 的一部分，登录信息、模型/provider、权限预设，以及机器级
//! 的 $DSH_HOME/cordis.patch.yml 仍然来自用户的 DSH 配置，没有任何凭据被复制到
//! 桌宠里。

use crate::cli_activity::ToolEvent;
use crate::direct_api::ToolConfirmPayload;
use crate::interactions::{PendingRequest, RequestGuard, RequestKind, WaitResult};
use crate::openclaw::StreamParseState;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::{Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// @deepseek-ai/dsh-home-paths 读取的 DSH 主目录覆盖变量。
const DSH_HOME_ENV: &str = "DSH_HOME";
/// $DSH_HOME/profiles 下桌宠专属的 profile 名。
pub const PROFILE_NAME: &str = "dsh-pet";
/// 桌宠 profile 挂载的随包 bundle：基础层 + 一次性任务应用。
const PROFILE_BUNDLES: [&str; 2] = ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-headless"];
/// 首轮重放会话记录时的提示词预算（UTF-16 单元）。任务已经走提示词文件，
/// 这里限制的是模型上下文，而不是命令行长度。
const MAX_TASK_UNITS: usize = 24_000;
/// 超长任务从中间截断时保留的头部长度：角色规则在最前面。
const HEAD_UNITS: usize = 6_000;
const TRUNCATION_NOTE: &str = "\n\n…（此处省略了较早的中间内容）…\n\n";
/// 桌宠自带的 runner 插件源码（编译期嵌入，运行期复制进 profile）。
const RUNNER_SOURCE: &str = include_str!("../dsh/pet-runner.mjs");
/// profile 补丁层的归属标记：只覆盖自己写过的版本，用户改动会被保留。
const PATCH_MARKER: &str = "pet-runner-patch";
const PATCH_FILENAME: &str = "cordis.patch.yml";
/// 提示词文件目录（在 DSH 沙箱根之外，模型不会把它当成工作区文件）。
const PROMPT_DIR: &str = "dsh-prompts";
/// 超过这个时间的提示词文件视为残留，启动新一轮时顺手清理。
const PROMPT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 启动方式：优先直接调用 Node 入口，退回 npm 的 .cmd 包装脚本。
///
/// 任务文本已经走提示词文件，两种方式都能正确处理参数；直接调用 Node 可以省掉
/// cmd.exe 一层包装，PATH/编码行为也更可预期。
enum Launcher {
    Node { node: PathBuf, entry: PathBuf },
    Shim(PathBuf),
}

impl Launcher {
    fn display_path(&self) -> &Path {
        match self {
            Launcher::Node { entry, .. } => entry,
            Launcher::Shim(path) => path,
        }
    }
}

fn is_absolute(path: &Path) -> bool {
    path.is_absolute()
}

fn find_cli() -> Result<PathBuf, String> {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    // GUI 应用可能继承到 npm 安装之前就存在的 PATH。
    if let Some(app_data) = dirs::data_dir() {
        directories.push(app_data.join("npm"));
    }
    if let Some(home) = dirs::home_dir() {
        directories.push(home.join(".local").join("bin"));
        directories.push(home.join(".cargo").join("bin"));
    }
    #[cfg(target_os = "windows")]
    let names = ["dsh.cmd", "dsh.exe", "dsh.bat"];
    #[cfg(not(target_os = "windows"))]
    let names = ["dsh"];

    for directory in directories.iter().filter(|path| is_absolute(path)) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("未找到本机 DeepSeek Harness CLI。请先运行 npm install -g @deepseek-ai/dsh，再刷新状态。".into())
}

/// 在包装脚本同级目录、PATH、常见安装位置里查找 node.exe。
fn find_node(shim_dir: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local = shim_dir.join("node.exe");
        if local.is_file() {
            return Some(local);
        }
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path).filter(|path| is_absolute(path)) {
                let candidate = directory.join("node.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let mut guesses = Vec::new();
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            guesses.push(PathBuf::from(program_files).join("nodejs").join("node.exe"));
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            guesses.push(
                PathBuf::from(local_app_data)
                    .join("Programs")
                    .join("nodejs")
                    .join("node.exe"),
            );
        }
        guesses.into_iter().find(|candidate| candidate.is_file())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = shim_dir;
        None
    }
}

/// 解析启动方式。npm 全局安装会把包放在包装脚本旁边的 node_modules 下，
/// 找到 lib/bin.js 就直启 Node。
fn resolve_launcher() -> Result<Launcher, String> {
    let shim = find_cli()?;
    if let Some(directory) = shim.parent() {
        let entry = directory
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js");
        if entry.is_file() {
            if let Some(node) = find_node(directory) {
                return Ok(Launcher::Node { node, entry });
            }
        }
    }
    Ok(Launcher::Shim(shim))
}

/// $DSH_HOME（非空时）优先，否则 ~/.dsh，与 resolveDshHome 保持一致。
pub fn resolve_home() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(DSH_HOME_ENV) {
        let text = value.to_string_lossy().trim().to_string();
        if !text.is_empty() {
            return Ok(PathBuf::from(text));
        }
    }
    dirs::home_dir()
        .map(|home| home.join(".dsh"))
        .ok_or_else(|| "无法定位 DeepSeek Harness 主目录，请设置 DSH_HOME。".to_string())
}

pub fn workspace_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("dsh-workspace"))
        .map_err(|error| error.to_string())
}

fn prompt_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join(PROMPT_DIR))
        .map_err(|error| error.to_string())
}

/// 桌宠 profile 的补丁层：关掉随包的一次性应用，挂上自带 runner。
fn patch_layer() -> String {
    format!(
        "# {PATCH_MARKER} v2 — 由 ai-desktop-pet 生成并维护，覆盖旧版本时请保留本标记行。\n\
         # 桌宠用自带 runner 替换随包的 headless 应用：流式输出推理/正文/工具事件，并支持会话续接。\n\
         - id: headless-startup\n\
         \x20\x20disabled: true\n\
         - id: headless-runner\n\
         \x20\x20disabled: true\n\
         - insert:\n\
         \x20\x20\x20\x20- id: pet-runner\n\
         \x20\x20\x20\x20\x20\x20name: './plugins/pet-runner.mjs'\n"
    )
}

/// 该补丁层是否由桌宠维护（空文件、空序列、旧版桌宠内容都算）。
fn patch_is_ours(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.is_empty()
        || trimmed == "[]"
        || content.contains(PATCH_MARKER)
        || content.contains("桌宠专用 profile")
}

/// 幂等地准备桌宠 profile：清单、runner 插件与补丁层。
/// 用户自己写进补丁层的内容不会被覆盖，只会在其后追加桌宠需要的条目。
fn ensure_profile_in(home: &Path) -> Result<PathBuf, String> {
    let dir = home.join("profiles").join(PROFILE_NAME);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("无法创建 DSH profile 目录 {}：{error}", dir.display()))?;

    let manifest = dir.join("package.json");
    if !manifest.is_file() {
        let bundles = PROFILE_BUNDLES
            .iter()
            .map(|name| format!("    \"{name}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        let content = format!(
            "{{\n  \"name\": \"dsh-profile-{PROFILE_NAME}\",\n  \"private\": true,\n  \"dependencies\": {{}},\n  \"dsh\": {{ \"profile\": {{ \"bundles\": [\n{bundles}\n  ] }} }}\n}}\n"
        );
        std::fs::write(&manifest, content)
            .map_err(|error| format!("无法写入 {}：{error}", manifest.display()))?;
    }

    // runner 是桌宠的资产，每次启动都同步到当前版本。
    let plugins = dir.join("plugins");
    std::fs::create_dir_all(&plugins)
        .map_err(|error| format!("无法创建 {}：{error}", plugins.display()))?;
    let runner = plugins.join("pet-runner.mjs");
    if std::fs::read_to_string(&runner).ok().as_deref() != Some(RUNNER_SOURCE) {
        std::fs::write(&runner, RUNNER_SOURCE)
            .map_err(|error| format!("无法写入 {}：{error}", runner.display()))?;
    }

    let patch = dir.join(PATCH_FILENAME);
    let existing = std::fs::read_to_string(&patch).unwrap_or_default();
    if patch_is_ours(&existing) {
        if existing != patch_layer() {
            std::fs::write(&patch, patch_layer())
                .map_err(|error| format!("无法写入 {}：{error}", patch.display()))?;
        }
    } else if !existing.contains(PATCH_MARKER) {
        // 用户在补丁层里写过自己的条目：保留它们，只追加桌宠需要的部分。
        let mut merged = existing;
        if !merged.ends_with('\n') {
            merged.push('\n');
        }
        merged.push_str(&patch_layer());
        std::fs::write(&patch, merged)
            .map_err(|error| format!("无法写入 {}：{error}", patch.display()))?;
    }
    Ok(dir)
}

pub fn ensure_profile() -> Result<PathBuf, String> {
    ensure_profile_in(&resolve_home()?)
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

fn take_units(text: &str, limit: usize) -> &str {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        let next = used + character.len_utf16();
        if next > limit {
            return &text[..index];
        }
        used = next;
    }
    text
}

fn take_last_units(text: &str, limit: usize) -> &str {
    let mut used = 0;
    let mut start = text.len();
    for (index, character) in text.char_indices().rev() {
        let next = used + character.len_utf16();
        if next > limit {
            break;
        }
        used = next;
        start = index;
    }
    &text[start..]
}

/// 超出命令行预算时保留头部（角色规则）与尾部（当前任务），从中间截断并留下标记。
fn fit_task(task: &str) -> String {
    if units(task) <= MAX_TASK_UNITS {
        return task.to_string();
    }
    let note_units = units(TRUNCATION_NOTE);
    let head = take_units(task, HEAD_UNITS);
    let tail = take_last_units(task, MAX_TASK_UNITS.saturating_sub(HEAD_UNITS + note_units));
    format!("{head}{TRUNCATION_NOTE}{tail}")
}

fn history_line(message: &crate::storage::ChatMessage) -> String {
    let speaker = message
        .agent_name
        .clone()
        .unwrap_or_else(|| if message.role == "user" { "用户".into() } else { "桌宠".into() });
    let mut line = format!("{speaker}: {}", message.content.trim());
    if let (Some(role), Some(content)) = (&message.quoted_role, &message.quoted_content) {
        line.push_str(&format!(
            "\n（引用了{}：{}）",
            if role == "user" { "用户" } else { "桌宠" },
            content.trim()
        ));
    }
    line
}

/// 组装首轮任务文本：角色规则 + 会话记录 + 本轮消息。
/// 只有还没有可续接的会话时才需要带记录，续接后的轮次靠 dsh 自己的会话历史。
pub fn assemble_task(
    system_prompt: &str,
    history: &[crate::storage::ChatMessage],
    message: &str,
) -> String {
    let header = format!("【桌宠角色与规则】\n{}\n\n", system_prompt.trim());
    let footer = format!("【用户消息】\n{}", message.trim());
    let mut entries: Vec<String> = history.iter().map(history_line).collect();
    loop {
        let transcript = if entries.is_empty() {
            String::new()
        } else {
            format!(
                "【当前会话记录（按时间顺序，最后一条最新）】\n{}\n\n",
                entries.join("\n")
            )
        };
        if units(&header) + units(&transcript) + units(&footer) <= MAX_TASK_UNITS || entries.is_empty()
        {
            return fit_task(&format!("{header}{transcript}{footer}"));
        }
        // 一条一条丢，但每次至少丢掉四分之一，避免长会话里逐条重算。
        let drop = (entries.len() / 4).max(1);
        entries.drain(..drop);
    }
}

/// 提示词落盘：内容再长也只是一次文件写入，不受命令行长度限制。
fn write_prompt(dir: &Path, task: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("无法创建 {}：{error}", dir.display()))?;
    sweep_stale_prompts(dir);
    let path = dir.join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, task).map_err(|error| format!("无法写入提示词文件：{error}"))?;
    Ok(path)
}

/// 清理被取消的轮次留下的提示词文件（尽力而为，失败不影响本轮）。
fn sweep_stale_prompts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|value| value != "txt").unwrap_or(true) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > PROMPT_TTL);
        if stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn runner_args(prompt_file: &Path, session: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--profile".into(),
        PROFILE_NAME.into(),
        "--prompt-file".into(),
        prompt_file.to_string_lossy().into_owned(),
    ];
    if let Some(id) = session.filter(|id| !id.trim().is_empty()) {
        args.push("--resume".into());
        args.push(id.to_string());
    }
    args
}

fn launcher_command(launcher: &Launcher, args: &[String]) -> Command {
    match launcher {
        Launcher::Node { node, entry } => {
            let mut command = Command::new(node);
            command.arg(entry);
            command.args(args);
            command
        }
        Launcher::Shim(path) => {
            let mut command = Command::new(path);
            command.args(args);
            command
        }
    }
}

/// 桌宠的 dsh 会话状态，用法与 CodexAdapter 对称：只续接自己创建的会话。
#[derive(Default)]
pub struct DshAdapter {
    session_id: Mutex<Option<String>>,
}

impl DshAdapter {
    pub fn has_session(&self) -> bool {
        self.session_id
            .lock()
            .map(|id| id.is_some())
            .unwrap_or(false)
    }

    pub fn update_session(&self, session_id: &str) {
        if session_id.trim().is_empty() || !session_id.starts_with("session-") {
            return;
        }
        if let Ok(mut current) = self.session_id.lock() {
            *current = Some(session_id.to_string());
        }
    }

    pub fn reset_session(&self) {
        if let Ok(mut current) = self.session_id.lock() {
            *current = None;
        }
    }

    /// 启动一轮 dsh 任务。工作目录就是 DSH 的 cwd 与 workspace-write 沙箱根。
    /// 已有会话时只发本轮消息；首轮才需要把会话记录一起带上。
    pub async fn spawn_streaming(
        &self,
        app: &tauri::AppHandle,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let workspace = workspace_dir(app)?;
        let prompts = prompt_dir(app)?;
        self.spawn_in_workspace(&workspace, &prompts, message, history, system_prompt)
            .await
    }

    /// 工作目录与提示词目录显式传入，便于测试复用同一条真实路径。
    pub async fn spawn_in_workspace(
        &self,
        workspace: &Path,
        prompt_root: &Path,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session_id.lock().ok().and_then(|id| id.clone());
        let task = if session.is_some() {
            assemble_task(system_prompt, &[], message)
        } else {
            // 首轮：没有可续接的会话，先把当前会话记录交给它。
            assemble_task(system_prompt, history, message)
        };
        if task.trim().is_empty() {
            return Err("DeepSeek Harness 需要一段非空任务文本".into());
        }
        ensure_profile()?;
        std::fs::create_dir_all(workspace)?;
        let prompt_file = write_prompt(prompt_root, &task)?;
        let launcher = match resolve_launcher() {
            Ok(launcher) => launcher,
            Err(error) => {
                let _ = std::fs::remove_file(&prompt_file);
                return Err(error.into());
            }
        };
        let mut command = launcher_command(&launcher, &runner_args(&prompt_file, session.as_deref()));
        command
            .current_dir(workspace)
            // stdin 是授权往返的回程通道：runner 把请求写在 stdout，桌宠把决定写回来。
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(target_os = "windows")]
        command.creation_flags(CREATE_NO_WINDOW);
        match command.spawn() {
            Ok(child) => Ok(child),
            Err(error) => {
                let _ = std::fs::remove_file(&prompt_file);
                Err(error.into())
            }
        }
    }
}

/// 一次需要用户确认的授权请求（来自 runner 的 approval-request 行）。
#[derive(Clone, Debug, PartialEq)]
pub struct DshApprovalRequest {
    /// runner 侧的请求 id：桌宠的决定必须带着它回传。
    pub id: String,
    pub tool: String,
    pub reason: Option<String>,
    /// provider 的调用 id：用来和已经流过来的 tool/call 事件对上，补全参数。
    pub call_id: Option<String>,
}

/// seam 结果词表里桌宠会用到的三种（unavailable 由应答者缺失触发）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DshDecision {
    Allow,
    Deny,
    Cancel,
}

impl DshDecision {
    fn as_str(self) -> &'static str {
        match self {
            DshDecision::Allow => "allow",
            DshDecision::Deny => "deny",
            DshDecision::Cancel => "cancel",
        }
    }
}

/// 回给 runner 的一行决定。
fn approval_decision_line(id: &str, decision: DshDecision) -> String {
    let id = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "{{\"type\":\"approval-decision\",\"id\":{id},\"decision\":\"{}\"}}\n",
        decision.as_str()
    )
}

/// 桌宠如何回答一次授权请求。生产路径是 UiApproval（弹窗 + 用户点击）；
/// 测试与无 UI 场景各自实现，或直接走拒绝（fail closed，与 DSH 自身一致）。
#[async_trait]
pub trait ApprovalAnswerer: Send + Sync {
    async fn decide(&self, request: &DshApprovalRequest, payload: ToolConfirmPayload) -> DshDecision;
}

/// 复用直连 API 的确认流程：pending_confirms + ai-tool-confirm 事件
/// （ToolConfirmPanel 窗口）→ 用户点击 → confirm_tool 唤醒等待。
pub struct UiApproval {
    app: tauri::AppHandle,
    cancel: CancellationToken,
    timeout: Duration,
}

impl UiApproval {
    pub fn new(app: tauri::AppHandle, cancel: CancellationToken) -> Self {
        Self {
            app,
            cancel,
            timeout: Duration::from_secs(120),
        }
    }

    /// 思考流里也留一行：弹窗被其他窗口挡住时，用户仍能看到正在等授权。
    fn note_waiting(&self, summary: &str) {
        let message = format!("[等待授权] {summary}\n");
        let state = self.app.state::<crate::AppState>();
        if let Ok(mut active) = state.active_chat.lock() {
            if let Some(active) = active.active.as_mut() {
                active.thinking.push_str(&message);
            }
        }
        let _ = self.app.emit("ai-thinking", &message);
    }
}

#[async_trait]
impl ApprovalAnswerer for UiApproval {
    async fn decide(&self, _request: &DshApprovalRequest, payload: ToolConfirmPayload) -> DshDecision {
        let confirm_id = payload.id.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel::<bool>();
        {
            let state = self.app.state::<crate::AppState>();
            let mut confirms = state.pending_confirms.lock().await;
            confirms.insert(
                confirm_id.clone(),
                PendingRequest {
                    sender,
                    payload: payload.clone(),
                },
            );
        }
        let mut guard = RequestGuard::new(&self.app, &confirm_id, RequestKind::Tool);
        self.note_waiting(&payload.summary);
        let _ = self.app.emit("ai-tool-confirm", payload);

        let outcome =
            crate::interactions::wait_for_response(receiver, &self.cancel, self.timeout).await;
        {
            let state = self.app.state::<crate::AppState>();
            let mut confirms = state.pending_confirms.lock().await;
            confirms.remove(&confirm_id);
        }
        guard.settle();

        let (approved, decision) = match outcome {
            WaitResult::Answered(true) => (true, DshDecision::Allow),
            WaitResult::Answered(false) => (false, DshDecision::Deny),
            WaitResult::Cancelled => (false, DshDecision::Cancel),
            // 超时与窗口关闭都按拒绝处理：失败关闭，不给未确认的操作开口子。
            WaitResult::TimedOut | WaitResult::Closed => (false, DshDecision::Deny),
        };
        let _ = self.app.emit(
            "ai-tool-confirm-resolved",
            serde_json::json!({ "id": confirm_id, "approved": approved }),
        );
        decision
    }
}

/// 工具事件在 start/call/result 三个阶段之间合并的暂存区。
#[derive(Default)]
struct RunnerParser {
    streamed_text: String,
    final_text: String,
    tools: HashMap<String, ToolEvent>,
    done_reason: Option<String>,
    error: Option<String>,
    last_log: Option<String>,
}

impl RunnerParser {
    fn text_of(&self) -> &str {
        if self.final_text.trim().is_empty() {
            self.streamed_text.as_str()
        } else {
            self.final_text.as_str()
        }
    }

    fn merge_tool(&mut self, id: &str, update: impl FnOnce(&mut ToolEvent)) -> Option<ToolEvent> {
        if id.is_empty() {
            return None;
        }
        let entry = self
            .tools
            .entry(id.to_string())
            .or_insert_with(|| ToolEvent {
                id: id.to_string(),
                tool_name: String::new(),
                status: "running".into(),
                summary: String::new(),
                arguments: String::new(),
                command: None,
                path: None,
                output: None,
                approved: None,
            });
        update(entry);
        // 摘要随阶段推进更新：start 只有工具名，call 之后能给出命令或路径。
        entry.summary = entry
            .command
            .clone()
            .or_else(|| entry.path.clone())
            .unwrap_or_else(|| entry.tool_name.clone());
        Some(entry.clone())
    }

    /// 解析一行 runner 协议，返回本行涉及的思考/正文/工具回调。
    /// 授权请求需要异步等待用户，所以交回调用方处理。
    fn parse_line(
        &mut self,
        state: &mut StreamParseState,
        line: &str,
        on_thinking: &mut dyn FnMut(&str),
        on_text: &mut dyn FnMut(&str),
        on_tool: &mut dyn FnMut(&ToolEvent),
    ) -> Option<DshApprovalRequest> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // 非协议行（插件日志、CLI 噪声）留作诊断，不进入回答。
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                self.last_log = Some(trimmed.chars().take(400).collect());
            }
            return None;
        };
        match value["type"].as_str().unwrap_or_default() {
            "session" => {
                if let Some(id) = value["session_id"].as_str() {
                    state.session_id = Some(id.to_string());
                }
            }
            "thinking" => {
                if let Some(text) = value["text"].as_str().filter(|text| !text.is_empty()) {
                    state.full_thinking.push_str(text);
                    state.thinking_len = state.full_thinking.len();
                    on_thinking(text);
                }
            }
            "text" => {
                if let Some(text) = value["text"].as_str().filter(|text| !text.is_empty()) {
                    self.streamed_text.push_str(text);
                    state.text_len = self.streamed_text.len();
                    on_text(text);
                }
            }
            "message" => {
                if let Some(text) = value["text"].as_str().filter(|text| !text.is_empty()) {
                    self.final_text = text.to_string();
                }
            }
            "tool" => {
                let id = value["id"].as_str().unwrap_or_default().to_string();
                let phase = value["phase"].as_str().unwrap_or_default().to_string();
                let name = value["name"].as_str().unwrap_or_default().to_string();
                let arguments = value["arguments"].as_str().unwrap_or_default().to_string();
                let content = value["content"].as_str().unwrap_or_default().to_string();
                let failed = value["is_error"].as_bool().unwrap_or(false);
                let tool = match phase.as_str() {
                    "start" => self.merge_tool(&id, |tool| {
                        if tool.tool_name.is_empty() {
                            tool.tool_name = name.clone();
                        }
                        tool.status = "running".into();
                    }),
                    "call" => self.merge_tool(&id, |tool| {
                        tool.tool_name = name.clone();
                        tool.status = "running".into();
                        let parsed = serde_json::from_str::<Value>(&arguments).ok();
                        if let Some(parsed) = parsed.as_ref() {
                            tool.command = parsed["command"].as_str().map(str::to_string);
                            tool.path = parsed["file_path"]
                                .as_str()
                                .or_else(|| parsed["path"].as_str())
                                .or_else(|| parsed["filename"].as_str())
                                .map(str::to_string);
                            tool.arguments = serde_json::to_string_pretty(parsed)
                                .unwrap_or_else(|_| arguments.clone());
                        } else {
                            tool.arguments = arguments.clone();
                        }
                    }),
                    "result" => self.merge_tool(&id, |tool| {
                        tool.status = if failed { "failed".into() } else { "completed".into() };
                        if !content.is_empty() {
                            tool.output = Some(content.clone());
                        }
                    }),
                    _ => None,
                };
                if let Some(tool) = tool {
                    on_tool(&tool);
                }
            }
            "done" => {
                self.done_reason = value["reason"].as_str().map(str::to_string);
            }
            "error" => {
                state.is_error = true;
                let code = value["code"].as_str().unwrap_or_default();
                let message = value["message"].as_str().unwrap_or("DeepSeek Harness 执行失败");
                state.error_msg = Some(if code.is_empty() || code == "PET_RUNNER_FAILED" {
                    message.to_string()
                } else {
                    format!("{code}: {message}")
                });
            }
            "log" => {
                if let Some(text) = value["text"].as_str() {
                    self.last_log = Some(text.chars().take(400).collect());
                }
            }
            "approval-request" => {
                return Some(DshApprovalRequest {
                    id: value["id"].as_str().unwrap_or_default().to_string(),
                    tool: value["tool"].as_str().unwrap_or_default().to_string(),
                    reason: value["reason"].as_str().map(str::to_string),
                    call_id: value["call_id"].as_str().map(str::to_string),
                });
            }
            _ => {}
        }
        None
    }

    /// 组装给确认弹窗的载荷。授权请求本身不带参数，用 call_id 和已经流过来的
    /// tool/call 事件对上，就能把命令、路径、参数补全，和直连 API 的观感一致。
    fn confirm_payload(&self, request: &DshApprovalRequest) -> ToolConfirmPayload {
        let known = request.call_id.as_ref().and_then(|id| self.tools.get(id));
        let tool_name = if request.tool.trim().is_empty() {
            known.map(|tool| tool.tool_name.clone()).unwrap_or_default()
        } else {
            request.tool.clone()
        };
        let mut summary = known
            .map(|tool| tool.summary.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| tool_name.clone());
        if let Some(reason) = request
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        {
            summary.push_str(&format!("（{}）", reason.chars().take(80).collect::<String>()));
        }
        ToolConfirmPayload {
            id: format!("confirm_{}", uuid::Uuid::new_v4()),
            tool_name,
            arguments: known.map(|tool| tool.arguments.clone()).unwrap_or_default(),
            summary,
            command: known.and_then(|tool| tool.command.clone()),
            path: known.and_then(|tool| tool.path.clone()),
        }
    }

    /// 收尾：确定最终文本，并把「进程失败 / 未完成 / 无输出」统一成错误。
    fn finish(&mut self, state: &mut StreamParseState, success: bool, stderr: &str) {
        state.full_text = self.text_of().trim_end().to_string();
        state.text_len = state.full_text.len();
        if state.is_error {
            return;
        }
        if !success || self.done_reason.as_deref() != Some("completed") {
            state.is_error = true;
            state.error_msg = Some(
                self.error
                    .clone()
                    .or_else(|| parse_stderr_error(stderr))
                    .unwrap_or_else(|| error_message(stderr, success)),
            );
            return;
        }
        if state.full_text.trim().is_empty() {
            state.is_error = true;
            state.error_msg = Some("DeepSeek Harness 本轮没有产生可见回复，请重试或检查模型配置。".into());
        }
    }
}

/// 从 stderr 里取出 dsh: <CODE>: <message>，作为协议错误的兜底诊断。
fn parse_stderr_error(stderr: &str) -> Option<String> {
    for line in stderr.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(index) = line.rfind("dsh: ") {
            let tail = line[index + "dsh: ".len()..].trim();
            if !tail.is_empty() {
                return Some(tail.to_string());
            }
        }
    }
    None
}

fn error_message(stderr: &str, success: bool) -> String {
    if let Some(message) = parse_stderr_error(stderr) {
        return message;
    }
    let trimmed = stderr.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if success {
        "DeepSeek Harness 没有返回内容，请检查本机 dsh 的登录与模型配置。".into()
    } else {
        "DeepSeek Harness 在本轮完成前退出，请检查本机 dsh 的登录与模型配置。".into()
    }
}

/// 读取 runner 的 JSONL 协议：推理增量、正文增量、工具事件、授权请求与会话 ID。
///
/// 授权请求在这里就地等待（DSH 此刻正阻塞在同一个调用上），拿到决定后写回子进程
/// 的 stdin。没有应答者时按拒绝处理，保持 DSH 自身的 fail-closed 语义。
pub async fn read_stream_with_activity<F, T, E>(
    child: &mut Child,
    mut on_thinking: F,
    mut on_text: T,
    mut on_tool: E,
    approvals: Option<Box<dyn ApprovalAnswerer>>,
) -> Result<StreamParseState, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(&str),
    T: FnMut(&str),
    E: FnMut(&ToolEvent),
{
    let stdout = child.stdout.take().ok_or("无法读取 DeepSeek Harness 输出")?;
    let mut stdin = child.stdin.take();
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut output = Vec::new();
            let mut buffer = [0u8; 4096];
            // 保留诊断上限之后仍然继续排空，避免子进程被管道写满卡住。
            while let Ok(count) = stderr.read(&mut buffer).await {
                if count == 0 {
                    break;
                }
                let keep = count.min((16 * 1024usize).saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
            }
            String::from_utf8_lossy(&output).into_owned()
        })
    });
    let mut lines = BufReader::new(stdout).lines();
    let mut state = StreamParseState::new();
    let mut parser = RunnerParser::default();
    let mut read_error = None;
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let request = parser.parse_line(
                    &mut state,
                    &line,
                    &mut on_thinking,
                    &mut on_text,
                    &mut on_tool,
                );
                if let Some(request) = request {
                    let payload = parser.confirm_payload(&request);
                    let decision = match approvals.as_ref() {
                        Some(answerer) => answerer.decide(&request, payload).await,
                        None => {
                            on_thinking(&format!(
                                "[未启用授权确认，已拒绝 {}]\n",
                                payload.tool_name
                            ));
                            DshDecision::Deny
                        }
                    };
                    if let Some(stdin) = stdin.as_mut() {
                        let line = approval_decision_line(&request.id, decision);
                        if stdin.write_all(line.as_bytes()).await.is_ok() {
                            let _ = stdin.flush().await;
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                read_error = Some(error);
                let _ = child.kill().await;
                break;
            }
        }
    }
    let status = child.wait().await?;
    let stderr = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if let Some(error) = read_error {
        return Err(error.into());
    }
    parser.finish(&mut state, status.success(), &stderr);
    if state.is_error {
        // 协议没给错误、stderr 也没有时，把 runner 的最后一条日志带上，便于定位。
        if let (Some(log), Some(message)) = (parser.last_log.as_ref(), state.error_msg.as_mut()) {
            if message.contains("没有返回内容") || message.contains("完成前退出") {
                message.push_str(&format!("（runner 日志：{log}）"));
            }
        }
    }
    Ok(state)
}

#[derive(Serialize)]
pub struct DshStatus {
    installed: bool,
    logged_in: bool,
    version: Option<String>,
    executable: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    profile: String,
    workspace: String,
    message: String,
}

/// 是否已经配置过凭据：DSH 的凭据存储有内容，或环境里直接给了 API Key。
fn credentials_present(home: &Path) -> bool {
    if std::env::var_os("DEEPSEEK_API_KEY").is_some_and(|value| !value.is_empty()) {
        return true;
    }
    // 只判断是否配置过，不读取也不回传任何密钥内容。
    std::fs::read_to_string(home.join(".credentials.yaml"))
        .map(|content| !content.trim().is_empty())
        .unwrap_or(false)
}

/// 读取 settings.yaml 里 agent-default-model 的模型与 provider。
fn read_default_model(home: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(home.join("settings.yaml")) else {
        return (None, None);
    };
    let mut in_section = false;
    let mut model = None;
    let mut provider = None;
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let indented = raw.starts_with(' ') || raw.starts_with('\t');
        if !indented {
            in_section = raw.trim().trim_end_matches(':').trim() == "agent-default-model";
            continue;
        }
        if !in_section {
            continue;
        }
        let entry = raw.trim();
        if let Some(value) = entry.strip_prefix("model:") {
            model = Some(unquote(value));
        } else if let Some(value) = entry.strip_prefix("provider:") {
            provider = Some(unquote(value));
        }
    }
    (model, provider)
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).to_string()
}

pub async fn check_status(app: &tauri::AppHandle) -> Result<DshStatus, String> {
    let home = resolve_home()?;
    let mut status = DshStatus {
        installed: false,
        logged_in: false,
        version: None,
        executable: None,
        model: None,
        provider: None,
        profile: home
            .join("profiles")
            .join(PROFILE_NAME)
            .to_string_lossy()
            .into_owned(),
        workspace: workspace_dir(app)?.to_string_lossy().into_owned(),
        message: String::new(),
    };
    let launcher = match resolve_launcher() {
        Ok(launcher) => launcher,
        Err(message) => {
            status.message = message;
            return Ok(status);
        }
    };
    status.executable = Some(launcher.display_path().to_string_lossy().into_owned());
    // 状态检查不创建 profile：只在真正启动任务时才写盘。
    let mut version = probe_version(&launcher).await;
    if version.is_none() {
        // 直启入口可能因 node 缺失而失败，退回包装脚本再试一次。
        if let Ok(shim) = find_cli() {
            version = probe_shim_version(&shim).await;
        }
    }
    let Some(version) = version else {
        status.message = "找到 DeepSeek Harness，但无法启动；请检查 Node.js 与 dsh 安装后刷新。".into();
        return Ok(status);
    };
    status.installed = true;
    status.version = Some(version);
    let (model, provider) = read_default_model(&home);
    status.model = model;
    status.provider = provider;
    status.logged_in = credentials_present(&home);
    status.message = if status.logged_in {
        "已连接：流式推理与工具轨迹、会话续接均可用，使用本机 dsh 的登录信息与模型配置。".into()
    } else {
        "未检测到 DeepSeek Harness 凭据：运行 dsh web 在 Models 页面配置模型，或为桌宠设置 DEEPSEEK_API_KEY。".into()
    };
    Ok(status)
}

/// 版本探测必须把 --version 交给启动器而不是应用：写在 --profile 之后会被当成
/// 应用的参数。
async fn probe_version(launcher: &Launcher) -> Option<String> {
    let mut command = launcher_command(launcher, &["--version".to_string()]);
    command.stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    match tokio::time::timeout(Duration::from_secs(15), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        _ => None,
    }
}

async fn probe_shim_version(shim: &Path) -> Option<String> {
    let mut command = Command::new(shim);
    command.arg("--version").stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    match tokio::time::timeout(Duration::from_secs(15), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        _ => None,
    }
}

/// 打开 DSH 主目录（settings.yaml / .credentials.yaml 都在这里）。
pub fn open_config() -> Result<(), String> {
    let home = resolve_home()?;
    std::fs::create_dir_all(&home).map_err(|error| format!("无法创建 {}：{error}", home.display()))?;
    #[cfg(target_os = "windows")]
    {
        // explorer 对已存在目录返回非零退出码也无所谓，这里不等它结束。
        std::process::Command::new("explorer.exe")
            .arg(&home)
            .spawn()
            .map_err(|error| format!("无法打开资源管理器：{error}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(format!(
            "请手动打开 {} 查看 DeepSeek Harness 配置。",
            home.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> crate::storage::ChatMessage {
        crate::storage::ChatMessage {
            role: role.into(),
            content: content.into(),
            thinking: None,
            created_at: 0,
            quoted_role: None,
            quoted_content: None,
            agent_id: None,
            agent_name: None,
            agent_avatar: None,
        }
    }

    fn drive(parser: &mut RunnerParser, state: &mut StreamParseState, lines: &[&str]) -> (String, String, Vec<ToolEvent>) {
        let mut thinking = String::new();
        let mut text = String::new();
        let mut tools = Vec::new();
        for line in lines {
            parser.parse_line(
                state,
                line,
                &mut |delta| thinking.push_str(delta),
                &mut |delta| text.push_str(delta),
                &mut |tool| tools.push(tool.clone()),
            );
        }
        (thinking, text, tools)
    }

    #[test]
    fn assembles_role_rules_history_and_current_message() {
        let history = vec![
            message("user", "帮我看看报错"),
            message("assistant", "先看日志"),
        ];
        let task = assemble_task("你是桌宠", &history, "继续");
        assert!(task.starts_with("【桌宠角色与规则】\n你是桌宠"));
        assert!(task.contains("用户: 帮我看看报错"));
        assert!(task.contains("桌宠: 先看日志"));
        assert!(task.trim_end().ends_with("【用户消息】\n继续"));
    }

    #[test]
    fn drops_oldest_history_before_truncating_the_task() {
        let history: Vec<_> = (0..40)
            .map(|index| message("user", &format!("很久以前的消息{index}").repeat(400)))
            .collect();
        let task = assemble_task("规则", &history, "现在的问题");
        assert!(units(&task) <= MAX_TASK_UNITS);
        assert!(task.contains("现在的问题"), "本轮消息必须完整保留");
        assert!(!task.contains("很久以前的消息0"));
        assert!(task.contains("很久以前的消息39"), "最新的记录优先保留");
    }

    #[test]
    fn truncates_oversized_single_message_from_the_middle() {
        let huge = format!("开头{}{}结尾", "字".repeat(20_000), "尾".repeat(20_000));
        let task = assemble_task("规则", &[], &huge);
        assert!(units(&task) <= MAX_TASK_UNITS);
        assert!(task.starts_with("【桌宠角色与规则】\n规则"));
        assert!(task.contains("…（此处省略了较早的中间内容）…"));
        assert!(task.ends_with("结尾"));
    }

    #[test]
    fn streams_thinking_text_and_final_message() {
        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        let (thinking, text, _) = drive(
            &mut parser,
            &mut state,
            &[
                r#"{"type":"session","session_id":"session-abc","resumed":false}"#,
                r#"{"type":"thinking","text":"先看配置"}"#,
                r#"{"type":"text","text":"连接"}"#,
                r#"{"type":"text","text":"正常。"}"#,
                r#"{"type":"message","text":"连接正常。"}"#,
                r#"{"type":"done","reason":"completed"}"#,
                "plugin boot log",
            ],
        );
        assert_eq!(thinking, "先看配置");
        assert_eq!(text, "连接正常。");
        assert_eq!(state.session_id.as_deref(), Some("session-abc"));
        parser.finish(&mut state, true, "");
        assert!(!state.is_error, "{:?}", state.error_msg);
        assert_eq!(state.full_text, "连接正常。");
        assert_eq!(state.full_thinking, "先看配置");
        assert_eq!(parser.last_log.as_deref(), Some("plugin boot log"));
    }

    #[test]
    fn merges_tool_start_call_and_result_into_one_event() {
        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        let (_, _, tools) = drive(
            &mut parser,
            &mut state,
            &[
                r#"{"type":"tool","phase":"start","id":"call-1","name":"pwsh"}"#,
                r#"{"type":"tool","phase":"call","id":"call-1","name":"pwsh","arguments":"{\"command\":\"npm test\",\"timeoutMs\":1000}"}"#,
                r#"{"type":"tool","phase":"result","id":"call-1","is_error":false,"content":"ok"}"#,
                r#"{"type":"tool","phase":"call","id":"call-2","name":"read","arguments":"{\"file_path\":\"C:/a.txt\"}"}"#,
                r#"{"type":"tool","phase":"result","id":"call-2","is_error":true,"content":"missing"}"#,
                r#"{"type":"message","text":"做完了。"}"#,
                r#"{"type":"done","reason":"completed"}"#,
            ],
        );
        assert_eq!(tools.len(), 5, "每个阶段各上报一次");
        let first = tools.last().unwrap();
        assert_eq!(tools[1].status, "running");
        assert_eq!(tools[1].command.as_deref(), Some("npm test"));
        assert_eq!(tools[1].summary, "npm test");
        assert!(tools[1].arguments.contains("\"command\""));
        assert_eq!(tools[2].status, "completed");
        assert_eq!(tools[2].output.as_deref(), Some("ok"));
        assert_eq!(tools[4].status, "failed");
        assert_eq!(tools[4].tool_name, "read");
        assert_eq!(first.id, "call-2");
        parser.finish(&mut state, true, "");
        assert!(!state.is_error);
        assert_eq!(state.full_text, "做完了。");
    }

    #[test]
    fn forwards_approval_requests_with_arguments_from_the_tool_call() {
        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        drive(
            &mut parser,
            &mut state,
            &[
                r#"{"type":"tool","phase":"call","id":"call-9","name":"pwsh","arguments":"{\"command\":\"echo hi\"}"}"#,
            ],
        );
        let line = r#"{"type":"approval-request","id":"approval-1","session_id":"session-x","tool":"pwsh","reason":"Command requires approval","call_id":"call-9"}"#;
        let request = parser
            .parse_line(&mut state, line, &mut |_| {}, &mut |_| {}, &mut |_| {})
            .expect("必须识别授权请求");
        assert_eq!(
            request,
            DshApprovalRequest {
                id: "approval-1".into(),
                tool: "pwsh".into(),
                reason: Some("Command requires approval".into()),
                call_id: Some("call-9".into()),
            }
        );
        let payload = parser.confirm_payload(&request);
        assert_eq!(payload.tool_name, "pwsh");
        assert_eq!(payload.command.as_deref(), Some("echo hi"));
        assert!(payload.arguments.contains("echo hi"), "参数必须从 tool/call 事件补齐");
        assert!(payload.summary.starts_with("echo hi"), "摘要：{}", payload.summary);
        assert!(payload.summary.contains("Command requires approval"));
        assert!(payload.id.starts_with("confirm_"));

        // 没有对应 tool/call 时也不能丢：退化成工具名 + 原因。
        let bare = DshApprovalRequest {
            id: "approval-2".into(),
            tool: "write".into(),
            reason: None,
            call_id: None,
        };
        let payload = parser.confirm_payload(&bare);
        assert_eq!(payload.tool_name, "write");
        assert_eq!(payload.summary, "write");
        assert!(payload.arguments.is_empty());
    }

    #[test]
    fn writes_the_decision_line_the_runner_expects() {
        assert_eq!(
            approval_decision_line("approval-1", DshDecision::Allow),
            "{\"type\":\"approval-decision\",\"id\":\"approval-1\",\"decision\":\"allow\"}\n"
        );
        assert!(approval_decision_line("a\"b", DshDecision::Deny).contains("\"a\\\"b\""));
        assert!(approval_decision_line("x", DshDecision::Cancel).contains("\"cancel\""));
    }

    #[test]
    fn answers_deny_without_an_answerer_and_never_blocks_on_unknown_lines() {
        // 没有应答者时读流仍然要能把普通协议行处理完（approval 行只会在有应答者时出现）。
        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        let (thinking, text, tools) = drive(
            &mut parser,
            &mut state,
            &[
                r#"{"type":"thinking","text":"想"}"#,
                r#"{"type":"text","text":"答"}"#,
                r#"{"type":"done","reason":"completed"}"#,
            ],
        );
        assert_eq!(thinking, "想");
        assert_eq!(text, "答");
        assert!(tools.is_empty());
        parser.finish(&mut state, true, "");
        assert!(!state.is_error);
        assert_eq!(state.full_text, "答");
    }

    #[test]
    fn reports_protocol_and_process_failures() {
        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        drive(
            &mut parser,
            &mut state,
            &[
                r#"{"type":"error","code":"MISSING_CREDENTIAL","message":"no API key"}"#,
                r#"{"type":"done","reason":"error"}"#,
            ],
        );
        parser.finish(&mut state, false, "");
        assert!(state.is_error);
        assert_eq!(state.error_msg.as_deref(), Some("MISSING_CREDENTIAL: no API key"));

        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        drive(
            &mut parser,
            &mut state,
            &[r#"{"type":"done","reason":"error"}"#],
        );
        parser.finish(&mut state, false, "node.exe : dsh: SOMETHING_BROKE: boom");
        assert_eq!(state.error_msg.as_deref(), Some("SOMETHING_BROKE: boom"));

        let mut parser = RunnerParser::default();
        let mut state = StreamParseState::new();
        drive(
            &mut parser,
            &mut state,
            &[
                r#"{"type":"thinking","text":"想了半天"}"#,
                r#"{"type":"done","reason":"completed"}"#,
            ],
        );
        parser.finish(&mut state, true, "");
        assert!(state.is_error, "没有可见正文时不能算成功");
        assert!(state.error_msg.as_deref().unwrap().contains("没有产生可见回复"));
    }

    #[test]
    fn extracts_the_dsh_error_from_powershell_noise() {
        let stderr = "node.exe : dsh: MISSING_CREDENTIAL: no API key for provider route \"deepseek-official\"\nAt C:\\dsh.ps1:24 char:5\n";
        assert_eq!(
            parse_stderr_error(stderr).as_deref(),
            Some("MISSING_CREDENTIAL: no API key for provider route \"deepseek-official\"")
        );
        assert_eq!(parse_stderr_error("harmless warning"), None);
    }

    #[test]
    fn builds_resume_arguments_without_a_positional_task() {
        let args = runner_args(Path::new("C:\\tmp\\p.txt"), Some("session-1"));
        assert_eq!(
            args,
            ["--profile", "dsh-pet", "--prompt-file", "C:\\tmp\\p.txt", "--resume", "session-1"]
        );
        let args = runner_args(Path::new("/tmp/p.txt"), None);
        assert_eq!(args.len(), 4);
        assert!(!args.iter().any(|arg| arg == "--resume"));
    }

    #[test]
    fn tracks_only_its_own_session_id() {
        let adapter = DshAdapter::default();
        assert!(!adapter.has_session());
        adapter.update_session("not-a-session");
        assert!(!adapter.has_session());
        adapter.update_session("session-1234");
        assert!(adapter.has_session());
        adapter.reset_session();
        assert!(!adapter.has_session());
    }

    #[test]
    fn prepares_the_pet_profile_and_keeps_user_patch_edits() {
        let home = std::env::temp_dir().join(format!("dsh-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let dir = ensure_profile_in(&home).unwrap();

        let manifest = std::fs::read_to_string(dir.join("package.json")).unwrap();
        assert!(manifest.contains("@deepseek-ai/dsh-base"));
        assert!(manifest.contains("@deepseek-ai/dsh-headless"));
        assert!(!manifest.starts_with('\u{feff}'), "json 不能带 BOM");
        serde_json::from_str::<serde_json::Value>(&manifest).expect("manifest 必须是合法 JSON");

        let runner = std::fs::read_to_string(dir.join("plugins\\pet-runner.mjs"))
            .or_else(|_| std::fs::read_to_string(dir.join("plugins/pet-runner.mjs")))
            .unwrap();
        assert_eq!(runner, RUNNER_SOURCE);
        assert!(runner.contains("session/event"));

        let patch = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(patch.contains(PATCH_MARKER));
        assert!(patch.contains("./plugins/pet-runner.mjs"));
        assert!(patch.contains("disabled: true"));

        // 旧版本桌宠写的空补丁层会被升级成 v2。
        std::fs::write(dir.join(PATCH_FILENAME), "# 桌宠专用 profile 补丁层：默认空。\n[]\n").unwrap();
        ensure_profile_in(&home).unwrap();
        let upgraded = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(upgraded.contains(PATCH_MARKER));
        assert!(!upgraded.contains("默认空"));

        // 用户自己写的内容必须保留，桌宠条目追加在后面。
        std::fs::write(dir.join(PATCH_FILENAME), "# 我的补丁\n- insert:\n    - id: mine\n      name: './plugins/mine.mjs'\n").unwrap();
        ensure_profile_in(&home).unwrap();
        let merged = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(merged.contains("我的补丁"));
        assert!(merged.contains("id: mine"));
        assert!(merged.contains("pet-runner"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn takes_unicode_units_into_account() {
        assert_eq!(units("😀"), 2);
        assert_eq!(take_units("😀ab", 2), "😀");
        assert_eq!(take_last_units("ab😀", 2), "😀");
    }

    #[test]
    fn reads_the_default_model_from_settings() {
        let home = std::env::temp_dir().join(format!("dsh-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("settings.yaml"),
            "agent-default-model:\n  provider: deepseek-official\n  model: deepseek-flash\n  reasoningEffort: max\nllm-deepseek:\n  models:\n    - id: \"deepseek-v4-pro\"\n",
        )
        .unwrap();
        let (model, provider) = read_default_model(&home);
        assert_eq!(model.as_deref(), Some("deepseek-flash"));
        assert_eq!(provider.as_deref(), Some("deepseek-official"));
        let _ = std::fs::remove_dir_all(home);
    }

    /// 离线端到端验证：本地假模型 + 工作区内的 DSH 主目录，不碰用户凭据。
    ///
    /// cargo test --lib dsh::tests::mock_provider_streaming_resume_and_tools -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "spawns the local dsh CLI plus a mock model server"]
    async fn mock_provider_streaming_resume_and_tools() {
        let Some(node) = find_node(Path::new(".")) else {
            eprintln!("跳过：本机没有 node");
            return;
        };
        if find_cli().is_err() {
            eprintln!("跳过：本机没有 dsh CLI");
            return;
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("dsh-mock-provider.cjs");
        assert!(script.is_file(), "缺少假模型脚本 {}", script.display());

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".dsh-selftest")
            .join(uuid::Uuid::new_v4().to_string());
        let home = root.join("home");
        let workspace = root.join("workspace");
        let prompts = root.join("prompts");
        for dir in [&home, &workspace, &prompts] {
            std::fs::create_dir_all(dir).unwrap();
        }

        // 1) 起假模型服务，端口由它自己选。
        let port_file = root.join("port.txt");
        let mut mock = Command::new(&node)
            .arg(&script)
            .arg("--port-file")
            .arg(&port_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("start mock provider");
        let mut port = None;
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                if let Ok(value) = text.trim().parse::<u16>() {
                    port = Some(value);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let port = port.expect("mock provider did not report a port");

        // 2) 工作区内的 DSH 主目录：设置文档把默认模型指向假模型。
        std::fs::write(
            home.join("settings.yaml"),
            format!(
                "agent-default-model:\n  provider: pet-mock\n  model: pet-mock-model\nllm-pi-ai:\n  providers:\n    pet-mock:\n      displayName: Pet Mock\n      apiKeyEnv: PET_MOCK_API_KEY\n      api: openai-completions\n      baseURL: http://127.0.0.1:{port}/v1\n      compat:\n        thinkingFormat: deepseek\n        maxTokensField: max_tokens\n      models:\n        - id: pet-mock-model\n          name: Pet Mock Model\n          contextWindow: 200000\n          maxTokens: 8192\n"
            ),
        )
        .unwrap();
        std::env::set_var(DSH_HOME_ENV, &home);
        // 假模型路由的凭据引用，同样只存在于测试进程里。
        std::env::set_var("PET_MOCK_API_KEY", "pet-mock-key");

        let adapter = DshAdapter::default();
        let system = "这是桌宠接入测试。回答简短。";
        let marker = format!("PET-MARKER-FIRST-TURN-{}", uuid::Uuid::new_v4());

        // 第一轮：推理增量、正文增量、会话 ID。
        let first = dsh_turn(
            &adapter,
            &workspace,
            &prompts,
            &format!("{marker} 请只回复这个标记。"),
            &[],
            system,
            |_| {},
        )
        .await;
        assert!(!first.is_error, "首轮失败：{}", first.error_msg.unwrap_or_default());
        assert!(
            first.full_thinking.contains("先确认用户要求"),
            "必须拿到推理增量，实际：{:?}",
            first.full_thinking
        );
        assert!(first.full_text.contains(&marker), "首轮正文：{}", first.full_text);
        assert!(first.full_text.contains("seen1=no"), "首轮不应带历史：{}", first.full_text);
        let session = first.session_id.clone().expect("runner 必须回报会话 ID");
        adapter.update_session(&session);

        // 第二轮：续接同一会话，请求历史里应当带得回第一轮的标记。
        let second = dsh_turn(
            &adapter,
            &workspace,
            &prompts,
            "我刚才给你的标记是什么？如果你还看得到历史就回答它。",
            &[],
            system,
            |_| {},
        )
        .await;
        assert!(!second.is_error, "续接轮失败：{}", second.error_msg.unwrap_or_default());
        assert_eq!(second.session_id.as_deref(), Some(session.as_str()), "必须续接同一会话");
        assert!(
            second.full_text.contains("seen1=yes"),
            "--resume 没有把历史带给模型：{}",
            second.full_text
        );

        // 第三轮：模型请求工具 → 授权请求落到桌宠 → 允许后工具真的执行、结果回流。
        let tools = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = tools.clone();
        let approvals = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let answerer = RecordingAnswerer {
            decision: DshDecision::Allow,
            seen: approvals.clone(),
        };
        let third = {
            let mut on_tool = move |tool: &ToolEvent| sink.lock().unwrap().push(tool.clone());
            dsh_turn_with(
                &adapter,
                &workspace,
                &prompts,
                "USE_TOOL 请用命令确认结果。",
                &[],
                system,
                &mut on_tool,
                Some(Box::new(answerer)),
            )
            .await
        };
        let approvals = approvals.lock().unwrap().clone();
        println!("授权请求 {} 条：{approvals:?}", approvals.len());
        assert_eq!(approvals.len(), 1, "必须把授权请求转发给桌宠一次");
        assert_eq!(approvals[0].0, "pwsh");
        assert!(
            approvals[0].1.contains("echo pet-tool-check"),
            "确认弹窗要拿到真实命令，实际摘要：{}",
            approvals[0].1
        );
        assert!(!third.is_error, "工具轮失败：{}", third.error_msg.unwrap_or_default());
        let tools = tools.lock().unwrap().clone();
        println!(
            "工具事件 {} 条：{:?}",
            tools.len(),
            tools
                .iter()
                .map(|tool| (tool.tool_name.clone(), tool.status.clone(), tool.summary.clone()))
                .collect::<Vec<_>>()
        );
        assert!(tools.iter().any(|tool| tool.tool_name == "pwsh"), "必须上报 pwsh 调用");
        assert!(
            tools.iter().any(|tool| tool.status == "completed" || tool.status == "failed"),
            "工具事件必须包含终态"
        );
        assert!(
            third.full_text.contains("工具返回"),
            "工具结果必须回流到下一轮回答：{}",
            third.full_text
        );

        let _ = mock.kill().await;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 测试用应答者：记录收到的授权请求（工具名、弹窗摘要、参数），按固定策略回答。
    struct RecordingAnswerer {
        decision: DshDecision,
        seen: std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    }

    #[async_trait]
    impl ApprovalAnswerer for RecordingAnswerer {
        async fn decide(
            &self,
            request: &DshApprovalRequest,
            payload: ToolConfirmPayload,
        ) -> DshDecision {
            self.seen.lock().unwrap().push((
                request.tool.clone(),
                payload.summary.clone(),
                payload.arguments.clone(),
            ));
            self.decision
        }
    }

    /// 实机跑一轮桌宠 runner：真实 profile、真实凭据、真实协议解析。
    async fn dsh_turn(
        adapter: &DshAdapter,
        workspace: &Path,
        prompts: &Path,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system: &str,
        mut on_tool: impl FnMut(&ToolEvent),
    ) -> StreamParseState {
        dsh_turn_with(adapter, workspace, prompts, message, history, system, &mut on_tool, None).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn dsh_turn_with(
        adapter: &DshAdapter,
        workspace: &Path,
        prompts: &Path,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system: &str,
        on_tool: &mut dyn FnMut(&ToolEvent),
        approvals: Option<Box<dyn ApprovalAnswerer>>,
    ) -> StreamParseState {
        let mut child = adapter
            .spawn_in_workspace(workspace, prompts, message, history, system)
            .await
            .expect("start local dsh");
        // 调试时可以收紧预算，避免一次挂起就等满四分钟。
        let budget = std::env::var("DSH_TEST_TURN_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(240);
        let result = tokio::time::timeout(
            Duration::from_secs(budget),
            read_stream_with_activity(&mut child, |_| {}, |_| {}, on_tool, approvals),
        )
        .await;
        if result.is_err() {
            if let Some(pid) = child.id() {
                crate::kill_process_tree(pid);
            }
        }
        result
            .expect("local dsh turn timed out")
            .expect("read local dsh turn")
    }

    /// 续接会话 + 工具事件的实机验证（需要本机已安装并登录 dsh）：
    /// cargo test --lib dsh::tests::local_cli_resume_and_tools -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a locally installed and authenticated DeepSeek Harness CLI"]
    async fn local_cli_resume_and_tools() {
        ensure_profile().expect("准备桌宠 profile");
        let adapter = DshAdapter::default();
        let root = std::env::temp_dir().join(format!("dsh-resume-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        let marker = format!("桌宠校验-{}", uuid::Uuid::new_v4());
        let system = "这是桌宠接入测试。按用户要求执行，回答简短。";

        // 第一轮：建立会话。
        let first = dsh_turn(
            &adapter,
            &workspace,
            &prompts,
            &format!("请只回复这个校验码，不要做别的事：{marker}"),
            &[],
            system,
            |_| {},
        )
        .await;
        assert!(!first.is_error, "首轮失败：{}", first.error_msg.unwrap_or_default());
        let session = first.session_id.clone().expect("runner 必须回报会话 ID");
        assert!(first.full_text.contains(&marker), "首轮回答：{}", first.full_text);
        adapter.update_session(&session);

        // 第二轮：续接同一会话，模型应当记得上一轮的校验码。
        let second = dsh_turn(
            &adapter,
            &workspace,
            &prompts,
            "我上一轮给你的校验码是什么？只回复校验码本身。",
            &[],
            system,
            |_| {},
        )
        .await;
        assert!(!second.is_error, "续接轮失败：{}", second.error_msg.unwrap_or_default());
        assert_eq!(
            second.session_id.as_deref(),
            Some(session.as_str()),
            "必须续接同一会话"
        );
        assert!(
            second.full_text.contains(&marker),
            "续接后应当记得上一轮内容，实际回答：{}",
            second.full_text
        );

        // 第三轮：必须真的调用工具才能回答，用来验证工具事件通道。
        let tools = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = tools.clone();
        let third = dsh_turn(
            &adapter,
            &workspace,
            &prompts,
            "用你的命令行工具执行 echo pet-tool-check，然后在回答里原样输出命令结果。",
            &[],
            "这是桌宠接入测试。必须真实调用工具完成任务，不要凭空作答。",
            move |tool| sink.lock().unwrap().push(tool.clone()),
        )
        .await;
        assert!(!third.is_error, "工具轮失败：{}", third.error_msg.unwrap_or_default());
        let tools = tools.lock().unwrap().clone();
        println!(
            "工具事件 {} 条：{:?}",
            tools.len(),
            tools
                .iter()
                .map(|tool| (tool.id.clone(), tool.tool_name.clone(), tool.status.clone()))
                .collect::<Vec<_>>()
        );
        assert!(!tools.is_empty(), "必须上报工具事件");
        assert!(
            tools
                .iter()
                .any(|tool| tool.status == "completed" || tool.status == "failed"),
            "工具事件必须包含终态"
        );
        println!("思考增量 {} 字", third.full_thinking.chars().count());

        let _ = std::fs::remove_dir_all(root);
    }

    // 需要本机安装并登录 dsh：cargo test --lib dsh::tests::local_cli_round_trip -- --ignored
    #[tokio::test]
    #[ignore = "requires a locally installed and authenticated DeepSeek Harness CLI"]
    async fn local_cli_round_trip() {
        let marker = format!("桌宠校验-{}", uuid::Uuid::new_v4());
        let root = std::env::temp_dir().join(format!("dsh-round-trip-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&workspace).unwrap();
        let adapter = DshAdapter::default();
        let state = dsh_turn(
            &adapter,
            &workspace,
            &prompts,
            &format!("只回复这个校验码：{marker}"),
            &[],
            "这是桌宠接入测试。只回答用户要求的内容，不要调用工具或读取文件。",
            |_| {},
        )
        .await;
        assert!(
            !state.is_error,
            "本机 dsh 执行失败：{}",
            state.error_msg.unwrap_or_default()
        );
        assert!(state.full_text.contains(&marker), "回答：{}", state.full_text);
        assert!(state.session_id.is_some(), "runner 必须回报会话 ID");
        let _ = std::fs::remove_dir_all(root);
    }
}
