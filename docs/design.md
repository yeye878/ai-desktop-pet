# AI Desktop Pet — 设计文档

## 架构

```
┌─────────────────────────────────────┐
│         Tauri 桌面窗口               │
│  ┌───────────────────────────────┐  │
│  │     Vue 3 前端 (WebView)      │  │
│  │  PetCanvas / ChatBubble / Menu│  │
│  └──────────────┬────────────────┘  │
│                 │ IPC                │
│  ┌──────────────┴────────────────┐  │
│  │        Rust 后端              │  │
│  │  openclaw/  behavior/ system/ │  │
│  │  storage/                     │  │
│  └──────────────┬────────────────┘  │
└─────────────────┼───────────────────┘
                  │
     ┌────────────┴────────────┐
     ▼                         ▼
┌──────────┐          ┌──────────────┐
│  Ollama  │          │   OpenClaw   │
│ (直连)   │          │   Gateway    │
└──────────┘          └──────────────┘
```

## 状态机

```
        ┌─→ Listening ─→ Thinking ─→ Speaking ─┐
        │                                       │
Idle ◄──┤                                       ├──► Idle
        │                                       │
        └─→ Happy / Confused / Waving ◄─────────┘
                    │
                Sleeping (长时间无交互)
```

## IPC 命令

| 命令 | 方向 | 说明 |
|------|------|------|
| `send_to_ai` | FE→BE | 发送消息给 AI，自动管理状态 |
| `get_pet_state` | FE→BE | 获取当前宠物状态 |
| `set_pet_state` | FE→BE | 手动设置宠物状态 |
| `get_system_info` | FE→BE | 获取 CPU/内存信息 |
| `get_chat_history` | FE→BE | 获取对话历史 |
| `clear_chat_history` | FE→BE | 清空对话 |
| `save_memory` | FE→BE | 保存宠物记忆 |
| `get_memories` | FE→BE | 获取记忆 |
| `tick` | FE→BE | 每秒调用，驱动行为引擎 |

## 开发阶段

| 阶段 | 内容 | 状态 |
|------|------|------|
| P0 | 项目骨架 + 透明窗口 + Canvas 桌宠 | ✅ |
| P1 | 状态机动画 + 拖拽 + 气泡对话 | ✅ |
| P2 | Ollama 直连（含 OpenClaw 骨架） | ✅ |
| P3 | 行为引擎 + 心情系统 + 自动状态转换 | ✅ |
| P4 | 系统感知（CPU/内存监控） | ✅ |
| P5 | SQLite 记忆系统 | ✅ |
| P6 | 设置面板 + 换肤 + 打包 | TODO |
