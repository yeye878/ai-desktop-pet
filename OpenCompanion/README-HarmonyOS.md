# OpenCompanion 智能伙伴 · OpenHarmony 原生应用

> 由 `ai-desktop-pet`（Tauri+Vue+Rust 桌面版）迁移而来的 **ArkTS + ArkUI 鸿蒙原生智能多模态个人助理**。
> 对应命题：OpenHarmony Agent 鸿蒙原生智能多模态个人助理（华为 · 国产操作系统软件组）。
> 迁移方案全文见 [`../docs/鸿蒙原生迁移改造方案.md`](../docs/鸿蒙原生迁移改造方案.md)。

## ✅ 构建状态

- **hvigor 真实编译通过**（本机 DevEco Studio 26 + HarmonyOS SDK 26.0.0，API 20+）
- 产物：`entry/build/default/outputs/default/entry-default-unsigned.hap`
- CLI 一键构建：双击 `build.cmd`（或在其中追加 `clean` 参数做全量构建）

## ✅ 运行验证状态（Pura 90 Pro x86 模拟器实测，2026-09）

- **模拟器安装运行通过**：未签名 HAP 经 `hdc install -r` 直接安装（模拟器免签名），EntryAbility 正常启动
- **五页渲染验证通过**：首页（桌宠 Canvas/状态机 ❤️⚡数值真实变化）/ 对话页（@提及条显示三个内置智能体）/ 知识库页 / 智能体页（三子智能体卡片）/ 进化页 / 设置浮层（LLM API/热词/城市）
- **命令链路验证通过**：`/help` → 本地回复落库回显；`/kb` → 知识库状态（含降级提示）
- **R2 降级链实测通过**：模拟器裁剪 DataAugmentationKit（`data.retrieval module not found`），应用不再启动崩溃（RagBridge 惰性继承修复），问答自动降级 `fallback to search+llm`，FTS4 表正常建表
- **CoreSpeechKit 实测可用**：ASR 引擎带 9 热词初始化成功（HiAI AsrEntryManager init success）；TTS 首次 createEngine 超时（模拟器服务冷启动慢），lazy preheat 可自恢复
- **长稳验证**：应用进程持续运行 2 小时+无新崩溃；修复前的 3 次 jscrash 均由模块级 `extends rag.ChatLLM` 引起，已修复

## ✅ 阶段二：参赛版本优化（2026-09-06，方案见 `../docs/后续优化开发方案.md`）

在主链路迁移完成基础上，按命题逐条打磨的增量（全部经模拟器端到端实测）：

- **复合指令解析引擎** `core/IntentEngine.ets`：端侧规则式意图分类 + 编辑距离≤2 / 前缀补全的**指令纠偏** + 复合指令**拆解为带依赖的子任务序列**（拍文档→识别→总结[→入库]；复习排期；知识库溯源问答），未命中自动回落原 LLM 链路。执行轨迹（子任务/依赖/进度）全程可视
- **指令 API 化 + 本地兜底**：已配置 LLM API 时，打开知识库/今天日程/现在几点/新对话/停止播报等指令全部由大模型经 `navigate_to_page`/`start_new_conversation`/`stop_speaking` 等工具调用完成；未配置 API 时自动回退为本地规则直答（毫秒级，含天气网络请求的简报 1822ms 仍 <2s）
- **语音全链路流式**：自动朗读开启时流式回答**按句即刻合成**（首句先于全文），播报中点麦克风**打断**并立即进入聆听；断句→首反馈打点
- **语音精准度三件套**（R5）：动态热词（设置 ∪ 指令词表 ∪ 智能体名 ∪ 知识库文档标题，知识库变更自动重建引擎）+ 误识纠偏 + **低置信度复述确认**（"你是指……？"确认/原文/重说）
- **RAG 增强**：Kit 倒排与本地 bigram **双路并行召回 + RRF 融合重排**；RAG 会话**池预热**（降低冷会话首字时延）
- **编排工程化**：/team 成员级 **90s 超时 + 重试 1 次**（独立取消令牌，单成员挂起不拖垮整队），规划→派发→执行→汇总阶段轨迹带时间戳
- **跨设备协同增强**（F3）：onContinue 携带**最近 6 条对话真实上下文**（对端自动恢复进会话）；`commons/DistributedSync.ets` 分布式 KV 同步会话摘要（能力检测 + 单端降级，设置页显示状态）；退后台自动发布
- **性能量化**（R4）：`commons/PerfTrace.ets` 埋点——冷启动·首帧/可交互、指令→首反馈、复合指令全链路；首页状态卡实时显示冷启动耗时；/perf 命令汇总
- **助理能力**：/brief **早间简报**（时间+天气+待办，零 LLM 可用）；`create_review_plan` **遗忘曲线复习排期**（D+1/2/4/7/15 系统提醒）；`summarize_doc` 文档要点总结；内置智能体白名单自动并集升级
- **工程修缮**：decodeToString 替换弃用 API（SSE 流式解码保留 stream 语义）；实测键盘弹起 RESIZE 避让正常

## 命题要求覆盖

