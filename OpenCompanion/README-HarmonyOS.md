# OpenCompanion 智能伙伴 · OpenHarmony 原生应用

> 由 `ai-desktop-pet`（Tauri+Vue+Rust 桌面版）迁移而来的 **ArkTS + ArkUI 鸿蒙原生智能多模态个人助理**。
> 对应命题：OpenHarmony Agent 鸿蒙原生智能多模态个人助理（华为 · 国产操作系统软件组）。
> 迁移方案全文见 [`../docs/鸿蒙原生迁移改造方案.md`](../docs/鸿蒙原生迁移改造方案.md)。

## ✅ 构建状态

- **hvigor 真实编译通过**（本机 DevEco Studio 26 + HarmonyOS SDK 26.0.0，API 20+）
- 产物：`entry/build/default/outputs/default/entry-default-unsigned.hap`
- CLI 一键构建：双击 `build.cmd`（或在其中追加 `clean` 参数做全量构建）

## 命题要求覆盖

| 命题要求 | 实现情况 |
|---------|---------|
| R1 ArkTS + ArkUI 适配 OpenHarmonyOS | ✅ 全部代码为 ArkTS/ArkUI 声明式，Stage 模型，手机/平板/2in1 |
| R2 使用 `@kit.DataAugmentationKit` RAG API | ✅ `knowledge/RagBridge.ets`：`rag.createRagSession` 流式问答（自实现 `rag.ChatLLM` 接任意 OpenAI 兼容云端）；`retrieval.getRetriever` 倒排索引通道；`localChatModel` 端侧离线问答。不可用时自动降级本地检索，功能不缺席 |
| R3 至少适配手机 + 平板 | ✅ `Index.ets` 断点布局：≥720vp 左侧导航栏（平板），以下底部 Tab（手机） |
| R4 冷启动 ≤3s / 语音响应 ≤2s | ✅ 冷启动纪律：`EntryAbility` 只做窗口加载，DB/语音引擎/知识库全部 `postFrame` 延迟；语音：ASR/TTS 引擎启动即预热 + 高频指令本地直答（/命令）+ 流式首包 |
| R5 提升语音识别精准度 | ✅ CoreSpeechKit 端侧离线识别 + `sysGeneralLexicon` 热词表（设置页可编辑，内置智能体名/指令短语）+ 静音断句自动发送 |
| F1 多模态（语音/文本/图像复合指令） | ✅ 语音（🎤 端侧 ASR）+ 文本（@提及 //命令 /team）+ 图像（📷 相册选择 → `@kit.CoreVisionKit` OCR / 视觉 LLM）；选图后自动带入"帮我总结重点"复合指令 |
| F2 端侧 RAG 知识库 | ✅ 知识库页：文件导入（txt/md 等）+ 图片 OCR 入库；检索-问答流式输出 + 引用来源；**数据全部端侧存储（应用私有 RDB）**，离线模式断网可用 |
| F3 跨设备协同 | ✅ `EntryAbility.onContinue` 任务接续（continuable=true）+ 服务卡片/路由参数跨端拉起 |
| F4 ≥3 子智能体自主分解任务 | ✅ 学习助手/日程管家/文档处理 三个内置子智能体（工具白名单隔离）+ `/team` 规划→并行执行→综合汇报编排 + 智能体工坊（AI 生成规格）+ **自进化系统**（复盘→画像准则→进化提案→采纳/回滚） |

## 构建与试运行

### 方式一：DevEco Studio（推荐，可签名安装到模拟器/真机）

1. 用 DevEco Studio 打开 `D:\ai-desktop-pet\OpenCompanion`
2. `File > Project Structure > Signing Configs` 勾选 **Automatically generate signature**（需登录华为账号）→ 完成（会自动写入 `build-profile.json5` 的 signingConfigs）
3. 选择模拟器或连接真机（HarmonyOS 6.0+/API 20+ 才有 RAG Kit；低版本设备会自动走降级链）
4. 点 ▶ Run

