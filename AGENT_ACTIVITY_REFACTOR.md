# Agent Activity Display Refactor

This change restructures the live agent surface around the same lifecycle used by modern agent clients:

```text
turn -> commentary/reasoning -> tool start -> tool result -> commentary -> final answer
```

The UI keeps these as separate activity entries instead of appending tool protocol markers to the thinking paragraph. Tool rows are compact by default, expandable for arguments and output, and preserve the latest terminal result. Both the dashboard and the compact chat window render the same activity model.

The design follows the DeepSeek Harness model of treating live agent events and durable session messages as separate concerns. The [Harness architecture](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/architecture.md) documents live `agent/*` events, tool lifecycle events, and an append-only session log as distinct surfaces. The [Codex SDK item definitions](https://github.com/openai/codex/blob/main/sdk/typescript/src/items.ts) and [Claude Code programmatic interface](https://code.claude.com/docs/en/headless) provided the CLI protocol references.

## Compatibility

- Direct API tool events keep their existing event names and confirmation payloads. The new renderer consumes the existing `ai-tool-event` stream and does not change permission decisions.
- Codex `--json` events now map command execution, file changes, web search, and MCP calls into the same activity protocol. Existing final text and session resume behavior remain unchanged.
- Claude `stream-json` events now map partial text, tool use, partial JSON arguments, and tool results. The parser de-duplicates partial and final snapshots.
- Legacy persisted `thinking` text is parsed on load. Unknown or malformed historical arguments remain visible as raw text.
- Message database schema and model-visible history are unchanged. The activity presentation cache is bounded to 100 entries and one million serialized characters, is display-only, and safely falls back when storage is unavailable or corrupt.
- Collaboration replies use the same activity store and reset it on run or turn handoff, preventing an old agent's live tools from appearing under a new agent.
- The normal confirmation UI and backend `confirm_tool` command remain in place. A 90-second loading fallback no longer clears a turn while a confirmation or tool execution is still active.

## Risk Evaluation

| Area | Result | Evidence |
| --- | --- | --- |
| Tool ordering and terminal status | Pass | 14 TypeScript activity tests; monotonic lifecycle and null-field preservation covered |
| Legacy history | Pass | Structured legacy marker parser and cache restore tests |
| Codex and Claude wire formats | Pass | 6 Rust parser/process fixture tests, including Chinese text, MCP, file changes and failed calls |
| Collaboration handoff | Pass | Existing multi-agent suite plus browser handoff scenario |
| Confirmation timeout | Pass | Browser test advances time by 95 seconds while confirmation remains active |
| Text output injection | Pass | Browser fixture verifies tool HTML is displayed literally and does not create elements or execute handlers |
| Small-window layout | Pass | Chromium at 340x400; no activity overflow |
| Build and Rust tests | Pass | `npm run build`; `cargo check --lib`; `cargo test --lib` (baseline of this earlier refactor: 101 passed, 4 pre-existing environment-dependent tests ignored). The Codex trajectory update below re-ran the same suite at 148 passed / 4 failed (environment) / 10 ignored |
| Existing multi-agent behavior | Pass | `npm run verify:multi-agent` (13 passed) |

The browser fixture can be run against the local development server with `npm run verify:agent-activity-browser` after setting `PLAYWRIGHT_MODULE` to the installed Playwright module. A visual demo is available at `/?activity=demo` in development mode.

## Codex 工具轨迹与思考过程 (2026-10 update)

桌宠的 Codex 后端（`codex exec --json`，会话续接与 workspace-write 沙箱保持不变）此前的轨迹展示有两个真实缺陷：`reasoning` 事件经常为空导致"思考过程"没有内容，且同一轮里思考与工具行的先后顺序不保证。本轮按本机 Codex CLI **0.162.0** 的真实事件流重做了映射与展示。

### 真实事件流（本机实测，非推测）

用桌宠同一套参数（`exec --sandbox workspace-write -c approval_policy="never" --json --skip-git-repo-check -`）抓到的实际输出：

```text
{"type":"thread.started","thread_id":"…"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 3 unrecognized configuration settings. …"}}
{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"我会先查看当前目录…"}}
{"type":"item.started","item":{"id":"item_3","type":"command_execution","command":"…powershell.exe\" -Command Get-ChildItem","aggregated_output":"","exit_code":null,"status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_3","type":"command_execution","command":"…","aggregated_output":"…","exit_code":-1,"status":"failed"}}
{"type":"item.completed","item":{"id":"item_4","type":"reasoning","text":"**Exploring command execution**\n\nI need to try…"}}
{"type":"item.completed","item":{"id":"item_5","type":"file_change","changes":[{"path":"…\\trace-note.txt","kind":"add"}],"status":"failed"}}
{"type":"turn.completed","usage":{"input_tokens":97199,"cached_input_tokens":49536,"output_tokens":667,"reasoning_output_tokens":379}}
```

由此确定并实现的映射：

| Codex 事件 | 桌宠轨迹 | 说明 |
| --- | --- | --- |
| `reasoning` | 思考过程（thinking 通道） | 取 `text`；为空时回退 `summary_text` → `raw_content`。`item.started/updated` 按 item id 用最长公共前缀只发增量，`item.completed` 只补未流出的尾部 |
| `agent_message` | 正文 | 增量由解析器计算，保持与思考的先后顺序 |
| `command_execution` | 执行命令 | `command` / `aggregated_output` / `exit_code` / `duration_ms` |
| `file_change` | 修改文件 | `changes:[{path,kind}]`，kind 为 add/update/delete |
| `web_search` | 网页搜索 | `query` + `action` |
| `mcp_tool_call` | MCP 工具 | 名称取 `server / tool`，输出取 error 或 result 文本 |
| `todo_list` | 更新任务清单 | 清单渲染为 ✓/• 步骤；读取时按 `items → plan → todos` 回退兼容版本差异 |
| `item.completed` + `type:"error"` | 提示 (notice) | Codex 每轮都会重复同一条配置告警；同一轮内按 message 去重，避免刷屏 |

关键顺序保证：`EventParser::parse_line` 现在返回 `Vec<Delta>`（`Thinking`/`Text`），`read_stream_with_activity` **签名不变**，按返回顺序分派回调，因此 reasoning 必然排在它之后出现的 `agent_message` 之前。

`exec_args` 追加了 `-c model_reasoning_summary="detailed"`：实测本机默认配置下同一任务**完全不产生 reasoning 事件**，加上该参数后才出现上面的 `reasoning` 行。sandbox / approval 相关参数一字未动。

### 界面

- 时间线：左侧竖线 + 每条目一个节点，节点颜色跟随状态（完成/失败/运行中/等待/提示）。
- 思考过程：流式时自动展开并显示"思考中"，结束自动折叠；显示字数，正文等宽灰底、可滚动。
- 工具行：动作中文名 + 图标（按工具类型）、目标命令/路径、耗时、非 0 退出码红标、状态文案。
- 展开详情：命令、文件变更清单（新增/修改/删除中文标注）、任务清单、参数、输出；输出超过 800 字先截断并提供"展开全部（共 N 字）"。
- 小窗 `.compact` 模式复用同一结构，不横向溢出。
- `/?activity=demo` 提供一套完整的 Codex 风格夹具（思考增量 → 提示 → 清单 → 命令失败 → 修改文件 → 命令重跑成功 → 最终回复），无需真实 CLI 即可验收展示链路。

### 本轮验证

| 检查 | 命令 | 结果 |
| --- | --- | --- |
| Rust 全量单测 | `cargo test --lib`（`CARGO_TARGET_DIR` 指向可写目录） | 148 passed / 4 failed / 10 ignored。4 个失败全部是环境性的且不在本次改动文件内：3 个 TTS 用例需要访问 speech.platform.bing.com（沙箱内 TLS 无凭证），1 个 `allows_current_workspace_paths` 需要往源码目录写文件（被沙箱 EPERM 拒绝）。跳过这 4 个后为 148 passed / 0 failed / 14 ignored |
| Rust 受影响模块 | `cargo test --lib cli_activity::` / `codex::` / `dsh::` | 8 / 10 / 15 passed，0 failed（codex 1 ignored、dsh 3 ignored） |
| Rust 编译告警 | `cargo check --lib` | 5 warnings，与改动前基线一致，零新增 |
| 活动模型 | `node scripts/verify-agent-activity.cjs` | 14 passed / 0 failed |
| 类型检查 | `npx vue-tsc --noEmit` | 退出码 0 |

真实 Codex CLI 被调用过一次用于采集事件流（需要 DSH 沙箱提权，因为 Codex 自身要写 `~/.codex`）。

## Known Boundaries

- Real authenticated Codex and Claude accounts were not invoked. The existing Codex account round-trip test requires credentials and an explicit `--ignored` test run; the two browser automation smoke tests and document dump test also retain their existing ignored status.
- Chromium screenshots and browser checks cover the frontend; native Windows WebView2 and release installers were not exercised.
- Direct API currently reports tool execution as `completed` even when the returned plain-text result describes an error. This refactor displays that result faithfully and does not infer success from text. CLI tools with structured failure signals display `failed`.
- Activity already emitted before a second window opens cannot be replayed in full by the existing backend snapshot. That window recovers legacy thinking/tool progress and then subscribes to new events; a full durable event journal would require a separate backend persistence change.
- Dependency audit still reports seven high-severity findings in the existing Vue/Vite toolchain. This refactor did not upgrade the framework because that would expand the compatibility surface beyond the requested display change.
- Tool output remains compacted by the backend event limit before it reaches the UI; users can expand the row but cannot recover content intentionally truncated by the backend.
- Codex 事件流的字段形状来自本机 Codex CLI 0.162.0 的一次真实采集；更早或更新的版本若改动 `command`（字符串 vs argv 数组）、`changes`（数组 vs 对象映射）或 `todo_list` 的键名，解析器已按已知变体做了回退，但未在那些版本上实测。
- 没有安装 Playwright，也没有启动浏览器；本轮前端验收依赖 `vue-tsc`、活动模型单测和一次服务端渲染（SSR）结构检查，未做真实 WebView2/Chromium 截图。
- 真实采集的事件流随仓库提供在 `scripts/fixtures/codex-exec-json-0.162.0.jsonl`（本机路径已脱敏），由 `npm run verify:codex-trajectory` 重放；采集本身需要一次 DSH 沙箱提权，因为 Codex 要写 `~/.codex`。
- `src-tauri/src/codex_probe.rs` 与 `src-tauri/src/__probe_write.txt` 是上一个会话遗留的未跟踪探针文件。前者会让 `cargo test --lib` 编译失败（闭包生命周期错误），已从 `lib.rs` 注销该模块（文件本身保留未删）；如需继续 app-server 方向的工作，应单独修复或删除它们。
- 该探针文件指向一个尚未实现的方向：`codex app-server` 的 JSON-RPC 协议带 `item/commandExecution/requestApproval` 等审批事件，可以在桌宠里做成确认弹窗。本轮只做 `exec --json` 的轨迹与思考展示，未改后端协议。
