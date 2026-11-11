# 对话背景 v3 · 柔和化修正记录

日期：2026-11-11
起因：第一版把极光/科幻做成了深色夜空，放在米白控制台里像「贴了一块黑方块」，反馈是「有点突兀」。

## 突兀的三个根因（实测）

| 根因 | 证据 |
| --- | --- |
| 深色底与浅色 UI 撞边 | 极光边界色阶 **218.7**、科幻 **224.3**（0–255 尺度），而水墨只有 9.4、可爱 4.8 |
| 舞台带描边 + 投影 | `.dash-chat-stage.has-bg` 有一圈 inset 白描边和外部投影，把背景框成一块「板」 |
| 四边硬裁 | 背景 canvas 贴满舞台，边缘被 `overflow: hidden` 一刀切掉，没有淡出 |

## 改法

1. **预设统一到纸面基调**：新增 `tone: "paper"` 与 `accent` 字段，所有预设的浅色底都对齐控制台的米白（约 `#f0f2ef`）。
   - 极光：淡青绿/淡紫的径向光晕，用 `multiply` 叠在纸面上，不再有夜空底
   - 科幻：浅冷灰蓝星图 + 淡网格，去掉深空底与霓虹描边
   - 水墨：远山墨色 alpha 从 0.1 降到 0.075，让纸色主导
2. **去掉「板感」**：`has-bg` 不再描边/投影；canvas 与蒙版加 `radial-gradient` 羽化，颜色从中部淡出到面板底色。
3. **气泡回到纸感**：浅色背景下不再需要深色毛玻璃，改为一层很薄的半透明白。
4. **新增背景浓度三档**（淡雅 0.32 / 适中 0.72 / 明显 1.15），外观页与对话页快捷条共用；`BgCanvas` 新增 `intensity` prop，所有绘制乘该系数。想更淡或更明显都能调，而不用换预设。
5. 修正 `normalizeChatBgIntensity` 把 `null` 当成 0 夹成下限的问题。

## 修正前后对比（0–255 色阶差）

| 预设 | 修正前（左/上边界） | 修正后（左/上边界） |
| --- | --- | --- |
| 极光 | 218.7 / 201.1 | **4.2 / 8.1** |
| 科幻 | 224.3 / 207.9 | **5.3 / 9.6** |
| 水墨 | 9.4 / 2.4 | **6.7 / 2.1** |
| 可爱 | 4.8 / 2.4 | 4.2 / 2.6 |

背景相对「无背景」基准仍有 **7–13** 的差异，说明柔和之后依然看得出来，没有调成看不见。

浓度档位生效（canvas 像素亮度跨度）：淡雅 13.4 → 适中 23.8 → 明显 35.1。

## 验证

- `npm run build`（含 `vue-tsc`）通过
- `npm run verify:chat-backgrounds`：12/12 通过（新增纸面基调、浓度单调、越界收敛、广播带浓度等用例）
- `npm run verify:chat-backgrounds-browser`：通过
- `verify:interaction-browser`、`verify:agent-activity`、`verify:codex-trajectory`、`verify:pending-interactions`、`verify:markdown`、`verify:custom-pixel-bg` 全部通过
- `verify:multi-agent` 的 Dashboard 用例失败是**更新前就存在的测试脚本缺陷**（脚本内的 `state` 桩缺少 `mainTurns`，与本次改动无关），补上桩后 13/13 通过

## 截图

- `appearance-panel.png` — 外观页（实时预览 + 卡片 + 浓度三档）
- `stage-none.png` — 无背景基准
- `stage-aurora-淡雅/适中/明显.png`、`stage-ink-*.png`、`stage-scifi-*.png` — 三档浓度下的实际渲染
- `edge-aurora.png` / `edge-ink.png` / `edge-scifi.png` / `edge-cute.png` — 带四周界面的边界截图，用于量化过渡
