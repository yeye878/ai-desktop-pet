# AI Desktop Pet

A local AI desktop pet built with Tauri, Vue, Rust, and Canvas. It provides a small companion window, chat UI, direct API agent mode, local memory, weather reminders, text-to-speech, scheduled tasks, and optional desktop automation tools.

## Features

- Desktop pet with multiple character/rendering modes
- Chat dashboard with streaming AI responses and tool traces
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
- Optional: a Claude Code CLI or OpenAI-compatible API provider, depending on the backend mode you use

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

Weather defaults to a wttr.in-compatible endpoint and falls back to a secondary provider for supported Chinese city names when wttr.in is unavailable.

## Repository Hygiene

Generated outputs such as `node_modules/`, `dist/`, `desktop-release/`, `src-tauri/target/`, bundled runtime resources, logs, temporary review files, and local lab artifacts are ignored.

Before publishing forks or releases, review local settings and generated assets to avoid accidentally sharing private data.

## License

MIT
