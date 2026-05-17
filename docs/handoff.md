# AI Desktop Pet — 交接日志

> 更新时间: 2026-05-17

## 项目概述

本地智能桌宠应用，通过 Claude Code CLI 实现桌面宠物 AI 交互。

- **项目路径**: `D:\ai-desktop-pet`
- **技术栈**: Tauri v2 (Rust) + Vue 3 + Pinia + SQLite
- **AI 接入**: Claude Code CLI（`claude.cmd` 子进程，复用本地 Claude Code 认证）

## 当前进度

| 阶段 | 内容 | 状态 |
|------|------|------|
| P0 | Tauri 项目骨架 + 透明无边框窗口 | ✅ 完成 |
| P1 | Canvas 简笔画桌宠 + 9 种状态动画 + 拖拽 + 气泡对话/剪切板双模式 | ✅ 完成 |
| P2 | Claude Code CLI 适配器（stream-json 流式输出 + 中止） | ✅ 完成 |
| P3 | 行为引擎 + 心情/精力系统 + 自动状态转换 + 每秒 tick | ✅ 完成 |
| P4 | 系统感知 (CPU/内存监控) | ✅ 完成 |
| P5 | SQLite 记忆系统 (对话历史 + 宠物记忆 + 桌宠剪切板) | ✅ 完成 |
| P6 | 设置面板换肤 + 多窗口拆分 + 打包分发 | 🔧 进行中（换肤/多窗口已完成，打包待做） |

## 文件结构

```
D:\ai-desktop-pet\
├── src/                          # Vue 3 前端
│   ├── App.vue                   # 主组件：按窗口 label 分流渲染 + chat/context-menu/settings 多窗口调度
│   ├── main.ts                   # 入口
│   ├── components/
│   │   ├── PetCanvas.vue         # 桌宠渲染：Canvas 120x140 + setPosition 拖拽 + 点击检测
│   │   ├── ChatBubble.vue        # 气泡面板：对话/剪切板双模式 + 文件拖入路径注入
│   │   ├── ContextMenu.vue       # 右键菜单：打招呼/开心/睡觉/叫醒/清空对话/设置
│   │   └── Settings.vue          # 设置面板：独立透明无边框窗口内容
│   └── stores/
│       ├── pet.ts                # 宠物状态：state/happiness/energy/position/expression
│       └── chat.ts               # 对话状态：messages/isLoading
├── src-tauri/                    # Rust 后端
│   ├── src/
│   │   ├── main.rs               # 入口
│   │   ├── lib.rs                # 核心：AppState + 13 个 Tauri IPC 命令 + 启动逻辑
│   │   ├── openclaw/
│   │   │   ├── client.rs         # AgentAdapter trait + ClaudeAdapter（CLI 子进程调用）
│   │   │   └── mod.rs
│   │   ├── behavior/
│   │   │   ├── state.rs          # BehaviorEngine：状态机 + tick 自动转换 + 心情/精力
│   │   │   ├── mood.rs           # Mood：happiness(-1~1) + energy(0~100)
│   │   │   └── mod.rs
│   │   ├── system/
│   │   │   ├── monitor.rs        # SystemMonitor：CPU/内存使用率 (sysinfo)
│   │   │   └── mod.rs
│   │   └── storage/
│   │       ├── db.rs             # Database：chat_history + pet_memory + clipboard_items 表 CRUD
│   │       └── mod.rs
│   ├── capabilities/
│   │   └── default.json          # Tauri 权限：main/chat/context-menu/settings 多窗口 + window APIs + shell
│   ├── Cargo.toml                # Rust 依赖
│   └── tauri.conf.json           # Tauri 配置：main 120x140 透明无边框无阴影 + 置顶
├── docs/
│   ├── design.md                 # 设计文档
│   └── handoff.md                # 本文件
├── package.json                  # Node 依赖
├── vite.config.ts
├── tsconfig.json
└── README.md
```

## IPC 命令清单

