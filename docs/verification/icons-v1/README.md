# 背景图标重做 + 换图（v8）

日期：2026-11-14

## 一、图标：从「彩色圆点」做成「色云缩略图」

### 参考图分析

你给的 `7d20f75e...png`（931×57）是背景选择条的一行截图，codex 逐像素分析出：

> soft shaded thumbnail tiles, with blurred gradients and restrained highlights...
> mostly a base gradient plus one or two blurred color patches

实测规格：
- tile 约 **19×19**，圆角约 8px
- 内部 = **基底渐变 + 1~2 个高斯模糊色斑**，不是纯色
- 深色 tile **无描边**，浅色 tile（无/简洁）有 1px 淡描边
- tile 下方一道柔和投影（黑 8~12% / dy 2 / blur 1.5~2）

### 三轮尝试与评分

| 方案 | codex 评分 | 问题 |
| --- | --- | --- |
| CSS 渐变 tile | 4/10 | "glossy gradient swatches, little spatial variation" |
| 照片裁切当缩略图 | 2~5/10 | 22px 下照片糊成一团，雨像红绿灯、樱花像字母 |
| **PIL 生成色云** | **8/10** | 达成 |

前两轮失败的关键教训：
- **照片缩到 22px 只能是一团色块**，看不出内容 —— 缩略图不该用照片
- 参考图本身就是抽象色云，不是照片

### 最终做法

用 PIL 离线生成 52×52 的 webp（显示 26px，2x 屏清晰）：

1. 铺基底渐变（左上→右下）
2. 在每个 tile 的**不同位置**画 2~3 个椭圆色斑
3. 对色斑图做高斯模糊（半径 28~34），用模糊后的蒙版叠回基底
4. 整图再模糊一次
5. 缩到目标尺寸 + 亮度/饱和校正 + 轻微噪点

**关键改动：每个 tile 的高光位置都错开**

| tile | 高光位置 | 理由 |
| --- | --- | --- |
| 极光 | 中上 | 极光在画面中央偏上 |
| 水墨 | 左中 | 云雾横向铺开 |
| 细雨 | 右下 | 雨滴反光在下方 |
| 科幻 | 右上 | 城市灯火在右上 |
| 樱花 | 左下 | 花瓣垂在枝下 |

这一条是评分从 4/10 提到 **8/10** 的决定性改动 ——
之前七个 tile 全是"左上亮、右下暗"，必然被读成"七个一样的立体按钮"。

同时**删掉了投影和统一高光**：参考图里的 tile 是平的，
加了投影立刻变成"凸起的按钮"。

## 二、换图：细雨与科幻

你说这两个不是想要的视觉效果。原来的问题：

| 背景 | 原图 | 问题 |
| --- | --- | --- |
| 细雨 | 玻璃雨滴 + 彩色 bokeh | 压暗后只剩色块，读不出"雨" |
| 科幻 | 长曝光车流光轨 | 压暗模糊后被视觉模型判成 CGI（4/10） |

换成：

| 背景 | 新图 | 原分辨率 | codex 评分 |
| --- | --- | --- | --- |
| 细雨 | Rain-streaked glass with red and teal lights | 3000×2000 | photo 9/10，明确读作"雨" |
| 科幻 | Skyscrapers rising through dense fog | 5893×3927 | photo 10/10，雾中摩天楼 |

同时把两张图的**背景本体提亮**（0.46→0.60 / 0.44→0.58）——
压太暗会丢失内容，而白字气泡的可读性靠暗角和底部渐变压住，不需要整图那么暗。

## 三、樱花：换成林间场景，贴近参考图

你说樱花要 `ab05d00a` 那样的效果。原图是**单枝特写**，而参考图是
**暗调林间、多枝、浅灰粉花、低饱和**。

对比了 4 张候选：

| 候选 | 内容 | likeness |
| --- | --- | --- |
| sakura-e（原） | 单枝特写，满树粉 | 风格不符（太亮太满） |
| sakura-j | 林间村落，村庄占主导 | 5/10 |
| **sakura-k（选用）** | 暗调林间多树 + 水岸 | **9/10** |

处理时**降饱和到 0.72**（参考图的核心特征就是低饱和灰调），
亮度 0.68，模糊 0.9 —— 复评 likeness **9/10**。

## 四、验证

| 项 | 结果 |
| --- | --- |
| tile 色云评分 | **8/10**（原 4/10） |
| 樱花 likeness | **9/10** |
| 细雨「能读出是雨」 | 6/10（原 4/10，"dense droplets suggest heavier rain"） |
| 科幻「能读出是科幻」 | 6/10（原 4/10，"reads more as modern Dubai than distinctly sci-fi"） |

- `verify:palette` 12/12
- `verify:chat-backgrounds` 13/13
- multi-agent / chat-sessions / agent-activity / pending-interactions /
  markdown / custom-pixel-bg 全部 fail 0
- `verify:chat-backgrounds-browser`、`verify:interaction-browser` 通过
- `vue-tsc` 与 `vite build` 通过

## 五、素材清单

`src/assets/bg/`：

| 文件 | 用途 | 大小 |
| --- | --- | --- |
| aurora.webp 2560×1440 | 极光背景 | 80 KB |
| ink.webp 2560×1441 | 水墨背景 | 584 KB |
| rain.webp 2560×1707 | 细雨背景 | 109 KB |
| scifi.webp 2560×1706 | 科幻背景 | 92 KB |
| sakura.webp 2560×1440 | 樱花背景 | 412 KB |
| tile-*.webp 52×52 | 选择器缩略图 | 0.2~0.4 KB 各 |

## 六、截图

- `bgbar-now.png` — 背景选择条（tile 色云效果）
- `photo-*.png` — 五个背景各自的效果
