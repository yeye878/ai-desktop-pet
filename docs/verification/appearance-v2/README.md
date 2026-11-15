# 个性外观页 v2 + 多窗格对话背景

日期：2026-11-13

## 一、个性外观页重做

### 先查资料，再动手

能找到并实际读到的参考（其余链接 404，已跳过）：

| 来源 | 采纳的结论 |
| --- | --- |
| [Primer · Customize 场景模式](https://primer.style/product/scenario-patterns/customize/) | 自定义界面必须先给**预览**，用户在改动前就能看到结果 |
| [Refactoring UI · Building Your Color Palette](https://refactoringui.com/previews/building-your-color-palette) | 层级靠字号/字重/颜色对比，不靠堆边框；一卡只讲一件事 |
| [Material 3 · Cards](https://m3.material.io/components/cards/overview) | 卡片是「一个主题的容器」，选中态要明确 |
| 格式塔「共同区域」+ Von Restorff 隔离效应 | 同类选项要被明确边界包住；选中项必须和其他项差异显著，一圈细边框不够 |

### 旧版的问题

1. **看不到结果**：改完要切到「对话互动」才知道好不好看，来回跳。
2. **结构散**：6 张卡片用 `<div>` 平铺，`appearance-atelier-layout` 这个类在 CSS 里根本不存在，所以布局全靠默认流。
3. **层级平**：卡片标题、分区、页头几乎一样重，眼睛没有落点。
4. **体量失衡**：「自定义像素桌宠」的工坊特别高，把整页撑得很难看。

### 新版结构

```
┌─────────────────────────────────────────────┐
│ 实时预览舞台              │ Live Preview     │
│ ┌────────┐  ┌─────────┐  │ 个性外观          │
│ │ 桌宠   │  │ 示例对话 │  │ 主题 / 形象 /    │
│ │        │  │         │  │ 背景 / 字体      │
│ └────────┘  └─────────┘  │                  │
└─────────────────────────────────────────────┘
形象   ── 3 个形象卡 + 折叠的自定义工坊
配色   ── 12 套主题 + 6 种字体颜色
对话背景 ─ 预览 + 说明 + 浓度 + 预设条
个人   ── 用户头像
```

1. **顶部实时预览**：左边是「桌宠 + 一段示例对话 + 当前背景」组成的迷你场景，
   右边是当前组合的四项摘要（主题/形象/背景/字体）。改任何一项，这里立刻变。
2. **四段分区**：每段有 kicker + 标题 + 一句话说明 + 左侧竖线，和系统设置页同一套视觉语言。
3. **折叠工坊**：`<details>` 默认收起，避免撑乱页面（渐进披露）。
4. **选中态重做**：从「一圈细边框」改成「实色边框 + 3px 外发光 + 右上角实心圆勾」，
   符合隔离效应。

## 二、多窗格背景不生效（bug 修复）

### 原因

合并上游 Phase 0 之后，对话页多了「同屏布局」多窗格模式，每个窗格渲染的是
新组件 `SessionPane.vue`。而对话背景只在**单窗格**视图里挂过 `BgCanvas`，
`SessionPane.vue` 里既没引 `BgCanvas` 也没读背景设置——所以多窗格下面背景整个消失。

### 修法

1. 抽出 `src/composables/useChatBackground.ts`：把「读 localStorage + 监听 storage 事件 +
   同窗口自定义事件」收敛成一个 composable。之前 Dashboard / ChatBubble / Settings
   各写一份，SessionPane 就是漏掉的那一个。
2. `SessionPane.vue` 接入：
   - 引入 `BgCanvas` 与 `useChatBackground()`
   - 消息区外面包一层 `.session-pane-stage`，`BgCanvas` 固定在其背后
   - 加 `.session-pane-bg-scrim` 保证深色背景下气泡文字可读
   - `.session-pane-messages` 抬高 z-index，让背景透上来

### 验证

多窗格模式下逐窗格检查：

| 指标 | 结果 |
| --- | --- |
| 窗格总数 | 2 |
| 绑定了会话的窗格 | 1 |
| 有背景舞台的窗格 | 1 / 1 |
| 渲染了 canvas 的窗格 | 1 / 1 |
| 启用 `has-bg` 的窗格 | 1 / 1 |

（空窗格显示的是「选择新会话」占位，本来就没有消息区，不参与断言。）

## 三、验证汇总

浏览器实测（1320×940），逐项断言：

- 外观页四个分区 `形象 / 配色 / 对话背景 / 个人` 顺序正确
- 实时预览舞台存在，右侧摘要 4 项
- 切换形象后摘要从「Bongo Cat」变「呆猫八条」
- 12 套主题全部渲染，切换后摘要从「青瓷」变「陶土」
- 8 张背景预设卡，点第 2 张后 `localStorage` 落盘 `aurora`，预览 canvas 存在，名称更新
- 折叠工坊默认关闭
- 多窗格背景逐窗格通过

回归：`verify:palette` 12/12、`verify:chat-backgrounds` 13/13、
multi-agent / chat-sessions / agent-activity / pending-interactions / markdown 全部 fail 0；
`verify:chat-backgrounds-browser`、`verify:interaction-browser`、主题切换实测通过；
`vue-tsc` 与 `vite build` 通过。

保留的类名（测试依赖）：`.bg-panel`、`.dash-bg-card`、`.dash-bg-stage-frame`、`.dash-bg-stage-tag`。

## 四、截图

- `appearance-top.png` — 页面顶部（实时预览 + 摘要）
- `appearance-full.png` — 整页四段分区
- `workspace-bg.png` — 多窗格模式下的对话背景
