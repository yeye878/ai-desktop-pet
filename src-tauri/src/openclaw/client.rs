use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// PATH 上的全局 Claude CLI：内置资源找不到时的回退。
const PATH_CLAUDE_CMD: &str = "claude.cmd";
/// 内置 node 目录相对 `<resource_dir>` 的候选路径；Tauri 的打包布局不同，逐个尝试。
const BUNDLED_NODE_DIRS: [&str; 4] = ["node", "_up_/resources/node", "_up/resources/node", "resources/node"];
/// npm 包里的原生 CLI：优先 spawn 它，绕开 `.cmd` 的 8191 命令行长度上限与转义问题。
const NATIVE_CLAUDE_RELATIVE: &str = "node_modules/@anthropic-ai/claude-code/bin/claude.exe";
/// `--help` 能力探测的超时；超时按“不支持新 flag”处理，避免阻塞 spawn。
const CLI_CAPS_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct AgentResponse {
    pub text: String,
    pub thinking: Option<String>,
    pub emotion: Option<String>,
}

// ===== Claude Code 适配器（通过 CLI 子进程）=====
pub const DEFAULT_PERSONALITY: &str = "gentle";
pub const DEFAULT_PROFESSION: &str = "companion";

/// Claude Code 系统提示词落盘目录名（位于应用数据目录下）。
const PROMPT_DIR_NAME: &str = "claude-prompts";
/// 落盘提示词文件的保留时长；每次 spawn 顺手清理超过 1 小时的旧文件。
const PROMPT_FILE_TTL: Duration = Duration::from_secs(60 * 60);

const BASE_SYSTEM_PROMPT: &str = "你是一个常驻在用户桌面上的 AI 桌宠。你既是陪伴者，也是一个能帮用户处理任务的小助手。你要保持桌宠的亲近感，但不要装作真实动物或真实人类；你可以表达轻松、关心和一点点俏皮。默认回复简短自然，通常 2-3 句话即可；遇到复杂任务、代码、文件处理、学习规划时，可以使用清晰的小段落和列表。不要频繁使用表情符号，每次最多 0-1 个。";

fn personality_prompt(personality: &str) -> &'static str {
    match personality {
        "energetic" => {
            "当前性格：活泼元气。你说话轻快、有行动感，喜欢用积极的短句推动用户开始下一步。你可以适度俏皮，但不要打断用户的严肃工作；当用户焦虑或卡住时，先给一个轻松的安定回应，再给一个可以立刻执行的小步骤。"
        }
        "calm" => {
            "当前性格：冷静专业。你说话克制、清晰、低情绪化，优先给判断、结论和可执行步骤。你不使用夸张语气，不强行卖萌；当信息不足时，用一两个关键问题补齐上下文。"
        }
        "mentor" => {
            "当前性格：严谨导师。你重视准确性、结构和反馈质量。你会指出问题背后的原因，帮助用户建立方法，而不是只给答案。提出批评时保持尊重，先说明风险或改进点，再给具体修正建议。"
        }
        "witty" => {
            "当前性格：毒舌吐槽。你嘴很利、反应快、擅长用夸张比喻和犀利短句吐槽问题本身，风格像一个嘴硬但靠谱的桌面搭子。你可以狠狠吐槽混乱的代码、离谱的需求、拖延、烂命名、重复劳动和不讲道理的报错，但不要羞辱用户的人格、能力、外貌、身份或现实处境。吐槽要有梗、有节奏，先给一句好笑的锐评，再给真正能解决问题的建议；当用户明显难过、焦虑或求安慰时，降低火力，改成嘴硬式关心。"
        }
        _ => {
            "当前性格：温柔陪伴。你语气柔和、耐心、稳定，擅长让用户感觉事情可以慢慢处理。你会先接住用户的情绪，再给简洁建议；不过度说教，不把普通问题夸大，也不制造焦虑。"
        }
    }
}

