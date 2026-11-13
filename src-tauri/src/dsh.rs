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
//!   * 任务文本走提示词文件，不再受 Windows 命令行长度限制；runner 读完即删，
//!     文件只在这一轮的启动窗口里存在，Rust 侧 sweep 仅作兜底。
//!
//! runner 是桌宠 profile 的一部分，登录信息、模型/provider、权限预设，以及机器级
//! 的 $DSH_HOME/cordis.patch.yml 仍然来自用户的 DSH 配置，没有任何凭据被复制到
//! 桌宠里。

use crate::agent_models::{AgentEffortOption, AgentModelCatalog, AgentModelOption};
use crate::cli_activity::ToolEvent;
use crate::direct_api::ToolConfirmPayload;
use crate::interactions::WaitResult;
use crate::openclaw::StreamParseState;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::Manager;
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
/// 文件由 runner 读完即删，正常轮次不会长期留存；sweep 只清理兜底残留。
const PROMPT_DIR: &str = "dsh-prompts";
/// 兜底 TTL：提示词文件正常由 runner 读完即删，只有启动失败/进程崩溃留下的
/// 残留才需要清理；写新提示词时顺手扫一遍。
const PROMPT_TTL: Duration = Duration::from_secs(60 * 60);

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
    if let Some(cli) = find_desktop_cli(&desktop_install_dirs()) {
        return Ok(cli);
    }
    Err("未找到本机 DeepSeek Harness CLI。请安装桌面版，或运行 npm install -g @deepseek-ai/dsh，再刷新状态。".into())
}

/// 桌面版附带自己的 CLI；它不一定已通过 Manage dsh Command 注册到 PATH。
fn find_desktop_cli(install_dirs: &[PathBuf]) -> Option<PathBuf> {
    install_dirs.iter().filter(|root| root.is_absolute())
        .map(|root| root.join("resources").join("runtime").join("cli").join("bin").join("dsh.cmd"))
        .find(|path| path.is_file())
}

#[cfg(target_os = "windows")]
fn desktop_install_dirs() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(executable) = std::env::var_os("DSH_DESKTOP_NODE_EXECUTABLE") {
        if let Some(parent) = Path::new(&executable).parent() { roots.push(parent.to_path_buf()); }
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(variable) {
            let base = PathBuf::from(base);
            roots.push(base.join("DeepSeek Harness"));
            roots.push(base.join("Programs").join("DeepSeek Harness"));
        }
    }
    // 开始菜单快捷方式保留真实安装位置，也适用于用户选择的其它盘符。
    // 只读取已安装应用的快捷方式，不改注册表、不扫描磁盘、不注册全局命令。
    let powershell = std::env::var_os("SystemRoot").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let script = r#"[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$shell = New-Object -ComObject WScript.Shell
foreach ($root in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('CommonPrograms'))) {
  if ($root -and (Test-Path -LiteralPath $root)) {
    Get-ChildItem -LiteralPath $root -Filter 'DeepSeek Harness.lnk' -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object {
      $target = $shell.CreateShortcut($_.FullName).TargetPath
      if ($target -and [IO.Path]::GetFileName($target) -ieq 'DeepSeek Harness.exe') { [IO.Path]::GetDirectoryName($target) }
    }
  }
}"#;
    let mut probe = std::process::Command::new(powershell);
    use std::os::windows::process::CommandExt;
    probe.creation_flags(CREATE_NO_WINDOW);
    if let Ok(output) = probe.args(["-NoProfile", "-NonInteractive", "-Command", script]).output() {
        if output.status.success() {
            roots.extend(String::from_utf8_lossy(&output.stdout).lines()
                .map(str::trim).filter(|line| !line.is_empty()).map(PathBuf::from));
        }
    }
    roots
}

#[cfg(not(target_os = "windows"))]
fn desktop_install_dirs() -> Vec<PathBuf> { Vec::new() }

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
         # 桌宠沿用 DSH 本机凭据与账号登录，不使用 headless 的无状态凭据模式。\n\
         - id: credentials\n\
         \x20\x20disabled: false\n\
         - id: authorization\n\
         \x20\x20disabled: false\n\
         - id: deepseek-account\n\
         \x20\x20disabled: false\n\
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

/// 需要从本机 DSH profile 同步的渠道配置行（id → CLI 可加载的 base 包名）。
///
/// 桌面 app 的 profile 里 llm-deepseek 用的是 app runtime 专用包名
/// （dsh-llm-deepseek-api-key），CLI 的 profiles/node_modules 里没有，照抄会加载失败，
/// 所以同步时统一改写成 base 包名；config 缩进原样保留。
const CHANNEL_ENTRIES: [(&str, &str); 3] = [
    ("llm-pi-ai", "@deepseek-ai/dsh-llm-pi-ai"),
    ("llm-deepseek", "@deepseek-ai/dsh-llm-deepseek"),
    ("agent-default-model", "@deepseek-ai/dsh-agent-default-model"),
];

/// $DSH_HOME/settings.yaml 是否已经配置了渠道段（顶层、非注释）。
/// 已配置时 dsh-base 的 settings 插件会自己生效，桌宠不能再复制一份（避免双份）。
fn settings_define_channels(home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join("settings.yaml")) else {
        return false;
    };
    text.lines().any(|line| {
        if line.starts_with(' ') || line.starts_with('\t') {
            return false;
        }
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        CHANNEL_ENTRIES.iter().any(|(id, _)| {
            line.strip_prefix(id)
                .is_some_and(|rest| rest.starts_with(':'))
        })
    })
}

/// profile 目录的同步优先级：desktop → web → headless → open-design → 其余字典序。
fn profile_rank(name: &str) -> usize {
    match name {
        "desktop" => 0,
        "web" => 1,
        "headless" => 2,
        "open-design" => 3,
        _ => 4,
    }
}

/// 顶层条目行（行首 "- id:"）的 id。
fn entry_id(line: &str) -> Option<&str> {
    line.strip_prefix("- id:").map(str::trim)
}

/// 是否是一个条目的起始行（顶层 "- " 开头）。
fn is_entry_line(line: &str) -> bool {
    line.starts_with("- ") || line == "-"
}

