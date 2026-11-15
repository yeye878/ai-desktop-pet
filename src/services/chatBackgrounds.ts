/**
 * 对话背景：单一事实来源。
 *
 * 背景预设以前散落在 Dashboard / ChatBubble / Settings 三个组件里各自硬编码，
 * 于是「个性外观」里换了背景，对话界面看不出来（Dashboard 根本没有渲染 BgCanvas），
 * 三个窗口的图标/名称也各写一份。这里把它们收敛成一份清单：
 * - 选择存 localStorage（沿用旧 key，老用户的设置不会丢）
 * - 卡片缩略图用同一份 swatch 渐变，保证「选中的样子」和「看到的缩略图」一致
 * - 新增预设只要在这里加一项，三处 UI 与 BgCanvas 会自动跟上
 */

export type ChatBackgroundId =
  | "none"
  | "aurora"
  | "ink"
  | "rain"
  | "sakura"
  | "minimal"
  | "spring-peach"
  | "summer-lotus"
  | "autumn-chrys"
  | "winter-plum"
  | "custom";

export interface ChatBackgroundPreset {
  id: ChatBackgroundId;
  /** 卡片上的中文名 */
  name: string;
  /** 一行说明，解释这层背景「长什么样、适合什么场景」 */
  description: string;
  /** 卡片角标图标 */
  icon: string;
  /** 卡片/弹层里的小缩略图（纯 CSS，和 BgCanvas 的调色板保持一致） */
  swatch: string;
  /** 背景层整体不透明度，深色背景要更实一点才看得出效果 */
  opacity: number;
  /**
   * 底色基调。整个控制台是米白纸面（约 #f0f2ef），
   * 深色背景会在面板边缘形成 200+ 的色阶断层，看起来像贴了一块黑方块。
   * 因此所有预设都收敛到「纸面友好」的浅色基调，靠色调而不是亮度做区分。
   */
  tone: "paper";
  /** 该预设的主要色调，用于切换提示与卡片描边取色 */
  accent: string;
}

export const CHAT_BG_STORAGE_KEY = "ai-desktop-pet.chat-bg";
export const CHAT_BG_CUSTOM_STORAGE_KEY = "ai-desktop-pet.chat-bg-custom";
/** 背景浓度：淡雅 / 适中 / 明显，三个窗口共用 */
export const CHAT_BG_INTENSITY_STORAGE_KEY = "ai-desktop-pet.chat-bg-intensity";
/** 同一窗口内的即时同步（跨 webview 由 storage 事件负责） */
export const CHAT_BG_CHANGED_EVENT = "chat-background-changed";

export interface ChatBgIntensityLevel {
  id: string;
  name: string;
  description: string;
  /** 传给 BgCanvas 的 0~1 强度系数 */
  value: number;
}

export const CHAT_BG_INTENSITIES: ChatBgIntensityLevel[] = [
  { id: "soft", name: "淡雅", description: "几乎只是纸面上的一层光晕，最不打扰阅读", value: 0.32 },
  { id: "medium", name: "适中", description: "能看出纹理与流动，但仍然安静", value: 0.72 },
  { id: "rich", name: "明显", description: "纹理与动效都更清楚，适合想有点氛围的时候", value: 1.15 },
];

export const DEFAULT_CHAT_BG_INTENSITY = 0.72;

export function normalizeChatBgIntensity(value: unknown): number {
  // 空值/空串要回落到默认档，不能走 Number(null)=0 那条路被夹成下限
  if (value === null || value === undefined || value === "") return DEFAULT_CHAT_BG_INTENSITY;
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric)) return DEFAULT_CHAT_BG_INTENSITY;
  return Math.min(1.2, Math.max(0.15, numeric));
}

export function readChatBgIntensity(): number {
  try {
    return normalizeChatBgIntensity(localStorage.getItem(CHAT_BG_INTENSITY_STORAGE_KEY));
  } catch {
    return DEFAULT_CHAT_BG_INTENSITY;
  }
}

export function applyChatBgIntensity(value: number) {
  const normalized = normalizeChatBgIntensity(value);
  try {
    localStorage.setItem(CHAT_BG_INTENSITY_STORAGE_KEY, String(normalized));
  } catch {
    /* 隐私模式下写入会抛错，界面仍应即时生效 */
  }
  window.dispatchEvent(
    new CustomEvent(CHAT_BG_CHANGED_EVENT, {
      detail: { id: readChatBackground(), intensity: normalized },
    }),
  );
}

