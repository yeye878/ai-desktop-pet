# v2 独立验收任务说明（task-6）

你是 ai-desktop-pet 项目的独立验收工程师（团队成员名 verifier-v2）。工作目录 D:/ai-desktop-pet（Windows，Tauri 2 + Vue3 + TypeScript + Rust）。

## 背景
用户反馈：桌宠 dsh 窗口的模型下拉只显示默认渠道的模型，但本机 DSH 还配置了其它渠道（opendesgin / together / qwen，见 ~/.dsh/profiles/desktop/cordis.patch.yml 的 llm-pi-ai 配置）。
v1 已实现真实模型目录+思考强度（task-1/2/3 已验收通过）；v2 由 backend-dev（task-4）与 frontend-dev（task-5）完成，需要你独立验收。

## 必读
1. docs/agent-model-catalog-contract.md 的 v1 全文 + 文末 v2 摘要。
2. docs/prompts/v2-backend.md、docs/prompts/v2-frontend.md（实现说明，看预期行为）。
3. 用 git diff / git status 审查 v2 改动（注意仓库里有大量与本功能无关的既有未提交改动，注意区分）。

## 写作用域
- scripts/**（更新/新增浏览器验收脚本）
- docs/verification/**（v2 报告与截图）
绝不修改 src/** 或 src-tauri/**；发现必须修复的 bug 先 send_message 给 lead 附最小复现，不要自己改实现。

## 验收清单（全部实跑并留证据）
1. 静态审查 v2：
   - src-tauri/src/dsh.rs 的渠道配置同步（settings.yaml 优先、profile 优先级 desktop→web→headless→open-design、只同步 llm-pi-ai/llm-deepseek/agent-default-model 三行、name 改写为 base 包名、config 原文保留、patch_is_ours 不破坏用户内容、幂等）。
   - pet-runner.mjs 的 v2 协议（local + providers + models 全 provider、--provider 校验/忽略策略、arg 顺序 --model→--provider→--effort→--resume）。
   - agent_models.rs 的 local_provider；storage/db.rs 的 model_provider 迁移与 ChatSession.provider；lib.rs 的 create/bind provider 参数、清 CLI 续接 id 条件、run_cli_turn 仅 dsh 传 provider。
   - 前端：services/agentModels.ts 的 (provider,id) 匹配与 local_provider、AgentModelPicker 的渠道分组/计数/搜索/(provider,id) 键/触发器渠道、Dashboard provider 透传与底部展示名、sessions.ts provider、dev-tauri-mock 多渠道 fixture。
   - git status 确认没有越界改动（scripts/verify-chat-sessions.cjs 只应多 provider: "" 期望；package.json 只多 verify 脚本）。
2. 构建与单测（独立复跑，贴 exit code 与摘要）：
   - npm run build
   - node --test scripts/verify-agent-model-catalog.cjs（v2 应为 14 用例全绿）
   - node --test scripts/verify-chat-sessions.cjs、node --test scripts/verify-multi-agent.cjs（及 verify-agent-activity / codex-trajectory / markdown / pending-interactions）
   - cargo test --manifest-path src-tauri/Cargo.toml --lib（注意：tts::edge_tts 两条 403 网络失败是改动前基线既有，核实 .ws-probe/baseline-test.log 或单跑这两条确认 403，并在报告注明与本次无关）
   - cargo build --manifest-path src-tauri/Cargo.toml
3. 真机后端验收：
   - 读 ~/.dsh/profiles/dsh-pet/cordis.patch.yml，确认托管段含「自动同步自本机 DSH profile: desktop」与三行渠道配置，llm-deepseek 的 name 是 @deepseek-ai/dsh-llm-deepseek（不是 api-key 包名），config 内模型 name 未被改写。
   - cargo test --manifest-path src-tauri/Cargo.toml --lib local_model_catalog -- --ignored --nocapture → 应 4 个 provider（deepseek-official/together/qwen/opendesgin）、local=opendesgin / deepseek-v4.1-flash、opendesgin 含 claude-opus-5 且 efforts=off/low/medium/high/xhigh/max；codex 不受影响。
   - 手工复跑：copy src-tauri/dsh/pet-runner.mjs 到 C:/Users/15188/.dsh/profiles/dsh-pet/plugins/pet-runner.mjs 后，dsh --profile dsh-pet --list-models（exit 0，v2 JSON：local/providers/models 四渠道）；dsh --profile dsh-pet --list-models --provider opendesgin（exit 0）。不要跑真实模型 token 回合。
4. 浏览器 mock E2E（重点）：
   - 复用并更新 scripts/verify-agent-model-catalog-browser.cjs（v1 已有 Chrome headless + CDP 实现，可读后改造，不要新增依赖；本机 Chrome：C:/Program Files/Google/Chrome/Application/chrome.exe，Node 24 自带 WebSocket）。
   - 启动 npm run dev（vite 端口 1420，DEV 下自动装 Tauri mock），浏览器导航后切到 dsh 窗口，点开模型 pill 断言：出现 opendesgin / DeepSeek / Together / Qwen 分组与组标题数量；模型总数 14 > 12 时出现搜索框，输入例如 opus 可过滤；点选 claude-opus-5（opendesgin）后触发器显示「Claude Opus 5 · opendesgin」、底部信息条「模型：…（opendesgin）· 思考：…」；同 id deepseek-v4.1-flash 在 together 与 opendesgin 两个渠道条目可区分且选择正确；localStorage dash.agent-window.dsh 持久化 provider；再次打开/reload 后恢复；codex/claude 仍为单组无渠道后缀、行为不回归；pageerror=0。
   - 截图存 docs/verification/screenshots-v2/（至少 dsh 弹层多渠道、选中 opendesgin 模型后、codex 回归各一张）；结束 kill vite 与 chrome、清理临时 user-data-dir。
5. 额外边界（尽量）：切渠道后思考强度 chips 是否按 (provider,model) 更新；不改 model 只改 effort 时 bind 只传 reasoningEffort（可从 mock 会话/脚本断言）；340x400 小窗弹层不再溢出明显。

## 产出
- docs/verification/agent-model-catalog-v2-report.md：结论（通过/有条件通过/不通过）、逐项证据（命令、exit code、关键输出、截图路径）、问题清单（必须修复/建议改进/已知无关）、是否满足用户「DSH 其它渠道模型也显示并可用」的诉求。
- 有必须修复项立即 send_message 给 lead（附最小复现与建议修法）。
- 完成后 team_task_update action=complete（task-6），并 send_message 给 lead 汇报结论摘要。

## 注意
- 不要 git commit/checkout/stash/revert；不要跑 npm run tauri build（Lead 负责发布构建）。
- 你已通过 reassign 拥有 task-6（owner=verifier-v2），无需再 claim，直接干活，读任务详情用 team_task_get。
