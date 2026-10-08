# AI Desktop Pet

A local AI desktop pet built with Tauri, Vue, Rust, and Canvas. It provides a small companion window, chat UI, direct API agent mode, local memory, weather reminders, text-to-speech, scheduled tasks, and optional desktop automation tools.

## Features

- Desktop pet with multiple character/rendering modes
- Chat dashboard with streaming AI responses and tool traces
- Local Claude Code and Codex CLI backends with conversation continuity
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
- Optional: a Claude Code CLI, a local Codex CLI, or an OpenAI-compatible API provider, depending on the backend mode you use

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

Codex runs through [`codex exec --json`](https://developers.openai.com/codex/noninteractive), resumes only the thread created for the pet, and resets its context when you start a new conversation or change backend/personality/profession. Session continuity lasts while the app is running. Its working directory is `codex-workspace` under the app data directory (shown in the connection panel). Commands use the `workspace-write` sandbox with approvals disabled: operations requiring extra permission fail instead of waiting for a hidden prompt. The existing Direct API tool permission controls apply only to the Direct API backend.

Weather defaults to a wttr.in-compatible endpoint and falls back to a secondary provider for supported Chinese city names when wttr.in is unavailable.

Create roles in **智能体工坊** and choose each role's backend (Direct API, local Claude Code, or local Codex). API roles can select their own saved provider profile and model. The built-in `@claude` and `@codex` roles are always available, regardless of the pet's selected backend; their cards provide login/configuration controls.

Mention roles with a space before the task, for example `@claude 审查方案，需要修改时交给 codex 实现` or `@周报助手 @代码助手 一起分析`. Each directly mentioned role is queued immediately; to let one role decide when to delegate, mention only that first role. Each role executes independently and posts its own attributed message. Before every invocation, the role receives all messages and quoted content from the current conversation in chronological order, including the other roles' newest replies. The shared transcript excludes private thinking and is not silently limited to the latest 50 messages. CLI role invocations reconstruct their own context from this transcript instead of sharing the ordinary pet's resumed CLI session.

**智能体协同** is enabled by default in both chat windows. In this mode, a role can request another role's help by writing a standalone line such as `@codex 根据审查意见修复代码`, and the recipient can later write `@claude 复核修改结果`. Requests run through a sequential queue so each role sees earlier results. Quoted text, code examples and incidental mentions do not trigger handoffs. Turning the switch off still executes the roles directly mentioned by the user, while disabling automatic role-to-role handoffs.

The chat displays the current worker and queue. Stop cancels the current invocation and remaining jobs. One collaboration is limited to 12 invocations and four per role to prevent loops; the status bar reports when a limit is reached, and the user can send another request to continue. A failed role posts its error under its own name while the other queued roles continue. Existing roles are migrated to the Direct API backend without losing their settings. Legacy custom roles named `claude` or `codex` remain callable by `@<role-id>`; newly created roles must use other names.

Role and active-skill tool grants apply only to that role's current Direct API invocation; plan mode blocks tool execution. Explicit API profiles keep their own execution settings. Local CLI roles use their own CLI permission controls. The AI role generator uses the configured Direct API connection.

Run `npm run verify:multi-agent` for mention routing, reply identity and cross-window store regression checks, and `cargo test --lib` from `src-tauri/` for backend tests.

## Repository Hygiene

Generated outputs such as `node_modules/`, `dist/`, `desktop-release/`, `src-tauri/target/`, bundled runtime resources, logs, temporary review files, and local lab artifacts are ignored.

Before publishing forks or releases, review local settings and generated assets to avoid accidentally sharing private data.

## License

MIT
