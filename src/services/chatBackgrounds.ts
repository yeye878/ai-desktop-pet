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
  | "cute"
  | "scifi"
  | "minimal"
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
    icon: "∅",
    swatch:
      "linear-gradient(150deg, rgba(255,255,255,0.92), rgba(244,240,232,0.88))",
    opacity: 1,
    tone: "paper",
    accent: "#e8e2d6",
  },
  {
    id: "aurora",
    name: "极光",
    description: "青绿与淡紫的光晕在纸面上缓缓流动，像清晨窗外的天光",
    icon: "☾",
    swatch:
      "radial-gradient(110% 80% at 16% 10%, rgba(148,236,224,0.55), transparent 62%)," +
      "radial-gradient(100% 75% at 84% 18%, rgba(186,182,255,0.42), transparent 64%)," +
      "radial-gradient(120% 90% at 52% 100%, rgba(178,240,226,0.4), transparent 70%)," +
      "linear-gradient(165deg, #f4fbf9, #f2f5fb 55%, #f7f4fa)",
    opacity: 0.95,
    tone: "paper",
    accent: "#8ed8cc",
  },
  {
    id: "ink",
    name: "水墨",
    description: "米色宣纸上的淡墨远山与流动雾霭，最安静的一个",
    icon: "墨",
    swatch:
      "radial-gradient(90% 70% at 22% 88%, rgba(120,128,140,0.22), transparent 64%)," +
      "radial-gradient(80% 60% at 74% 96%, rgba(140,148,160,0.16), transparent 66%)," +
      "radial-gradient(120% 90% at 60% 8%, rgba(255,255,255,0.75), transparent 70%)," +
      "linear-gradient(168deg, #fbf8f1, #f6f2e9 52%, #f1ece1)",
    opacity: 0.95,
    tone: "paper",
    accent: "#b09a7e",
  },
  {
    id: "cute",
    name: "可爱",
    description: "樱花飘落与糖果色气泡，点击还会撒出爱心",
    icon: "🐾",
    swatch:
      "radial-gradient(100% 80% at 20% 20%, rgba(255,196,208,0.6), transparent 62%)," +
      "radial-gradient(90% 70% at 80% 30%, rgba(255,222,238,0.6), transparent 60%)," +
      "radial-gradient(120% 90% at 50% 100%, rgba(206,182,255,0.28), transparent 66%)," +
      "linear-gradient(165deg, #fff7fa, #fff0f5 55%, #faf3ff)",
    opacity: 0.95,
    tone: "paper",
    accent: "#f7a8bf",
  },
  {
    id: "scifi",
    name: "科幻",
    description: "淡蓝星图与六边形网格，会跟随桌宠状态呼吸",
    icon: "🚀",
    swatch:
      "radial-gradient(90% 70% at 26% 26%, rgba(140,205,255,0.45), transparent 64%)," +
      "radial-gradient(85% 65% at 76% 72%, rgba(178,172,255,0.38), transparent 66%)," +
      "repeating-linear-gradient(0deg, rgba(120,170,220,0.10) 0 1px, transparent 1px 9px)," +
      "linear-gradient(160deg, #f2f8fd, #eef3fb 55%, #f3f1fc)",
    opacity: 0.95,
    tone: "paper",
    accent: "#79b1b3",
  },
  {
    id: "minimal",
    name: "简洁",
    description: "会跟随鼠标亮起的点阵星图，克制又灵巧",
    icon: "○",
    swatch:
      "radial-gradient(80% 60% at 30% 30%, rgba(160,170,255,0.20), transparent 64%)," +
      "repeating-linear-gradient(90deg, rgba(120,132,150,0.13) 0 1px, transparent 1px 12px)," +
      "repeating-linear-gradient(0deg, rgba(120,132,150,0.13) 0 1px, transparent 1px 12px)," +
      "linear-gradient(160deg, #fafbfd, #f3f6fa)",
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

export const CHAT_BACKGROUND_MAP: Record<string, ChatBackgroundPreset> =
  Object.fromEntries(CHAT_BACKGROUNDS.map((preset) => [preset.id, preset]));

/** 在「个性外观」里展示的预设（自定义单独占一格，不做成普通预设） */
export const SELECTABLE_CHAT_BACKGROUNDS = CHAT_BACKGROUNDS.filter(
  (preset) => preset.id !== "custom",
);

export function isChatBackgroundId(value: string | null | undefined): value is ChatBackgroundId {
  return Boolean(value && value in CHAT_BACKGROUND_MAP);
}

export function normalizeChatBackgroundId(value: string | null | undefined): ChatBackgroundId {
  return isChatBackgroundId(value) ? value : "none";
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
