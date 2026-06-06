import { defineStore } from "pinia";
import { ref, computed, watch } from "vue";
import type { CustomPixelPetAsset } from "../services/customPixelPet";
import type { PetCharacterId } from "../services/petCharacters";

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

export const THEMES: Record<string, Theme> = {
  default: {
    id: "default",
    name: "樱花红",
    primary: "#ff6b6b",
    primaryDark: "#e55a5a",
    accent: "#ffa07a",
    bgGlass: "rgba(255, 255, 255, 0.92)",
    bubbleUser: "linear-gradient(135deg, #ff6b6b, #ff8e8e)",
    bubbleBot: "rgba(255, 107, 107, 0.08)",
    bubbleBotText: "#4a4a4a",
    headerGradient: "linear-gradient(135deg, #ff6b6b, #ff8e53)",
    particleColors: ["#ff6b6b", "#ff8e8e", "#ffa07a", "#ffb6c1"],
  },
  ocean: {
    id: "ocean",
    name: "深海蓝",
    primary: "#4ecdc4",
    primaryDark: "#3db8b0",
    accent: "#45b7d1",
    bgGlass: "rgba(240, 253, 252, 0.92)",
    bubbleUser: "linear-gradient(135deg, #4ecdc4, #44b8c4)",
    bubbleBot: "rgba(78, 205, 196, 0.08)",
    bubbleBotText: "#3a5a5c",
    headerGradient: "linear-gradient(135deg, #4ecdc4, #45b7d1)",
    particleColors: ["#4ecdc4", "#45b7d1", "#96e6df", "#a7f3d0"],
  },
  galaxy: {
    id: "galaxy",
    name: "星空紫",
    primary: "#a855f7",
    primaryDark: "#9333ea",
    accent: "#c084fc",
    bgGlass: "rgba(250, 245, 255, 0.92)",
    bubbleUser: "linear-gradient(135deg, #a855f7, #8b5cf6)",
    bubbleBot: "rgba(168, 85, 247, 0.08)",
    bubbleBotText: "#4a3b5c",
    headerGradient: "linear-gradient(135deg, #a855f7, #ec4899)",
    particleColors: ["#a855f7", "#c084fc", "#e879f9", "#f0abfc"],
  },
  forest: {
    id: "forest",
    name: "森林绿",
    primary: "#22c55e",
    primaryDark: "#16a34a",
    accent: "#4ade80",
    bgGlass: "rgba(240, 255, 244, 0.92)",
    bubbleUser: "linear-gradient(135deg, #22c55e, #16a34a)",
    bubbleBot: "rgba(34, 197, 94, 0.08)",
    bubbleBotText: "#2d5a3e",
    headerGradient: "linear-gradient(135deg, #22c55e, #06b6d4)",
    particleColors: ["#22c55e", "#4ade80", "#86efac", "#a7f3d0"],
  },
  sunset: {
    id: "sunset",
    name: "日落橙",
    primary: "#f97316",
    primaryDark: "#ea580c",
    accent: "#fb923c",
    bgGlass: "rgba(255, 251, 240, 0.92)",
    bubbleUser: "linear-gradient(135deg, #f97316, #ef4444)",
    bubbleBot: "rgba(249, 115, 22, 0.08)",
    bubbleBotText: "#5c3a1e",
    headerGradient: "linear-gradient(135deg, #f97316, #ef4444)",
    particleColors: ["#f97316", "#fb923c", "#fbbf24", "#f87171"],
  },
  sakura: {
    id: "sakura",
    name: "樱花粉",
    primary: "#ec4899",
    primaryDark: "#db2777",
    accent: "#f472b6",
    bgGlass: "rgba(255, 241, 248, 0.92)",
    bubbleUser: "linear-gradient(135deg, #ec4899, #f472b6)",
    bubbleBot: "rgba(236, 72, 153, 0.08)",
    bubbleBotText: "#5c2d4a",
    headerGradient: "linear-gradient(135deg, #ec4899, #a855f7)",
    particleColors: ["#ec4899", "#f472b6", "#f9a8d4", "#fbcfe8"],
  },
  aurora: {
    id: "aurora",
    name: "流光变色",
    primary: "#ff6b9d",
    primaryDark: "#d9467a",
    accent: "#38bdf8",
    bgGlass: "rgba(248, 252, 255, 0.9)",
    bubbleUser: "linear-gradient(135deg, #ff6b9d, #38bdf8)",
    bubbleBot: "rgba(56, 189, 248, 0.1)",
    bubbleBotText: "#334155",
    headerGradient: "linear-gradient(135deg, #ff6b9d, #f59e0b, #38bdf8, #8b5cf6)",
    particleColors: ["#ff6b9d", "#f59e0b", "#45d483", "#38bdf8", "#8b5cf6"],
    animated: true,
    cycleColors: ["#ff6b9d", "#f59e0b", "#45d483", "#38bdf8", "#8b5cf6"],
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
  if (colors.length === 0) return "#ff6b6b";
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
  const isDragging = ref(false);
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