fn profession_prompt(profession: &str) -> &'static str {
    match profession {
        "coding" => {
            "当前职业：编程助手。你擅长阅读代码、定位报错、解释架构、设计改动方案和给出测试建议。处理代码问题时，优先关注复现路径、错误信息、数据流、边界条件和最小改动。不要凭空假设代码内容；如果用户给了文件路径或项目上下文，要围绕现有代码风格行动。"
        }
        "study" => {
            "当前职业：学习教练。你擅长把知识点拆成层级、设计学习计划、用提问检查理解、把抽象概念讲成例子。回答时尽量先给框架，再给关键步骤；如果用户像是在备考或赶作业，优先帮他确定范围、节奏和可交付结果。"
        }
        "writing" => {
            "当前职业：写作编辑。你擅长润色、改写、提纲、标题、摘要、语气调整和逻辑顺序优化。编辑文本时尽量保留用户原意和个人表达，只修掉冗余、含混和结构问题；必要时提供一个更自然的版本和简短修改说明。"
        }
        "file_intake" => {
            "当前职业：文件整理员。你擅长处理用户拖入的本地文件路径、归纳文件用途、建议命名规则、分类目录和后续处理流程。看到文件路径时，先确认用户目标，再建议分类维度；如果需要读取文件内容，应明确需要通过 Claude Code 或相关工具执行读取。不要声称已经读取了没有实际读取的文件。"
        }
        "secretary" => {
            "当前职业：效率秘书。你擅长拆解待办、安排优先级、整理会议/日程信息、生成提醒文案和把混乱想法变成行动清单。回答时优先给最小下一步、时间顺序和风险提醒；不要把计划做得过重。"
        }
        _ => {
            "当前职业：日常陪伴。你擅长轻量聊天、鼓励、简单建议、桌面工作陪伴和把用户零散想法整理成清楚的话。默认不要过度展开，除非用户明确要求深入；当用户只是闲聊时，保持自然和简短。"
        }
    }
}

pub fn normalize_personality_id(personality: &str) -> String {
    match personality {
        "gentle" | "energetic" | "calm" | "mentor" | "witty" => personality.to_string(),
        _ => DEFAULT_PERSONALITY.to_string(),
    }
}

pub fn normalize_profession_id(profession: &str) -> String {
    match profession {
        "companion" | "coding" | "study" | "writing" | "file_intake" | "secretary" => {
            profession.to_string()
        }
        _ => DEFAULT_PROFESSION.to_string(),
    }
}

pub fn build_system_prompt(personality: &str, profession: &str) -> String {
    format!(
        "{}\n\n{}\n\n{}\n\n回复约束：优先使用中文回复。不要在每次回复里解释你的设定，除非用户询问。性格决定语气，职业决定工作方法；两者冲突时，以用户当前任务的有效性和安全性优先。",
        BASE_SYSTEM_PROMPT,
        personality_prompt(personality),
        profession_prompt(profession)
    )
}

/// 本回合的模型 / 思考强度 / 续接参数；空串表示跟随本机配置（不追加）。
pub(crate) fn claude_flags(
    model: Option<&str>,
    effort: Option<&str>,
    session: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    if let Some(effort) = effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    if let Some(id) = session.map(str::trim).filter(|id| !id.is_empty()) {
        args.extend(["--resume".to_string(), id.to_string()]);
    }
    args
}

/// 本回合的权限模式参数；空串 / 未知值表示跟随本机 Claude Code 配置。
fn claude_permission_args(value: &str) -> Vec<String> {
    match crate::cli_permission::claude_permission_mode(value) {
        // `manual` 只在较新 CLI 里存在，且是 `default` 的别名；旧版（2.1.158）的
        // 合法值里只有 `default`，直接传 `manual` 会让 CLI 以非法参数启动失败。
        Some("manual") => vec!["--permission-mode".to_string(), "default".to_string()],
        Some(mode) => vec!["--permission-mode".to_string(), mode.to_string()],
        None => Vec::new(),
    }
}

/// 实际 spawn 的 Claude CLI。`.cmd` 场景需要把 bundled node 目录注入子进程 PATH
/// （shim 内部还要调用 node.exe）；原生 exe 不需要，`node_dir` 为 `None`。
#[derive(Debug, PartialEq, Eq)]
struct ClaudeCli {
    program: PathBuf,
    node_dir: Option<PathBuf>,
}

