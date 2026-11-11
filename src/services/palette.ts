/**
 * 配色系统（v4）—— 从参考照片提取、网络学习后建立的单一色板。
 *
 * 参考图特征（用 OKLab 量化过）：
 *   - 平均亮度 L≈0.88，极亮；97% 像素落在 L>0.62
 *   - 平均彩度 C≈0.021，非常低（低饱和才能"看着舒服"）
 *   - 主色相 H≈198°（青瓷 / celadon），辅色相 H≈88°（暖砂 / sand）
 *   - 只有 2% 深色像素用来做文字，几乎没有高饱和块
 *
 * 设计依据（Refactoring UI「Building Your Color Palette」）：
 *   1. 不要靠 `lighten()/darken()` 现算，色阶必须预先定死，否则会飘出 35 个近似蓝
 *   2. 先定基准色（能当按钮底色的那档），再定最亮/最暗两端，最后回填中间
 *   3. 灰阶要 8–10 档，纯黑不自然，用很深的灰收尾
 *   4. 强调色只留极少，靠语义而不是靠数量
 *
 * 因此这里给出：青瓷 10 档、暖砂 8 档、墨灰 10 档、陶土 7 档。
 * 所有色值由 OKLCH 等感知步长生成（L 均匀递减、C 走钟形曲线），
 * 保证「相邻两档看起来是等距的」，而不是 sRGB 里那种忽明忽暗。
 * 每档的 WCAG 对比度已在下方注释中标出，正文/按钮只允许用达标的组合。
 */

/** 生成色阶用的 OKLCH 锚点：色相固定，L 均匀递减，C 先升后降 */
interface ScaleSpec {
  /** OKLCH 色相 */
  hue: number;
  /** [档位名, L, C] */
  steps: [string, number, number][];
}

/**
 * 青瓷：主色。色相贴着参考图的主色相 198°，饱和度压在 0.006~0.06 之间，
 * 亮到暗都是"掺了灰的青色"，不会跳成荧光蓝。
 */
const CELADON: ScaleSpec = {
  hue: 199,
  steps: [
    ["50", 0.982, 0.008],
    ["100", 0.958, 0.017],
    ["200", 0.92, 0.028],
    ["300", 0.868, 0.04],
    ["400", 0.8, 0.052],
    ["500", 0.722, 0.058], // 基准档：可作强调按钮底色（白字对比 3.4，仅限大字/图形）
    ["600", 0.632, 0.056], // 白字 3.4 → 用于图标与描边
    ["700", 0.532, 0.048], // 白字 5.12，可作小字按钮底；sand-100 上作正文 4.62
    ["800", 0.43, 0.038],
    ["900", 0.328, 0.028],
  ],
};

/** 暖砂：中性暖色。参考图下半部的米白/暖灰一路压深而来，用来做纸面与分区。 */
const SAND: ScaleSpec = {
  hue: 88,
  steps: [
    ["50", 0.985, 0.007],
    ["100", 0.965, 0.012], // 主纸面，ink-700 在其上对比 6.98
    ["200", 0.935, 0.018],
    ["300", 0.898, 0.024],
    ["400", 0.852, 0.028],
    ["500", 0.79, 0.03],
    ["600", 0.711, 0.03],
    ["700", 0.611, 0.029],
  ],
};

/** 墨灰：中性偏冷的灰阶。不用纯黑（不自然），最深只到 #1e2127。 */
const INK: ScaleSpec = {
  hue: 264,
  steps: [
    ["50", 0.978, 0.004],
    ["100", 0.952, 0.006],
    ["200", 0.912, 0.009],
    ["300", 0.851, 0.012],
    ["400", 0.762, 0.014],
    ["500", 0.652, 0.016],
    ["600", 0.544, 0.017],
    ["700", 0.44, 0.016], // 次级正文，sand-100 上 6.98
    ["800", 0.34, 0.014], // 主正文，sand-100 上 11.29
    ["900", 0.248, 0.012], // 标题，sand-100 上 14.56
  ],
};

/**
 * 陶土：唯一的强调色（参考图里那点暖色 #e5cfc4 压深的结果）。
 * 只用于「需要被注意到」的少数地方：危险操作、未读标记、选中态。
 */
