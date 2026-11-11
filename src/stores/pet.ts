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
    primary: "#609497",
    primaryDark: "#4a7577",
    accent: "#79b1b3",
    bgGlass: "rgba(252, 250, 245, 0.92)",
    bubbleUser: "linear-gradient(135deg, #609497, #79b1b3)",
    bubbleBot: "rgba(96, 148, 151, 0.08)",
    bubbleBotText: "#34383f",
    headerGradient: "linear-gradient(135deg, #609497, #4a7577)",
    particleColors: ["#609497", "#79b1b3", "#97c8ca", "#b6dcdd"],
  },
  ocean: {
    id: "ocean",
    name: "雨过天青",
    primary: "#4a7577",
    primaryDark: "#365658",
    accent: "#609497",
    bgGlass: "rgba(243, 251, 251, 0.92)",
    bubbleUser: "linear-gradient(135deg, #4a7577, #609497)",
    bubbleBot: "rgba(74, 117, 119, 0.08)",
    bubbleBotText: "#2b3238",
    headerGradient: "linear-gradient(135deg, #4a7577, #97c8ca)",
    particleColors: ["#4a7577", "#609497", "#97c8ca", "#d0ebec"],
  },
  galaxy: {
    id: "galaxy",
    name: "远山黛",
    primary: "#5b6480",
    primaryDark: "#464e66",
    accent: "#7b849f",
    bgGlass: "rgba(248, 249, 252, 0.92)",
    bubbleUser: "linear-gradient(135deg, #5b6480, #7b849f)",
    bubbleBot: "rgba(91, 100, 128, 0.08)",
    bubbleBotText: "#2f333d",
    headerGradient: "linear-gradient(135deg, #5b6480, #9aa3ba)",
    particleColors: ["#5b6480", "#7b849f", "#9aa3ba", "#c6ccda"],
  },
  forest: {
    id: "forest",
    name: "鼠尾草",
    primary: "#6f8f74",
    primaryDark: "#57735c",
    accent: "#8aa98f",
    bgGlass: "rgba(247, 250, 246, 0.92)",
    bubbleUser: "linear-gradient(135deg, #6f8f74, #8aa98f)",
    bubbleBot: "rgba(111, 143, 116, 0.08)",
    bubbleBotText: "#2f3830",
    headerGradient: "linear-gradient(135deg, #6f8f74, #9dbaa1)",
    particleColors: ["#6f8f74", "#8aa98f", "#adc6b1", "#cfe0d2"],
  },
  sunset: {
    id: "sunset",
    name: "陶土",
    primary: "#bb8166",
    primaryDark: "#9f684f",
    accent: "#cf9e89",
    bgGlass: "rgba(252, 249, 244, 0.92)",
    bubbleUser: "linear-gradient(135deg, #bb8166, #cf9e89)",
    bubbleBot: "rgba(187, 129, 102, 0.08)",
    bubbleBotText: "#3b332e",
    headerGradient: "linear-gradient(135deg, #bb8166, #9f684f)",
    particleColors: ["#bb8166", "#cf9e89", "#e3beae", "#f2dad0"],
  },
  sakura: {
    id: "sakura",
    name: "藕荷",
    primary: "#a97f92",
    primaryDark: "#8e6879",
    accent: "#c29cad",
    bgGlass: "rgba(252, 248, 249, 0.92)",
    bubbleUser: "linear-gradient(135deg, #a97f92, #c29cad)",
    bubbleBot: "rgba(169, 127, 146, 0.08)",
    bubbleBotText: "#3a3237",
    headerGradient: "linear-gradient(135deg, #a97f92, #c29cad)",
    particleColors: ["#a97f92", "#c29cad", "#d9bcc8", "#eddfe5"],
  },
  moon: {
    id: "moon",
    name: "月白",
    primary: "#7d8aa3",
    primaryDark: "#65708a",
    accent: "#97a4bc",
    bgGlass: "rgba(248, 250, 253, 0.92)",
    bubbleUser: "linear-gradient(135deg, #7d8aa3, #97a4bc)",
    bubbleBot: "rgba(125, 138, 163, 0.08)",
    bubbleBotText: "#30343b",
    headerGradient: "linear-gradient(135deg, #7d8aa3, #b3bccd)",
    particleColors: ["#7d8aa3", "#97a4bc", "#b3bccd", "#d8dde6"],
  },
  tea: {
    id: "tea",
    name: "茶褐",
    primary: "#8f7a63",
    primaryDark: "#75624e",
    accent: "#a8937b",
    bgGlass: "rgba(251, 249, 245, 0.92)",
    bubbleUser: "linear-gradient(135deg, #8f7a63, #a8937b)",
    bubbleBot: "rgba(143, 122, 99, 0.08)",
    bubbleBotText: "#37322c",
    headerGradient: "linear-gradient(135deg, #8f7a63, #bfae99)",
    particleColors: ["#8f7a63", "#a8937b", "#c6b7a3", "#e4dbcd"],
  },
  indigo: {
    id: "indigo",
    name: "靛青",
    primary: "#5a6b86",
    primaryDark: "#455569",
    accent: "#76879f",
    bgGlass: "rgba(247, 249, 252, 0.92)",
    bubbleUser: "linear-gradient(135deg, #5a6b86, #76879f)",
    bubbleBot: "rgba(90, 107, 134, 0.08)",
    bubbleBotText: "#2c3138",
    headerGradient: "linear-gradient(135deg, #5a6b86, #93a3b8)",
    particleColors: ["#5a6b86", "#76879f", "#9dabbd", "#c9d3df"],
  },
  moss: {
    id: "moss",
    name: "苔痕",
    primary: "#79875f",
    primaryDark: "#606d4a",
    accent: "#93a077",
    bgGlass: "rgba(249, 250, 245, 0.92)",
    bubbleUser: "linear-gradient(135deg, #79875f, #93a077)",
    bubbleBot: "rgba(121, 135, 95, 0.08)",
    bubbleBotText: "#33362c",
    headerGradient: "linear-gradient(135deg, #79875f, #aab694)",
    particleColors: ["#79875f", "#93a077", "#b3bf9d", "#d9dfcc"],
  },
  coral: {
    id: "coral",
    name: "柚粉",
    primary: "#b58485",
    primaryDark: "#976a6b",
    accent: "#c9a0a0",
    bgGlass: "rgba(253, 249, 249, 0.92)",
    bubbleUser: "linear-gradient(135deg, #b58485, #c9a0a0)",
    bubbleBot: "rgba(181, 132, 133, 0.08)",
    bubbleBotText: "#3a3131",
    headerGradient: "linear-gradient(135deg, #b58485, #d9bcbc)",
    particleColors: ["#b58485", "#c9a0a0", "#dcc0c0", "#efe0e0"],
  },
  aurora: {
    id: "aurora",
    name: "流光",
    primary: "#7e93a8",
    primaryDark: "#63788c",
    accent: "#8aa6b8",
    bgGlass: "rgba(249, 250, 251, 0.9)",
    bubbleUser: "linear-gradient(135deg, #7e93a8, #8aa6b8)",
    bubbleBot: "rgba(126, 147, 168, 0.1)",
    bubbleBotText: "#2f353b",
    headerGradient: "linear-gradient(135deg, #7e93a8, #a8b8a8, #c2b6a4, #8aa6b8)",
    particleColors: ["#7e93a8", "#a8b8a8", "#c2b6a4", "#97c8ca", "#b6b0c2"],
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
    root.style.setProperty("--dash-accent-rgb", `${accentScale.r}, ${accentScale.g}, ${accentScale.b}`);
    const strongRgb = hexToRgb(t.primaryDark);
    root.style.setProperty("--dash-accent-strong-rgb", `${strongRgb.r}, ${strongRgb.g}, ${strongRgb.b}`);

    root.style.setProperty("--paper-tint-50", tone.tint50);
    root.style.setProperty("--paper-tint-100", tone.tint100);
    root.style.setProperty("--paper-tint-150", tone.tint150);
    root.style.setProperty("--paper-tint-200", tone.tint200);
    root.style.setProperty("--paper-tint-300", tone.tint300);
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
