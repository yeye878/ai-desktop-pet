# 对话背景 v3 重设计 · 验收记录

日期：2026-11-11
范围：个性外观页的「对话背景」+ 对话互动页的背景渲染与切换

## 改了什么

| 项 | 改动 |
| --- | --- |
| 背景来源 | 新增 `src/services/chatBackgrounds.ts`：预设清单、localStorage 读写、广播事件统一到一处，三个窗口不再各写一份 |
| 新预设 | **极光**（Aurora：多层竖直光带 + 星点闪烁 + 点击光爆 + 跟随桌宠状态变亮）、**水墨**（Ink：宣纸底 + 三层远山 + 流动雾霭 + 墨晕扩散） |
| 对话页渲染 | 新增 `.dash-chat-stage` 舞台：`BgCanvas` 固定在消息区背后，不随滚动移动；深色背景（极光/科幻）自动切换为毛玻璃气泡保证可读性 |
| 根因修复 | 之前换背景看不出来有两个原因：① `Dashboard.vue`（对话互动页）根本没挂 `BgCanvas`；② 消息区自带半透明白底与 `backdrop-filter`，把背景盖住了。现在两处都改掉了 |
| 切换可见性 | 对话页顶部新增背景快捷条（色板圆点 + 名称），切换时舞台右侧浮出「对话背景 · 极光」提示条并有一次亮度闪动；浮动聊天窗切换时右上角显示「背景 · 极光」徽标 |
| 外观页 | 从「图标格子」改为**真实实时预览**（跑同一份 `BgCanvas`，上面叠示意气泡）+ 缩略色板卡片（名称 + 一句话说明 + 选中勾），另有「导入图片 / 关闭背景 / 清除自定义」按钮 |
| 跨窗口同步 | 三处都监听 `storage` 事件；同一窗口内用自定义事件即时同步，改完立刻生效 |

## 验证

离线用例 `npm run verify:chat-backgrounds`（8/8 通过）：预设清单完整性、可选预设不含自定义、
未知取值回落 `none`、读不到存储不抛错、写入广播、自定义图片自动切 `custom`、清除回 `none`、
localStorage 配额抛错时界面仍生效、三个窗口共用同一组 key（兼容老版本设置）。

浏览器实测 `npm run verify:chat-backgrounds-browser`（1320×860，dev mock 接管 invoke）：
7 张预设卡片、实时预览 canvas、逐个切换落盘与高亮一致、对话页舞台 `data-chat-bg` 与 canvas 数量正确、
切换提示出现、关背景后 canvas 与 `has-bg` 一并移除、自定义图片生效。截图见本目录。

回归：`verify:interaction-browser`、`verify:agent-activity-browser`、`verify:agent-activity`、
`verify:codex-trajectory`、`verify:pending-interactions`、`verify:markdown`、`verify:custom-pixel-bg` 全部通过。

## 截图

- `appearance-panel.png` — 外观页整块效果（实时预览 + 卡片网格）
- `appearance-aurora.png` / `appearance-ink.png` / `appearance-cute.png` / `appearance-scifi.png` / `appearance-minimal.png` — 各预设的预览框
- `chat-aurora-full.png` — 对话页整体（含快捷切换条）
- `chat-aurora-stage.png` / `chat-ink-stage.png` / `chat-scifi-stage.png` / `chat-minimal-stage.png` / `chat-cute-stage.png` — 消息区背景实际渲染
- `chat-custom-full.png` — 自定义图片背景
