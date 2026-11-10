# v2 前端任务：DSH 多渠道（provider）模型选择器

用户新反馈：本机 DSH 还配置了其它渠道模型（opendesgin 渠道的 claude-opus-5、deepseek-v4.1-flash、gemini、glm 等，另有 together、qwen 渠道），桌宠 dsh 窗口只显示 deepseek-official 的 3 个模型，看不到也选不了其它渠道。v2 要支持按渠道分组展示全部模型，并能选中其它渠道的模型（provider + model 一起写入会话/偏好）。

## 必读
- docs/agent-model-catalog-contract.md 的 v1 全文 + 文末 v2 摘要；docs/prompts/v2-backend.md 是后端并行实现的说明（接口对齐）。
- v1 前端代码已存在：src/services/agentModels.ts、src/components/AgentModelPicker.vue、Dashboard.vue 接入、src/stores/sessions.ts、src/dev-tauri-mock.ts、src/assets/dashboard.css、scripts/verify-agent-model-catalog.cjs。

## 后端接口变化（v2）
- get_agent_model_catalog 返回增加 local_provider: string；dsh 的每个模型都带 provider/provider_label（同一 model id 可能出现在多个 provider，如 deepseek-v4.1-flash 同时属于 together/qwen/opendesgin）。
- 会话接口：create_chat_session / set_chat_session_binding 新增 provider 参数；ChatSession JSON 增加 provider: string（空串 = 跟随本机 provider）。
- 模型选择 = (provider, model) 唯一；provider 为空表示跟随本机 DSH 默认渠道。

## 前端实现清单
1. src/services/agentModels.ts：
   - 类型加 AgentModelCatalog.local_provider、AgentModelOption.provider/provider_label（字段已存在，dsh 现在全量使用）。
   - modelOptionFor(catalog, modelId, provider?) / effortOptionsFor(catalog, modelId, provider?)：先按 provider 非空时精确匹配 provider+id、provider 为空时用 local_provider+id；再退化为同 id 任一模型；空 modelId 用 local_provider+local_model。
   - 新增 providerLabel(option/catalog, provider) 与 isLocalProvider(catalog, provider) 小工具（空 provider 视为本机）。
2. AgentModelPicker.vue：
   - props 增加 provider；emit select({ model, provider, effort })；内部选中/高亮以 (provider,id) 为键；模型行 key = provider + ':' + id。
   - 弹层分组：dsh 按 provider_label 分组，组标题显示渠道名与模型数（如 opendesgin · 40）；组内模型显示 name/id/描述/选中勾。
   - 模型总数 > 12 时显示搜索框（按 name/id/provider 大小写不敏感过滤，过滤时仍按渠道分组，空组隐藏）。
   - 「跟随本机」行显示 local_model_label（后端会给 'opendesgin / deepseek-v4.1-flash' 之类）。
   - effort chips 用 (provider,model) 的能力；兼容性回退逻辑同 v1。
   - 触发器：主文本模型展示名；副文本 思考：xx；选中非本机渠道时在主文本后或副文本里显示渠道（如 claude-opus-5 · opendesgin），保证一眼看出渠道。
3. Dashboard.vue：
   - ChatSession.provider 与 AgentWindowPreference.provider 透传；onAgentModelSelect 把 provider 一起写 sessions.bind({ model, provider, reasoningEffort })（只传真正变化的字段）；无会话存窗口偏好；createChatSession/ensureAgentSessionForSend 传 provider。
   - 顶部 picker 与底部信息条：底部用目录把 model id 解析为展示名并显示渠道（如 模型：claude-opus-5（opendesgin）· 思考：高）。窗口挂载/切 slot 时预取 catalog（service 已带缓存），修复 v1 建议 A/B（重启后 pill 先显示原始 id）。
4. src/stores/sessions.ts：ChatSession.provider、CreateSessionOptions.provider、BindSessionPatch.provider，create/bind invoke 传 provider。
5. src/dev-tauri-mock.ts：
   - dsh fixture 改为多渠道：local_provider=opendesgin、local_model=deepseek-v4.1-flash；
     opendesgin 组至少 5 个模型（claude-opus-5、claude-sonnet-5.5、deepseek-v4.1-flash、gemini-3.7-flash、glm-5.2），efforts off/low/medium/high/xhigh/max；
     deepseek-official 组 3 个（deepseek-v4-flash/pro/vision-exp，off/low/high/max）；together 组 1 个（deepseek-v4.1-flash，off..max）。
   - MockChatSession 增加 provider，create/bind 处理 provider。
6. 验证脚本 scripts/verify-agent-model-catalog.cjs 更新：provider 透传（create/bind 参数含 provider）、同一 id 不同渠道可区分（modelOptionFor 带 provider 返回正确条目）、搜索过滤、local 标签、mock 多渠道 fixture 结构。

## 验收
- npm run build 通过；node --test scripts/verify-agent-model-catalog.cjs 与 verify-chat-sessions.cjs 全绿；其余既有套件不回归。
- 自查：Dashboard 无 datalist/硬编码；dsh 弹层出现 opendesgin 等渠道分组；切换渠道模型后底部信息条显示渠道；codex/claude 行为不变。

## 收尾
- 完成后 team_task_update action=complete（task-5），并 send_message 给 lead：文件清单、npm run build / node --test 结果、UI 交互说明、契约偏差。
- 不要改 src-tauri/**；不要 git commit。