Rust 后端暴露给前端的命令：

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `send_to_ai` | `message: String` | `{text, thinking}` | 流式调用 AI，通过 `ai-thinking` 事件推送思考内容 |
| `abort_ai` | 无 | `()` | 中止正在进行的 AI 调用（kill 进程树） |
| `get_pet_state` | 无 | `Value` | 获取 `{state, happiness, energy}` |
| `set_pet_state` | `newState: String` | `()` | 手动设置状态 (idle/happy/sleeping/waving) |
| `get_system_info` | 无 | `Value` | 获取 `{cpu, memory}` |
| `get_chat_history` | 无 | `Value` | 获取最近 50 条对话 |
| `clear_chat_history` | 无 | `()` | 清空对话 |
| `save_memory` | `category, key, value` | `()` | 保存记忆 |
| `get_memories` | `category: String` | `Value` | 获取某分类下的记忆 |
| `tick` | 无 | `Value` | 每秒调用，驱动行为引擎自动转换状态 |
| `switch_model` | 无 | `()` | 重置 Claude 会话（清空上下文） |
| `get_current_model` | 无 | `String` | 获取当前模型标识 |
| `set_skin` | `skin: String` | `()` | 设置皮肤 ID |
| `get_skin` | 无 | `String` | 获取当前皮肤 ID |
| `set_font_color` | `color: String` | `()` | 保存字体颜色到 `pet_settings` |
| `get_font_color` | 无 | `String` | 获取已保存字体颜色 |
| `set_user_avatar` | `avatar: String` | `()` | 保存用户自定义头像（Data URL，存入 `pet_settings.user_avatar`） |
| `get_user_avatar` | 无 | `String` | 获取用户自定义头像 |
| `save_clipboard_item` | `content: String` | `()` | 保存一条桌宠剪切板内容，不调用 AI |
| `get_clipboard_items` | 无 | `Vec<ClipboardItem>` | 获取最近 100 条剪切板内容，置顶优先 |
| `delete_clipboard_item` | `id: i64` | `()` | 删除单条剪切板内容 |
| `set_clipboard_item_pinned` | `id: i64, pinned: bool` | `()` | 设置/取消剪切板条目置顶 |
| `clear_clipboard_items` | 无 | `()` | 清空桌宠剪切板 |

## Claude Code 适配器

通过 `tokio::process::Command` 调用 `claude.cmd` CLI 子进程：

- **命令**: `claude.cmd -p "<消息>" --output-format stream-json --include-partial-messages --append-system-prompt "<桌宠人格>" --max-budget-usd 0.5`
- **流式输出**: 逐行读取 NDJSON，解析 `thinking` / `text` 内容块，通过 Tauri `window.emit("ai-thinking", delta)` 推送思考增量到前端
- **中止支持**: 子进程存储在 `AppState.child_process`，`abort_ai` 通过 `taskkill /F /T` 杀整个进程树
- **多轮对话**: 首次调用后从 JSON 输出提取 `session_id`，后续调用追加 `--resume <session_id>`
- **认证**: 复用本地 Claude Code 登录状态，无需额外 API Key
- **错误处理**: 检查 exit code、`is_error` 字段、`errors` 数组，返回详细错误信息
- **人格提示**: "你是一个可爱的桌面宠物。用简短、活泼的语气回复，每次回复控制在 2-3 句话以内。偶尔加一些表情符号但不要过多。"

## 桌宠状态机

```
开机 → Waving(3秒) → Idle
Idle → 用户发消息 → Thinking → Speaking(5秒) → Idle
Idle → 空闲600秒 → Sleeping → 精力满 → Idle
任意 → 右键操作 → Happy(4秒) → Idle / Sleeping / Waving
```

心情/精力：每秒自动衰减，用户交互恢复，睡觉恢复精力。

## 今日变更 (2026-05-10)

1. **窗口布局修复**:
   - `.pet-container` 从 `100vw/100vh` 改为固定 `460x460px`，避免子元素被视口缩放
   - Canvas 初始化改为固定 `120x140` 物理像素（不再用 `offsetWidth/offsetHeight`）
   - ChatBubble/Settings 面板 `left` 从 130px 改为 120px
2. **DPI 缩放修复**:
   - 窗口尺寸/位置 API 从 `PhysicalSize`/`PhysicalPosition` 改为 `LogicalSize`/`LogicalPosition`
   - 拖拽中 `outerPosition()` 返回的物理像素除以 `devicePixelRatio` 转为逻辑像素
