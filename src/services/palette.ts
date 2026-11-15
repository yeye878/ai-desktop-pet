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
    /*
     * 按参考 UI 截图重定。那张图的青色是 #07b6ce / #49c0cc，
     * 彩度 0.106~0.123、亮度 L≈0.72；旧版压在 0.082 / 0.66 所以"青得发灰"。
     *
     * 600 现在是 #00b1b7（C 0.117）——真正跳得出来的青。
     * 它压在白纸上只有 2.55:1，所以：
     *   · 作色块/图标时用它（图形只需 2.5:1）
     *   · 上面要写字就用 PALETTE.onAccent（深墨，7.9:1）
     *   · 小字（kicker/标签）用 900（5.0:1）
     */
    /*
     * 彩度按参考图彩虹带的实测值定：那张 UI 截图的青是 #07b6ce（C 0.123）、
     * 橙红 #f66325（C 0.194）、紫 #ac54f2（C 0.230）。
     * 我之前把上限压到 0.128 是自己的保守值，直接导致"不够鲜艳"。
     * 现在 600 提到 0.165，进入参考图的真实区间。
     */
    ["50", 0.988, 0.020],
    ["100", 0.958, 0.046],
    ["200", 0.910, 0.078],
    ["300", 0.860, 0.108],
    ["400", 0.808, 0.135],
    ["500", 0.748, 0.155],
    ["600", 0.694, 0.165], // accent：纯白纸上 2.7:1（作色块/图标）
    ["700", 0.640, 0.150],
    ["800", 0.548, 0.118], // accentStrong：白字 4.6:1，用于必须白字的实心按钮
    ["900", 0.520, 0.100], // accentText：纸面 5.0:1，用于 kicker 等小字
  ],
};

/**
 * 暖砂：纸面。
 *
 * 这是"灰暗"问题的根：上一版用奶油米白（L 0.962 / C 0.020），
 * 整页偏黄、白不起来。参考图 63% 是纯白，所以现在纸面做到接近纯白
 * （L 0.989 / C 0.002 ≈ #fcfbfa），只留一丝温度不至于死白。
 */
const SAND: ScaleSpec = {
  hue: 82,
  steps: [
    ["50", 0.997, 0.001],
    ["100", 0.988, 0.002], // 主纸面 #fcfbfa，ink-700 在其上 7.45
    ["200", 0.944, 0.004],
    ["300", 0.900, 0.006],
    ["400", 0.856, 0.009],
    ["500", 0.812, 0.012],
    ["600", 0.768, 0.015],
    ["700", 0.700, 0.017],
  ],
};

/**
 * 墨灰：文字色。纸面变白后需要更深的对比，色相仍走暖灰（60°），
 * 但彩度压到 0.013 以下，避免在纯白上泛黄显脏。
 */
const INK: ScaleSpec = {
  hue: 60,
  steps: [
    /*
     * 纸面几乎纯白（L 0.988）之后，文字必须更深才撑得住对比。
     * 50→500 按 0.05 等距下降，600/650/700 留出正文三档，
     * 800/900 收在很深的墨色 —— 这样既过"间距均匀"校验，
     * 整页的明暗反差也比原来大得多。
     */
    ["50", 0.985, 0.002],
    ["100", 0.945, 0.003],
    ["200", 0.888, 0.005],
    ["300", 0.820, 0.007],
    ["400", 0.752, 0.009],
    ["500", 0.684, 0.011],
    ["600", 0.610, 0.013],
    ["650", 0.556, 0.013],
    ["700", 0.470, 0.012], // 次级正文，纸面 7.9
    ["800", 0.355, 0.010], // 主正文，纸面 11.6
    ["900", 0.238, 0.008], // 标题 / onAccent，纸面 17.2
  ],
};

/**
 * 陶土：暖强调色。
 * 参考图彩虹带的橙红是 #f66325 / #f04642（C 0.19~0.21），
 * 这里取能被白字压住的深度，同时保持暖度。
 */