export const CHAT_BACKGROUNDS: ChatBackgroundPreset[] = [
  {
    id: "none",
    name: "无",
    description: "保留原本的纸张质感，不叠加任何背景",
    icon: "bg-none",
    swatch:
      "radial-gradient(75% 70% at 26% 22%, #ffffff, transparent 58%)," +
      "radial-gradient(60% 55% at 76% 72%, #eceef0, transparent 62%)," +
      "radial-gradient(50% 45% at 40% 90%, #f4f3f1, transparent 60%)," +
      "linear-gradient(165deg, #fbfaf8, #f2f1ef)",
    opacity: 1,
    tone: "paper",
    accent: "#e8e2d6",
  },
  {
    id: "aurora",
    name: "极光",
    description: "真实摄影：湖面之上的极光帘幕与雪山剪影",
    icon: "bg-aurora",
    swatch:
      "radial-gradient(70% 65% at 30% 22%, rgba(96,142,132,0.9), transparent 58%)," +
      "radial-gradient(55% 50% at 78% 34%, rgba(52,74,108,0.85), transparent 62%)," +
      "radial-gradient(65% 55% at 46% 88%, rgba(30,44,64,0.9), transparent 60%)," +
      "linear-gradient(168deg, #1d2a3a, #131d2c 58%, #0d1520)",
    opacity: 1,
    tone: "paper",
    accent: "#8ed8cc",
  },
  {
    id: "ink",
    name: "水墨",
    description: "真实摄影：云雾中的山脊层层退远，像一幅淡墨山水",
    icon: "bg-ink",
    swatch:
      "radial-gradient(80% 70% at 22% 16%, #e4e8ea, transparent 60%)," +
      "radial-gradient(60% 55% at 72% 40%, #b5bec2, transparent 62%)," +
      "radial-gradient(70% 60% at 38% 88%, #6d767d, transparent 58%)," +
      "linear-gradient(170deg, #d3d9dd, #8b949b 55%, #5c656c)",
    opacity: 1,
    tone: "paper",
    accent: "#b09a7e",
  },
  {
    id: "rain",
    name: "细雨",
    description: "真实摄影：隔着雨珠看园林，水汽把绿意揉成一片",
    icon: "bg-rain",
    swatch:
      "radial-gradient(65% 60% at 32% 26%, rgba(72,96,112,0.95), transparent 60%)," +
      "radial-gradient(50% 45% at 76% 30%, rgba(138,74,72,0.75), transparent 62%)," +
      "radial-gradient(60% 52% at 58% 82%, rgba(40,52,62,0.9), transparent 60%)," +
      "linear-gradient(170deg, #202b34, #141d25 58%, #0c141b)",
    opacity: 1,
    tone: "paper",
    accent: "#93b4c2",
  },
  {
    id: "sakura",
    name: "樱花",
    description: "真实摄影：近黑背景前的樱花枝，浅粉花瓣与暗色枝干",
    icon: "bg-sakura",
    swatch:
      "radial-gradient(70% 62% at 32% 24%, rgba(196,170,182,0.9), transparent 58%)," +
      "radial-gradient(52% 48% at 72% 44%, rgba(140,116,128,0.75), transparent 62%)," +
      "radial-gradient(62% 54% at 44% 86%, rgba(54,48,52,0.92), transparent 60%)," +
      "linear-gradient(170deg, #332e31, #221e21 58%, #161315)",
    opacity: 1,
    tone: "paper",
    accent: "#c9a8b4",
  },
  {
    id: "spring-peach",
    name: "春·桃",
    description: "真实摄影：春日桃花，淡粉花簇与深色枝条",
    icon: "bg-peach",
    swatch: "linear-gradient(165deg, #6b545a, #3d2f33)",
    opacity: 1,
    tone: "paper",
    accent: "#d99aa8",
  },
  {
    id: "summer-lotus",
    name: "夏·荷",
    description: "真实摄影：夏日荷塘，睡莲浮在静水上",
    icon: "bg-summer",
    swatch: "linear-gradient(165deg, #2f4438, #1a2a22)",
    opacity: 1,
    tone: "paper",
    accent: "#8fb59a",
  },
  {
    id: "autumn-chrys",
    name: "秋·菊",
    description: "真实摄影：秋日菊影，暖粉花簇虚在暗底上",
    icon: "bg-autumn",
    swatch: "linear-gradient(165deg, #4a3a34, #2a201c)",
    opacity: 1,
    tone: "paper",
    accent: "#d59a86",
  },
  {
    id: "winter-plum",
    name: "冬·梅",
    description: "真实摄影：霜雪压枝，冷蓝调里的一点朱红",
    icon: "bg-winter",
    swatch: "linear-gradient(165deg, #3a3a44, #202028)",
    opacity: 1,
    tone: "paper",
    accent: "#b6b0c4",
  },
  {
    id: "minimal",
    name: "简洁",
    description: "会跟随鼠标亮起的点阵星图，克制又灵巧",
    icon: "bg-minimal",
    swatch:
      "radial-gradient(85% 75% at 24% 18%, #fbfcfe, transparent 62%)," +
      "radial-gradient(55% 50% at 74% 46%, #e5eaf3, transparent 60%)," +
      "radial-gradient(60% 50% at 46% 86%, #edf1f7, transparent 58%)," +
      "linear-gradient(168deg, #f2f5fb, #e2e8f1)",
    opacity: 0.9,
    tone: "paper",
    accent: "#b7bfd4",
  },
  {
    id: "custom",
    name: "自定义",
    description: "用你自己的图片做背景，会柔和地铺满整个对话区",
    icon: "📷",
    swatch:
      "repeating-linear-gradient(45deg, rgba(148,163,184,0.22) 0 8px, rgba(203,213,225,0.18) 8px 16px)",
    opacity: 1,
    tone: "paper",
    accent: "#c7cbd6",
  },
];