3. **拖拽事件隔离**:
   - PetCanvas 添加 `dragFromCanvas` 标志，只有从 canvas 发起的 mousedown 才触发拖拽
   - ChatBubble/Settings 添加 `@mousedown.stop` 阻止事件穿透
4. **Claude Code 集成**:
   - 新增 `ClaudeAdapter`，通过 `claude.cmd` CLI 子进程调用
   - 支持多轮对话（`--resume`）和桌宠人格系统提示
   - 详细错误处理（exit code + stdout + stderr + is_error 检查）
5. **移除 Ollama/OpenClaw**:
   - 删除 `OllamaAdapter`、`OpenClawAdapter`、`protocol.rs`
   - 移除 `reqwest` 依赖
   - 默认模型改为 `claude:claude-code`
6. **设置面板简化**:
   - 移除模型类型选择/地址/名称字段
   - 模型持久化修复：`loadCurrentModel` 解析 `"type:name"` 格式同步下拉框状态
   - 只保留 "重置会话" 按钮

## 今日变更 (2026-05-12) — UI 重新设计

1. **主题系统**:
   - `pet.ts` 新增 `Theme` 接口，每个皮肤从单一颜色升级为完整主题对象（主色/暗色/强调色/渐变/毛玻璃背景/气泡颜色/粒子颜色组）
   - 6 套主题：樱花红 / 深海蓝 / 星空紫 / 森林绿 / 日落橙 / 樱花粉
   - 主题通过 CSS 变量 (`--pet-primary`, `--pet-header-gradient` 等) 注入全局，所有组件自动跟随
   - `watch([skin, fontColor])` 自动触发 `applyCssVars()`
2. **字体颜色自定义**:
   - `pet.ts` 新增 `FONT_COLORS` 预设（默认/深色/暖灰/冷灰/咖啡/藏蓝）
   - `Settings.vue` 新增字体颜色选择器（Aa 预览网格）
   - 字体色通过 `--pet-font-color` CSS 变量传递到 AI 气泡和其它区域
3. **ChatBubble 重新设计**:
   - 毛玻璃面板 (`backdrop-filter: blur(20px)`) + 渐变头部 + 顶部装饰条纹
   - 用户/AI 头像、气泡尾巴、消息时间戳、入场动画、思考展开/收起 Transition
   - 圆形图标发送按钮、关闭按钮旋转 hover、超细滚动条、文件拖入弹跳蒙版
4. **Settings / ContextMenu 重新设计**:
   - 毛玻璃风格统一、皮肤渐变预览 + 勾选标记 + hover 放大
   - 右键菜单弹出缩放动画、hover 主题色填充 + 微移
5. **PetCanvas 交互增强**:
   - 粒子系统：心形(happy)/星形(thinking)/音符(speaking)/问号(confused)/火花(waving)
   - 身体光晕 + 高光 + 腮红 + 眼睛亮点 + 开心弹跳拉伸 + 思考旋转星星
6. **验证**: `npm run build` + `cargo check` 通过

## 今日变更 (2026-05-12) — 流式思考

1. **流式思考输出 + 可折叠思考 UI**:
   - Rust 后端 `ClaudeAdapter` 从 `--output-format json` 改为 `--output-format stream-json --include-partial-messages`
   - 不再用 `cmd.output().await` 一次性等待，改为 `cmd.spawn()` + `BufReader::lines()` 逐行读取 NDJSON
   - 新增 `StreamParseState` 解析器：提取 `thinking` / `text` 内容块，计算增量 delta
   - 思考增量通过 `window.emit("ai-thinking", delta)` 实时推送到前端
   - `send_to_ai` 返回值从 `String` 改为 `{text, thinking}` JSON 对象
   - 前端 `ChatBubble.vue` 通过 `listen("ai-thinking")` 实时累积思考内容
2. **可折叠思考显示**:
   - 正在思考时：显示 `▾ 思考中...` 标题 + 可展开/收起的思考内容区 + 停止按钮
   - 已完成消息：如果有思考内容，显示 `▸ 思考过程` 可点击展开查看
   - 思考区域样式：浅灰背景、小字号 11px、最高 120px 可滚动、流式时左边框红色高亮
   - `Message` 类型新增 `thinking?: string` 字段
