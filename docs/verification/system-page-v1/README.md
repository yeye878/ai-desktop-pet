# 系统设置页 · 分类重组（v1）

日期：2026-11-13

## 问题

系统设置页原来是 6 张卡片平铺：自定义系统提示、技能、AI 后端与模型、天气、性格、职业。
没有分组、没有层级，只能一路往下滚着找，和「对话互动」那种带子项的分组导航不是一个体验。

## 改法

### 1. 按「这个设置管什么」分成 3 类

不是按卡片类型分，而是按用户的心理模型分：

| 分类 | 副标题（页面上的说明） | 包含卡片 |
| --- | --- | --- |
| **对话设定** | 决定 AI 怎么说话：系统提示、技能、性格与职业 | 自定义系统提示、技能、性格、职业 |
| **AI 连接** | 决定接到哪个 AI：本地 CLI 后端或直连 API，含模型与连通性探测 | AI 后端与模型 |
| **服务与提醒** | 外部服务与定时提醒：天气栏、每日问候 | 天气 API 与提醒 |

### 2. 把「性格 / 职业」上移

原来它们排在页面最底部（天气之后），同属「AI 怎么说话」却和系统提示、技能隔了 200 多行。
现在移到技能后面，同一分类的卡片连续，滚动时不用来回跳。

### 3. 页头加快速跳转

页头标题下方加了三个胶囊按钮：`对话设定` / `AI 连接` / `服务与提醒`。
点击平滑滚动到对应分类，并在标题上闪一下强调（1.2 秒后淡出），确认「到了」。

滚动目标是 `.dashboard-content` 而不是 window —— 控制台的内容区是自己滚的，window 不滚。

### 4. 视觉层级

分类标题用左侧 3px 竖线 + 小号大写 kicker + 17px 标题 + 灰色说明，
和卡片标题（15px 图标 + 文字）拉开两级，扫一眼就能分清「组」和「项」。

## 顺带修的一个 bug

改完在浏览器里发现系统设置页**整个渲染不出来**。查下来是 dev mock 缺 `list_skills`
命令，`handleMockCommand` 落到 `default: return null`，而 skills store 的
`activeSkill` 计算属性直接对结果调用 `.find`：

```
TypeError: Cannot read properties of null (reading 'find')
  at ComputedRefImpl.fn (src/stores/skills.ts:9)
```

一个计算属性抛错会让整个组件渲染失败。

补了 12 个 mock 命令（`list_skills`、`set_active_skill`、`get/set_custom_system_prompt`、
`set_personality/profession/skin/font_color/user_avatar`、`tts_cache_stats`），
并给 `mockSkills` 加了默认夹具。

> 注：这是**预览环境的夹具缺口**，真实应用走 Rust 后端不受影响。
> 但它会让浏览器验收完全跑不起来，所以一并补了。

## 验证

浏览器实测（1320×900）：

- 分类标题数量与顺序：`对话设定 / AI 连接 / 服务与提醒` ✓
- 6 张卡片归属全部正确（逐张比对 DOM 位置最近的分类标题）✓
- 跳转条 3 个入口，点击后内容区 scrollTop 确实变化 ✓

回归：`verify:palette` 12/12、`verify:chat-backgrounds` 13/13、
`verify:multi-agent` / `chat-sessions` / `agent-activity` / `pending-interactions` /
`markdown` / `custom-pixel-bg` 全部 fail 0；`verify:interaction-browser`、
主题切换 12 个、背景预设 6 个均通过。`vue-tsc` 与 `vite build` 通过。

## 截图

- `system-top.png` — 页面顶部（含快速跳转条）
- `full-conversation.png` — 对话设定分类（4 张卡片）
- `full-connection.png` — AI 连接分类
- `full-service.png` — 服务与提醒分类
