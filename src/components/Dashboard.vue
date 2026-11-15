<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick, reactive, type Component } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import CustomPixelPetWorkshop from "./CustomPixelPetWorkshop.vue";
import CodexConnection from "./CodexConnection.vue";
import DshConnection from "./DshConnection.vue";
import AgentPermissionControl from "./AgentPermissionControl.vue";
import AgentActivity from "./AgentActivity.vue";
import AgentModelPicker from "./AgentModelPicker.vue";
import SessionPermissionPicker from "./SessionPermissionPicker.vue";
import AgentPicker from "./AgentPicker.vue";
import MetaPicker from "./MetaPicker.vue";
import EffortSlider from "./EffortSlider.vue";
import SessionPane from "./SessionPane.vue";
import AttachmentTray from "./AttachmentTray.vue";
import { useAttachmentsStore } from "../stores/attachments";
import { attachmentDropTarget, buildMessageWithFiles, getFileIcon } from "../services/attachments";
import UserQuestion from "./UserQuestion.vue";
import { createInteractionSubmission, subscribePendingInteraction, type AskUserPayload } from "../services/pendingInteractions";
import { isNearBottom, legacyActivity, toolStatusLabel, createAgentActivity, type AgentActivityItem } from "../services/agentActivity";
import { saveActivityPresentation } from "../services/agentActivityCache";
import {
  effortLabel,
  fetchAgentModelCatalog,
  modelOptionFor,
  providerLabel,
  type AgentModelCatalog,
} from "../services/agentModels";
import { ArrowDown, House, MessageCircle, Brain, Palette, Bot, Mic, Settings as SettingsIcon, Info, PawPrint, Send, Plus, Power, Square as LayoutSingle, Columns2, Columns3, Grid2x2, PanelLeft, Paperclip } from "@lucide/vue";
import AppIcon from "./AppIcon.vue";
import CollaborationBar from "./CollaborationBar.vue";
import type { CollaborationReply, CollaborationState } from "../services/collaboration";
import PetCanvas from "./PetCanvas.vue";
import PetStage from "./PetStage.vue";
import { usePetStore, THEMES, FONT_COLORS, resolveSkinId, type PetState } from "../stores/pet";
import { useSkillsStore, type Skill } from "../stores/skills";
import { useChatStore, messageAgentFromPayload, type AiFinishedPayload, type Message, type MessageAgent, type QuotedMessage, type ToolEventItem } from "../stores/chat";
import { useAgentsStore, type Agent } from "../stores/agents";
import { useSessionsStore, SLOT_DEFAULT_AGENT, AGENT_SLOTS, type ChatSession, type ChatSlot } from "../stores/sessions";
import { useSessionRuntimeStore, type SessionTurn } from "../stores/sessionRuntime";
import {
  WORKSPACE_LAYOUTS,
  layoutPaneCount,
  readWorkspaceLayout,
  readWorkspacePanes,
  saveWorkspaceLayout,
  saveWorkspacePanes,
  type WorkspaceLayout,
} from "../services/workspace";
import {
  stripQuoteMarkers,
  truncateQuoteContent,
} from "../services/quoteParser";
import {
  SLASH_COMMANDS,
  slashCommandMatches,
  slashQueryFromInput,
  type SlashCommand,
} from "../services/slashCommands";
import {
  extractMentionNames,
  insertMentionAt,
  mentionQueryFromInput,
} from "../services/mentions";
import { markdownToPlainText, renderMessageSegments, type MessageRenderSegment } from "../services/markdown";
import {
  PET_CHARACTERS,
  PET_CHARACTER_SETTING_KEY,
  resolvePetCharacterId,
  type PetCharacterId,
} from "../services/petCharacters";
import {
  getActiveCustomPixelPetAsset,
  notifyPetAppearanceChanged,
  setActiveCustomPixelPetAsset,
} from "../services/customPixelPetAssets";
import {
  DEFAULT_VOICE_SETTINGS,
  getAvailableVoices,
  parseVoiceSettings,
  serializeVoiceSettings,
  type VoiceSettings,
} from "../services/voice";
import {
  listEdgeVoices,
  TtsPlayer,
  DEFAULT_TTS_SETTINGS,
  type TtsSettings,
  type TtsVoice,
} from "../services/tts";
import { ensurePlaybackSubscription, isSpeaking, speakingText } from "../services/ttsPlaybackState";
import {
  DEFAULT_WEATHER_CONFIG,
  formatTemperature,
  formatWeatherDisplay,
  normalizeWeatherInfo,
  type WeatherConfig,
  type WeatherInfo,
  type WeatherUpdateEvent,
} from "../services/weather";
import "../assets/dashboard.css";

const emit = defineEmits<{ openPet: []; closePet: [] }>();
const pet = usePetStore();
const chat = useChatStore();
const currentWindow = getCurrentWindow();

/**
 * 控制台里改桌宠状态要写两处：本地 store 只影响本窗口的预览，
 * 真正常驻的桌宠窗口每秒 tick 都会从后端状态机回读，只写本地活不过一秒。
 */
function syncPetState(state: PetState) {
  pet.setState(state);
  void Promise.resolve(invoke("set_pet_state", { newState: state })).catch(() => {});
}

// === 导航 ===
type NavPage = "home" | "chat" | "memory" | "agents" | "appearance" | "voice" | "system" | "about";
const activePage = ref<NavPage>(import.meta.env.DEV && new URLSearchParams(window.location.search).get("activity") === "demo" ? "chat" : "home");
const isPetActive = ref(false);
const skillsStore = useSkillsStore();
const agentsStore = useAgentsStore();
const sessions = useSessionsStore();
const sessionRuntime = useSessionRuntimeStore();

// === 多会话 / 三个 agent 独立对话窗口 ===
type AgentWindowSlot = Exclude<ChatSlot, "main">;
const SLOT_WINDOW_LABELS: Record<AgentWindowSlot, string> = {
  claude: "Claude Code",
  codex: "Codex",
  dsh: "DeepSeek Harness",
};
/** 侧边栏子项用固定图标区分三个窗口；窗口 header 优先用 agent 自己的头像。 */
const SLOT_WINDOW_ICONS: Record<AgentWindowSlot, string> = { claude: "claude", codex: "codex", dsh: "dsh" };
const CHAT_SLOT_STORAGE_KEY = "dash.chat-slot";
const AGENT_WINDOW_PREF_PREFIX = "dash.agent-window.";
const AGENT_SESSION_STORAGE_PREFIX = "dash.agent-session.";

function readStoredValue(key: string): string {
  try { return localStorage.getItem(key) ?? ""; } catch { return ""; }
}
function writeStoredValue(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch { /* WebView 可能禁用 storage。 */ }
}
function isAgentWindowSlot(value: unknown): value is AgentWindowSlot {
  return value === "claude" || value === "codex" || value === "dsh";
}
function readStoredChatSlot(): ChatSlot {
  const raw = readStoredValue(CHAT_SLOT_STORAGE_KEY);
  return raw === "main" || isAgentWindowSlot(raw) ? raw : "main";
}

/** 窗口偏好：新建会话 / 未绑定会话时使用的 agent、模型、渠道与思考强度。 */
interface AgentWindowPreference { agentId: string; model: string; provider: string; effort: string }
function readStoredWindowPreference(slot: AgentWindowSlot): AgentWindowPreference {
  const raw = readStoredValue(`${AGENT_WINDOW_PREF_PREFIX}${slot}`);
  if (raw) {
    try {
      const parsed = JSON.parse(raw) as Partial<AgentWindowPreference> | null;
      if (parsed && typeof parsed === "object") {
        return {
          agentId: typeof parsed.agentId === "string" && parsed.agentId ? parsed.agentId : SLOT_DEFAULT_AGENT[slot],
          model: typeof parsed.model === "string" ? parsed.model : "",
          provider: typeof parsed.provider === "string" ? parsed.provider : "",
          effort: typeof parsed.effort === "string" ? parsed.effort : "",
        };
      }
    } catch { /* 旧数据损坏时回退默认偏好。 */ }
  }
  return { agentId: SLOT_DEFAULT_AGENT[slot], model: "", provider: "", effort: "" };
}

const chatSlot = ref<ChatSlot>(readStoredChatSlot());
const chatNavExpanded = ref(chatSlot.value !== "main");
const agentWindowPrefs = reactive<Record<AgentWindowSlot, AgentWindowPreference>>({
  claude: readStoredWindowPreference("claude"),
  codex: readStoredWindowPreference("codex"),
  dsh: readStoredWindowPreference("dsh"),
});
/** 每个 agent 窗口最近一次模型目录：底部信息条用它把 model id 解析成展示名 + 渠道。 */
const agentCatalogs = reactive<Record<AgentWindowSlot, AgentModelCatalog | null>>({
  claude: null,
  codex: null,
  dsh: null,
});
async function ensureAgentCatalog(slot: AgentWindowSlot) {
  if (agentCatalogs[slot]) return;
  try {
    const catalog = await fetchAgentModelCatalog(slot);
    if (!agentCatalogs[slot]) agentCatalogs[slot] = catalog;
  } catch { /* 目录不可用时保留原始 id 展示，选择器里仍可手输。 */ }
}
const agentSessionIds = reactive<Record<AgentWindowSlot, string>>({
  claude: readStoredValue(`${AGENT_SESSION_STORAGE_PREFIX}claude`),
  codex: readStoredValue(`${AGENT_SESSION_STORAGE_PREFIX}codex`),
  dsh: readStoredValue(`${AGENT_SESSION_STORAGE_PREFIX}dsh`),
});
const sessionError = ref("");
const creatingSession = ref(false);
const switchingSessionId = ref("");
const renamingSessionId = ref<string | null>(null);
const renameDraft = ref("");
const renameInputRef = ref<HTMLInputElement | null>(null);
const isMainSlot = computed(() => chatSlot.value === "main");
const agentSlot = computed<AgentWindowSlot | null>(() => (isAgentWindowSlot(chatSlot.value) ? chatSlot.value : null));
const agentWindowNavItems = computed(() =>
  AGENT_SLOTS.map((slot) => ({ slot, label: SLOT_WINDOW_LABELS[slot], icon: SLOT_WINDOW_ICONS[slot] })),
);
function saveWindowPreference(slot: AgentWindowSlot, patch: Partial<AgentWindowPreference>) {
  const next: AgentWindowPreference = { ...agentWindowPrefs[slot], ...patch };
  if (!next.agentId) next.agentId = SLOT_DEFAULT_AGENT[slot];
  agentWindowPrefs[slot] = next;
  writeStoredValue(`${AGENT_WINDOW_PREF_PREFIX}${slot}`, JSON.stringify(next));
}

function rememberAgentSession(slot: AgentWindowSlot, id: string) {
  agentSessionIds[slot] = id;
  writeStoredValue(`${AGENT_SESSION_STORAGE_PREFIX}${slot}`, id);
}

function openChatSlot(slot: ChatSlot) {
  activePage.value = "chat";
  if (slot !== "main") chatNavExpanded.value = true;
  chatSlot.value = slot;
}

function toggleChatSubnav() {
  chatNavExpanded.value = !chatNavExpanded.value;
}

watch(chatSlot, (slot) => {
  writeStoredValue(CHAT_SLOT_STORAGE_KEY, slot);
  if (slot !== "main") chatNavExpanded.value = true;
});

// === 系统设置：侧边栏子列表（与「对话互动」同一套展开交互） ===
type SystemSection = "backend" | "models" | "permissions" | "persona" | "skills" | "weather";
interface SystemSectionItem { id: SystemSection; label: string; icon: string; desc: string }
const SYSTEM_SECTION_STORAGE_KEY = "dash.system-section";
const SYSTEM_SECTIONS: SystemSectionItem[] = [
  { id: "backend", label: "AI 后端", icon: "sliders", desc: "选择由谁来回答，查看连接状态" },
  { id: "models", label: "模型接口", icon: "api", desc: "直连 API 的模型配置、密钥与接口协议" },
  { id: "permissions", label: "执行权限", icon: "shield-bolt", desc: "工具执行前要不要先问你" },
  { id: "persona", label: "角色设定", icon: "persona-heart", desc: "性格、职业，以及追加到每次对话的系统提示" },
  { id: "skills", label: "技能", icon: "skill-puzzle", desc: "激活一个技能，改变它处理任务的方式" },
  { id: "weather", label: "天气提醒", icon: "weather-sun", desc: "天气栏与每日天气问候" },
];
function isSystemSection(value: unknown): value is SystemSection {
  return SYSTEM_SECTIONS.some((item) => item.id === value);
}
function readStoredSystemSection(): SystemSection {
  const raw = readStoredValue(SYSTEM_SECTION_STORAGE_KEY);
  return isSystemSection(raw) ? raw : "backend";
}
const systemSection = ref<SystemSection>(readStoredSystemSection());
const systemNavExpanded = ref(false);
const currentSystemSection = computed(
  () => SYSTEM_SECTIONS.find((item) => item.id === systemSection.value) ?? SYSTEM_SECTIONS[0],
);

/** 进入系统设置的某个子页（缺省回到上次停留的子页），并展开侧边栏子列表。 */
function openSystemSection(section: SystemSection = systemSection.value) {
  systemSection.value = section;
  activePage.value = "system";
  systemNavExpanded.value = true;
}

function toggleSystemSubnav() {
  systemNavExpanded.value = !systemNavExpanded.value;
}

watch(systemSection, (section) => {
  writeStoredValue(SYSTEM_SECTION_STORAGE_KEY, section);
  // 子页切换不经过页面级 Transition，手动回到顶部，避免停在上个子页的滚动位置。
  void nextTick(() => document.querySelector(".dashboard-content")?.scrollTo({ top: 0 }));
});
watch(activePage, (page) => {
  if (page === "system") systemNavExpanded.value = true;
});

const backendOptions = [
  { id: "claude_code", label: "Claude Code", short: "Claude", icon: "claude", desc: "本机 Claude Code CLI，可读写文件、执行命令" },
  { id: "codex", label: "Codex", short: "Codex", icon: "codex", desc: "本机 Codex CLI，默认在工作目录沙箱内执行" },
  { id: "dsh", label: "DeepSeek Harness", short: "DSH", icon: "dsh", desc: "本机 DeepSeek Harness，复用本机登录与模型" },
  { id: "direct_api", label: "直连 API", short: "API", icon: "api", desc: "OpenAI 兼容接口，内置文件、搜索、记忆等工具" },
] as const;
function backendLabel(id: string) {
  return backendOptions.find((item) => item.id === id)?.label ?? id;
}
function backendShortLabel(id: string) {
  return backendOptions.find((item) => item.id === id)?.short ?? id;
}
function backendIcon(id: string) {
  return backendOptions.find((item) => item.id === id)?.icon ?? "sliders";
}

// === 同屏多窗格工作区 ===
// 窗格身份（paneId）与会话身份（sessionId）分离；布局与绑定持久化到 localStorage。
const workspaceLayout = ref<WorkspaceLayout>(readWorkspaceLayout());
const workspacePanes = ref<string[]>(readWorkspacePanes(workspaceLayout.value));
const workspacePaneIds = computed(() =>
  Array.from({ length: layoutPaneCount(workspaceLayout.value) }, (_, index) => `pane-${index}`),
);
const isWorkspaceMode = computed(() => workspaceLayout.value !== "single");
const WORKSPACE_LAYOUT_ICONS: Record<WorkspaceLayout, Component> = {
  single: LayoutSingle,
  two: Columns2,
  three: Columns3,
  master: PanelLeft,
  four: Grid2x2,
};

function setWorkspaceLayout(layout: WorkspaceLayout) {
  const previous = workspacePanes.value;
  workspaceLayout.value = layout;
  saveWorkspaceLayout(layout);
  // 单窗格只是暂时查看会话，保留多窗格绑定，返回同屏时附件仍出现在原会话。
  if (layout !== "single") {
    workspacePanes.value = Array.from({ length: layoutPaneCount(layout) }, (_, index) => previous[index] || "");
  }
  // 从单窗格切到多窗格时，把当前会话带进第一个窗格，避免面对一屏空窗格。
  if (layout !== "single" && !workspacePanes.value[0]) workspacePanes.value[0] = currentSessionId.value;
  saveWorkspacePanes(workspacePanes.value);
}
function assignWorkspacePane(paneId: string, sessionId: string) {
  const index = workspacePaneIds.value.indexOf(paneId);
  if (index < 0) return;
  const next = [...workspacePanes.value];
  // 同一会话只在一个窗格显示，避免两个输入框同时作用于同一回合。
  const duplicate = next.findIndex((id, i) => id === sessionId && i !== index);
  if (duplicate >= 0) next[duplicate] = "";
  next[index] = sessionId;
  workspacePanes.value = next;
  saveWorkspacePanes(next);
}
function closeWorkspacePane(paneId: string) {
  // 只移除视图：不停止回合、不删除会话。剩一个窗格时回到单窗格视图。
  const index = workspacePaneIds.value.indexOf(paneId);
  if (index < 0 || workspaceLayout.value === "single") return;
  const next = workspacePanes.value.filter((_, i) => i !== index);
  if (next.filter(Boolean).length <= 1) {
    const only = next.find(Boolean) || "";
    workspacePanes.value = [only];
    setWorkspaceLayout("single");
    if (only) void openSessionInSingle(only);
  } else {
    // 数组变短后必须同步降级布局，否则窗格数与绑定数错位、会话整体左移。
    const layout = WORKSPACE_LAYOUTS.find((item) => item.panes === next.length)?.id ?? "two";
    workspaceLayout.value = layout;
    saveWorkspaceLayout(layout);
    workspacePanes.value = next;
    saveWorkspacePanes(next);
  }
}
function moveWorkspacePane(targetPaneId: string, sourcePaneId: string) {
  const target = workspacePaneIds.value.indexOf(targetPaneId);
  const source = workspacePaneIds.value.indexOf(sourcePaneId);
  if (target < 0 || source < 0 || target === source) return;
  const next = [...workspacePanes.value];
  [next[target], next[source]] = [next[source], next[target]];
  workspacePanes.value = next;
  saveWorkspacePanes(next);
}
async function openSessionInSingle(sessionId: string) {
  const session = sessions.find(sessionId);
  if (!session) return;
  if (session.slot === "main") {
    await sessions.setActiveMain(session.id);
    openChatSlot("main");
  } else {
    rememberAgentSession(session.slot, session.id);
    openChatSlot(session.slot);
    await ensureAgentWindow(session.slot);
  }
  setWorkspaceLayout("single");
}
// 会话列表就绪后剔除失效绑定（如此前删除的会话），避免窗格指向不存在的会话。
watch(() => sessions.sessions, (list) => {
  if (!list.length) return;
  const ids = new Set(list.map((item) => item.id));
  if (workspacePanes.value.some((id) => id && !ids.has(id))) {
    workspacePanes.value = workspacePanes.value.map((id) => (id && !ids.has(id) ? "" : id));
    saveWorkspacePanes(workspacePanes.value);
  }
});

// 自定义提示（Dashboard 速览面板）
const customPromptDraft = ref("");
const customPromptSaved = ref(false);
watch(
  () => skillsStore.customPrompt,
  (val) => {
    if (customPromptDraft.value !== val) customPromptDraft.value = val ?? "";
  },
  { immediate: true }
);

async function saveCustomPrompt() {
  try {
    await skillsStore.saveCustomPrompt(customPromptDraft.value);
    customPromptSaved.value = true;
    setTimeout(() => (customPromptSaved.value = false), 2000);
  } catch (e) {
    alert("保存自定义提示失败: " + e);
  }
}

async function activateSkill(id: string | null) {
  try {
    await skillsStore.setActive(id);
  } catch (e) {
    alert("切换激活技能失败: " + e);
  }
}

type MemoryItem = {
  id: number;
  category: string;
  key: string;
  value: string;
  created_at: number;
};

type ScheduledTask = {
  id: number;
  title: string;
  note: string;
  due_at: number;
  repeat: "once" | "daily" | "weekly" | "monthly" | string;
  enabled: boolean;
  last_triggered_at?: number | null;
  created_at: number;
  updated_at: number;
};

type ApiProfile = {
  id: string;
  name: string;
  api_key?: string;
  api_key_mask?: string;
  has_api_key?: boolean;
  base_url: string;
  model: string;
  confirm_enabled: boolean;
  thinking_depth: string;
  execution_mode: string;
  search_provider: string;
  auto_approved_tools: string[];
  stream_mode: string;
  api_mode: string;
};

type ToolEvent = {
  id: string;
  tool_name: string;
  status: string;
  summary: string;
  arguments: string;
  command?: string | null;
  path?: string | null;
  output?: string | null;
  approved?: boolean | null;
  /** Codex 命令类工具的退出码，非 0 时在轨迹里标红。 */
  exit_code?: number | null;
  /** 工具耗时（毫秒），用于轨迹行右侧的耗时标注。 */
  duration_ms?: number | null;
  /** 文件变更清单，轨迹展开后逐条列出。 */
  changes?: { path: string; kind?: string | null }[] | null;
};

type ToolConfirmPayload = {
  id: string;
  tool_name: string;
  arguments: string;
  summary?: string;
  command?: string | null;
  path?: string | null;
};

type AnswerDeltaPayload = {
  text: string;
};

type DashSlashPickerItem =
  | {
      type: "skill";
      key: string;
      title: string;
      description: string;
      hint: string;
      skill: Skill;
    }
  | {
      type: "command";
      key: string;
      title: string;
      description: string;
      hint: string;
      command: SlashCommand;
    };

// === 系统信息 ===
const systemInfo = ref({ cpu: 0, memory: 0 });
let sysInfoTimer: ReturnType<typeof setInterval> | null = null;

// === 天气信息 ===
const weatherInfo = ref<WeatherInfo | null>(null);
const weatherConfig = ref<WeatherConfig>({ ...DEFAULT_WEATHER_CONFIG });
const weatherError = ref("");
const isWeatherLoading = ref(false);
const isSavingWeatherConfig = ref(false);
const isTestingWeatherConfig = ref(false);
const weatherConfigResult = ref({ success: false, message: "" });
let weatherTimer: ReturnType<typeof setInterval> | null = null;

// === 模型 ===
const currentModel = ref("");
const modelSaved = ref(false);

// === 后端 ===
const backendType = ref("claude_code");
const apiConfig = ref({
  id: "",
  name: "",
  api_key: "",
  api_key_mask: "",
  has_api_key: false,
  base_url: "",
  model: "",
  confirm_enabled: true,
  thinking_depth: "auto",
  execution_mode: "normal",
  search_provider: "bing",
  auto_approved_tools: [] as string[],
  stream_mode: "auto",
  api_mode: "chat_completions",
});
const apiProfiles = ref<ApiProfile[]>([]);
const activeApiProfileId = ref("");
const isTestingConnection = ref(false);
const testResult = ref({ success: false, message: "" });
const isSavingConfig = ref(false);
const configSaved = ref(false);
const profileDeletingId = ref("");
const isFetchingModels = ref(false);
const modelFetchResult = ref({ success: false, message: "" });
const fetchedModels = ref<string[]>([]);
const selectedFetchedModel = ref("");
const isAddingFetchedModel = ref(false);
const activeDashMessageMenuId = ref<number | null>(null);

// === 引用回复（微信式引用前文对话） ===
const pendingQuote = ref<QuotedMessage | null>(null);

/** 引用块里显示的说话者昵称 */
function quoteSpeakerLabel(role: "user" | "assistant"): string {
  return role === "assistant" ? "小家伙" : "我";
}

// === Claude Code 状态 ===
const claudeStatus = ref({
  logged_in: false,
  token_preview: null,
  model: null,
  base_url: null
});

async function loadClaudeStatus() {
  try {
    claudeStatus.value = await invoke("check_claude_status");
  } catch (e) {
    console.error("加载 Claude Code 状态失败:", e);
  }
}

// === 皮肤 ===
const currentSkin = ref("default");
const themes = Object.values(THEMES);
const petCharacters = PET_CHARACTERS;
const currentCharacter = ref<PetCharacterId>("classic");

// === 字体颜色 ===
const currentFontColor = ref("");
const avatarInputRef = ref<HTMLInputElement | null>(null);

// === 对话背景 ===
const BG_KEY = "ai-desktop-pet.chat-bg";
const CUSTOM_BG_KEY = "ai-desktop-pet.chat-bg-custom";
const VOICE_SETTINGS_KEY = "voice_settings";
const TTS_SETTINGS_KEY = "tts_settings";
const COMPUTER_USE_ENABLED_KEY = "computer_use_enabled";
const chatBg = ref(localStorage.getItem(BG_KEY) || "none");
const customBgImage = ref(localStorage.getItem(CUSTOM_BG_KEY) || "");
const bgInputRef = ref<HTMLInputElement | null>(null);
const computerUseEnabled = ref(false);
const computerUseSaved = ref(false);

// === 语音 ===
const voiceSettings = ref<VoiceSettings>({ ...DEFAULT_VOICE_SETTINGS });
const availableVoices = ref<SpeechSynthesisVoice[]>([]);
const ttsSettings = ref<TtsSettings>({ ...DEFAULT_TTS_SETTINGS });
const edgeVoices = ref<TtsVoice[]>([]);
const edgeVoiceSearch = ref("");
const showAllEdgeVoices = ref(false);
// 试听与自动播报共用同一个 TtsPlayer：后端 state.tts_playback 是单会话播放器，
// 两条路径若各持一个实例会互相抢后端（play_wav_file 起手 self.stop() 杀对方会话）。
// 合并为单实例后，新 speak 通过 stopLocal() 递增共享 playbackToken，让旧循环优雅退出。
const ttsPlayer = new TtsPlayer();

// === 性格/职业 ===
const personalities = [
  { id: "gentle", name: "温柔陪伴", desc: "柔和耐心，先接住情绪" },
  { id: "energetic", name: "活泼元气", desc: "轻快积极，推动下一步" },
  { id: "calm", name: "冷静专业", desc: "克制清晰，优先结论" },
  { id: "mentor", name: "严谨导师", desc: "重视方法，指出问题" },
  { id: "witty", name: "毒舌吐槽", desc: "嘴硬犀利，骂醒问题" },
];

const professions = [
  { id: "companion", name: "日常陪伴", desc: "聊天、鼓励、整理想法" },
  { id: "coding", name: "编程助手", desc: "代码、报错、架构建议" },
  { id: "study", name: "学习教练", desc: "拆知识点、做计划" },
  { id: "writing", name: "写作编辑", desc: "润色、改写、提纲" },
  { id: "file_intake", name: "文件整理员", desc: "路径、分类、命名规则" },
  { id: "secretary", name: "效率秘书", desc: "待办、优先级、日程" },
];

const currentPersonality = ref("gentle");
const currentProfession = ref("companion");
const memories = ref<MemoryItem[]>([]);
const memoryDraft = ref({
  key: "",
  value: "",
  category: "manual",
});
const isSavingMemory = ref(false);
const memorySaved = ref(false);
const memoryDeletingId = ref<number | null>(null);
const scheduledTasks = ref<ScheduledTask[]>([]);
const taskDraft = ref({
  title: "",
  note: "",
  dueAtLocal: "",
  repeat: "once",
});
const isSavingTask = ref(false);
const taskSaved = ref(false);
const taskDeletingId = ref<number | null>(null);
const taskTogglingId = ref<number | null>(null);
const taskError = ref("");

// === Computed ===
const filteredEdgeVoices = computed(() => {
  const lang = (voiceSettings.value.language || "zh-CN").toLowerCase();
  const langBase = lang.split("-")[0];
  const query = edgeVoiceSearch.value.trim().toLowerCase();
  const selectedVoice = ttsSettings.value.voice;

  return edgeVoices.value.filter((voice) => {
    const voiceLang = voice.language.toLowerCase();
    const isSameLanguage = voiceLang === lang || voiceLang.startsWith(`${langBase}-`);
    const isSelected = voice.id === selectedVoice;
    if (!showAllEdgeVoices.value && !isSameLanguage && !isSelected) return false;
    if (!query) return true;
    return [voice.id, voice.name, voice.language, voice.gender].some((v) =>
      v.toLowerCase().includes(query)
    );
  });
});

const edgeVoiceSummary = computed(() => {
  if (edgeVoices.value.length === 0) return "正在加载人声列表";
  if (showAllEdgeVoices.value) return `全部 ${filteredEdgeVoices.value.length} / ${edgeVoices.value.length} 个`;
  return `当前语言 ${filteredEdgeVoices.value.length} / ${edgeVoices.value.length} 个`;
});

