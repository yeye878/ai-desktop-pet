# frontend-dev 任务说明

你是 ai-desktop-pet 项目的实现工程师（前端方向）。工作目录 D:/ai-desktop-pet（Windows，Tauri 2 + Vue3 + TypeScript + Vite）。

## 背景
用户反馈（截图）：agent 窗口顶部的「模型」下拉只显示硬编码的 deepseek-chat / deepseek-reasoner，没有显示本机 dsh 真实拥有的模型，也没有模型可选的思考强度（reasoning effort）；codex、claude 窗口同样如此。
本轮把三个 agent 窗口（claude/codex/dsh）的模型选择器改为读取本机后端真实模型目录，并支持按模型选择思考强度，选择结果写入会话（后端队友负责 Rust 侧）。
截图现状：顶部一条 pill 形控件「AGENT | dsh」「模型 | 跟随本机 ▼」「● 就绪」「会话」；模型 pill 点开只有一个原生 datalist，只有 deepseek-chat / deepseek-reasoner。

## 必读
1. docs/agent-model-catalog-contract.md —— 接口契约，严格按它实现（第 2/6/7 节是你的范围）。
2. 任务 task-2：先 team_task_get 读取，team_task_update action=claim expected_revision=1 认领；完成时 action=complete。

## 你的写作用域（只改这些）
- src/components/**（新增 AgentModelPicker.vue；改 Dashboard.vue）
- src/services/**（新增 agentModels.ts）
- src/stores/**（sessions.ts）
- src/dev-tauri-mock.ts、src/assets/dashboard.css、scripts/verify-agent-model-catalog.cjs、package.json
不要改 src-tauri/**（后端队友在改）。仓库里有大量与本次无关的未提交改动：不要 git commit/checkout/stash/revert。

## 关键文件位置（已探明）
- Dashboard.vue 共 5566 行：AGENT_MODEL_PRESETS L108-113、agentModelDraft L162、agentModelPresets L174、AgentWindowPreference/readStoredWindowPreference L133-148、currentSessionModelLabel L2060、agent 窗口头部模板 L3902-3928（含 input+datalist）、底部信息条 L4220-4223、createChatSession 与 commitAgentModel/onAgentWindowAgentChange 请 grep 定位。
- src/stores/sessions.ts：ChatSession/CreateSessionOptions/BindSessionPatch/create/bind。
- src/dev-tauri-mock.ts：MockChatSession 类型、mock 会话存储、handleMockCommand 的 switch（L628-1090 区域），新增 case 与 fixture。
- src/assets/dashboard.css：已有 .dash-agent-field / .dash-agent-select / .dash-agent-model-input 等样式，新样式沿用设计 token。
- scripts/verify-multi-agent.cjs：node:test + typescript transpile + vm 的模块加载模式，照它写新验证脚本。

## 实现要点
见契约第 6 节。补充约束：
- 弹层视觉要与现有 pill / 圆角卡片风格一致（浅色、细边框、轻阴影），不要破坏小窗布局；弹层宽度约 300-340px，最大高度约 420px，内部滚动。
- 三个 agent 窗口共用同一个 AgentModelPicker；main 主对话不使用它。
- 模型选择立即生效：有当前会话时 sessions.bind(sessionId, { model, reasoningEffort })，无会话时存窗口偏好；新建会话时带上偏好 model 与 reasoningEffort。
- effort 切换后若新模型不支持该 effort，回退空串（跟随本机默认）。
- 底部信息条保留「模型：xxx」，effort 非空时追加「 · 思考：中文名」。
- 删除不再使用的旧函数/常量，注意 tsconfig 严格检查（noUnusedLocals）。
- dev-tauri-mock 的 get_agent_model_catalog fixture 必须与契约完全一致，三种 slot 都要有（dsh 多 provider 分组字段 provider/provider_label；codex/claude 不需要 provider）。
- 验证脚本用 vm 加载 TS 纯函数并断言 Dashboard.vue 模板不再含 datalist/AGENT_MODEL_PRESETS、含 AgentModelPicker、sessions 透传 reasoning_effort、mock 三 slot fixture 齐全。

## 验收（必须全部执行并贴结果给我）
1. npm run build（vue-tsc --noEmit + vite build）通过。
2. node --test scripts/verify-agent-model-catalog.cjs 全绿。
3. 自查：Dashboard.vue 不再出现 datalist 与 AGENT_MODEL_PRESETS；三个 slot 在 mock 下打开选择器都能看到真实模型与思考强度。

## 协作与收尾
- 后端接口契约已冻结；若实现中发现契约缺口，立即 send_message 给 target="lead"，不要自行改契约或改 src-tauri。
- 完成后：team_task_update action=complete，并 send_message 给 target="lead" 汇报：改了哪些文件、npm run build 与 node --test 结果摘要、UI 交互说明、契约偏差/遗留问题。
- 遇到阻塞先自行排查，超过 15 分钟仍阻塞再汇报。