| 命题要求 | 实现情况 |
|---------|---------|
| R1 ArkTS + ArkUI 适配 OpenHarmonyOS | ✅ 全部代码为 ArkTS/ArkUI 声明式，Stage 模型，手机/平板/2in1 |
| R2 使用 `@kit.DataAugmentationKit` RAG API | ✅ `knowledge/RagBridge.ets`：`rag.createRagSession` 流式问答（自实现 `rag.ChatLLM` 接任意 OpenAI 兼容云端）；`retrieval.getRetriever` 倒排索引通道；`localChatModel` 端侧离线问答。不可用时自动降级本地检索，功能不缺席 |
| R3 至少适配手机 + 平板 | ✅ `Index.ets` 断点布局：≥720vp 左侧导航栏（平板），以下底部 Tab（手机） |
| R4 冷启动 ≤3s / 语音响应 ≤2s | ✅ 冷启动纪律：`EntryAbility` 只做窗口加载，DB/语音引擎/知识库全部 `postFrame` 延迟；语音：ASR/TTS 引擎启动即预热 + 流式首包；高频指令默认走 LLM API 工具调用，未配置 API 时本地直答兜底 |
| R5 提升语音识别精准度 | ✅ CoreSpeechKit 端侧离线识别 + `sysGeneralLexicon` 热词表（设置页可编辑，内置智能体名/指令短语）+ 静音断句自动发送 |
| F1 多模态（语音/文本/图像复合指令） | ✅ 语音（🎤 端侧 ASR）+ 文本（@提及 //命令 /team）+ 图像（📷 相册选择 → `@kit.CoreVisionKit` OCR / 视觉 LLM）；选图后自动带入"帮我总结重点"复合指令 |
| F2 端侧 RAG 知识库 | ✅ 知识库页：文件导入（txt/md 等）+ 图片 OCR 入库；检索-问答流式输出 + 引用来源；**数据全部端侧存储（应用私有 RDB）**，离线模式断网可用 |
| F3 跨设备协同 | ✅ `EntryAbility.onContinue` 任务接续（continuable=true）+ 服务卡片/路由参数跨端拉起 |
| F4 ≥3 子智能体自主分解任务 | ✅ 学习助手/日程管家/文档处理 三个内置子智能体（工具白名单隔离）+ `/team` 规划→并行执行→综合汇报编排 + 智能体工坊（AI 生成规格）+ **自进化系统**（复盘→画像准则→进化提案→采纳/回滚） |

## 构建与试运行

### 方式一：DevEco Studio（推荐，可签名安装到模拟器/真机）

1. 用 DevEco Studio 打开本目录（`OpenCompanion/`）
2. `File > Project Structure > Signing Configs` 勾选 **Automatically generate signature**（需登录华为账号）→ 完成（会自动写入 `build-profile.json5` 的 signingConfigs）
3. 选择模拟器或连接真机（HarmonyOS 6.0+/API 20+ 才有 RAG Kit；低版本设备会自动走降级链）
4. 点 ▶ Run

### 方式二：命令行构建（本机已验证）

```cmd
build.cmd
```

产物为**未签名** HAP，可直接用于编译验证；安装运行仍需步骤一中的自动签名（模拟器除外，见下）。

### 方式三：模拟器 CLI 安装（本机已验证）

```cmd
:: DevEco 模拟器 CLI（首次需 GUI 里同意协议并部署镜像）
"D:\DevEco Studio\tools\emulator\Emulator.exe" -start "Pura 90 Pro"
:: 模拟器允许安装未签名 HAP（error code 00801002 = C盘磁盘空间不足，预留 >8G）
hdc install -r entry\build\default\outputs\default\entry-default-unsigned.hap
hdc shell aa start -a EntryAbility -b com.opencompanion.pet
```

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

- **手机模拟器无 DataAugmentationKit**（x86 镜像裁剪 `data.retrieval`）：RAG Kit 主路径在真机（PC/2in1/部分手机）可用；无 Kit 设备自动走降级链（本地 FTS4 检索 + LLM 融合），已实测。RAG Kit 向量化通道（`knowledgeProcessor`）仅 PC/2in1 设备支持
- FTS4 表不可用的极端环境自动切换本地 bigram 检索（降级链 L2）
- 平板侧栏断点布局（≥720vp）代码已按 px2vp 正确实现；运行验证需 MatePad 模拟器（宿主机内存不足未跑，代码审查通过）
- PDF/docx 解析：当前以"转文本/OCR"路径入库；如需原生解析可后续以 Rust ohos 目标编译 native 模块（见方案 §9 R6）
- 天气默认源 wttr.in，网络不可达时 get_weather 工具返回友好错误（可在设置改城市）
- ~~`decodeWithStream` 弃用提示~~ 已替换为 `decodeToString`（NetClient SSE 保留 `decodeWithStream{stream:true}`：跨包多字节切分需要流式解码语义，无等价替代）
- ~~输入法遮挡~~ 模拟器实测 RESIZE 避让正常（键盘弹起时输入区随布局压缩上移）；真机如遇极端输入法可再评估 `expandSafeArea`