/// 把条目块里的插件 name 改写成 CLI 可加载的 base 包名（缩进保留）。
///
/// 只改条目自身的 name（config: 之前那一行）；config 里的模型 name 是用户配置的
/// 展示名，必须原样保留。
fn rewrite_entry_name(block: &[String], base_name: &str) -> Vec<String> {
    let mut rewritten = Vec::with_capacity(block.len());
    let mut in_config = false;
    let mut replaced = false;
    for line in block {
        let trimmed = line.trim_start();
        if trimmed.starts_with("config:") {
            in_config = true;
        }
        if !in_config && !replaced && trimmed.starts_with("name:") {
            let indent = &line[..line.len() - trimmed.len()];
            rewritten.push(format!("{indent}name: '{base_name}'"));
            replaced = true;
        } else {
            rewritten.push(line.clone());
        }
    }
    rewritten
}

/// 找到本机第一个配置了渠道行的 DSH profile（返回 profile 名与 patch 内容）。
fn find_channel_source(home: &Path) -> Option<(String, String)> {
    let profiles = home.join("profiles");
    let mut names: Vec<String> = std::fs::read_dir(&profiles)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != PROFILE_NAME && name != "node_modules")
        .collect();
    names.sort_by(|left, right| {
        profile_rank(left)
            .cmp(&profile_rank(right))
            .then_with(|| left.cmp(right))
    });
    for name in names {
        let patch = profiles.join(&name).join(PATCH_FILENAME);
        let Ok(content) = std::fs::read_to_string(&patch) else {
            continue;
        };
        if CHANNEL_ENTRIES
            .iter()
            .any(|(id, _)| content.lines().any(|line| entry_id(line) == Some(*id)))
        {
            return Some((name, content));
        }
    }
    None
}

/// 生成从本机 DSH profile 同步来的渠道配置段（无源 / settings 已配置时返回 None）。
/// 只提取 llm-pi-ai / llm-deepseek / agent-default-model 三个顶层条目，顺序沿用源文件。
fn channel_sync_section(home: &Path) -> Option<String> {
    if settings_define_channels(home) {
        return None;
    }
    let (source_name, content) = find_channel_source(home)?;
    let lines: Vec<&str> = content.lines().collect();
    let mut blocks: Vec<String> = Vec::new();
    let mut emitted: Vec<String> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(id) = entry_id(lines[index]) else {
            index += 1;
            continue;
        };
        let mut end = index + 1;
        while end < lines.len() && !is_entry_line(lines[end]) {
            end += 1;
        }
        if let Some((_, base_name)) = CHANNEL_ENTRIES
            .iter()
            .find(|(channel_id, _)| *channel_id == id)
        {
            if !emitted.iter().any(|entry| entry == id) {
                let mut block: Vec<String> = lines[index..end]
                    .iter()
                    .map(|line| (*line).to_string())
                    .collect();
                while block.last().is_some_and(|line| line.trim().is_empty()) {
                    block.pop();
                }
                blocks.push(rewrite_entry_name(&block, base_name).join("\n"));
                emitted.push(id.to_string());
            }
        }
        index = end;
    }
    if blocks.is_empty() {
        return None;
    }
    Some(format!(
        "# 自动同步自本机 DSH profile: {source_name}（源文件变化自动重建，勿手改）\n{}\n",
        blocks.join("\n")
    ))
}

/// 桌宠 profile 的完整托管补丁：静态层 + 从本机 DSH profile 同步的渠道配置。
fn managed_patch(home: &Path) -> String {
    let mut patch = patch_layer();
    if let Some(section) = channel_sync_section(home) {
        patch.push_str(&section);
    }
    patch
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
    // 每轮重建托管段：本机渠道配置变化后下一次 ensure_profile 就能生效。
    let target = managed_patch(home);
    let existing = std::fs::read_to_string(&patch).unwrap_or_default();
    if patch_is_ours(&existing) {
        if existing != target {
            std::fs::write(&patch, &target)
                .map_err(|error| format!("无法写入 {}：{error}", patch.display()))?;
        }
    } else if !existing.contains(PATCH_MARKER) {
        // 用户在补丁层里写过自己的条目：保留它们，只追加桌宠需要的部分。
        let mut merged = existing;
        if !merged.ends_with('\n') {
            merged.push('\n');
        }
        merged.push_str(&target);
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
///
/// runner 启动后第一件事就是读取并删除它（用后即删），正常情况下文件只存在很短
/// 时间；写入前先 sweep，清掉启动失败/进程崩溃留下的旧残留。
fn write_prompt(dir: &Path, task: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("无法创建 {}：{error}", dir.display()))?;
    sweep_stale_prompts(dir);
    let path = dir.join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, task).map_err(|error| format!("无法写入提示词文件：{error}"))?;
    Ok(path)
}

/// 兜底清理：正常轮次由 runner 读完即删；这里只清启动失败/进程崩溃等留下的残留
/// （超过 [`PROMPT_TTL`]）。尽力而为，失败不影响本轮。
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

fn runner_args(
    prompt_file: &Path,
    session: Option<&str>,
    model: Option<&str>,
    provider: Option<&str>,
    effort: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--profile".into(),
        PROFILE_NAME.into(),
        "--prompt-file".into(),
        prompt_file.to_string_lossy().into_owned(),
    ];
    // 本回合模型覆盖：runner 支持 --model 时直接覆盖 selection.model（provider 不变）。
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        args.push("--model".into());
        args.push(model.to_string());
    }
    // 本回合渠道覆盖：provider 与 model 一起决定最终模型（仅本机已配置的渠道有效）。
    if let Some(provider) = provider.map(str::trim).filter(|provider| !provider.is_empty()) {
        args.push("--provider".into());
        args.push(provider.to_string());
    }
    // 本回合思考强度覆盖：runner 会用最终 provider+model 校验 efforts，不支持时忽略。
    if let Some(effort) = effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        args.push("--effort".into());
        args.push(effort.to_string());
    }
    if let Some(id) = session.filter(|id| !id.trim().is_empty()) {
        args.push("--resume".into());
        args.push(id.to_string());
    }
    args
}

