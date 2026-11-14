# 真实照片背景（v7）：极光/水墨/细雨/科幻/樱花

日期：2026-11-14

## 一、你的参考图：codex 视觉分析

你的参考图 `ab05d00a...jpg`（1200×2670）被 codex 判定为：

> **photographic botanical close-up, subdued woodland photography,
> overcast natural light, low saturation, dark exposure, cool-neutral grading,
> diagonal branch composition, moderate depth of field**

关键量化特征：

| 项 | 值 |
| --- | --- |
| 风格 | 真实摄影（不是插画/渲染） |
| 整体明度 | **暗**（大片 `#191E1A` ~ `#252B25` 深墨绿） |
| 饱和 | **低**（橄榄绿、灰粉） |
| 高光 | 花瓣 `#C9CBC7` 灰白，**不到纯白** |
| 光 | 阴天漫射，**没有 bloom/发光/体积光** |
| 景深 | 中等，背景树干可辨认但细节减弱 |
| 纹理 | 有噪点/颗粒，非刻意加的胶片颗粒 |

**核心结论：你要的是「真实摄影的质感」，而不是「手绘渐变」** ——
这直接否定了我之前用 Canvas 画程序化图案的整个路线。

## 二、壁纸来源与筛选

从 **wallhaven.cc** 搜索下载了 25 张候选（分辨率 2560~8248 宽），
逐张用 codex 做「写真度评分」，只保留 photo ≥ 9/10 的：

| 主题 | 最终选用 | 原分辨率 | codex 评分 | 内容 |
| --- | --- | --- | --- | --- |
| 极光 | aurora-c | 3840×2160 | **photo 10/10** | 雪山与湖面上的极光 |
| 水墨 | ink-d | 4592×2584 | photo 9/10 | 云雾中的层叠山脊 |
| 细雨 | rain-c | 2560×1600 | photo 9/10 | 玻璃雨滴 + 窗外彩色散景 |
| 科幻 | scifi-e | 5288×2975 | photo 9/10 | 长曝光车流光轨 |
| 樱花 | sakura-e | 7360×4912 | **photo 10/10** | 樱花枝条，柔焦淡彩 |

被淘汰的例子（避免踩坑）：
- `ink-a` 5443×3061 — codex 判定 "abstract geometric background, photo 1/10"
- `scifi-b` 3840×2161 — "Illustration, photorealism 2/10"
- `scifi-c` 2560×1440 — "Illustration, photorealism 2/10"
- `sakura-a` 5120×2880 — 虽是真实照片 10/10，但太亮太满，不适合做背景

**wallhaven 的 `scifi` 关键词大量返回插画**，所以科幻最后选了
「长曝光车流光轨」这种真实摄影题材来代替。

## 三、图片处理

统一流水线（离线跑，脚本见提交记录）：

```
原图 → 缩到 2560 宽（LANCZOS）→ 高斯模糊（忙碌的图）→ 压暗 → webp q80
```

| 主题 | 亮度系数 | 模糊半径 | 输出 |
| --- | --- | --- | --- |
| 极光 | 0.72 | 0 | 2560×1440 80KB |
| 水墨 | 0.78 | 0 | 2560×1441 584KB |
| 细雨 | 0.52 | 3.2 | 2560×1600 82KB |
| 科幻 | 0.46 | 2.6 | 2560×1440 69KB |
| 樱花 | 0.56 | 3.6 | 2560×1709 81KB |

压暗的理由：codex 评审未经处理的原始照片时指出
"too bright and visually busy, white bubbles may blend in"（4/10），
压暗 + 模糊后重新评分升到 **7~8/10**。

## 四、渲染实现

新增 `src/services/bgPhotos.ts` 注册表，`BgCanvas.vue` 增加照片绘制路径：

1. **cover 铺满**（绝不拉伸 —— 拉伸一眼假）
2. **ken-burns 漂移**：缩放 1.05±0.028、平移 ±0.018W，周期约 40 秒
3. **色调统一层**：每张图叠一层自己的 tint，避免不同照片色彩打架
4. **暗角**：压住四角，中间气泡区保持视觉中心
5. **顶部/底部压暗**：标题栏和输入框区域保持可读
6. **保留交互层**：鼠标涟漪 + 点击光点叠在照片上（照片是静的，加一点活气）

照片模式的绘制在 `draw()` 开头拦截，不再叠加任何程序化图案。

## 五、「可爱」→「樱花」

按你的要求直接改名并换内容：

| 项 | 改前 | 改后 |
| --- | --- | --- |
| id | `cute` | `sakura` |
| 名称 | 可爱 | **樱花** |
| 图标 | 🐾 | 樱 |
| 描述 | 樱花飘落与糖果色气泡 | 真实摄影：压暗的樱花枝，浅灰粉的花瓣与深色枝干 |
| 配色 | 糖果粉 | 深炭灰底 + 灰粉 |

同步更新了两个校验脚本里的 `cute` → `sakura`。

## 六、验证

### codex 视觉复评（每个背景截图）

| 背景 | 评分 | 判定 |
| --- | --- | --- |
| 极光 | **9/10** | photo，文字可读 |
| 水墨 | **9/10** | photo，文字可读 |
| 细雨 | **9/10** | photo，文字可读 |
| 科幻 | 8/10 | photo，文字可读 |
| 樱花 | **10/10** | photo，文字可读 |

对比之前的程序化绘制版本（极光 6/10、细雨 6/10），这次是质的提升。

### 自动化

- `verify:palette` 12/12
- `verify:chat-backgrounds` 13/13
- multi-agent / chat-sessions / agent-activity / pending-interactions /
  markdown / custom-pixel-bg 全部 fail 0
- `verify:chat-backgrounds-browser`、`verify:interaction-browser` 通过
- `vue-tsc` 与 `vite build` 通过

### 顺手修正的一条过时断言

`verify-chat-backgrounds` 里原本有一条「所有预设 swatch 亮度 >= 150」，
它编码的是上一代"浅色纸面光晕"的设计约束。
背景换成真实照片后深色底是**必需的**（否则白字气泡压不住），
所以这条断言按新设计意图重写了：
- 「无 / 简洁 / 自定义」仍是浅色图案 → 保持亮度 >= 150
- 照片背景 → 必须有压暗底色，且彩度仍 <= 100（不出现荧光色）

## 七、截图

- `photo-aurora.png` / `photo-ink.png` / `photo-rain.png` /
  `photo-scifi.png` / `photo-sakura.png`
- `photo-full-chat.png` — 完整对话页效果