const petStateLabel = computed(() => {
  const map: Record<string, string> = {
    idle: "闲逛中", listening: "正在听", thinking: "思考中",
    speaking: "说话中", working: "工作中", sleeping: "睡觉中",
    happy: "开心", confused: "困惑", waving: "打招呼",
    hungry: "饿了", stuffed: "吃饱了", refusing: "拒绝中", dragging: "被拖拽",
  };
  return map[pet.state] || pet.state;
});

const happinessPercent = computed(() => Math.round((pet.happiness + 1) / 2 * 100));
const energyPercent = computed(() => Math.round(Math.max(0, Math.min(100, pet.energy))));

// 首页"最近一次对话"卡片：取最后一条用户或助手消息，跳过系统分割线
const lastConversationMessage = computed(() => {
  for (let i = chat.messages.length - 1; i >= 0; i--) {
    const msg = chat.messages[i];
    if (msg.role !== "system" && (msg.displayContent ?? msg.content).trim()) return msg;
  }
  return null;
});
const lastConversationPreview = computed(() => {
  const msg = lastConversationMessage.value;
  if (!msg) return "";
  const text = speakableText(msg.displayContent ?? msg.content).replace(/\s+/g, " ").trim();
  return text.length > 90 ? text.slice(0, 90) + "…" : text;
});
const lastConversationSpeaker = computed(() => {
  const msg = lastConversationMessage.value;
  if (!msg) return "";
  if (msg.role === "user") return "你";
  return msg.agent?.name || "桌宠";
});
const conversationTurnCount = computed(() => chat.messages.filter((msg) => msg.role === "user").length);

/**
 * 消息正文按 Markdown 渲染（**粗体**、# 标题、- 列表、[链接](url)……）。
 * 渲染器按消息 id + 文本缓存，流式刷新不会重复解析；@提及 输出 .dash-msg-mention 高亮。
 */
const renderedMessages = computed(() => {
  const map = new Map<number, MessageRenderSegment[]>();
  for (const message of paneMessages.value) {
    if (message.role !== "assistant" && message.role !== "user") continue;
    const raw = message.displayContent ?? message.content;
    map.set(
      message.id,
      renderMessageSegments(raw, {
        names: extractMentionNames(raw),
        mention: { htmlTag: '<span class="dash-msg-mention">' },
        cacheKey: "dash:" + message.id,
      }),
    );
  }
  return map;
});
function renderedMessageSegments(message: Message): MessageRenderSegment[] {
  return renderedMessages.value.get(message.id) ?? [];
}

/** 朗读前先转纯文本：星号、井号、链接地址都不念出来 */
function speakableText(raw: string): string {
  return markdownToPlainText(stripQuoteMarkers(raw));
}
const nextScheduledTask = computed(() => scheduledTasks.value
  .filter((task) => task.enabled)
  .reduce<ScheduledTask | null>((soonest, task) => (!soonest || task.due_at < soonest.due_at ? task : soonest), null));

function formatRelativeTime(timestamp: number) {
  if (!timestamp) return "";
  // 后端时间戳可能是秒，也可能是毫秒
  const ms = timestamp < 1e12 ? timestamp * 1000 : timestamp;
  const diff = Date.now() - ms;
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return `${Math.floor(diff / 86_400_000)} 天前`;
}

const thinkingDepthOptions = [
  { value: "auto", label: "自动" },
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高" },
];

/** 主对话思考强度滑块的档位说明（auto 由滑块的「自动」档承担）。 */
const mainEffortOptions = [
  { id: "low", name: "低", description: "快速回应，适合闲聊与简单问答" },
  { id: "medium", name: "中", description: "平衡速度与深度，日常任务推荐" },
  { id: "high", name: "高", description: "更充分的推理，适合复杂调试与规划" },
];
const mainEffortValue = computed(() => {
  const value = apiConfig.value.thinking_depth;
  return !value || value === "auto" ? "" : value;
});
const mainThinkingLabel = computed(() =>
  thinkingDepthOptions.find((item) => item.value === apiConfig.value.thinking_depth)?.label || "自动",
);
const mainModelOptions = computed(() =>
  apiProfiles.value.map((profile) => ({ value: profile.id, label: profile.name || profile.model, desc: profile.model })),
);
function onMainEffortChange(id: string) {
  void updateThinkingDepth(id || "auto");
}

const executionModeOptions = [
  { value: "plan", label: "计划模式", desc: "只制定计划，不调用工具和改文件" },
  { value: "normal", label: "普通模式", desc: "每类操作首次执行前需要确认" },
  { value: "unreviewed", label: "无审查模式", desc: "所有工具直接执行" },
  { value: "custom", label: "自定义模式", desc: "自行选择免确认工具" },
];

const searchProviderOptions = [
  { value: "bing", label: "Bing", desc: "国内网络推荐使用" },
  { value: "duckduckgo", label: "DuckDuckGo", desc: "海外网络可选" },
];

const toolPermissionOptions = [
  { id: "read_file", label: "读取文件" },
  { id: "write_file", label: "写入文件" },
  { id: "list_directory", label: "列出目录" },
  { id: "run_command", label: "执行命令" },
  { id: "web_search", label: "网页搜索" },
  { id: "open_app", label: "打开应用" },
  { id: "create_scheduled_task", label: "创建定时任务" },
  { id: "computer_screenshot", label: "查看屏幕" },
  { id: "computer_wait", label: "等待界面" },
  { id: "window_list", label: "列出窗口" },
  { id: "browser_snapshot", label: "查看浏览器" },
  { id: "create_agent", label: "创建智能体" },
  { id: "update_agent", label: "修改智能体" },
];

const currentModelLabel = computed(() => {
  if (backendType.value === "direct_api") {
    return apiConfig.value.model || "未选择模型";
  }
  if (backendType.value === "dsh") return currentModel.value || "DeepSeek Harness / 本机配置";
  return currentModel.value || (backendType.value === "codex" ? "Codex / 本机配置" : "Claude Code");
});

const chatHeaderTitle = computed(() => {
  for (let i = chat.messages.length - 1; i >= 0; i--) {
    const name = chat.messages[i].agent?.name;
    if (name) return name;
  }
  return "主助理";
});
const recentMemories = computed(() => memories.value.slice(0, 5));

const currentExecutionModeLabel = computed(() => {
  if (backendType.value === "codex") return "Codex";
  if (backendType.value === "dsh") return "DeepSeek Harness";
  if (backendType.value !== "direct_api") return "Claude Code";
  return apiExecutionModeLabel.value;
});
/** 直连 API 当前模型配置的执行模式（不随后端切换变化，供系统设置页展示）。 */
const apiExecutionModeLabel = computed(
  () => executionModeOptions.find((item) => item.value === apiConfig.value.execution_mode)?.label || "普通模式",
);

const contextUsageLabel = computed(() => {
  const chars = chat.messages.reduce((sum, msg) => {
    return sum + msg.content.length + (msg.thinking?.length || 0);
  }, thinkingContent.value.length + streamingAnswer.value.length);
  if (chars === 0) return "0 字符";
  const approxTokens = Math.max(1, Math.ceil(chars / 2));
  return `${chars} 字符 / 约 ${approxTokens} tokens`;
});

const dashSlashQuery = computed(() => {
  if (chat.isLoading || !isMainSlot.value) return null;
  return slashQueryFromInput(chatInput.value);
});
const dashSlashItems = computed<DashSlashPickerItem[]>(() => {
  const query = dashSlashQuery.value;
  if (query === null) return [];

  const skillItems: DashSlashPickerItem[] = skillsStore.skills
    .filter((skill) => skillMatchesSlashQuery(skill, query))
    .map((skill) => ({
      type: "skill",
      key: `skill:${skill.id}`,
      title: skill.name || "(未命名技能)",
      description: skill.description || "激活这个 skill 并继续输入你的需求",
      hint: skill.is_active ? "已激活" : "/skill",
      skill,
    }));

  const commandItems: DashSlashPickerItem[] = SLASH_COMMANDS
    .filter((command) => slashCommandMatches(command, query, "dashboard"))
    .map((command) => ({
      type: "command",
      key: `command:${command.id}`,
      title: command.title,
      description: command.description,
      hint: command.trigger,
      command,
    }));

  return [...skillItems, ...commandItems].slice(0, 10);
});
const showDashSlashMenu = computed(
  () => dashSlashQuery.value !== null && dashSlashItems.value.length > 0,
);

// === @ 提及智能体 ===
const dashMentionQuery = computed(() => {
  if (chat.isLoading || !isMainSlot.value) return null;
  return mentionQueryFromInput(chatInput.value);
});
const dashMentionItems = computed<Agent[]>(() => {
  const query = dashMentionQuery.value;
  if (query === null) return [];
  const q = query.query.toLowerCase();
  return agentsStore.agents
    .filter((agent) => {
      if (!q) return true;
      return [agent.name, agent.description].some((v) => v.toLowerCase().includes(q));
    })
    .slice(0, 8);
});
const showDashMentionMenu = computed(
  () => dashMentionQuery.value !== null && dashMentionItems.value.length > 0,
);

/** 用户消息里被 @ 的智能体名字（用于发送时传给后端） */
function dashMentionNames(text: string): string[] {
  return extractMentionNames(text).filter((name) => agentsStore.findByName(name));
}

// === 智能体管理（智能体工坊） ===
const AGENT_AVATARS = [
  "🤖", "🧑‍💻", "📚", "✍️", "🗂️", "🧮", "🔍", "🌍",
  "🎨", "💼", "🧭", "🛠️", "🧠", "🕵️", "🎯", "✈️",
];
const agentEditorOpen = ref(false);
const editingAgentId = ref<string | null>(null);
const agentDraft = ref({
  id: "",
  name: "",
  avatar: "🤖",
  description: "",
  system_prompt: "",
  model: "",
  backend: "direct_api" as Agent["backend"],
  api_profile_id: "",
  allowed_tools: [] as string[],
});
const agentSaveError = ref("");
const agentSaved = ref(false);
const agentDeletingId = ref<string | null>(null);
const agentGenDesc = ref("");
const agentGenLoading = ref(false);
const agentGenError = ref("");

function openAgentsPage() {
  activePage.value = "agents";
  void agentsStore.load();
}

async function configureBuiltinAgent(agent: Agent) {
  const command = agent.backend === "codex"
    ? "open_codex_config"
    : agent.backend === "dsh" ? "open_dsh_config" : "open_claude_config";
  try {
    await invoke(command);
  } catch (error) { alert("打开登录配置失败：" + error); }
}

function openAgentEditor(agent?: Agent) {
  if (agent?.is_builtin) return;
  if (agent) {
    editingAgentId.value = agent.id;
    agentDraft.value = {
      id: agent.id,
      name: agent.name,
      avatar: agent.avatar || "🤖",
      description: agent.description,
      system_prompt: agent.system_prompt,
      model: agent.model,
      backend: agent.backend || "direct_api",
      api_profile_id: agent.api_profile_id || "",
      allowed_tools: [...agent.allowed_tools],
    };
  } else {
    editingAgentId.value = null;
    agentDraft.value = {
      id: "agent-" + Date.now().toString(36),
      name: "",
      avatar: "🤖",
      description: "",
      system_prompt: "",
      model: "",
      backend: "direct_api",
      api_profile_id: "",
      allowed_tools: [],
    };
  }
  agentSaveError.value = "";
  agentSaved.value = false;
  agentEditorOpen.value = true;
}

function closeAgentEditor() {
  agentEditorOpen.value = false;
}

function toggleAgentTool(toolId: string, enabled: boolean) {
  const set = new Set(agentDraft.value.allowed_tools);
  if (enabled) set.add(toolId);
  else set.delete(toolId);
  agentDraft.value.allowed_tools = Array.from(set).sort();
}

async function saveAgentDraft() {
  agentSaveError.value = "";
  const name = agentDraft.value.name.trim();
  if (!name) {
    agentSaveError.value = "请填写智能体名字";
    return;
  }
  if (/\s|@/.test(name)) {
    agentSaveError.value = "名字不能包含空格或 @（名字会用于对话中的 @ 提及）";
    return;
  }
  try {
    const now = Math.floor(Date.now() / 1000);
    await agentsStore.upsert({
      id: agentDraft.value.id,
      name,
      avatar: agentDraft.value.avatar.trim() || "🤖",
      description: agentDraft.value.description.trim(),
      system_prompt: agentDraft.value.system_prompt.trim(),
      model: agentDraft.value.model.trim(),
      backend: agentDraft.value.backend,
      api_profile_id: agentDraft.value.api_profile_id,
      is_builtin: false,
      allowed_tools: agentDraft.value.allowed_tools,
      created_at: now,
      updated_at: now,
    });
    agentEditorOpen.value = false;
    agentSaved.value = true;
    setTimeout(() => (agentSaved.value = false), 2000);
  } catch (e) {
    agentSaveError.value = String(e);
  }
}

async function deleteAgentDraft(agent: Agent) {
  if (agent.is_builtin) return;
  if (!window.confirm("确定删除智能体「" + agent.name + "」吗？此操作不可撤销。")) return;
  agentDeletingId.value = agent.id;
  try {
    await agentsStore.remove(agent.id);
  } catch (e) {
    alert("删除失败: " + e);
  } finally {
    agentDeletingId.value = null;
  }
}

/** 用 AI 根据自然语言描述生成智能体定义并预填表单 */
async function generateAgentFromDesc() {
  const desc = agentGenDesc.value.trim();
  if (!desc || agentGenLoading.value) return;
  agentGenLoading.value = true;
  agentGenError.value = "";
  try {
    const spec = await agentsStore.generateSpec(desc);
    editingAgentId.value = null;
    agentDraft.value = {
      id: "agent-" + Date.now().toString(36),
      name: spec.name || "新智能体",
      avatar: spec.avatar || "🤖",
      description: spec.description || "",
      system_prompt: spec.system_prompt || "",
      model: "",
      backend: "direct_api",
      api_profile_id: "",
      allowed_tools: spec.allowed_tools || [],
    };
    agentSaveError.value = "";
    agentSaved.value = false;
    agentEditorOpen.value = true;
  } catch (e) {
    agentGenError.value = String(e);
  } finally {
    agentGenLoading.value = false;
  }
}

/** 编辑器里只写了名字 / 一句话描述时，让 AI 补全角色设定（保留 id、后端和模型配置）。 */
const agentCompleteLoading = ref(false);
async function completeAgentDraft() {
  const draft = agentDraft.value;
  const brief = [
    draft.name.trim() && `名字：${draft.name.trim()}`,
    draft.description.trim() && `定位：${draft.description.trim()}`,
    draft.system_prompt.trim() && `已有设定（请在此基础上扩写完善）：${draft.system_prompt.trim()}`,
  ].filter(Boolean).join("\n");
  if (!brief || agentCompleteLoading.value) {
    agentSaveError.value = "先写个名字或一句话描述，AI 才知道要补全什么";
    return;
  }
  agentCompleteLoading.value = true;
  agentSaveError.value = "";
  try {
    const spec = await agentsStore.generateSpec(brief);
    agentDraft.value = {
      ...draft,
      // 用户已经起好的名字不改，避免 @ 习惯被打乱
      name: draft.name.trim() || spec.name || "新智能体",
      avatar: draft.avatar && draft.avatar !== "🤖" ? draft.avatar : spec.avatar || "🤖",
      description: draft.description.trim() || spec.description || "",
      system_prompt: spec.system_prompt || draft.system_prompt,
      allowed_tools: draft.allowed_tools.length ? draft.allowed_tools : spec.allowed_tools || [],
    };
  } catch (e) {
    agentSaveError.value = String(e);
  } finally {
    agentCompleteLoading.value = false;
  }
}

// 陪伴动作：只改桌宠状态，点在舞台上当场演出来
const companionActions = [
  { id: "wave", icon: "wave", label: "打招呼", desc: "挥挥手" },
  { id: "happy", icon: "happy", label: "开心一下", desc: "加一点元气" },
  { id: "sleep", icon: "sleep", label: "去睡觉", desc: "安静休息" },
  { id: "wake", icon: "wake", label: "叫醒它", desc: "回到待命" },
];

// 对话操作：动的是上下文，与陪伴动作分层；清空是破坏性操作，必须走确认
const conversationActions = [
  { id: "new-chat", icon: "+", label: "新对话", desc: "保存摘要并重开上下文" },
  { id: "clear", icon: "⌫", label: "清空对话", desc: "删除全部聊天记录", danger: true },
];

const missionEntries = [
  { page: "chat" as NavPage, icon: "chat-cabin", title: "对话舱", desc: "直接派发任务或闲聊" },
  { page: "appearance" as NavPage, icon: "wardrobe", title: "换装台", desc: "皮肤、字体和背景" },
  { page: "voice" as NavPage, icon: "voice-station", title: "声线站", desc: "快捷键、语音和试听" },
];

const activeActionId = ref("");
let actionFeedbackTimer: ReturnType<typeof setTimeout> | null = null;

// === 预设 ===
const PRESETS: Record<string, { base_url: string; model: string }> = {
  openai: { base_url: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  deepseek: { base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  qwen: { base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen-plus" },
  ollama: { base_url: "http://localhost:11434/v1", model: "qwen2.5:7b" },
};
const presetOptions = [
  { id: "openai", label: "OpenAI" },
  { id: "deepseek", label: "DeepSeek" },
  { id: "qwen", label: "通义千问" },
  { id: "ollama", label: "Ollama 本地" },
];
const streamModeOptions = [
  { value: "auto", label: "自动检测" },
  { value: "stream", label: "流式 SSE" },
  { value: "non_stream", label: "非流式 JSON" },
];
const apiModeOptions = [
  { value: "chat_completions", label: "Chat Completions" },
  { value: "responses", label: "Responses" },
];
const streamModeLabel = computed(
  () => streamModeOptions.find((item) => item.value === apiConfig.value.stream_mode)?.label || "自动检测",
);
const apiModeLabel = computed(
  () => apiModeOptions.find((item) => item.value === apiConfig.value.api_mode)?.label || "Chat Completions",
);

async function confirmDeleteApiProfile(profile: ApiProfile) {
  const name = profile.name || profile.model;
  if (!window.confirm(`删除模型配置「${name}」？保存的 API 密钥也会一起移除。`)) return;
  await deleteApiProfile(profile.id);
}

// === 加载函数 ===
async function loadSystemInfo() {
  try { systemInfo.value = await invoke("get_system_info"); } catch {}
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function delay(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms));
}

async function loadWeatherConfig() {
  try {
    const loaded = await invoke<WeatherConfig>("get_weather_config");
    weatherConfig.value = { ...DEFAULT_WEATHER_CONFIG, ...loaded };
  } catch (e) {
    console.error("加载天气设置失败:", e);
    weatherConfig.value = { ...DEFAULT_WEATHER_CONFIG };
  }
}

async function loadWeather(forceRefresh = false) {
  if (!weatherConfig.value.enabled) {
    weatherInfo.value = null;
    weatherError.value = "";
    return;
  }
  if (isWeatherLoading.value) return;

  isWeatherLoading.value = true;
  try {
    const weather = await invoke<WeatherInfo | null>(forceRefresh ? "refresh_weather" : "get_weather");
    weatherInfo.value = normalizeWeatherInfo(weather);
    weatherError.value = "";
  } catch (e) {
    if (!forceRefresh) {
      await delay(1200);
      try {
        const weather = await invoke<WeatherInfo | null>("get_weather");
        weatherInfo.value = normalizeWeatherInfo(weather);
        weatherError.value = "";
        return;
      } catch {}
    }
    weatherInfo.value = null;
    weatherError.value = errorMessage(e);
    console.error("获取天气失败:", e);
  } finally {
    isWeatherLoading.value = false;
  }
}

async function refreshWeather() {
  await loadWeather(true);
}

function handleWeatherUpdate(event: { payload: WeatherUpdateEvent }) {
  void loadWeather();
  if (event.payload.sent_today && event.payload.message) {
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "system" && last.content === event.payload.message)) {
      chat.addSystemMessage(event.payload.message);
    }
  }
  scrollDashChatToBottom();
}

async function saveWeatherConfig() {
  isSavingWeatherConfig.value = true;
  weatherConfigResult.value = { success: false, message: "" };
  try {
    const saved = await invoke<WeatherConfig>("set_weather_config", {
      enabled: weatherConfig.value.enabled,
      location: weatherConfig.value.location,
      apiUrl: weatherConfig.value.api_url,
    });
    weatherConfig.value = saved;
    weatherConfigResult.value = { success: true, message: "天气设置已保存。" };
    if (saved.enabled) {
      await loadWeather(true);
    } else {
      weatherInfo.value = null;
      weatherError.value = "";
    }
  } catch (e) {
    weatherConfigResult.value = { success: false, message: "天气设置保存失败: " + errorMessage(e) };
  } finally {
    isSavingWeatherConfig.value = false;
  }
}

async function testWeatherConfig() {
  if (isTestingWeatherConfig.value) return;
  isTestingWeatherConfig.value = true;
  weatherConfigResult.value = { success: false, message: "" };
  try {
    const weather = await invoke<WeatherInfo>("test_weather_config", {
      location: weatherConfig.value.location,
      apiUrl: weatherConfig.value.api_url,
    });
    const normalized = normalizeWeatherInfo(weather);
    if (normalized) weatherInfo.value = normalized;
    weatherConfigResult.value = {
      success: true,
      message: normalized ? `天气 API 正常：${formatWeatherDisplay(normalized)}` : "天气 API 正常。",
    };
  } catch (e) {
    weatherConfigResult.value = { success: false, message: "天气 API 测试失败: " + errorMessage(e) };
  } finally {
    isTestingWeatherConfig.value = false;
  }
}

async function loadMemories() {
  try { memories.value = await invoke<MemoryItem[]>("get_memories"); } catch {}
}

async function loadScheduledTasks() {
  try { scheduledTasks.value = await invoke<ScheduledTask[]>("get_scheduled_tasks"); } catch {}
}

async function loadCurrentModel() {
  try { currentModel.value = await invoke("get_current_model"); } catch {}
}

async function loadCurrentSkin() {
  try {
    currentSkin.value = resolveSkinId(await invoke<string>("get_skin"));
    pet.skin = currentSkin.value;
  } catch {}
}

async function loadCurrentCharacter() {
  try {
    currentCharacter.value = resolvePetCharacterId(await invoke<string>("get_setting_value", {
      key: PET_CHARACTER_SETTING_KEY,
    }));
    pet.customPixelPetAsset = await getActiveCustomPixelPetAsset();
    if (currentCharacter.value === "custom-pixel" && !pet.customPixelPetAsset) {
      currentCharacter.value = "classic";
      await setActiveCustomPixelPetAsset(null).catch(() => null);
    }
    pet.character = currentCharacter.value;
  } catch {
    currentCharacter.value = pet.character;
  }
}

async function loadCurrentFontColor() {
  try {
    currentFontColor.value = await invoke<string>("get_font_color");
    pet.fontColor = currentFontColor.value;
  } catch { currentFontColor.value = pet.fontColor; }
}

async function loadUserAvatar() {
  try { pet.userAvatar = await invoke<string>("get_user_avatar"); } catch {}
}

async function loadPersonality() {
  try { currentPersonality.value = await invoke("get_personality"); } catch {}
}

async function loadProfession() {
  try { currentProfession.value = await invoke("get_profession"); } catch {}
}

async function loadVoiceSettings() {
  try {
    voiceSettings.value = parseVoiceSettings(await invoke<string>("get_setting_value", { key: VOICE_SETTINGS_KEY }));
  } catch { voiceSettings.value = { ...DEFAULT_VOICE_SETTINGS }; }

  availableVoices.value = await getAvailableVoices();

  try {
    const raw = await invoke<string>("get_setting_value", { key: TTS_SETTINGS_KEY });
    if (raw) ttsSettings.value = { ...DEFAULT_TTS_SETTINGS, ...JSON.parse(raw) };
  } catch { ttsSettings.value = { ...DEFAULT_TTS_SETTINGS }; }

  try { edgeVoices.value = await listEdgeVoices(); } catch {}

  if (ttsSettings.value.engine === "edge" && edgeVoices.value.length > 0) {
    const hasSavedVoice = edgeVoices.value.some((v) => v.id === ttsSettings.value.voice);
    if (!hasSavedVoice) await updateTtsSettings({ voice: edgeVoices.value[0].id });
  }
}

async function loadComputerUseSettings() {
  try {
    const value = await invoke<string>("get_setting_value", { key: COMPUTER_USE_ENABLED_KEY });
    computerUseEnabled.value = value === "true";
  } catch {
    computerUseEnabled.value = false;
  }
}

async function updateComputerUseEnabled(enabled: boolean) {
  computerUseEnabled.value = enabled;
  computerUseSaved.value = false;
  try {
    await invoke("set_setting_value", {
      key: COMPUTER_USE_ENABLED_KEY,
      value: enabled ? "true" : "false",
    });
    computerUseSaved.value = true;
    setTimeout(() => {
      computerUseSaved.value = false;
    }, 1800);
  } catch (e) {
    computerUseEnabled.value = !enabled;
    alert("Computer Use setting save failed: " + e);
  }
}

async function loadBackendSettings() {
  try {
    backendType.value = await invoke("get_backend_type");
    const config = await invoke<any>("get_api_config");
    applyApiConfig(config);
    await loadApiProfiles();
    if (backendType.value === "direct_api") {
      currentModel.value = `直连 API: ${apiConfig.value.model}`;
    } else if (backendType.value === "claude_code") {
      await loadClaudeStatus();
    }
  } catch {}
}

function applyApiConfig(config: Partial<ApiProfile>) {
  apiConfig.value = {
    id: config.id || "",
    name: config.name || config.model || "",
    api_key: "", // Keep empty to prevent browser autofill/overwrite bugs
    api_key_mask: (config as any).api_key_mask || config.api_key || "",
    has_api_key: (config as any).has_api_key || Boolean(config.api_key),
    base_url: config.base_url || "https://api.openai.com/v1",
    model: config.model || "gpt-4o-mini",
    confirm_enabled: config.confirm_enabled !== false,
    thinking_depth: config.thinking_depth || "auto",
    execution_mode: config.execution_mode || (config.confirm_enabled === false ? "unreviewed" : "normal"),
    search_provider: (config as any).search_provider || "bing",
    auto_approved_tools: Array.isArray(config.auto_approved_tools) ? config.auto_approved_tools : [],
    stream_mode: (config as any).stream_mode || "auto",
    api_mode: (config as any).api_mode || "chat_completions",
  };
  activeApiProfileId.value = apiConfig.value.id;
}

async function loadApiProfiles() {
  try {
    const result = await invoke<{ active_id: string; profiles: ApiProfile[] }>("list_api_profiles");
    apiProfiles.value = result.profiles || [];
    activeApiProfileId.value = result.active_id || apiConfig.value.id;
  } catch {
    apiProfiles.value = [];
  }
}

// === 操作函数 ===
async function selectBackend(type: string) {
  try {
    await invoke("set_backend_type", { backend: type });
    backendType.value = type;
    if (type !== "direct_api") {
      await loadCurrentModel();
      if (type === "claude_code") await loadClaudeStatus();
    } else {
      currentModel.value = `直连 API: ${apiConfig.value.model}`;
    }
  } catch (e) { alert("切换后端失败: " + e); }
}

function applyPreset(key: string) {
  const preset = PRESETS[key];
  if (!preset) return;
  apiConfig.value.base_url = preset.base_url;
  apiConfig.value.model = preset.model;
  if (!apiConfig.value.name.trim()) apiConfig.value.name = preset.model;
  testResult.value = { success: false, message: "" };
  clearFetchedModels();
}

type SaveApiConfigOptions = {
  quiet?: boolean;
  profileId?: string;
  profileName?: string;
  model?: string;
};

async function saveApiConfig(options: SaveApiConfigOptions = {}) {
  isSavingConfig.value = true;
  configSaved.value = false;
  const model = options.model || apiConfig.value.model;
  try {
    apiConfig.value.confirm_enabled = apiConfig.value.execution_mode !== "unreviewed";
    const saved = await invoke<ApiProfile>("set_api_config", {
      apiKey: apiConfig.value.api_key,
      baseUrl: apiConfig.value.base_url,
      model,
      confirmEnabled: apiConfig.value.confirm_enabled,
      thinkingDepth: apiConfig.value.thinking_depth,
      executionMode: apiConfig.value.execution_mode,
      searchProvider: apiConfig.value.search_provider,
      autoApprovedTools: apiConfig.value.auto_approved_tools,
      streamMode: apiConfig.value.stream_mode,
      apiMode: apiConfig.value.api_mode,
      profileId: options.profileId ?? apiConfig.value.id,
      profileName: options.profileName ?? (apiConfig.value.name || model),
    });
    applyApiConfig(saved);
    await loadApiProfiles();
    configSaved.value = true;
    currentModel.value = `直连 API: ${apiConfig.value.model}`;
    await currentWindow.emit("api-config-changed");
    setTimeout(() => { configSaved.value = false; }, 2000);
  } catch (e) {
    if (!options.quiet) alert("保存失败: " + e);
    if (options.quiet) throw e;
  }
  finally { isSavingConfig.value = false; }
}