3. **中止 AI 思考**:
   - 新增 `abort_ai` IPC 命令
   - 子进程句柄存储在 `AppState.child_process`
   - Windows 上使用 `taskkill /F /T /PID` 杀整个进程树（cmd.exe → node.exe）
   - 中止后显示 "已中止~" 消息，并保留已收到的思考内容
4. **架构变更**:
   - `AppState.ai` 从 `Box<dyn AgentAdapter>` 改为具体类型 `ClaudeAdapter`（简化流式调用）
   - 移除 `AgentAdapter` trait（不再需要多态）
   - `openclaw/client.rs` 新增 `spawn_streaming()`、`update_session()`、`reset_session()` 方法
   - `openclaw/client.rs` 新增 `read_stream()` 函数和 `StreamParseState` 解析器
5. **验证记录**:
   - `cargo check` 通过，零 warning
   - `npm run build` 通过（vue-tsc 类型检查 + Vite 构建）

## 今日变更 (2026-05-11)

1. **对话框滚动体验修复**:
   - `ChatBubble.vue` 在组件挂载、消息数量变化、加载状态变化和页签切换后自动滚动到底部
   - 修复重复打开对话框时停留在历史记录顶部、需要手动下翻的问题
   - 使用 `nextTick + requestAnimationFrame` 等待 DOM 渲染完成后再滚动，避免滚动时机过早
2. **消息文本可选择复制**:
   - 外层 `.pet-container` 仍保持 `user-select: none`，避免影响桌宠拖拽体验
   - 在 `.chat-messages` 和 `.bubble` 上显式设置 `user-select: text`
   - 用户和 AI 的消息内容均可拖选复制
3. **文件拖入路径注入**:
   - 使用 Tauri v2 `getCurrentWindow().onDragDropEvent()` 监听文件拖放
   - 仅当文件落在 ChatBubble 区域内时处理，避免拖到宠物本体误触发
   - 拖入后将本地绝对路径写入输入框，提示词为：`请使用文件投喂分类流程处理这些本地文件路径："..."`
   - 桌宠项目只负责路径入口；文件理解/分类能力建议通过 Claude Code Skill 实现
4. **架构纠偏：撤回项目内分类数据库方案**:
   - 曾短暂尝试在项目内加入 `feed_file` IPC、`fed_files` / `file_categories` 表和文件分类逻辑
   - 已全部移除，避免桌宠项目与 Claude Code Skill 形成两套分类经验系统
   - 当前原则：桌宠做“桌面投递口 + 状态化陪伴层”，分类智慧放到 Claude Code Skill
5. **新增桌宠剪切板模式**:
   - `ChatBubble.vue` 顶部新增 `对话 / 剪切板` 双页签
   - 剪切板模式下输入内容只保存到 SQLite，不调用 AI
   - 新增 `clipboard_items` 表，内容持久化存储在原 `pet.db`
   - 剪切板页签提供一键清空，清空剪切板不影响聊天记录
   - 对话页签也提供一键清空，调用原 `clear_chat_history`，不影响剪切板
   - 当前页签写入 `localStorage`，关闭对话框再打开会保持上次的 `对话` 或 `剪切板` 状态
6. **验证记录**:
   - 已运行 `npm run build`，前端类型检查与 Vite 构建通过
   - 已运行 `cargo check`，Rust 后端编译检查通过
   - 已重新启动 Tauri dev；若看不到新 UI，优先确认是否存在旧 `ai-desktop-pet.exe` 窗口

## 今日变更 (2026-05-14) — 外观、剪切板与设置体验

1. **桌宠外观与主题增强**:
   - 新增“流光变色”动态主题，桌宠本体、粒子、按钮、面板主色随时间平滑变色
   - `PetCanvas.vue` 重画桌宠本体：耳朵、柔和渐变身体、光晕、高光、腮红、爪子细节，整体更可爱
   - `Settings.vue`、`ChatBubble.vue`、`ContextMenu.vue` 继续统一玻璃质感、渐变、阴影和 hover 反馈
2. **设置持久化补全**:
   - 修复字体颜色只存在前端内存的问题：新增 `set_font_color` / `get_font_color`，写入 SQLite `pet_settings`
   - 新增用户自定义头像：设置页支持选择头像、预览、恢复默认，头像保存到 `pet_settings.user_avatar`
   - 聊天气泡启动时会加载上次保存的头像；用户消息头像移动到消息右侧，符合常见聊天应用布局
