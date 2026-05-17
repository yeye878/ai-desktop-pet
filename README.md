# AI Desktop Pet

智能桌宠 — 连接 OpenClaw / Ollama 等 AI Agent 的本地桌宠应用。

## 技术栈

- **桌面框架**: Tauri v2 (Rust + WebView)
- **前端**: Vue 3 + Canvas + Pinia
- **AI 接入**: OpenClaw Gateway (WebSocket RPC) / Ollama 直连
- **持久化**: SQLite

## 前置依赖

1. **Node.js** >= 22
2. **Rust** >= 1.77
3. **Ollama** (可选，用于直连本地模型)

### 安装 Rust

```bash
winget install Rustlang.Rustup
```

### 安装 Ollama (可选)

```bash
winget install Ollama.Ollama
ollama pull hermes3
```

## 开发

```bash
# 安装依赖
npm install

# 启动开发模式
npm run tauri dev

# 构建
npm run tauri build
```

## AI 模式

| 模式 | 说明 | 配置 |
|------|------|------|
| **Ollama** (默认) | 直连本地 Ollama，默认用 hermes3 | 需要 Ollama 运行中 |
| **OpenClaw** | 通过 OpenClaw Gateway 走全功能 AI | 需要 OpenClaw 运行中 |

## 项目结构

```
ai-desktop-pet/
├── src/                  # Vue 3 前端
│   ├── components/       # UI 组件
│   ├── stores/           # 状态管理
│   └── assets/           # 素材
├── src-tauri/            # Rust 后端
│   ├── src/
│   │   ├── openclaw/     # AI Agent 适配器
│   │   ├── behavior/     # 行为引擎
│   │   ├── system/       # 系统感知
│   │   └── storage/      # SQLite 存储
│   └── Cargo.toml
└── docs/                 # 设计文档
```
