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
| Build and Rust tests | Pass | `npm run build`; `cargo check --lib`; `cargo test --lib` (101 passed, 4 pre-existing environment-dependent tests ignored) |
| Existing multi-agent behavior | Pass | `npm run verify:multi-agent` (13 passed) |

The browser fixture can be run against the local development server with `npm run verify:agent-activity-browser` after setting `PLAYWRIGHT_MODULE` to the installed Playwright module. A visual demo is available at `/?activity=demo` in development mode.

## Known Boundaries

- Real authenticated Codex and Claude accounts were not invoked. The existing Codex account round-trip test requires credentials and an explicit `--ignored` test run; the two browser automation smoke tests and document dump test also retain their existing ignored status.
- Chromium screenshots and browser checks cover the frontend; native Windows WebView2 and release installers were not exercised.
- Direct API currently reports tool execution as `completed` even when the returned plain-text result describes an error. This refactor displays that result faithfully and does not infer success from text. CLI tools with structured failure signals display `failed`.
- Activity already emitted before a second window opens cannot be replayed in full by the existing backend snapshot. That window recovers legacy thinking/tool progress and then subscribes to new events; a full durable event journal would require a separate backend persistence change.
- Dependency audit still reports seven high-severity findings in the existing Vue/Vite toolchain. This refactor did not upgrade the framework because that would expand the compatibility surface beyond the requested display change.
- Tool output remains compacted by the backend event limit before it reaches the UI; users can expand the row but cannot recover content intentionally truncated by the backend.
