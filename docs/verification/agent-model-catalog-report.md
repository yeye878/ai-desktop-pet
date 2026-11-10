# Agent 模型目录 + 思考强度 — 独立验收报告

- **验收对象**：task-1（后端 Rust + pet-runner，backend-dev）/ task-2（前端 AgentModelPicker + Dashboard，frontend-dev）
- **验收依据**：docs/agent-model-catalog-contract.md（v1）、docs/prompts/verifier.md 验收清单
- **验收人**：verifier（独立于实现者，未信任实现者自测结论）
- **时间**：2026-11-10 04:47 ~ 05:10（Asia/Tokyo）
- **工作目录**：D:/ai-desktop-pet（Windows 11，Tauri 2 + Vue3 + TS + Rust；Chrome 156.0.8078.81 headless + CDP）
- **结论**：**通过**（0 个必须修复项；4 条建议改进，均不阻断用户原始诉求）
- **是否满足用户原始诉求**：**满足**。三个 agent 窗口（claude / codex / dsh）的模型下拉已改为读取本机真实模型目录，并按模型提供思考强度（reasoning effort）；选择会写入会话绑定 / 窗口偏好，并在后续回合通过 CLI 参数（claude --effort、codex -c model_reasoning_effort=...、dsh runner --effort）生效。**残留验证边界**：按任务要求未跑真实模型 token 回合（省钱省时），"生效链路"验证到参数构造（Rust 单测）与真实目录/runner 冒烟层，未做真实模型返回内容级验证。

---

## 1. 验收方法与环境

| 项 | 值 |
| --- | --- |
| 独立复跑 | 构建/单测/真机/浏览器全部在 verifier 会话内重新执行，未复用实现者日志（仅用其 .ws-probe 产物做基线对比） |
| 后端真机 | 本机 dsh CLI（profile dsh-pet）+ codex CLI + ~/.claude/settings.json / ~/.codex/config.toml |
| 浏览器 E2E | Vite dev server（1420 strictPort，DEV 下 main.ts 自动装 Tauri mock）+ 本机 Chrome headless + CDP（Node 24 内置 WebSocket，无新增依赖） |
| 仓库状态 | 仓库含大量与本轮无关的既有未提交改动；本轮改动文件用 mtime（2026-11-10 04:25~04:45）识别，见 §2 |
| 未执行 | npm run tauri build（Lead 负责）、git commit/checkout/stash/revert（均未执行） |

---

## 2. 静态审查（改动范围 + 契约核对）

### 2.1 本轮改动文件（按 mtime 识别，均在预期写作用域内）