3. **AI 中止稳定性修复**:
   - `send_to_ai` 不再长时间持有 Claude 子进程锁
   - 后端改为保存当前 AI 进程 PID，`abort_ai` 通过 PID kill 进程树，避免“停止思考”按钮被流式读取锁卡住
4. **剪切板增强**:
   - `clipboard_items` 自动补 `pinned` 列；后端 `get_clipboard_items` 返回对象结构 `{id, content, pinned, created_at}`
   - 新增单条删除、置顶/取消置顶；旧字符串式剪切板数据在前端仍兼容
   - 前端新增剪切板搜索、复制、删除、置顶；列表按置顶优先、时间倒序展示
   - 剪切板每条内容的“置顶 / 复制 / 删除”已移动到消息底部，并改成常见的图标 + 文字胶囊按钮
5. **审查与验证**:
   - 本轮剪切板/头像功能曾使用子代理并行开发；已审查后端和前端结果，确认未覆盖此前 `abort_ai` PID 修复
   - 已运行 `npm run build` 通过
   - 已运行 `cargo check` 通过
   - 浏览器插件预览在最后一次按钮样式检查时连接超时，未拿到截图；构建和代码审查已通过

## 今日变更 (2026-05-15) — 多窗口架构落地

1. **主窗口收回到桌宠本体**:
   - `tauri.conf.json` 显式设置主窗口 label 为 `main`，尺寸固定 `120x140`
   - 主窗口保持透明、无边框、置顶、跳过任务栏，并新增 `shadow: false`
   - `App.vue` 不再承载聊天框、右键菜单、设置面板，也不再使用窗口扩缩逻辑
2. **独立透明窗口拆分**:
   - 左键点击桌宠本体时创建/关闭 `chat` 窗口，尺寸 `340x400`，默认定位在宠物右侧
   - 右键点击桌宠本体时创建 `context-menu` 窗口，尺寸 `170x260`，定位在鼠标处，失焦自动关闭
   - 从右键菜单打开 `settings` 窗口，尺寸 `380x460`，定位在宠物右侧，打开后关闭菜单窗口
   - 已有 `chat/settings` 窗口在宠物拖动时会跟随宠物右侧重新定位
3. **组件独立窗口适配**:
   - `ChatBubble.vue` 和 `ContextMenu.vue` 改为占满各自窗口，不再依赖主窗口内部偏移
   - `ContextMenu.vue` 移除全屏 mask 和外部 `position` prop，菜单项点击后关闭当前菜单窗口
   - `Settings.vue` 修改主题、字体颜色、头像后广播 `appearance-changed`，其他窗口会重新加载外观设置
   - 右键菜单清空对话后广播 `chat-history-cleared`，已打开的聊天窗会同步清空本地消息状态
4. **鼠标命中与透明区域处理**:
   - 保留主窗口的桌宠形状 hit-test / `setIgnoreCursorEvents` 补丁，只用于主窗口 `120x140` 内的透明像素穿透
   - 左键/右键仍然只由 `PetCanvas` 的桌宠本体命中区域触发
   - 打开聊天、菜单、设置不再改变主窗口矩形，因此桌宠周围不会再出现主窗口制造的大透明边框
5. **验证记录**:
   - 已运行 `npm run build` 通过
   - 已运行 `cargo check` 通过

## 今日变更 (2026-05-15) — 对话窗口关闭后的思考恢复

1. **AI 任务与 `chat` 窗口解耦**:
   - `send_to_ai` 改为启动后端后台任务并立即返回，不再把 AI 流式请求绑定到当前 `chat` 窗口生命周期
   - 新增后端 `active_chat` 状态，缓存正在处理的用户消息、思考内容和开始时间
   - 思考 delta 改为通过 `AppHandle.emit("ai-thinking")` 广播，窗口关闭后后台任务仍继续执行
2. **消息持久化修复**:
   - 用户消息在发送时立即写入 SQLite，避免窗口中途关闭导致消息只停留在前端 Pinia 内存里
   - `chat_history` 新增 `thinking` 列，AI 完整回复后会把 assistant 回复和最终思考过程一起持久化
   - `get_chat_history` 现在返回 `{role, content, thinking, created_at}`，前端可恢复历史 assistant 的思考过程
