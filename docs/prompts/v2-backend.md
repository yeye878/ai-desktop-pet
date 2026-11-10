# v2 后端任务：DSH 多渠道（provider）模型目录与切换

用户新反馈：本机 DSH 还配置了其它渠道模型（如 opendesgin 渠道的 claude-opus-5、deepseek-v4.1-flash、gemini、glm，另有 together、qwen 渠道），桌宠 dsh 窗口没有列出来、也选不了。v1 只列当前默认 provider 且只允许覆盖 model；v2 要把本机 DSH 已配置的全部 provider 纳入目录，并允许选中其它渠道（provider + model 一起生效）。

## 必读
- docs/agent-model-catalog-contract.md 的 v1 全文 + 文末 v2 摘要；本文件是 v2 的完整实现说明。
- v1 代码已存在：src-tauri/src/agent_models.rs、dsh.rs 的 model_catalog/ensure_profile_in、pet-runner.mjs 的 --list-models/--effort、codex.rs/openclaw client 的 effort 参数、storage/db.rs 的 reasoning_effort、lib.rs 的 get_agent_model_catalog 命令。

## 本机事实（Lead 已验证可行，可直接照做）
- 用户渠道配置在 C:/Users/15188/.dsh/profiles/desktop/cordis.patch.yml：
  - 行 id: llm-pi-ai（name @deepseek-ai/dsh-llm-pi-ai，config.providers 含 together/qwen/opendesgin，433 行），
  - 行 id: llm-deepseek（name @deepseek-ai/dsh-llm-deepseek-api-key，config.models 两个：deepseek-flash、deepseek-v4-pro），
  - 行 id: agent-default-model（config provider: opendesgin, model: deepseek-v4.1-flash, reasoningEffort: max）。
- 用 CLI 0.1.1-rc.2（pet 用的 dsh）把上述行（name 统一改写成 base 包名）合入一个临时 profile 后启动，llm.listProviders() 得到 4 个 provider：deepseek-official(DeepSeek)、together、qwen、opendesgin；模型数量分别 2、1、6、40；opendesgin 含 claude-fable-5/claude-haiku-5.5/claude-opus-4.8/claude-opus-5/claude-sonnet-5/claude-sonnet-5.5/deepseek-v4*/gemini-3.7-flash/gemini-3.8-flash/glm-5.2/glm-5.3-flash/gpt-6*/grok-4.6/grok-4.7/... 等；agent-default-model 覆盖后 currentSelection() = { provider: opendesgin, model: deepseek-v4.1-flash }（plugin config 里的 reasoningEffort 不被 currentSelection 返回，这是 DSH 设计，属正常）。
- 关键坑：llm-deepseek 源行 name 是 dsh-llm-deepseek-api-key，该包只在桌面 app runtime 里有，CLI 的 profiles/node_modules 里没有；照抄 name 会加载失败。必须改写为 base 包名 @deepseek-ai/dsh-llm-deepseek。llm-pi-ai 与 agent-default-model 的 name 同理用 base 包名。
- 凭据在 C:/Users/15188/.dsh/.credentials.yaml（OPENDESGIN_API_KEY / TOGETHER_API_KEY / QWEN_API_KEY），dsh-credentials-local 会自动读取，无需处理。
- ~/.dsh/settings.yaml 当前不存在；若将来存在且含 llm-pi-ai/llm-deepseek/agent-default-model 段，则 dsh-base 的 settings 插件会自动生效，此时不要再复制配置（避免双份）。

## 实现清单
1. dsh.rs 增加用户渠道配置同步（ensure_profile 生成 cordis.patch.yml 时）：
   - 若 $DSH_HOME/settings.yaml 含非注释的 llm-pi-ai: / llm-deepseek: / agent-default-model: 段 → 不同步，只用现有静态补丁。
   - 否则按 desktop → web → headless → open-design → 其余 profile 目录名字典序，找第一个 cordis.patch.yml 中含 id: llm-pi-ai 或 llm-deepseek 或 agent-default-model 的 profile，作为同步源。
   - 从源文件按行提取这三个顶层行的原文块（行首 - id: 开始，到下一个行首 - 为止，去掉尾部空行/注释无妨），把 name 改写成 base 包名（llm-pi-ai → @deepseek-ai/dsh-llm-pi-ai；llm-deepseek → @deepseek-ai/dsh-llm-deepseek；agent-default-model → @deepseek-ai/dsh-agent-default-model），config 缩进原样保留。
   - 追加到 pet profile 的 cordis.patch.yml 托管段，之前加一行注释：# 自动同步自本机 DSH profile: <名字>（源文件变化自动重建，勿手改）。只允许这三行，禁止同步其它行。
   - 仍然遵守 patch_is_ours：用户自己写的补丁内容不被覆盖（现有逻辑）。每轮 ensure_profile 重建即可。
   - 单测：内联 YAML fixture 提取（多行 config、注释、截断、name 改写）；settings.yaml 存在优先；无源 profile 时不追加；临时 home 下 ensure_profile 输出包含/不包含托管段。