**后端（task-1，写作用域 src-tauri/src/** + src-tauri/dsh/pet-runner.mjs）**

- 新增：src-tauri/src/agent_models.rs（493 行）
- 修改：src-tauri/src/codex.rs、dsh.rs、lib.rs、storage/db.rs、openclaw/client.rs、collaboration/runtime.rs（仅适配新增 effort 形参的调用点，传 None）、src-tauri/dsh/pet-runner.mjs
- 未改 Cargo.toml / Cargo.lock（无新增 Rust 依赖）

**前端（task-2，写作用域 src/components、src/services、src/stores、src/dev-tauri-mock.ts、dashboard.css、scripts、package.json）**

- 新增：src/components/AgentModelPicker.vue（357 行）、src/services/agentModels.ts（172 行）、scripts/verify-agent-model-catalog.cjs、scripts/verify-chat-sessions.cjs（仅补 create_chat_session 断言的 reasoningEffort: ""，全文仅此 1 处 reasoning 字段，见 §2.3）
- 修改：src/components/Dashboard.vue、src/dev-tauri-mock.ts、src/stores/sessions.ts、src/assets/dashboard.css、package.json
- 未改 package.json dependencies/devDependencies（仅新增 3 个 verify script，见 §5 已知无关项）

**意外改动检查**：未发现本轮改动越界（src-tauri/src/** 与前端各写作用域均被 task 授权）；Dashboard.vue 中 datalist / AGENT_MODEL_PRESETS / agentModelPresets 已全部清除（grep 无匹配）。

### 2.2 契约逐条核对（静态审查）

| 契约项 | 结论 | 证据（文件:行） |
| --- | --- | --- |
| §2 目录结构字段（slot/backend/local_model/local_model_label/local_effort/models/error/fetched_at；effort 含 id/name/description） | 通过 | agent_models.rs:20-62；serde 输出 snake_case；前端 agentModels.ts:31-40 一致 |
| §2 60s 缓存 + force 绕过 | 通过 | agent_models.rs:16,113-162（Mutex 缓存、force 跳过） |
| §2 探测失败返回正常结构（error 非空、models=[]）不抛错 | 通过 | agent_models.rs:89-94,164-173；lib.rs 命令仅 slot 非法才 Err |
| §2 总探测超时 20s | 通过 | agent_models.rs:18,165；codex/dsh 各自亦有 20s 子超时 |
| §2 dsh 只返回当前本机 provider 模型 | 通过 | pet-runner.mjs:113-174（selection.provider -> llm.listModels(provider)）；真机输出 provider=deepseek-official |
| §2 codex：debug models JSON + visibility=list + priority 排序 + config.toml 顶层 model/effort | 通过 | codex.rs:546-574,589-664,682-716；真机 9 模型且 local=gpt-6.1-terra/xhigh |
| §2 claude：settings.json 解析 + 别名 + modelSettings 完整模型名 + env 具体模型 id + 固定 5 档 efforts | 通过 | agent_models.rs:184-398；单测 claude_catalog_exposes_aliases_mapped_models_and_efforts |
| §3 chat_sessions.reasoning_effort 列 + 幂等迁移 | 通过 | storage/db.rs:443,453,537-550；单测 legacy_chat_sessions_gain_an_empty_reasoning_effort_column |
| §3 ChatSession/NewChatSession 字段 + serde 默认空串 + JSON 字段名 reasoning_effort | 通过 | storage/db.rs:135-157,170-179 |
| §3 update_chat_session effort: Option<&str>（None 不改，Some("") 清空） | 通过 | storage/db.rs:1037-1086；单测 updating_a_session_writes_binding_model_and_cli_ids |
| §3 命令参数 create(slot,agentId,model,reasoningEffort,title) / bind(id,agentId,agentName,agentAvatar,model,reasoningEffort) | 通过 | lib.rs:4126-4131,4254-4260（Tauri 默认 camelCase 转换，sessions.ts:107-149 传参一致） |
| §3 模型变更清 CLI 续接 id，仅 effort 变化不清空 | 通过 | lib.rs:4310-4325 |
| §4 claude 非空 effort -> --effort | 通过 | openclaw/client.rs:94-135,219 + 单测 claude_flags_carry_model_effort_and_resume |
| §4 codex 非空 effort -> -c model_reasoning_effort="<id>" 且位于 resume 前（含引号/换行清洗） | 通过 | codex.rs:69-100；单测 reasoning_effort_is_passed_as_a_codex_config_override |
| §4 dsh 非空 effort -> runner --effort，位于 resume 前 | 通过 | dsh.rs:392-419；单测 passes_an_explicit_reasoning_effort_to_the_runner |
| §4 回合调用链传入会话 reasoning_effort（direct_api 不传） | 通过 | lib.rs:2728-2729,2756-2792,3275-3285；collaboration/runtime.rs 角色调用传 None |
| §5 runner --list-models 协议行 +JSON、不启动 stdin、异常也输出结构 | 通过 | pet-runner.mjs:260-288,113-179；dsh.rs:1166-1328；真机冒烟通过 |
| §5 --effort 先校验模型 efforts、不支持则 log 忽略 | 通过 | pet-runner.mjs:181-197,302-305 |
| §5 帮助文本同步 | 通过 | pet-runner.mjs:52-66 |
| §6 前端 service：类型、缓存+进行中去重、effortLabel、modelOptionFor、effortOptionsFor | 通过 | agentModels.ts:1-172 |
| §6 选择器交互：跟随本机行/分组/选中勾/思考强度 chips/自定义输入/刷新/loading/error/外部+Escape 关闭 | 通过 | AgentModelPicker.vue:239-357；浏览器 E2E 实测（§4） |
| §6 新模型不支持当前 effort 时回退 ""；目录外自定义模型保留 effort | 通过 | AgentModelPicker.vue:88-99；verify-agent-model-catalog.cjs 对应用例 + E2E |
| §6 窗口偏好增加 effort（兼容旧数据）/ 选中回调 bind 或存偏好 / 新建会话带 model+effort / 底部信息条追加思考 | 通过 | Dashboard.vue:128-175,191-195,2054-2069,2256-2287,2363-2387,2408-2428,4236-4238 |
| §6 sessions store 透传 reasoning_effort | 通过 | src/stores/sessions.ts:19-40,107-150 |
| §6 dev mock 三 slot fixture 与契约一致 + create/bind 落库 | 通过 | dev-tauri-mock.ts:621-765,779-783,803-818,862-879；verify 脚本断言 |
| §6 CSS 追加 dash-agent-* 样式 | 通过 | src/assets/dashboard.css（新增 .dash-agent-model-* 样式，41 处匹配；E2E 渲染正常） |
| §7 单测覆盖解析/参数/迁移 | 通过 | cargo test 中 codex/claude/dsh/DB 相关用例均存在且通过（§3） |

### 2.3 scripts/verify-chat-sessions.cjs 检查

该文件在本轮前已存在（untracked 的既有未提交工作），本轮 mtime 04:44 的改动经内容核对：全文仅 1 处 reasoning 相关字段，位于 create_chat_session 调用的参数断言（第 266 行 args: { slot: "main", agentId: "", model: "", reasoningEffort: "", title: "" }），与 task-2 补充授权一致，无其他改动迹象。

---

## 3. 构建与单测（独立复跑）

所有命令在 D:/ai-desktop-pet 执行；日志在 .ws-probe/verifier/。

| # | 命令 | exit code | 结果摘要 |
| --- | --- | --- | --- |
| 1 | npm run build（vue-tsc --noEmit + vite build） | **0** | vue-tsc 无报错；vite 构建成功（index-hDeK4-1y.js 753.23 kB / index-Bx2OvL_q.css 267.61 kB，built in 4.21s），dist 已更新 |
| 2 | node --test scripts/verify-agent-model-catalog.cjs | **0** | tests 12 / pass 12 / fail 0（538ms） |
| 3 | node --test scripts/verify-chat-sessions.cjs | **0** | tests 12 / pass 12 / fail 0（6393ms） |
| 4 | node --test scripts/verify-multi-agent.cjs | **0** | tests 13 / pass 13 / fail 0（587ms） |
| 5 | cargo test --manifest-path src-tauri/Cargo.toml --lib | **101**（2 条已知 tts 网络失败） | 183 passed；2 failed；12 ignored；1.20s。失败仅 tts::tests::edge_synthesis_decodes_to_wav 与 tts::edge_tts::tests::test_synthesize，均为 "WebSocket 连接失败: HTTP error: 403 Forbidden" |
| 6 | cargo build --manifest-path src-tauri/Cargo.toml | **0** | Finished dev profile，0.67s；仅既有 warning（dead_code/未用 import 等 5 条） |

**tts 2 条失败与本轮改动无关的核实**：
- 实现者基线 .ws-probe/baseline-test.log 明确记录改动前即为 "test result: FAILED. 172 passed; 2 failed; 10 ignored"，失败用例同为上述 2 条，错误同为 HTTP 403 Forbidden；
- 本次复跑为 183 passed / 2 failed / 12 ignored（+11 passed 为后端新增单测，+2 ignored 为新增真机 ignored 测试），失败集合完全一致且均为外部网络 403，属环境性既有问题，**与本次改动无关**。

其它相关回归套件（可选）：scripts/verify-agent-model-catalog.cjs 覆盖 service 纯函数、组件、Dashboard 模板、sessions 透传、mock 三 slot fixture；以上均通过。

---

## 4. 真机后端验收

### 4.1 ignored 真机目录测试（Rust 命令层真实调用本机 CLI）

命令：cargo test --manifest-path src-tauri/Cargo.toml --lib local_model_catalog -- --ignored --nocapture
结果：**2 passed; 0 failed; 0 ignored; 195 filtered out; 2.84s；exit code 0**

- codex 目录：local=gpt-6.1-terra (GPT-6.1 Terra) effort="xhigh"，模型 9 个：
  ["gpt-6.1-sol@low", "gpt-6-astra@low", "gpt-6-sol@medium", "gpt-6-luna@medium", "gpt-5.6-sol@low", "gpt-6.1-terra@low", "gpt-5.6-terra@medium", "gpt-5.6-luna@medium", "gpt-5.5@medium"]
  （visibility=list 过滤、priority 升序与本机 config.toml 顶层 model/model_reasoning_effort 均符合契约）
- dsh 目录：local=deepseek-v4-flash (DeepSeek-V4-Flash) effort=""，模型 3 个

### 4.2 pet-runner 独立复跑（Rust 命令层之外的真机冒烟）

1. 复制 runner：src-tauri/dsh/pet-runner.mjs -> C:/Users/15188/.dsh/profiles/dsh-pet/plugins/pet-runner.mjs
   - 源/目标 SHA256 均为 51F7B73B590A06F41B71836F8F517038465DCB27074F68290B828C9C26E89878（一致）
2. dsh --profile dsh-pet --list-models -> **exit 0**，stderr 为空。stdout 输出一行协议：
   - provider=deepseek-official / provider_label=DeepSeek / model=deepseek-v4-flash / effort=""
   - models 3 个：deepseek-v4-flash（DeepSeek-V4-Flash）、deepseek-v4-pro（DeepSeek-V4-Pro）、deepseek-v4-flash-vision-exp（DeepSeek-V4-Flash-Vision-Exp）
   - 每个模型 efforts = off/low/high/max，default_effort=high，context_window=1000000
3. dsh --profile dsh-pet --list-models --effort high -> **exit 0**，stderr 为空，协议结构与上面一致（目录模式不受 --effort 影响，正常退出）

证据文件：.ws-probe/verifier/dsh-list-models.out / .err、dsh-list-models-effort.out / .err（PowerShell 重定向为 UTF-16LE，内容已逐字段核对）。

---

## 5. 浏览器端到端验收（mock 模式，重点）

### 5.1 方式

- 脚本：scripts/verify-agent-model-catalog-browser.cjs（**新增、可重复运行**；自管 vite + Chrome，结束 kill 两者并删临时 user-data-dir；若 1420 已有 dev server 则复用且不 kill 它）
- 环境：本机 Chrome 156.0.8078.81 --headless=new，CDP 端口 9223（Node 24 内置 WebSocket 直连，**未新增任何 npm 依赖**）
- 视图：1258x802 桌面视口（截图 8 张，见 §5.3）
- 覆盖：三个 slot 的「跟随本机/真实模型名/思考强度中文 chips」断言；点击外部关闭、Escape 关闭、再次打开命中前端缓存不重复请求目录、刷新按钮 force 重新探测；
  选模型+effort 后触发器/副标题/底部信息条/localStorage 窗口偏好更新；reload 后窗口偏好仍用于新建会话；自定义模型 ID 提交并原样显示；页面无 pageerror。
- 结果：**exit code 0**，页面未捕获异常 0，致命 console error 0（普通 console.error 也为 0）。

### 5.2 三个窗口实测结果

| slot | 弹层模型（真实名 + id） | 思考强度 chips（中文） | 选择操作 | 触发器 | 底部信息条 | localStorage 窗口偏好 |
| --- | --- | --- | --- | --- | --- | --- |
| dsh | DeepSeek-V4-Flash / deepseek-v4-flash；DeepSeek-V4-Pro / deepseek-v4-pro；DeepSeek-V4-Flash-Vision-Exp / deepseek-v4-flash-vision-exp；provider 分组 "DeepSeek"；本机默认：DeepSeek-V4-Flash | 跟随本机默认 / 关闭 / 低 / 高 / 最高 | DeepSeek-V4-Pro + 最高 | 模型 DeepSeek-V4-Pro · 思考：最高 | 模型：deepseek-v4-pro · 思考：最高 | {"agentId":"builtin-dsh","model":"deepseek-v4-pro","effort":"max"} |
| codex | GPT-6.1 Terra / GPT-6.1 Pro / GPT-6.1 / GPT-5.4 Codex / GPT-5.4 / GPT-5 Codex Mini；本机默认：GPT-6.1 Terra | 跟随本机默认 / 低 / 中 / 高 / 极高 / 最高 / 极限 | GPT-6.1 Pro + 极高 | 模型 GPT-6.1 Pro · 思考：极高 | 模型：gpt-6.1-pro · 思考：极高 | {"agentId":"builtin-codex","model":"gpt-6.1-pro","effort":"xhigh"} |
| claude | Opus / Sonnet / Haiku（别名，本机映射说明）；本机默认：Sonnet | 跟随本机默认 / 低 / 中 / 高 / 极高 / 最高 | Opus + 高；再自定义输入 claude-opus-5-5（Enter 提交） | 模型 Opus · 思考：高；自定义后 模型 claude-opus-5-5 · 思考：高 | 模型：opus · 思考：高；自定义后 模型：claude-opus-5-5 · 思考：高 | {"agentId":"builtin-claude","model":"opus","effort":"high"}；自定义后 model=claude-opus-5-5 |

**边界与交互补充**：
- 首次打开弹层只触发 1 次 get_agent_model_catalog（前端 + Rust 缓存）；再次打开命中前端缓存（请求数不增、模型行数不重复）；点刷新按钮后请求数 +1（force 生效）。
- 点击外部、Escape 均可关闭弹层；自定义模型 ID 可提交并在触发器/底部信息条原样显示。
- reload 后 localStorage 窗口偏好保留，新建会话按偏好带上模型/思考强度（dsh: DeepSeek-V4-Pro/最高；codex: GPT-6.1 Pro/极高；claude: claude-opus-5-5/高）。
- 目录未加载前（打开弹层前）触发器会回退显示原始 model id（如 deepseek-v4-pro），打开弹层加载目录后恢复展示名 —— 记为建议改进 B。
- **小窗 340x400（可选项）**：弹层 box 300x409 @ (49,55)，底部超出视口 64px、右侧超出 9px（overflows=true，见截图 dsh-small-window-340x400.png）；桌面 1258x802 下无溢出 —— 记为建议改进 C。

### 5.3 截图（docs/verification/screenshots/）

- dsh-picker-open.png / dsh-picker-selected.png
- codex-picker-open.png / codex-picker-selected.png
- claude-picker-open.png / claude-picker-selected.png / claude-custom-model.png
- dsh-small-window-340x400.png

---

## 6. 问题清单

### 6.1 必须修复

**无。** 未发现阻断用户诉求、契约违约或数据/参数错误的缺陷。

### 6.2 建议改进（均不阻断）

| # | 级别 | 问题 | 最小复现 | 建议修法 |
| --- | --- | --- | --- | --- |
| A | 低（观感/契约用词） | 底部信息条的模型部分显示原始 model id（如 模型：deepseek-v4-pro / 模型：opus），而顶部触发器显示展示名（DeepSeek-V4-Pro / Opus）。契约 §6 写的是 "模型：{{ label }}" | mock 三个窗口任选，选 DeepSeek-V4-Pro 后看窗口底部信息条 | Dashboard.vue 的 currentSessionModelLabel 用 agentModels.modelOptionFor(catalog, session.model)?.name 优先，目录未就绪时再回退 id |
| B | 低（观感） | 应用重启/reload 后、目录尚未探测前，模型 pill 回退显示原始 model id（deepseek-v4-pro），直到用户打开一次选择器才恢复展示名 | 设置模型后 reload，进入该 agent 窗口，不打开选择器直接看 pill | agent 窗口挂载时预取一次目录（fetchAgentModelCatalog，利用 60s 缓存），或把最近目录持久化到 localStorage |
| C | 低（可选验收项） | 340x400 小窗下弹层溢出视口：300x409 @ (49,55)，底部超出 64px、右缘超出 9px | 浏览器 340x400 视口打开 dsh 模型弹层 | popup 增加 max-height: min(420px, calc(100vh - 24px))，并约束右边界（如 right: 0 或 max-width: calc(100vw - 16px)） |
| D | 信息 | 前端 mock 夹具 claude 只有 opus/sonnet/haiku 三个别名（契约 §6 规定），不含 claude-opus-5-5 这类本机 modelSettings 完整模型名 | 浏览器 mock 模式打开 claude 弹层 | 无需修复：真实后端目录由 Rust 单测（含 claude-opus-5-5）+ 本机 ~/.claude/settings.json 实际内容覆盖；浏览器已用自定义模型 ID 流程验证 claude-opus-5-5 原样显示 |

### 6.3 已知无关问题（非本轮引入）

- **tts 2 条单测失败**：tts::edge_tts::tests::test_synthesize、tts::tests::edge_synthesis_decodes_to_wav 均为 WebSocket 403（外部网络环境），改动前基线 .ws-probe/baseline-test.log 已存在同样失败；与本次模型目录/思考强度改动无关。
- **仓库存在大量既有未提交改动**（agent activity、chat sessions、voice 等，2026-10 的 WIP）：本轮仅按 mtime 与契约核对相关文件，未对其做验收；未发现本轮改动越界。
- **package.json 另有 verify:codex-trajectory 脚本行新增**（非本特性）：属验证脚本，未新增任何依赖，无运行时影响。

---

## 7. 结论

**通过。**

- 用户原始诉求（三个 agent 窗口显示本机真实模型 + 思考强度，并让选择生效）**满足**：
  - 显示：dsh 3 个真实模型、codex 6 个（真机 9 个，mock 夹具 6 个）、claude 别名/映射模型，均来自本机后端目录；思考强度按模型给出并中文化；
  - 生效：选择写入 chat_sessions.reasoning_effort / 窗口偏好，后续回合分别落到 claude --effort、codex -c model_reasoning_effort="..."、dsh runner --effort（Rust 单测覆盖参数构造；真机 runner 冒烟覆盖目录与参数接受）。
- 构建/单测/真机目录冒烟/浏览器三窗口 E2E 全部达到验收标准；未发现必须修复项。
- 建议在发布前顺手处理 A/B/C 三条低成本改进（尤其是 C 的小窗溢出），它们不影响本轮可交付性。

---

## 8. 复跑指引

1. 构建与单测（D:/ai-desktop-pet）：
   - npm run build
   - node --test scripts/verify-agent-model-catalog.cjs
   - node --test scripts/verify-chat-sessions.cjs
   - node --test scripts/verify-multi-agent.cjs
   - cargo test --manifest-path src-tauri/Cargo.toml --lib（预期 183 passed / 2 known tts 403 failed / 12 ignored）
   - cargo build --manifest-path src-tauri/Cargo.toml
2. 真机后端：
   - cargo test --manifest-path src-tauri/Cargo.toml --lib local_model_catalog -- --ignored --nocapture
   - Copy-Item -Force src-tauri/dsh/pet-runner.mjs C:/Users/15188/.dsh/profiles/dsh-pet/plugins/pet-runner.mjs
   - dsh --profile dsh-pet --list-models（应输出 ===AGENT_MODELS===+JSON，含 3 模型与 off/low/high/max）
   - dsh --profile dsh-pet --list-models --effort high（应 exit 0）
3. 浏览器 E2E（自管 1420 端口与 Chrome，结束自动 kill）：
   - node scripts/verify-agent-model-catalog-browser.cjs
   - 可选：CATALOG_CDP_PORT=9224 避免端口冲突；CATALOG_BASE_URL 指向已有 dev server（此时脚本复用不 kill）

---

## 9. 清理与产物说明

- 本次验收启动的 vite dev server、headless Chrome 进程均已 kill，临时 user-data-dir 已删除（脚本日志 [cleanup] chrome=killed vite=killed profileRemoved=true port1420Free=true）；1420/9223 无 LISTEN 残留（仅 TIME_WAIT）。本机常驻的普通 Chrome 进程未受影响。
- 验收人未修改 src/** 或 src-tauri/**；本轮只新增/修改 scripts/verify-agent-model-catalog-browser.cjs 与 docs/verification/**（含 screenshots）。
- 证据日志保留在 .ws-probe/verifier/：npm-build.log、cargo-test.log、cargo-real-machine.log、cargo-build.log、node-*.log、dsh-list-models*.out/err、browser-e2e.log。

---

## 附：证据文件索引

| 证据 | 路径 |
| --- | --- |
| 浏览器 E2E 完整日志（含 JSON 结果） | .ws-probe/verifier/browser-e2e.log |
| npm build 日志 | .ws-probe/verifier/npm-build.log |
| cargo test --lib 日志（含 2 条 tts 403） | .ws-probe/verifier/cargo-test.log |
| cargo ignored 真机目录测试日志 | .ws-probe/verifier/cargo-real-machine.log |
| cargo build 日志 | .ws-probe/verifier/cargo-build.log |
| node 三套 verify 日志 | .ws-probe/verifier/node-agent-model-catalog.log、node-chat-sessions.log、node-multi-agent.log |
| 真机 dsh 目录输出 | .ws-probe/verifier/dsh-list-models.out、dsh-list-models-effort.out |
| 改动前测试基线 | .ws-probe/baseline-test.log |
| 截图 | docs/verification/screenshots/*.png |
| 可复跑浏览器验收脚本 | scripts/verify-agent-model-catalog-browser.cjs |
