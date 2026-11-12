# AI Desktop Pet

A local AI desktop pet built with Tauri, Vue, Rust, and Canvas. It provides a small companion window, chat UI, direct API agent mode, local memory, weather reminders, text-to-speech, scheduled tasks, and optional desktop automation tools.

## Features

- Desktop pet with multiple character/rendering modes
- Chat dashboard with streaming AI responses and tool traces
- Independent per-agent conversation windows (Claude Code / Codex / DeepSeek Harness) with per-session history, model overrides and session management
- Local Claude Code, Codex, and DeepSeek Harness CLI backends with conversation continuity
- Per-session permission presets for the three local CLIs (follow local config, ask every time, auto-approve, or full access), settable right in the conversation
- Direct API profiles for OpenAI-compatible chat/completions APIs
- Local SQLite storage for conversation history, memories, clipboard items, and scheduled tasks
- Weather bar and daily weather greeting with a fallback weather provider
- Text-to-speech support through Edge TTS
- Optional computer-use tools for screenshots, mouse/keyboard, windows, and browsers
- Custom pixel pet workshop for user-generated pet sprites

## Tech Stack

- Tauri v2
- Rust
- Vue 3
- Pinia
- Vite
- SQLite

## Requirements

- Windows 10/11
- Node.js 22 or newer
- Rust 1.77 or newer
- Optional: a Claude Code CLI, a local Codex CLI, a local DeepSeek Harness (`dsh`) CLI, or an OpenAI-compatible API provider, depending on the backend mode you use

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

The NSIS installer is generated under:

```text
src-tauri/target/release/bundle/nsis/
```