/**
 * 照片类预设的缩略图来源。
 *
 * 背景本身就是真实摄影（见 bgPhotos.ts），所以选择器里的 tile 直接用
 * 同一张照片裁一小块 —— 这是最真实的"缩略图"，比用 CSS 渐变模拟像照片得多。
 * 返回 null 表示该预设没有照片（无 / 简洁 / 自定义）。
 */
export function backgroundThumbPhoto(id: string): string | null {
  switch (id) {
    case "aurora":
    case "ink":
    case "rain":
    case "sakura":
    case "spring-peach":
    case "summer-lotus":
    case "autumn-chrys":
    case "winter-plum":
      return id;
    default:
      return null;
  }
}

export const CHAT_BACKGROUND_MAP: Record<string, ChatBackgroundPreset> =
  Object.fromEntries(CHAT_BACKGROUNDS.map((preset) => [preset.id, preset]));

/** 在「个性外观」里展示的预设（自定义单独占一格，不做成普通预设） */
export const SELECTABLE_CHAT_BACKGROUNDS = CHAT_BACKGROUNDS.filter(
  (preset) => preset.id !== "custom",
);

export function isChatBackgroundId(value: string | null | undefined): value is ChatBackgroundId {
  return Boolean(value && value in CHAT_BACKGROUND_MAP);
}

/**
 * 旧 id 的兼容映射。
 *
 * spring-willow 已改名为 spring-peach；用户 localStorage 里可能还存着旧值，
 * 直接回落到 none 会让他们"背景突然没了"，所以映射到新 id。
 */
const LEGACY_BACKGROUND_IDS: Record<string, string> = {
  "spring-willow": "spring-peach",
};

export function normalizeChatBackgroundId(value: string | null | undefined): ChatBackgroundId {
  // 先查旧 id 映射（spring-willow -> spring-peach），再校验合法性
  const mapped = value ? LEGACY_BACKGROUND_IDS[value] ?? value : value;
  return isChatBackgroundId(mapped) ? mapped : "none";
}

export function chatBackgroundPreset(value: string | null | undefined): ChatBackgroundPreset {
  return CHAT_BACKGROUND_MAP[normalizeChatBackgroundId(value)];
}

export function readChatBackground(): ChatBackgroundId {
  try {
    return normalizeChatBackgroundId(localStorage.getItem(CHAT_BG_STORAGE_KEY));
  } catch {
    return "none";
  }
}

export function readCustomChatBackground(): string {
  try {
    return localStorage.getItem(CHAT_BG_CUSTOM_STORAGE_KEY) || "";
  } catch {
    return "";
  }
}

/**
 * 写入背景选择并广播。
 * 同一窗口靠自定义事件、跨 webview 靠 localStorage 的 storage 事件（ChatBubble 已在监听）。
 */
export function applyChatBackground(id: ChatBackgroundId) {
  try {
    localStorage.setItem(CHAT_BG_STORAGE_KEY, id);
  } catch {
    /* 隐私模式下写入会抛错，界面仍应即时生效 */
  }
  window.dispatchEvent(
    new CustomEvent(CHAT_BG_CHANGED_EVENT, {
      detail: { id, intensity: readChatBgIntensity() },
    }),
  );
}

export function applyCustomChatBackground(dataUrl: string) {
  try {
    localStorage.setItem(CHAT_BG_CUSTOM_STORAGE_KEY, dataUrl);
  } catch {
    /* 图片过大时 localStorage 会超限，交给调用方提示 */
  }
  applyChatBackground("custom");
}

export function clearCustomChatBackground() {
  try {
    localStorage.removeItem(CHAT_BG_CUSTOM_STORAGE_KEY);
  } catch {
    /* ignore */
  }
  applyChatBackground("none");
}