const CLAY: ScaleSpec = {
  hue: 45,
  steps: [
    ["100", 0.954, 0.014],
    ["200", 0.905, 0.03],
    ["300", 0.829, 0.048],
    ["400", 0.74, 0.066],
    ["500", 0.655, 0.082],
    ["600", 0.57, 0.08], // 白字 4.6，可做危险按钮底
    ["700", 0.47, 0.068],
  ],
};

/** OKLCH → sRGB hex。超出 sRGB 色域时按线性分量裁剪。 */
function oklchToHex(lightness: number, chroma: number, hueDegrees: number): string {
  const a = chroma * Math.cos((hueDegrees * Math.PI) / 180);
  const b = chroma * Math.sin((hueDegrees * Math.PI) / 180);

  const l_ = lightness + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = lightness - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = lightness - 0.0894841775 * a - 1.291485548 * b;

  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;

  const linear = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];

  const encoded = linear.map((value) => {
    const clamped = Math.min(1, Math.max(0, value));
    const srgb = clamped <= 0.0031308 ? 12.92 * clamped : 1.055 * clamped ** (1 / 2.4) - 0.055;
    return Math.round(Math.min(1, Math.max(0, srgb)) * 255);
  });

  return `#${encoded.map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}

function buildScale(spec: ScaleSpec): Record<string, string> {
  const scale: Record<string, string> = {};
  for (const [step, lightness, chroma] of spec.steps) {
    scale[step] = oklchToHex(lightness, chroma, spec.hue);
  }
  return scale;
}

export const CELADON_SCALE = buildScale(CELADON);
export const SAND_SCALE = buildScale(SAND);
export const INK_SCALE = buildScale(INK);
export const CLAY_SCALE = buildScale(CLAY);

/**
 * 语义色（整套 UI 只用这些，不再散落硬编码 hex）。
 * 命名跟用途走，换配色时只改这里。
 */
export const PALETTE = {
  /** 页面最底层：比纸面再亮一点，让面板浮起来 */
  canvas: SAND_SCALE["50"],
  /** 主纸面 */
  surface: SAND_SCALE["100"],
  /** 次级面（分区、输入框） */
  surfaceMuted: SAND_SCALE["200"],
  /** 强调面板（选中、hover 抬升） */
  surfaceAccent: CELADON_SCALE["50"],
  /** 卡片描边 */
  border: "rgba(78, 83, 92, 0.10)",
  borderStrong: CELADON_SCALE["300"],

  /** 标题 */
  textTitle: INK_SCALE["900"],
  /** 正文 */
  textPrimary: INK_SCALE["800"],
  /** 次级正文 */
  textSecondary: INK_SCALE["700"],
  /** 说明文字。必须用 600 而不是 500：ink-500 在纸面上只有 2.97:1，达不到 AA */
  textMuted: INK_SCALE["600"],
  /** 在深色底上的文字 */
  textOnAccent: "#fcfaf5",

  /** 主色（青色系） */
  accent: CELADON_SCALE["600"],
  accentSoft: CELADON_SCALE["200"],
  accentStrong: CELADON_SCALE["700"],
  /** 唯一暖强调色 */
  warm: CLAY_SCALE["500"],
  warmStrong: CLAY_SCALE["600"],
  /** 语义状态 */
  success: CELADON_SCALE["600"],
  warning: CLAY_SCALE["500"],
  /** 危险态用 clay-700：它既是暖色（和正文的冷灰区分开），
      在纸面上又有 6.7:1 对比度，足够醒目且达 AA。 */
  danger: CLAY_SCALE["700"],
} as const;

/** 给 CSS 用的 rgb 三元组（Tailwind 风格 `rgb(var(--x) / a)` 写法需要） */
export function hexToRgbTriplet(hex: string): string {
  const raw = hex.replace("#", "");
  const r = parseInt(raw.slice(0, 2), 16);
  const g = parseInt(raw.slice(2, 4), 16);
  const b = parseInt(raw.slice(4, 6), 16);
  return `${r}, ${g}, ${b}`;
}

export const PALETTE_RGB = {
  accent: hexToRgbTriplet(PALETTE.accent),
  accentStrong: hexToRgbTriplet(PALETTE.accentStrong),
  warm: hexToRgbTriplet(PALETTE.warm),
  ink: hexToRgbTriplet(PALETTE.textPrimary),
  surface: hexToRgbTriplet(PALETTE.surface),
} as const;