async function updateThinkingDepth(value: string) {
  apiConfig.value.thinking_depth = value;
  if (!apiConfig.value.id || !apiConfig.value.base_url || !apiConfig.value.model) return;
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    testResult.value = { success: false, message: "思考深度保存失败: " + e };
  }
}

async function updateExecutionMode(value: string) {
  apiConfig.value.execution_mode = value;
  apiConfig.value.confirm_enabled = value !== "unreviewed";
  if (!apiConfig.value.id || !apiConfig.value.base_url || !apiConfig.value.model) return;
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    testResult.value = { success: false, message: "执行模式保存失败: " + e };
  }
}

async function updateSearchProvider(value: string) {
  apiConfig.value.search_provider = value;
  if (!apiConfig.value.id || !apiConfig.value.base_url || !apiConfig.value.model) return;
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    testResult.value = { success: false, message: "搜索源保存失败: " + e };
  }
}

async function testConnection() {
  isTestingConnection.value = true;
  testResult.value = { success: false, message: "" };
  try {
    const res = await invoke<string>("test_api_connection", {
      apiKey: apiConfig.value.api_key,
      baseUrl: apiConfig.value.base_url,
      model: apiConfig.value.model,
      thinkingDepth: apiConfig.value.thinking_depth,
    });
    testResult.value = { success: true, message: res };
  } catch (e: any) { testResult.value = { success: false, message: e.toString() }; }
  finally { isTestingConnection.value = false; }
}

async function updateStreamMode(value: string) {
  apiConfig.value.stream_mode = value;
  if (!apiConfig.value.id || !apiConfig.value.base_url || !apiConfig.value.model) return;
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    testResult.value = { success: false, message: "流式模式保存失败: " + e };
  }
}

async function updateApiMode(value: string) {
  apiConfig.value.api_mode = value;
  if (!apiConfig.value.id || !apiConfig.value.base_url || !apiConfig.value.model) return;
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    testResult.value = { success: false, message: "API 模式保存失败: " + e };
  }
}

const isTestingCompatibility = ref(false);
const compatibilityResult = ref<any>(null);

async function testCompatibility() {
  isTestingCompatibility.value = true;
  compatibilityResult.value = null;
  try {
    const res = await invoke<any>("test_api_compatibility", {
      apiKey: apiConfig.value.api_key,
      baseUrl: apiConfig.value.base_url,
      model: apiConfig.value.model,
      thinkingDepth: apiConfig.value.thinking_depth,
    });
    compatibilityResult.value = res;
  } catch (e: any) {
    compatibilityResult.value = {
      http_ok: false,
      errors: [e.toString()]
    };
  } finally {
    isTestingCompatibility.value = false;
  }
}

function clearFetchedModels() {
  fetchedModels.value = [];
  selectedFetchedModel.value = "";
  modelFetchResult.value = { success: false, message: "" };
}

async function fetchApiModels() {
  if (!apiConfig.value.base_url.trim() || isFetchingModels.value) return;
  isFetchingModels.value = true;
  modelFetchResult.value = { success: false, message: "" };
  try {
    const models = await invoke<string[]>("list_api_models", {
      apiKey: apiConfig.value.api_key,
      baseUrl: apiConfig.value.base_url,
    });
    fetchedModels.value = models;
    selectedFetchedModel.value = models.includes(apiConfig.value.model)
      ? apiConfig.value.model
      : (models[0] || "");
    if (selectedFetchedModel.value) {
      apiConfig.value.model = selectedFetchedModel.value;
      if (!apiConfig.value.name.trim() || apiConfig.value.name === "gpt-4o-mini") {
        apiConfig.value.name = selectedFetchedModel.value;
      }
    }
    modelFetchResult.value = {
      success: true,
      message: `已获取 ${models.length} 个模型，请选择后加入列表。`,
    };
  } catch (e: any) {
    fetchedModels.value = [];
    selectedFetchedModel.value = "";
    modelFetchResult.value = { success: false, message: e.toString() };
  } finally {
    isFetchingModels.value = false;
  }
}

function selectFetchedModel(model: string) {
  selectedFetchedModel.value = model;
  apiConfig.value.model = model;
  if (!apiConfig.value.id && !apiConfig.value.name.trim()) {
    apiConfig.value.name = model;
  }
}

async function addFetchedModelProfile() {
  const model = selectedFetchedModel.value || apiConfig.value.model;
  if (!model || isAddingFetchedModel.value) return;
  isAddingFetchedModel.value = true;
  try {
    apiConfig.value.model = model;
    const profileName = apiConfig.value.id ? model : (apiConfig.value.name.trim() || model);
    await saveApiConfig({
      quiet: true,
      profileId: "",
      profileName,
      model,
    });
    modelFetchResult.value = { success: true, message: `已将 ${model} 加入模型列表。` };
  } catch (e: any) {
    modelFetchResult.value = { success: false, message: "加入模型列表失败: " + e };
  } finally {
    isAddingFetchedModel.value = false;
  }
}

function newApiProfileDraft() {
  apiConfig.value = {
    id: "",
    name: "",
    api_key: "",
    api_key_mask: "",
    has_api_key: false,
    base_url: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    confirm_enabled: true,
    thinking_depth: "auto",
    execution_mode: "normal",
    search_provider: "bing",
    auto_approved_tools: [],
    stream_mode: "auto",
    api_mode: "chat_completions",
  };
  testResult.value = { success: false, message: "" };
  clearFetchedModels();
}

async function activateApiProfile(id: string, options: { switchBackend?: boolean } = {}) {
  if (!id) return;
  try {
    const profile = await invoke<ApiProfile>("set_active_api_profile", { id });
    applyApiConfig(profile);
    clearFetchedModels();
    testResult.value = { success: false, message: "" };
    compatibilityResult.value = null;
    currentModel.value = `直连 API: ${apiConfig.value.model}`;
    await currentWindow.emit("api-config-changed");
    // 聊天栏切模型 = 要用它聊天，顺手切到直连 API；设置页里点配置只是编辑，不改后端。
    if (options.switchBackend !== false && backendType.value !== "direct_api") {
      await selectBackend("direct_api");
    }
  } catch (e) {
    alert("切换模型配置失败: " + e);
  }
}

async function deleteApiProfile(id: string) {
  if (!id) return;
  profileDeletingId.value = id;
  try {
    await invoke("delete_api_profile", { id });
    await loadApiProfiles();
    const config = await invoke<any>("get_api_config");
    applyApiConfig(config);
    currentModel.value = backendType.value === "direct_api" ? `直连 API: ${apiConfig.value.model}` : currentModel.value;
    await currentWindow.emit("api-config-changed");
  } catch (e) {
    alert("删除模型配置失败: " + e);
  } finally {
    profileDeletingId.value = "";
  }
}

async function resetModel() {
  // switch_model 会清空当前主对话的消息并断开 CLI 续接，属于不可恢复操作。
  if (!window.confirm("重置会清空当前主对话的全部消息，并让本机 CLI 重新开始一段会话。确定继续吗？")) return;
  try {
    await invoke("switch_model");
    resetDashChatUi();
    await loadMemories();
    await currentWindow.emit("chat-history-cleared");
    await loadCurrentModel();
    modelSaved.value = true;
    setTimeout(() => (modelSaved.value = false), 2000);
  } catch (e) { alert("重置失败: " + e); }
}

async function startClaudeConfig() {
  try { await invoke("open_claude_config"); } catch (e) { alert("启动配置终端失败: " + e); }
}

async function selectSkin(skinId: string) {
  try {
    await invoke("set_skin", { skin: skinId });
    currentSkin.value = skinId;
    pet.skin = skinId;
    // 通知桌宠窗口
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) { alert("皮肤切换失败: " + e); }
}

async function selectCharacter(characterId: PetCharacterId) {
  try {
    if (characterId === "custom-pixel") {
      const active = await getActiveCustomPixelPetAsset();
      if (!active) {
        alert("请先上传生成一个自定义像素形象。");
        return;
      }
      pet.customPixelPetAsset = active;
    }
    await invoke("set_setting_value", {
      key: PET_CHARACTER_SETTING_KEY,
      value: characterId,
    });
    currentCharacter.value = characterId;
    pet.character = characterId;
    await notifyPetAppearanceChanged();
  } catch (e) { alert("本体形象切换失败: " + e); }
}

function onCustomPixelSelected() {
  currentCharacter.value = pet.character;
}

async function selectFontColor(value: string) {
  try {
    await invoke("set_font_color", { fontColor: value });
    currentFontColor.value = value;
    pet.fontColor = value;
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) { alert("字体颜色保存失败: " + e); }
}

function chooseAvatar() { avatarInputRef.value?.click(); }

async function onAvatarSelected(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file || !file.type.startsWith("image/")) { alert("请选择图片文件"); return; }
  const avatar = await readFile(file);
  try {
    await invoke("set_user_avatar", { avatar });
    pet.userAvatar = avatar;
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) { alert("头像保存失败: " + e); }
}

async function clearAvatar() {
  try {
    await invoke("set_user_avatar", { avatar: "" });
    pet.userAvatar = "";
  } catch (e) { alert("头像清除失败: " + e); }
}

function readFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ""));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function selectBg(bg: string) {
  chatBg.value = bg;
  localStorage.setItem(BG_KEY, bg);
}

function chooseBgImage() { bgInputRef.value?.click(); }

function onBgImageSelected(event: Event) {
  const el = event.target as HTMLInputElement;
  const file = el.files?.[0];
  el.value = "";
  if (!file || !file.type.startsWith("image/")) return;
  const reader = new FileReader();
  reader.onload = () => {
    const url = String(reader.result || "");
    customBgImage.value = url;
    localStorage.setItem(CUSTOM_BG_KEY, url);
    chatBg.value = "custom";
    localStorage.setItem(BG_KEY, "custom");
  };
  reader.readAsDataURL(file);
}

function clearCustomBg() {
  customBgImage.value = "";
  localStorage.removeItem(CUSTOM_BG_KEY);
  selectBg("none");
}

async function selectPersonality(personality: string) {
  try { await invoke("set_personality", { personality }); currentPersonality.value = personality; }
  catch (e) { alert("性格切换失败: " + e); }
}

async function selectProfession(profession: string) {
  try { await invoke("set_profession", { profession }); currentProfession.value = profession; }
  catch (e) { alert("职业切换失败: " + e); }
}

async function saveMemoryDraft() {
  const key = memoryDraft.value.key.trim();
  const value = memoryDraft.value.value.trim();
  if (!key || !value || isSavingMemory.value) return;

  isSavingMemory.value = true;
  memorySaved.value = false;
  try {
    const saved = await invoke<MemoryItem>("save_memory", {
      category: memoryDraft.value.category || "manual",
      key,
      value,
    });
    memories.value = [saved, ...memories.value.filter((item) => item.id !== saved.id)];
    memoryDraft.value.key = "";
    memoryDraft.value.value = "";
    memorySaved.value = true;
    setTimeout(() => { memorySaved.value = false; }, 2000);
  } catch (e) {
    alert("保存记忆失败: " + e);
  } finally {
    isSavingMemory.value = false;
  }
}

async function deleteMemoryItem(id: number) {
  if (memoryDeletingId.value !== null) return;
  const item = memories.value.find((memory) => memory.id === id);
  const confirmed = window.confirm(`确定删除记忆“${item?.key || "未命名"}”吗？`);
  if (!confirmed) return;
  memoryDeletingId.value = id;
  try {
    await invoke("delete_memory", { id });
    // Optimistic update
    memories.value = memories.value.filter((item) => item.id !== id);
    // Verify deletion by reloading from backend
    const fresh = await invoke<MemoryItem[]>("get_memories");
    const stillExists = fresh.some((m) => m.id === id);
    if (stillExists) {
      alert("删除未生效，请重试");
    }
    memories.value = fresh;
  } catch (e) {
    alert("删除记忆失败: " + e);
  } finally {
    memoryDeletingId.value = null;
  }
}

function memoryCategoryLabel(category: string) {
  const map: Record<string, string> = {
    manual: "手动",
    conversation: "对话摘要",
    general: "通用",
  };
  return map[category] || category;
}

function pad2(value: number) {
  return String(value).padStart(2, "0");
}