2. pet-runner.mjs 的 --list-models 改为遍历全部 provider：
   - 输出协议：{ local: { provider, model, effort }, providers: [{id,label}], models: [ { provider, provider_label, id, name, description?, default_effort, efforts:[{id,name,description?}], context_window? } ], error: '' }。
   - 逐个 provider listModels + 逐模型 resolveModelInfo；单模型失败只 emit log 并继续；单 provider listModels 失败也继续并在 error 里追加说明（可选）。
   - 新增 --provider <id>：先覆盖 model 再覆盖 provider（非空才覆盖）；校验 provider 存在于 listProviders，否则 emit log 警告并忽略；之后用最终 provider+model 校验 --effort（沿用 v1 逻辑）。
   - 帮助文本更新；真机测试 dsh --profile dsh-pet --list-models 输出应包含 opendesgin 的 claude-opus-5 等。
3. Rust 目录结构：
   - AgentModelCatalog 增加 local_provider: String（dsh 用 local selection.provider；claude/codex 空串）。
   - dsh model_catalog 解析 v2 协议：models 全部带 provider/provider_label；local_model/local_effort/local_provider 来自 local 字段；local_model_label 用 provider_label + ' / ' + 模型 name（可用 provider_label 为空时用 provider）。兼容旧协议（无 providers/local 字段时按原 v1 字段推断，防御性）。
4. 会话 provider 持久化（v2）：
   - chat_sessions 加 model_provider TEXT NOT NULL DEFAULT ''，幂等迁移（照 reasoning_effort 模式）；ChatSession/NewChatSession 加 provider: String（JSON 字段 provider，serde default）；update_chat_session 加 provider: Option<&str>。
   - create_chat_session 命令加 provider: Option<String>；set_chat_session_binding 加 provider: Option<String>；模型或 provider 变化时清空 CLI 续接 id，仅 effort 变化不清。
   - run_cli_turn 从 session.provider 取 provider（trim 后空 = None），只有 dsh 传；dsh.rs spawn* / runner_args 增加 provider 参数，arg 顺序 --model → --provider → --effort → --resume；claude/codex 忽略 provider（调用处传 None，可沿用现有签名不动）。
   - 单测：runner_args 含 --provider 且顺序正确、空 provider 不追加；DB 迁移旧会话 provider 默认空串；update_chat_session provider None 不改 / Some('') 清空；create/bind 命令参数落库（参考 v1 已有测试模式）。
5. 真机验收（ignored 测试）：
   - cargo test --manifest-path src-tauri/Cargo.toml --lib local_model_catalog -- --ignored --nocapture 应输出 4 个 provider（本机配置存在时）：deepseek-official/together/qwen/opendesgin；dsh 目录含 opendesgin 的 claude-opus-5 且 efforts 含 off/low/medium/high/xhigh/max；local_provider=opendesgin、local_model=deepseek-v4.1-flash。
   - 手工把 src-tauri/dsh/pet-runner.mjs 同步到 ~/.dsh/profiles/dsh-pet/plugins 并手工把同步后的 cordis.patch.yml 内容 effect 到 dsh-pet profile（或直接跑一次 spawn 前先调用 ensure_profile）后，dsh --profile dsh-pet --list-models 检查输出；再跑一次带 --provider opendesgin（无 prompt，--list-models 模式）确认 exit 0。
   - cargo test --lib 全绿（tts 2 条既有网络失败可排除并注明基线）；cargo build 通过。

## 收尾
- 完成后 team_task_update action=complete（task-4），并 send_message 给 lead：文件清单、测试输出摘要、真机 4-provider 证据、契约偏差。
- 不要改前端文件；不要 git commit；临时探测 profile（dsh-catalog-probe2）由你删除。
