import { defineStore } from "pinia";
import { ref, computed, watch } from "vue";
import type { CustomPixelPetAsset } from "../services/customPixelPet";
import type { PetCharacterId } from "../services/petCharacters";
import { themeToneScale } from "../services/palette";

export type PetState =
  | "idle"
  | "listening"
  | "thinking"
  | "speaking"
  | "working"
  | "sleeping"
  | "happy"
  | "confused"
  | "waving"
  | "hungry"
  | "stuffed"
  | "refusing"
  | "dragging";

// ===== 主题系统 =====
export interface Theme {
  id: string;
  name: string;
  primary: string;       // 主色（头部/按钮/桌宠身体）
  primaryDark: string;    // 主色暗色（hover）
  accent: string;         // 强调色（装饰/高亮）
  bgGlass: string;        // 面板毛玻璃背景
  bubbleUser: string;     // 用户气泡背景
  bubbleBot: string;      // AI 气泡背景
  bubbleBotText: string;  // AI 气泡文字色
  headerGradient: string; // 头部渐变
  particleColors: string[]; // 粒子颜色组
  animated?: boolean;     // 是否随时间动态变色
  cycleColors?: string[]; // 动态变色调色盘
}

/**
 * 主题（v4「青瓷」）—— 从一张参考照片提取、按 OKLCH 等感知步长重建的低饱和纸面系。
 *
 * 旧版是一堆高饱和糖果色（#609497 / #7b849f / #6f8f74 …），单看很跳，
 * 放在一起就吵：色相散、明度接近、互相抢。新方案沿用参考照片的规律：
 *   明度高（L≈0.88）、饱和低（C≈0.02）、色相只留青瓷(198°) 与暖砂(88°) 两族，
 * 每个主题靠「色相微调 + 明度差」区分，而不是靠把饱和度拉满。
 *
 * 保留 default / ocean / galaxy / forest / sunset / sakura 这些 id 是为了
 * 不让老用户的已选皮肤失效（后端 DB 里存的就是这些字符串）。
 */
