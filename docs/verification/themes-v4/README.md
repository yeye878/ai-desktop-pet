# 主题皮肤 + 对话背景 v4

日期：2026-11-11

## 一、修复「换皮肤界面颜色不变」

**根因**：上一轮把 `--dash-accent-rgb` 写死成了青瓷（141 处界面用色都引用它），
而皮肤切换只改 `--pet-primary`，两者没打通。所以点皮肤时：
- 按钮变了一点点（因为它用 `--pet-primary`）
- 侧栏、卡片、选中态、描边全部纹丝不动（因为它们用 `--dash-accent-*`）

**修法**：`stores/pet.ts` 的 `applyCssVars()` 现在会从主题主色派生整套纸面色阶：

```
primary → OKLab 混白 → paper-tint-50/100/150/200/300
        → --dash-accent / --dash-accent-rgb / --dash-accent-strong
```

关键细节：**必须在 OKLab 里混，且用纯白打底**。
试过在 sRGB 里混、或用暖砂打底，结果青瓷混出来是 `#e7ece8`（脏绿灰，像褪色）；
OKLab + 白色才是干净的 `#eaf1f1`。

`dashboard.css` 里写死的青纸层（22 处）同步换成 `rgba(var(--paper-tint-*-rgb), a)`，
这样侧栏、内容区、卡片全都会跟着皮肤走。

**验证**（浏览器实测，12 个主题）：
- accent 各不相同：`#609497 #4a7577 #5b6480 #6f8f74 #bb8166 #a97f92 #7d8aa3 #8f7a63 #5a6b86 #79875f #b58485 #7e93a8`
- 侧栏底色 12 个互不相同（此前全是同一个青纸色）
- `paper-tint-150` 12 个互不相同

## 二、新增 5 个主题皮肤

原有 7 个 + 新增：**月白 / 茶褐 / 靛青 / 苔痕 / 柚粉**，共 **12 个**。

全部沿用 v4 配色纪律：明度 L≥0.85、彩度 C≤0.06、成对给出 primary/primaryDark/accent。
没有引入任何高饱和色，避免破坏「纸面」整体感。

## 三、对话背景新增 4 个预设

原有：无 / 极光 / 水墨 / 可爱 / 科幻 / 简洁 / 自定义
新增：**晨雾 / 细雨 / 黄昏 / 花影**，共 **11 个**

| 预设 | 画面 |
| --- | --- |
| 晨雾 | 4 层雾带由下往上缓慢抬升 + 极细浮尘，点击把雾推开 |
| 细雨 | 斜向雨丝 + 玻璃水珠高光 + 落点涟漪，点击溅起水花 |
| 黄昏 | 底部两角暖橙余晖 + 顶部淡紫补光 + 浮尘，点击亮一下（会自动回收） |
| 花影 | 花枝投影 + 缓慢飘落花瓣，点击散落花瓣 |

四个都是「浅纸面 + 低对比」路线，与 v3 确定的柔和基调一致。

**实测每个预设的 canvas 统计**（颜色种类 / 明暗跨度 / 平均亮度）：

| 预设 | colors | spread | avg |
| --- | --- | --- | --- |
| 极光 | 19 | 21.4 | 243.6 |
| 水墨 | 11 | 21.4 | 237.7 |
| 晨雾 | 13 | 30.6 | 237.8 |
| 细雨 | 14 | 39.1 | 233.6 |
| 黄昏 | 32 | 38.1 | 226.2 |
| 花影 | 15 | 60.3 | 235.8 |
| 可爱 | 65 | 68.9 | 224.7 |
| 科幻 | 37 | 62.7 | 239.8 |
| 简洁 | 29 | 49.8 | 240.0 |

全部满足：colors≥8（有内容）、spread≥3（有层次）、avg≥180（保持纸面基调）。

## 四、验证

- `npm run verify:palette` 12/12
- `npm run verify:chat-backgrounds` 13/13（新增「预设数量≥10」「无深底/荧光色」用例）
- `verify:chat-backgrounds-browser` 通过（11 个预设逐个渲染）
- `verify:interaction-browser` 通过
- `npm run build`（含 vue-tsc）通过

## 五、文件

- `src/stores/pet.ts` —— 12 个主题 + 主题色派生 CSS 变量
- `src/services/palette.ts` —— `themeToneScale()` / `mixAccentIntoPaper()`（OKLab 混色）
- `src/services/chatBackgrounds.ts` —— 11 个背景预设
- `src/components/BgCanvas.vue` —— 新增 4 套渲染器（mist/rain/dusk/bloom）
- `src/assets/dashboard.css` —— 语义色改为可被主题覆盖
- 截图：`theme-*.png`（12 个主题）、`bg-*.png`（9 个背景）、`chat-bgbar.png`