For the local desktop shortcut workflow used by this project:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\create_shortcut.ps1
```

## Configuration

Runtime settings, chat history, memories, and API profiles are stored locally in the app data directory. API keys are stored through the system credential helper where supported and are not intended to be committed to the repository.

To use local Codex, install it with `npm install -g @openai/codex`, run `codex login`, then select **Codex** under **AI 后端与模型** in the dashboard or settings. The connection panel also provides login, refresh, and conversation reset controls. It uses your existing Codex authentication, model/provider configuration and `CODEX_HOME`, without copying credentials into the pet's settings.

Claude Code runs through the local `claude` CLI in `-p` (print) mode with `--output-format stream-json`, and keeps its own session for the pet. Because Claude's permission prompts have no terminal to land in when the pet drives it, the pet starts a small permission bridge: a loopback-only MCP HTTP server (`src-tauri/src/mcp_permission.rs`) exposing a single `confirm_permission` tool, handed to Claude with `--mcp-config`, `--permission-prompt-tool mcp__pet__confirm_permission` and `--permission-prompts host`. Anything the session's permission mode would prompt for now appears in the same confirmation window the Direct API backend uses — with the tool name, the command or path, and the full tool input — and the answer goes back as `{"behavior":"allow"}` or `{"behavior":"deny","message":…}`; a denial is reported to the model, and a 120-second timeout denies. On Windows the shell tool is `PowerShell` rather than `Bash`, which the bridge handles by reading what Claude sends instead of assuming names. The server listens only on 127.0.0.1, on a random port, under a random per-run path token, so another local process cannot drive the pet's dialogs. The **Claude Code 权限模式** selector in the settings card is the default: 跟随本机配置 (no `--permission-mode` argument), `auto` (only risky calls prompt), `manual` (everything prompts), `acceptEdits`, `plan`, `dontAsk`, or `bypassPermissions` (no checks at all). Sessions can override it individually — the permission chip in the chat composer (main chat, the Claude Code / Codex / DSH agent windows, the multi-pane session board, and the pet chat window) writes `chat_sessions.permission` for that conversation only, with 跟随设置页默认 as the reset option; a session that never sets one follows the settings default. The choice is applied to the next Claude process; it does not edit your Claude Code settings files.
Codex runs through [`codex exec --json`](https://developers.openai.com/codex/noninteractive), resumes only the thread created for the pet, and resets its context when you start a new conversation or change backend/personality/profession. Session continuity lasts while the app is running. Its working directory is `codex-workspace` under the app data directory (shown in the connection panel). The chat shows Codex's own trajectory while it runs: `reasoning` items stream into the collapsible 思考过程 block, and `command_execution`, `file_change`, `web_search`, `mcp_tool_call`, `todo_list` and Codex's repeated configuration notices become tool rows with their command, changed files, exit code, duration and output. Reasoning summaries are requested with `-c model_reasoning_summary="detailed"`, because this machine's default configuration emits no `reasoning` items at all — without it the thinking block would always be empty. The **Codex 沙箱权限** selector in the settings card sets the default (`read-only`, `workspace-write` or `danger-full-access`) and the permission chip in each conversation overrides it per session; approvals stay disabled in every case: `codex exec` is a one-way event stream with no channel to answer an approval request, so operations requiring extra permission fail instead of waiting for a hidden prompt — surfacing Codex prompts would mean moving this backend to `codex app-server`, whose JSON-RPC protocol carries `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval` and `item/tool/requestUserInput`. The existing Direct API tool permission controls apply only to the Direct API backend.

To use local DeepSeek Harness, install it with `npm install -g @deepseek-ai/dsh`, configure a model once (run `dsh web` and add one on the **Models** page, or export `DEEPSEEK_API_KEY`), then select **DeepSeek Harness** under **AI 后端与模型**. The connection panel shows the CLI version, the model from your DSH settings, the pet profile and the working directory, and opens your DSH home folder for configuration. Its **DeepSeek Harness 权限** selector in the settings card maps the default to `DSH_PERMISSION_MODE`: 跟随本机配置 (leave the variable alone), `read-only`, `workspace-write`, `workspace-write-auto` (keep the workspace sandbox but auto-approve escalation requests without a dialog), or `danger-full-access` (sandbox and approval prompts off), while each conversation's permission chip overrides it per session (the effective value is resolved at turn start, session first, settings second). The choice is applied to the next `dsh` process; it does not rewrite your DSH settings or profile overlays.

The pet boots DSH through its own profile, `$DSH_HOME/profiles/dsh-pet`, which mounts the shipped `dsh-base` and `dsh-headless` bundles and then replaces the shipped one-shot app with the pet's own runner plugin (`plugins/pet-runner.mjs`, generated from `src-tauri/dsh/pet-runner.mjs`). Your own `headless` profile overlays are never loaded into a pet turn, while login, model/provider choice, permission presets and the machine-level `$DSH_HOME/cordis.patch.yml` all keep coming from your DSH home; no credential is copied into the pet. The pet maintains only its own patch layer: your edits to that file are preserved and the pet's rows are appended after them.

The runner drives the same core API the shipped app uses (`agentDefaultModel` / `agents` / `sessions`) but subscribes to `session/event` and speaks a small JSONL protocol on stdout, so the pet gets what Codex and Claude Code give it: reasoning deltas, streamed answer text, tool-call/tool-result events in the tool trace panel, and a real resumable session (`--resume`, flushed before exit). The task text travels in a prompt file instead of an argument, so long prompts are no longer bounded by the Windows command line; very large first-turn replays are still trimmed from the middle (oldest transcript entries first) with the omission marked in the task text. Its working directory is `dsh-workspace` under the app data directory and is the root of DSH's own sandbox.

DSH asks the pet for permission through its own approval seam (`ctx.approval.request`), and the pet's runner plugin answers it: sandboxed commands run without prompting, but when the agent wants to step outside the sandbox it must request escalation (`sandbox_permissions` + a one-sentence `justification`), and that request is forwarded to the pet's confirmation window — the same `ai-tool-confirm` panel the Direct API backend uses, showing the command, the arguments and the escalation reason, with 允许/拒绝. The answer goes back over the child's stdin as `allowed-once` or `rejected`; a rejection is final for that command, a 120-second timeout denies (fail closed), and DSH records both the ask and the outcome in the session log. Because the seam has no remember-forever grant, "allow" applies to that one call. This only works while the session's approval policy is `ask` (the default `workspace-write` preset); under the `danger-full-access` preset the policy is `never`, prompts are disabled and anything needing approval is rejected outright. The pet's `workspace-write-auto` option keeps the workspace-write sandbox but answers every escalation request with `allowed-once` itself, so the model can cross the boundary without a dialog.

Run `npm run verify:dsh-backend` to check the whole DSH path offline: it boots the real `dsh` CLI with a DSH home inside the repository, points the default model at `scripts/dsh-mock-provider.cjs` (an OpenAI-compatible mock that streams reasoning, a tool call and the final answer), and asserts reasoning deltas, the streamed answer, `--resume` continuity and the tool-event channel. Two further `#[ignore]`d tests (`cargo test --lib dsh::tests::local_cli_ -- --ignored`) run the same path against your own DSH login.