### 方式二：命令行构建（本机已验证）

```cmd
build.cmd
```

产物为**未签名** HAP，可直接用于编译验证；安装运行仍需步骤一中的自动签名。

### 运行后 3 分钟配置

1. **设置（⚙）→ LLM API 配置 → ＋添加**：填任意 OpenAI 兼容服务（Base URL / 模型 / API Key，密钥入系统资产安全存储）→ 测试连接 → 保存并选中
2. 首页确认"✅ LLM API 已配置"
3. 知识库页导入 1-2 个文档（课程笔记/会议纪要的 txt/md），或📷拍照识字入库

### 演示脚本（对应命题四大能力）

| 能力 | 操作 |
|------|------|
| 多模态复合指令 | 对话页 📷 选一张文档照片 → 自动填入"看一下这个文档，帮我总结重点" → 发送（OCR/视觉理解 + 总结） |
| 端侧 RAG | 知识库页导入文档 → 提问 → 看流式回答 + 📌引用来源；打开"离线模式(端侧模型)"后可断网问答 |
| 跨设备接续 | 手机/模拟器 A 对话到一半 → 任务中心点应用"接力"图标流转到设备 B |
| 智能体编排 | 对话页发送 `/team 准备下周的智能体竞赛答辩` → 规划→三智能体并行→综合汇报；`@学习助手` 单独点名；进化页点"立即复盘"看提案并采纳 |

## 目录结构（迁移映射）

```
entry/src/main/ets/
├─ commons/   Log·EventBus(对应emit/listen)·AppContext(pet_settings KV)·RdbHelper(9表schema平移)
│             AssetStore(keyring→系统资产存储)·NetClient(reqwest SSE→@ohos.net.http dataReceive)
├─ core/      Types·LlmClient(api_client.rs平移)·AgentLoop(agent.rs ReAct循环)·ToolRegistry(tools.rs收敛22工具)
│             Gates(确认/ask_user)·ChatService(send_to_ai全链路)·AgentRepo(智能体/技能/记忆)
│             TeamOrchestrator(collab/三段编排)·EvolutionManager(evolution/复盘-提案-回滚)
├─ knowledge/ KbService(端侧RAG+降级链)·RagBridge(rag会话+端侧模型)·DocImporter(文件导入+CoreVisionKit OCR)
├─ voice/     AsrController(CoreSpeechKit端侧ASR+热词)·TtsController(CoreSpeechKit端侧TTS,分句播报)
├─ pet/       BehaviorEngine(behavior/状态机平移)·ThemeEngine(7套主题)·PetCanvas(ArkUI Canvas桌宠)
├─ pages/     Index(断点壳)·Home·Chat(对话+语音+确认+ask_user)·Knowledge·Agents·Evolution·Settings
├─ entryability/    EntryAbility(冷启动纪律+onContinue接续)
├─ entryformability/ PetFormAbility(服务卡片,替代桌面版悬浮窗)
└─ widget/pages/PetCard.ets  2×2桌宠卡片
```

**裁剪项**（鸿蒙无对应开放能力，桌面版专属）：Computer Use、CDP 浏览器、悬浮窗、全局快捷键、子进程面板。

## 已知限制 / 后续路线

- RAG Kit 向量化通道（`knowledgeProcessor`）仅 PC/2in1 设备支持；手机走倒排索引（BM25）+ 端侧/云端问答，已按 SDK 官方约束设计
- FTS4 表不可用的极端环境自动切换本地 bigram 检索（降级链 L2）
- PDF/docx 解析：当前以"转文本/OCR"路径入库；如需原生解析可后续以 Rust ohos 目标编译 native 模块（见方案 §9 R6）
- 天气默认源 wttr.in，网络不可达时 get_weather 工具返回友好错误（可在设置改城市）
- `decodeWithStream` 有弃用提示（功能正常），后续可换 `decodeToString`