export const THEMES: Record<string, Theme> = {
  default: {
    id: "default",
    name: "青瓷",
    primary: "#04b1b7",
    primaryDark: "#148286",
    accent: "#75d4d8",
    bgGlass: "rgba(252, 250, 245, 0.92)",
    bubbleUser: "linear-gradient(135deg, #008389, #00959a)",
    bubbleBot: "rgba(4, 177, 183, 0.07)",
    bubbleBotText: "#34383f",
    headerGradient: "linear-gradient(135deg, #008d93, #11b4ba)",
    particleColors: ["#008388", "#02a6ac", "#4ac6cb", "#93e5e8"],
  },
  ocean: {
    id: "ocean",
    name: "雨过天青",
    primary: "#09b0be",
    primaryDark: "#15818b",
    accent: "#76d3dd",
    bgGlass: "rgba(243, 251, 251, 0.92)",
    bubbleUser: "linear-gradient(135deg, #00828f, #0094a1)",
    bubbleBot: "rgba(9, 176, 190, 0.07)",
    bubbleBotText: "#2b3238",
    headerGradient: "linear-gradient(135deg, #008c99, #14b3c1)",
    particleColors: ["#00828e", "#05a5b3", "#4ac5d2", "#93e4ed"],
  },
  galaxy: {
    id: "galaxy",
    name: "远山黛",
    primary: "#789ae4",
    primaryDark: "#5871a7",
    accent: "#a6c1fd",
    bgGlass: "rgba(248, 249, 252, 0.92)",
    bubbleUser: "linear-gradient(135deg, #536faf, #6180c4)",
    bubbleBot: "rgba(120, 154, 228, 0.07)",
    bubbleBotText: "#2f333d",
    headerGradient: "linear-gradient(135deg, #5a78bc, #7b9ce7)",
    particleColors: ["#5671ac", "#7090d7", "#8fb0f6", "#bbd4ff"],
  },
  forest: {
    id: "forest",
    name: "鼠尾草",
    primary: "#62b074",
    primaryDark: "#498156",
    accent: "#98d3a3",
    bgGlass: "rgba(247, 250, 246, 0.92)",
    bubbleUser: "linear-gradient(135deg, #3e8250, #4c945e)",
    bubbleBot: "rgba(98, 176, 116, 0.07)",
    bubbleBotText: "#2f3830",
    headerGradient: "linear-gradient(135deg, #448c56, #65b377)",
    particleColors: ["#448253", "#5ca56d", "#7dc58c", "#afe4b9"],
  },
  sunset: {
    id: "sunset",
    name: "陶土",
    primary: "#d88260",
    primaryDark: "#9f6047",
    accent: "#f4af94",
    bgGlass: "rgba(252, 249, 244, 0.92)",
    bubbleUser: "linear-gradient(135deg, #a45b3e, #b86a4b)",
    bubbleBot: "rgba(216, 130, 96, 0.07)",
    bubbleBotText: "#3b332e",
    headerGradient: "linear-gradient(135deg, #b06244, #db8563)",
    particleColors: ["#a25e43", "#cb7a5a", "#eb9a7a", "#ffc4ab"],
  },
  sakura: {
    id: "sakura",
    name: "藕荷",
    primary: "#d27ca6",
    primaryDark: "#9a5c7a",
    accent: "#efaaca",
    bgGlass: "rgba(252, 248, 249, 0.92)",
    bubbleUser: "linear-gradient(135deg, #9f567a, #b2658b)",
    bubbleBot: "rgba(210, 124, 166, 0.07)",
    bubbleBotText: "#3a3237",
    headerGradient: "linear-gradient(135deg, #aa5d83, #d57fa9)",
    particleColors: ["#9d597a", "#c5749c", "#e594bb", "#febfdc"],
  },
  moon: {
    id: "moon",
    name: "月白",
    primary: "#3ca8d6",
    primaryDark: "#2f7c9d",
    accent: "#82cdf1",
    bgGlass: "rgba(248, 250, 253, 0.92)",
    bubbleUser: "linear-gradient(135deg, #127ca3, #248db7)",
    bubbleBot: "rgba(60, 168, 214, 0.07)",
    bubbleBotText: "#30343b",
    headerGradient: "linear-gradient(135deg, #1785af, #3fabd9)",
    particleColors: ["#237ca1", "#379ec9", "#60bee8", "#9ddfff"],
  },
  tea: {
    id: "tea",
    name: "茶褐",
    primary: "#d38750",
    primaryDark: "#9b633c",
    accent: "#f0b38a",
    bgGlass: "rgba(251, 249, 245, 0.92)",
    bubbleUser: "linear-gradient(135deg, #a05f2f, #b46e3c)",
    bubbleBot: "rgba(211, 135, 80, 0.07)",
    bubbleBotText: "#37322c",
    headerGradient: "linear-gradient(135deg, #ac6734, #d68a53)",
    particleColors: ["#9e6236", "#c77f4b", "#e69e6d", "#fec7a2"],
  },
  indigo: {
    id: "indigo",
    name: "靛青",
    primary: "#689ee3",
    primaryDark: "#4d74a6",
    accent: "#9bc5fc",
    bgGlass: "rgba(247, 249, 252, 0.92)",
    bubbleUser: "linear-gradient(135deg, #4573ae, #5284c2)",
    bubbleBot: "rgba(104, 158, 227, 0.07)",
    bubbleBotText: "#2c3138",
    headerGradient: "linear-gradient(135deg, #4b7cba, #6ba1e6)",
    particleColors: ["#4974ab", "#6195d6", "#82b4f5", "#b1d7ff"],
  },
  moss: {
    id: "moss",
    name: "苔痕",
    primary: "#83aa59",
    primaryDark: "#617d43",
    accent: "#afce90",
    bgGlass: "rgba(249, 250, 245, 0.92)",
    bubbleUser: "linear-gradient(135deg, #5d7e38, #6c8f44)",
    bubbleBot: "rgba(131, 170, 89, 0.07)",
    bubbleBotText: "#33362c",
    headerGradient: "linear-gradient(135deg, #64873d, #86ad5c)",
    particleColors: ["#5f7e3d", "#7ba054", "#9abf75", "#c3dfa8"],
  },
  coral: {
    id: "coral",
    name: "柚粉",
    primary: "#db7d76",
    primaryDark: "#a15d57",
    accent: "#f7aba5",
    bgGlass: "rgba(253, 249, 249, 0.92)",
    bubbleUser: "linear-gradient(135deg, #a65752, #bb6660)",
    bubbleBot: "rgba(219, 125, 118, 0.07)",
    bubbleBotText: "#3a3131",
    headerGradient: "linear-gradient(135deg, #b25e59, #de8079)",
    particleColors: ["#a45a55", "#ce766f", "#ee968e", "#ffc0ba"],
  },
  aurora: {
    id: "aurora",
    name: "流光",
    primary: "#9491e1",
    primaryDark: "#6d6ba5",
    accent: "#bbbafa",
    bgGlass: "rgba(249, 250, 251, 0.9)",
    bubbleUser: "linear-gradient(135deg, #6c68ac, #7b77c1)",
    bubbleBot: "rgba(148, 145, 225, 0.07)",
    bubbleBotText: "#2f353b",
    headerGradient: "linear-gradient(135deg, #7470b9, #9793e4)",
    particleColors: ["#6d6aa9", "#8b88d4", "#aaa7f3", "#ceceff"],
    animated: true,
    cycleColors: ["#7e93a8", "#a8b8a8", "#c2b6a4", "#97c8ca", "#b6b0c2"],
  },
};