/// 在 `<resource_dir>` 下按候选顺序查找内置 CLI：同一个 node 目录里优先原生 exe
/// （不受 `.cmd` 命令行上限与转义影响），其次 `claude.cmd`。
fn bundled_claude_cli(resource_dir: &Path) -> Option<ClaudeCli> {
    for relative in BUNDLED_NODE_DIRS {
        let node_dir = resource_dir.join(relative);
        let native = node_dir.join(NATIVE_CLAUDE_RELATIVE);
        if native.is_file() {
            return Some(ClaudeCli {
                program: native,
                node_dir: None,
            });
        }
        let cmd = node_dir.join(PATH_CLAUDE_CMD);
        if cmd.is_file() {
            return Some(ClaudeCli {
                program: cmd,
                node_dir: Some(node_dir),
            });
        }
    }
    None
}

/// 解析 spawn 用的 CLI：内置优先，找不到回退 PATH 上的 `claude.cmd`。
fn resolve_claude_cli(resource_dir: Option<&Path>) -> ClaudeCli {
    resource_dir
        .and_then(bundled_claude_cli)
        .unwrap_or_else(|| ClaudeCli {
            program: PathBuf::from(PATH_CLAUDE_CMD),
            node_dir: None,
        })
}

/// 每个 CLI 路径的 `--permission-prompts` 支持情况：`--help` 只探测一次，之后走缓存。
static PERMISSION_PROMPTS_CAPS: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();

/// 该 CLI 是否支持 `--permission-prompts`（旧版 2.1.158 没有这个 flag，传了会启动失败）。
fn cli_supports_permission_prompts(cli: &Path) -> bool {
    let cache = PERMISSION_PROMPTS_CAPS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(supported) = guard.get(cli) {
            return *supported;
        }
    }
    // 缓存未命中时才跑一次 `--help`；探测结果（包括失败）也缓存，避免每轮 spawn 都跑。
    let supported = probe_cli_supports_permission_prompts(cli);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(cli.to_path_buf(), supported);
    }
    supported
}

/// 跑一次 `<cli> --help`，stdout + stderr 里出现该 flag 才算支持。
/// spawn 失败 / 超时 / 读输出失败都按不支持处理（宁可不传，也不让旧版 CLI 启动失败）。
fn probe_cli_supports_permission_prompts(cli: &Path) -> bool {
    let mut command = std::process::Command::new(cli);
    command
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_pipe(stdout));
    let stderr_reader = std::thread::spawn(move || read_pipe(stderr));

    let deadline = std::time::Instant::now() + CLI_CAPS_PROBE_TIMEOUT;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => {
                timed_out = true;
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if timed_out {
        let _ = child.kill();
        let _ = child.wait();
        // 读线程可能被继承了管道的后代进程拖住；超时分支不再等它们（直接丢弃句柄）。
        return false;
    }
    let _ = child.wait();
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    help_mentions_permission_prompts(&stdout) || help_mentions_permission_prompts(&stderr)
}

/// 把子进程管道里的输出读到字符串；读失败按空串处理。
fn read_pipe<R: std::io::Read>(pipe: Option<R>) -> String {
    let mut output = String::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_string(&mut output);
    }
    output
}

/// 帮助文本里是否列出 `--permission-prompts`（旧版 CLI 的帮助里没有这一行）。
fn help_mentions_permission_prompts(help: &str) -> bool {
    help.contains("--permission-prompts")
}

/// Claude Code 系统提示词落盘目录：`<app_data>/claude-prompts`。
fn system_prompt_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_data_dir()
        .map(|dir| dir.join(PROMPT_DIR_NAME))
        .map_err(|error| error.to_string())
}

