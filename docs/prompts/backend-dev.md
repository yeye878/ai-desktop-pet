# backend-dev 任务说明

你是 ai-desktop-pet 项目的实现工程师（后端方向）。工作目录 D:/ai-desktop-pet（Windows，Tauri 2 + Vue3 + TypeScript + Rust）。

## 背景
用户反馈（截图）：agent 窗口顶部的「模型」下拉只显示硬编码的 deepseek-chat / deepseek-reasoner，没有显示本机 dsh 真实拥有的模型，也没有模型可选的思考强度（reasoning effort）；codex、claude 窗口同样如此。
本轮把三个 agent 窗口（claude/codex/dsh）的模型目录与思考强度做成读取本机后端的真实能力，选择结果写入会话并在后续回合真实生效。

## 必读
1. docs/agent-model-catalog-contract.md —— 接口契约，严格按它实现（第 2/3/4/5/7 节是你的范围）。
2. 任务 task-1：先 team_task_get 读取，team_task_update action=claim expected_revision=1 认领；完成时 action=complete。

## 你的写作用域（只改这些）
- src-tauri/src/**（新增 src-tauri/src/agent_models.rs；改 dsh.rs、codex.rs、storage/db.rs、storage/mod.rs、openclaw/client.rs、lib.rs）
- src-tauri/dsh/pet-runner.mjs
不要改 src/**、scripts/**、package.json（前端队友在改）。仓库里有大量与本次无关的未提交改动：不要 git commit/checkout/stash/revert。

## 环境事实（Lead 已探明，直接用）
- dsh CLI 0.1.1-rc.2：C:/Users/15188/AppData/Roaming/npm/dsh.cmd，profile 名 dsh-pet，bundles = @deepseek-ai/dsh-base + @deepseek-ai/dsh-headless。
- DSH 模型目录实测：profile 插件里 ctx.get('llm') 有 listProviders()/listModels(provider)/resolveModelInfo(provider, model)；返回 reasoning.efforts([{id,name}]) / reasoning.defaultEffort / context.contextWindow。本机 provider=deepseek-official，模型 deepseek-v4-flash、deepseek-v4-pro、deepseek-v4-flash-vision-exp，efforts=off/low/high/max，default=high，context=1000000。agentDefaultModel.currentSelection() 返回 {provider, model, reasoningEffort?}。
- codex-cli 0.167.0：codex debug models 输出 JSON（大，12 个模型）：{models:[{slug,display_name,description,default_reasoning_level,supported_reasoning_levels:[{effort,description}],visibility,supported_in_api,priority}]}；visibility=list 保留、按 priority 升序。本机 ~/.codex/config.toml 顶层 model="gpt-6.1-terra"、model_reasoning_effort="xhigh"（解析到第一个 [section] 之前即可，别引入 toml 依赖）。
- claude 2.1.319：--model <alias|全名>，--effort <low|medium|high|xhigh|max>；没有模型列表命令。读 ~/.claude/settings.json：model、env.ANTHROPIC_MODEL、env.ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU,FABLE}_MODEL、modelSettings{模型名:{effortLevel}}。
- 参考位置：dsh.rs 的 runner_args L391 / launcher_command L410 / spawn* L458-575 / ensure_profile_in L219 / check_status L1063 / 测试在文件底部；codex.rs 的 exec_args L68 / find_cli L20 / spawn* L97-160 / 测试 L714+；openclaw/client.rs 的 ClaudeAdapter::spawn_streaming_with L107-222；storage/db.rs chat_sessions 建表 L430-442、迁移 L455-530、CRUD L938-1056；lib.rs 的 run_cli_turn L2701 / run_agent_session_turn L3038 / create_chat_session L4118 / set_chat_session_binding L4241 / generate_handler L6201 / 既有单测 L2065 起。
- Lead 在 ~/.dsh/profiles/dsh-catalog-probe 留了个临时探测 profile，请删除。
- pet-runner.mjs 由 include_str! 嵌入，ensure_profile 会同步到 ~/.dsh/profiles/dsh-pet/plugins/pet-runner.mjs；改完可手工 copy 到该路径做真机目录冒烟（属于应用自管 profile，允许）。

## 实现要点
见契约第 2/3/4/5 节。补充约束：
- agent_models.rs 放目录结构体 + claude 探测 + 汇总函数；codex 探测放 codex.rs，dsh 探测放 dsh.rs，复用各模块私有 launcher/config 逻辑。
- 命令 get_agent_model_catalog(slot, force)，slot 只接受 claude/codex/dsh；缓存按 slot 60s，force 绕过；单 slot 失败返回 error 文案不抛错；总探测超时 20s。
- ChatSession/NewChatSession 新字段 reasoning_effort: String；JSON 回传字段名 reasoning_effort；create_chat_session 命令参数 reasoningEffort，set_chat_session_binding 参数 reasoningEffort（Tauri 自动 camelCase→snake_case）。
- 回合：run_cli_turn 里 claude/codex，run_agent_session_turn 里 claude_code/codex/dsh，都取 session.reasoning_effort 并在非空时传参：claude --effort <id>；codex -c model_reasoning_effort="<id>"；dsh runner --effort <id>。direct_api 分支不传。
- pet-runner --list-models：不启动 stdin/授权通道，loader await 后输出一行 ===AGENT_MODELS===+JSON 后 exit(0)；--effort：先 resolveModelInfo 校验该模型 efforts，含则覆盖 selection.reasoningEffort，不含则 emit log 警告并忽略；帮助文本同步更新。
- 单测：codex JSON 解析/过滤/排序、config.toml 解析；claude settings 解析；codex exec_args 有/无 effort；claude 参数构造（可抽 pub(crate)）；dsh runner_args 有/无 effort；DB 迁移默认空串。

## 验收（必须全部执行并贴结果给我）
1. cargo test --manifest-path src-tauri/Cargo.toml --lib 全绿（timeoutMs 给 600000）。
2. cargo build --manifest-path src-tauri/Cargo.toml 编译通过。
3. runner 冒烟：copy src-tauri/dsh/pet-runner.mjs -> C:/Users/15188/.dsh/profiles/dsh-pet/plugins/pet-runner.mjs；在 D:/ai-desktop-pet 执行 dsh --profile dsh-pet --list-models，确认输出 ===AGENT_MODELS=== JSON 含 3 个 dsh 模型和 efforts；再执行一次带 --effort high 确认正常退出。

## 协作与收尾
- 完成后：team_task_update action=complete，并 send_message 给 target="lead" 汇报：改了哪些文件、测试命令与结果摘要、runner 目录 JSON 摘要、契约偏差或遗留问题。
- 发现契约需要变更或与前端接口对不上：立即 send_message 给 lead，不要擅自改契约。
- 遇到阻塞（工具/编译环境）先自行排查，超过 15 分钟仍阻塞再汇报。