// 向后兼容旧的 SKIN_COLORS（PetCanvas 需要读取身体颜色）
export const SKIN_COLORS: Record<string, string> = Object.fromEntries(
  Object.entries(THEMES).map(([id, t]) => [id, t.primary])
);
// 旧 key 映射
SKIN_COLORS["blue"] = THEMES.ocean.primary;
SKIN_COLORS["purple"] = THEMES.galaxy.primary;
SKIN_COLORS["green"] = THEMES.forest.primary;
SKIN_COLORS["orange"] = THEMES.sunset.primary;
SKIN_COLORS["pink"] = THEMES.sakura.primary;

const SKIN_ALIASES: Record<string, string> = {
  blue: "ocean",
  purple: "galaxy",
  green: "forest",
  orange: "sunset",
  pink: "sakura",
};

export function resolveSkinId(skinId: string) {
  return SKIN_ALIASES[skinId] || skinId;
}

// 预设字体颜色
export const FONT_COLORS = [
  { id: "default", name: "默认", value: "" },
  { id: "dark", name: "深色", value: "#2c2c2c" },
  { id: "warm", name: "暖灰", value: "#5c4b3a" },
  { id: "cool", name: "冷灰", value: "#3a4a5c" },
  { id: "brown", name: "咖啡", value: "#6b4423" },
  { id: "navy", name: "藏蓝", value: "#1e3a5f" },
];

function hexToRgb(hex: string) {
  const safeHex = hex.replace("#", "");
  const num = parseInt(safeHex.length === 3
    ? safeHex.split("").map((char) => char + char).join("")
    : safeHex, 16);
  return {
    r: (num >> 16) & 0xff,
    g: (num >> 8) & 0xff,
    b: num & 0xff,
  };
}

function rgbToHex(r: number, g: number, b: number) {
  return `#${[r, g, b]
    .map((value) => Math.round(value).toString(16).padStart(2, "0"))
    .join("")}`;
}

function mixHex(from: string, to: string, amount: number) {
  const a = hexToRgb(from);
  const b = hexToRgb(to);
  const t = Math.max(0, Math.min(1, amount));
  return rgbToHex(
    a.r + (b.r - a.r) * t,
    a.g + (b.g - a.g) * t,
    a.b + (b.b - a.b) * t,
  );
}

function colorAt(colors: string[], phase: number) {
  if (colors.length === 0) return "#609497";
  const wrapped = ((phase % 1) + 1) % 1;
  const scaled = wrapped * colors.length;
  const index = Math.floor(scaled);
  const next = (index + 1) % colors.length;
  const local = scaled - index;
  const eased = 0.5 - Math.cos(local * Math.PI) / 2;
  return mixHex(colors[index], colors[next], eased);
}