/// 把系统提示词原样写成 UTF-8 文件（保留换行），返回文件路径。
/// 走文件而不是把整段提示词内联进命令行：内联会受命令行长度上限约束，
/// 且历史上为了塞进参数把换行压平，导致提示词里的分段结构丢失。
fn write_system_prompt_file(dir: &Path, system_prompt: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("无法创建 {}：{error}", dir.display()))?;
    sweep_stale_prompt_files(dir);
    let path = dir.join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, system_prompt)
        .map_err(|error| format!("无法写入提示词文件 {}：{error}", path.display()))?;
    Ok(path)
}

/// 清理被中断的轮次遗留的提示词文件（尽力而为，失败不影响本轮）。
fn sweep_stale_prompt_files(dir: &Path) {
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
            .is_some_and(|age| age > PROMPT_FILE_TTL);
        if stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub struct ClaudeAdapter {
    session_id: Mutex<Option<String>>,
}

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self {
            session_id: Mutex::new(None),
        }
    }

    /// 启动 Claude CLI 子进程（stream-json 模式），返回 Child 供调用方逐行读取。
    /// 旧签名：会话 id 取自适配器内部状态。
    pub async fn spawn_streaming(
        &self,
        app_handle: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session_id.lock().ok().and_then(|id| id.clone());
        self.spawn_streaming_with(
            app_handle,
            message,
            system_prompt,
            session.as_deref(),
            None,
            None,
            "",
        )
        .await
    }

    /// 显式会话 / 模型 / 思考强度 / 权限模式版本：不依赖适配器内部状态，供多会话与协作调用。
    /// session 为 Some 时追加 `--resume`，model / effort 非空时追加覆盖参数，
    /// permission 为空串时跟随本机 Claude Code 配置（不传 `--permission-mode`）。
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_streaming_with(
        &self,
        app_handle: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
        session: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        permission: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        // 内置 CLI 优先（打包资源分布在 `_up_/resources/node` 等布局下），
        // 找不到才回退 PATH 上的 `claude.cmd`。原生 exe 不需要 bundled node。
        let cli = resolve_claude_cli(app_handle.path().resource_dir().ok().as_deref());
        let mut cmd = Command::new(&cli.program);

        // `.cmd` 内部还要调用 node.exe：把 bundled node 目录临时注入子进程 PATH，
        // 这样 shim 才能找到同目录下的便携版 node；原生 exe 则不需要注入。
        if let Some(node_dir) = cli.node_dir.as_ref() {
            if let Some(current_path) = std::env::var_os("PATH") {
                let mut new_path = node_dir.clone().into_os_string();
                new_path.push(";");
                new_path.push(current_path);
                cmd.env("PATH", new_path);
            }
        }

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        // 系统提示词落盘为 UTF-8 文件，再用 `--append-system-prompt-file` 传入：
        // 保留原始换行 / 段落结构，也避开命令行长度上限。写文件失败时退回旧的
        // inline 参数（当时的做法是压平换行），保证 spawn 不会因为写盘失败整体失败。
        let prompt_file = system_prompt_dir(app_handle)
            .ok()
            .and_then(|dir| write_system_prompt_file(&dir, system_prompt).ok());
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages");
        match prompt_file {
            Some(path) => {
                cmd.arg("--append-system-prompt-file").arg(path);
            }
            None => {
                // 回退旧行为：内联参数 + 压平换行（结构信息会丢，但保证可用）。
                cmd.arg("--append-system-prompt")
                    .arg(system_prompt.replace(['\r', '\n'], " "));
            }
        }
        // 不限制本回合花费：Claude Code 对第三方中转/未知模型按内置 Anthropic
        // 价目估算成本，与真实账单差异很大，用估算值做预算只会误杀正常任务。

        // 需要授权的工具调用交给桌宠的 MCP 工具决定，而不是被自动拒绝。
        // 优先写带会话后缀的配置，让桥把授权弹窗归属到本回合会话；
        // 桥没起来（或配置没写成）时退回全局配置，保持 Claude 自己的默认行为。
        let state = app_handle.state::<crate::AppState>();
        let turn_session = crate::current_turn_session(&state);
        let session_config = if turn_session.trim().is_empty() {
            None
        } else {
            state
                .permission_bridge
                .lock()
                .ok()
                .and_then(|bridge| bridge.clone())
                .and_then(|bridge| {
                    crate::mcp_permission::write_session_config(app_handle, &bridge, &turn_session)
                        .ok()
                })
        };
        let config = session_config.or_else(|| crate::mcp_permission::config_path(app_handle).ok());
        if let Some(config) = config {
            if config.is_file() {
                cmd.arg("--mcp-config")
                    .arg(&config)
                    .arg("--permission-prompt-tool")
                    .arg(crate::mcp_permission::PROMPT_TOOL);
                // `--permission-prompts` 只有较新的 CLI 支持；旧版没有该 flag，
                // 传未知参数会导致启动失败。按 CLI 路径探测一次并缓存，仅在支持时追加。
                if cli_supports_permission_prompts(&cli.program) {
                    cmd.arg("--permission-prompts").arg("host");
                }
            }
        }

        // 会话级 / 设置页选定的权限模式：空串时沿用 Claude Code 自己的配置。
        // 显式档位（auto / manual / bypassPermissions 等）让桌宠能直接控制
        // 每次工具调用是否需要确认。
        cmd.args(claude_permission_args(permission));

        // 本回合模型 / 思考强度覆盖（空 = 跟随本机配置）与多轮续接。
        cmd.args(claude_flags(model, effort, session));

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().ok_or("无法打开 Claude 标准输入")?;
        let written = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            stdin.write_all(message.as_bytes()).await?;
            stdin.shutdown().await
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            if let Some(pid) = child.id() {
                crate::kill_process_tree(pid);
            }
            let _ = child.wait().await;
            written??;
        }
        drop(stdin);
        Ok(child)
    }

    pub fn update_session(&self, session_id: &str) {
        if let Ok(mut lock) = self.session_id.lock() {
            *lock = Some(session_id.to_string());
        }
    }

    pub fn reset_session(&self) {
        if let Ok(mut lock) = self.session_id.lock() {
            *lock = None;
        }
    }
}