function toDatetimeLocalValue(date: Date) {
  return [
    date.getFullYear(),
    pad2(date.getMonth() + 1),
    pad2(date.getDate()),
  ].join("-") + `T${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
}

function resetTaskDraftTime() {
  const date = new Date(Date.now() + 60 * 60 * 1000);
  date.setMinutes(0, 0, 0);
  taskDraft.value.dueAtLocal = toDatetimeLocalValue(date);
}

function parseTaskDueAt() {
  if (!taskDraft.value.dueAtLocal) return 0;
  const time = new Date(taskDraft.value.dueAtLocal).getTime();
  return Number.isFinite(time) ? Math.floor(time / 1000) : 0;
}

async function saveTaskDraft() {
  const title = taskDraft.value.title.trim();
  const dueAt = parseTaskDueAt();
  taskError.value = "";
  if (!title || !dueAt || isSavingTask.value) return;
  if (dueAt <= Math.floor(Date.now() / 1000) + 5) {
    taskError.value = "提醒时间需要晚于当前时间。";
    return;
  }

  isSavingTask.value = true;
  taskSaved.value = false;
  try {
    const saved = await invoke<ScheduledTask>("save_scheduled_task", {
      title,
      note: taskDraft.value.note.trim(),
      dueAt,
      repeat: taskDraft.value.repeat,
      enabled: true,
    });
    scheduledTasks.value = [
      saved,
      ...scheduledTasks.value.filter((item) => item.id !== saved.id),
    ].sort(compareScheduledTasks);
    taskDraft.value.title = "";
    taskDraft.value.note = "";
    taskDraft.value.repeat = "once";
    resetTaskDraftTime();
    taskSaved.value = true;
    setTimeout(() => { taskSaved.value = false; }, 2000);
  } catch (e) {
    taskError.value = `保存定时任务失败: ${e}`;
  } finally {
    isSavingTask.value = false;
  }
}

async function toggleScheduledTask(task: ScheduledTask) {
  if (taskTogglingId.value !== null) return;
  taskTogglingId.value = task.id;
  taskError.value = "";
  try {
    const updated = await invoke<ScheduledTask>("set_scheduled_task_enabled", {
      id: task.id,
      enabled: !task.enabled,
    });
    scheduledTasks.value = scheduledTasks.value
      .map((item) => item.id === updated.id ? updated : item)
      .sort(compareScheduledTasks);
  } catch (e) {
    taskError.value = `更新定时任务失败: ${e}`;
  } finally {
    taskTogglingId.value = null;
  }
}

async function deleteScheduledTask(id: number) {
  if (taskDeletingId.value !== null) return;
  const task = scheduledTasks.value.find((item) => item.id === id);
  const confirmed = window.confirm(`确定删除提醒“${task?.title || "未命名"}”吗？`);
  if (!confirmed) return;
  taskDeletingId.value = id;
  taskError.value = "";
  try {
    await invoke("delete_scheduled_task", { id });
    scheduledTasks.value = scheduledTasks.value.filter((item) => item.id !== id);
  } catch (e) {
    taskError.value = `删除定时任务失败: ${e}`;
  } finally {
    taskDeletingId.value = null;
  }
}

function compareScheduledTasks(a: ScheduledTask, b: ScheduledTask) {
  if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
  return a.due_at - b.due_at || b.id - a.id;
}

function taskRepeatLabel(repeat: string) {
  const map: Record<string, string> = {
    once: "一次",
    daily: "每天",
    weekly: "每周",
    monthly: "每月",
  };
  return map[repeat] || repeat;
}

function formatTaskTime(timestamp: number) {
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) return "时间无效";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function formatTaskFullTime(timestamp: number) {
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) return "时间无效";
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function taskStatusLabel(task: ScheduledTask) {
  if (!task.enabled) return "已暂停";
  const delta = task.due_at - Math.floor(Date.now() / 1000);
  if (delta <= 0) return "即将提醒";
  if (delta < 3600) return `${Math.ceil(delta / 60)} 分钟后`;
  if (delta < 86400) return `${Math.ceil(delta / 3600)} 小时后`;
  return `${Math.ceil(delta / 86400)} 天后`;
}

async function updateTtsSettings(patch: Partial<TtsSettings>) {
  ttsSettings.value = { ...ttsSettings.value, ...patch };
  try {
    await invoke("set_setting_value", { key: TTS_SETTINGS_KEY, value: JSON.stringify(ttsSettings.value) });
    window.dispatchEvent(new CustomEvent("tts-settings-changed"));
    await currentWindow.emit("tts-settings-changed");
  } catch {}
}

async function previewVoice() {
  // 用真实播放状态判断"是否在播"，而不是自己记一个等异步返回的标志。
  if (isSpeaking.value) { ttsPlayer.stop(); return; }
  try { await ttsPlayer.speak("你好，我是你的桌宠伙伴，很高兴认识你。", ttsSettings.value); }
  catch (e: any) { if (e.message !== "Aborted") alert("试听失败: " + (e.message || e)); }
}

async function speakDashboardReply(text: string) {
  if (!voiceSettings.value.enabled || !voiceSettings.value.autoSpeak) return;
  try {
    await ttsPlayer.speak(text, ttsSettings.value);
  } catch (e: any) {
    if (e?.message !== "Aborted") {
      console.warn("Dashboard auto speech failed:", e);
    }
  }
}

async function updateVoiceSettings(patch: Partial<VoiceSettings>) {
  voiceSettings.value = { ...voiceSettings.value, ...patch };
  try {
    await invoke("set_setting_value", { key: VOICE_SETTINGS_KEY, value: serializeVoiceSettings(voiceSettings.value) });
    window.dispatchEvent(new CustomEvent("voice-settings-changed"));
    await currentWindow.emit("voice-settings-changed");
  } catch (e) { alert("语音设置保存失败: " + e); }
}

// === 陪伴动作 / 对话操作 ===
function flashActiveAction(actionId: string) {
  activeActionId.value = actionId;
  if (actionFeedbackTimer) clearTimeout(actionFeedbackTimer);
  actionFeedbackTimer = setTimeout(() => { activeActionId.value = ""; }, 1600);
}

/**
 * 陪伴动作：只改桌宠状态。演出交给首页舞台上的 PetCanvas（live 模式会轮询回显），
 * 所以这里不再打印"已触发…"这类文字反馈，不要让文案去描述用户看不到的动作。
 */
const PET_ACTION_STATES: Record<string, string> = {
  wave: "waving", happy: "happy", sleep: "sleeping", wake: "idle",
};

async function companionAction(name: string) {
  const state = PET_ACTION_STATES[name];
  if (!state) return;
  flashActiveAction(name);
  try {
    await invoke("set_pet_state", { newState: state });
  } catch (err) {
    console.error("设置桌宠状态失败", err);
  }
}

/** 对话操作：复用已有的带确认/带错误上报实现，避免与此前重复的那份逻辑分叉 */
async function conversationAction(name: string) {
  if (name === "new-chat") {
    await startDashboardNewConversation();
    return;
  }
  if (name === "clear") {
    await clearDashboardChatWithConfirm();
  }
}

// === 窗口控制 ===
async function minimizeWindow() { await currentWindow.minimize(); }
async function closeWindow() { await currentWindow.hide(); }
async function exitApp() { await invoke("exit_app"); }

function cpuBarColor(cpu: number): string {
  if (cpu > 80) return "#ef4444";
  if (cpu > 50) return "#f59e0b";
  return "var(--pet-primary, #ff6b6b)";
}

function memBarColor(mem: number): string {
  if (mem > 85) return "#ef4444";
  if (mem > 60) return "#f59e0b";
  return "var(--pet-accent, #ffa07a)";
}

// === 窗口最大化/还原控制 ===
const isMaximized = ref(false);
async function toggleMaximize() {
  const maximized = await currentWindow.isMaximized();
  if (maximized) {
    await currentWindow.unmaximize();
    isMaximized.value = false;
  } else {
    await currentWindow.maximize();
    isMaximized.value = true;
  }
}

// === 对话框与消息同步 ===
const chatInput = ref("");
const dashChatInputRef = ref<HTMLInputElement | null>(null);
const dashSlashSelectedIndex = ref(0);
const dashMentionSelectedIndex = ref(0);
const thinkingContent = ref("");
const streamingAnswer = ref("");
const toolEvents = ref<ToolEvent[]>([]);
const pendingConfirm = ref<ToolConfirmPayload | null>(null);
// === 智能体主动询问弹窗 (ask_user) ===
const pendingQuestion = ref<AskUserPayload | null>(null);
const confirmSubmission = createInteractionSubmission(pendingConfirm, (id, value) => invoke("confirm_tool", { id, approved: value }));
const confirmSubmitting = computed(() => confirmSubmission.submittingId.value === pendingConfirm.value?.id);
const dashChatMessagesRef = ref<HTMLDivElement | null>(null);
const dashChatEndRef = ref<HTMLDivElement | null>(null);
const followingActivity = ref(true);
function trackChatScroll() {
  if (dashChatMessagesRef.value) followingActivity.value = isNearBottom(dashChatMessagesRef.value);
}
function jumpToLatest() {
  followingActivity.value = true;
  scrollDashChatToBottom();
}
let dashChatScrollFrame: number | null = null;
let dashChatScrollTimers: ReturnType<typeof setTimeout>[] = [];

let unlistenThinking: UnlistenFn | null = null;
let unlistenAnswerDelta: UnlistenFn | null = null;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;
let unlistenChatCleared: UnlistenFn | null = null;
let unlistenSkillActivated: UnlistenFn | null = null;
let unlistenSyncMessage: UnlistenFn | null = null;
let unlistenToolEvent: UnlistenFn | null = null;
let stopToolConfirm: (() => void) | undefined;
let stopQuestion: (() => void) | undefined;
let unlistenScheduledTasksChanged: UnlistenFn | null = null;
let unlistenScheduledTaskTriggered: UnlistenFn | null = null;
let unlistenWeatherUpdate: UnlistenFn | null = null;
let unlistenDragDrop: UnlistenFn | null = null;
let unlistenVoiceSettingsChanged: UnlistenFn | null = null;
let unlistenTtsSettingsChanged: UnlistenFn | null = null;
// === 对话状态：agent 与同屏视图共享运行时；主对话保留旧事件兼容状态 ===
type AgentTurn = SessionTurn;
type MainTurn = {
  loading: boolean;
  thinking: string;
  streamingAnswer: string;
  toolEvents: ToolEventItem[];
  activity: ReturnType<typeof createAgentActivity>;
};
const mainTurns = reactive<Record<string, MainTurn>>({});
/** 主对话当前已载入的会话 id：从其它页面切回来时据此判断要不要重载。 */
const loadedMainSessionId = ref("");
let agentPresentationStorage: Storage | undefined;
try { agentPresentationStorage = globalThis.localStorage; } catch { /* WebView 可能禁用 storage。 */ }

function ensureAgentMessages(sessionId: string): Message[] {
  return sessionRuntime.ensure(sessionId).messages;
}
function ensureAgentTurn(sessionId: string): AgentTurn {
  return sessionRuntime.ensure(sessionId).turn;
}
function ensureMainTurn(sessionId: string): MainTurn {
  if (!mainTurns[sessionId]) {
    mainTurns[sessionId] = {
      loading: false,
      thinking: "",
      streamingAnswer: "",
      toolEvents: [],
      activity: createAgentActivity(reactive<AgentActivityItem[]>([])),
    };
  }
  return mainTurns[sessionId];
}
function snapshotActiveMainTurn(sessionId = loadedMainSessionId.value || sessions.activeMainId) {
  if (!sessionId) return;
  if (
    !chat.isLoading &&
    !thinkingContent.value &&
    !streamingAnswer.value &&
    !toolEvents.value.length &&
    !chat.activity.items.length
  ) {
    delete mainTurns[sessionId];
    return;
  }
  const turn = ensureMainTurn(sessionId);
  turn.loading = chat.isLoading;
  turn.thinking = thinkingContent.value;
  turn.streamingAnswer = streamingAnswer.value;
  turn.toolEvents = JSON.parse(JSON.stringify(toolEvents.value));
  turn.activity.items.splice(
    0,
    turn.activity.items.length,
    ...JSON.parse(JSON.stringify(chat.activity.items)),
  );
}
function settleBackgroundMainTurn(
  sessionId: string,
  content: string,
  options: { thinking?: string; agent?: MessageAgent | null; reason?: "completed" | "failed" | "aborted" } = {},
) {
  const turn = mainTurns[sessionId];
  if (turn && turn.activity.items.length) {
    const presentation = turn.activity.finish(content, options.reason ?? "completed");
    saveActivityPresentation(agentPresentationStorage, {
      content,
      thinking: options.thinking,
      timestamp: Date.now(),
      agent: options.agent ?? null,
      ...presentation,
    });
  }
  delete mainTurns[sessionId];
}
function isSessionBusy(sessionId: string): boolean {
  if (!sessionId) return false;
  const runtime = sessionRuntime.peek(sessionId);
  if (runtime) return runtime.turn.loading;
  if (sessionId === sessions.activeMainId && chat.isLoading) return true;
  return !!mainTurns[sessionId]?.loading;
}
function clearAgentSessionState(sessionId: string) {
  if (!sessionId) return;
  sessionRuntime.remove(sessionId);
  delete mainTurns[sessionId];
}
function clearAllAgentSessionState() {
  sessionRuntime.clear();
  for (const key of Object.keys(mainTurns)) delete mainTurns[key];
  for (const slot of AGENT_SLOTS) rememberAgentSession(slot, "");
}

// 当前窗格显示的会话绑定。
const currentAgentSession = computed<ChatSession | undefined>(() => {
  const slot = agentSlot.value;
  if (!slot) return undefined;
  const storedId = agentSessionIds[slot];
  if (storedId) {
    const stored = sessions.find(storedId);
    if (stored && stored.slot === slot) return stored;
  }
  return sessions.bySlot(slot)[0];
});
const currentAgentSessionId = computed(() => currentAgentSession.value?.id ?? "");
const currentSessionId = computed(() => (isMainSlot.value ? sessions.activeMainId : currentAgentSessionId.value));
const currentSession = computed<ChatSession | undefined>(() => {
  if (isMainSlot.value) return sessions.find(sessions.activeMainId) ?? sessions.activeMain();
  return currentAgentSession.value;
});
// 有 sessionId 的两种布局都显示同一个运行时；旧版无 session_id 事件沿用主对话兼容状态。
const currentRuntime = computed(() => currentSessionId.value ? sessionRuntime.peek(currentSessionId.value) : undefined);
const paneMessages = computed<Message[]>(() => currentRuntime.value?.messages ?? (isMainSlot.value ? chat.messages : []));
const paneIsLoading = computed(() => currentRuntime.value?.turn.loading ?? (isMainSlot.value ? chat.isLoading : false));
const paneThinking = computed(() => currentRuntime.value?.turn.thinking ?? (isMainSlot.value ? thinkingContent.value : ""));
const paneActivityItems = computed<AgentActivityItem[]>(() => currentRuntime.value?.turn.activity.items ?? (isMainSlot.value ? chat.activity.items : []));
const paneSessions = computed<ChatSession[]>(() =>
  isMainSlot.value ? sessions.bySlot("main") : agentSlot.value ? sessions.bySlot(agentSlot.value) : [],
);
const activeToolItems = computed(() => paneActivityItems.value.filter((item) => item.kind === "tool"));
const paneActivityHeadline = computed(() => {
  const last = activeToolItems.value[activeToolItems.value.length - 1];
  if (last && last.kind === "tool") {
    return (last.tool.summary || last.tool.tool_name) + " · " + toolStatusLabel(last.tool.status);
  }
  return "正在理解你的需求";
});
const currentSlotLabel = computed(() => (agentSlot.value ? SLOT_WINDOW_LABELS[agentSlot.value] : "主对话"));
const currentAgentIdentity = computed<{ id: string; name: string; avatar: string }>(() => {
  const slot = agentSlot.value;
  if (!slot) return { id: "", name: "", avatar: "" };
  const session = currentAgentSession.value;
  const agentId = session?.agent_id || agentWindowPrefs[slot].agentId || SLOT_DEFAULT_AGENT[slot];
  const agent = agentsStore.agents.find((item) => item.id === agentId);
  return {
    id: agentId,
    name: session?.agent_name || agent?.name || SLOT_WINDOW_LABELS[slot],
    avatar: session?.agent_avatar || agent?.avatar || SLOT_WINDOW_ICONS[slot],
  };
});
/** agent 窗口当前绑定的后端：权限选择器按后端展示不同档位。 */
const currentAgentBackend = computed(() => {
  const agent = agentsStore.agents.find((item) => item.id === currentAgentIdentity.value.id);
  if (agent) return agent.backend;
  return agentSlot.value === "codex"
    ? "codex"
    : agentSlot.value === "dsh"
      ? "dsh"
      : agentSlot.value === "claude"
        ? "claude_code"
        : "";
});
/** 多窗格看板：会话所属后端（主会话跟随全局后端）。 */
function paneSessionBackend(sessionId: string): string {
  const session = sessions.find(sessionId);
  if (!session) return "";
  if (session.slot === "main") return backendType.value;
  const agent = agentsStore.agents.find((item) => item.id === session.agent_id);
  if (agent) return agent.backend;
  return session.slot === "codex"
    ? "codex"
    : session.slot === "dsh"
      ? "dsh"
      : session.slot === "claude"
        ? "claude_code"
        : "";
}
const agentWindowAgentOptions = computed<{ id: string; name: string; avatar: string; description?: string }[]>(() => {
  const options: { id: string; name: string; avatar: string; description?: string }[] = agentsStore.agents.map((agent) => ({ id: agent.id, name: agent.name, avatar: agent.avatar, description: agent.description }));
  const current = currentAgentIdentity.value;
  if (current.id && !options.some((item) => item.id === current.id)) options.unshift({ ...current });
  if (!options.length && agentSlot.value) {
    const slot = agentSlot.value;
    return [{ id: SLOT_DEFAULT_AGENT[slot], name: SLOT_WINDOW_LABELS[slot], avatar: SLOT_WINDOW_ICONS[slot] }];
  }
  return options;
});
const currentAgentCatalog = computed(() =>
  agentSlot.value ? agentCatalogs[agentSlot.value] : null,
);
/** 顶部模型选择器显示的值：有会话取会话绑定，无会话取窗口偏好。 */
const currentAgentModelValue = computed(() => {
  const session = currentAgentSession.value;
  if (session) return session.model || "";
  return agentSlot.value ? agentWindowPrefs[agentSlot.value].model : "";
});
const currentAgentProviderValue = computed(() => {
  const session = currentAgentSession.value;
  if (session) return session.provider || "";
  return agentSlot.value ? agentWindowPrefs[agentSlot.value].provider : "";
});
/** 底部信息条：用目录把 model id 解析成展示名，再展示渠道（如 claude-opus-5（opendesgin））。 */
const currentSessionModelLabel = computed(() => {
  const model = currentAgentModelValue.value;
  if (!model) return "跟随本机";
  return modelOptionFor(currentAgentCatalog.value, model, currentAgentProviderValue.value)?.name || model;
});
const currentSessionChannelLabel = computed(() => {
  if (!currentAgentModelValue.value) return "";
  return providerLabel(currentAgentCatalog.value, currentAgentProviderValue.value);
});
const currentAgentEffortValue = computed(() => {
  const session = currentAgentSession.value;
  const sessionEffort = session?.reasoning_effort;
  if (session) return typeof sessionEffort === "string" ? sessionEffort : "";
  return agentSlot.value ? agentWindowPrefs[agentSlot.value].effort : "";
});
const currentSessionEffortLabel = computed(() =>
  currentAgentEffortValue.value ? effortLabel(currentAgentEffortValue.value) : "",
);

// 进入 / 切换 agent 窗口时预取模型目录（service 自带缓存），修复重启后 pill/信息条先显示原始 id 的问题。
watch(
  [activePage, agentSlot],
  ([page, slot]) => {
    if (page === "chat" && slot) void ensureAgentCatalog(slot);
  },
  { immediate: true },
);

/** 某个 agent 窗口当前是否有回合在跑（侧边栏子项的运行指示灯）。 */
function agentSlotBusy(slot: AgentWindowSlot): boolean {
  const sessionId = agentSessionIds[slot];
  if (sessionId && sessionRuntime.peek(sessionId)?.turn.loading) return true;
  return sessions.bySlot(slot).some((item) => !!sessionRuntime.peek(item.id)?.turn.loading);
}

/** 历史消息（Rust ChatMessage / mock ChatMessage）转成前端 MessageDraft。 */
function historyToMessageDraft(item: any): Omit<Message, "id"> {
  return {
    role: item?.role === "assistant" ? "assistant" : "user",
    content: item?.content || (item?.quoted_content ? "（引用消息）" : item?.content),
    thinking: item?.thinking || undefined,
    timestamp: typeof item?.created_at === "number" ? item.created_at : new Date(item?.created_at).getTime(),
    quote: item?.quoted_content
      ? {
          role: item?.quoted_role === "assistant" ? "assistant" as const : "user" as const,
          content: item.quoted_content,
        }
      : undefined,
    agent: item?.agent_name
      ? {
          id: item.agent_id || undefined,
          name: item.agent_name,
          avatar: item.agent_avatar || undefined,
        }
      : undefined,
  };
}

/** 事件路由：旧 payload / 空 session_id 回退主对话；非活跃主会话走 main-bg，agent 会话按 slot 分流。 */
type RoutedSession =
  | { kind: "main" }
  | { kind: "main-bg"; sessionId: string }
  | { kind: "agent"; slot: AgentWindowSlot; sessionId: string }
  | { kind: "ignore" };
function routeIncomingSession(sessionId: string | undefined | null): RoutedSession {
  if (!sessionId) return { kind: "main" };
  if (sessionId === sessions.activeMainId) return { kind: "main" };
  const session = sessions.find(sessionId);
  if (session && session.slot !== "main") {
    return { kind: "agent", slot: session.slot, sessionId };
  }
  return { kind: "main-bg", sessionId };
}
/** collaboration-* 只服务活跃主会话，agent 窗口的回合不显示协同条。 */
function isMainScopedPayload(sessionId: string | undefined | null): boolean {
  return !sessionId || sessionId === sessions.activeMainId;
}

/** 替代 services/collaboration 的订阅：按 session_id 过滤，避免 agent 窗口回合污染主对话协同条。 */
function subscribeDashboardCollaboration(): () => void {
  let stopped = false;
  const listeners: UnlistenFn[] = [];
  let observedEvent = false;
  const handleReply = (reply: CollaborationReply & { session_id?: string }) => {
    if (stopped || !isMainScopedPayload(reply.session_id)) return;
    if (!chat.receiveCollaborationReply(reply)) return;
    const last = chat.messages[chat.messages.length - 1];
    if (last && toolEvents.value.length) last.toolEvents = JSON.parse(JSON.stringify(toolEvents.value));
    thinkingContent.value = "";
    streamingAnswer.value = "";
    toolEvents.value = [];
    pendingConfirm.value = null;
    pendingQuestion.value = null;
    if (!reply.failed) void speakDashboardReply(speakableText(reply.text));
    void scrollDashChatToBottom();
    sessions.scheduleReload();
  };
  const handleState = (state: CollaborationState & { session_id?: string }) => {
    if (stopped || !isMainScopedPayload(state.session_id)) return;
    const newTurn = chat.collaboration?.run_id !== state.run_id || chat.collaboration?.turn !== state.turn;
    if (!chat.receiveCollaborationState(state)) return;
    if (newTurn || state.status !== "running") {
      thinkingContent.value = "";
      streamingAnswer.value = "";
      toolEvents.value = [];
      if (state.status !== "running") {
        pendingConfirm.value = null;
        pendingQuestion.value = null;
      }
    }
    if (state.status === "running") resetLoadingTimeout();
    else clearLoadingTimeout();
  };
  void (async () => {
    for (const register of [
      () => listen<CollaborationState & { session_id?: string }>("collaboration-state", ({ payload }) => {
        observedEvent = true;
        handleState(payload);
      }),
      () => listen<CollaborationReply & { session_id?: string }>("collaboration-reply", ({ payload }) => {
        handleReply(payload);
      }),
    ]) {
      const unlisten = await register();
      if (stopped) { unlisten(); return; }
      listeners.push(unlisten);
    }
    if (stopped || observedEvent) return;
    const current = await invoke<(CollaborationState & { session_id?: string }) | null>("get_collaboration_state");
    if (!stopped && current) handleState(current);
  })().catch((error) => console.error("加载协同状态失败", error));
  return () => { stopped = true; listeners.splice(0).forEach((unlisten) => unlisten()); };
}

async function loadAgentSessionMessages(sessionId: string, force = false): Promise<void> {
  await sessionRuntime.load(sessionId, force);
}

async function ensureAgentWindow(slot: AgentWindowSlot): Promise<void> {
  if (!sessions.bySlot(slot).length) return;
  const stored = sessions.find(agentSessionIds[slot]);
  const session = stored && stored.slot === slot ? stored : sessions.bySlot(slot)[0];
  if (!session) return;
  if (agentSessionIds[slot] !== session.id) rememberAgentSession(slot, session.id);
  await loadAgentSessionMessages(session.id);
  scrollDashChatToBottom();
}

async function ensureCurrentChatPane() {
  if (chatSlot.value === "main") {
    const stale = loadedMainSessionId.value !== sessions.activeMainId;
    if (sessions.activeMainId) await sessionRuntime.load(sessions.activeMainId);
    if (sessions.activeMainId && stale) await refreshChatState();
  } else {
    await ensureAgentWindow(chatSlot.value);
  }
  scrollDashChatToBottom();
}

// 主会话被其它窗口切换 / 新建时刷新主对话历史。
watch(() => sessions.activeMainId, (id, previous) => {
  if (!id || id === previous) return;
  if (previous && loadedMainSessionId.value === previous) {
    snapshotActiveMainTurn(previous);
  }
  if (activePage.value === "chat" && isMainSlot.value) void refreshChatState();
});
// 切到 agent 窗口时惰性加载历史；会话列表异步就绪后自动补齐当前会话。
watch(chatSlot, (slot, previousSlot) => {
  if (previousSlot === "main") {
    snapshotActiveMainTurn();
  }
  closeDashMessageMenu();
  followingActivity.value = true;
  sessionError.value = "";
  if (slot === "main") void refreshChatState();
  else void ensureAgentWindow(slot);
});
watch(currentAgentSessionId, (id, previous) => {
  if (!id || id === previous) return;
  const slot = agentSlot.value;
  if (!slot) return;
  rememberAgentSession(slot, id);
  void loadAgentSessionMessages(id);
});
function openChatSessionFromList(session: ChatSession) {
  void switchChatSession(session);
}
async function createChatSession() {
  if (creatingSession.value || switchingSessionId.value) return;
  creatingSession.value = true;
  sessionError.value = "";
  try {
    const slot = agentSlot.value;
    if (!slot) {
      snapshotActiveMainTurn();
      const created = await sessions.create("main");
      if (!created || !created.id) throw new Error("后端未返回新会话");
      resetMainPaneTransientState();
      await refreshChatState();
      scrollDashChatToBottom();
      return;
    }
    const pref = agentWindowPrefs[slot];
    const created = await sessions.create(slot, {
      agentId: pref.agentId || SLOT_DEFAULT_AGENT[slot],
      model: pref.model,
      provider: pref.provider,
      reasoningEffort: pref.effort,
    });
    if (!created || !created.id) throw new Error("后端未返回新会话");
    rememberAgentSession(slot, created.id);
    sessionRuntime.ensure(created.id).loaded = true;
    ensureAgentTurn(created.id);
    await nextTick();
    scrollDashChatToBottom();
  } catch (error) {
    sessionError.value = "新建会话失败：" + errorMessage(error);
  } finally {
    creatingSession.value = false;
  }
}
async function switchChatSession(session: ChatSession) {
  if (!session || session.id === currentSessionId.value) return;
  if (sessions.loading || switchingSessionId.value) return;
  switchingSessionId.value = session.id;
  sessionError.value = "";
  try {
    if (session.slot === "main") {
      snapshotActiveMainTurn();
      await sessions.setActiveMain(session.id);
      resetMainPaneTransientState();
      await refreshChatState();
    } else {
      const slot = agentSlot.value;
      if (!slot) return;
      rememberAgentSession(slot, session.id);
      await loadAgentSessionMessages(session.id);
    }
    await nextTick();
    scrollDashChatToBottom();
  } catch (error) {
    sessionError.value = "切换会话失败：" + errorMessage(error);
  } finally {
    switchingSessionId.value = "";
  }
}
async function removeChatSession(session: ChatSession) {
  if (!session) return;
  const label = session.title || "未命名会话";
  if (!window.confirm("确定删除会话「" + label + "」吗？该会话的历史消息会一并删除，且不可撤销。")) return;
  sessionError.value = "";
  try {
    await sessions.remove(session.id);
    clearAgentSessionState(session.id);
    if (workspacePanes.value.includes(session.id)) {
      workspacePanes.value = workspacePanes.value.map((id) => (id === session.id ? "" : id));
      saveWorkspacePanes(workspacePanes.value);
    }
    if (renamingSessionId.value === session.id) renamingSessionId.value = null;
    if (session.slot === "main") {
      await sessions.load();
      if (loadedMainSessionId.value === session.id) {
        resetMainPaneTransientState();
      }
      await refreshChatState();
    } else if (agentSlot.value && agentSessionIds[agentSlot.value] === session.id) {
      rememberAgentSession(agentSlot.value, "");
      await ensureAgentWindow(agentSlot.value);
    }
    await nextTick();
    scrollDashChatToBottom();
  } catch (error) {
    sessionError.value = "删除会话失败：" + errorMessage(error);
  }
}
function startRenameChatSession(session: ChatSession) {
  renamingSessionId.value = session.id;
  renameDraft.value = session.title || "";
  void nextTick(() => {
    renameInputRef.value?.focus();
    renameInputRef.value?.select();
  });
}
function cancelRenameChatSession() {
  renamingSessionId.value = null;
}
function setRenameInputElement(element: unknown) {
  renameInputRef.value = (element as HTMLInputElement | null) ?? null;
}
async function commitRenameChatSession(session: ChatSession) {
  if (renamingSessionId.value !== session.id) return;
  const title = renameDraft.value.trim();
  renamingSessionId.value = null;
  if (!title || title === (session.title || "")) return;
  sessionError.value = "";
  try {
    await sessions.rename(session.id, title);
  } catch (error) {
    sessionError.value = "重命名失败：" + errorMessage(error);
  }
}

/**
 * 模型选择器回调：窗口偏好始终保存；有当前会话时立即写回会话绑定。
 * 模型 / 渠道变化后端会清空 CLI 续接 id，仅 effort 变化不动续接 id；
 * 因此只把真正变化的字段写回会话，避免多余清空。
 */
async function onAgentModelSelect(payload: { model: string; provider: string; effort: string }) {
  const slot = agentSlot.value;
  if (!slot) return;
  const model = (payload.model || "").trim();
  const provider = (payload.provider || "").trim();
  const effort = (payload.effort || "").trim();
  saveWindowPreference(slot, { model, provider, effort });
  const session = currentAgentSession.value;
  if (!session) return;
  const sessionEffort = typeof session.reasoning_effort === "string" ? session.reasoning_effort : "";
  const sessionProvider = session.provider || "";
  const patch: { model?: string; provider?: string; reasoningEffort?: string } = {};
  if (model !== (session.model || "")) patch.model = model;
  if (provider !== sessionProvider) patch.provider = provider;
  if (effort !== sessionEffort) patch.reasoningEffort = effort;
  if (patch.model === undefined && patch.provider === undefined && patch.reasoningEffort === undefined) return;
  sessionError.value = "";
  try {
    await sessions.bind(session.id, patch);
  } catch (error) {
    sessionError.value = "保存模型失败：" + errorMessage(error);
  }
}
async function onAgentWindowAgentChange(agentId: string) {
  const slot = agentSlot.value;
  if (!slot) return;
  const agent = agentsStore.agents.find((item) => item.id === agentId);
  saveWindowPreference(slot, { agentId });
  const session = currentAgentSession.value;
  if (!session) return;
  sessionError.value = "";
  try {
    if (agent) {
      await sessions.bind(session.id, { agentId: agent.id, agentName: agent.name, agentAvatar: agent.avatar, model: session.model || undefined });
    } else {
      await sessions.bind(session.id, { agentId, agentName: SLOT_WINDOW_LABELS[slot], agentAvatar: SLOT_WINDOW_ICONS[slot], model: session.model || undefined });
    }
  } catch (error) {
    sessionError.value = "切换智能体失败：" + errorMessage(error);
  }
}

async function ensureAgentSessionForSend(slot: AgentWindowSlot): Promise<ChatSession | null> {
  const existing = currentAgentSession.value;
  if (existing) return existing;
  const pref = agentWindowPrefs[slot];
  try {
    const created = await sessions.create(slot, {
      agentId: pref.agentId || SLOT_DEFAULT_AGENT[slot],
      model: pref.model,
      provider: pref.provider,
      reasoningEffort: pref.effort,
    });
    if (!created || !created.id) throw new Error("后端未返回新会话");
    rememberAgentSession(slot, created.id);
    ensureAgentMessages(created.id);
    sessionRuntime.ensure(created.id).loaded = true;
    ensureAgentTurn(created.id);
    return created;
  } catch (error) {
    sessionError.value = "创建会话失败：" + errorMessage(error);
    return null;
  }
}

/** agent 窗口发送：单 agent 回合（不走协同、不带 @ 提及）。 */
async function sendAgentWindowMessage() {
  const slot = agentSlot.value;
  if (!slot) return;
  const draftKey = attachmentDraftKey.value;
  const text = chatInput.value.trim();
  const files = [...pendingFiles.value];
  const quote = pendingQuote.value ? { ...pendingQuote.value } : undefined;
  if (!text && files.length === 0 && !quote) return;
  if (paneIsLoading.value || filesLoading.value) return;
  closeDashMessageMenu();
  if (voiceSettings.value.enabled && voiceSettings.value.autoSpeak) ttsPlayer.preparePlayback();
  const session = await ensureAgentSessionForSend(slot);
  if (!session) return;
  if (sessionRuntime.ensure(session.id).turn.loading) return;
  chatInput.value = "";
  pendingQuote.value = null;
  for (const file of files) attachments.remove(draftKey, file.id);
  followingActivity.value = true;
  await sessionRuntime.send(session.id, text, files, quote);
  scrollDashChatToBottom();
}

async function abortCurrentTurn() {
  const id = currentSessionId.value;
  if (id && sessionRuntime.peek(id)) {
    await sessionRuntime.abort(id);
    return;
  }
  await abortAi();
}

/** 主对话的多会话能力也走这里：后端清空所有会话后重置本地 agent 状态。 */
function resetAllChatSessionUi() {
  resetDashChatUi();
  loadedMainSessionId.value = "";
  clearAllAgentSessionState();
  void sessions.load().catch((error) => console.error("刷新会话列表失败", error));
}

let stopCollaboration: (() => void) | null = null;

// === 附件草稿按会话共享，单窗格与同屏窗格使用同一份文件列表 ===
const attachments = useAttachmentsStore();
const attachmentDraftKey = computed(() => currentSessionId.value || `slot:${chatSlot.value}`);
const attachmentDraft = computed(() => attachments.ensure(attachmentDraftKey.value));
const pendingFiles = computed(() => attachmentDraft.value.files);
const filesLoading = computed(() => pendingFiles.value.some((file) => file.status === "loading"));
const isFileOver = ref(false);
const fileOverPaneId = ref("");
const workspaceFileError = ref("");

async function pickDashboardFiles() {
  const key = attachmentDraftKey.value;
  await attachments.pick(key, (paths) => addToPendingFiles(paths, key));
}

async function addToPendingFiles(paths: string[], key = attachmentDraftKey.value) {
  const showFileError = (message: string) => { attachments.ensure(key).error = message; };
  const hasLnk = paths.some(p => p.toLowerCase().endsWith(".lnk"));
  if (hasLnk) {
    for (const p of paths) {
      if (p.toLowerCase().endsWith(".lnk")) {
        try {
          const res = await invoke<{ name: string; path: string }>("register_shortcut_file", { path: p });
          const successMsg = `已自动记住应用「${res.name}」的启动路径：\n\`${res.path}\`\n\n下次你可以对我说：“打开 ${res.name}”啦！`;
          
          chat.addMessage("assistant", successMsg);
          await currentWindow.emit("sync-chat-message", {
            role: "assistant",
            content: successMsg
          });
          
          try {
            await invoke("set_pet_state", { newState: "happy" });
          } catch {}
        } catch (err) {
          showFileError(`注册快捷方式失败: ${err}`);
        }
      } else {
        await attachments.add(key, [p]);
      }
    }
    return;
  }

  await attachments.add(key, paths);
}

function clearPendingFiles() {
  attachments.clear(attachmentDraftKey.value);
}

function handleDragDropEvent(event: any) {
  const hasLnk = event.type === "drop" && event.paths?.some((path: string) => path.toLowerCase().endsWith(".lnk"));
  const hovering = event.type === "enter" || event.type === "over";
  isFileOver.value = activePage.value === "chat" && hovering;
  fileOverPaneId.value = "";
  if (activePage.value !== "chat" && !hasLnk) return;

  if (activePage.value === "chat" && isWorkspaceMode.value) {
    const panes = Array.from(document.querySelectorAll<HTMLElement>(".dash-workspace-grid .session-pane")).map((element) => {
      const paneId = element.dataset.paneId || "";
      return { paneId, sessionId: workspacePanes.value[workspacePaneIds.value.indexOf(paneId)] || "", rect: element.getBoundingClientRect() };
    });
    const target = event.position ? attachmentDropTarget(event.position, window.devicePixelRatio || 1, panes) : null;
    if (hovering) {
      fileOverPaneId.value = target?.paneId || "";
      workspaceFileError.value = "";
    } else if (event.type === "drop" && event.paths?.length) {
      if (target?.sessionId) void addToPendingFiles(event.paths, target.sessionId);
      else workspaceFileError.value = target ? "先为此窗格选择会话，再添加文件" : "请将文件拖入要发送的会话窗格";
    }
    return;
  }
  if (event.type === "drop" && event.paths?.length) void addToPendingFiles(event.paths);
}

let refreshChatStatePromise: Promise<void> | null = null;
let refreshChatStateSession = "";
let refreshChatStateToken = 0;
/** 载入「活跃主会话」的历史；同会话的并发调用共享请求，切换会话时另起一次。 */
function refreshChatState(): Promise<void> {
  const sessionId = sessions.activeMainId || "";
  if (refreshChatStatePromise && refreshChatStateSession === sessionId) return refreshChatStatePromise;
  const token = ++refreshChatStateToken;
  refreshChatStateSession = sessionId;
  const promise = doRefreshChatState().finally(() => {
    if (refreshChatStateToken === token) refreshChatStatePromise = null;
  });
  refreshChatStatePromise = promise;
  return promise;
}
async function doRefreshChatState() {
  const sessionId = sessions.activeMainId;
  try {
    const [history, active] = await Promise.all([
      invoke<any[]>("get_chat_history", sessionId ? { sessionId } : {}),
      invoke<{ thinking?: string; session_id?: string } | null>("get_active_chat", sessionId ? { sessionId } : {}),
    ]);
    // 请求期间用户又切了主会话：丢弃旧响应。
    if (sessionId && sessions.activeMainId && sessions.activeMainId !== sessionId) return;
    if (sessionId) await sessionRuntime.load(sessionId);
    if (sessionId && sessionId !== sessions.activeMainId) return;
    chat.setMessages(Array.isArray(history) ? history.map(historyToMessageDraft) : []);
    loadedMainSessionId.value = sessions.activeMainId || sessionId;
    const cachedTurn = sessionId ? mainTurns[sessionId] : undefined;
    if (cachedTurn?.loading) {
      chat.isLoading = true;
      thinkingContent.value = cachedTurn.thinking;
      streamingAnswer.value = cachedTurn.streamingAnswer;
      toolEvents.value = JSON.parse(JSON.stringify(cachedTurn.toolEvents));
      chat.activity.items.splice(
        0,
        chat.activity.items.length,
        ...JSON.parse(JSON.stringify(cachedTurn.activity.items)),
      );
      resetLoadingTimeout();
    } else if (active && isMainScopedPayload(active.session_id) && !chat.activity.items.length) {
      chat.isLoading = true;
      thinkingContent.value = active.thinking || "";
      chat.activity.items.push(...legacyActivity(active.thinking));
      resetLoadingTimeout();
    } else {
      chat.isLoading = false;
      thinkingContent.value = "";
      streamingAnswer.value = "";
      toolEvents.value = [];
      chat.activity.reset();
    }
    scrollDashChatToBottom();
  } catch {}
}

function resetDashChatUi() {
  if (sessions.activeMainId) delete mainTurns[sessions.activeMainId];
  chat.clearMessages();
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;
  pendingQuestion.value = null;
}

/**
 * 切换/新建主会话时只清「进行中」的瞬时状态。
 * 不能走 resetDashChatUi：它会清掉 agentActivityCache，导致回看其它会话时轨迹快照丢失。
 */
function resetMainPaneTransientState() {
  clearLoadingTimeout();
  chat.isLoading = false;
  chat.activity.reset();
  chat.collaboration = null;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;
  pendingQuestion.value = null;
}

async function abortAi() {
  const targetSessionId = sessions.activeMainId || undefined;
  clearLoadingTimeout();
  if (chat.isLoading) {
    const message = chat.addMessage("assistant", "已中止", thinkingContent.value || undefined);
    chat.captureActivity(message, "aborted");
  }
  if (targetSessionId) delete mainTurns[targetSessionId];
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;  // Clear pending tool confirmation on abort
  pendingQuestion.value = null; // Clear pending ask_user popup on abort
  try {
    await invoke("abort_ai", { sessionId: targetSessionId });
  } catch {}
}

async function sendDashboardMessage() {
  // agent 发送与同屏窗格共用会话运行时。
  if (!isMainSlot.value) {
    await sendAgentWindowMessage();
    return;
  }

  const sessionId = sessions.activeMainId;
  const text = chatInput.value.trim();
  const files = [...pendingFiles.value];
  const quote = pendingQuote.value ? { ...pendingQuote.value } : undefined;

  if (!text && files.length === 0 && !quote) return;
  if (paneIsLoading.value || filesLoading.value) return;
  closeDashMessageMenu();
  if (voiceSettings.value.enabled && voiceSettings.value.autoSpeak) {
    ttsPlayer.preparePlayback();
  }

  const fullMessage = buildMessageWithFiles(text, files);

  const fileAttachments =
    files.length > 0
      ? files.map((f) => ({ name: f.name, isImage: f.isImage, extension: f.extension, isDir: f.isDir }))
      : undefined;
  const aiAttachments =
    files.length > 0
      ? files.map((f) => ({
          path: f.path,
          name: f.name,
          size: f.size,
          extension: f.extension,
          isImage: f.isImage,
        }))
      : [];

  const displayText = text || (quote ? "（引用消息）" : "(已发送文件)");
  chat.addMessage("user", displayText, undefined, fileAttachments, undefined, quote);
  chatInput.value = "";
  pendingQuote.value = null;
  clearPendingFiles();
  chat.collaboration = null;
  chat.activity.reset();
  followingActivity.value = true;
  chat.isLoading = true;
  resetLoadingTimeout(); // 启动超时保底
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;
  syncPetState("thinking");

  // Broadcast user message to other windows (like the pet chat bubble)
  await currentWindow.emit("sync-chat-message", {
    role: "user",
    content: displayText,
    files: fileAttachments,
    quote,
    session_id: sessions.activeMainId || undefined,
  });

  try {
    await invoke("send_to_ai", {
      message: fullMessage,
      attachments: aiAttachments,
      quotedRole: quote?.role,
      quotedContent: quote ? truncateQuoteContent(quote.content) : undefined,
      mentionAgents: dashMentionNames(text),
      collaborationEnabled: chat.collaborationEnabled,
      sessionId: sessions.activeMainId || undefined,
    });
  } catch (err) {
    chat.isLoading = false;
    streamingAnswer.value = "";
    chat.addMessage("assistant", `出错了: ${err}`);
    if (sessionId) sessionRuntime.settle(sessionId, `出错了: ${err}`, { reason: "failed" });
    syncPetState("confused");
  }
}

function skillMatchesSlashQuery(skill: Skill, query: string): boolean {
  if (!query) return true;
  const haystack = [
    skill.name,
    skill.description,
    skill.id,
    ...skill.keywords,
  ].join(" ").toLowerCase();
  return haystack.includes(query);
}

function moveDashSlashSelection(delta: number) {
  const total = dashSlashItems.value.length;
  if (total === 0) return;
  dashSlashSelectedIndex.value = (dashSlashSelectedIndex.value + delta + total) % total;
}