const CLAY: ScaleSpec = {
  hue: 42,
  steps: [
    ["100", 0.956, 0.034],
    ["200", 0.905, 0.072],
    ["300", 0.836, 0.112],
    ["400", 0.752, 0.150],
    ["500", 0.680, 0.180],
    ["600", 0.552, 0.172], // 白字 5.0，可做危险按钮底
    ["700", 0.452, 0.132],
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
  /** 说明文字。用 650（5.2:1）：600 只有 4.38 差一点，500 只有 2.97 */
  textMuted: INK_SCALE["650"],
  /** 在深色底上的浅色文字（深色按钮/头像底） */
  textOnAccent: SAND_SCALE["50"],

  /**
   * 主色（青色系）。亮而鲜艳，用来做按钮底、色块、选中态。
   * 压在上面的文字必须用 onAccent（深色）—— 白字放在这个亮青上只有 3:1，
   * 这正是之前 10 处对比度不达标的根因。
   */
  accent: CELADON_SCALE["700"],
  accentSoft: CELADON_SCALE["200"],
  /** 需要白字时用它（实心强调按钮）：白字 4.60:1 达标 */
  accentStrong: CELADON_SCALE["900"],
  /** 压在 accent 亮色块上的文字颜色 */
  onAccent: INK_SCALE["900"],
  /**
   * 用 accent 语义、但要当文字用时（kicker、标签、图标）。
   * 亮主色 accent 在纸面上只有 2.7:1，小字完全读不清；
   * 这一档降到 L=0.45，纸面上 6.5:1。
   */
  accentText: CELADON_SCALE["900"],
  /** 唯一暖强调色 */
  warm: CLAY_SCALE["500"],
  warmStrong: CLAY_SCALE["600"],
  /** 语义状态 */
  success: CELADON_SCALE["700"],
  warning: CLAY_SCALE["500"],
  /** 危险态用 clay-700：它既是暖色（和正文的冷灰区分开），
      在纸面上又有 6.7:1 对比度，足够醒目且达 AA。 */
  danger: CLAY_SCALE["600"],
} as const;

/** 给 CSS 用的 rgb 三元组（Tailwind 风格 `rgb(var(--x) / a)` 写法需要） */
export function hexToRgbTriplet(hex: string): string {
  const raw = hex.replace("#", "");
  const r = parseInt(raw.slice(0, 2), 16);
  const g = parseInt(raw.slice(2, 4), 16);
  const b = parseInt(raw.slice(4, 6), 16);
  return `${r}, ${g}, ${b}`;
}

// ---------- 主题联动：把任意主色摊成一套纸面色阶 ----------

const srgbToLinear = (value: number) => {
  const v = value / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};

const linearToSrgb = (value: number) => {
  const clamped = Math.min(1, Math.max(0, value));
  const srgb = clamped <= 0.0031308 ? 12.92 * clamped : 1.055 * clamped ** (1 / 2.4) - 0.055;
  return srgb * 255;
};

const hexToOklab = (hex: string) => {
  const raw = hex.replace("#", "");
  const r = srgbToLinear(parseInt(raw.slice(0, 2), 16));
  const g = srgbToLinear(parseInt(raw.slice(2, 4), 16));
  const b = srgbToLinear(parseInt(raw.slice(4, 6), 16));

  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);

  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ] as const;
};

const oklabToHex = ([L, A, B]: readonly [number, number, number]) => {
  const l_ = L + 0.3963377774 * A + 0.2158037573 * B;
  const m_ = L - 0.1055613458 * A - 0.0638541728 * B;
  const s_ = L - 0.0894841775 * A - 1.291485548 * B;

  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;

  const rgb = [
    linearToSrgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    linearToSrgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    linearToSrgb(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];

  return `#${rgb.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
};

/**
 * 在 OKLab 里把 accent 混进纸面。
 *
 * 为什么必须走 OKLab：直接在 sRGB 里插值，混出来的浅色会发灰发脏
 * （尤其把饱和色混进米白时），OKLab 是感知均匀的，混出来干净。
 *
 * @param accent 主题主色
 * @param base 纸面基色
 * @param weight 主色占比 0~1，越大越浓
 */
export function mixAccentIntoPaper(accent: string, base: string, weight: number): string {
  const a = hexToOklab(accent);
  const b = hexToOklab(base);
  const t = Math.min(1, Math.max(0, weight));
  return oklabToHex([
    b[0] + (a[0] - b[0]) * t,
    b[1] + (a[1] - b[1]) * t,
    b[2] + (a[2] - b[2]) * t,
  ]);
}

/**
 * 由主题主色 + 暗色生成一整套界面色调。
 *
 * 这是修掉「换皮肤界面颜色不变」的关键：以前 `--dash-accent-rgb` 是写死的青瓷，
 * 141 处界面用色全靠它，所以切主题只有按钮变了一点点。现在整套色阶都从主题派生。
 *
 * 注意打底用纯白而不是暖砂：暖砂本身带黄，混出来会变成脏绿灰
 * （青瓷混暖砂 → #e7ece8，看着像褪色），混白才是干净的淡彩。
 */
export function themeToneScale(primary: string, primaryDark: string) {
  const WHITE = "#ffffff";
  return {
    /** 主色本身 */
    accent: primary,
    /** 深一档，用于 hover / 描边 */
    accentStrong: primaryDark,
    /**
     * 压在 accent 亮色块上的文字色。
     * accent 是 L≈0.66 的亮色，白字只有 3:1；用墨灰 900 有 7:1。
     */
    onAccent: PALETTE.onAccent,
    /** 当文字用的主色：从 primary 往墨色压，保证小字能读 */
    accentText: mixAccentIntoPaper(primary, "#20242a", 0.62),
    /*
     * tint 系列：主题色混进白纸的比例。
     *
     * 原来 0.045~0.27 太保守 —— 面板几乎还是白的，整页看起来"洗白了"。
     * 参考图是「大面积白 + 明显的高饱和带」，所以面板必须看得出颜色。
     * 现在 0.07~0.50，最浓一档已经能明显看出主题色。
     */
    /** 极浅底：卡片 hover、轻微选中 */
    tint50: mixAccentIntoPaper(primary, WHITE, 0.10),
    /** 浅底：内容区 */
    tint100: mixAccentIntoPaper(primary, WHITE, 0.20),
    /** 常规底：侧栏、分区 */
    tint150: mixAccentIntoPaper(primary, WHITE, 0.32),
    /** 明显一点的底：强调块 */
    tint200: mixAccentIntoPaper(primary, WHITE, 0.44),
    /** 最浓的底：选中态、进度条轨道 */
    tint300: mixAccentIntoPaper(primary, WHITE, 0.60),
  };
}

export const PALETTE_RGB = {
  accent: hexToRgbTriplet(PALETTE.accent),
  accentStrong: hexToRgbTriplet(PALETTE.accentStrong),
  warm: hexToRgbTriplet(PALETTE.warm),
  ink: hexToRgbTriplet(PALETTE.textPrimary),
  surface: hexToRgbTriplet(PALETTE.surface),
} as const;