/// 从一行 stream-json NDJSON 中提取思考内容和回复文本
pub struct StreamParseState {
    pub thinking_len: usize,
    pub text_len: usize,
    pub full_thinking: String,
    pub full_text: String,
    pub session_id: Option<String>,
    /// 是否收到过 Claude Code 的最终 `result` 事件（用于流完整性校验）。
    pub completed: bool,
    pub is_error: bool,
    pub error_msg: Option<String>,
}

impl StreamParseState {
    pub fn new() -> Self {
        Self {
            thinking_len: 0,
            text_len: 0,
            full_thinking: String::new(),
            full_text: String::new(),
            session_id: None,
            completed: false,
            is_error: false,
            error_msg: None,
        }
    }

    /// 解析一行 NDJSON，返回新增的思考内容 delta（如有）
    pub fn parse_line(&mut self, line: &str) -> Option<String> {
        let data: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };

        let msg_type = data["type"].as_str().unwrap_or("");

        match msg_type {
            "stream_event" => {
                let delta = &data["event"]["delta"];
                match delta["type"].as_str() {
                    Some("thinking_delta") => {
                        if let Some(thinking) = delta["thinking"].as_str() {
                            self.full_thinking.push_str(thinking);
                            self.thinking_len = self.full_thinking.len();
                            return Some(thinking.to_string());
                        }
                    }
                    Some("text_delta") => {
                        if let Some(text) = delta["text"].as_str() {
                            self.full_text.push_str(text);
                            self.text_len = self.full_text.len();
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                // 从 message.content 数组提取 thinking / text
                if let Some(content) = data["message"]["content"].as_array() {
                    let mut thinking_delta = None;
                    for block in content {
                        match block["type"].as_str() {
                            Some("thinking") => {
                                if let Some(thinking) = block["thinking"].as_str() {
                                    if thinking.len() > self.thinking_len {
                                        let delta = if thinking.is_char_boundary(self.thinking_len)
                                        {
                                            thinking[self.thinking_len..].to_string()
                                        } else {
                                            let mut start = self.thinking_len;
                                            while start < thinking.len()
                                                && !thinking.is_char_boundary(start)
                                            {
                                                start += 1;
                                            }
                                            thinking[start..].to_string()
                                        };
                                        self.thinking_len = thinking.len();
                                        self.full_thinking = thinking.to_string();
                                        thinking_delta = Some(delta);
                                    }
                                }
                            }
                            Some("text") => {
                                if let Some(text) = block["text"].as_str() {
                                    if text.len() > self.text_len {
                                        self.full_text = text.to_string();
                                        self.text_len = text.len();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    return thinking_delta;
                }
            }
            "result" => {
                // 收到最终 result 即视为本回合正常收尾；流完整性校验依赖这个标记。
                self.completed = true;
                if let Some(result) = data["result"].as_str() {
                    self.full_text = result.to_string();
                }
                if let Some(sid) = data["session_id"].as_str() {
                    self.session_id = Some(sid.to_string());
                }
                if data["is_error"].as_bool().unwrap_or(false) {
                    self.is_error = true;
                    self.error_msg = data["errors"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .or_else(|| data["result"].as_str().map(|s| s.to_string()));
                }
            }
            _ => {}
        }

        None
    }
}

/// 取 UTF-8 安全尾部：超过 `max_chars` 个字符时只保留最后 `max_chars` 个字符。
fn tail_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

/// 流结束后补齐状态：Claude Code 的 stream-json 以一条 `result` 事件收尾，
/// 没等到它就意味着进程被中断、崩溃或异常退出，需要标记成错误，而不是静默返回空回复。
/// 已有错误优先保留；同时保留“stderr 非空且正文为空”的旧错误兜底语义。
fn finalize_stream(state: &mut StreamParseState, stderr: &str) {
    if state.is_error {
        return;
    }
    let stderr = stderr.trim();
    if !state.completed {
        let mut message = String::from("未收到最终 result，Claude Code 可能被中断或异常退出");
        if !stderr.is_empty() {
            message.push_str("；stderr（尾部）：");
            message.push_str(&tail_chars(stderr, 500));
        }
        state.is_error = true;
        state.error_msg = Some(message);
        return;
    }
    // 流正常收尾但 stderr 有输出、正文为空：沿用原有的错误兜底。
    if !stderr.is_empty() && state.full_text.is_empty() {
        state.is_error = true;
        state.error_msg = Some(stderr.to_string());
    }
}

/// 从 Child 的 stdout 逐行读取 stream-json，通过回调推送思考 delta
pub async fn read_stream_with_activity<F, T, E>(
    child: &mut Child,
    mut on_thinking_delta: F,
    mut on_text: T,
    mut on_tool: E,
) -> Result<StreamParseState, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(&str),
    T: FnMut(&str),
    E: FnMut(&crate::cli_activity::ToolEvent),
{
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture stdout from claude process")?;
    let stderr = child.stderr.take();
    let stderr_task = stderr.map(|mut stderr| {
        tokio::spawn(async move {
            let mut output = String::new();
            let _ = stderr.read_to_string(&mut output).await;
            output
        })
    });

    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut state = StreamParseState::new();
    let mut activity = crate::cli_activity::CliActivityParser::default();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(delta) = state.parse_line(&line) {
            on_thinking_delta(&delta);
        }
        for event in activity.claude(&line) {
            match event {
                crate::cli_activity::Activity::Text(text) => on_text(&text),
                crate::cli_activity::Activity::Tool(tool) => on_tool(&tool),
            }
        }
    }

    let stderr_output = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    finalize_stream(&mut state, &stderr_output);

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_flags_carry_model_effort_and_resume() {
        let flags = claude_flags(Some("opus"), Some("high"), Some("session-1"));
        assert!(flags.windows(2).any(|pair| pair == ["--model", "opus"]));
        assert!(
            flags.windows(2).any(|pair| pair == ["--effort", "high"]),
            "思考强度必须作为 --effort 传给 claude: {flags:?}"
        );
        assert!(flags.windows(2).any(|pair| pair == ["--resume", "session-1"]));

        // 空值表示跟随本机配置，一个参数都不追加。
        assert!(claude_flags(None, None, None).is_empty());
        assert!(claude_flags(Some("  "), Some(""), Some(" ")).is_empty());

        // effort 与 --model 属于同一组本回合参数，顺序稳定。
        assert_eq!(
            claude_flags(Some("sonnet"), Some("max"), None),
            vec!["--model", "sonnet", "--effort", "max"]
        );
        // 只改 effort 时不影响续接参数。
        assert_eq!(
            claude_flags(None, Some("low"), Some("s")),
            vec!["--effort", "low", "--resume", "s"]
        );
    }

    #[test]
    fn claude_permission_mode_is_forwarded_only_when_explicit() {
        assert_eq!(
            claude_permission_args("auto"),
            vec!["--permission-mode", "auto"]
        );
        assert_eq!(
            claude_permission_args("bypassPermissions"),
            vec!["--permission-mode", "bypassPermissions"]
        );
        // `manual` 是旧版 CLI 不认识的档位（新版也只是 `default` 的别名）：
        // 统一映射成两版都合法的 `default`。
        assert_eq!(
            claude_permission_args("manual"),
            vec!["--permission-mode", "default"]
        );
        assert_eq!(
            claude_permission_args("  manual "),
            vec!["--permission-mode", "default"]
        );
        // 空值 / 未知值表示跟随本机配置，不追加参数。
        for follow in ["", "  ", "default"] {
            assert!(claude_permission_args(follow).is_empty(), "{follow:?}");
        }
    }

    #[test]
    fn permission_prompts_capability_is_detected_from_help_text() {
        // 新版 CLI 的帮助里列出该 flag；旧版（2.1.158）只有 --permission-prompt-tool。
        let new_help = "--permission-prompt-tool <tool>  委托授权给 MCP 工具\n  --permission-prompts <mode>";
        assert!(help_mentions_permission_prompts(new_help));
        let old_help = "--permission-prompt-tool <tool>\n  --mcp-config <configs...>";
        assert!(!help_mentions_permission_prompts(old_help));
    }

    #[test]
    fn bundled_cli_hits_up_resources_native_exe() {
        // Tauri 把 `../resources` 打成 `_up_/resources`，旧布局的 `<resource_dir>/node`
        // 实际不存在；同一 node 目录下应优先命中原生 exe。
        let root = std::env::temp_dir().join(format!("pet-claude-cli-{}", uuid::Uuid::new_v4()));
        let node_dir = root.join("_up_").join("resources").join("node");
        let native = node_dir
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin")
            .join("claude.exe");
        std::fs::create_dir_all(native.parent().expect("claude.exe 父目录"))
            .expect("创建原生 exe 目录");
        std::fs::write(&native, b"").expect("写入原生 exe");
        // 同一目录同时放一个 .cmd：原生 exe 必须优先（绕开 8191 命令行上限 / 转义）。
        std::fs::write(node_dir.join("claude.cmd"), b"").expect("写入 claude.cmd");

        let resolved = resolve_claude_cli(Some(root.as_path()));
        assert_eq!(resolved.program, native, "应命中 _up_/resources/node 下的原生 exe");
        assert!(
            resolved.node_dir.is_none(),
            "原生 exe 不依赖 bundled node，不应注入 PATH"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bundled_cli_finds_resources_node_cmd_and_marks_node_dir() {
        // 第三个候选布局：`<resource_dir>/resources/node/claude.cmd`。
        let root = std::env::temp_dir().join(format!("pet-claude-cli-{}", uuid::Uuid::new_v4()));
        let node_dir = root.join("resources").join("node");
        std::fs::create_dir_all(&node_dir).expect("创建 resources/node");
        std::fs::write(node_dir.join("claude.cmd"), b"").expect("写入 claude.cmd");

        let resolved = resolve_claude_cli(Some(root.as_path()));
        assert_eq!(resolved.program, node_dir.join("claude.cmd"));
        assert_eq!(
            resolved.node_dir.as_deref(),
            Some(node_dir.as_path()),
            ".cmd 需要把 bundled node 目录注入子进程 PATH"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bundled_cli_falls_back_to_path_claude_cmd() {
        // 空资源目录（以及拿不到 resource_dir）时回退 PATH 上的全局 claude.cmd。
        let root = std::env::temp_dir().join(format!("pet-claude-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("创建空资源目录");
        assert!(bundled_claude_cli(&root).is_none());

        let fallback = resolve_claude_cli(Some(root.as_path()));
        assert_eq!(fallback.program, PathBuf::from("claude.cmd"));
        assert!(fallback.node_dir.is_none());
        assert_eq!(
            resolve_claude_cli(None).program,
            PathBuf::from("claude.cmd")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn result_line_marks_stream_completed() {
        let mut state = StreamParseState::new();
        state.parse_line(
            r#"{"type":"result","result":"收到","session_id":"session-1","is_error":false}"#,
        );
        assert!(state.completed, "收到最终 result 后 completed 必须为 true");
        assert_eq!(state.full_text, "收到");
        assert_eq!(state.session_id.as_deref(), Some("session-1"));
        assert!(!state.is_error);

        // 正常收尾的流再做一次收尾校验也不会被误判成中断。
        finalize_stream(&mut state, "");
        assert!(!state.is_error);
        assert!(state.error_msg.is_none());
    }

    #[test]
    fn finalize_stream_flags_missing_result_as_error() {
        let mut state = StreamParseState::new();
        state.parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"半截回复"}]}}"#,
        );
        assert!(!state.completed, "没有 result 行的流不能算完成");
        finalize_stream(&mut state, "");
        assert!(state.is_error);
        let message = state.error_msg.as_deref().unwrap_or_default();
        assert!(
            message.contains("未收到最终 result"),
            "缺少“未收到最终 result”的提示: {message}"
        );
        // 已经流出的正文保留，方便上层展示半截回复。
        assert_eq!(state.full_text, "半截回复");
    }

    #[test]
    fn finalize_stream_appends_stderr_tail_and_keeps_existing_error() {
        // stderr 只追加尾部，避免错误信息无限长（且按字符截断，UTF-8 安全）。
        let mut state = StreamParseState::new();
        let stderr = "错误".repeat(600);
        finalize_stream(&mut state, &stderr);
        assert!(state.is_error);
        let message = state.error_msg.clone().unwrap_or_default();
        assert!(message.contains("未收到最终 result"));
        assert!(
            message.chars().count() <= 560,
            "stderr 尾部必须截断，实际长度 {}",
            message.chars().count()
        );

        // 已有错误信息优先保留，不被通用文案覆盖。
        let mut errored = StreamParseState::new();
        errored.is_error = true;
        errored.error_msg = Some("已有的错误".to_string());
        finalize_stream(&mut errored, "stderr-ignored");
        assert_eq!(errored.error_msg.as_deref(), Some("已有的错误"));

        // 旧兜底：完成但正文为空且 stderr 非空时仍算错误。
        let mut completed_empty = StreamParseState::new();
        completed_empty.completed = true;
        finalize_stream(&mut completed_empty, "fatal: something went wrong");
        assert!(completed_empty.is_error);
        assert!(completed_empty
            .error_msg
            .as_deref()
            .unwrap_or_default()
            .contains("fatal"));
    }

    #[test]
    fn system_prompt_file_keeps_newlines() {
        // `--append-system-prompt-file` 的核心收益：落盘内容保留原始换行，
        // 不会再被压平成一行，提示词的分段结构因此得以保留。
        let dir = std::env::temp_dir().join(format!("pet-claude-prompts-{}", uuid::Uuid::new_v4()));
        let prompt = "第一段\n第二段\r\n第三段\n\n第四段";
        let path = write_system_prompt_file(&dir, prompt).expect("写入提示词文件");
        assert_eq!(
            std::fs::read_to_string(&path).expect("读取提示词文件"),
            prompt,
            "落盘提示词必须保留原始换行"
        );
        // 1 小时内的文件不会被清理掉。
        sweep_stale_prompt_files(&dir);
        assert!(path.is_file(), "新鲜提示词文件不应被清扫");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