fn launcher_command(launcher: &Launcher, args: &[String]) -> Command {
    match launcher {
        Launcher::Node { node, entry } => {
            let mut command = crate::cli_launch::command(node);
            command.arg(entry);
            command.args(args);
            command
        }
        Launcher::Shim(path) => {
            let mut command = crate::cli_launch::command(path);
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
    /// 旧签名：会话 id 取自适配器内部状态。
    pub async fn spawn_streaming(
        &self,
        app: &tauri::AppHandle,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session_id.lock().ok().and_then(|id| id.clone());
        self.spawn_streaming_with(
            app,
            message,
            history,
            system_prompt,
            session.as_deref(),
            None,
            None,
            None,
            "",
        )
        .await
    }

    /// 显式会话 / 模型 / 渠道 / 思考强度 / 权限档位版本：不依赖适配器内部状态，供多会话与协作调用。
    /// permission 为空串时不覆写 `DSH_PERMISSION_MODE`，沿用本机 DSH 配置。
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_streaming_with(
        &self,
        app: &tauri::AppHandle,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system_prompt: &str,
        session: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
        effort: Option<&str>,
        permission: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let workspace = workspace_dir(app)?;
        let prompts = prompt_dir(app)?;
        // 会话级 / 设置页选定的权限档位：空串不覆写 DSH 环境，保持本机配置。
        let permission = crate::cli_permission::dsh_permission(permission);
        let state = app.state::<crate::AppState>();
        let memory_url = state.permission_bridge.lock().ok().and_then(|bridge| {
            bridge.as_ref().map(|b| b.url_for_session(&crate::current_turn_session(&state)))
        });
        self.spawn_in_workspace_with(
            &workspace,
            &prompts,
            message,
            history,
            system_prompt,
            session,
            model,
            provider,
            effort,
            permission,
            memory_url.as_deref(),
        )
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
        self.spawn_in_workspace_with(
            workspace,
            prompt_root,
            message,
            history,
            system_prompt,
            session.as_deref(),
            None,
            None,
            None,
            crate::cli_permission::DshPermission::default(),
            None,
        )
        .await
    }

    /// 显式会话 / 模型 / 渠道 / 思考强度 / 权限档位版本。
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_in_workspace_with(
        &self,
        workspace: &Path,
        prompt_root: &Path,
        message: &str,
        history: &[crate::storage::ChatMessage],
        system_prompt: &str,
        session: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
        effort: Option<&str>,
        permission: crate::cli_permission::DshPermission,
        memory_url: Option<&str>,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = session.map(str::to_string);
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
        let mut command = launcher_command(
            &launcher,
            &runner_args(&prompt_file, session.as_deref(), model, provider, effort),
        );
        command.env_remove("PET_MEMORY_MCP_URL");
        if let Some(url) = memory_url { command.env("PET_MEMORY_MCP_URL", url); }
        // dsh-base 用 DSH_PERMISSION_MODE 决定默认沙箱与审批策略；不设置时沿用
        // 用户环境里的值，这是 DSH 官方支持的部署覆盖方式。
        if let Some(mode) = permission.env_mode {
            command.env("DSH_PERMISSION_MODE", mode);
        }
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
    session_id: String,
    /// 本会话解析出的 DSH 权限档位是否允许自动批准升权请求。
    auto_approve: bool,
    timeout: Duration,
}

impl UiApproval {
    pub fn new(app: tauri::AppHandle, cancel: CancellationToken) -> Self {
        Self::for_session(app, cancel, String::new(), false)
    }

    /// `auto_approve` 来自本会话解析出的 DSH 权限档位（`workspace-write-auto`）：
    /// 为 true 时升权请求直接放行，不再弹窗。
    pub fn for_session(
        app: tauri::AppHandle,
        cancel: CancellationToken,
        session_id: String,
        auto_approve: bool,
    ) -> Self {
        Self {
            app,
            cancel,
            session_id,
            auto_approve,
            timeout: Duration::from_secs(120),
        }
    }

}

#[async_trait]
impl ApprovalAnswerer for UiApproval {
    async fn decide(&self, _request: &DshApprovalRequest, mut payload: ToolConfirmPayload) -> DshDecision {
        if payload.session_id.is_empty() {
            payload.session_id = self.session_id.clone();
        }
        // “工作区可写 + 自动批准”档位：保留 DSH 沙箱，但桌宠不再为升权弹窗。
        // 允许只作用于当前这一次请求，后续请求仍会重新询问这里。
        if self.auto_approve {
            // 用户已经按下停止时不再放行，避免取消后仍执行越界命令。
            if self.cancel.is_cancelled() {
                return DshDecision::Cancel;
            }
            return DshDecision::Allow;
        }
        match crate::interactions::confirm_with_ui(&self.app, &self.cancel, payload, self.timeout)
            .await
        {
            WaitResult::Answered(true) => DshDecision::Allow,
            WaitResult::Cancelled => DshDecision::Cancel,
            // 用户拒绝、超时、窗口关闭都按拒绝处理：失败关闭，不给未确认的操作开口子。
            WaitResult::Answered(false) | WaitResult::TimedOut | WaitResult::Closed => {
                DshDecision::Deny
            }
        }
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
                ..Default::default()
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
            // 授权决定在子进程读取循环中同步进行，这里读到的回合会话即当前 DSH 任务归属。
            session_id: String::new(),
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
    let mut command = crate::cli_launch::command(shim);
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


// ===== 模型目录（agent 窗口模型选择器的数据源）=====

/// pet-runner `--list-models` 的协议行前缀。
const MODEL_LINE_PREFIX: &str = "===AGENT_MODELS===";

/// 本机 dsh 的模型目录：用桌宠 profile 跑 `--profile dsh-pet --list-models`。
///
/// runner 是桌宠资产，ensure_profile() 会先把它同步到当前版本，因此即使旧 profile
/// 里的 runner 还不支持 --list-models，也会在第一次探测时被升级。
pub async fn model_catalog() -> Result<AgentModelCatalog, String> {
    ensure_profile()?;
    let launcher = resolve_launcher()?;
    let args = vec![
        "--profile".to_string(),
        PROFILE_NAME.to_string(),
        "--list-models".to_string(),
    ];
    let mut command = launcher_command(&launcher, &args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .map_err(|_| "读取 dsh 模型目录超时（20 秒）。".to_string())?
        .map_err(|error| format!("无法运行 dsh --list-models：{error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse_models_output(&stdout) {
        Ok(catalog) => Ok(catalog),
        Err(error) if !output.status.success() => {
            // 协议行缺失时用 stderr 最后一条非空行做诊断（登录 / profile 报错都在这）。
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail = stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().chars().take(300).collect::<String>())
                .unwrap_or_default();
            Err(if tail.is_empty() { error } else { format!("{error}（runner 诊断：{tail}）") })
        }
        Err(error) => Err(error),
    }
}

/// 取 JSON 字段的 trim 后文本（非字符串 / 空串 → ""）。
fn text_field(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().trim().to_string()
}

/// 解析 runner 的 `--list-models` 协议：前缀行 + JSON。
pub(crate) fn parse_models_output(stdout: &str) -> Result<AgentModelCatalog, String> {
    let line = stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix(MODEL_LINE_PREFIX))
        .ok_or_else(|| {
            "pet-runner 没有输出 ===AGENT_MODELS=== 协议行，请确认桌宠 profile 已更新。".to_string()
        })?;
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|error| format!("无法解析 pet-runner 模型目录 JSON：{error}"))?;
    if let Some(error) = value["error"]
        .as_str()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        return Err(format!("dsh 模型目录探测失败：{error}"));
    }
    // v2 协议：local { provider, model, effort } + providers 列表；
    // v1 协议兼容：顶层 provider / provider_label / model / effort。
    let local = &value["local"];
    let (local_provider, local_model, local_effort) = if local.is_object() {
        (
            text_field(local, "provider"),
            text_field(local, "model"),
            text_field(local, "effort"),
        )
    } else {
        (
            text_field(&value, "provider"),
            text_field(&value, "model"),
            text_field(&value, "effort"),
        )
    };
    let legacy_provider_label = text_field(&value, "provider_label");
    // provider id → 展示名（v2 providers 列表；旧协议没有时退回 id）。
    let provider_labels: Vec<(String, String)> = value["providers"]
        .as_array()
        .map(|providers| {
            providers
                .iter()
                .filter_map(|entry| {
                    let id = text_field(entry, "id");
                    if id.is_empty() {
                        return None;
                    }
                    let label = text_field(entry, "label");
                    Some((id.clone(), if label.is_empty() { id } else { label }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let provider = local_provider.clone();
    let mut catalog = AgentModelCatalog::empty("dsh");
    catalog.models = value["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|entry| {
                    let id = entry["id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|id| !id.is_empty())?
                        .to_string();
                    let name = {
                        let name = text_field(entry, "name");
                        // llm 包在 config 未给 name 时会回退成包名，不能直接当展示名。
                        if name.is_empty() || name.starts_with('@') {
                            id.clone()
                        } else {
                            name
                        }
                    };
                    let description = entry["description"]
                        .as_str()
                        .map(str::trim)
                        .filter(|description| !description.is_empty())
                        .map(str::to_string);
                    let entry_provider = {
                        let id = text_field(entry, "provider");
                        if id.is_empty() {
                            (!provider.is_empty()).then(|| provider.clone())
                        } else {
                            Some(id)
                        }
                    };
                    let entry_provider_label = {
                        let label = text_field(entry, "provider_label");
                        if !label.is_empty() {
                            Some(label)
                        } else {
                            entry_provider.as_deref().map(|id| {
                                provider_labels
                                    .iter()
                                    .find(|(key, _)| key == id)
                                    .map(|(_, label)| label.clone())
                                    .or_else(|| {
                                        (!legacy_provider_label.is_empty()
                                            && legacy_provider_label != *id)
                                            .then(|| legacy_provider_label.clone())
                                    })
                                    .unwrap_or_else(|| id.to_string())
                            })
                        }
                    };
                    let default_effort = entry["default_effort"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let efforts = entry["efforts"]
                        .as_array()
                        .map(|efforts| {
                            efforts
                                .iter()
                                .filter_map(|effort| {
                                    let id = effort["id"]
                                        .as_str()
                                        .map(str::trim)
                                        .filter(|id| !id.is_empty())?
                                        .to_string();
                                    let name = effort["name"]
                                        .as_str()
                                        .map(str::trim)
                                        .filter(|name| !name.is_empty())
                                        .map(str::to_string)
                                        .unwrap_or_else(|| id.clone());
                                    let description = effort["description"]
                                        .as_str()
                                        .map(str::trim)
                                        .filter(|description| !description.is_empty())
                                        .map(str::to_string);
                                    Some(AgentEffortOption {
                                        id,
                                        name,
                                        description,
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    Some(AgentModelOption {
                        id,
                        name,
                        description,
                        provider: entry_provider,
                        provider_label: entry_provider_label,
                        default_effort,
                        efforts,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // v2：本机默认模型展示名为「渠道 / 模型」；旧协议只有单 provider 时退回模型名。
    let local_model_label = if local_model.is_empty() {
        String::new()
    } else {
        let matched = catalog
            .models
            .iter()
            .find(|option| {
                option.id == local_model
                    && (local_provider.is_empty()
                        || option.provider.as_deref() == Some(local_provider.as_str()))
            })
            .or_else(|| catalog.models.iter().find(|option| option.id == local_model));
        let model_name = matched
            .map(|option| option.name.clone())
            .unwrap_or_else(|| local_model.clone());
        let provider_label = if local_provider.is_empty() {
            String::new()
        } else {
            matched
                .and_then(|option| option.provider_label.clone())
                .filter(|label| !label.is_empty())
                .or_else(|| {
                    provider_labels
                        .iter()
                        .find(|(key, _)| *key == local_provider)
                        .map(|(_, label)| label.clone())
                })
                .unwrap_or_else(|| local_provider.clone())
        };
        if provider_label.is_empty() {
            model_name
        } else {
            format!("{provider_label} / {model_name}")
        }
    };
    catalog.local_provider = local_provider;
    catalog.local_model = local_model;
    catalog.local_model_label = local_model_label;
    catalog.local_effort = local_effort;
    Ok(catalog)
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
            id: 0,
            session_id: String::new(),
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
    fn desktop_cli_detects_custom_install_path_without_path_registration() {
        let root = std::env::temp_dir().join(format!("dsh 桌面版 {}", uuid::Uuid::new_v4()));
        let shim = root.join("resources/runtime/cli/bin/dsh.cmd");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        std::fs::write(&shim, "@echo off").unwrap();
        assert_eq!(find_desktop_cli(&[root.join("missing"), root.clone()]), Some(shim));
        assert_eq!(find_desktop_cli(&[PathBuf::from("relative-install")]), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "probes the installed desktop or npm DSH CLI without a model call"]
    async fn local_cli_detected_launcher_reports_version() {
        let launcher = resolve_launcher().expect("detect installed DSH CLI");
        let version = probe_version(&launcher).await.expect("DSH version probe");
        assert!(!version.trim().is_empty());
        eprintln!("Detected {}: {}", launcher.display_path().display(), version);
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
    fn writes_unique_prompt_files_that_runner_deletes_after_read() {
        // 文件名唯一、扩展名 .txt、内容原样落盘；刚写入的文件不算残留，不会被 sweep 清掉。
        let dir = std::env::temp_dir().join(format!("dsh-prompts-test-{}", uuid::Uuid::new_v4()));
        let first = write_prompt(&dir, "任务一").expect("写入提示词失败");
        let second = write_prompt(&dir, "任务二").expect("写入提示词失败");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "任务一");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "任务二");
        assert_ne!(first, second, "每轮提示词文件必须唯一");
        assert_eq!(first.extension().and_then(|ext| ext.to_str()), Some("txt"));
        assert!(
            first.exists() && second.exists(),
            "TTL 内的提示词文件不应被 sweep 清掉"
        );
        assert_eq!(
            PROMPT_TTL,
            Duration::from_secs(60 * 60),
            "runner 读完即删，TTL 只需兜底并保持 1 小时"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
        let args = runner_args(Path::new("C:\\tmp\\p.txt"), Some("session-1"), None, None, None);
        assert_eq!(
            args,
            ["--profile", "dsh-pet", "--prompt-file", "C:\\tmp\\p.txt", "--resume", "session-1"]
        );
        let args = runner_args(Path::new("/tmp/p.txt"), None, None, None, None);
        assert_eq!(args.len(), 4);
        assert!(!args.iter().any(|arg| arg == "--resume"));
    }

    #[test]
    fn passes_an_explicit_model_override_to_the_runner() {
        let args = runner_args(Path::new("/tmp/p.txt"), None, Some("deepseek-reasoner"), None, None);
        assert!(args.windows(2).any(|pair| pair == ["--model", "deepseek-reasoner"]));
        // 空模型字符串表示跟随本机配置。
        let args = runner_args(Path::new("/tmp/p.txt"), Some("session-1"), Some("  "), None, None);
        assert!(!args.iter().any(|arg| arg == "--model"));
        assert!(args.windows(2).any(|pair| pair == ["--resume", "session-1"]));
    }

    #[test]
    fn passes_an_explicit_reasoning_effort_to_the_runner() {
        let args = runner_args(Path::new("/tmp/p.txt"), None, None, None, Some("high"));
        assert!(args.windows(2).any(|pair| pair == ["--effort", "high"]));
        let args = runner_args(Path::new("/tmp/p.txt"), Some("session-1"), Some("m"), None, Some("max"));
        let effort = args.iter().position(|arg| arg == "--effort").expect("缺少 --effort");
        assert_eq!(args[effort + 1], "max");
        let resume = args.iter().position(|arg| arg == "--resume").expect("缺少 --resume");
        assert!(effort < resume, "effort 必须在 resume 之前");
        // 空 effort 表示跟随本机默认，不追加参数。
        for empty in [None, Some(""), Some("   ")] {
            assert!(
                !runner_args(Path::new("/tmp/p.txt"), None, None, None, empty)
                    .iter()
                    .any(|arg| arg == "--effort"),
                "空 effort 不应出现: {empty:?}"
            );
        }
    }

    #[test]
    fn passes_an_explicit_provider_before_effort_and_resume() {
        let args = runner_args(
            Path::new("/tmp/p.txt"),
            Some("session-1"),
            Some("deepseek-v4.1-flash"),
            Some("opendesgin"),
            Some("max"),
        );
        let position = |needle: &str| {
            args.iter()
                .position(|arg| arg == needle)
                .unwrap_or_else(|| panic!("缺少 {needle}: {args:?}"))
        };
        let model = position("--model");
        let provider = position("--provider");
        let effort = position("--effort");
        let resume = position("--resume");
        assert_eq!(args[provider + 1], "opendesgin");
        assert!(
            model < provider && provider < effort && effort < resume,
            "参数顺序必须是 --model → --provider → --effort → --resume: {args:?}"
        );
        // 空 provider 表示跟随本机渠道，不追加参数。
        for empty in [None, Some(""), Some("   ")] {
            assert!(
                !runner_args(Path::new("/tmp/p.txt"), None, None, empty, None)
                    .iter()
                    .any(|arg| arg == "--provider"),
                "空 provider 不应出现: {empty:?}"
            );
        }
    }

    #[test]
    fn parses_the_runner_model_catalog_protocol_line() {
        let payload = serde_json::json!({
            "provider": "deepseek-official",
            "provider_label": "DeepSeek",
            "model": "deepseek-v4-pro",
            "effort": "high",
            "models": [
                {
                    "provider": "deepseek-official",
                    "provider_label": "DeepSeek",
                    "id": "deepseek-v4-flash",
                    "name": "DeepSeek V4 Flash",
                    "description": "快速模型",
                    "default_effort": "high",
                    "efforts": [
                        { "id": "off", "name": "关闭" },
                        { "id": "low", "name": "低" },
                        { "id": "high", "name": "高" },
                        { "id": "max", "name": "最高" }
                    ],
                    "context_window": 1000000
                },
                {
                    "provider": "deepseek-official",
                    "provider_label": "DeepSeek",
                    "id": "deepseek-v4-pro",
                    "name": "DeepSeek V4 Pro",
                    "default_effort": "",
                    "efforts": []
                }
            ]
        });
        let stdout = format!("plugin boot log\n===AGENT_MODELS==={}\n", payload);
        let catalog = parse_models_output(&stdout).expect("parse catalog");
        assert_eq!(catalog.slot, "dsh");
        assert_eq!(catalog.backend, "dsh");
        assert_eq!(catalog.local_provider, "deepseek-official");
        assert_eq!(catalog.local_model, "deepseek-v4-pro");
        assert_eq!(
            catalog.local_model_label, "DeepSeek / DeepSeek V4 Pro",
            "v1 协议兼容：展示名 = provider 展示名 / 模型名"
        );
        assert_eq!(catalog.local_effort, "high");
        assert_eq!(catalog.models.len(), 2);
        let flash = &catalog.models[0];
        assert_eq!(flash.id, "deepseek-v4-flash");
        assert_eq!(flash.provider.as_deref(), Some("deepseek-official"));
        assert_eq!(flash.provider_label.as_deref(), Some("DeepSeek"));
        assert_eq!(flash.description.as_deref(), Some("快速模型"));
        assert_eq!(flash.default_effort, "high");
        assert_eq!(
            flash.efforts.iter().map(|effort| effort.id.as_str()).collect::<Vec<_>>(),
            vec!["off", "low", "high", "max"]
        );
        assert_eq!(flash.efforts[3].name, "最高");
        assert!(catalog.models[1].efforts.is_empty());

        // 缺协议行 / JSON 坏 / runner 报错都是明确错误。
        assert!(parse_models_output("no catalog here").is_err());
        assert!(parse_models_output("===AGENT_MODELS==={oops").is_err());
        let error_line = r#"===AGENT_MODELS==={"error":"llm service missing"}"#;
        assert!(parse_models_output(error_line).unwrap_err().contains("llm service missing"));
        // 空模型列表是合法目录（不是错误）。
        let empty = r#"===AGENT_MODELS==={"provider":"p","provider_label":"P","model":"","effort":"","models":[]}"#;
        assert!(parse_models_output(empty).unwrap().models.is_empty());
    }

    #[test]
    fn parses_the_v2_all_provider_catalog_protocol() {
        let payload = serde_json::json!({
            "local": { "provider": "opendesgin", "model": "deepseek-v4.1-flash", "effort": "" },
            "providers": [
                { "id": "deepseek-official", "label": "DeepSeek" },
                { "id": "together", "label": "together" },
                { "id": "qwen", "label": "qwen" },
                { "id": "opendesgin", "label": "opendesgin" }
            ],
            "models": [
                {
                    "provider": "opendesgin",
                    "provider_label": "opendesgin",
                    "id": "claude-opus-5",
                    "name": "Claude Opus 5",
                    "default_effort": "high",
                    "efforts": [
                        { "id": "off", "name": "Off" },
                        { "id": "low", "name": "Low" },
                        { "id": "medium", "name": "Medium" },
                        { "id": "high", "name": "High" },
                        { "id": "xhigh", "name": "XHigh" },
                        { "id": "max", "name": "Max" }
                    ],
                    "context_window": 1000000
                },
                {
                    "provider": "opendesgin",
                    "provider_label": "opendesgin",
                    "id": "deepseek-v4.1-flash",
                    "name": "DeepSeek-V4.1-Flash",
                    "default_effort": "max",
                    "efforts": []
                },
                {
                    "provider": "deepseek-official",
                    "provider_label": "DeepSeek",
                    "id": "deepseek-v4-pro",
                    "name": "DeepSeek-V4-Pro",
                    "default_effort": "high",
                    "efforts": [{ "id": "high", "name": "High" }]
                }
            ],
            "error": ""
        });
        let stdout = format!("===AGENT_MODELS==={}
", payload);
        let catalog = parse_models_output(&stdout).expect("parse v2 catalog");
        assert_eq!(catalog.slot, "dsh");
        assert_eq!(catalog.local_provider, "opendesgin");
        assert_eq!(catalog.local_model, "deepseek-v4.1-flash");
        assert_eq!(
            catalog.local_model_label, "opendesgin / DeepSeek-V4.1-Flash",
            "v2 展示名 = 渠道 / 模型"
        );
        assert_eq!(catalog.local_effort, "");
        assert_eq!(catalog.error, "");
        assert_eq!(catalog.models.len(), 3);
        let opus = &catalog.models[0];
        assert_eq!(opus.provider.as_deref(), Some("opendesgin"));
        assert_eq!(opus.provider_label.as_deref(), Some("opendesgin"));
        assert_eq!(
            opus.efforts.iter().map(|effort| effort.id.as_str()).collect::<Vec<_>>(),
            vec!["off", "low", "medium", "high", "xhigh", "max"]
        );
        // (provider, id) 唯一键：同名模型在不同渠道是不同条目。
        let deepseek_official = &catalog.models[2];
        assert_eq!(deepseek_official.provider.as_deref(), Some("deepseek-official"));
        assert_eq!(deepseek_official.provider_label.as_deref(), Some("DeepSeek"));

        // 模型条目缺 provider 时回退到 local.provider（防御旧 runner）。
        let fallback = serde_json::json!({
            "local": { "provider": "opendesgin", "model": "m1", "effort": "low" },
            "models": [{ "id": "m1", "name": "M1" }]
        });
        let catalog = parse_models_output(&format!("===AGENT_MODELS==={fallback}"))
            .expect("parse fallback catalog");
        assert_eq!(catalog.models[0].provider.as_deref(), Some("opendesgin"));
        assert_eq!(catalog.models[0].provider_label.as_deref(), Some("opendesgin"));
        assert_eq!(catalog.local_model_label, "opendesgin / M1");
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
        assert!(!patch.contains("自动同步自本机"), "没有源 profile 时不能追加同步段");

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


    /// 渠道配置同步 fixture：desktop profile 的三个托管行 + 无关行。
    const CHANNEL_PATCH_FIXTURE: &str = r#"- id: ui-settings-general
  name: '@deepseek-ai/dsh-client-ui-settings-general'
  config:
    welcomeNoticeVersion: 2026-08-13.1
- id: agent-default-model
  name: '@deepseek-ai/dsh-agent-default-model'
  config:
    provider: opendesgin
    model: deepseek-v4.1-flash
    reasoningEffort: max
- id: llm-deepseek
  name: '@deepseek-ai/dsh-llm-deepseek-api-key'
  config:
    models:
      - id: deepseek-flash
        name: DeepSeek Flash
      - id: deepseek-v4-pro
        name: deepseek-v4-pro
- id: llm-pi-ai
  name: '@deepseek-ai/dsh-llm-pi-ai'
  config:
    providers:
      qwen:
        models:
          - id: qwen3.8-flash
            name: Qwen3.8 Flash
            reasoningEfforts:
              high: high
- id: ui-chat
  name: '@deepseek-ai/dsh-client-ui-chat'
"#;

    fn write_profile_patch(home: &Path, name: &str, content: &str) {
        let dir = home.join("profiles").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(PATCH_FILENAME), content).unwrap();
    }

    #[test]
    fn syncs_only_the_three_channel_entries_and_rewrites_package_names() {
        let home = std::env::temp_dir().join(format!("dsh-sync-{}", uuid::Uuid::new_v4()));
        write_profile_patch(&home, "desktop", CHANNEL_PATCH_FIXTURE);
        let section = channel_sync_section(&home).expect("必须生成渠道同步段");
        assert!(section.contains("自动同步自本机 DSH profile: desktop"), "{section}");
        // 只允许这三个 id，别的条目不能带进来。
        for forbidden in ["ui-settings-general", "ui-chat", "welcomeNoticeVersion"] {
            assert!(!section.contains(forbidden), "不该同步 {forbidden}：{section}");
        }
        for expected in ["llm-pi-ai", "llm-deepseek", "agent-default-model"] {
            assert!(section.contains(expected), "缺少渠道行 {expected}: {section}");
        }
        // name 改写成 CLI 可加载的 base 包名。
        assert!(section.contains("name: '@deepseek-ai/dsh-llm-deepseek'"));
        assert!(!section.contains("dsh-llm-deepseek-api-key"), "app 专用包名必须改写");
        assert!(section.contains("name: '@deepseek-ai/dsh-llm-pi-ai'"));
        assert!(section.contains("name: '@deepseek-ai/dsh-agent-default-model'"));
        // config 原文与多行缩进原样保留，config 里的模型 name 不能被插件包名覆盖。
        assert!(section.contains("    provider: opendesgin"));
        assert!(section.contains("              high: high"));
        assert!(section.contains("        name: DeepSeek Flash"), "{section}");
        assert!(section.contains("            name: Qwen3.8 Flash"), "{section}");
        assert_eq!(
            section.matches("name: '@deepseek-ai/dsh-llm-pi-ai'").count(),
            1,
            "只改写插件行 name，不能改模型 name: {section}"
        );
        assert_eq!(
            section.matches("name: '@deepseek-ai/dsh-llm-deepseek'").count(),
            1,
            "只改写插件行 name: {section}"
        );
        // 顺序沿用源文件：agent-default-model → llm-deepseek → llm-pi-ai。
        let index_of = |needle: &str| section.find(needle).unwrap_or_else(|| panic!("缺少 {needle}"));
        assert!(index_of("agent-default-model") < index_of("llm-deepseek"));
        assert!(index_of("llm-deepseek") < index_of("llm-pi-ai"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn channel_sync_respects_settings_yaml_source_priority_and_comments() {
        let home = std::env::temp_dir().join(format!("dsh-sync-{}", uuid::Uuid::new_v4()));
        // desktop 没有渠道行 → 跳过，按优先级选 web（headless 虽也有但优先级更低）。
        write_profile_patch(
            &home,
            "desktop",
            "- id: ui-chat\n  name: '@deepseek-ai/dsh-client-ui-chat'\n",
        );
        write_profile_patch(&home, "web", CHANNEL_PATCH_FIXTURE);
        write_profile_patch(&home, "headless", CHANNEL_PATCH_FIXTURE);
        let section = channel_sync_section(&home).expect("web 应作为同步源");
        assert!(section.contains("profile: web"), "{section}");
        // settings.yaml 顶层已有渠道段 → 不同步（避免双份）。
        std::fs::write(
            home.join("settings.yaml"),
            "# llm-pi-ai:\nllm-deepseek:\n  models: []\n",
        )
        .unwrap();
        assert!(
            channel_sync_section(&home).is_none(),
            "settings.yaml 已配置渠道时不能再复制"
        );
        // 注释掉的段、缩进行、同前缀的其它 key 都不算渠道段。
        std::fs::write(
            home.join("settings.yaml"),
            "# 只是注释\nllm-pi-ai-extra: true\n  llm-pi-ai: nested\n",
        )
        .unwrap();
        let section = channel_sync_section(&home).expect("非顶层 / 注释行不算渠道段");
        assert!(section.contains("profile: web"));
        // 无源 profile 时不生成同步段。
        let empty_home = std::env::temp_dir().join(format!("dsh-sync-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(empty_home.join("profiles")).unwrap();
        assert!(channel_sync_section(&empty_home).is_none());
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(empty_home);
    }

    #[test]
    fn ensure_profile_in_merges_and_refreshes_synced_channels() {
        let home = std::env::temp_dir().join(format!("dsh-sync-profile-{}", uuid::Uuid::new_v4()));
        write_profile_patch(&home, "desktop", CHANNEL_PATCH_FIXTURE);
        let dir = ensure_profile_in(&home).unwrap();
        let patch = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(patch.contains(PATCH_MARKER), "静态层必须保留");
        assert!(patch.contains("自动同步自本机 DSH profile: desktop"));
        assert!(patch.contains("id: llm-pi-ai"));
        assert!(patch.contains("name: '@deepseek-ai/dsh-llm-deepseek'"));
        assert!(!patch.contains("dsh-llm-deepseek-api-key"));
        // 幂等：再跑一次内容不变。
        ensure_profile_in(&home).unwrap();
        let again = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert_eq!(patch, again);
        // 源文件变化后下一次 ensure_profile 自动重建。
        let updated = CHANNEL_PATCH_FIXTURE.replace("reasoningEffort: max", "reasoningEffort: high");
        write_profile_patch(&home, "desktop", &updated);
        ensure_profile_in(&home).unwrap();
        let refreshed = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(refreshed.contains("reasoningEffort: high"));
        // settings.yaml 出现渠道段后托管段移除同步内容。
        std::fs::write(home.join("settings.yaml"), "llm-pi-ai:\n  providers: {}\n").unwrap();
        ensure_profile_in(&home).unwrap();
        let without = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(
            !without.contains("自动同步自本机"),
            "settings.yaml 生效后不应再同步渠道配置: {without}"
        );
        assert!(without.contains(PATCH_MARKER));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn ensure_profile_in_without_a_channel_source_keeps_the_static_patch() {
        let home = std::env::temp_dir().join(format!("dsh-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("profiles")).unwrap();
        let dir = ensure_profile_in(&home).unwrap();
        let patch = std::fs::read_to_string(dir.join(PATCH_FILENAME)).unwrap();
        assert!(patch.contains(PATCH_MARKER));
        assert!(!patch.contains("自动同步自本机"), "没有源 profile 时不能追加同步段");
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

    /// 检查现有本机账号的模型目录，不发送模型对话请求。
    #[tokio::test]
    #[ignore = "reads the installed DSH profile and its configured provider model directory"]
    async fn local_cli_current_profile_model_catalog() {
        let catalog = model_catalog().await.expect("读取本机 DSH 模型目录");
        assert!(!catalog.local_provider.is_empty(), "本机默认渠道不能为空");
        assert!(!catalog.local_model.is_empty(), "本机默认模型不能为空");
        assert!(catalog.models.iter().any(|model| model.provider.as_deref() == Some(catalog.local_provider.as_str())
            && model.id == catalog.local_model), "本机默认模型必须可以由当前登录配置使用：{} / {}", catalog.local_provider, catalog.local_model);
        println!("DSH 本机默认渠道 {}，模型 {}；可用模型 {} 个", catalog.local_provider, catalog.local_model, catalog.models.len());
    }

    /// 离线端到端验证：本地假模型 + 工作区内的 DSH 主目录，不碰用户凭据。
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

        // 2) 使用机器级补丁配置假模型，兼容新版已移除 settings.yaml 的 DSH。
        std::fs::write(
            home.join("cordis.patch.yml"),
            format!(
                "- id: agent-default-model\n  config:\n    provider: pet-mock\n    model: pet-mock-model\n- id: llm-pi-ai\n  config:\n    providers:\n      pet-mock:\n        displayName: Pet Mock\n        apiKeyEnv: PET_MOCK_API_KEY\n        api: openai-completions\n        baseURL: http://127.0.0.1:{port}/v1\n        compat:\n          thinkingFormat: deepseek\n          maxTokensField: max_tokens\n        models:\n          - id: pet-mock-model\n            name: Pet Mock Model\n            contextWindow: 200000\n            maxTokens: 8192\n"
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

        // 第四轮：真实 DSH 工具注册 / 参数校验 / 执行 → 本地记忆桥 → 共享 SQLite。
        struct MemoryFixture(PathBuf);
        #[async_trait]
        impl crate::mcp_permission::PermissionAnswerer for MemoryFixture {
            async fn decide(&self, _: crate::mcp_permission::PermissionRequest, _: &str) -> crate::mcp_permission::PermissionDecision {
                crate::mcp_permission::PermissionDecision::Deny("test does not approve destructive calls".into())
            }
            fn memory_db_path(&self) -> Option<PathBuf> { Some(self.0.clone()) }
        }
        let memory_path = root.join("memory.db");
        let memory_db = crate::storage::Database::new(&memory_path).unwrap();
        let bridge = crate::mcp_permission::serve(std::sync::Arc::new(MemoryFixture(memory_path))).await.unwrap();
        let memory_url = bridge.url_for_session("dsh-memory-test");
        let mut child = adapter.spawn_in_workspace_with(&workspace, &prompts,
            "USE_MEMORY 请记住我使用中文。", &[], system, None, None, None, None,
            crate::cli_permission::DshPermission::default(), Some(&memory_url)).await.unwrap();
        let fourth = tokio::time::timeout(Duration::from_secs(45),
            read_stream_with_activity(&mut child, |_| {}, |_| {}, |_| {}, None)).await.unwrap().unwrap();
        assert!(!fourth.is_error, "memory turn failed: {:?}", fourth.error_msg);
        assert!(fourth.full_text.contains("记忆已保存"), "memory result must return to model: {}", fourth.full_text);
        assert!(memory_db.get_all_memories().unwrap().iter().any(|m| m.key == "mock-language" && m.value == "中文"));
        drop(memory_db);

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


    // 需要本机安装并登录 dsh，且本机已配置多渠道（desktop profile 的渠道行会被同步）：
    // cargo test --lib dsh::tests::local_model_catalog_lists_real_models -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a locally installed DeepSeek Harness CLI"]
    async fn local_model_catalog_lists_real_models() {
        let catalog = model_catalog().await.expect("读取本机 dsh 模型目录");
        assert_eq!(catalog.slot, "dsh");
        assert_eq!(catalog.backend, "dsh");
        assert!(catalog.error.is_empty(), "目录错误：{}", catalog.error);
        assert!(!catalog.models.is_empty(), "本机 dsh 目录不能为空");

        // v2：本机已配置的全部渠道都要出现在目录里，且每个模型都带 provider。
        let providers: Vec<&str> = catalog
            .models
            .iter()
            .filter_map(|model| model.provider.as_deref())
            .collect();
        for provider in ["deepseek-official", "together", "qwen", "opendesgin"] {
            assert!(
                providers.contains(&provider),
                "目录缺少渠道 {provider}: {providers:?}"
            );
        }
        assert!(
            catalog.models.iter().all(|model| model.provider.is_some()
                && model.provider_label.as_deref().is_some_and(|label| !label.is_empty())),
            "dsh 的每个模型都必须带 provider / provider_label"
        );

        // 本机默认渠道 / 模型来自 agent-default-model 配置。
        assert_eq!(catalog.local_provider, "opendesgin", "本机默认渠道");
        assert_eq!(catalog.local_model, "deepseek-v4.1-flash", "本机默认模型");
        assert!(
            catalog.local_model_label.contains("opendesgin"),
            "本机展示名要带渠道：{}",
            catalog.local_model_label
        );

        // opendesgin 渠道的 claude-opus-5 及完整思考强度。
        let opus = catalog
            .models
            .iter()
            .find(|model| {
                model.provider.as_deref() == Some("opendesgin") && model.id == "claude-opus-5"
            })
            .unwrap_or_else(|| {
                panic!(
                    "opendesgin 渠道缺少 claude-opus-5：{:?}",
                    catalog
                        .models
                        .iter()
                        .filter(|model| model.provider.as_deref() == Some("opendesgin"))
                        .map(|model| model.id.clone())
                        .collect::<Vec<_>>()
                )
            });
        let efforts: Vec<&str> = opus.efforts.iter().map(|effort| effort.id.as_str()).collect();
        for effort in ["off", "low", "medium", "high", "xhigh", "max"] {
            assert!(
                efforts.contains(&effort),
                "claude-opus-5 缺少思考强度 {effort}: {efforts:?}"
            );
        }

        println!(
            "dsh 目录：local={}@{} ({}) effort={:?}，渠道 {:?}，模型 {} 个",
            catalog.local_model,
            catalog.local_provider,
            catalog.local_model_label,
            catalog.local_effort,
            catalog
                .models
                .iter()
                .filter_map(|model| model.provider.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            catalog.models.len()
        );
        println!(
            "opendesgin claude-opus-5 efforts={:?} default={:?}",
            efforts, opus.default_effort
        );
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