async function chooseDashSlashItem(item = dashSlashItems.value[dashSlashSelectedIndex.value]) {
  if (!item) return;
  if (item.type === "skill") {
    await activateDashSlashSkill(item.skill);
  } else {
    await runDashSlashCommand(item.command);
  }
}

async function activateDashSlashSkill(skill: Skill) {
  try {
    await skillsStore.setActive(skill.id);
    chatInput.value = `请使用「${skill.name || "这个"}」技能：`;
    dashSlashSelectedIndex.value = 0;
    activePage.value = "chat";
    await nextTick();
    dashChatInputRef.value?.focus();
  } catch (err) {
    chat.addMessage("assistant", `切换技能失败：${err}`);
  }
}

async function runDashSlashCommand(command: SlashCommand) {
  chatInput.value = "";
  dashSlashSelectedIndex.value = 0;

  switch (command.id) {
    case "clear-skill":
      try {
        await skillsStore.setActive(null);
        chat.addMessage("assistant", "已清除当前激活的 skill。");
      } catch (err) {
        chat.addMessage("assistant", `清除技能失败：${err}`);
      }
      break;
    case "new-chat":
      await startDashboardNewConversation();
      break;
    case "clear-chat":
      await clearDashboardChatWithConfirm();
      break;
    case "skills":
      openSystemSection("skills");
      await skillsStore.load();
      break;
    case "chat":
    case "clipboard":
      break;
  }

  await nextTick();
  dashChatInputRef.value?.focus();
}

async function startDashboardNewConversation() {
  try {
    snapshotActiveMainTurn();
    const created = await invoke<ChatSession | null>("start_new_conversation");
    resetMainPaneTransientState();
    if (created && typeof created === "object" && "id" in created && created.id) {
      // 后端已新建并设为活跃主会话：刷新列表与当前历史。
      await sessions.load();
      await refreshChatState();
    } else if (chat.messages.length > 0) {
      // 老后端 / 预览环境的兼容兜底。
      chat.addSystemMessage("新对话");
    }
    await loadMemories();
  } catch (err) {
    chat.addMessage("assistant", `新对话创建失败：${err}`);
  }
}

async function clearDashboardChatWithConfirm() {
  const confirmed = window.confirm("确定要清空所有聊天记录吗？此操作不可撤销。");
  if (!confirmed) return;

  try {
    await invoke("clear_chat_history");
    resetAllChatSessionUi();
    await loadMemories();
    await currentWindow.emit("chat-history-cleared");
  } catch (err) {
    chat.addMessage("assistant", `对话清空失败：${err}`);
  }
}

function moveDashMentionSelection(delta: number) {
  const total = dashMentionItems.value.length;
  if (total === 0) return;
  dashMentionSelectedIndex.value = (dashMentionSelectedIndex.value + delta + total) % total;
}

/** 选中一个智能体：把 @查询 替换成 @名字 并继续输入 */
function chooseDashMentionItem(agent = dashMentionItems.value[dashMentionSelectedIndex.value]) {
  const query = dashMentionQuery.value;
  if (!agent || !query) return;
  chatInput.value = insertMentionAt(chatInput.value, query, agentsStore.mentionName(agent));
  dashMentionSelectedIndex.value = 0;
  void nextTick(() => dashChatInputRef.value?.focus());
}

function onDashboardChatKeyDown(e: KeyboardEvent) {
  if (showDashMentionMenu.value) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveDashMentionSelection(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveDashMentionSelection(-1);
      return;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      void chooseDashMentionItem();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      // 只还原正在输入的 @ 片段，保留消息其余部分
      const query = dashMentionQuery.value;
      if (query) {
        chatInput.value = chatInput.value.slice(0, query.start);
      }
      dashMentionSelectedIndex.value = 0;
      return;
    }
  }

  if (showDashSlashMenu.value) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveDashSlashSelection(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveDashSlashSelection(-1);
      return;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      void chooseDashSlashItem();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      chatInput.value = "";
      dashSlashSelectedIndex.value = 0;
      return;
    }
  }

  if (e.key === "Enter") {
    e.preventDefault();
    void sendDashboardMessage();
  }
}

function openDashMessageMenu(msg: Message) {
  activeDashMessageMenuId.value = activeDashMessageMenuId.value === msg.id ? null : msg.id;
}

function closeDashMessageMenu() {
  activeDashMessageMenuId.value = null;
}

async function copyDashMessage(msg: Message) {
  try {
    await navigator.clipboard.writeText(msg.content);
    pet.updateMood({ happiness: 0.01 });
  } catch (err) {
    chat.addMessage("assistant", `复制失败: ${err}`);
  } finally {
    closeDashMessageMenu();
  }
}

/** 引用某条消息：填充输入框上方的引用预览条 */
function quoteDashMessage(msg: Message) {
  if (msg.role === "system") return;
  pendingQuote.value = {
    role: msg.role === "assistant" ? "assistant" : "user",
    content: msg.content,
  };
  closeDashMessageMenu();
  const targetSlot = msg.session_id ? sessions.find(msg.session_id)?.slot : undefined;
  if (isAgentWindowSlot(targetSlot)) chatSlot.value = targetSlot;
  activePage.value = "chat";
  void nextTick(() => dashChatInputRef.value?.focus());
}

/** 清除待发送的引用 */
function clearPendingQuote() {
  pendingQuote.value = null;
}

async function resendDashMessage(msg: Message) {
  const text = msg.content.trim();
  if (!text || paneIsLoading.value) return;
  closeDashMessageMenu();
  activePage.value = "chat";
  chatInput.value = text;
  await nextTick();
  await sendDashboardMessage();
}

function upsertToolEvent(event: ToolEvent) {
  chat.activity.upsertTool(event);
  const index = toolEvents.value.findIndex((item) => item.id === event.id);
  if (index >= 0) {
    toolEvents.value[index] = { ...toolEvents.value[index], ...event };
  } else {
    toolEvents.value.push(event);
  }
}

async function handleDashboardToolConfirm(approved: boolean) {
  await confirmSubmission.submit(approved);
}

function questionAnswered(id: string) {
  if (pendingQuestion.value?.id === id) pendingQuestion.value = null;
}

async function toggleAutoApprovedTool(toolId: string, enabled: boolean) {
  const current = new Set(apiConfig.value.auto_approved_tools);
  if (enabled) current.add(toolId);
  else current.delete(toolId);
  apiConfig.value.auto_approved_tools = Array.from(current).sort();
  // 与执行模式一致：改了立即保存；新建中的配置等用户点「保存配置」时一起保存。
  if (!apiConfig.value.id || !apiConfig.value.base_url || !apiConfig.value.model) return;
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    testResult.value = { success: false, message: "免确认工具保存失败: " + e };
  }
}

function isToolAutoApproved(toolId: string) {
  return apiConfig.value.auto_approved_tools.includes(toolId);
}

function scrollDashChatToBottom() {
  void nextTick(() => {
    runDashChatScroll();
    if (dashChatScrollFrame !== null) {
      cancelAnimationFrame(dashChatScrollFrame);
    }
    dashChatScrollFrame = requestAnimationFrame(() => {
      dashChatScrollFrame = null;
      runDashChatScroll();
    });

    dashChatScrollTimers.forEach(clearTimeout);
    dashChatScrollTimers = [80, 220, 420].map((delay) =>
      setTimeout(() => {
        runDashChatScroll();
      }, delay),
    );
  });
}

function runDashChatScroll() {
  const container = dashChatMessagesRef.value;
  if (!container || !followingActivity.value) return;

  container.scrollTop = container.scrollHeight;
  dashChatEndRef.value?.scrollIntoView({ block: "end" });
}

watch(() => [
  paneMessages.value.length,
  paneIsLoading.value,
  paneThinking.value,
  isMainSlot.value ? streamingAnswer.value : "",
  paneActivityItems.value.length,
  pendingConfirm.value?.id || "",
  currentSessionId.value,
], () => {
  scrollDashChatToBottom();
}, { flush: "post" });

watch(
  () => [dashSlashQuery.value, dashSlashItems.value.length],
  () => {
    dashSlashSelectedIndex.value = 0;
  },
);

watch(
  () => [dashMentionQuery.value?.query ?? "", dashMentionItems.value.length],
  () => {
    dashMentionSelectedIndex.value = 0;
  },
);

watch(activePage, (newPage) => {
  if (newPage === 'chat') {
    void ensureCurrentChatPane();
  }
  if (newPage === 'memory') {
    void loadMemories();
    void loadScheduledTasks();
  }
}, { flush: "post" });

function handlePageEntered() {
  if (activePage.value === "chat") {
    void ensureCurrentChatPane();
  }
}

// === 生命周期 ===
let unlistenPetStatus: UnlistenFn | null = null;

// Loading 超时保底机制
let loadingTimeout: ReturnType<typeof setTimeout> | null = null;
function resetLoadingTimeout() {
  if (loadingTimeout) clearTimeout(loadingTimeout);
  loadingTimeout = setTimeout(() => {
    if (chat.collaboration?.status === "running") return;
    if (pendingConfirm.value || pendingQuestion.value || chat.activity.items.some((item) => item.kind === "tool" && ["running", "waiting"].includes(item.tool.status))) {
      resetLoadingTimeout();
      return;
    }
    if (chat.isLoading) {
      chat.isLoading = false;
      const last = chat.messages[chat.messages.length - 1];
      const timeoutMsg = "⚠️ 响应超时，请重试";
      if (!(last?.role === "assistant" && last.content === timeoutMsg)) {
        const message = chat.addMessage("assistant", timeoutMsg, thinkingContent.value || undefined);
        chat.captureActivity(message, "failed");
      }
      thinkingContent.value = "";
      streamingAnswer.value = "";
      pendingConfirm.value = null;
    }
  }, 90000); // 90 秒超时
}
function clearLoadingTimeout() {
  if (loadingTimeout) {
    clearTimeout(loadingTimeout);
    loadingTimeout = null;
  }
}

onMounted(async () => {
  stopToolConfirm = subscribePendingInteraction("tool", pendingConfirm);
  stopQuestion = subscribePendingInteraction("question", pendingQuestion);
  stopCollaboration = subscribeDashboardCollaboration();
  void agentsStore.startSync().catch((error) => console.error("加载智能体失败", error));
  // 会话列表要在载入历史之前就绪，activeMainId 才能用于事件路由。
  try { await sessions.startSync(); } catch (error) { console.error("加载会话列表失败", error); }
  unlistenWeatherUpdate = await listen<WeatherUpdateEvent>("weather-update", handleWeatherUpdate);
  // 播放状态由后端事件驱动；必须在挂载时就订阅，否则首个播报会整段没有状态反馈。
  void ensurePlaybackSubscription();

  resetTaskDraftTime();
  loadSystemInfo();
  loadMemories();
  loadScheduledTasks();
  loadCurrentModel();
  loadCurrentSkin();
  loadCurrentCharacter();
  loadCurrentFontColor();
  loadUserAvatar();
  loadPersonality();
  loadProfession();
  loadVoiceSettings();
  loadComputerUseSettings();
  loadBackendSettings();
  loadClaudeStatus();
  void loadWeatherConfig().then(() => loadWeather());
  void skillsStore.load();

  sysInfoTimer = setInterval(loadSystemInfo, 5000);
  weatherTimer = setInterval(loadWeather, 300000); // 每5分钟更新天气

  // 检查最大化状态
  isMaximized.value = await currentWindow.isMaximized();

  // 加载对话历史并监听实时同步
  await sessionRuntime.start();
  await refreshChatState();

  unlistenThinking = await listen<{ session_id?: string; text?: string } | string>("ai-thinking", (event) => {
    // 兼容旧 payload（直接是字符串）与契约新 payload（{ session_id, text }）。
    const raw = typeof event.payload === "string" ? { text: event.payload, session_id: undefined } : event.payload;
    const text = raw?.text || "";
    if (!text) return;
    const route = routeIncomingSession(raw?.session_id);
    if (route.kind === "ignore") return;
    if (route.kind === "agent") return; // 由共享运行时处理，避免重复追加或重置轨迹。
    if (route.kind === "main-bg") {
      const turn = ensureMainTurn(route.sessionId);
      turn.loading = true;
      turn.thinking += text;
      turn.activity.appendThinking(text);
      return;
    }
    chat.isLoading = true;
    resetLoadingTimeout(); // 重置超时
    thinkingContent.value += text;
    chat.activity.appendThinking(text);
  });

  unlistenAnswerDelta = await listen<AnswerDeltaPayload & { session_id?: string }>("ai-answer-delta", (event) => {
    const text = event.payload?.text || "";
    if (!text) return;
    const route = routeIncomingSession(event.payload?.session_id);
    if (route.kind === "ignore") return;
    if (route.kind === "agent") return; // 由共享运行时处理，避免重复追加或重置轨迹。
    if (route.kind === "main-bg") {
      const turn = ensureMainTurn(route.sessionId);
      turn.loading = true;
      turn.streamingAnswer += text;
      turn.activity.appendText(text);
      return;
    }
    chat.isLoading = true;
    resetLoadingTimeout(); // 重置超时
    streamingAnswer.value += text;
    chat.activity.appendText(text);
  });

  unlistenAiFinished = await listen<AiFinishedPayload & { session_id?: string }>("ai-finished", (event) => {
    const payload = event.payload;
    const route = routeIncomingSession(payload?.session_id);
    if (route.kind === "ignore") return;
    if (route.kind === "agent") {
      sessions.scheduleReload();
      if (route.sessionId === currentAgentSessionId.value) void speakDashboardReply(speakableText(payload.text));
      return;
    }
    if (route.kind === "main-bg") {
      settleBackgroundMainTurn(route.sessionId, payload.text, {
        thinking: payload.thinking || undefined,
        agent: messageAgentFromPayload(payload),
        reason: "completed",
      });
      sessions.scheduleReload();
      return;
    }
    clearLoadingTimeout(); // 清除超时
    // 将当前工具事件快照附加到 AI 回复消息上
    const toolSnapshot = toolEvents.value.length > 0
      ? JSON.parse(JSON.stringify(toolEvents.value)) as any[]
      : undefined;
    const replyAgent = messageAgentFromPayload(payload);
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === payload.text)) {
      const message = chat.addMessage("assistant", payload.text, payload.thinking || undefined, undefined, toolSnapshot, undefined, replyAgent);
      chat.captureActivity(message);
    }
    if (sessions.activeMainId) delete mainTurns[sessions.activeMainId];
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    toolEvents.value = [];
    pendingConfirm.value = null;
    void speakDashboardReply(speakableText(payload.text));
    sessions.scheduleReload();
  });

  unlistenAiError = await listen<{ message?: string; aborted?: boolean; thinking?: string; session_id?: string }>("ai-error", (event) => {
    const payload = event.payload ?? {};
    const route = routeIncomingSession(payload.session_id);
    if (route.kind === "ignore") return;
    if (route.kind === "agent") return; // 由共享运行时处理，避免重复追加或重置轨迹。
    if (route.kind === "main-bg") {
      const turn = ensureMainTurn(route.sessionId);
      let bgText: string;
      if (payload.aborted) {
        bgText = "已中止";
      } else if (turn.streamingAnswer || turn.thinking || turn.activity.items.length) {
        bgText = "⚠️ " + payload.message;
      } else {
        bgText = "出错了: " + payload.message;
      }
      settleBackgroundMainTurn(route.sessionId, bgText, {
        thinking: payload.thinking || turn.thinking || undefined,
        reason: payload.aborted ? "aborted" : "failed",
      });
      sessions.scheduleReload();
      return;
    }
    clearLoadingTimeout(); // 清除超时
    const toolSnapshot = toolEvents.value.length > 0
      ? JSON.parse(JSON.stringify(toolEvents.value)) as any[]
      : undefined;
    // 区分可恢复中断和致命错误
    let text: string;
    if (payload.aborted) {
      text = "已中止";
    } else if (streamingAnswer.value || thinkingContent.value) {
      // 有部分内容时，显示警告而非错误
      text = "⚠️ " + payload.message;
    } else {
      text = "出错了: " + payload.message;
    }
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === text)) {
      const message = chat.addMessage("assistant", text, payload.thinking || undefined, undefined, toolSnapshot);
      chat.captureActivity(message, payload.aborted ? "aborted" : "failed");
    }
    if (sessions.activeMainId) delete mainTurns[sessions.activeMainId];
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    toolEvents.value = [];
    pendingConfirm.value = null;
  });

  unlistenChatCleared = await listen("chat-history-cleared", () => {
    resetAllChatSessionUi();
    void loadMemories();
  });

  unlistenSkillActivated = await listen<{ skill_id: string; skill_name: string; source: string }>(
    "ai-skill-activated",
    (event) => {
      const sourceLabel = event.payload.source === "manual" ? "手动" : "关键词触发";
      chat.addMessage(
        "assistant",
        `🧩 已激活技能：${event.payload.skill_name}（${sourceLabel}）`,
      );
    }
  );

  unlistenSyncMessage = await listen<any>("sync-chat-message", (event) => {
    const payload = event.payload ?? {};
    const route = routeIncomingSession(payload.session_id);
    if (route.kind === "ignore") return;
    const syncAgent: MessageAgent | null = payload.agent?.name
      ? { id: payload.agent.id || undefined, name: payload.agent.name, avatar: payload.agent.avatar || undefined }
      : null;
    if (route.kind === "agent") return; // 由共享运行时处理，避免重复追加或重置轨迹。
    if (route.kind === "main-bg") {
      if (payload.role === "user") {
        const turn = ensureMainTurn(route.sessionId);
        turn.loading = true;
        turn.thinking = "";
        turn.streamingAnswer = "";
        turn.toolEvents = [];
        turn.activity.reset();
      }
      return;
    }
    const last = chat.messages[chat.messages.length - 1];
    if (last && last.role === payload.role && last.content === payload.content) {
      return;
    }
    chat.addMessage(payload.role, payload.content, undefined, payload.files ?? payload.fileAttachments, undefined, payload.quote, syncAgent);
    if (payload.role === "user") {
      chat.isLoading = true;
      chat.activity.reset();
      followingActivity.value = true;
      thinkingContent.value = "";
      streamingAnswer.value = "";
      toolEvents.value = [];
      pendingConfirm.value = null;
      syncPetState("thinking");
    }
  });

  unlistenToolEvent = await listen<ToolEvent & { session_id?: string }>("ai-tool-event", (event) => {
    const payload = event.payload;
    const route = routeIncomingSession(payload?.session_id);
    if (route.kind === "ignore") return;
    if (route.kind === "agent") return; // 由共享运行时处理，避免重复追加或重置轨迹。
    if (route.kind === "main-bg") {
      const turn = ensureMainTurn(route.sessionId);
      turn.loading = true;
      turn.activity.upsertTool(payload);
      const index = turn.toolEvents.findIndex((item) => item.id === payload.id);
      if (index >= 0) turn.toolEvents[index] = { ...turn.toolEvents[index], ...payload };
      else turn.toolEvents.push(payload);
      return;
    }
    chat.isLoading = true;
    resetLoadingTimeout();
    upsertToolEvent(payload);
  });

  unlistenScheduledTasksChanged = await listen("scheduled-tasks-changed", () => {
    void loadScheduledTasks();
  });

  unlistenScheduledTaskTriggered = await listen<{ message: string }>("scheduled-task-triggered", () => {
    void loadScheduledTasks();
  });

  // 检查桌宠窗口是否已存在
  const existing = await WebviewWindow.getByLabel("pet");
  isPetActive.value = !!existing;

  unlistenPetStatus = await listen("pet-window-closed", () => {
    isPetActive.value = false;
  });
  unlistenVoiceSettingsChanged = await listen("voice-settings-changed", () => {
    void loadVoiceSettings();
  });
  unlistenTtsSettingsChanged = await listen("tts-settings-changed", () => {
    void loadVoiceSettings();
  });

  // 注册控制台文件拖拽事件
  unlistenDragDrop = await currentWindow.onDragDropEvent((event) => {
    handleDragDropEvent(event.payload);
  });

  await nextTick();
  scrollDashChatToBottom();
  if (chatSlot.value !== "main") void ensureAgentWindow(chatSlot.value);
});

onUnmounted(() => {
  clearLoadingTimeout();
  stopCollaboration?.();
  agentsStore.stopSync();
  sessions.stopSync();
  sessionRuntime.stop();
  if (sysInfoTimer) clearInterval(sysInfoTimer);
  if (weatherTimer) clearInterval(weatherTimer);
  if (actionFeedbackTimer) clearTimeout(actionFeedbackTimer);
  if (dashChatScrollFrame !== null) cancelAnimationFrame(dashChatScrollFrame);
  dashChatScrollTimers.forEach(clearTimeout);
  unlistenPetStatus?.();
  unlistenThinking?.();
  unlistenAnswerDelta?.();
  unlistenAiFinished?.();
  unlistenAiError?.();
  unlistenChatCleared?.();
  unlistenSyncMessage?.();
  unlistenToolEvent?.();
  stopToolConfirm?.();
  stopQuestion?.();
  unlistenScheduledTasksChanged?.();
  unlistenScheduledTaskTriggered?.();
  unlistenWeatherUpdate?.();
  unlistenDragDrop?.();
  unlistenVoiceSettingsChanged?.();
  unlistenTtsSettingsChanged?.();
  unlistenSkillActivated?.();
ttsPlayer.stop();
});
</script>