3. **重开对话框恢复进行中状态**:
   - 新增 `get_active_chat` IPC，`ChatBubble.vue` 挂载时同时读取历史记录和当前 active chat
   - 如果 AI 仍在思考，重开 `chat` 窗口会恢复用户刚发的消息、已产生的思考内容和 loading 状态
   - `ai-finished` / `ai-error` 事件负责在窗口打开时补上最终回复或错误状态
4. **验证记录**:
   - 已运行 `npm run build` 通过
   - 已运行 `cargo check` 通过

## 今日变更 (2026-05-16) — 背景与本体深度交互升级

1. **聊天动态背景系统全面重构**：
   - 弃用简易静态 CSS 背景，全线升级为基于 Canvas 的高性能动态渲染引擎（`BgCanvas.vue`）。
   - **科幻模式 (Sci-fi)**：深空底层与六边形网格，带有呼吸灯效果的神经元节点。节点距离靠近自动建立能量连线并发射数据包。支持全息扫描线和跟随鼠标的旋转准星；点击爆发 EMP 震荡波推开节点。
   - **可爱模式 (Cute)**：物理风摆旋转飘落的樱花花瓣与缓慢上浮的彩色半透明气泡，气泡会被鼠标磁场吸引；移动留有金色星光尾迹；点击爆出大量爱心与花瓣。
   - **简洁模式 (Minimal)**：等距点阵如同星图，鼠标靠近自动发光并连线成“星座”；点击触发星座射线爆发。
2. **聊天气泡可读性优化**：
   - 为解决复杂高光背景干扰文字的问题，为气泡引入毛玻璃（Frosted Glass）材质（`backdrop-filter: blur(4px)`）。
   - AI 回复气泡下方垫入 45% 透明度纯白打底，并增加微弱发光阴影（Text Shadow），在保留通透感的同时确保文字绝对清晰。
3. **桌宠本体深度交互 (PetCanvas.vue)**：
   - **眼球追踪与感知**：桌宠瞳孔实时朝鼠标相对位置微调偏移；当鼠标位于左/右侧时身体有轻微旋转倾斜。
   - **动作反馈**：鼠标悬停在桌宠时，耳朵抖动加快并产生火花粒子；点击桌宠触发带有重力物理模拟的弹跳（Bounce）；长按超过 500ms 触发受压变形（Squash）。
   - **防走失与 Bug 修复**：修复因 `ctx.restore()` 缺失导致的“桌宠飞天”问题；新增画布双击（`dblclick`）直接重置窗口到屏幕中心功能。

## 已知待实现

1. **打包分发**: `npm run tauri build` 生成安装包
2. **多窗口边缘场景**: 需补充 chat/settings/context-menu 在屏幕边缘的防溢出定位策略
3. **Claude Code Skill 化文件分类**: 建议新增 `file-intake-classifier` Skill，而不是在桌宠项目内实现分类数据库
4. **剪切板二次体验**: 可继续增加复制成功 toast、删除确认/撤销、更多筛选和批量管理
5. **安全与发布**: 打包前建议移除或配置化 `--dangerously-skip-permissions`，并补充 Tauri CSP

## 启动方式

```powershell
# 1. 安装依赖（首次）
cd D:\ai-desktop-pet
npm install

# 2. 启动开发模式（需要 VS Build Tools 环境）
npm run tauri dev
```

> 注意：Tauri 编译需要 VS Build Tools 2022 (C++ 桌面开发工作负载)，已安装。

## 窗口交互模型