Weather defaults to a wttr.in-compatible endpoint and falls back to a secondary provider for supported Chinese city names when wttr.in is unavailable.

Create roles in **智能体工坊** and choose each role's backend (Direct API, local Claude Code, local Codex, or local DeepSeek Harness). API roles can select their own saved provider profile and model. The built-in `@claude`, `@codex` and `@dsh` roles are always available, regardless of the pet's selected backend; their cards provide login/configuration controls.

Mention roles with a space before the task, for example `@claude 审查方案，需要修改时交给 codex 实现` or `@周报助手 @代码助手 一起分析`. Each directly mentioned role is queued immediately; to let one role decide when to delegate, mention only that first role. Each role executes independently and posts its own attributed message. Before every invocation, the role receives all messages and quoted content from the current conversation in chronological order, including the other roles' newest replies. The shared transcript excludes private thinking and is not silently limited to the latest 50 messages. CLI role invocations reconstruct their own context from this transcript instead of sharing the ordinary pet's resumed CLI session.

**智能体协同** is enabled by default in both chat windows. In this mode, a role can request another role's help by writing a standalone line such as `@codex 根据审查意见修复代码`, and the recipient can later write `@claude 复核修改结果`. Requests run through a sequential queue so each role sees earlier results. Quoted text, code examples and incidental mentions do not trigger handoffs. Turning the switch off still executes the roles directly mentioned by the user, while disabling automatic role-to-role handoffs.

The chat displays the current worker and queue. Stop cancels the current invocation and remaining jobs. One collaboration is limited to 12 invocations and four per role to prevent loops; the status bar reports when a limit is reached, and the user can send another request to continue. A failed role posts its error under its own name while the other queued roles continue. Existing roles are migrated to the Direct API backend without losing their settings. Legacy custom roles named `claude`, `codex` or `dsh` remain callable by `@<role-id>`; newly created roles must use other names.

Role and active-skill tool grants apply only to that role's current Direct API invocation; plan mode blocks tool execution. Explicit API profiles keep their own execution settings. Local CLI roles use their own CLI permission controls. The AI role generator uses the configured Direct API connection.

**对话互动** has an expandable sub-list with three independent conversation windows — Claude Code, Codex and DeepSeek Harness — alongside the main chat. Each window keeps its own message list, can switch its bound agent, and stores a per-session model override that is passed to the local CLI (`--model` for Claude Code, `-m` for Codex, `--model` for the DSH runner); leaving it empty follows the local CLI configuration. Direct API roles instead take a model override for their profile.

Every window keeps a session list in the right sidebar: create a session, switch between sessions, reopen old ones, rename or delete them. The main chat window has the same list. Sessions live in SQLite (`chat_sessions` plus `chat_history.session_id`), and each session remembers its own CLI resume ids, so switching back to an older session continues that session's context instead of replaying the shared one. `@` mentions, role handoffs and collaboration stay in the main chat; the three dedicated windows run a single agent turn without collaboration.

Run `npm run verify:multi-agent` for mention routing, reply identity and cross-window store regression checks, `npm run verify:chat-sessions` for the multi-session store / dev-mock regression checks, and `cargo test --lib` from `src-tauri/` for backend tests.

## Repository Hygiene

Generated outputs such as `node_modules/`, `dist/`, `desktop-release/`, `src-tauri/target/`, bundled runtime resources, logs, temporary review files, and local lab artifacts are ignored.

Before publishing forks or releases, review local settings and generated assets to avoid accidentally sharing private data.

## License

MIT
