# v2 验收报告：DSH 渠道（provider）模型目录与切换

- 日期：2026-11-10 22:15（Asia/Tokyo）
- 验收对象：task-4（后端，backend-dev）、task-5（前端，frontend-dev）
- 验收执行：verifier-v2（独立 teammate，fresh 上下文）
- 说明：verifier-v2 已完成全部实跑并把证据写入 .ws-probe/verifier/，但其会话在撰写报告一步异常结束；本报告由 Lead 依据该证据、结合 Lead 的独立复跑与代码抽查整理，未改动验收结论。

## 结论
**通过**（用户诉求「DSH 已配置的其它渠道模型也要出现在桌宠 dsh 窗口并能选用」已达成；无必须修复项；1 项建议改进见文末）。

## 用户诉求覆盖
| 诉求 | 状态 | 证据 |
| --- | --- | --- |
| dsh 窗口显示本机 DSH 全部渠道模型 | 通过 | 真机目录 4 个 provider / 45 个模型（deepseek-official 2、together 1、qwen 6、opendesgin 36）；浏览器 mock 弹层分组 opendesgin·8 / DeepSeek·3 / Together·1 / Qwen·2 |
| 能选中其它渠道模型并真正生效 | 通过 | 真实回合 `--provider opendesgin --model deepseek-v4.1-flash --effort max` 返回 `done completed`、无 provider/effort 警告；会话 command 参数含 provider |
| 渠道选择持久化 | 通过 | 浏览器 reload 后 pill 仍「DeepSeek V4.1 Flash · opendesgin」，localStorage 偏好含 provider；DB 新增 model_provider 列 |
| 思考强度按所选渠道/模型生效 | 通过 | opendesgin 模型 efforts=off/low/medium/high/xhigh/max；同 id 在 together 只有低/中/高/极高/最高（无关闭）；effort chips 随 (provider,model) 更新 |
| codex / claude 不回归 | 通过 | codex 9 模型 local=gpt-6.1-terra effort=xhigh、claude Opus/Sonnet/Haiku 行为与 v1 一致；mock E2E 断言单组、无渠道后缀、pageErrors=0 |

## 构建与单测（verifier-v2 实跑 + Lead 抽查）
| 项目 | 结果 |
| --- | --- |
| npm run build | 通过（vue-tsc --noEmit + vite build ✓ built in 4.37s） |
| node --test verify-agent-model-catalog.cjs | 14/14 pass（v2 用例：provider 透传、同 id 不同渠道、搜索、分组、local 标签、Dashboard 接线、mock 多渠道） |
| node --test verify-chat-sessions.cjs | 12/12 pass |
| node --test verify-multi-agent / agent-activity / codex-trajectory / markdown / pending-interactions | 13/13、14/14、10/10、1/1、6/6 pass |
| cargo test --lib | 191 passed / 2 failed / 12 ignored（2 条失败为改动前基线既有的 Edge TTS 网络 403 用例，与本次无关） |
| cargo build | 通过（仅既有 warning） |
| ignored 真机 local_model_catalog | 2 passed：dsh 4 providers / 45 模型 / local=opendesgin deepseek-v4.1-flash / claude-opus-5 efforts 六档；codex 9 模型不受影响 |

## 真机（CLI + dsh-pet profile）
- `~/.dsh/profiles/dsh-pet/cordis.patch.yml` 465 行：托管段含「自动同步自本机 DSH profile: desktop」与三行渠道配置；llm-deepseek name 已改写为 @deepseek-ai/dsh-llm-deepseek（无 api-key 包名泄漏）；config 内模型 name 保留；重复 ensure_profile 幂等。
- `dsh --profile dsh-pet --list-models`：exit 0，v2 JSON，local={provider:opendesgin, model:deepseek-v4.1-flash}，providers 4 个，模型计数 2/1/6/36；opendesgin 的 claude-opus-5 efforts=[off,low,medium,high,xhigh,max]。
- `dsh --profile dsh-pet --list-models --provider opendesgin`：exit 0；`--provider bogus` 输出「渠道 bogus 不在本机配置中，忽略 --provider」后正常输出目录（exit 0）。

## 浏览器 mock E2E（Chrome headless + CDP，无新增依赖）
- 环境：vite dev 1420 + Chrome/156.0.8078.81 CDP 9223，清理后 chrome/vite killed、port 1420 free、pageErrors=0、consoleErrors=0。
- dsh：分组与计数正确；搜索 opus 过滤；选 Claude Opus 5（opendesgin）后触发器「Claude Opus 5 · opendesgin」、底部「模型：Claude Opus 5（opendesgin） · 思考：高」；只改 effort 时 bind 只传 reasoningEffort=max（model/provider 为 null，不清续接 id）；reload 持久化；同 id deepseek-v4.1-flash 在 Together 与 opendesgin 条目可区分且选中正确。
- codex / claude 回归：单组、无渠道后缀，触发器/底部展示名与思考强度正常；claude 自定义模型 ID 仍可用。
- 小窗 340x400：弹层 316x328 @ (49,55)，`overflows: true`（仅尺寸收缩优化，未做翻转定位）——列为建议改进，不阻塞。
- 截图 11 张：docs/verification/screenshots-v2/。

## 问题清单
| 级别 | 问题 | 说明 |
| --- | --- | --- |
| 必须修复 | 无 | — |
| 建议改进 | 340x400 小窗弹层仍判定溢出（316x328 定位 (49,55)） | v1 既有问题，v2 已做 max-height/max-width 收缩缓解；如需彻底解决可做弹层翻转定位 |
| 已知无关 | cargo test 2 条 Edge TTS 403 失败 | 改动前基线既有（.ws-probe/baseline-test.log 同样 172 passed/2 failed），与本次无关 |

## 发布
- `npx tauri build --no-bundle` exit 0，新 exe 29,199,360 B（2026-11-10 17:33:56 JST）。
- 已部署 `desktop-release/AI Desktop Pet.exe`（覆盖前备份为 `AI Desktop Pet.exe.prev`，29,188,608 B）；exe 内包含 `===AGENT_MODELS===` 与「自动同步自本机 DSH profile」标记，确认是 v2 构建。
- 用户重启桌宠后生效；dsh 窗口模型弹层将出现本机全部渠道。

## 契约
- 与 docs/agent-model-catalog-contract.md v2 摘要一致，无偏离；provider 仅 dsh 下传，claude/codex local_provider 为空串。