- **主窗口 `main`**: 120x140，刚好包住宠物，透明+无边框+无阴影+置顶+跳过任务栏，只渲染 `PetCanvas`
- **拖拽**: 用 `setPosition()` 手动移动主窗口 + `dragFromCanvas` 标志隔离非桌宠事件
- **左键点击**: 在桌宠本体命中区域触发，创建/关闭独立 `chat` 窗口；主窗口不扩展
- **`chat` 窗口**: 340x400，透明无边框，默认定位到 `main.x + 120 + 8`；顶部可在"对话"与"剪切板"之间切换
- **剪切板模式**: 输入内容只保存到本地 `clipboard_items`，不发送给 AI；支持搜索、复制、置顶、删除和一键清空
- **文件拖入**: 拖文件到 `chat` 窗口区域会将路径注入输入框，由 Claude Code / Skill 读取处理
- **右键点击**: 在桌宠本体命中区域触发，创建独立 `context-menu` 窗口并定位到鼠标处；菜单失焦自动关闭
- **设置窗口**: 从右键菜单打开独立 `settings` 窗口，默认定位到宠物右侧；不再撑大主窗口
- **面板跟随**: 拖动主窗口时，已打开的 `chat` / `settings` 会跟随宠物右侧重新定位
- **透明穿透**: 主窗口仍保留桌宠形状 hit-test，只让 `120x140` 内的透明像素穿透；大透明矩形已由多窗口拆分根除

## 注意事项

- 桌宠需要 Claude Code 已登录（`claude` CLI 可用），否则 AI 功能不可用
- SQLite 数据存储在 `C:\Users\15188\AppData\Roaming\ai-desktop-pet\pet.db`
- `clipboard_items` 与聊天记录共用同一个 SQLite 数据库；清空对话与清空剪切板互不影响
- 前端通过 `invoke()` 调用 Rust 命令，通过 `setInterval(syncState, 1000)` 每秒同步状态
- Tauri 权限配置在 `src-tauri/capabilities/default.json`，新增 API 需在此添加权限

## 今日变更 (2026-05-17) — 语音交互与稳定性修复

1. **中文乱码统一修复**：
   - 修复 `ChatBubble.vue` 中对话、思考状态、剪切板按钮、空状态等用户可见中文乱码
   - 修复 `Settings.vue`、`ContextMenu.vue`、`pet.ts` 中的主题名、菜单文案、设置项中文乱码
   - 修复 Rust 后端 `lib.rs` 中会透传到前端的异常文案，如“AI 正在思考中”“已中止”“启动 Claude 失败”
2. **语音交互 MVP 接入**：
   - 新增 `src/services/voice.ts`，封装 Web Speech 语音识别、语音播报、设置解析与默认值
   - 设置页新增语音配置：启用语音、自动发送、自动朗读、语言、声音、语速、音调、音量
   - 桌宠状态新增并补齐 `listening / thinking / speaking / confused / dragging` 映射
3. **独立语音窗口落地**：
   - 新增 `src/components/VoicePanel.vue`，作为独立居中的语音窗口
   - 语音窗口支持实时字幕、动态声波、发送/重说/清空/关闭
   - 聊天框内麦克风按钮改为唤起独立语音窗口，而不是直接在聊天输入框内录音
4. **全局快捷键支持**：
   - 新增 Tauri 全局快捷键插件：`@tauri-apps/plugin-global-shortcut` 与 `tauri-plugin-global-shortcut`
   - 默认快捷键为 `Ctrl+Alt+V`，可在设置页中修改
   - 从任意状态按快捷键可直接打开语音窗口
5. **语音窗口显示修复**：
   - `VoicePanel.vue` 改为稳定的纵向 flex 布局，底部按钮固定占位
   - 压缩中部状态球与波形区域，文本框吸收剩余高度
   - `App.vue` 中 `voice` 窗口高度从 `470` 调整到 `520`，提升高 DPI / 缩放场景的安全余量
6. **Tauri 多窗口与权限扩展**：
   - `App.vue` 新增 `voice` 窗口 label 与调度逻辑
   - `src-tauri/capabilities/default.json` 新增 `voice` 窗口与 `global-shortcut`、`window:show`、`window:center` 等权限
   - `lib.rs` 中注册 `tauri_plugin_global_shortcut`
7. **验证记录**：
   - `npm run build` 通过
   - `cargo check` 通过
   - `npm run tauri build` 通过
   - 新版已打包并启动：`D:\ai-desktop-pet\src-tauri\target\release\ai-desktop-pet.exe`
8. **仓库状态说明**：
   - 当前目录 `D:\ai-desktop-pet` 不存在 `.git` 目录，已确认不是 Git 工作树
   - 因此本轮无法执行 `git status`、`git add`、`git commit`
   - 如后续需要版本提交，需先恢复原仓库的 `.git` 元数据，或明确要在当前目录重新初始化 Git 仓库
