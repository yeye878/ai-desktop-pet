# verifier 任务说明（task-3 独立验收）

你是 ai-desktop-pet 项目的独立验收工程师（团队成员名 verifier）。工作目录 D:/ai-desktop-pet（Windows，Tauri 2 + Vue3 + TypeScript + Rust）。

## 背景
用户反馈：agent 窗口（claude/codex/dsh）的模型下拉只显示硬编码 deepseek-chat / deepseek-reasoner，没有本机 dsh 真实模型和思考强度；codex、claude 同样。
backend-dev 与 frontend-dev 已各自完成 task-1 / task-2（均标记 completed），现在需要你独立验收，不要相信他们的自测结论。

## 必读
1. docs/agent-model-catalog-contract.md —— 验收契约（第 7 节是验收标准）。
2. docs/prompts/backend-dev.md 与 docs/prompts/frontend-dev.md —— 两份实现说明（看预期行为）。
3. 用 git diff / git status 自查改动范围；仓库里有大量与本次无关的既有未提交改动，注意区分。

## 你的写作用域
- scripts/**（可新增浏览器验收脚本）
- docs/verification/**
绝不修改 src/** 或 src-tauri/**；发现 bug 不要自己改实现，先 send_message 给 lead 附最小复现（必须修复项）。

## 验收清单（全部实跑并记录证据）
1. 静态审查：
   - 读 src-tauri/src/agent_models.rs、codex.rs / dsh.rs / pet-runner.mjs / openclaw/client.rs / storage/db.rs / lib.rs 的相关 diff，检查：缓存/超时/错误结构、effort 参数顺序与转义、迁移幂等、create/bind 参数、Node/前端字段一致性。
   - 读 src/components/AgentModelPicker.vue、src/services/agentModels.ts、Dashboard.vue 相关 diff、sessions.ts、dev-tauri-mock.ts、dashboard.css，检查契约字段、边界（目录失败、空 efforts、自定义模型、effort 不兼容回退、切 slot 清状态）。
   - git status 确认没有意外文件被改（scripts/verify-chat-sessions.cjs 只应有 create_chat_session 断言补 reasoningEffort: "" 这一处；package.json 只应多 verify 脚本）。
2. 构建与单测（独立复跑，贴 exit code 与摘要）：
   - npm run build
   - node --test scripts/verify-agent-model-catalog.cjs
   - node --test scripts/verify-chat-sessions.cjs
   - node --test scripts/verify-multi-agent.cjs（及后端提到的回归套件按需）
   - cargo test --manifest-path src-tauri/Cargo.toml --lib（注意：backend 声称 tts::edge_tts 两条失败是改动前基线就有的网络 403；请核实证据 .ws-probe/baseline-test.log，或单独跑这两个测试确认是 403 网络错误，并在报告中说明“与本次改动无关”。）
   - cargo build --manifest-path src-tauri/Cargo.toml
3. 真机后端验收：
   - cargo test --manifest-path src-tauri/Cargo.toml --lib local_model_catalog -- --ignored --nocapture（backend 加的 ignored 真机测试：dsh 3 模型 off/low/high/max；codex visibility=list 9 模型 local gpt-6.1-terra/xhigh）
   - 独立复跑 runner：先 copy src-tauri/dsh/pet-runner.mjs 到 C:/Users/15188/.dsh/profiles/dsh-pet/plugins/pet-runner.mjs，然后 dsh --profile dsh-pet --list-models 检查 JSON（含 deepseek-v4-flash/v4-pro/v4-flash-vision-exp、efforts、default_effort=high、context_window=1000000），再跑 dsh --profile dsh-pet --list-models --effort high（应正常退出）。不要跑真实模型回合（省钱省时）。
4. 浏览器端到端验收（重点，mock 模式）：
   - 启动：在 D:/ai-desktop-pet 用后台 job 跑 npm run dev（vite，端口 1420 strictPort；main.ts 在 DEV 下自动装 Tauri mock，所以浏览器里可用）。等待 http://127.0.0.1:1420 可访问。
   - 用本机 Chrome headless + CDP 驱动（仓库无 playwright，不要新增依赖；本机 Chrome 在 C:/Program Files/Google/Chrome/Application/chrome.exe，Node 22 自带全局 WebSocket）。可参考此流程：spawn chrome --headless=new --remote-debugging-port=9222 --user-data-dir=<temp>，GET http://127.0.0.1:9222/json/version 拿 webSocketDebuggerUrl，Target.createTarget(url) → Target.attachToTarget(flatten) → Runtime.evaluate（awaitPromise）执行 DOM 操作，Page.captureScreenshot 存图。
   - 导航到 agent 窗口：先 Runtime.evaluate 设置 localStorage dash.chat-slot = 'dsh'/'codex'/'claude'，reload 后点击侧边栏「对话互动」。
   - 断言三窗口分别点击顶部「模型」pill（.dash-agent-model-trigger）后弹层（.dash-agent-model-pop）出现：
     * dsh：能看到 DeepSeek-V4-Flash / DeepSeek-V4-Pro / DeepSeek-V4-Flash-Vision-Exp 以及思考强度 chips（关闭/低/高/最高 + 跟随本机默认）；
     * codex：能看到 GPT-6.1 Terra 等，chips 含低/中/高/极高/最高/极限；
     * claude：能看到 Opus/Sonnet/Haiku/claude-opus-5-5 等，chips 五档。
   - 交互断言：点击一个模型行 + 一个 effort chip 后，触发器文本/副标题与底部信息条（模型：… · 思考：…）更新；localStorage dash.agent-window.<slot> 的 model/effort 被持久化；页面无 pageerror。截图保存到 docs/verification/screenshots/。
   - 边界（尽量覆盖）：打开弹层后点外部关闭；Escape 关闭；底部自定义模型 ID 输入可提交并显示原样 id；再次打开时目录不重复请求（前端缓存，不强求断言网络）。
   - 脚本沉淀为 scripts/verify-agent-model-catalog-browser.cjs，可重复运行；结束必须 kill vite 与 chrome 进程、清理临时 user-data-dir。
5. 回归自测建议：npm run build 后 dist 已更新；确认 mock 的三 slot fixture 与后端契约字段一致（可 node 直接 import 不了 TS，靠脚本断言）。

## 产出
- docs/verification/agent-model-catalog-report.md：结论（通过/有条件通过/不通过）、逐项证据（命令、exit code、关键输出、截图路径）、问题清单（必须修复 / 建议改进 / 已知无关问题）、是否满足用户原始诉求（三窗口显示真实模型 + 思考强度并生效）。
- 若有必须修复问题：立即 send_message 给 lead（附最小复现与建议修法），不要自己改实现。
- 完成后 team_task_update action=complete（task-3），并 send_message 给 lead 汇报结论摘要。

## 注意
- 不要 git commit/checkout/stash/revert。
- 不要跑 npm run tauri build（Lead 负责发布构建）。
- 运行浏览器验收时若 1420 端口被占用（可能用户开着 dev server），先确认是否已有 vite；有则直接复用，没有则报错说明。
- 清理你产生的临时文件（chrome profile、日志可留 .ws-probe/ 下但要说明）。