<template>
  <div class="dashboard-root">
    <!-- 自定义标题栏 -->
    <div class="dashboard-titlebar" data-tauri-drag-region>
      <div class="titlebar-title">
        <span class="titlebar-mark"><AppIcon name="brand-pet" :size="14" /></span>
        <span>Desktop Pet</span>
      </div>
      <div class="titlebar-controls">
        <button class="titlebar-btn" @click="minimizeWindow" title="最小化">
          <svg width="12" height="2" viewBox="0 0 12 2"><rect width="12" height="2" rx="1" fill="currentColor"/></svg>
        </button>
        <button class="titlebar-btn" @click="toggleMaximize" :title="isMaximized ? '还原' : '最大化'">
          <svg v-if="!isMaximized" width="10" height="10" viewBox="0 0 10 10" fill="none">
            <rect x="1" y="1" width="8" height="8" rx="1.5" stroke="currentColor" stroke-width="1.5"/>
          </svg>
          <svg v-else width="10" height="10" viewBox="0 0 10 10" fill="none">
            <rect x="1" y="3" width="6" height="6" rx="1.2" stroke="currentColor" stroke-width="1.2"/>
            <path d="M3 3V1.8C3 1.35782 3.35782 1 3.8 1H8.2C8.64218 1 9 1.35782 9 1.8V6.2C9 6.64218 8.64218 7 8.2 7H7" stroke="currentColor" stroke-width="1.2"/>
          </svg>
        </button>
        <button class="titlebar-btn close" @click="closeWindow" title="关闭">
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <path d="M1 1L11 11M11 1L1 11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="dashboard-body">
      <!-- 侧边栏 -->
      <!-- 子列表展开后导航变长，会压到底部的装饰插画：展开时先把插画淡出 -->
      <div :class="['dashboard-sidebar', { 'sidebar-art-hidden': chatNavExpanded || systemNavExpanded }]">
        <div class="sidebar-header">
          <div class="sidebar-brand">
            <span class="brand-logo"><AppIcon name="brand-pet" :size="22" /></span>
            <div class="brand-title-group">
              <span class="brand-name">AI Desktop Pet</span>
              <span class="brand-tag">Control Suite</span>
            </div>
          </div>
        </div>

        <nav class="sidebar-nav">
          <button
            :class="['sidebar-item', { active: activePage === 'home' }]"
            @click="activePage = 'home'"
          >
            <span class="sidebar-item-icon"><House :size="15" /></span>
            <span>首页</span>
          </button>
          <div :class="['sidebar-group', { expanded: chatNavExpanded }]">
            <div :class="['sidebar-item', 'sidebar-item-parent', { active: activePage === 'chat' && chatSlot === 'main' }]">
              <button type="button" class="sidebar-item-main" title="进入主对话" @click="openChatSlot('main')">
                <span class="sidebar-item-icon"><MessageCircle :size="15" /></span>
                <span class="sidebar-item-label">对话互动</span>
              </button>
              <button
                type="button"
                :class="['sidebar-item-caret', { open: chatNavExpanded }]"
                :aria-expanded="chatNavExpanded"
                aria-label="展开/收起 agent 对话窗口"
                title="展开/收起 agent 对话窗口"
                @click.stop="toggleChatSubnav"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                  <path d="M2 3.5L5 6.5L8 3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
            </div>
            <div v-show="chatNavExpanded" class="sidebar-subnav">
              <button
                v-for="item in agentWindowNavItems"
                :key="item.slot"
                :class="['sidebar-subitem', { active: activePage === 'chat' && chatSlot === item.slot }]"
                @click="openChatSlot(item.slot)"
              >
                <span class="sidebar-subitem-avatar" aria-hidden="true"><AppIcon :name="item.icon" :size="13" /></span>
                <span class="sidebar-subitem-label">{{ item.label }}</span>
                <span v-if="agentSlotBusy(item.slot)" class="sidebar-subitem-live" aria-hidden="true" />
              </button>
            </div>
          </div>
          <button
            :class="['sidebar-item', { active: activePage === 'memory' }]"
            @click="activePage = 'memory'"
          >
            <span class="sidebar-item-icon"><Brain :size="15" /></span>
            <span>记忆库</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'appearance' }]"
            @click="activePage = 'appearance'"
          >
            <span class="sidebar-item-icon"><Palette :size="15" /></span>
            <span>个性外观</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'agents' }]"
            @click="openAgentsPage()"
          >
            <span class="sidebar-item-icon"><Bot :size="15" /></span>
            <span>智能体</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'voice' }]"
            @click="activePage = 'voice'"
          >
            <span class="sidebar-item-icon"><Mic :size="15" /></span>
            <span>语音设置</span>
          </button>
          <div :class="['sidebar-group', { expanded: systemNavExpanded }]">
            <div :class="['sidebar-item', 'sidebar-item-parent', { active: activePage === 'system' }]">
              <button type="button" class="sidebar-item-main" title="进入系统设置" @click="openSystemSection()">
                <span class="sidebar-item-icon"><SettingsIcon :size="15" /></span>
                <span class="sidebar-item-label">系统设置</span>
              </button>
              <button
                type="button"
                :class="['sidebar-item-caret', { open: systemNavExpanded }]"
                :aria-expanded="systemNavExpanded"
                aria-label="展开/收起系统设置分区"
                title="展开/收起系统设置分区"
                @click.stop="toggleSystemSubnav"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                  <path d="M2 3.5L5 6.5L8 3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
            </div>
            <div v-show="systemNavExpanded" class="sidebar-subnav">
              <button
                v-for="item in SYSTEM_SECTIONS"
                :key="item.id"
                :class="['sidebar-subitem', { active: activePage === 'system' && systemSection === item.id }]"
                :aria-current="activePage === 'system' && systemSection === item.id ? 'page' : undefined"
                @click="openSystemSection(item.id)"
              >
                <span class="sidebar-subitem-avatar" aria-hidden="true"><AppIcon :name="item.icon" :size="13" /></span>
                <span class="sidebar-subitem-label">{{ item.label }}</span>
                <span
                  v-if="item.id === 'backend'"
                  class="sidebar-subitem-hint"
                  :title="'当前 AI 后端：' + backendLabel(backendType)"
                >{{ backendShortLabel(backendType) }}</span>
              </button>
            </div>
          </div>

          <div class="sidebar-divider" />

          <button
            :class="['sidebar-item', { active: activePage === 'about' }]"
            @click="activePage = 'about'"
          >
            <span class="sidebar-item-icon"><Info :size="15" /></span>
            <span>关于</span>
          </button>
        </nav>

        <div class="sidebar-footer">
          <button
            :class="['sidebar-action-btn', 'primary', { active: isPetActive }]"
            @click="isPetActive ? emit('closePet') : emit('openPet'); isPetActive = !isPetActive"
          >
            <span class="sidebar-action-icon"><PawPrint v-if="isPetActive" :size="14" /><Plus v-else :size="14" /></span>
            <span>{{ isPetActive ? '收回桌宠' : '召唤桌宠' }}</span>
          </button>
          <button class="sidebar-action-btn danger" @click="exitApp">
            <span class="sidebar-action-icon"><Power :size="14" /></span>
            <span>退出应用</span>
          </button>
        </div>
      </div>

      <!-- 内容区 -->
      <div :class="['dashboard-content', `page-${activePage}`, { 'chat-page-active': activePage === 'chat' }]">
        <Transition name="page-fade" mode="out-in" @after-enter="handlePageEntered">
          <!-- ========== 首页 ========== -->
          <div v-if="activePage === 'home'" key="home" class="home-page">
            <div class="page-header home-header">
              <div>
                <div class="page-kicker">Companion</div>
                <h1 class="page-title">你的桌宠</h1>
                <p class="page-subtitle">摸摸它、逗逗它，或者直接派个活</p>
              </div>
              <div :class="['status-pill', isPetActive ? 'live' : 'idle']">
                <span class="status-pill-dot" />
                <span>{{ isPetActive ? '桌宠在线' : '桌宠未启动' }}</span>
              </div>
            </div>

            <div class="home-layout">
              <!-- 舞台：活的桌宠本体 -->
              <section class="home-stage-col">
                <PetStage
                  :state-label="petStateLabel"
                  :happiness-percent="happinessPercent"
                  :energy="energyPercent"
                  :model-label="currentModelLabel"
                  @petted="flashActiveAction('petted')"
                />

                <!-- 陪伴动作：点一下，舞台上的它当场演 -->
                <div class="companion-bar" role="group" aria-label="陪伴动作">
                  <button
                    v-for="action in companionActions"
                    :key="action.id"
                    type="button"
                    :class="['companion-chip', { active: activeActionId === action.id }]"
                    :title="action.desc"
                    @click="companionAction(action.id)"
                  >
                    <span class="companion-chip-icon" aria-hidden="true"><AppIcon :name="action.icon" :size="16" /></span>
                    <span>{{ action.label }}</span>
                  </button>
                </div>
                <p class="companion-hint">
                  {{ activeActionId === 'petted' ? '它很喜欢被摸摸 ♥' : '点按钮或直接点它试试' }}
                </p>
              </section>

              <!-- 信息列 -->
              <section class="home-info-col">
                <!-- 最近对话 + 对话操作 -->
                <div class="dash-card home-card home-convo-card">
                  <div class="home-card-head">
                    <span class="home-card-title">最近对话</span>
                    <span v-if="conversationTurnCount" class="home-card-meta">{{ conversationTurnCount }} 轮</span>
                  </div>
                  <button
                    v-if="lastConversationMessage"
                    type="button"
                    class="home-convo-preview"
                    @click="openChatSlot('main')"
                  >
                    <span class="home-convo-speaker">{{ lastConversationSpeaker }}</span>
                    <span class="home-convo-text">{{ lastConversationPreview }}</span>
                    <span class="home-convo-time">{{ formatRelativeTime(lastConversationMessage.timestamp) }}</span>
                  </button>
                  <button v-else type="button" class="home-convo-preview empty" @click="openChatSlot('main')">
                    <span class="home-convo-text">还没有聊过天，去和它说句话吧 →</span>
                  </button>
                  <div class="home-convo-actions">
                    <button type="button" class="dash-mini-btn primary" @click="openChatSlot('main')">继续聊天</button>
                    <button
                      v-for="action in conversationActions"
                      :key="action.id"
                      type="button"
                      :class="['dash-mini-btn', { 'home-danger-btn': action.danger }]"
                      :title="action.desc"
                      @click="conversationAction(action.id)"
                    >
                      {{ action.label }}
                    </button>
                  </div>
                </div>

                <!-- 今日概览 -->
                <div class="dash-card home-card home-glance-card">
                  <div class="home-card-head">
                    <span class="home-card-title">今日概览</span>
                  </div>
                  <div class="home-glance-grid">
                    <div class="home-glance-item">
                      <span class="home-glance-label">天气</span>
                      <span v-if="weatherInfo" class="home-glance-value">
                        {{ weatherInfo.icon }} {{ formatTemperature(weatherInfo.temperature) }}°C
                      </span>
                      <span v-else class="home-glance-value muted">{{ isWeatherLoading ? '加载中' : '未开启' }}</span>
                      <span v-if="weatherInfo" class="home-glance-sub">{{ weatherInfo.city }} · {{ weatherInfo.description }}</span>
                    </div>
                    <button type="button" class="home-glance-item clickable" @click="activePage = 'memory'">
                      <span class="home-glance-label">下个提醒</span>
                      <span v-if="nextScheduledTask" class="home-glance-value">{{ taskStatusLabel(nextScheduledTask) }}</span>
                      <span v-else class="home-glance-value muted">暂无</span>
                      <span v-if="nextScheduledTask" class="home-glance-sub">{{ nextScheduledTask.title }}</span>
                    </button>
                    <button type="button" class="home-glance-item clickable" @click="activePage = 'memory'">
                      <span class="home-glance-label">记忆</span>
                      <span class="home-glance-value">{{ memories.length }} 条</span>
                      <span class="home-glance-sub">它记住的关于你的事</span>
                    </button>
                    <div class="home-glance-item">
                      <span class="home-glance-label">模式</span>
                      <span class="home-glance-value">{{ currentExecutionModeLabel }}</span>
                      <span class="home-glance-sub" :title="contextUsageLabel">{{ contextUsageLabel }}</span>
                    </div>
                  </div>
                </div>

                <!-- 快速入口 -->
                <div class="home-shortcuts">
                  <button
                    v-for="entry in missionEntries"
                    :key="entry.page"
                    type="button"
                    class="home-shortcut"
                    @click="activePage = entry.page"
                  >
                    <span class="home-shortcut-icon" aria-hidden="true"><AppIcon :name="entry.icon" :size="17" /></span>
                    <span class="home-shortcut-copy">
                      <strong>{{ entry.title }}</strong>
                      <small>{{ entry.desc }}</small>
                    </span>
                  </button>
                  <button type="button" class="home-shortcut" @click="openAgentsPage()">
                    <span class="home-shortcut-icon" aria-hidden="true"><AppIcon name="agent-hub" :size="17" /></span>
                    <span class="home-shortcut-copy">
                      <strong>智能体</strong>
                      <small>召集角色一起干活</small>
                    </span>
                  </button>
                </div>

                <!-- 系统资源：降级为一条细带 -->
                <div class="home-resources">
                  <div class="home-resource">
                    <span class="home-resource-label">CPU</span>
                    <div class="monitor-bar-wrap">
                      <div class="monitor-bar" :style="{ width: systemInfo.cpu + '%', background: cpuBarColor(systemInfo.cpu) }" />
                    </div>
                    <span class="home-resource-value">{{ systemInfo.cpu.toFixed(0) }}%</span>
                  </div>
                  <div class="home-resource">
                    <span class="home-resource-label">内存</span>
                    <div class="monitor-bar-wrap">
                      <div class="monitor-bar" :style="{ width: systemInfo.memory + '%', background: memBarColor(systemInfo.memory) }" />
                    </div>
                    <span class="home-resource-value">{{ systemInfo.memory.toFixed(0) }}%</span>
                  </div>
                </div>
              </section>
            </div>
          </div>

          <!-- ========== 对话互动 (独立页面) ========== -->
          <div v-else-if="activePage === 'chat'" key="chat" class="dash-chat-page">
           <div class="dash-workspace-wrap">
            <div class="dash-workspace-toolbar">
              <span class="dash-workspace-toolbar-label">同屏布局</span>
              <div class="dash-workspace-seg" role="group" aria-label="同屏布局">
                <button
                  v-for="item in WORKSPACE_LAYOUTS"
                  :key="item.id"
                  type="button"
                  :class="['dash-workspace-seg-btn', { active: workspaceLayout === item.id }]"
                  :title="item.label"
                  :aria-label="item.label"
                  :aria-pressed="workspaceLayout === item.id"
                  @click="setWorkspaceLayout(item.id)"
                >
                  <component :is="WORKSPACE_LAYOUT_ICONS[item.id]" :size="14" />
                  <span class="dash-workspace-seg-text">{{ item.label }}</span>
                </button>
              </div>
            </div>
            <p v-if="isWorkspaceMode && workspaceFileError" class="dash-workspace-file-error" role="alert">
              <span>{{ workspaceFileError }}</span>
              <button type="button" aria-label="关闭附件提示" @click="workspaceFileError = ''">×</button>
            </p>
            <div v-if="isWorkspaceMode" :class="['dash-workspace-grid', `layout-${workspaceLayout}`]">
              <SessionPane
                v-for="(paneId, index) in workspacePaneIds"
                :key="paneId"
                :pane-id="paneId"
                :file-over="isFileOver && fileOverPaneId === paneId"
                :session-id="workspacePanes[index] || ''"
                :backend="paneSessionBackend(workspacePanes[index] || '')"
                @assign="assignWorkspacePane"
                @close="closeWorkspacePane"
                @open-single="openSessionInSingle"
                @pane-drop="moveWorkspacePane"
              />
            </div>
            <template v-else>
           <div class="dash-chat-main">
            <!-- 对话页头部：主对话显示会话标题/状态；agent 窗口显示 agent 与模型选择 -->
            <div :class="['dash-chat-head', { 'dash-chat-head-agent': !isMainSlot }]">
              <template v-if="isMainSlot">
                <div class="dash-chat-head-title">{{ chatHeaderTitle }}</div>
                <div class="dash-chat-head-status">
                  <span class="dash-status-pill">{{ currentExecutionModeLabel }}</span>
                  <span :class="['dash-status-pill', 'state', paneIsLoading ? 'busy' : 'ready']">
                    {{ paneIsLoading ? "运行中" : "就绪" }}
                  </span>
                </div>
              </template>
              <template v-else>
                <div class="dash-agent-head-info">
                  <AgentPicker
                    :agents="agentWindowAgentOptions"
                    :model-value="currentAgentIdentity.id"
                    :subtitle="currentSlotLabel + ' · ' + (currentSession?.title || '新会话')"
                    :disabled="paneIsLoading"
                    @change="onAgentWindowAgentChange"
                  />
                </div>
                <div class="dash-agent-head-controls">
                  <AgentModelPicker
                    v-if="agentSlot"
                    :slot="agentSlot"
                    :model="currentAgentModelValue"
                    :provider="currentAgentProviderValue"
                    :effort="currentAgentEffortValue"
                    :disabled="paneIsLoading"
                    @select="onAgentModelSelect"
                  />
                  <SessionPermissionPicker
                    v-if="agentSlot && currentSessionId"
                    :session-id="currentSessionId"
                    :backend="currentAgentBackend"
                    :disabled="paneIsLoading"
                    placement="down"
                  />
                  <span :class="['dash-status-pill', 'state', paneIsLoading ? 'busy' : 'ready']">
                    {{ paneIsLoading ? "运行中" : "就绪" }}
                  </span>
                </div>
              </template>
            </div>
            <div v-if="sessionError" class="dash-session-error-bar">
              <span>{{ sessionError }}</span>
              <button type="button" title="关闭提示" @click="sessionError = ''">×</button>
            </div>
            <!-- 天气信息显示栏 -->
            <div
              v-if="weatherInfo || weatherError || isWeatherLoading"
              :class="['weather-info-bar', { error: weatherError && !weatherInfo }]"
            >
              <template v-if="weatherInfo">
                <span class="weather-icon">{{ weatherInfo.icon }}</span>
                <span class="weather-temp">{{ formatTemperature(weatherInfo.temperature) }}°C</span>
                <span class="weather-city">{{ weatherInfo.city }}</span>
                <span class="weather-desc">{{ weatherInfo.description }}</span>
                <span class="weather-humidity">湿度 {{ formatTemperature(weatherInfo.humidity) }}%</span>
                <button
                  type="button"
                  class="weather-refresh-btn"
                  title="刷新天气"
                  :disabled="isWeatherLoading"
                  @click="refreshWeather"
                >
                  <AppIcon name="refresh" :size="13" />
                </button>
              </template>
              <template v-else-if="isWeatherLoading">
                <span class="weather-icon"><AppIcon name="weather-sun" :size="14" /></span>
                <span class="weather-desc">天气刷新中...</span>
              </template>
              <template v-else>
                <span class="weather-icon"><AppIcon name="alert" :size="14" /></span>
                <span class="weather-desc">天气获取失败：{{ weatherError }}</span>
                <button type="button" class="weather-refresh-btn" title="重试" @click="refreshWeather"><AppIcon name="refresh" :size="13" /></button>
              </template>
            </div>
            <div class="dash-chat-messages" ref="dashChatMessagesRef" @click="closeDashMessageMenu" @scroll.passive="trackChatScroll">
              <!-- Glassmorphism drop zone overlay -->
              <div v-if="isFileOver" class="dash-drop-overlay">
                <div class="dash-drop-overlay-box">
                  <span class="dash-drop-icon"><AppIcon name="folder-drop" :size="44" /></span>
                  <span class="dash-drop-text">释放文件以添加为附件</span>
                </div>
              </div>
              <div v-if="paneMessages.length === 0" class="dash-chat-empty">
                <template v-if="isMainSlot">暂无对话历史，跟小家伙说点什么吧</template>
                <template v-else>{{ currentAgentIdentity.name }} 还没有消息，直接发一条开始对话吧</template>
              </div>
              
              <div
                v-for="msg in paneMessages" :key="msg.id"
                :class="['dash-msg-wrapper', msg.role]"
                @contextmenu.prevent.stop="msg.role !== 'system' && openDashMessageMenu(msg)"
              >
                <!-- 系统分割线 -->
                <div v-if="msg.role === 'system'" class="dash-system-divider">
                  <span class="dash-system-divider-line"></span>
                  <span class="dash-system-divider-text">{{ msg.content }}</span>
                  <span class="dash-system-divider-line"></span>
                </div>
                <AgentActivity v-if="msg.role === 'assistant'" :items="msg.activity" :tools="msg.toolEvents" :thinking="msg.activity ? undefined : msg.thinking" />
                <div v-if="msg.role !== 'system'" class="dash-msg-bubble">
                  <div v-if="msg.agent && msg.role === 'assistant'" class="dash-msg-agent-badge">
                    <span class="dash-msg-agent-avatar">
                      <AppIcon v-if="msg.agent.id === 'builtin-claude'" name="claude" :size="12" />
                      <AppIcon v-else-if="msg.agent.id === 'builtin-codex'" name="codex" :size="12" />
                      <AppIcon v-else-if="msg.agent.id === 'builtin-dsh'" name="dsh" :size="12" />
                      <template v-else>{{ msg.agent.avatar || "🤖" }}</template>
                    </span>
                    <span class="dash-msg-agent-name">{{ msg.agent.name }}</span>
                  </div>
                  <!-- 用户引用的前文对话（微信式引用块） -->
                  <div v-if="msg.quote" class="dash-msg-quote" :title="msg.quote.content">
                    <div class="dash-msg-quote-name">{{ quoteSpeakerLabel(msg.quote.role) }}：</div>
                    <div class="dash-msg-quote-text">{{ msg.quote.content }}</div>
                  </div>
                  <!-- AI 回复中的 [QUOTE] 标记渲染为引用块，其余正文按 Markdown 渲染 -->
                  <div class="dash-msg-text md-body">
                    <template v-for="(seg, si) in renderedMessageSegments(msg)" :key="si">
                      <div v-if="seg.type === 'quote'" class="dash-msg-quote ai-quote dash-quote-body" v-html="seg.html"></div>
                      <div v-else class="dash-msg-body" v-html="seg.html"></div>
                    </template>
                  </div>

                  <!-- 显示已发送的本地文件信息 -->
                  <div v-if="msg.files && msg.files.length > 0" class="dash-msg-files">
                    <div v-for="(file, fi) in msg.files" :key="fi" class="dash-msg-file-tag">
                      <span>{{ file.isDir ? "\u{1F4C2}" : file.isImage ? "\u{1F4F7}" : getFileIcon(file.extension) }}</span>
                      <span>{{ file.name }}</span>
                    </div>
                  </div>
                </div>
                <div
                  v-if="activeDashMessageMenuId === msg.id"
                  class="dash-message-action-menu"
                  @click.stop
                  @contextmenu.prevent.stop
                >
                  <button type="button" @click="copyDashMessage(msg)">复制</button>
                  <button type="button" @click="quoteDashMessage(msg)">引用</button>
                  <button type="button" :disabled="paneIsLoading" @click="resendDashMessage(msg)">重新发送</button>
                </div>
              </div>

              <div v-if="paneIsLoading" class="dash-msg-wrapper assistant loading">
                <AgentActivity :items="paneActivityItems" :thinking="paneThinking" :active="true" :waiting="!!pendingConfirm || !!pendingQuestion" :awaiting-answer="!!pendingQuestion" @stop="abortCurrentTurn" />
              </div>
              <div ref="dashChatEndRef" class="dash-chat-end" aria-hidden="true"></div>
              <!-- 新消息悬浮按钮（跟随最新消息时隐藏） -->
              <button v-if="!followingActivity" type="button" class="dash-scroll-fab" title="回到最新消息" aria-label="回到最新消息" @click="jumpToLatest">
                <ArrowDown :size="16" />
              </button>
            </div>

            <div class="dash-chat-composer">
              <!-- 引用回复预览条（微信式） -->
              <Transition name="slide-up">
                <div v-if="pendingQuote" class="dash-quote-preview">
                  <div class="dash-quote-preview-bar">
                    <div class="dash-quote-preview-content">
                      <span class="dash-quote-preview-name">{{ quoteSpeakerLabel(pendingQuote.role) }}：</span>
                      <span class="dash-quote-preview-text">{{ pendingQuote.content }}</span>
                    </div>
                    <button type="button" class="dash-quote-remove-btn" title="取消引用" @click="clearPendingQuote">&times;</button>
                  </div>
                </div>
              </Transition>

              <AttachmentTray :draft-key="attachmentDraftKey" />

              <Transition name="slide-up">
                <div v-if="isMainSlot && showDashMentionMenu" class="dash-mention-menu" @mousedown.prevent>
                  <button
                    v-for="(agent, index) in dashMentionItems"
                    :key="agent.id"
                    type="button"
                    class="dash-mention-item"
                    :class="{ active: index === dashMentionSelectedIndex }"
                    @mouseenter="dashMentionSelectedIndex = index"
                    @click="chooseDashMentionItem(agent)"
                  >
                    <span class="dash-mention-avatar">
                      <AppIcon v-if="agent.id === 'builtin-claude'" name="claude" :size="14" />
                      <AppIcon v-else-if="agent.id === 'builtin-codex'" name="codex" :size="14" />
                      <AppIcon v-else-if="agent.id === 'builtin-dsh'" name="dsh" :size="14" />
                      <template v-else>{{ agent.avatar || "🤖" }}</template>
                    </span>
                    <span class="dash-mention-main">
                      <span class="dash-mention-title">@{{ agent.name }}</span>
                      <span class="dash-mention-desc">{{ agent.description || "自定义智能体" }}</span>
                    </span>
                    <span class="dash-mention-hint">Tab 选择</span>
                  </button>
                  <button
                    type="button"
                    class="dash-mention-item dash-mention-new"
                    @click="openAgentEditor(); activePage = 'agents'"
                  >
                    <span class="dash-mention-avatar"><AppIcon name="plus" :size="14" /></span>
                    <span class="dash-mention-main">
                      <span class="dash-mention-title">新建智能体</span>
                      <span class="dash-mention-desc">打开智能体工坊，自由定义或用 AI 生成</span>
                    </span>
                  </button>
                </div>
              </Transition>

              <Transition name="slide-up">
                <div v-if="isMainSlot && showDashSlashMenu" class="dash-slash-menu" @mousedown.prevent>
                  <button
                    v-for="(item, index) in dashSlashItems"
                    :key="item.key"
                    type="button"
                    class="dash-slash-item"
                    :class="{ active: index === dashSlashSelectedIndex, skill: item.type === 'skill' }"
                    @mouseenter="dashSlashSelectedIndex = index"
                    @click="chooseDashSlashItem(item)"
                  >
                    <span class="dash-slash-mark">{{ item.type === "skill" ? "#" : "/" }}</span>
                    <span class="dash-slash-main">
                      <span class="dash-slash-title">{{ item.title }}</span>
                      <span class="dash-slash-desc">{{ item.description }}</span>
                    </span>
                    <span class="dash-slash-hint">{{ item.hint }}</span>
                  </button>
                </div>
              </Transition>

              <CollaborationBar v-if="isMainSlot" />
              <div class="dash-chat-input-area">
                <button type="button" class="dash-chat-attach-btn" title="添加附件（也可拖入文件）" aria-label="添加附件" :disabled="attachmentDraft.picking" @click="pickDashboardFiles">
                  <Paperclip :size="17" />
                </button>
                <input
                  ref="dashChatInputRef"
                  type="text"
                  v-model="chatInput"
                  @keydown="onDashboardChatKeyDown"
                  :placeholder="isMainSlot ? '发送消息或拖入文件/应用快捷方式给桌宠...' : ('给 ' + currentAgentIdentity.name + ' 发消息，可拖入文件...')"
                  :disabled="paneIsLoading"
                  class="dash-chat-input"
                />
                <button
                  @click="sendDashboardMessage"
                  :disabled="(!chatInput.trim() && pendingFiles.length === 0 && !pendingQuote) || paneIsLoading || filesLoading"
                  class="dash-chat-send-btn"
                  title="发送"
                  aria-label="发送"
                >
                  <Send :size="16" />
                </button>
              </div>
              <div class="dash-chat-meta-row">
                <span
                  class="dash-chat-hint"
                  :title="isMainSlot ? undefined : ('模型：' + currentSessionModelLabel + (currentSessionChannelLabel ? '（' + currentSessionChannelLabel + '）' : '') + (currentSessionEffortLabel ? ' · 思考：' + currentSessionEffortLabel : ''))"
                >Enter 发送 · 拖入文件添加附件</span>
                <span class="dash-chat-meta-spacer"></span>
                <template v-if="isMainSlot">
                  <label class="dash-meta-field" title="模型">
                    <MetaPicker
                      v-if="apiProfiles.length > 0"
                      label="模型"
                      :options="mainModelOptions"
                      :model-value="activeApiProfileId"
                      @change="activateApiProfile"
                    />
                    <template v-else>
                      <span class="dash-meta-label">模型</span>
                      <span class="dash-meta-value">{{ currentModelLabel }}</span>
                    </template>
                  </label>
                  <MetaPicker label="思考" :current-label="mainThinkingLabel">
                    <template #panel>
                      <EffortSlider
                        :options="mainEffortOptions"
                        :model-value="mainEffortValue"
                        @change="onMainEffortChange"
                      />
                    </template>
                  </MetaPicker>
                  <MetaPicker
                    label="模式"
                    :options="executionModeOptions"
                    :model-value="apiConfig.execution_mode"
                    @change="updateExecutionMode"
                  />
                  <SessionPermissionPicker
                    v-if="backendType !== 'direct_api' && currentSessionId"
                    :session-id="currentSessionId"
                    :backend="backendType"
                    :disabled="paneIsLoading"
                    placement="up"
                  />
                  <button type="button" class="dash-meta-btn" @click="activePage = 'memory'">记忆</button>
                  <button type="button" class="dash-meta-btn" @click="openSystemSection('backend')">设置</button>
                </template>
              </div>
            </div>
           </div>
            </template>
           </div>

            <!-- 右侧信息面板：会话 / 参与者 / 最近记忆 / 任务进度 -->
            <aside v-if="!isWorkspaceMode" class="dash-chat-aside">
              <section class="dash-aside-card dash-sessions-card">
                <div class="dash-aside-title">
                  会话
                  <button
                    type="button"
                    class="dash-aside-link dash-session-new-btn"
                    :disabled="sessions.loading || creatingSession"
                    title="新建会话"
                    @click="createChatSession"
                  >
                    ＋ 新建
                  </button>
                </div>
                <div v-if="sessions.loading && paneSessions.length === 0" class="dash-aside-empty">正在加载会话…</div>
                <div v-else-if="paneSessions.length === 0" class="dash-aside-empty">
                  还没有会话，点「＋ 新建」开始。
                </div>
                <div v-else class="dash-session-list">
                  <div
                    v-for="item in paneSessions"
                    :key="item.id"
                    :class="['dash-session-item', { active: item.id === currentSessionId, switching: switchingSessionId === item.id }]"
                  >
                    <template v-if="renamingSessionId === item.id">
                      <input
                        :ref="setRenameInputElement"
                        v-model="renameDraft"
                        class="dash-session-rename-input"
                        maxlength="80"
                        @click.stop
                        @keydown.enter.prevent="commitRenameChatSession(item)"
                        @keydown.esc.prevent="cancelRenameChatSession"
                        @blur="commitRenameChatSession(item)"
                      />
                    </template>
                    <template v-else>
                      <button
                        type="button"
                        class="dash-session-main"
                        :disabled="sessions.loading || !!switchingSessionId"
                        :title="item.title || '未命名会话'"
                        @click="openChatSessionFromList(item)"
                      >
                        <span class="dash-session-title">{{ item.title || "未命名会话" }}</span>
                        <span class="dash-session-meta">
                          <span>{{ formatRelativeTime(item.updated_at) }}</span>
                          <span v-if="isSessionBusy(item.id)">思考中…</span>
                          <span v-else-if="item.message_count">{{ item.message_count }} 条</span>
                        </span>
                        <span v-if="item.preview" class="dash-session-preview">{{ item.preview }}</span>
                      </button>
                      <div class="dash-session-actions">
                        <button type="button" title="重命名会话" @click.stop="startRenameChatSession(item)">重命名</button>
                        <button type="button" class="danger" title="删除会话" @click.stop="removeChatSession(item)">删除</button>
                      </div>
                    </template>
                  </div>
                </div>
              </section>

              <section class="dash-aside-card">
                <div class="dash-aside-title">参与者</div>
                <div class="dash-aside-hero">
                  <div :class="['dash-aside-avatar', { active: paneIsLoading }]">
                    <AppIcon v-if="isMainSlot" name="brand-pet" :size="20" />
                    <AppIcon v-else-if="currentAgentIdentity.id === 'builtin-claude'" name="claude" :size="20" />
                    <AppIcon v-else-if="currentAgentIdentity.id === 'builtin-codex'" name="codex" :size="20" />
                    <AppIcon v-else-if="currentAgentIdentity.id === 'builtin-dsh'" name="dsh" :size="20" />
                    <template v-else>{{ currentAgentIdentity.avatar }}</template>
                  </div>
                  <div class="dash-aside-hero-main">
                    <div class="dash-aside-hero-name">{{ isMainSlot ? chatHeaderTitle : currentAgentIdentity.name }}</div>
                    <div class="dash-aside-hero-desc">{{ isMainSlot ? "桌面桌宠 · 主助理" : currentSlotLabel + " 独立窗口" }}</div>
                  </div>
                </div>
                <div class="dash-aside-actions">
                  <button
                    v-if="paneIsLoading"
                    type="button"
                    class="dash-aside-btn danger"
                    title="停止当前回复"
                    :disabled="!paneIsLoading"
                    @click="abortCurrentTurn"
                  >
                    停止
                  </button>
                  <button
                    v-if="isMainSlot"
                    type="button"
                    class="dash-aside-btn"
                    title="清空主对话当前窗口"
                    :disabled="paneIsLoading || paneMessages.length === 0"
                    @click="resetDashChatUi"
                  >
                    清空
                  </button>
                  <button
                    v-else
                    type="button"
                    class="dash-aside-btn"
                    title="管理当前智能体"
                    @click="openAgentEditor(); activePage = 'agents'"
                  >
                    管理智能体
                  </button>
                </div>
              </section>

              <section class="dash-aside-card">
                <div class="dash-aside-title">
                  最近记忆
                  <button type="button" class="dash-aside-link" @click="activePage = 'memory'">管理</button>
                </div>
                <div v-if="recentMemories.length === 0" class="dash-aside-empty">还没有长期记忆</div>
                <div v-else class="dash-aside-list">
                  <div v-for="memory in recentMemories" :key="memory.id" class="dash-aside-item">
                    <span class="dash-aside-date">{{ formatRelativeTime(memory.created_at) }}</span>
                    <span class="dash-aside-text" :title="memory.value">{{ memory.key }}</span>
                  </div>
                </div>
              </section>

              <section class="dash-aside-card">
                <div class="dash-aside-title">任务进度</div>
                <div v-if="!paneIsLoading && activeToolItems.length === 0" class="dash-aside-empty">当前没有运行中的任务</div>
                <div v-else class="dash-aside-progress">
                  <div class="dash-aside-progress-meta">
                    <span class="dash-aside-progress-label">{{ paneActivityHeadline }}</span>
                    <span class="dash-aside-progress-count">{{ activeToolItems.length }} 项</span>
                  </div>
                  <div class="dash-aside-progress-bar"><span></span></div>
                  <div class="dash-aside-progress-hint">完成后会自动收起，可随时点击“停止”中断</div>
                </div>
              </section>
            </aside>
          </div>

          <!-- ========== 记忆库 ========== -->
          <div v-else-if="activePage === 'memory'" key="memory">
            <div class="page-header">
              <div class="page-kicker">Memory</div>
              <h1 class="page-title">长期记忆库</h1>
              <p class="page-subtitle">保存稳定偏好、重要背景，并管理 AI 的定时提醒</p>
            </div>

            <div class="dash-memory-layout">
              <div class="dash-memory-editor-stack">
                <div class="dash-card palette-panel skin-panel">
                  <div class="dash-card-title"><span class="card-icon"><AppIcon name="memory-note" :size="15" /></span> 新建记忆</div>
                  <div class="dash-form-group">
                    <label>标题</label>
                    <input type="text" v-model="memoryDraft.key" placeholder="例如：沟通偏好" />
                  </div>
                  <div class="dash-form-group">
                    <label>内容</label>
                    <textarea
                      class="dash-textarea"
                      v-model="memoryDraft.value"
                      rows="5"
                      placeholder="写下希望 AI 长期记住的事实、偏好或背景"
                    ></textarea>
                  </div>
                  <div class="dash-field-row">
                    <span class="dash-field-label">分类</span>
                    <select class="dash-select" v-model="memoryDraft.category">
                      <option value="manual">手动</option>
                      <option value="general">通用</option>
                    </select>
                  </div>
                  <button
                    class="dash-btn primary"
                    :disabled="isSavingMemory || !memoryDraft.key.trim() || !memoryDraft.value.trim()"
                    @click="saveMemoryDraft"
                  >
                    {{ memorySaved ? '已保存' : (isSavingMemory ? '保存中...' : '保存记忆') }}
                  </button>
                </div>

                <div class="dash-card dash-task-composer">
                  <div class="dash-card-title"><span class="card-icon"><AppIcon name="clock-task" :size="15" /></span> 新建定时任务</div>
                  <div class="dash-form-group">
                    <label>任务</label>
                    <input type="text" v-model="taskDraft.title" placeholder="例如：提醒我喝水" />
                  </div>
                  <div class="dash-form-group">
                    <label>时间</label>
                    <input type="datetime-local" v-model="taskDraft.dueAtLocal" />
                  </div>
                  <div class="dash-field-row">
                    <span class="dash-field-label">重复</span>
                    <select class="dash-select" v-model="taskDraft.repeat">
                      <option value="once">仅一次</option>
                      <option value="daily">每天</option>
                      <option value="weekly">每周</option>
                      <option value="monthly">每月</option>
                    </select>
                  </div>
                  <div class="dash-form-group">
                    <label>备注</label>
                    <textarea
                      class="dash-textarea"
                      v-model="taskDraft.note"
                      rows="3"
                      placeholder="可选：补充提醒内容"
                    ></textarea>
                  </div>
                  <div v-if="taskError" class="dash-test-result compact error">{{ taskError }}</div>
                  <button
                    class="dash-btn primary"
                    :disabled="isSavingTask || !taskDraft.title.trim() || !taskDraft.dueAtLocal"
                    @click="saveTaskDraft"
                  >
                    {{ taskSaved ? '已创建' : (isSavingTask ? '创建中...' : '创建任务') }}
                  </button>
                </div>
              </div>

              <div class="dash-memory-list">
                <section class="dash-memory-section">
                  <div class="dash-memory-section-head">
                    <div>
                      <span class="dash-section-kicker">Schedule</span>
                      <h2>定时任务</h2>
                    </div>
                    <span class="dash-memory-count">{{ scheduledTasks.length }} 项</span>
                  </div>
                  <div v-if="scheduledTasks.length === 0" class="dash-memory-empty">
                    暂无定时任务。可以在左侧创建，也可以在对话框里让 AI 帮你设置提醒。
                  </div>
                  <div
                    v-for="task in scheduledTasks"
                    :key="task.id"
                    :class="['dash-task-record', { disabled: !task.enabled }]"
                  >
                    <div class="dash-task-time-block">
                      <strong>{{ formatTaskTime(task.due_at) }}</strong>
                      <span>{{ taskRepeatLabel(task.repeat) }}</span>
                    </div>
                    <div class="dash-task-main">
                      <div class="dash-memory-record-head">
                        <div>
                          <span class="dash-memory-category">{{ taskStatusLabel(task) }}</span>
                          <h3>{{ task.title }}</h3>
                        </div>
                        <div class="dash-task-actions">
                          <button
                            class="dash-icon-btn"
                            :title="task.enabled ? '暂停任务' : '启用任务'"
                            :disabled="taskTogglingId === task.id"
                            @click="toggleScheduledTask(task)"
                          >
                            <AppIcon :name="task.enabled ? 'pause' : 'play'" :size="13" />
                          </button>
                          <button
                            class="dash-icon-btn danger"
                            title="删除任务"
                            :disabled="taskDeletingId === task.id"
                            @click="deleteScheduledTask(task.id)"
                          >
                            <AppIcon name="close" :size="13" />
                          </button>
                        </div>
                      </div>
                      <p v-if="task.note">{{ task.note }}</p>
                      <div class="dash-task-meta">下次提醒：{{ formatTaskFullTime(task.due_at) }}</div>
                    </div>
                  </div>
                </section>

                <section class="dash-memory-section">
                  <div class="dash-memory-section-head">
                    <div>
                      <span class="dash-section-kicker">Memory</span>
                      <h2>记忆条目</h2>
                    </div>
                    <span class="dash-memory-count">{{ memories.length }} 条</span>
                  </div>
                  <div v-if="memories.length === 0" class="dash-memory-empty">
                    暂无长期记忆。清空一段对话后会自动生成摘要，也可以在左侧手动添加。
                  </div>
                  <div v-for="memory in memories" :key="memory.id" class="dash-memory-record">
                    <div class="dash-memory-record-head">
                      <div>
                        <span class="dash-memory-category">{{ memoryCategoryLabel(memory.category) }}</span>
                        <h3>{{ memory.key }}</h3>
                      </div>
                      <button
                        class="dash-icon-btn danger"
                        title="删除记忆"
                        :disabled="memoryDeletingId === memory.id"
                        @click="deleteMemoryItem(memory.id)"
                      >
                        <AppIcon name="close" :size="13" />
                      </button>
                    </div>
                    <p>{{ memory.value }}</p>
                  </div>
                </section>
              </div>
            </div>
          </div>

          <!-- ========== 智能体工坊 ========== -->
          <div v-else-if="activePage === 'agents'" key="agents" class="dash-agents-page">
            <div class="page-header">
              <div class="page-kicker">Agents</div>
              <h1 class="page-title">智能体工坊</h1>
              <p class="page-subtitle">每个角色独立执行并读取当前会话全部发言。开启协同后，角色可用独立的 @名字 任务 行交接工作。</p>
            </div>

            <div class="dash-agents-layout">
              <div class="dash-card dash-agents-list-panel">
                <div class="dash-card-title">
                  <span class="card-icon"><AppIcon name="agent-hub" :size="15" /></span> 我的智能体
                  <button class="dash-agents-add-btn" @click="openAgentEditor()">＋ 新建</button>
                </div>
                <div v-if="agentSaved" class="dash-agents-saved-tip">✓ 已保存</div>
                <div v-if="agentsStore.agents.length === 0" class="dash-agents-empty">
                  还没有自定义智能体。<br />点击「＋ 新建」自由定义，或用下方的 AI 帮你生成一个。
                </div>
                <div v-for="agent in agentsStore.agents" :key="agent.id" class="dash-agent-card">
                  <span class="dash-agent-card-avatar">
                    <AppIcon v-if="agent.id === 'builtin-claude'" name="claude" :size="18" />
                    <AppIcon v-else-if="agent.id === 'builtin-codex'" name="codex" :size="18" />
                    <AppIcon v-else-if="agent.id === 'builtin-dsh'" name="dsh" :size="18" />
                    <template v-else>{{ agent.avatar || "🤖" }}</template>
                  </span>
                  <div class="dash-agent-card-main">
                    <div class="dash-agent-card-name">@{{ agent.name }}</div>
                    <div class="dash-agent-card-desc">{{ agent.description || "（无描述）" }}</div>
                    <div class="dash-agent-card-meta">
                      {{ agent.backend === 'codex' ? '本机 Codex' : agent.backend === 'dsh' ? '本机 DeepSeek Harness' : agent.backend === 'claude_code' ? '本机 Claude Code' : '直连 API' }}{{ agent.is_builtin ? ' · 内置角色' : '' }}{{ agent.model ? ' · ' + agent.model : '' }}{{ !agent.is_builtin && !agent.system_prompt.trim() ? ' · 未写角色设定' : '' }}
                    </div>
                  </div>
                  <div class="dash-agent-card-actions">
                    <button class="dash-mini-btn" @click="openChatSlot('main'); chatInput = chatInput + (chatInput && !/\s$/.test(chatInput) ? ' ' : '') + '@' + agentsStore.mentionName(agent) + ' '">召唤</button>
                    <button v-if="agent.is_builtin" class="dash-mini-btn" @click="configureBuiltinAgent(agent)">登录配置</button>
                    <button v-if="!agent.is_builtin" class="dash-mini-btn" @click="openAgentEditor(agent)">编辑</button>
                    <button
                      class="dash-mini-btn danger"
                      v-if="!agent.is_builtin"
                      :disabled="agentDeletingId === agent.id"
                      @click="deleteAgentDraft(agent)"
                    >
                      删除
                    </button>
                  </div>
                </div>

                <div class="dash-card-title" style="margin-top: 18px;">
                  <span class="card-icon"><AppIcon name="ai-spark" :size="15" /></span> 用 AI 创建
                </div>
                <p class="dash-agents-ai-hint">一句话说清你要它做什么，AI 会推断使用场景，写出名字、头像、完整角色设定和工具权限，你确认后即可保存。也可以在对话里直接说「帮我创建一个……的智能体」。此功能使用系统设置中的直连 API 配置。</p>
                <textarea
                  v-model="agentGenDesc"
                  class="dash-agents-gen-input"
                  rows="3"
                  placeholder="例如：一个帮我写周报的智能体，语气正式，善于从零散信息里提炼重点……"
                ></textarea>
                <div class="dash-agents-gen-row">
                  <button
                    class="dash-primary-btn"
                    :disabled="agentGenLoading || !agentGenDesc.trim()"
                    @click="generateAgentFromDesc"
                  >
                    {{ agentGenLoading ? "生成中…" : "生成智能体" }}
                  </button>
                  <span v-if="agentGenError" class="dash-agents-gen-error">{{ agentGenError }}</span>
                </div>
              </div>

              <div v-if="agentEditorOpen" class="dash-card dash-agent-editor-panel">
                <div class="dash-card-title">
                  <span class="card-icon">{{ agentDraft.avatar || "🤖" }}</span>
                  {{ editingAgentId ? "编辑智能体" : "新建智能体" }}
                  <button class="dash-agents-close-btn" @click="closeAgentEditor()">×</button>
                </div>
                <div class="dash-form-group">
                  <label>名字（对话中 @ 它用，不能含空格和 @）</label>
                  <input v-model="agentDraft.name" type="text" placeholder="例如：周报助手" maxlength="40" />
                </div>
                <div class="dash-form-group">
                  <label>头像</label>
                  <div class="dash-avatar-picker">
                    <button
                      v-for="emoji in AGENT_AVATARS"
                      :key="emoji"
                      type="button"
                      class="dash-avatar-option"
                      :class="{ active: agentDraft.avatar === emoji }"
                      @click="agentDraft.avatar = emoji"
                    >
                      {{ emoji }}
                    </button>
                  </div>
                </div>
                <div class="dash-form-group">
                  <label>一句话描述</label>
                  <input v-model="agentDraft.description" type="text" placeholder="它擅长什么（会显示在 @ 列表中）" maxlength="100" />
                </div>
                <div class="dash-form-group">
                  <label>角色设定（system prompt）</label>
                  <textarea
                    v-model="agentDraft.system_prompt"
                    rows="6"
                    maxlength="4000"
                    placeholder="可以留空后点下方「AI 补全设定」：AI 会根据名字和一句话描述写出身份、职责、工作方式和边界"
                  ></textarea>
                  <div class="dash-agents-gen-row">
                    <button
                      type="button"
                      class="dash-mini-btn"
                      :disabled="agentCompleteLoading"
                      @click="completeAgentDraft()"
                    >
                      {{ agentCompleteLoading ? "补全中…" : agentDraft.system_prompt.trim() ? "AI 完善设定" : "AI 补全设定" }}
                    </button>
                  </div>
                </div>
                <div class="dash-form-group">
                  <label>执行后端</label>
                  <select v-model="agentDraft.backend" class="dash-select">
                    <option value="direct_api">直连 API</option>
                    <option value="claude_code">本机 Claude Code</option>
                    <option value="codex">本机 Codex</option>
                    <option value="dsh">本机 DeepSeek Harness</option>
                  </select>
                </div>
                <div v-if="agentDraft.backend === 'direct_api'" class="dash-form-group">
                  <label>API 配置</label>
                  <select v-model="agentDraft.api_profile_id" class="dash-select">
                    <option value="">使用当前 API 配置</option>
                    <option v-for="profile in apiProfiles" :key="profile.id" :value="profile.id">{{ profile.name || profile.model }}</option>
                  </select>
                </div>
                <div v-if="agentDraft.backend === 'direct_api'" class="dash-form-group">
                  <label>专属模型（留空跟随所选 API 配置）</label>
                  <input v-model="agentDraft.model" type="text" placeholder="例如：gpt-4o-mini / deepseek-chat" />
                </div>
                <div class="dash-form-group">
                  <label>免确认工具（仅直连 API，本轮被 @ 时生效；计划模式不执行工具）</label>
                  <div class="dash-tool-perm-list">
                    <label v-for="tool in toolPermissionOptions" :key="tool.id" class="dash-tool-perm-item">
                      <input
                        type="checkbox"
                        :checked="agentDraft.allowed_tools.includes(tool.id)"
                        @change="toggleAgentTool(tool.id, ($event.target as HTMLInputElement).checked)"
                      />
                      <span>{{ tool.label }}</span>
                    </label>
                  </div>
                </div>
                <div class="dash-agents-editor-actions">
                  <span v-if="agentSaveError" class="dash-agents-gen-error">{{ agentSaveError }}</span>
                  <button class="dash-primary-btn" @click="saveAgentDraft()">保存智能体</button>
                  <button class="dash-mini-btn" @click="closeAgentEditor()">取消</button>
                </div>
              </div>
            </div>
          </div>

          <!-- ========== 外观 ========== -->
          <div v-else-if="activePage === 'appearance'" key="appearance" class="appearance-atelier-page">
            <div class="page-header atelier-header">
              <div class="page-kicker">Appearance</div>
              <h1 class="page-title">个性外观</h1>
              <p class="page-subtitle">自定义桌宠的主题、字体和外观风格</p>
            </div>

            <!-- 主题皮肤 -->
            <div class="appearance-atelier-layout">
              <div class="dash-card character-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="pet-figure" :size="15" /></span> 本体形象</div>
              <div class="dash-character-grid">
                <button
                  v-for="character in petCharacters"
                  :key="character.id"
                  :class="['dash-character-item', { active: currentCharacter === character.id }]"
                  @click="selectCharacter(character.id)"
                >
                  <div class="dash-character-preview">
                    <PetCanvas
                      v-if="character.id === 'classic' || character.id === 'custom-pixel'"
                      preview
                      :character="character.id"
                      :scale="0.73"
                      style="pointer-events: none;"
                    />
                    <img v-else src="../assets/pets/daimao-batiao/stills/still-03.png" alt="" />
                  </div>
                  <span class="dash-character-name">{{ character.name }}</span>
                  <span class="dash-character-desc">{{ character.description }}</span>
                </button>
              </div>
            </div>

              <div class="dash-card character-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="pixel-grid" :size="15" /></span> 自定义像素桌宠</div>
              <CustomPixelPetWorkshop @selected="onCustomPixelSelected" />
            </div>

              <div class="dash-card palette-panel skin-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="wardrobe" :size="15" /></span> 主题皮肤</div>
              <div class="dash-skin-grid">
                <div
                  v-for="t in themes" :key="t.id"
                  :class="['dash-skin-item', { active: currentSkin === t.id }]"
                  @click="selectSkin(t.id)"
                >
                  <div class="dash-skin-preview" :style="{ background: t.headerGradient }">
                    <span v-if="currentSkin === t.id" class="dash-skin-check"><AppIcon name="check" :size="13" /></span>
                  </div>
                  <span class="dash-skin-name">{{ t.name }}</span>
                  <span v-if="t.animated" class="dash-skin-tag">随时间变色</span>
                </div>
              </div>
            </div>

            <!-- 字体颜色 -->
              <div class="dash-card sample-sheet font-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="type-color" :size="15" /></span> 字体颜色</div>
              <div class="dash-font-color-grid">
                <div
                  v-for="fc in FONT_COLORS" :key="fc.id"
                  :class="['dash-font-color-item', { active: currentFontColor === fc.value }]"
                  @click="selectFontColor(fc.value)"
                >
                  <span class="dash-font-color-preview" :style="{ color: fc.value || 'var(--pet-bubble-bot-text, #4a4a4a)' }">Aa</span>
                  <span class="dash-font-color-name">{{ fc.name }}</span>
                </div>
              </div>
            </div>

            <!-- 对话背景 -->
              <div class="dash-card film-strip-panel bg-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="scene-bg" :size="15" /></span> 对话背景</div>
              <div class="dash-bg-grid">
                <button :class="['dash-bg-opt', { active: chatBg === 'none' }]" @click="selectBg('none')">
                  <div class="dash-bg-preview"><AppIcon name="bg-none" :size="20" /></div>
                  <span class="dash-bg-label">无</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'cute' }]" @click="selectBg('cute')">
                  <div class="dash-bg-preview"><AppIcon name="bg-cute" :size="20" /></div>
                  <span class="dash-bg-label">可爱</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'scifi' }]" @click="selectBg('scifi')">
                  <div class="dash-bg-preview"><AppIcon name="bg-scifi" :size="20" /></div>
                  <span class="dash-bg-label">科幻</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'minimal' }]" @click="selectBg('minimal')">
                  <div class="dash-bg-preview"><AppIcon name="bg-minimal" :size="20" /></div>
                  <span class="dash-bg-label">简洁</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'custom' }]" @click="chooseBgImage">
                  <div class="dash-bg-preview">
                    <img v-if="customBgImage" :src="customBgImage" alt="" />
                    <AppIcon v-else name="bg-upload" :size="20" />
                  </div>
                  <span class="dash-bg-label">自定义</span>
                </button>
                <button v-if="chatBg === 'custom' && customBgImage" class="dash-bg-opt" @click="clearCustomBg">
                  <div class="dash-bg-preview"><AppIcon name="trash" :size="20" /></div>
                  <span class="dash-bg-label">清除</span>
                </button>
              </div>
              <input ref="bgInputRef" class="hidden-input" type="file" accept="image/*" @change="onBgImageSelected" />
            </div>

            <!-- 用户头像 -->
            <div class="dash-card portrait-frame-panel avatar-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="user-avatar" :size="15" /></span> 用户头像</div>
              <div class="dash-avatar-section">
                <div class="dash-avatar-preview">
                  <img v-if="pet.userAvatar" :src="pet.userAvatar" alt="用户头像" />
                  <AppIcon v-else name="user-avatar" :size="26" />
                </div>
                <div class="dash-avatar-actions">
                  <button class="dash-btn primary" @click="chooseAvatar">选择图片</button>
                  <button class="dash-btn" :disabled="!pet.userAvatar" @click="clearAvatar">恢复默认</button>
                </div>
                <input ref="avatarInputRef" class="hidden-input" type="file" accept="image/*" @change="onAvatarSelected" />
              </div>
            </div>
          </div>
          </div>

          <!-- ========== 语音 ========== -->
          <div v-else-if="activePage === 'voice'" key="voice" class="voice-studio-page">
            <div class="page-header sound-header">
              <div class="page-kicker">Voice</div>
              <h1 class="page-title">语音设置</h1>
              <p class="page-subtitle">配置语音交互和文字转语音引擎</p>
            </div>

            <div class="dash-card sound-panel voice-interaction-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="voice-station" :size="15" /></span> 语音交互</div>

              <div class="voice-toggle-grid">
              <div class="dash-switch-row sound-toggle-card">
                <div class="dash-switch-info">
                  <span class="dash-switch-label">启用语音</span>
                  <span class="dash-switch-desc">允许麦克风输入和回复播报</span>
                </div>
                <input
                  type="checkbox" class="dash-toggle"
                  :checked="voiceSettings.enabled"
                  @change="updateVoiceSettings({ enabled: ($event.target as HTMLInputElement).checked })"
                />
              </div>

              <div class="dash-switch-row sound-toggle-card">
                <div class="dash-switch-info">
                  <span class="dash-switch-label">识别后自动发送</span>
                  <span class="dash-switch-desc">关闭后会先填入输入框</span>
                </div>
                <input
                  type="checkbox" class="dash-toggle"
                  :checked="voiceSettings.autoSend"
                  @change="updateVoiceSettings({ autoSend: ($event.target as HTMLInputElement).checked })"
                />
              </div>

              <div class="dash-switch-row sound-toggle-card">
                <div class="dash-switch-info">
                  <span class="dash-switch-label">自动朗读回复</span>
                  <span class="dash-switch-desc">AI 回复完成后直接播报</span>
                </div>
                <input
                  type="checkbox" class="dash-toggle"
                  :checked="voiceSettings.autoSpeak"
                  @change="updateVoiceSettings({ autoSpeak: ($event.target as HTMLInputElement).checked })"
                />
              </div>
              </div>

              <div class="dash-field-row">
                <span class="dash-field-label">快捷键</span>
                <input
                  type="text" class="dash-input" style="min-width:120px;"
                  :value="voiceSettings.shortcut"
                  placeholder="Alt+V"
                  @change="updateVoiceSettings({ shortcut: ($event.target as HTMLInputElement).value.trim() || DEFAULT_VOICE_SETTINGS.shortcut })"
                />
              </div>

              <div class="dash-field-row">
                <span class="dash-field-label">语言</span>
                <select
                  class="dash-select"
                  :value="voiceSettings.language"
                  @change="updateVoiceSettings({ language: ($event.target as HTMLSelectElement).value })"
                >
                  <option value="zh-CN">中文普通话</option>
                  <option value="en-US">English (US)</option>
                  <option value="ja-JP">日本語</option>
                </select>
              </div>
            </div>

            <div class="dash-card mixer-panel voice-engine-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="voice-mixer" :size="15" /></span> 声音引擎</div>

              <div class="dash-field-row">
                <span class="dash-field-label">声音引擎</span>
                <select
                  class="dash-select"
                  :value="ttsSettings.engine"
                  @change="updateTtsSettings({ engine: ($event.target as HTMLSelectElement).value as any })"
                >
                  <option value="edge">微软自然语音</option>
                  <option value="system">系统语音</option>
                </select>
              </div>

              <template v-if="ttsSettings.engine === 'edge'">
                <div class="dash-voice-picker-tools">
                  <input class="dash-voice-search" type="search" v-model="edgeVoiceSearch" placeholder="搜索人声、地区或编号" />
                  <label class="dash-mini-check">
                    <input type="checkbox" v-model="showAllEdgeVoices" />
                    <span>显示全部</span>
                  </label>
                </div>
                <div class="dash-field-row">
                  <span class="dash-field-label">
                    声音角色
                    <small>{{ edgeVoiceSummary }}</small>
                  </span>
                  <select
                    class="dash-select" style="min-width:220px;"
                    :value="ttsSettings.voice"
                    @change="updateTtsSettings({ voice: ($event.target as HTMLSelectElement).value })"
                    :disabled="filteredEdgeVoices.length === 0"
                  >
                    <option v-for="v in filteredEdgeVoices" :key="v.id" :value="v.id">
                      {{ v.name }} · {{ v.language }} · {{ v.gender === 'Female' ? '女' : '男' }}
                    </option>
                  </select>
                </div>
                <div v-if="filteredEdgeVoices.length === 0" class="dash-voice-empty">
                  没找到匹配的人声，可以清空搜索或打开"显示全部"。
                </div>

                <div class="dash-range-row mixer-control">
                  <div class="dash-range-header">
                    <span class="dash-range-label">语速</span>
                    <span class="dash-range-value">{{ ttsSettings.rate >= 0 ? '+' : '' }}{{ ttsSettings.rate }}%</span>
                  </div>
                  <input class="dash-range" type="range" min="-50" max="100" step="10" :value="ttsSettings.rate"
                    @input="updateTtsSettings({ rate: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row mixer-control">
                  <div class="dash-range-header">
                    <span class="dash-range-label">音调</span>
                    <span class="dash-range-value">{{ ttsSettings.pitch >= 0 ? '+' : '' }}{{ ttsSettings.pitch }}Hz</span>
                  </div>
                  <input class="dash-range" type="range" min="-50" max="50" step="5" :value="ttsSettings.pitch"
                    @input="updateTtsSettings({ pitch: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row mixer-control">
                  <div class="dash-range-header">
                    <span class="dash-range-label">音量</span>
                    <span class="dash-range-value">{{ ttsSettings.volume }}%</span>
                  </div>
                  <input class="dash-range" type="range" min="0" max="100" step="10" :value="ttsSettings.volume"
                    @input="updateTtsSettings({ volume: Number(($event.target as HTMLInputElement).value) })" />
                </div>

                <div class="dash-preview-row">
                  <button class="dash-btn primary" :disabled="filteredEdgeVoices.length === 0" @click="previewVoice">
                    <AppIcon :name="isSpeaking ? 'stop' : 'play'" :size="13" />
                    <span>{{ isSpeaking ? '停止' : '试听' }}</span>
                  </button>
                  <span v-if="isSpeaking" class="dash-preview-hint">正在播放 {{ speakingText.slice(0, 18) }}…</span>
                </div>
              </template>

              <template v-else>
                <div class="dash-field-row">
                  <span class="dash-field-label">声音</span>
                  <select class="dash-select"
                    :value="voiceSettings.voiceName"
                    @change="updateVoiceSettings({ voiceName: ($event.target as HTMLSelectElement).value })"
                  >
                    <option value="">自动选择</option>
                    <option v-for="voice in availableVoices" :key="voice.name" :value="voice.name">
                      {{ voice.name }} · {{ voice.lang }}
                    </option>
                  </select>
                </div>
                <div class="dash-range-row mixer-control">
                  <div class="dash-range-header">
                    <span class="dash-range-label">语速</span>
                    <span class="dash-range-value">{{ voiceSettings.rate.toFixed(1) }}</span>
                  </div>
                  <input class="dash-range" type="range" min="0.6" max="1.5" step="0.1" :value="voiceSettings.rate"
                    @input="updateVoiceSettings({ rate: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row mixer-control">
                  <div class="dash-range-header">
                    <span class="dash-range-label">音调</span>
                    <span class="dash-range-value">{{ voiceSettings.pitch.toFixed(1) }}</span>
                  </div>
                  <input class="dash-range" type="range" min="0.6" max="1.6" step="0.1" :value="voiceSettings.pitch"
                    @input="updateVoiceSettings({ pitch: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row mixer-control">
                  <div class="dash-range-header">
                    <span class="dash-range-label">音量</span>
                    <span class="dash-range-value">{{ Math.round(voiceSettings.volume * 100) }}%</span>
                  </div>
                  <input class="dash-range" type="range" min="0" max="1" step="0.1" :value="voiceSettings.volume"
                    @input="updateVoiceSettings({ volume: Number(($event.target as HTMLInputElement).value) })" />
                </div>
              </template>
            </div>
          </div>

          <!-- ========== 系统设置（侧边栏子列表分区，见 docs/settings-redesign.md） ========== -->
          <div v-else-if="activePage === 'system'" key="system" class="system-lab-page sys-page">
            <div class="page-header lab-header sys-header">
              <div class="sys-header-copy">
                <div class="page-kicker">系统设置</div>
                <h1 class="page-title">{{ currentSystemSection.label }}</h1>
                <p class="page-subtitle">{{ currentSystemSection.desc }}</p>
              </div>
              <button
                type="button"
                class="sys-header-badge"
                :title="'当前 AI 后端：' + backendLabel(backendType)"
                @click="openSystemSection('backend')"
              >
                <AppIcon :name="backendIcon(backendType)" :size="14" />
                <span>{{ backendLabel(backendType) }}</span>
              </button>
            </div>

            <Transition name="page-fade" mode="out-in">
              <div :key="systemSection" class="sys-body">
                <!-- ===== AI 后端 ===== -->
                <template v-if="systemSection === 'backend'">
                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">选择 AI 后端</h2>
                        <p class="sys-section-desc">主对话和桌宠聊天使用这里选中的后端；Claude Code / Codex / DSH 三个 agent 窗口固定使用各自的本机 CLI，不受这里影响。</p>
                      </div>
                    </header>
                    <div class="sys-backend-grid" role="radiogroup" aria-label="AI 后端">
                      <button
                        v-for="item in backendOptions"
                        :key="item.id"
                        type="button"
                        role="radio"
                        :aria-checked="backendType === item.id"
                        :class="['sys-backend-card', { active: backendType === item.id }]"
                        @click="backendType !== item.id && selectBackend(item.id)"
                      >
                        <span class="sys-backend-icon"><AppIcon :name="item.icon" :size="18" /></span>
                        <span class="sys-backend-copy">
                          <strong>{{ item.label }}</strong>
                          <small>{{ item.desc }}</small>
                        </span>
                        <span v-if="backendType === item.id" class="sys-backend-check" aria-hidden="true"><AppIcon name="check" :size="12" :stroke-width="2.4" /></span>
                      </button>
                    </div>
                  </section>

                  <section v-if="backendType === 'claude_code'" class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">Claude Code 连接</h2>
                        <p class="sys-section-desc">使用本机 Claude Code CLI 子进程，复用本机登录，会话自动保持上下文。</p>
                      </div>
                      <span :class="['sys-status', claudeStatus.logged_in ? 'ok' : 'off']">
                        <span class="sys-status-dot" />{{ claudeStatus.logged_in ? '已连接' : '未连接' }}
                      </span>
                    </header>
                    <dl class="sys-facts">
                      <div><dt>驱动模型</dt><dd class="sys-mono">{{ claudeStatus.model || '跟随本机配置' }}</dd></div>
                      <div v-if="claudeStatus.token_preview"><dt>授权凭证</dt><dd class="sys-mono">{{ claudeStatus.token_preview }}</dd></div>
                      <div v-if="claudeStatus.base_url"><dt>API 接入点</dt><dd class="sys-mono">{{ claudeStatus.base_url }}</dd></div>
                      <div><dt>当前会话模型</dt><dd>{{ currentModel || '加载中…' }}</dd></div>
                    </dl>
                    <footer class="sys-section-foot">
                      <button type="button" class="sys-link-btn" @click="openSystemSection('permissions')">权限模式在「执行权限」里设置 →</button>
                      <button type="button" class="dash-btn secondary" @click="startClaudeConfig">登录 / 配置</button>
                      <button type="button" class="dash-btn" @click="resetModel">{{ modelSaved ? '已重置' : '重置会话' }}</button>
                    </footer>
                  </section>

                  <section v-else-if="backendType === 'codex'" class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">Codex 连接</h2>
                        <p class="sys-section-desc">本机 Codex CLI 的安装、登录与工作目录。</p>
                      </div>
                    </header>
                    <CodexConnection :model-saved="modelSaved" :show-permission="false" @reset="resetModel" />
                  </section>

                  <section v-else-if="backendType === 'dsh'" class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">DeepSeek Harness 连接</h2>
                        <p class="sys-section-desc">本机 DSH 的安装状态、模型与桌宠专属 profile。</p>
                      </div>
                    </header>
                    <DshConnection :model-saved="modelSaved" :show-permission="false" @reset="resetModel" />
                  </section>

                  <section v-else class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">直连 API</h2>
                        <p class="sys-section-desc">通过 OpenAI 兼容接口直接调用模型，内置文件、命令、搜索、记忆等工具。</p>
                      </div>
                      <span :class="['sys-status', apiConfig.has_api_key ? 'ok' : 'off']">
                        <span class="sys-status-dot" />{{ apiConfig.has_api_key ? '密钥已保存' : '未填密钥' }}
                      </span>
                    </header>
                    <dl class="sys-facts">
                      <div><dt>当前配置</dt><dd>{{ apiConfig.name || apiConfig.model || '未选择' }}</dd></div>
                      <div><dt>模型</dt><dd class="sys-mono">{{ apiConfig.model || '—' }}</dd></div>
                      <div><dt>接口地址</dt><dd class="sys-mono">{{ apiConfig.base_url || '—' }}</dd></div>
                      <div><dt>执行模式</dt><dd>{{ apiExecutionModeLabel }}</dd></div>
                    </dl>
                    <footer class="sys-section-foot">
                      <button type="button" class="dash-btn secondary" @click="openSystemSection('permissions')">执行权限</button>
                      <button type="button" class="dash-btn primary" @click="openSystemSection('models')">管理模型配置</button>
                    </footer>
                  </section>
                </template>

                <!-- ===== 模型接口（直连 API） ===== -->
                <template v-else-if="systemSection === 'models'">
                  <div v-if="backendType !== 'direct_api'" class="sys-notice" role="status">
                    <AppIcon name="alert" :size="15" />
                    <span>当前 AI 后端是 {{ backendLabel(backendType) }}，这里的模型配置只在「直连 API」后端下生效。</span>
                    <button type="button" class="dash-mini-btn primary" @click="selectBackend('direct_api')">切换到直连 API</button>
                  </div>

                  <div class="sys-models-layout">
                    <aside class="dash-card sys-section sys-profile-pane" aria-label="已保存的模型配置">
                      <header class="sys-section-head compact">
                        <h2 class="sys-section-title">模型配置</h2>
                        <button type="button" class="dash-mini-btn" @click="newApiProfileDraft">
                          <AppIcon name="plus" :size="12" /> 新建
                        </button>
                      </header>
                      <ul class="sys-profile-list">
                        <li v-if="!apiConfig.id" class="sys-profile is-draft is-current">
                          <span class="sys-profile-main">
                            <span class="sys-profile-name">{{ apiConfig.name.trim() || '新建配置' }}</span>
                            <span class="sys-profile-meta">尚未保存</span>
                          </span>
                        </li>
                        <li
                          v-for="profile in apiProfiles"
                          :key="profile.id"
                          :class="['sys-profile', { 'is-current': profile.id === apiConfig.id }]"
                        >
                          <button
                            type="button"
                            class="sys-profile-main"
                            :title="profile.base_url"
                            @click="activateApiProfile(profile.id, { switchBackend: false })"
                          >
                            <span class="sys-profile-name">{{ profile.name || profile.model }}</span>
                            <span class="sys-profile-meta">{{ profile.model }}</span>
                          </button>
                          <span v-if="profile.id === activeApiProfileId" class="sys-pill">使用中</span>
                          <button
                            type="button"
                            class="sys-profile-delete"
                            title="删除模型配置"
                            :aria-label="'删除模型配置 ' + (profile.name || profile.model)"
                            :disabled="profileDeletingId === profile.id || apiProfiles.length <= 1"
                            @click="confirmDeleteApiProfile(profile)"
                          >
                            <AppIcon name="close" :size="12" />
                          </button>
                        </li>
                      </ul>
                      <p class="sys-pane-note">点一下即切换为使用中的配置；聊天输入框右下角也能一键切换。</p>
                    </aside>

                    <div class="sys-editor">
                      <section class="dash-card sys-section">
                        <header class="sys-section-head">
                          <div>
                            <h2 class="sys-section-title">{{ apiConfig.id ? '连接' : '新建模型配置' }}</h2>
                            <p class="sys-section-desc">名称、地址、密钥、模型改完后点「保存配置」；下方其他选项修改后会自动保存。</p>
                          </div>
                        </header>
                        <div class="sys-fields">
                          <label class="sys-field">
                            <span class="sys-field-label">配置名称</span>
                            <input v-model="apiConfig.name" class="dash-input" type="text" placeholder="例如：DeepSeek 工作模型" />
                          </label>
                          <div class="sys-field">
                            <label class="sys-field-label" for="sys-api-base-url">接口地址 (Base URL)</label>
                            <div class="sys-inline">
                              <input
                                id="sys-api-base-url"
                                v-model="apiConfig.base_url"
                                class="dash-input"
                                type="text"
                                placeholder="https://api.openai.com/v1"
                                @input="clearFetchedModels"
                              />
                              <button
                                type="button"
                                class="dash-btn secondary"
                                :disabled="isFetchingModels || !apiConfig.base_url.trim()"
                                @click="fetchApiModels"
                              >
                                {{ isFetchingModels ? '获取中…' : '获取模型' }}
                              </button>
                            </div>
                            <div class="sys-chips" role="group" aria-label="常用服务商预设">
                              <span class="sys-chips-label">快速填充</span>
                              <button v-for="preset in presetOptions" :key="preset.id" type="button" class="sys-chip" @click="applyPreset(preset.id)">{{ preset.label }}</button>
                            </div>
                          </div>
                          <label class="sys-field">
                            <span class="sys-field-label">API 密钥</span>
                            <input
                              v-model="apiConfig.api_key"
                              class="dash-input"
                              type="password"
                              autocomplete="new-password"
                              :placeholder="apiConfig.has_api_key ? '已保存（留空表示不修改）' : 'sk-...'"
                              @input="clearFetchedModels"
                            />
                            <span class="sys-field-hint">保存后只显示掩码，不会明文回显；本地 Ollama 等不需要密钥的接口可以留空。</span>
                          </label>
                          <div class="sys-field">
                            <label class="sys-field-label" for="sys-api-model">模型名称</label>
                            <input id="sys-api-model" v-model="apiConfig.model" class="dash-input" type="text" placeholder="gpt-4o-mini" />
                            <div v-if="fetchedModels.length > 0" class="sys-inline">
                              <select
                                class="dash-select"
                                :value="selectedFetchedModel"
                                aria-label="从接口获取到的模型"
                                @change="selectFetchedModel(($event.target as HTMLSelectElement).value)"
                              >
                                <option v-for="model in fetchedModels" :key="model" :value="model">{{ model }}</option>
                              </select>
                              <button
                                type="button"
                                class="dash-btn secondary"
                                :disabled="isAddingFetchedModel || !selectedFetchedModel"
                                @click="addFetchedModelProfile"
                              >
                                {{ isAddingFetchedModel ? '加入中…' : '加入模型列表' }}
                              </button>
                            </div>
                            <span v-if="modelFetchResult.message" :class="['sys-field-hint', modelFetchResult.success ? 'ok' : 'error']">{{ modelFetchResult.message }}</span>
                          </div>
                        </div>
                        <footer class="sys-section-foot">
                          <button
                            type="button"
                            class="dash-btn"
                            :disabled="isTestingConnection || !apiConfig.base_url || !apiConfig.model"
                            @click="testConnection"
                          >
                            {{ isTestingConnection ? '测试中…' : '测试连接' }}
                          </button>
                          <button type="button" class="dash-btn primary" :disabled="isSavingConfig" @click="saveApiConfig()">
                            {{ configSaved ? '已保存' : (isSavingConfig ? '保存中…' : '保存配置') }}
                          </button>
                        </footer>
                        <div v-if="testResult.message" :class="['sys-result', testResult.success ? 'ok' : 'error']" role="status">
                          {{ testResult.success ? '连接成功' : testResult.message }}
                        </div>
                      </section>

                      <section class="dash-card sys-section">
                        <header class="sys-section-head">
                          <div>
                            <h2 class="sys-section-title">推理与搜索</h2>
                            <p class="sys-section-desc">修改后立即保存到当前配置，下一次请求生效。</p>
                          </div>
                        </header>
                        <div class="sys-rows">
                          <div class="sys-row">
                            <div class="sys-row-label">
                              <span id="sys-thinking-label">思考深度</span>
                              <small>「自动」不额外指定；低、中、高需要模型本身支持推理强度参数。</small>
                            </div>
                            <div class="sys-segmented" role="radiogroup" aria-labelledby="sys-thinking-label">
                              <button
                                v-for="item in thinkingDepthOptions"
                                :key="item.value"
                                type="button"
                                role="radio"
                                :aria-checked="apiConfig.thinking_depth === item.value"
                                :class="{ active: apiConfig.thinking_depth === item.value }"
                                @click="updateThinkingDepth(item.value)"
                              >{{ item.label }}</button>
                            </div>
                          </div>
                          <div class="sys-row">
                            <div class="sys-row-label">
                              <label for="sys-search-provider">优先搜索源</label>
                              <small>国内网络推荐 Bing；DuckDuckGo 在国内网络下容易超时。</small>
                            </div>
                            <select
                              id="sys-search-provider"
                              class="dash-select sys-row-select"
                              :value="apiConfig.search_provider"
                              @change="updateSearchProvider(($event.target as HTMLSelectElement).value)"
                            >
                              <option v-for="item in searchProviderOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
                            </select>
                          </div>
                        </div>
                      </section>

                      <details class="dash-card sys-section sys-collapsible">
                        <summary class="sys-section-head">
                          <div>
                            <h2 class="sys-section-title">接口协议<span class="sys-tag">高级</span></h2>
                            <p class="sys-section-desc">只有接口不完全兼容 OpenAI 时才需要改。当前：{{ streamModeLabel }} · {{ apiModeLabel }}</p>
                          </div>
                          <span class="sys-collapse-caret" aria-hidden="true"><AppIcon name="chevron-down" :size="14" /></span>
                        </summary>
                        <div class="sys-rows">
                          <div class="sys-row">
                            <div class="sys-row-label">
                              <span id="sys-stream-label">流式模式</span>
                              <small>自动：先按 SSE 解析，失败再按 JSON。Auto Code 等非标准接口选「非流式」。</small>
                            </div>
                            <div class="sys-segmented" role="radiogroup" aria-labelledby="sys-stream-label">
                              <button
                                v-for="item in streamModeOptions"
                                :key="item.value"
                                type="button"
                                role="radio"
                                :aria-checked="apiConfig.stream_mode === item.value"
                                :class="{ active: apiConfig.stream_mode === item.value }"
                                @click="updateStreamMode(item.value)"
                              >{{ item.label }}</button>
                            </div>
                          </div>
                          <div class="sys-row">
                            <div class="sys-row-label">
                              <span id="sys-apimode-label">API 格式</span>
                              <small>大多数服务选 Chat Completions（/v1/chat/completions）；Responses 是 OpenAI 的新格式（/v1/responses）。</small>
                            </div>
                            <div class="sys-segmented" role="radiogroup" aria-labelledby="sys-apimode-label">
                              <button
                                v-for="item in apiModeOptions"
                                :key="item.value"
                                type="button"
                                role="radio"
                                :aria-checked="apiConfig.api_mode === item.value"
                                :class="{ active: apiConfig.api_mode === item.value }"
                                @click="updateApiMode(item.value)"
                              >{{ item.label }}</button>
                            </div>
                          </div>
                        </div>
                        <footer class="sys-section-foot">
                          <span class="sys-foot-note">不确定选哪种时，跑一次深度检测看推荐模式。</span>
                          <button
                            type="button"
                            class="dash-btn"
                            :disabled="isTestingCompatibility || !apiConfig.base_url || !apiConfig.model"
                            @click="testCompatibility"
                          >
                            {{ isTestingCompatibility ? '检测中…' : '深度测试兼容性' }}
                          </button>
                        </footer>
                        <div
                          v-if="compatibilityResult"
                          class="dash-compat-result sys-compat"
                          :class="compatibilityResult.http_ok ? 'success' : 'error'"
                        >
                          <h4>兼容性检测报告</h4>
                          <div class="compat-grid">
                            <div class="compat-item">
                              <span class="compat-label">HTTP 状态:</span>
                              <span :class="compatibilityResult.http_ok ? 'pass' : 'fail'">
                                {{ compatibilityResult.http_ok ? '通过' : '失败' }} ({{ compatibilityResult.http_status }})
                              </span>
                            </div>
                            <div class="compat-item">
                              <span class="compat-label">SSE 格式:</span>
                              <span :class="compatibilityResult.is_sse_format ? 'pass' : 'fail'">
                                {{ compatibilityResult.is_sse_format ? '支持' : '不支持' }}
                              </span>
                            </div>
                            <div class="compat-item">
                              <span class="compat-label">JSON 格式:</span>
                              <span :class="compatibilityResult.is_json_format ? 'pass' : 'fail'">
                                {{ compatibilityResult.is_json_format ? '支持' : '不支持' }}
                              </span>
                            </div>
                            <div class="compat-item">
                              <span class="compat-label">内容提取:</span>
                              <span :class="compatibilityResult.content_extracted ? 'pass' : 'fail'">
                                {{ compatibilityResult.content_extracted ? '成功' : '失败' }}
                              </span>
                            </div>
                            <div v-if="compatibilityResult.content_extracted" class="compat-item">
                              <span class="compat-label">提取内容:</span>
                              <span class="compat-content">{{ compatibilityResult.content_extracted }}</span>
                            </div>
                            <div v-if="compatibilityResult.recommended_mode" class="compat-item">
                              <span class="compat-label">推荐模式:</span>
                              <span class="compat-recommend">{{ compatibilityResult.recommended_mode === 'stream' ? '流式 (SSE)' : compatibilityResult.recommended_mode === 'non_stream' ? '非流式 (JSON)' : '自动' }}</span>
                            </div>
                            <div v-if="compatibilityResult.errors && compatibilityResult.errors.length > 0" class="compat-item">
                              <span class="compat-label">错误:</span>
                              <ul class="compat-errors">
                                <li v-for="(err, idx) in compatibilityResult.errors" :key="idx">{{ err }}</li>
                              </ul>
                            </div>
                          </div>
                          <details v-if="compatibilityResult.raw_preview" class="compat-raw">
                            <summary>查看原始响应（前 500 字符）</summary>
                            <pre>{{ compatibilityResult.raw_preview }}</pre>
                          </details>
                          <p class="compat-time">响应时间: {{ compatibilityResult.response_time_ms }} ms</p>
                        </div>
                      </details>
                    </div>
                  </div>
                </template>

                <!-- ===== 执行权限 ===== -->
                <template v-else-if="systemSection === 'permissions'">
                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">直连 API 执行模式</h2>
                        <p class="sys-section-desc">
                          作用于模型配置「{{ apiConfig.name || apiConfig.model || '未命名' }}」，修改后立即保存。
                          <template v-if="backendType !== 'direct_api'">当前后端是 {{ backendLabel(backendType) }}，切换到直连 API 后生效。</template>
                        </p>
                      </div>
                    </header>
                    <div class="sys-choice-grid" role="radiogroup" aria-label="直连 API 执行模式">
                      <button
                        v-for="mode in executionModeOptions"
                        :key="mode.value"
                        type="button"
                        role="radio"
                        :aria-checked="apiConfig.execution_mode === mode.value"
                        :class="['sys-choice', { active: apiConfig.execution_mode === mode.value, risky: mode.value === 'unreviewed' }]"
                        @click="updateExecutionMode(mode.value)"
                      >
                        <strong>{{ mode.label }}</strong>
                        <small>{{ mode.desc }}</small>
                      </button>
                    </div>
                    <div v-if="apiConfig.execution_mode === 'custom'" class="sys-subsection">
                      <div class="sys-subsection-head">
                        <span class="sys-subsection-title">免确认工具</span>
                        <span class="sys-subsection-desc">勾选的工具执行前不再弹窗确认，勾选后立即保存。</span>
                      </div>
                      <div class="sys-check-grid">
                        <label v-for="tool in toolPermissionOptions" :key="tool.id" class="sys-check">
                          <input
                            type="checkbox"
                            :checked="isToolAutoApproved(tool.id)"
                            @change="toggleAutoApprovedTool(tool.id, ($event.target as HTMLInputElement).checked)"
                          />
                          <span>{{ tool.label }}</span>
                        </label>
                      </div>
                    </div>
                    <div v-if="!apiConfig.id" class="sys-result warn">当前是还没保存的新建配置：执行模式会在你保存这份配置时一起保存。</div>
                  </section>

                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">桌面控制</h2>
                        <p class="sys-section-desc">只对直连 API 后端生效。</p>
                      </div>
                    </header>
                    <div class="sys-rows">
                      <div class="sys-row">
                        <div class="sys-row-label">
                          <label for="sys-computer-use">Computer Use</label>
                          <small>开启后 AI 可以截屏、聚焦窗口、打开浏览器、操作鼠标和键盘；只在需要桌面自动化时开启。<template v-if="computerUseSaved"> 已保存。</template></small>
                        </div>
                        <input
                          id="sys-computer-use"
                          type="checkbox"
                          class="dash-toggle"
                          :checked="computerUseEnabled"
                          @change="updateComputerUseEnabled(($event.target as HTMLInputElement).checked)"
                        />
                      </div>
                    </div>
                  </section>

                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">本机 CLI 默认权限</h2>
                        <p class="sys-section-desc">主对话使用对应后端时、以及三个 agent 窗口的默认档位；单个会话可以在聊天栏里单独覆盖。</p>
                      </div>
                    </header>
                    <div class="sys-permission-stack">
                      <AgentPermissionControl backend="claude_code" />
                      <AgentPermissionControl backend="codex" />
                      <AgentPermissionControl backend="dsh" />
                    </div>
                  </section>
                </template>

                <!-- ===== 角色设定 ===== -->
                <template v-else-if="systemSection === 'persona'">
                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">性格</h2>
                        <p class="sys-section-desc">决定它说话的语气，从下一条消息开始生效。</p>
                      </div>
                    </header>
                    <div class="sys-choice-grid" role="radiogroup" aria-label="性格">
                      <button
                        v-for="item in personalities"
                        :key="item.id"
                        type="button"
                        role="radio"
                        :aria-checked="currentPersonality === item.id"
                        :class="['sys-choice', { active: currentPersonality === item.id }]"
                        @click="selectPersonality(item.id)"
                      >
                        <strong>{{ item.name }}</strong>
                        <small>{{ item.desc }}</small>
                      </button>
                    </div>
                  </section>

                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">职业</h2>
                        <p class="sys-section-desc">决定它擅长帮你做哪类事。</p>
                      </div>
                    </header>
                    <div class="sys-choice-grid" role="radiogroup" aria-label="职业">
                      <button
                        v-for="item in professions"
                        :key="item.id"
                        type="button"
                        role="radio"
                        :aria-checked="currentProfession === item.id"
                        :class="['sys-choice', { active: currentProfession === item.id }]"
                        @click="selectProfession(item.id)"
                      >
                        <strong>{{ item.name }}</strong>
                        <small>{{ item.desc }}</small>
                      </button>
                    </div>
                  </section>

                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">自定义系统提示</h2>
                        <p class="sys-section-desc">追加到系统提示末尾，对所有对话生效；留空则不追加。</p>
                      </div>
                    </header>
                    <textarea
                      v-model="customPromptDraft"
                      class="dash-textarea sys-textarea"
                      rows="5"
                      aria-label="自定义系统提示"
                      placeholder="例如：你每次回复都以『汪！』开头..."
                    />
                    <footer class="sys-section-foot">
                      <span class="sys-foot-note">{{ customPromptDraft.trim().length }} 字</span>
                      <button type="button" class="dash-btn primary" @click="saveCustomPrompt">
                        {{ customPromptSaved ? '已保存' : '保存提示' }}
                      </button>
                    </footer>
                  </section>
                </template>

                <!-- ===== 技能 ===== -->
                <template v-else-if="systemSection === 'skills'">
                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">技能</h2>
                        <p class="sys-section-desc">同一时间只能激活一个技能；新建和编辑技能请到「设置」窗口。</p>
                      </div>
                      <span v-if="skillsStore.activeSkill" class="sys-pill">已激活：{{ skillsStore.activeSkill.name }}</span>
                    </header>
                    <p v-if="skillsStore.skills.length === 0" class="sys-empty">还没有任何技能。</p>
                    <ul v-else class="sys-skill-list">
                      <li v-for="s in skillsStore.skills" :key="s.id" :class="['sys-skill', { active: s.is_active }]">
                        <div class="sys-skill-copy">
                          <span class="sys-skill-name">{{ s.name }}</span>
                          <span class="sys-skill-desc">{{ s.description || '（无描述）' }}</span>
                        </div>
                        <button
                          type="button"
                          :class="['dash-mini-btn', { primary: !s.is_active }]"
                          @click="activateSkill(s.is_active ? null : s.id)"
                        >
                          {{ s.is_active ? '取消激活' : '激活' }}
                        </button>
                      </li>
                    </ul>
                  </section>
                </template>

                <!-- ===== 天气提醒 ===== -->
                <template v-else>
                  <section class="dash-card sys-section">
                    <header class="sys-section-head">
                      <div>
                        <h2 class="sys-section-title">天气栏与每日问候</h2>
                        <p class="sys-section-desc">开启后首页显示天气，并每天发送一次天气问候。</p>
                      </div>
                    </header>
                    <div class="sys-rows">
                      <div class="sys-row">
                        <div class="sys-row-label">
                          <label for="sys-weather-enabled">启用天气</label>
                          <small>关闭后不再请求天气接口。</small>
                        </div>
                        <input id="sys-weather-enabled" v-model="weatherConfig.enabled" type="checkbox" class="dash-toggle" />
                      </div>
                    </div>
                    <div class="sys-fields sys-fields-2">
                      <label class="sys-field">
                        <span class="sys-field-label">城市 / 地区</span>
                        <input v-model="weatherConfig.location" class="dash-input" type="text" placeholder="建议填中文城市，例如 上海" />
                      </label>
                      <label class="sys-field">
                        <span class="sys-field-label">wttr.in 兼容接口</span>
                        <input v-model="weatherConfig.api_url" class="dash-input" type="text" placeholder="http://wttr.in 或 https://example.com/{location}?format=j1" />
                      </label>
                    </div>
                    <div v-if="weatherInfo" class="sys-result info">当前天气：{{ formatWeatherDisplay(weatherInfo) }} · {{ weatherInfo.description }}</div>
                    <div v-if="weatherConfigResult.message" :class="['sys-result', weatherConfigResult.success ? 'ok' : 'error']">
                      {{ weatherConfigResult.message }}
                    </div>
                    <footer class="sys-section-foot">
                      <button
                        type="button"
                        class="dash-btn"
                        :disabled="isTestingWeatherConfig || !weatherConfig.api_url.trim()"
                        @click="testWeatherConfig"
                      >
                        {{ isTestingWeatherConfig ? '测试中…' : '测试接口' }}
                      </button>
                      <button
                        type="button"
                        class="dash-btn secondary"
                        :disabled="isWeatherLoading || !weatherConfig.enabled"
                        @click="refreshWeather"
                      >
                        {{ isWeatherLoading ? '刷新中…' : '立即刷新' }}
                      </button>
                      <button type="button" class="dash-btn primary" :disabled="isSavingWeatherConfig" @click="saveWeatherConfig">
                        {{ isSavingWeatherConfig ? '保存中…' : '保存天气设置' }}
                      </button>
                    </footer>
                  </section>
                </template>
              </div>
            </Transition>
          </div>

          <!-- ========== 关于 ========== -->
          <div v-else-if="activePage === 'about'" key="about">
            <div class="page-header">
              <div class="page-kicker">About</div>
              <h1 class="page-title">关于</h1>
              <p class="page-subtitle">应用信息</p>
            </div>

            <div class="dash-card about-card">
              <div class="about-logo"><AppIcon name="brand-pet" :size="38" /></div>
              <div class="about-name">AI Desktop Pet</div>
              <div class="about-version">v0.1.0</div>
              <p class="about-desc">
                一个可爱的 AI 桌面伙伴，支持智能对话、语音交互、多主题皮肤和丰富的个性化设置。
                基于 Tauri + Vue 3 构建，轻量高效。
              </p>
            </div>
          </div>
        </Transition>

        <!-- 工具确认悬浮层 (全局，任何页面可见) -->
        <Transition name="slide-up">
          <div v-if="pendingConfirm" class="dash-floating-confirm-overlay">
            <div class="dash-tool-confirm-card">
              <div>
                <strong>{{ pendingConfirm.summary || pendingConfirm.tool_name }}</strong>
                <p v-if="pendingConfirm.command">命令：{{ pendingConfirm.command }}</p>
                <p v-if="pendingConfirm.path">路径：{{ pendingConfirm.path }}</p>
                <div v-if="pendingConfirm.arguments" class="dash-tool-args">
                  <span class="args-label">参数:</span>
                  <pre class="args-code"><code>{{ pendingConfirm.arguments }}</code></pre>
                </div>
              </div>
              <div class="dash-tool-confirm-actions">
                <button class="dash-btn secondary" :disabled="confirmSubmitting" @click="handleDashboardToolConfirm(false)">拒绝</button>
                <button class="dash-btn primary" :disabled="confirmSubmitting" @click="handleDashboardToolConfirm(true)">允许</button>
              </div>
            </div>
          </div>
        </Transition>

        <!-- 智能体主动询问弹窗 (ask_user, 全局，任何页面可见) -->
        <Transition name="slide-up">
          <div v-if="pendingQuestion" class="dash-floating-confirm-overlay">
            <UserQuestion :question="pendingQuestion" @answered="questionAnswered" />
          </div>
        </Transition>
      </div>
    </div>
  </div>
</template>