function rgba(hex: string, alpha: number) {
  const { r, g, b } = hexToRgb(hex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export function buildAnimatedTheme(theme: Theme, tick: number): Theme {
  if (!theme.animated) return theme;

  const palette = theme.cycleColors?.length ? theme.cycleColors : theme.particleColors;
  const phase = (tick % 1000) / 1000;
  const primary = colorAt(palette, phase);
  const accent = colorAt(palette, phase + 0.34);
  const warm = colorAt(palette, phase + 0.17);
  const cool = colorAt(palette, phase + 0.62);

  return {
    ...theme,
    primary,
    primaryDark: mixHex(primary, "#1f2937", 0.22),
    accent,
    bubbleUser: `linear-gradient(135deg, ${primary}, ${warm})`,
    bubbleBot: rgba(cool, 0.12),
    headerGradient: `linear-gradient(135deg, ${primary}, ${warm}, ${accent}, ${cool})`,
    particleColors: [primary, warm, accent, cool],
  };
}

export const usePetStore = defineStore("pet", () => {
  const state = ref<PetState>("idle");
  const happiness = ref(0.5);   // -1.0 ~ 1.0
  const energy = ref(80);        // 0 ~ 100
  const position = ref({ x: 60, y: 60 });
  // 拖拽状态统一走 PetState 的 "dragging"（后端正行为状态机里也有一份），
  // 这里不再单独维护一个永远不会被写入的布尔量。
  const isDragging = computed(() => state.value === "dragging");
  const skin = ref("default");
  const character = ref<PetCharacterId>("classic");
  const customPixelPetAsset = ref<CustomPixelPetAsset | null>(null);
  const fontColor = ref("");     // 用户自定义字体色，空=跟随主题
  const userAvatar = ref("");    // 用户头像 data URL / URL
  const themeTick = ref(0);
  let themeTimer: ReturnType<typeof setInterval> | null = null;

  // 当前主题
  const resolvedSkin = computed(() => resolveSkinId(skin.value));
  const theme = computed<Theme>(() => {
    return THEMES[resolvedSkin.value] || THEMES.default;
  });
  const visualTheme = computed<Theme>(() => buildAnimatedTheme(theme.value, themeTick.value));
  const isAnimatedSkin = computed(() => Boolean(theme.value.animated));

  // 表情映射
  const expression = computed(() => {
    if (state.value === "thinking") return "\u{1F914}";
    if (state.value === "happy") return "\u{1F60A}";
    if (state.value === "sleeping") return "\u{1F634}";
    if (state.value === "confused") return "\u{1F615}";
    if (state.value === "speaking") return "\u{1F4AC}";
    if (state.value === "waving") return "\u{1F44B}";
    if (state.value === "hungry") return "\u{1F924}";
    if (state.value === "stuffed") return "\u{1F60B}";
    if (state.value === "refusing") return "\u{1F616}";
    if (happiness.value > 0.3) return "\u{1F642}";
    if (happiness.value < -0.3) return "\u{1F61F}";
    return "\u{1F610}";
  });

  /**
   * 只改本窗口的展示状态。
   *
   * 注意：常驻桌宠窗口每秒都会用后端行为状态机的值覆盖 store，所以只调这里的状态
   * 活不过一秒。要让桌宠真的进入某个状态，请用各窗口自己的 syncPetState（会同时写后端）。
   */
  function setState(newState: PetState) {
    state.value = newState;
  }

  function updateMood(delta: { happiness?: number; energy?: number }) {
    if (delta.happiness !== undefined) {
      happiness.value = Math.max(-1, Math.min(1, happiness.value + delta.happiness));
    }
    if (delta.energy !== undefined) {
      energy.value = Math.max(0, Math.min(100, energy.value + delta.energy));
    }
  }

  function moveTo(x: number, y: number) {
    position.value = { x, y };
  }

  // 将主题写入 CSS 变量，供所有组件消费
  function applyCssVars() {
    if (typeof document === "undefined") return;

    const t = visualTheme.value;
    const root = document.documentElement;
    const primaryRgb = hexToRgb(t.primary);
    const accentRgb = hexToRgb(t.accent);

    root.style.setProperty("--pet-primary", t.primary);
    root.style.setProperty("--pet-primary-dark", t.primaryDark);
    root.style.setProperty("--pet-accent", t.accent);
    root.style.setProperty("--pet-primary-rgb", `${primaryRgb.r}, ${primaryRgb.g}, ${primaryRgb.b}`);
    root.style.setProperty("--pet-accent-rgb", `${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}`);
    root.style.setProperty("--pet-bg-glass", t.bgGlass);
    root.style.setProperty("--pet-bubble-user", t.bubbleUser);
    root.style.setProperty("--pet-bubble-bot", t.bubbleBot);
    root.style.setProperty("--pet-bubble-bot-text", t.bubbleBotText);
    root.style.setProperty("--pet-header-gradient", t.headerGradient);
    root.style.setProperty("--pet-font-color", fontColor.value || t.bubbleBotText);

    /*
     * 控制台配色也要跟着皮肤走。
     *
     * 之前 `--dash-accent-rgb` 是写死青瓷的，而 dashboard.css 里有 141 处用它，
     * 于是切皮肤只有按钮变了、侧栏/卡片/选中态纹丝不动——看起来就是「换皮肤没反应」。
     * 现在整套纸面色阶都从主题主色派生，切换时整个控制台一起换色调。
     */
    const tone = themeToneScale(t.primary, t.primaryDark);
    const accentScale = hexToRgb(t.primary);
    root.style.setProperty("--dash-accent", tone.accent);
    root.style.setProperty("--dash-accent-strong", tone.accentStrong);
    /*
     * 压在 accent 亮色块上的文字色。
     * accent 是 L≈0.66 的亮色，白字只有 3:1；用本主题的深墨色则有 7:1。
     * 这是"既亮又能读"的关键，不是把主色压暗。
     */
    root.style.setProperty("--dash-on-accent", tone.onAccent);
    // 小字用的深主色：亮主色当文字只有 2.7:1，读不清
    root.style.setProperty("--dash-accent-text", tone.accentText);
    root.style.setProperty("--dash-accent-rgb", `${accentScale.r}, ${accentScale.g}, ${accentScale.b}`);
    const strongRgb = hexToRgb(t.primaryDark);
    root.style.setProperty("--dash-accent-strong-rgb", `${strongRgb.r}, ${strongRgb.g}, ${strongRgb.b}`);

    root.style.setProperty("--paper-tint-50", tone.tint50);
    root.style.setProperty("--paper-tint-100", tone.tint100);
    root.style.setProperty("--paper-tint-150", tone.tint150);
    root.style.setProperty("--paper-tint-200", tone.tint200);
    root.style.setProperty("--paper-tint-300", tone.tint300);
    const tint50 = hexToRgb(tone.tint50);
    root.style.setProperty("--paper-tint-50-rgb", `${tint50.r}, ${tint50.g}, ${tint50.b}`);
    const tint150 = hexToRgb(tone.tint150);
    const tint200 = hexToRgb(tone.tint200);
    const tint300 = hexToRgb(tone.tint300);
    root.style.setProperty("--paper-tint-150-rgb", `${tint150.r}, ${tint150.g}, ${tint150.b}`);
    root.style.setProperty("--paper-tint-200-rgb", `${tint200.r}, ${tint200.g}, ${tint200.b}`);
    root.style.setProperty("--paper-tint-300-rgb", `${tint300.r}, ${tint300.g}, ${tint300.b}`);
  }

  function syncThemeTimer() {
    if (theme.value.animated && !themeTimer) {
      themeTimer = setInterval(() => {
        themeTick.value = (themeTick.value + 1) % 1000;
      }, 90);
      return;
    }

    if (!theme.value.animated && themeTimer) {
      clearInterval(themeTimer);
      themeTimer = null;
      themeTick.value = 0;
    }
  }

  watch(skin, syncThemeTimer, { immediate: true });

  // 监听主题/字体色变化，自动刷新 CSS 变量
  watch([visualTheme, fontColor], () => applyCssVars(), { immediate: true });

  return {
    state, happiness, energy, position, isDragging,
    skin, character, customPixelPetAsset, fontColor, userAvatar, resolvedSkin, theme, visualTheme, isAnimatedSkin, expression,
    setState, updateMood, moveTo, applyCssVars,
  };
});
