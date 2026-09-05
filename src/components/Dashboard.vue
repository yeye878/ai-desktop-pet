<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import AppIcon from "./AppIcon.vue";
import CustomPixelPetWorkshop from "./CustomPixelPetWorkshop.vue";
import EvolutionCenter from "./EvolutionCenter.vue";
import PetCanvas from "./PetCanvas.vue";
import { usePetStore, THEMES, FONT_COLORS, resolveSkinId } from "../stores/pet";
import { useSkillsStore, type Skill } from "../stores/skills";
import { useChatStore, type Message, type MessageAgent, type QuotedMessage } from "../stores/chat";
import { useAgentsStore, type Agent } from "../stores/agents";
import { useEvolutionStore } from "../stores/evolution";
import {
  parseQuoteSegments,
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
  parseAgentHeaders,
  splitMentionSegments,
} from "../services/mentions";
import { AVAILABLE_TOOL_NAMES, toolLabel } from "../services/tools";
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

// === 导航 ===
type NavPage = "home" | "chat" | "evolution" | "memory" | "agents" | "appearance" | "voice" | "system" | "about";
const activePage = ref<NavPage>("home");
const isPetActive = ref(false);
const skillsStore = useSkillsStore();
const agentsStore = useAgentsStore();
const evolutionStore = useEvolutionStore();

function openEvolutionPage() {
  activePage.value = "evolution";
  void evolutionStore.loadAll();
}

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
};

type ToolConfirmPayload = {
  id: string;
  tool_name: string;
  arguments: string;
  summary?: string;
  command?: string | null;
  path?: string | null;
};

type AskUserOption = {
  label: string;
  description?: string | null;
};

type AskUserPayload = {
  id: string;
  question: string;
  options: AskUserOption[];
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
const isPreviewing = ref(false);

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
  category: "preference",
});
const isSavingMemory = ref(false);
const memorySaved = ref(false);
const memoryDeletingId = ref<number | null>(null);
const memorySearchQuery = ref("");
const memoryCategoryFilter = ref("all");
const editMemoryModalOpen = ref(false);
const editingMemoryDraft = ref<{
  id: number;
  key: string;
  value: string;
  category: string;
} | null>(null);
const isUpdatingMemory = ref(false);

const filteredMemories = computed(() => {
  let list = memories.value;
  if (memoryCategoryFilter.value !== "all") {
    list = list.filter((m) => m.category === memoryCategoryFilter.value);
  }
  const q = memorySearchQuery.value.trim().toLowerCase();
  if (q) {
    list = list.filter(
      (m) =>
        m.key.toLowerCase().includes(q) ||
        m.value.toLowerCase().includes(q) ||
        memoryCategoryLabel(m.category).toLowerCase().includes(q)
    );
  }
  return list;
});
const isScanningShortcuts = ref(false);
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

const thinkingDepthOptions = [
  { value: "auto", label: "自动" },
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高" },
];

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

const toolPermissionOptions = AVAILABLE_TOOL_NAMES.map((name) => ({
  id: name,
  label: toolLabel(name),
}));

const currentModelLabel = computed(() => {
  if (backendType.value === "direct_api") {
    return apiConfig.value.model || "未选择模型";
  }
  return currentModel.value || "Claude Code";
});

const currentExecutionModeLabel = computed(() => {
  if (backendType.value !== "direct_api") return "Claude Code";
  return executionModeOptions.find((item) => item.value === apiConfig.value.execution_mode)?.label || "普通模式";
});

const contextUsageLabel = computed(() => {
  const chars = chat.messages.reduce((sum, msg) => {
    return sum + msg.content.length + (msg.thinking?.length || 0);
  }, thinkingContent.value.length + streamingAnswer.value.length);
  if (chars === 0) return "0 字符";
  const approxTokens = Math.max(1, Math.ceil(chars / 2));
  return `${chars} 字符 / 约 ${approxTokens} tokens`;
});

const dashSlashQuery = computed(() => {
  if (chat.isLoading) return null;
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
const dashMentionDismissed = ref(false);

const dashMentionQuery = computed(() => {
  if (chat.isLoading) return null;
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
  () => !dashMentionDismissed.value && dashMentionQuery.value !== null && dashMentionItems.value.length > 0,
);

const knownAgentNames = computed(() => agentsStore.agents.map((a) => a.name));

/** 用户消息里被 @ 的智能体名字（用于发送时传给后端） */
function dashMentionNames(text: string): string[] {
  return extractMentionNames(text, knownAgentNames.value).filter((name) => agentsStore.findByName(name));
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

function openAgentEditor(agent?: Agent) {
  if (agent) {
    editingAgentId.value = agent.id;
    agentDraft.value = {
      id: agent.id,
      name: agent.name,
      avatar: agent.avatar || "🤖",
      description: agent.description,
      system_prompt: agent.system_prompt,
      model: agent.model,
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
      id: agentDraft.value.id.trim(),
      name,
      avatar: agentDraft.value.avatar.trim() || "🤖",
      description: agentDraft.value.description.trim(),
      system_prompt: agentDraft.value.system_prompt.trim(),
      model: agentDraft.value.model.trim(),
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

function thinkingDepthLabel(value: string) {
  return thinkingDepthOptions.find((item) => item.value === value)?.label || "自动";
}

function searchProviderLabel(value: string) {
  return searchProviderOptions.find((item) => item.value === value)?.label || "Bing";
}

const quickActions = [
  { id: "wave", icon: "wave", label: "打招呼", desc: "挥挥手，进入陪伴状态" },
  { id: "happy", icon: "smile", label: "开心一下", desc: "给它加一点元气" },
  { id: "sleep", icon: "moon", label: "去睡觉", desc: "切换到安静休息" },
  { id: "wake", icon: "bell", label: "叫醒它", desc: "回到待命状态" },
  { id: "new-chat", icon: "plus", label: "新对话", desc: "保存摘要并重开上下文" },
  { id: "clear", icon: "trash", label: "清空对话", desc: "整理上下文与摘要" },
];

const missionEntries = [
  { page: "chat" as NavPage, icon: "chat", title: "对话", desc: "派发任务或闲聊" },
  { page: "appearance" as NavPage, icon: "appearance", title: "外观", desc: "皮肤、字体和背景" },
  { page: "voice" as NavPage, icon: "voice", title: "语音", desc: "快捷键、人声和试听" },
];

const activeActionId = ref("");
const actionFeedback = ref("点一下动作卡片，小家伙会在这里回应你。");
let actionFeedbackTimer: ReturnType<typeof setTimeout> | null = null;

// === 预设 ===
const PRESETS: Record<string, { base_url: string; model: string }> = {
  openai: { base_url: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  deepseek: { base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  qwen: { base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen-plus" },
  ollama: { base_url: "http://localhost:11434/v1", model: "qwen2.5:7b" },
};

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
    } else {
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
    if (type === "claude_code") {
      await loadCurrentModel();
      await loadClaudeStatus();
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

async function activateApiProfile(id: string) {
  if (!id) return;
  try {
    const profile = await invoke<ApiProfile>("set_active_api_profile", { id });
    applyApiConfig(profile);
    clearFetchedModels();
    currentModel.value = `直连 API: ${apiConfig.value.model}`;
    await currentWindow.emit("api-config-changed");
    if (backendType.value !== "direct_api") {
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

async function broadcastChatBg(bg: string, customImage: string) {
  await currentWindow.emit("chat-bg-changed", { bg, customImage });
  const [chatWin, petWin] = await Promise.all([
    WebviewWindow.getByLabel("chat"),
    WebviewWindow.getByLabel("pet"),
  ]);
  if (chatWin) await chatWin.emit("chat-bg-changed", { bg, customImage });
  if (petWin) await petWin.emit("appearance-changed");
}

async function selectBg(bg: string) {
  chatBg.value = bg;
  localStorage.setItem(BG_KEY, bg);
  await broadcastChatBg(bg, customBgImage.value);
}

function chooseBgImage() { bgInputRef.value?.click(); }

function onBgImageSelected(event: Event) {
  const el = event.target as HTMLInputElement;
  const file = el.files?.[0];
  el.value = "";
  if (!file || !file.type.startsWith("image/")) return;
  const reader = new FileReader();
  reader.onload = async () => {
    const url = String(reader.result || "");
    customBgImage.value = url;
    localStorage.setItem(CUSTOM_BG_KEY, url);
    chatBg.value = "custom";
    localStorage.setItem(BG_KEY, "custom");
    await broadcastChatBg("custom", url);
  };
  reader.readAsDataURL(file);
}

async function clearCustomBg() {
  customBgImage.value = "";
  localStorage.removeItem(CUSTOM_BG_KEY);
  await selectBg("none");
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

function openEditMemoryModal(item: MemoryItem) {
  editingMemoryDraft.value = {
    id: item.id,
    key: item.key,
    value: item.value,
    category: item.category,
  };
  editMemoryModalOpen.value = true;
}

async function saveEditedMemory() {
  if (!editingMemoryDraft.value || isUpdatingMemory.value) return;
  const key = editingMemoryDraft.value.key.trim();
  const value = editingMemoryDraft.value.value.trim();
  if (!key || !value) return;

  isUpdatingMemory.value = true;
  try {
    const updated = await invoke<MemoryItem>("update_memory", {
      id: editingMemoryDraft.value.id,
      category: editingMemoryDraft.value.category || "preference",
      key,
      value,
    });
    memories.value = memories.value.map((m) => (m.id === updated.id ? updated : m));
    editMemoryModalOpen.value = false;
    editingMemoryDraft.value = null;
  } catch (e) {
    alert("更新记忆失败: " + e);
  } finally {
    isUpdatingMemory.value = false;
  }
}

async function handleScanShortcuts() {
  if (isScanningShortcuts.value) return;
  isScanningShortcuts.value = true;
  try {
    const res = await invoke<{ count: number }>("scan_and_update_desktop_shortcuts");
    await loadMemories();
    alert(`已成功扫描并同步了 ${res.count} 个桌面快捷应用！现在您可以直接对桌宠说“打开微信”或“打开VSCode”了。`);
  } catch (e) {
    alert("扫描快捷方式失败: " + e);
  } finally {
    isScanningShortcuts.value = false;
  }
}

function memoryCategoryLabel(category: string) {
  const map: Record<string, string> = {
    preference: "偏好习惯",
    fact: "个人事实",
    habit: "作息习惯",
    work_domain: "工作领域",
    note: "随手备忘",
    manual: "手动设定",
    conversation: "对话摘要",
    app_path: "注册应用",
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
if (isPreviewing.value) { ttsPlayer.stop(); isPreviewing.value = false; return; }
  isPreviewing.value = true;
  try { await ttsPlayer.speak("你好，我是你的桌宠伙伴，很高兴认识你。", ttsSettings.value); }
  catch (e: any) { if (e.message !== "Aborted") alert("试听失败: " + (e.message || e)); }
  isPreviewing.value = false;
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

// === 快捷操作 ===
function showActionFeedback(actionId: string) {
  const action = quickActions.find((item) => item.id === actionId);
  activeActionId.value = actionId;
  actionFeedback.value = action
    ? `已触发「${action.label}」，桌宠正在同步状态。`
    : "桌宠正在同步状态。";

  if (actionFeedbackTimer) clearTimeout(actionFeedbackTimer);
  actionFeedbackTimer = setTimeout(() => {
    activeActionId.value = "";
    actionFeedback.value = "点一下动作卡片，小家伙会在这里回应你。";
  }, 2600);
}

async function petAction(name: string) {
  showActionFeedback(name);
  const stateMap: Record<string, string> = {
    wave: "waving", happy: "happy", sleep: "sleeping", wake: "idle",
  };
  const state = stateMap[name];
  if (state) {
    try { await invoke("set_pet_state", { newState: state }); } catch {}
  }
  if (name === "new-chat") {
    try {
      await invoke("start_new_conversation");
      // 新对话：保留聊天记录显示，仅重置 AI 状态
      chat.isLoading = false;
      thinkingContent.value = "";
      streamingAnswer.value = "";
      pendingConfirm.value = null;
      // 插入分割线，标记新对话开始
      if (chat.messages.length > 0) {
        chat.addSystemMessage("新对话");
      }
      await loadMemories();
    } catch {}
  }
  if (name === "clear") {
    try {
      await invoke("clear_chat_history");
      resetDashChatUi();
      await loadMemories();
      await currentWindow.emit("chat-history-cleared");
    } catch {}
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
watch(chatInput, () => {
  dashMentionDismissed.value = false;
});
const dashChatInputRef = ref<HTMLInputElement | null>(null);
const dashSlashSelectedIndex = ref(0);
const dashMentionSelectedIndex = ref(0);
const thinkingContent = ref("");
const streamingAnswer = ref("");
const toolEvents = ref<ToolEvent[]>([]);
const pendingConfirm = ref<ToolConfirmPayload | null>(null);
// === 智能体主动询问弹窗 (ask_user) ===
const pendingQuestion = ref<AskUserPayload | null>(null);
const askUserSelected = ref("");
const askUserAnswer = ref("");
const askUserSubmitting = ref(false);
const dashChatMessagesRef = ref<HTMLDivElement | null>(null);
const dashChatEndRef = ref<HTMLDivElement | null>(null);
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
let unlistenToolConfirm: UnlistenFn | null = null;
let unlistenToolConfirmResolved: UnlistenFn | null = null;
let unlistenAskUser: UnlistenFn | null = null;
let unlistenAskUserResolved: UnlistenFn | null = null;
let unlistenScheduledTasksChanged: UnlistenFn | null = null;
let unlistenScheduledTaskTriggered: UnlistenFn | null = null;
let unlistenWeatherUpdate: UnlistenFn | null = null;
let unlistenDragDrop: UnlistenFn | null = null;
let unlistenVoiceSettingsChanged: UnlistenFn | null = null;
let unlistenTtsSettingsChanged: UnlistenFn | null = null;
let unlistenNavigate: UnlistenFn | null = null;

// === 文件拖拽与预加载相关数据 ===
interface PendingFile {
  id: string;
  path: string;
  name: string;
  size: number;
  extension: string;
  isImage: boolean;
  previewUrl: string;
  status: "loading" | "ready" | "error";
  errorMessage?: string;
}
interface RustFileMetadata {
  path: string;
  name: string;
  size: number;
  extension: string;
  exists: boolean;
  is_file: boolean;
}
const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
const MAX_FILE_SIZE = 50 * 1024 * 1024;
const MAX_FILES = 5;
const pendingFiles = ref<PendingFile[]>([]);
const fileValidationError = ref("");
const isFileOver = ref(false);
let fileErrorTimer: ReturnType<typeof setTimeout> | null = null;

function showFileError(msg: string) {
  fileValidationError.value = msg;
  if (fileErrorTimer) clearTimeout(fileErrorTimer);
  fileErrorTimer = setTimeout(() => {
    fileValidationError.value = "";
  }, 3000);
}

async function addToPendingFiles(paths: string[]) {
  if (fileErrorTimer) clearTimeout(fileErrorTimer);
  fileValidationError.value = "";

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
        await processNormalFiles([p]);
      }
    }
    return;
  }

  await processNormalFiles(paths);
}

async function processNormalFiles(paths: string[]) {
  if (pendingFiles.value.length + paths.length > MAX_FILES) {
    fileValidationError.value = `最多同时添加 ${MAX_FILES} 个文件`;
    fileErrorTimer = setTimeout(() => {
      fileValidationError.value = "";
    }, 3000);
    return;
  }

  try {
    const metadataList = await invoke<RustFileMetadata[]>("get_file_metadata", { paths });

    for (const meta of metadataList) {
      if (!meta.exists || !meta.is_file) {
        showFileError(`文件不存在或是目录: ${meta.name}`);
        continue;
      }
      if (meta.size > MAX_FILE_SIZE) {
        showFileError(`文件过大: ${meta.name} (最大 50MB)`);
        continue;
      }

      const isImage = IMAGE_EXTENSIONS.includes(meta.extension);
      let previewUrl = "";

      if (isImage) {
        previewUrl = convertFileSrc(meta.path);
      }

      pendingFiles.value.push({
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        path: meta.path,
        name: meta.name,
        size: meta.size,
        extension: meta.extension,
        isImage,
        previewUrl,
        status: "ready",
      });
    }
  } catch (err) {
    showFileError(`获取文件信息失败: ${err}`);
  }
}

function removePendingFile(id: string) {
  pendingFiles.value = pendingFiles.value.filter((f) => f.id !== id);
}

function clearPendingFiles() {
  pendingFiles.value = [];
  fileValidationError.value = "";
}

async function handleImageError(file: PendingFile) {
  try {
    const dataUrl = await invoke<string>("read_file_as_data_url", { path: file.path });
    file.previewUrl = dataUrl;
  } catch {
    file.status = "error";
    file.errorMessage = "预览不可用";
  }
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// 用 unicode 字符作为文件图标以兼容各平台
function getFileIcon(ext: string): string {
  const map: Record<string, string> = {
    pdf: "\u{1F4C4}",
    doc: "\u{1F4DD}",
    docx: "\u{1F4DD}",
    xls: "\u{1F4CA}",
    xlsx: "\u{1F4CA}",
    csv: "\u{1F4CA}",
    ppt: "\u{1F4D1}",
    pptx: "\u{1F4D1}",
    zip: "\u{1F4E6}",
    rar: "\u{1F4E6}",
    "7z": "\u{1F4E6}",
    mp3: "\u{1F3B5}",
    wav: "\u{1F3B5}",
    flac: "\u{1F3B5}",
    mp4: "\u{1F3AC}",
    avi: "\u{1F3AC}",
    mkv: "\u{1F3AC}",
    txt: "\u{1F4C3}",
    md: "\u{1F4C3}",
    json: "\u{1F4C3}",
    js: "\u{1F4BB}",
    ts: "\u{1F4BB}",
    py: "\u{1F4BB}",
    rs: "\u{1F4BB}",
    go: "\u{1F4BB}",
    java: "\u{1F4BB}",
  };
  return map[ext] || "\u{1F4CE}";
}

function buildMessageWithFiles(text: string, files: PendingFile[]): string {
  if (files.length === 0) return text;

  const imageCount = files.filter((f) => f.isImage).length;
  const regularFileCount = files.length - imageCount;
  const fileLines = files
    .map((f, i) => {
      const typeLabel = f.isImage ? "图片" : "文件";
      const icon = f.isImage ? "\u{1F4F7}" : getFileIcon(f.extension);
      return `${i + 1}. ${icon} ${typeLabel}: "${f.name}" (路径: ${f.path}, 大小: ${formatFileSize(f.size)})`;
    })
    .join("\n");

  const hints = [
    imageCount > 0 ? "图片会随消息作为视觉输入发送，请直接观察图片内容。" : "",
    regularFileCount > 0 ? "普通文件可使用 read_file 工具读取内容。" : "",
  ].filter(Boolean).join(" ");
  const fileBlock = `\n\n---\n[用户附加了以下本地文件]\n${fileLines}\n\n提示: ${hints}`;

  return text ? `${text}${fileBlock}` : `[用户附加了文件，但没有输入文字]${fileBlock}`;
}

function handleDragDropEvent(event: any) {
  // 检查是否是 .lnk 文件（快捷方式在任何页面都可注册）
  const hasLnk = event.type === "drop" && event.paths?.some((p: string) => p.toLowerCase().endsWith(".lnk"));

  if (activePage.value !== "chat" && !hasLnk) {
    isFileOver.value = false;
    return;
  }
  if (event.type === "enter" || event.type === "over") {
    isFileOver.value = true;
    return;
  }
  isFileOver.value = false;
  if (event.type === "drop" && event.paths?.length > 0) {
    void addToPendingFiles(event.paths);
  }
}

async function refreshChatState() {
  try {
    const history = await invoke<any[]>("get_chat_history");
    chat.setMessages(history.map(item => ({
      role: item.role === "assistant" ? "assistant" as const : "user" as const,
      content: item.content || (item.quoted_content ? "（引用消息）" : item.content),
      thinking: item.thinking || undefined,
      timestamp: typeof item.created_at === 'number' ? item.created_at : new Date(item.created_at).getTime(),
      quote: item.quoted_content
        ? {
            role: item.quoted_role === "assistant" ? "assistant" as const : "user" as const,
            content: item.quoted_content,
          }
        : undefined,
      agent: item.agent_name
        ? {
            id: item.agent_id || undefined,
            name: item.agent_name,
            avatar: item.agent_avatar || undefined,
          }
        : undefined,
    })));
    scrollDashChatToBottom();
  } catch {}
}

function resetDashChatUi() {
  chat.clearMessages();
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;
  pendingQuestion.value = null;
}

async function abortAi() {
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;  // Clear pending tool confirmation on abort
  pendingQuestion.value = null; // Clear pending ask_user popup on abort
  try {
    await invoke("abort_ai");
  } catch {}
}

async function sendDashboardMessage() {

  const text = chatInput.value.trim();
  const files = [...pendingFiles.value];
  const quote = pendingQuote.value ? { ...pendingQuote.value } : undefined;

  if (!text && files.length === 0 && !quote) return;
  if (chat.isLoading) return;
  closeDashMessageMenu();
  if (voiceSettings.value.enabled && voiceSettings.value.autoSpeak) {
    ttsPlayer.preparePlayback();
  }

  const fullMessage = buildMessageWithFiles(text, files);

  const fileAttachments =
    files.length > 0
      ? files.map((f) => ({ name: f.name, isImage: f.isImage, extension: f.extension }))
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
  chat.isLoading = true;
  resetLoadingTimeout(); // 启动超时保底
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;
  pet.setState("thinking");

  // Broadcast user message to other windows (like the pet chat bubble)
  await currentWindow.emit("sync-chat-message", { role: "user", content: displayText, files: fileAttachments, quote });

  try {
    await invoke("send_to_ai", {
      message: fullMessage,
      attachments: aiAttachments,
      quotedRole: quote?.role,
      quotedContent: quote ? truncateQuoteContent(quote.content) : undefined,
      mentionAgents: dashMentionNames(text),
    });
  } catch (err) {
    chat.isLoading = false;
    streamingAnswer.value = "";
    chat.addMessage("assistant", `出错了: ${err}`);
    pet.setState("confused");
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
      activePage.value = "system";
      await skillsStore.load();
      break;
    case "agents":
      openAgentsPage();
      break;
    case "evolution":
      openEvolutionPage();
      break;
    case "team":
      chatInput.value = "/team ";
      activePage.value = "chat";
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
    await invoke("start_new_conversation");
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    pendingConfirm.value = null;
    toolEvents.value = [];
    if (chat.messages.length > 0) {
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
    resetDashChatUi();
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
  chatInput.value = insertMentionAt(chatInput.value, query, agent.name);
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
      dashMentionDismissed.value = true;
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
  activePage.value = "chat";
  void nextTick(() => dashChatInputRef.value?.focus());
}

/** 清除待发送的引用 */
function clearPendingQuote() {
  pendingQuote.value = null;
}

async function resendDashMessage(msg: Message) {
  const text = msg.content.trim();
  if (!text || chat.isLoading) return;
  closeDashMessageMenu();
  activePage.value = "chat";
  chatInput.value = text;
  await nextTick();
  await sendDashboardMessage();
}

function upsertToolEvent(event: ToolEvent) {
  const index = toolEvents.value.findIndex((item) => item.id === event.id);
  if (index >= 0) {
    toolEvents.value[index] = { ...toolEvents.value[index], ...event };
  } else {
    toolEvents.value.push(event);
  }
}

function formatArguments(argsStr?: string) {
  if (!argsStr || !argsStr.trim()) return "";
  try {
    const parsed = JSON.parse(argsStr);
    return JSON.stringify(parsed, null, 2);
  } catch {
    return argsStr;
  }
}

async function handleDashboardToolConfirm(approved: boolean) {
  if (!pendingConfirm.value) return;
  try {
    await invoke("confirm_tool", {
      id: pendingConfirm.value.id,
      approved,
    });
  } catch (e) {
    const errStr = String(e);
    if (!errStr.includes("无效的确认请求 ID")) {
      console.warn("确认工具操作失败:", e);
    }
  } finally {
    pendingConfirm.value = null;
  }
}

// === 智能体主动询问 (ask_user) 弹窗逻辑 ===
function selectAskUserOption(label: string) {
  askUserSelected.value = label;
  askUserAnswer.value = "";
}

async function submitAskUserAnswer(skip = false) {
  if (!pendingQuestion.value || askUserSubmitting.value) return;
  const questionId = pendingQuestion.value.id;
  // 自定义输入优先于选项; 跳过时发送空字符串
  const answer = skip ? "" : (askUserAnswer.value.trim() || askUserSelected.value);
  if (!skip && !answer) return;
  askUserSubmitting.value = true;
  try {
    await invoke("answer_question", { id: questionId, answer });
  } catch (e) {
    const errStr = String(e);
    if (!errStr.includes("无效的问题 ID")) {
      alert("提交回答失败: " + e);
    }
  } finally {
    pendingQuestion.value = null;
    askUserSubmitting.value = false;
  }
}

function toolStatusLabel(status: string) {
  const map: Record<string, string> = {
    requested: "已请求",
    waiting: "待确认",
    approved: "已授权",
    denied: "已拒绝",
    running: "执行中",
    completed: "已完成",
    skipped: "已跳过",
  };
  return map[status] || status;
}

function toggleAutoApprovedTool(toolId: string, enabled: boolean) {
  const current = new Set(apiConfig.value.auto_approved_tools);
  if (enabled) current.add(toolId);
  else current.delete(toolId);
  apiConfig.value.auto_approved_tools = Array.from(current).sort();
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
  if (!container) return;

  container.scrollTop = container.scrollHeight;
  dashChatEndRef.value?.scrollIntoView({ block: "end" });
}

watch(() => [
  chat.messages.length,
  chat.isLoading,
  thinkingContent.value,
  streamingAnswer.value,
  toolEvents.value.length,
  pendingConfirm.value?.id || "",
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
    scrollDashChatToBottom();
  }
  if (newPage === 'memory') {
    void loadMemories();
    void loadScheduledTasks();
  }
}, { flush: "post" });

function handlePageEntered() {
  if (activePage.value === "chat") {
    scrollDashChatToBottom();
  }
}

// === 生命周期 ===
let unlistenPetStatus: UnlistenFn | null = null;
let unlistenEvolutionUpdated: UnlistenFn | null = null;
let unlistenAgentsChanged: UnlistenFn | null = null;
let unlistenSkillsChanged: UnlistenFn | null = null;
let unlistenMemoriesChanged: UnlistenFn | null = null;

// Loading 超时保底机制
let loadingTimeout: ReturnType<typeof setTimeout> | null = null;
function resetLoadingTimeout() {
  if (loadingTimeout) clearTimeout(loadingTimeout);
  loadingTimeout = setTimeout(() => {
    if (chat.isLoading) {
      chat.isLoading = false;
      const last = chat.messages[chat.messages.length - 1];
      const timeoutMsg = "⚠️ 响应超时，请重试";
      if (!(last?.role === "assistant" && last.content === timeoutMsg)) {
        chat.addMessage("assistant", timeoutMsg, thinkingContent.value || undefined);
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
  unlistenWeatherUpdate = await listen<WeatherUpdateEvent>("weather-update", handleWeatherUpdate);

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
  void agentsStore.load();

  sysInfoTimer = setInterval(loadSystemInfo, 5000);
  weatherTimer = setInterval(loadWeather, 300000); // 每5分钟更新天气

  // 检查最大化状态
  isMaximized.value = await currentWindow.isMaximized();

  // 加载对话历史并监听实时同步
  await refreshChatState();

  unlistenThinking = await listen<string>("ai-thinking", (event) => {
    chat.isLoading = true;
    resetLoadingTimeout(); // 重置超时
    thinkingContent.value += event.payload;
  });

  unlistenAnswerDelta = await listen<AnswerDeltaPayload>("ai-answer-delta", (event) => {
    chat.isLoading = true;
    resetLoadingTimeout(); // 重置超时
    streamingAnswer.value += event.payload.text;
  });

  unlistenAiFinished = await listen<any>("ai-finished", (event) => {
    clearLoadingTimeout(); // 清除超时
    // 将当前工具事件快照附加到 AI 回复消息上
    const toolSnapshot = toolEvents.value.length > 0
      ? JSON.parse(JSON.stringify(toolEvents.value)) as any[]
      : undefined;
    const name = event.payload.agentName || event.payload.agent_name;
    const id = event.payload.agentId || event.payload.agent_id;
    const avatar = event.payload.agentAvatar || event.payload.agent_avatar;
    const replyAgent: MessageAgent | null = name
      ? {
          id: id || undefined,
          name,
          avatar: avatar || undefined,
        }
      : null;
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === event.payload.text)) {
      chat.addMessage("assistant", event.payload.text, event.payload.thinking || undefined, undefined, toolSnapshot, undefined, replyAgent);
    }
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    toolEvents.value = [];
    pendingConfirm.value = null;
    void speakDashboardReply(stripQuoteMarkers(event.payload.text));
  });

  unlistenAiError = await listen<any>("ai-error", (event) => {
    clearLoadingTimeout(); // 清除超时
    const toolSnapshot = toolEvents.value.length > 0
      ? JSON.parse(JSON.stringify(toolEvents.value)) as any[]
      : undefined;
    // 区分可恢复中断和致命错误
    let text: string;
    if (event.payload.aborted) {
      text = "已中止";
    } else if (streamingAnswer.value || thinkingContent.value) {
      // 有部分内容时，显示警告而非错误
      text = `⚠️ ${event.payload.message}`;
    } else {
      text = `出错了: ${event.payload.message}`;
    }
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === text)) {
      chat.addMessage("assistant", text, event.payload.thinking || undefined, undefined, toolSnapshot);
    }
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    toolEvents.value = [];
    pendingConfirm.value = null;
  });

  unlistenChatCleared = await listen("chat-history-cleared", () => {
    resetDashChatUi();
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
    const payload = event.payload;
    const last = chat.messages[chat.messages.length - 1];
    if (last && last.role === payload.role && last.content === payload.content) {
      return;
    }
    const syncAgent: MessageAgent | null = payload.agent?.name
      ? { id: payload.agent.id || undefined, name: payload.agent.name, avatar: payload.agent.avatar || undefined }
      : null;
    chat.addMessage(payload.role, payload.content, undefined, payload.files ?? payload.fileAttachments, undefined, payload.quote, syncAgent);
    if (payload.role === "user") {
      chat.isLoading = true;
      thinkingContent.value = "";
      streamingAnswer.value = "";
      toolEvents.value = [];
      pendingConfirm.value = null;
      pet.setState("thinking");
    }
  });

  unlistenToolEvent = await listen<ToolEvent>("ai-tool-event", (event) => {
    upsertToolEvent(event.payload);
  });

  unlistenToolConfirm = await listen<ToolConfirmPayload>("ai-tool-confirm", (event) => {
    pendingConfirm.value = event.payload;
  });

  unlistenToolConfirmResolved = await listen<{ id: string; approved: boolean }>("ai-tool-confirm-resolved", (event) => {
    if (pendingConfirm.value?.id === event.payload.id) {
      pendingConfirm.value = null;
    }
  });

  // 智能体主动询问 (ask_user): 收到问题弹出内嵌询问层
  unlistenAskUser = await listen<AskUserPayload>("ai-ask-user", (event) => {
    pendingQuestion.value = event.payload;
    askUserSelected.value = event.payload.options[0]?.label ?? "";
    askUserAnswer.value = "";
    askUserSubmitting.value = false;
  });

  unlistenAskUserResolved = await listen<{ id: string; answered: boolean }>("ai-ask-user-resolved", (event) => {
    if (pendingQuestion.value?.id === event.payload.id) {
      pendingQuestion.value = null;
      askUserSubmitting.value = false;
    }
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
  void evolutionStore.loadAll();
  unlistenEvolutionUpdated = await listen("evolution-updated", () => {
    void evolutionStore.loadAll();
  });
  unlistenAgentsChanged = await listen("agents-changed", () => {
    void agentsStore.load();
  });
  unlistenSkillsChanged = await listen("skills-changed", () => {
    void skillsStore.load();
  });
  unlistenMemoriesChanged = await listen("memories-changed", () => {
    void loadMemories();
  });
  unlistenNavigate = await currentWindow.listen<string>("navigate-to-page", (event) => {
    if (event.payload === "agents") {
      openAgentsPage();
    } else if (event.payload === "evolution") {
      openEvolutionPage();
    } else if (event.payload) {
      activePage.value = event.payload as NavPage;
    }
  });

  // 注册控制台文件拖拽事件
  unlistenDragDrop = await currentWindow.onDragDropEvent((event) => {
    handleDragDropEvent(event.payload);
  });

  await nextTick();
  scrollDashChatToBottom();
});

onUnmounted(() => {
  if (sysInfoTimer) clearInterval(sysInfoTimer);
  if (weatherTimer) clearInterval(weatherTimer);
  if (actionFeedbackTimer) clearTimeout(actionFeedbackTimer);
  if (dashChatScrollFrame !== null) cancelAnimationFrame(dashChatScrollFrame);
  dashChatScrollTimers.forEach(clearTimeout);
  unlistenPetStatus?.();
  unlistenEvolutionUpdated?.();
  unlistenAgentsChanged?.();
  unlistenSkillsChanged?.();
  unlistenMemoriesChanged?.();
  unlistenThinking?.();
  unlistenAnswerDelta?.();
  unlistenAiFinished?.();
  unlistenAiError?.();
  unlistenChatCleared?.();
  unlistenSyncMessage?.();
  unlistenToolEvent?.();
  unlistenToolConfirm?.();
  unlistenToolConfirmResolved?.();
  unlistenAskUser?.();
  unlistenAskUserResolved?.();
  unlistenScheduledTasksChanged?.();
  unlistenScheduledTaskTriggered?.();
  unlistenWeatherUpdate?.();
  unlistenDragDrop?.();
  unlistenVoiceSettingsChanged?.();
  unlistenTtsSettingsChanged?.();
  unlistenNavigate?.();
  unlistenSkillActivated?.();
ttsPlayer.stop();
});
</script>

<template>
  <div class="dashboard-root">
    <!-- 自定义标题栏 -->
    <div class="dashboard-titlebar" data-tauri-drag-region>
      <div class="titlebar-title">
        <span class="titlebar-mark">AP</span>
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
      <div class="dashboard-sidebar">
        <div class="sidebar-header">
          <div class="sidebar-brand">
            <span class="brand-logo">ap</span>
            <div class="brand-title-group">
              <span class="brand-name">AI Desktop Pet</span>
              <span class="brand-tag">Companion</span>
            </div>
          </div>
        </div>

        <nav class="sidebar-nav">
          <button
            :class="['sidebar-item', { active: activePage === 'home' }]"
            @click="activePage = 'home'"
          >
            <span class="sidebar-item-icon"><AppIcon name="home" :size="15" /></span>
            <span>首页</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'chat' }]"
            @click="activePage = 'chat'"
          >
            <span class="sidebar-item-icon"><AppIcon name="chat" :size="15" /></span>
            <span>对话互动</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'memory' }]"
            @click="activePage = 'memory'"
          >
            <span class="sidebar-item-icon"><AppIcon name="memory" :size="15" /></span>
            <span>记忆库</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'appearance' }]"
            @click="activePage = 'appearance'"
          >
            <span class="sidebar-item-icon"><AppIcon name="appearance" :size="15" /></span>
            <span>个性外观</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'agents' }]"
            @click="openAgentsPage()"
          >
            <span class="sidebar-item-icon"><AppIcon name="agents" :size="15" /></span>
            <span>智能体</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'evolution' }]"
            @click="openEvolutionPage()"
          >
            <span class="sidebar-item-icon"><AppIcon name="evolution" :size="15" /></span>
            <span>自进化</span>
            <span v-if="evolutionStore.pendingProposals.length > 0" class="sidebar-item-badge">
              {{ evolutionStore.pendingProposals.length }}
            </span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'voice' }]"
            @click="activePage = 'voice'"
          >
            <span class="sidebar-item-icon"><AppIcon name="voice" :size="15" /></span>
            <span>语音设置</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'system' }]"
            @click="activePage = 'system'"
          >
            <span class="sidebar-item-icon"><AppIcon name="system" :size="15" /></span>
            <span>系统设置</span>
          </button>

          <div class="sidebar-divider" />

          <button
            :class="['sidebar-item', { active: activePage === 'about' }]"
            @click="activePage = 'about'"
          >
            <span class="sidebar-item-icon"><AppIcon name="info" :size="15" /></span>
            <span>关于</span>
          </button>
        </nav>

        <div class="sidebar-footer">
          <button
            :class="['sidebar-action-btn', 'primary', { active: isPetActive }]"
            @click="isPetActive ? emit('closePet') : emit('openPet'); isPetActive = !isPetActive"
          >
            <span class="sidebar-action-icon"><AppIcon :name="isPetActive ? 'minus' : 'paw'" :size="14" /></span>
            <span>{{ isPetActive ? '收回桌宠' : '召唤桌宠' }}</span>
          </button>
          <button class="sidebar-action-btn danger" @click="exitApp">
            <span class="sidebar-action-icon"><AppIcon name="power" :size="14" /></span>
            <span>退出应用</span>
          </button>
        </div>
      </div>

      <!-- 内容区 -->
      <div :class="['dashboard-content', `page-${activePage}`, { 'chat-page-active': activePage === 'chat' }]">
        <Transition name="page-fade" mode="out-in" @after-enter="handlePageEntered">
          <!-- ========== 首页 ========== -->
          <div v-if="activePage === 'home'" key="home" class="home-studio-page">
            <div class="page-header home-header studio-header">
              <div>
                <div class="page-kicker">Overview</div>
                <h1 class="page-title">伙伴总览</h1>
                <p class="page-subtitle">查看伙伴状态，快速开始今天的协作</p>
              </div>
              <div :class="['status-pill', isPetActive ? 'live' : 'idle']">
                <span class="status-pill-dot" />
                <span>{{ isPetActive ? '桌宠在线' : '桌宠未启动' }}</span>
              </div>
            </div>

            <div class="home-grid">
              <!-- 宠物状态预览 -->
              <div class="dash-card pet-preview-card studio-panel">
                <div class="pet-preview-visual">
                  <div class="pet-orbit-ring ring-a" />
                  <div class="pet-orbit-ring ring-b" />
                  <div class="pet-preview-canvas-wrap">
                    <PetCanvas preview style="width: 120px; height: 140px; pointer-events: none;" />
                  </div>
                </div>
                <div class="pet-info">
                  <div class="pet-label">当前伙伴</div>
                  <div class="pet-status-name">{{ petStateLabel }}</div>
                  <div class="pet-meta-pills">
                    <button class="pet-meta-pill" @click="activePage = 'appearance'">换装</button>
                    <button class="pet-meta-pill" @click="activePage = 'chat'">聊天</button>
                    <button class="pet-meta-pill" @click="activePage = 'voice'">声音</button>
                  </div>
                  <div class="pet-stats">
                    <div class="pet-stat">
                      <span class="pet-stat-label">心情</span>
                      <div class="pet-stat-bar-wrap">
                        <div
                          class="pet-stat-bar"
                          :style="{
                            width: happinessPercent + '%',
                            background: 'var(--pet-primary, #ff6b6b)'
                          }"
                        />
                      </div>
                    </div>
                    <div class="pet-stat">
                      <span class="pet-stat-label">能量</span>
                      <div class="pet-stat-bar-wrap">
                        <div
                          class="pet-stat-bar"
                          :style="{
                            width: pet.energy + '%',
                            background: 'var(--pet-accent, #ffa07a)'
                          }"
                        />
                      </div>
                    </div>
                  </div>
                </div>
                <div class="pet-action-feedback">
                  <span class="pet-action-kicker">状态反馈</span>
                  <strong>{{ activeActionId ? '收到指令' : '待机中' }}</strong>
                  <p>{{ actionFeedback }}</p>
                </div>
              </div>


              <!-- 快捷操作 -->
              <div class="dash-card action-lab-card sticker-board-card">
                <div class="dash-card-title">
                  <span class="card-icon"><AppIcon name="zap" :size="13" /></span> 快捷操作
                </div>
                <div class="quick-actions">
                  <button
                    v-for="action in quickActions"
                    :key="action.id"
                    :class="['quick-action-btn', { active: activeActionId === action.id }]"
                    :title="`${action.label} - ${action.desc}`"
                    @click="petAction(action.id)"
                  >
                    <span class="quick-action-icon"><AppIcon :name="action.icon" :size="15" /></span>
                    <span class="quick-action-copy">
                      <span class="quick-action-label">{{ action.label }}</span>
                      <span class="quick-action-desc">{{ action.desc }}</span>
                    </span>
                  </button>
                </div>
              </div>

              <!-- 快速入口 -->
              <div class="dash-card mission-rail-card mission-ribbon-card">
                <div class="dash-card-title">
                  <span class="card-icon"><AppIcon name="home" :size="13" /></span> 快速入口
                </div>
                <div class="mission-rail">
                  <button
                    v-for="entry in missionEntries"
                    :key="entry.page"
                    class="mission-node"
                    @click="activePage = entry.page"
                  >
                    <span class="mission-node-icon"><AppIcon :name="entry.icon" :size="15" /></span>
                    <span>
                      <strong>{{ entry.title }}</strong>
                      <small>{{ entry.desc }}</small>
                    </span>
                  </button>
                </div>
                <div class="mission-metrics">
                  <span>模型 {{ currentModelLabel }}</span>
                  <span>模式 {{ currentExecutionModeLabel }}</span>
                  <span>{{ contextUsageLabel }}</span>
                </div>
              </div>

              <!-- 系统监控 -->
              <div class="dash-card system-pulse-card resource-strip-card">
                <div class="dash-card-title">
                  <span class="card-icon"><AppIcon name="cpu" :size="13" /></span> 系统资源
                </div>
                <div class="system-monitor">
                  <div class="monitor-item">
                    <div class="monitor-header">
                      <span class="monitor-label">CPU</span>
                      <span class="monitor-value">{{ systemInfo.cpu.toFixed(1) }}%</span>
                    </div>
                    <div class="monitor-bar-wrap">
                      <div class="monitor-bar" :style="{ width: systemInfo.cpu + '%', background: cpuBarColor(systemInfo.cpu) }" />
                    </div>
                  </div>
                  <div class="monitor-item">
                    <div class="monitor-header">
                      <span class="monitor-label">内存</span>
                      <span class="monitor-value">{{ systemInfo.memory.toFixed(1) }}%</span>
                    </div>
                    <div class="monitor-bar-wrap">
                      <div class="monitor-bar" :style="{ width: systemInfo.memory + '%', background: memBarColor(systemInfo.memory) }" />
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- ========== 对话互动 (独立页面) ========== -->
          <div v-else-if="activePage === 'chat'" key="chat" class="dash-chat-page">
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
                  <AppIcon name="refresh" :size="11" />
                </button>
              </template>
              <template v-else-if="isWeatherLoading">
                <span class="weather-icon"><AppIcon name="weather" :size="13" /></span>
                <span class="weather-desc">天气刷新中...</span>
              </template>
              <template v-else>
                <span class="weather-icon"><AppIcon name="weather" :size="13" /></span>
                <span class="weather-desc">天气获取失败：{{ weatherError }}</span>
                <button type="button" class="weather-refresh-btn" title="重试" @click="refreshWeather"><AppIcon name="refresh" :size="11" /></button>
              </template>
            </div>
            <!-- Glassmorphism drop zone overlay -->
            <div v-if="isFileOver" class="dash-drop-overlay">
              <div class="dash-drop-overlay-box">
                <span class="dash-drop-icon"><AppIcon name="folder" :size="26" /></span>
                <span class="dash-drop-text">释放文件以添加为附件</span>
              </div>
            </div>

            <div class="dash-chat-messages" ref="dashChatMessagesRef" @click="closeDashMessageMenu">
              <div v-if="chat.messages.length === 0" class="dash-chat-empty">
                还没有对话。<br />和它打个招呼，或直接描述你想做的事。
              </div>
              
              <div
                v-for="msg in chat.messages" :key="msg.id"
                :class="['dash-msg-wrapper', msg.role]"
                @contextmenu.prevent.stop="msg.role !== 'system' && openDashMessageMenu(msg)"
              >
                <!-- 系统分割线 -->
                <div v-if="msg.role === 'system'" class="dash-system-divider">
                  <span class="dash-system-divider-line"></span>
                  <span class="dash-system-divider-text">{{ msg.content }}</span>
                  <span class="dash-system-divider-line"></span>
                </div>
                <!-- 工具执行轨迹（显示在 AI 回复消息的上方） -->
                <div v-else-if="msg.role === 'assistant' && msg.toolEvents && msg.toolEvents.length > 0" class="dash-tool-trace-panel">
                  <div class="dash-tool-trace-header">
                    <span>执行轨迹</span>
                    <small>{{ msg.toolEvents.length }} 个工具事件</small>
                  </div>
                  <div class="dash-tool-trace-list">
                    <div
                      v-for="event in msg.toolEvents"
                      :key="event.id"
                      :class="['dash-tool-event', event.status]"
                    >
                      <div class="dash-tool-event-head">
                        <span class="dash-tool-status">{{ toolStatusLabel(event.status) }}</span>
                        <span class="dash-tool-name">{{ event.tool_name }}</span>
                      </div>
                      <div class="dash-tool-summary">{{ event.summary }}</div>
                      <div v-if="event.command" class="dash-tool-meta">命令：{{ event.command }}</div>
                      <div v-if="event.path" class="dash-tool-meta">路径：{{ event.path }}</div>
                      <details v-if="event.arguments" class="dash-tool-details">
                        <summary>参数</summary>
                        <pre>{{ event.arguments }}</pre>
                      </details>
                      <details v-if="event.output" class="dash-tool-details">
                        <summary>输出</summary>
                        <pre>{{ event.output }}</pre>
                      </details>
                    </div>
                  </div>
                </div>
                <div v-if="msg.role !== 'system'" class="dash-msg-bubble">
                  <div v-if="msg.agent && msg.role === 'assistant'" class="dash-msg-agent-badge">
                    <span class="dash-msg-agent-avatar">{{ msg.agent.avatar || "🤖" }}</span>
                    <span class="dash-msg-agent-name">{{ msg.agent.name }}</span>
                  </div>
                  <div v-if="msg.thinking" class="dash-msg-thinking">
                    <details>
                      <summary>思考过程</summary>
                      <p>{{ msg.thinking }}</p>
                    </details>
                  </div>
                  <!-- 用户引用的前文对话（微信式引用块） -->
                  <div v-if="msg.quote" class="dash-msg-quote" :title="msg.quote.content">
                    <div class="dash-msg-quote-name">{{ quoteSpeakerLabel(msg.quote.role) }}：</div>
                    <div class="dash-msg-quote-text">{{ msg.quote.content }}</div>
                  </div>
                  <!-- AI 回复中的 [QUOTE] 标记渲染为引用块 -->
                  <div class="dash-msg-text">
                    <template v-for="(seg, si) in parseQuoteSegments(msg.content)" :key="si">
                      <div v-if="seg.type === 'quote'" class="dash-msg-quote ai-quote" :title="seg.content">
                        <div class="dash-msg-quote-text">{{ seg.content }}</div>
                      </div>
                      <template v-else>
                        <template v-for="(ablock, bi) in parseAgentHeaders(seg.content, agentsStore.agents)" :key="si + '-' + bi">
                          <div v-if="ablock.type === 'agent_header'" class="dash-msg-agent-section-header">
                            <span class="dash-msg-agent-avatar">{{ ablock.avatar }}</span>
                            <span class="dash-msg-agent-name">{{ ablock.agentName }}</span>
                          </div>
                          <template v-else>
                            <template v-for="(mseg, mi) in splitMentionSegments(ablock.content, knownAgentNames)" :key="si + '-' + bi + '-' + mi">
                              <span v-if="mseg.type === 'mention'" class="dash-msg-mention">@{{ mseg.content }}</span>
                              <template v-else>{{ mseg.content }}</template>
                            </template>
                          </template>
                        </template>
                      </template>
                    </template>
                  </div>

                  <!-- 显示已发送的本地文件信息 -->
                  <div v-if="msg.files && msg.files.length > 0" class="dash-msg-files">
                    <div v-for="(file, fi) in msg.files" :key="fi" class="dash-msg-file-tag">
                      <span>{{ file.isImage ? "\u{1F4F7}" : getFileIcon(file.extension) }}</span>
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
                  <button type="button" :disabled="chat.isLoading" @click="resendDashMessage(msg)">重新发送</button>
                </div>
              </div>

              <!-- 工具执行轨迹（仅流式时显示，完成后转移到消息上） -->
              <div v-if="toolEvents.length > 0 && chat.isLoading" class="dash-msg-wrapper assistant">
                <div class="dash-tool-trace-panel">
                  <div class="dash-tool-trace-header">
                    <span>执行轨迹</span>
                    <small>{{ toolEvents.length }} 个工具事件</small>
                  </div>
                  <div class="dash-tool-trace-list">
                    <div
                      v-for="event in toolEvents"
                      :key="event.id"
                      :class="['dash-tool-event', event.status]"
                    >
                      <div class="dash-tool-event-head">
                        <span class="dash-tool-status">{{ toolStatusLabel(event.status) }}</span>
                        <span class="dash-tool-name">{{ event.tool_name }}</span>
                      </div>
                      <div class="dash-tool-summary">{{ event.summary }}</div>
                      <div v-if="event.command" class="dash-tool-meta">命令：{{ event.command }}</div>
                      <div v-if="event.path" class="dash-tool-meta">路径：{{ event.path }}</div>
                      <details v-if="event.arguments" class="dash-tool-details">
                        <summary>参数</summary>
                        <pre>{{ event.arguments }}</pre>
                      </details>
                      <details v-if="event.output" class="dash-tool-details">
                        <summary>输出</summary>
                        <pre>{{ event.output }}</pre>
                      </details>
                    </div>
                  </div>
                </div>
              </div>

              <!-- AI 实时打字状态 -->
              <div v-if="chat.isLoading" class="dash-msg-wrapper assistant loading">
                <div class="dash-msg-bubble">
                  <!-- 正在思考与停止按钮头部 -->
                  <div class="dash-msg-thinking-header">
                    <span>{{ thinkingContent ? "小家伙正在思考中..." : "等待思考输出..." }}</span>
                    <button class="dash-stop-btn" title="停止思考与输出" @click.stop="abortAi">
                      <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
                        <rect width="10" height="10" rx="2"/>
                      </svg>
                      <span>停止</span>
                    </button>
                  </div>
                  
                  <div v-if="thinkingContent" class="dash-msg-thinking">
                    <details open>
                      <summary>思维过程：</summary>
                      <p>{{ thinkingContent }}</p>
                    </details>
                  </div>
                  <div v-if="streamingAnswer" class="dash-msg-text">
                    <template v-for="(seg, si) in parseQuoteSegments(streamingAnswer, true)" :key="si">
                      <div v-if="seg.type === 'quote'" class="dash-msg-quote ai-quote" :title="seg.content">
                        <div class="dash-msg-quote-text">{{ seg.content }}</div>
                      </div>
                      <template v-else>
                        <template v-for="(ablock, bi) in parseAgentHeaders(seg.content, agentsStore.agents)" :key="si + '-' + bi">
                          <div v-if="ablock.type === 'agent_header'" class="dash-msg-agent-section-header">
                            <span class="dash-msg-agent-avatar">{{ ablock.avatar }}</span>
                            <span class="dash-msg-agent-name">{{ ablock.agentName }}</span>
                          </div>
                          <template v-else>{{ ablock.content }}</template>
                        </template>
                      </template>
                    </template>
                  </div>
                  <div class="dash-typing-dots">
                    <span class="dot"></span>
                    <span class="dot"></span>
                    <span class="dot"></span>
                  </div>
                </div>
              </div>
              <div ref="dashChatEndRef" class="dash-chat-end" aria-hidden="true"></div>
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

              <!-- 文件校验错误提示 -->
              <Transition name="fade">
                <div v-if="fileValidationError" class="dash-file-validation-error">
                  {{ fileValidationError }}
                </div>
              </Transition>

              <!-- 文件预加载面板 -->
              <Transition name="slide-up">
                <div v-if="pendingFiles.length > 0" class="dash-file-preview-area">
                  <div class="dash-file-preview-header">
                    <span>{{ pendingFiles.length }} 个文件待发送</span>
                    <button class="dash-clear-all-btn" @click="clearPendingFiles">全部移除</button>
                  </div>
                  <div class="dash-file-preview-list">
                    <TransitionGroup name="file-item">
                      <div
                        v-for="file in pendingFiles"
                        :key="file.id"
                        class="dash-file-preview-item"
                        :class="{ error: file.status === 'error' }"
                      >
                        <template v-if="file.isImage">
                          <img
                            v-if="file.previewUrl && file.status !== 'error'"
                            :src="file.previewUrl"
                            class="dash-preview-thumbnail"
                            @error="handleImageError(file)"
                          />
                          <div v-else class="dash-preview-fallback">
                            <span class="dash-file-icon">{{ getFileIcon(file.extension) }}</span>
                            <span class="dash-fallback-text">预览不可用</span>
                          </div>
                        </template>
                        <template v-else>
                          <div class="dash-preview-file-card">
                            <span class="dash-file-icon-lg">{{ getFileIcon(file.extension) }}</span>
                            <span class="dash-file-name" :title="file.name">{{ file.name }}</span>
                            <span class="dash-file-size">{{ formatFileSize(file.size) }}</span>
                          </div>
                        </template>
                        <button class="dash-remove-file-btn" @click="removePendingFile(file.id)">&times;</button>
                      </div>
                    </TransitionGroup>
                  </div>
                </div>
              </Transition>

              <Transition name="slide-up">
                <div v-if="showDashMentionMenu" class="dash-mention-menu" @mousedown.prevent>
                  <button
                    v-for="(agent, index) in dashMentionItems"
                    :key="agent.id"
                    type="button"
                    class="dash-mention-item"
                    :class="{ active: index === dashMentionSelectedIndex }"
                    @mouseenter="dashMentionSelectedIndex = index"
                    @click="chooseDashMentionItem(agent)"
                  >
                    <span class="dash-mention-avatar">{{ agent.avatar || "🤖" }}</span>
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
                    <span class="dash-mention-avatar">＋</span>
                    <span class="dash-mention-main">
                      <span class="dash-mention-title">新建智能体</span>
                      <span class="dash-mention-desc">打开智能体工坊，自由定义或用 AI 生成</span>
                    </span>
                  </button>
                </div>
              </Transition>

              <Transition name="slide-up">
                <div v-if="showDashSlashMenu" class="dash-slash-menu" @mousedown.prevent>
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

              <div class="dash-chat-input-area">
                <input
                  ref="dashChatInputRef"
                  type="text"
                  v-model="chatInput"
                  @keydown="onDashboardChatKeyDown"
                  placeholder="发送消息或拖入文件/应用快捷方式给桌宠..."
                  :disabled="chat.isLoading"
                  class="dash-chat-input"
                />
                <select
                  v-if="apiProfiles.length > 0"
                  class="dash-chat-model-select"
                  :value="activeApiProfileId"
                  title="切换直连 API 模型"
                  @change="activateApiProfile(($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="profile in apiProfiles" :key="profile.id" :value="profile.id">
                    {{ profile.name || profile.model }}
                  </option>
                </select>
                <button
                  @click="sendDashboardMessage"
                  :disabled="(!chatInput.trim() && pendingFiles.length === 0 && !pendingQuote) || chat.isLoading"
                  class="dash-chat-send-btn"
                >
                  发送
                </button>
              </div>
              <div class="dash-chat-meta-row">
                <span>模型：{{ currentModelLabel }}</span>
                <span>上下文：{{ contextUsageLabel }}</span>
                <select
                  class="dash-thinking-select"
                  :value="apiConfig.thinking_depth"
                  title="思考深度"
                  @change="updateThinkingDepth(($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="opt in thinkingDepthOptions" :key="opt.value" :value="opt.value">
                    思考：{{ opt.label }}
                  </option>
                </select>
                <select
                  class="dash-mode-select"
                  :value="apiConfig.execution_mode"
                  title="执行模式"
                  @change="updateExecutionMode(($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="mode in executionModeOptions" :key="mode.value" :value="mode.value">
                    {{ mode.label }}
                  </option>
                </select>
              </div>
            </div>
          </div>

          <!-- ========== 记忆库 ========== -->
          <div v-else-if="activePage === 'memory'" key="memory">
            <div class="page-header" style="display: flex; justify-content: space-between; align-items: flex-start;">
              <div>
                <div class="page-kicker">Memory</div>
                <h1 class="page-title">长期记忆库</h1>
                <p class="page-subtitle">保存稳定偏好、重要背景，并管理 AI 的定时提醒与桌面应用</p>
              </div>
              <button
                class="dash-btn secondary"
                :disabled="isScanningShortcuts"
                @click="handleScanShortcuts"
                style="display: inline-flex; align-items: center; gap: 6px;"
              >
                <AppIcon name="refresh" :size="12" />
                <span>{{ isScanningShortcuts ? "正在扫描桌面..." : "同步桌面快捷方式" }}</span>
              </button>
            </div>

            <div class="dash-memory-layout">
              <div class="dash-memory-editor-stack">
                <div class="dash-card palette-panel skin-panel">
                  <div class="dash-card-title"><span class="card-icon"><AppIcon name="plus" :size="13" /></span> 新建记忆</div>
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
                      <option value="preference">偏好习惯</option>
                      <option value="fact">个人事实</option>
                      <option value="habit">作息习惯</option>
                      <option value="work_domain">工作领域</option>
                      <option value="note">随手备忘</option>
                      <option value="manual">手动设定</option>
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
                  <div class="dash-card-title"><span class="card-icon"><AppIcon name="clock" :size="13" /></span> 新建定时任务</div>
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
                            <AppIcon :name="task.enabled ? 'pause' : 'play'" :size="11" />
                          </button>
                          <button
                            class="dash-icon-btn danger"
                            title="删除任务"
                            :disabled="taskDeletingId === task.id"
                            @click="deleteScheduledTask(task.id)"
                          >
                            <AppIcon name="x" :size="12" />
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
                    <span class="dash-memory-count">{{ filteredMemories.length }} / {{ memories.length }} 条</span>
                  </div>

                  <div class="dash-memory-filters">
                    <input
                      type="text"
                      class="dash-memory-search-input"
                      v-model="memorySearchQuery"
                      placeholder="搜索记忆标题或内容..."
                    />
                    <select class="dash-select dash-memory-cat-select" v-model="memoryCategoryFilter">
                      <option value="all">全部分类</option>
                      <option value="preference">偏好习惯</option>
                      <option value="fact">个人事实</option>
                      <option value="habit">作息习惯</option>
                      <option value="work_domain">工作领域</option>
                      <option value="note">随手备忘</option>
                      <option value="manual">手动设定</option>
                      <option value="conversation">对话摘要</option>
                      <option value="app_path">注册应用</option>
                      <option value="general">通用</option>
                    </select>
                  </div>

                  <div v-if="filteredMemories.length === 0" class="dash-memory-empty">
                    {{ memories.length === 0 ? "暂无长期记忆。在聊天中告诉桌宠“请记住……”，或在左侧手动添加。" : "未找到匹配的记忆条目。" }}
                  </div>
                  <div v-for="memory in filteredMemories" :key="memory.id" class="dash-memory-record">
                    <div class="dash-memory-record-head">
                      <div>
                        <span class="dash-memory-category">{{ memoryCategoryLabel(memory.category) }}</span>
                        <h3>{{ memory.key }}</h3>
                      </div>
                      <div class="dash-memory-actions">
                        <button
                          class="dash-icon-btn"
                          title="编辑此条记忆"
                          @click="openEditMemoryModal(memory)"
                        >
                          <AppIcon name="edit" :size="12" />
                        </button>
                        <button
                          class="dash-icon-btn danger"
                          title="删除记忆"
                          :disabled="memoryDeletingId === memory.id"
                          @click="deleteMemoryItem(memory.id)"
                        >
                          <AppIcon name="x" :size="12" />
                        </button>
                      </div>
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
              <p class="page-subtitle">创建专属智能体，在对话里输入 @ 就能召唤它工作</p>
            </div>

            <div class="dash-agents-layout">
              <div class="dash-card dash-agents-list-panel">
                <div class="dash-card-title">
                  <span class="card-icon"><AppIcon name="agents" :size="13" /></span> 我的智能体
                  <button class="dash-agents-add-btn" @click="openAgentEditor()">新建</button>
                </div>
                <div v-if="agentSaved" class="dash-agents-saved-tip">✓ 已保存</div>
                <div v-if="agentsStore.agents.length === 0" class="dash-agents-empty">
                  还没有自定义智能体。<br />点击「＋ 新建」自由定义，或用下方的 AI 帮你生成一个。
                </div>
                <div v-for="agent in agentsStore.agents" :key="agent.id" class="dash-agent-card">
                  <span class="dash-agent-card-avatar">{{ agent.avatar || "🤖" }}</span>
                  <div class="dash-agent-card-main">
                    <div class="dash-agent-card-name">@{{ agent.name }}</div>
                    <div class="dash-agent-card-desc">{{ agent.description || "（无描述）" }}</div>
                    <div class="dash-agent-card-meta">
                      {{ agent.allowed_tools.length }} 个免确认工具{{ agent.model ? " · " + agent.model : "" }}
                    </div>
                  </div>
                  <div class="dash-agent-card-actions">
                    <button class="dash-mini-btn" @click="openAgentEditor(agent)">编辑</button>
                    <button
                      class="dash-mini-btn danger"
                      :disabled="agentDeletingId === agent.id"
                      @click="deleteAgentDraft(agent)"
                    >
                      删除
                    </button>
                  </div>
                </div>

                <div class="dash-card-title" style="margin-top: 18px;">
                  <span class="card-icon"><AppIcon name="sparkles" :size="13" /></span> 用 AI 创建
                </div>
                <p class="dash-agents-ai-hint">描述你想要的智能体，AI 会生成名字、头像、角色设定和工具权限，你确认后即可保存。</p>
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
                    placeholder="用第二人称描述它的身份、职责、工作方式和边界，例如：你是用户的周报助手……"
                  ></textarea>
                </div>
                <div class="dash-form-group">
                  <label>专属模型（留空 = 使用当前模型）</label>
                  <input v-model="agentDraft.model" type="text" placeholder="例如：gpt-4o-mini / deepseek-chat" />
                </div>
                <div class="dash-form-group">
                  <label>免确认工具（该智能体被 @ 时自动授权）</label>
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

          <!-- ========== 自进化中心 ========== -->
          <div v-else-if="activePage === 'evolution'" key="evolution" class="dash-evolution-page">
            <div class="page-header">
              <div class="page-kicker">Self Evolution</div>
              <h1 class="page-title">自进化中心</h1>
              <p class="page-subtitle">基于历史对话持续迭代，沉淀默契画像、自动衍生专属智能体与定时工作流</p>
            </div>
            <EvolutionCenter />
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
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="paw" :size="13" /></span> 本体形象</div>
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
                      style="width: 88px; height: 102px; pointer-events: none;"
                    />
                    <img v-else src="../assets/pets/daimao-batiao/stills/still-03.png" alt="" />
                  </div>
                  <span class="dash-character-name">{{ character.name }}</span>
                  <span class="dash-character-desc">{{ character.description }}</span>
                </button>
              </div>
            </div>

              <div class="dash-card character-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="image" :size="13" /></span> 自定义像素桌宠</div>
              <CustomPixelPetWorkshop @selected="onCustomPixelSelected" />
            </div>

              <div class="dash-card palette-panel skin-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="appearance" :size="13" /></span> 主题皮肤</div>
              <div class="dash-skin-grid">
                <div
                  v-for="t in themes" :key="t.id"
                  :class="['dash-skin-item', { active: currentSkin === t.id }]"
                  @click="selectSkin(t.id)"
                >
                  <div class="dash-skin-preview" :style="{ background: t.headerGradient }">
                    <span v-if="currentSkin === t.id" class="dash-skin-check">✔</span>
                  </div>
                  <span class="dash-skin-name">{{ t.name }}</span>
                  <span v-if="t.animated" class="dash-skin-tag">随时间变色</span>
                </div>
              </div>
            </div>

            <!-- 字体颜色 -->
              <div class="dash-card sample-sheet font-panel">
              <div class="dash-card-title"><span class="card-icon card-icon-serif">Aa</span> 字体颜色</div>
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
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="image" :size="13" /></span> 对话背景</div>
              <div class="dash-bg-grid">
                <button :class="['dash-bg-opt', { active: chatBg === 'none' }]" @click="selectBg('none')">
                  <div class="dash-bg-preview"><AppIcon name="minus" :size="16" /></div>
                  <span class="dash-bg-label">无</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'cute' }]" @click="selectBg('cute')">
                  <div class="dash-bg-preview"><AppIcon name="paw" :size="16" /></div>
                  <span class="dash-bg-label">可爱</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'scifi' }]" @click="selectBg('scifi')">
                  <div class="dash-bg-preview"><AppIcon name="zap" :size="16" /></div>
                  <span class="dash-bg-label">科幻</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'minimal' }]" @click="selectBg('minimal')">
                  <div class="dash-bg-preview"><AppIcon name="check" :size="16" /></div>
                  <span class="dash-bg-label">简洁</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'custom' }]" @click="chooseBgImage">
                  <div class="dash-bg-preview">
                    <img v-if="customBgImage" :src="customBgImage" alt="" />
                    <AppIcon v-else name="image" :size="16" />
                  </div>
                  <span class="dash-bg-label">自定义</span>
                </button>
                <button v-if="chatBg === 'custom' && customBgImage" class="dash-bg-opt" @click="clearCustomBg">
                  <div class="dash-bg-preview"><AppIcon name="trash" :size="16" /></div>
                  <span class="dash-bg-label">清除</span>
                </button>
              </div>
              <input ref="bgInputRef" class="hidden-input" type="file" accept="image/*" @change="onBgImageSelected" />
            </div>

            <!-- 用户头像 -->
            <div class="dash-card portrait-frame-panel avatar-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="user" :size="13" /></span> 用户头像</div>
              <div class="dash-avatar-section">
                <div class="dash-avatar-preview">
                  <img v-if="pet.userAvatar" :src="pet.userAvatar" alt="用户头像" />
                  <AppIcon v-else name="user" :size="24" />
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
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="voice" :size="13" /></span> 语音交互</div>

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
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="system" :size="13" /></span> 声音引擎</div>

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
                    <AppIcon :name="isPreviewing ? 'stop' : 'play'" :size="11" />
                    {{ isPreviewing ? '停止' : '试听' }}
                  </button>
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

          <!-- ========== 系统 ========== -->
          <div v-else-if="activePage === 'system'" key="system" class="system-lab-page">
            <div class="page-header lab-header">
              <div class="page-kicker">System</div>
              <h1 class="page-title">系统设置</h1>
              <p class="page-subtitle">配置 AI 后端、性格和职业定位</p>
            </div>

            <!-- 自定义系统提示 -->
            <div class="dash-card lab-module">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="edit" :size="13" /></span> 自定义系统提示</div>
              <p class="dash-card-subtitle">追加到系统提示末尾，对所有对话生效。留空则不追加。</p>
              <textarea
                class="dash-textarea"
                rows="3"
                v-model="customPromptDraft"
                placeholder="例如：你每次回复都以『汪！』开头..."
              />
              <div class="dash-card-actions">
                <button class="dash-mini-btn primary" @click="saveCustomPrompt">
                  {{ customPromptSaved ? "已保存" : "保存" }}
                </button>
                <span v-if="skillsStore.activeSkill" class="dash-card-subtitle">
                  当前激活技能：<b>{{ skillsStore.activeSkill.name }}</b>
                </span>
              </div>
            </div>

            <!-- 技能（只读摘要） -->
            <div class="dash-card lab-module">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="cpu" :size="13" /></span> 技能</div>
              <p class="dash-card-subtitle">
                编辑请到设置页。这里只展示摘要与激活切换。
              </p>
              <div v-if="skillsStore.skills.length === 0" class="dash-card-subtitle">
                还没有任何技能。
              </div>
              <div v-else class="dash-skill-list">
                <div
                  v-for="s in skillsStore.skills"
                  :key="s.id"
                  class="dash-skill-row"
                  :class="{ active: s.is_active }"
                >
                  <div class="dash-skill-info">
                    <span class="dash-skill-name">{{ s.name }}</span>
                    <span v-if="s.is_active" class="dash-skill-badge">已激活</span>
                  </div>
                  <div class="dash-skill-desc">{{ s.description || "(无描述)" }}</div>
                  <div class="dash-card-actions">
                    <button
                      v-if="!s.is_active"
                      class="dash-mini-btn"
                      @click="activateSkill(s.id)"
                    >激活</button>
                    <button
                      v-else
                      class="dash-mini-btn"
                      @click="activateSkill(null)"
                    >取消激活</button>
                  </div>
                </div>
              </div>
            </div>

            <!-- AI 后端 -->
            <div class="dash-card lab-module agent-backend-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="terminal" :size="13" /></span> AI 后端与模型</div>

              <div class="dash-backend-selector">
                <button :class="['dash-backend-btn', { active: backendType === 'claude_code' }]" @click="selectBackend('claude_code')">
                  <span>CC</span> Claude Code
                </button>
                <button :class="['dash-backend-btn', { active: backendType === 'direct_api' }]" @click="selectBackend('direct_api')">
                  <span>API</span> 直连 API Agent
                </button>
              </div>

              <template v-if="backendType === 'claude_code'">
                <!-- 连接状态展示 -->
                <div class="connection-status-row">
                  <span class="status-label">本地连接状态：</span>
                  <span :class="['status-value', claudeStatus.logged_in ? 'connected' : 'disconnected']">
                    <span class="status-dot"></span>
                    {{ claudeStatus.logged_in ? '已连接 (Local Connected)' : '未连接 (Unconnected)' }}
                  </span>
                </div>

                <div v-if="claudeStatus.logged_in" class="connection-detail-box">
                  <div class="detail-item">
                    <span class="detail-label">AI 驱动模型:</span>
                    <span class="detail-value font-mono">{{ claudeStatus.model || 'Claude 3.5 Sonnet' }}</span>
                  </div>
                  <div class="detail-item">
                    <span class="detail-label">授权凭证:</span>
                    <span class="detail-value font-mono">{{ claudeStatus.token_preview }}</span>
                  </div>
                  <div v-if="claudeStatus.base_url" class="detail-item">
                    <span class="detail-label">API 接入点:</span>
                    <span class="detail-value font-mono text-truncate">{{ claudeStatus.base_url }}</span>
                  </div>
                </div>

                <div class="dash-current-model" style="margin-top: 14px;">当前会话模型：{{ currentModel || '加载中...' }}</div>
                <p class="dash-hint">使用本地 Claude Code CLI 子进程，会话自动保持上下文与本地执行能力。</p>
                
                <div style="display:flex;gap:12px;margin-top:12px;">
                  <button class="dash-btn primary" @click="resetModel">{{ modelSaved ? "已重置" : "重置会话" }}</button>
                  <button class="dash-btn secondary" @click="startClaudeConfig">
                    登录/配置 Claude Code
                  </button>
                </div>
              </template>

              <template v-else>
                <div class="dash-profile-toolbar">
                  <div>
                    <div class="dash-profile-toolbar-title">已保存模型配置</div>
                    <div class="dash-profile-toolbar-subtitle">保存后可在聊天输入框右下角一键切换</div>
                  </div>
                  <button class="dash-btn secondary" @click="newApiProfileDraft">新建模型</button>
                </div>
                <div v-if="apiProfiles.length > 0" class="dash-profile-list">
                  <div
                    v-for="profile in apiProfiles"
                    :key="profile.id"
                    :class="['dash-profile-item', { active: profile.id === activeApiProfileId }]"
                  >
                    <button class="dash-profile-main" @click="activateApiProfile(profile.id)">
                      <span class="dash-profile-name">{{ profile.name || profile.model }}</span>
                      <span class="dash-profile-meta">{{ profile.model }} · {{ profile.base_url }}</span>
                      <span class="dash-profile-badges">
                        <span>思考 {{ thinkingDepthLabel(profile.thinking_depth) }}</span>
                        <span>搜索 {{ searchProviderLabel(profile.search_provider) }}</span>
                        <span>{{ profile.has_api_key ? 'API Key 已隐藏' : '无 API Key' }}</span>
                      </span>
                    </button>
                    <button
                      class="dash-icon-btn danger"
                      title="删除模型配置"
                      :disabled="profileDeletingId === profile.id || apiProfiles.length <= 1"
                      @click="deleteApiProfile(profile.id)"
                    >
                      ×
                    </button>
                  </div>
                </div>
                <div class="dash-form-group">
                  <label>配置名称</label>
                  <input type="text" v-model="apiConfig.name" placeholder="例如：OpenAI 工作模型" />
                </div>
                <div class="dash-form-group">
                  <label>接口地址 (Base URL)</label>
                  <div class="dash-inline-control">
                    <input type="text" v-model="apiConfig.base_url" placeholder="https://api.openai.com/v1" @input="clearFetchedModels" />
                    <button
                      type="button"
                      class="dash-btn secondary"
                      :disabled="isFetchingModels || !apiConfig.base_url.trim()"
                      @click="fetchApiModels"
                    >
                      {{ isFetchingModels ? '获取中...' : '获取模型' }}
                    </button>
                  </div>
                </div>
                <div class="dash-form-group">
                  <label>API 密钥 (API Key)</label>
                  <input 
                    type="password" 
                    v-model="apiConfig.api_key" 
                    :placeholder="apiConfig.has_api_key ? '•••••••••••••••• (已保存)' : '请输入 sk-... 格式的 API Key'" 
                    autocomplete="new-password" 
                    @input="clearFetchedModels" 
                  />
                  <p class="dash-hint compact">已保存配置只显示“API Key 已隐藏”，不会在模型列表里明文展示。</p>
                </div>
                <div class="dash-form-group">
                  <label>模型名称 (Model)</label>
                  <input type="text" v-model="apiConfig.model" placeholder="gpt-4o-mini" />
                </div>
                <div v-if="modelFetchResult.message" :class="['dash-test-result', 'compact', modelFetchResult.success ? 'success' : 'error']">
                  {{ modelFetchResult.message }}
                </div>
                <div v-if="fetchedModels.length > 0" class="dash-model-picker">
                  <div class="dash-field-row">
                    <span class="dash-field-label">选择模型</span>
                    <select
                      class="dash-select"
                      :value="selectedFetchedModel"
                      @change="selectFetchedModel(($event.target as HTMLSelectElement).value)"
                    >
                      <option v-for="model in fetchedModels" :key="model" :value="model">{{ model }}</option>
                    </select>
                  </div>
                  <button class="dash-btn primary" :disabled="isAddingFetchedModel || !selectedFetchedModel" @click="addFetchedModelProfile">
                    {{ isAddingFetchedModel ? '加入中...' : '加入模型列表' }}
                  </button>
                </div>
                <div class="dash-form-group">
                  <label>思考深度</label>
                  <div class="dash-thinking-switch">
                    <button
                      v-for="item in thinkingDepthOptions"
                      :key="item.value"
                      type="button"
                      :class="['dash-thinking-option', { active: apiConfig.thinking_depth === item.value }]"
                      @click="updateThinkingDepth(item.value)"
                    >
                      {{ item.label }}
                    </button>
                  </div>
                  <p class="dash-hint compact">切换后会立即保存到当前模型配置，并在下一次直连 API 请求中生效。</p>
                </div>
                <div class="dash-form-group">
                  <label>优先搜索源</label>
                  <select
                    class="dash-select"
                    :value="apiConfig.search_provider"
                    @change="updateSearchProvider(($event.target as HTMLSelectElement).value)"
                  >
                    <option v-for="item in searchProviderOptions" :key="item.value" :value="item.value">
                      {{ item.label }} - {{ item.desc }}
                    </option>
                  </select>
                  <p class="dash-hint compact">国内网络推荐使用 Bing；DuckDuckGo 在国内网络下容易超时。</p>
                </div>
                <div class="dash-form-group">
                  <label>执行模式</label>
                  <div class="dash-mode-grid">
                    <button
                      v-for="mode in executionModeOptions"
                      :key="mode.value"
                      type="button"
                      :class="['dash-mode-option', { active: apiConfig.execution_mode === mode.value }]"
                      @click="apiConfig.execution_mode = mode.value"
                    >
                      <span>{{ mode.label }}</span>
                      <small>{{ mode.desc }}</small>
                    </button>
                  </div>
                </div>
                <div class="dash-toggle-group">
                  <label>Computer Use 桌面控制</label>
                  <input
                    type="checkbox"
                    class="dash-toggle"
                    :checked="computerUseEnabled"
                    @change="updateComputerUseEnabled(($event.target as HTMLInputElement).checked)"
                  />
                </div>
                <p class="dash-hint" style="margin-top:-4px;">
                  开启后 AI 可使用查看屏幕、聚焦窗口、打开浏览器、鼠标和键盘工具。仅在需要桌面自动化时开启。
                  <span v-if="computerUseSaved"> 已保存。</span>
                </p>
                <div v-if="apiConfig.execution_mode === 'custom'" class="dash-form-group">
                  <label>自定义免确认操作</label>
                  <div class="dash-tool-permission-grid">
                    <label v-for="tool in toolPermissionOptions" :key="tool.id" class="dash-tool-permission">
                      <input
                        type="checkbox"
                        :checked="isToolAutoApproved(tool.id)"
                        @change="toggleAutoApprovedTool(tool.id, ($event.target as HTMLInputElement).checked)"
                      />
                      <span>{{ tool.label }}</span>
                    </label>
                  </div>
                </div>
                <div class="dash-form-group">
                  <label>流式模式</label>
                  <div class="dash-mode-grid">
                    <button
                      type="button"
                      :class="['dash-mode-option', { active: apiConfig.stream_mode === 'auto' }]"
                      @click="updateStreamMode('auto')"
                    >
                      <span>自动检测</span>
                      <small>先 SSE 后 JSON</small>
                    </button>
                    <button
                      type="button"
                      :class="['dash-mode-option', { active: apiConfig.stream_mode === 'stream' }]"
                      @click="updateStreamMode('stream')"
                    >
                      <span>流式 SSE</span>
                      <small>OpenAI 标准</small>
                    </button>
                    <button
                      type="button"
                      :class="['dash-mode-option', { active: apiConfig.stream_mode === 'non_stream' }]"
                      @click="updateStreamMode('non_stream')"
                    >
                      <span>非流式 JSON</span>
                      <small>Auto Code 等</small>
                    </button>
                  </div>
                  <p class="dash-hint">自动：先尝试 SSE，失败则回退到 JSON。Auto Code 等非标准接口选"非流式"。</p>
                </div>
                <div class="dash-form-group">
                  <label>API 格式</label>
                  <div class="dash-mode-grid">
                    <button
                      type="button"
                      :class="['dash-mode-option', { active: apiConfig.api_mode === 'chat_completions' }]"
                      @click="updateApiMode('chat_completions')"
                    >
                      <span>Chat Completions</span>
                      <small>传统兼容格式 /v1/chat/completions</small>
                    </button>
                    <button
                      type="button"
                      :class="['dash-mode-option', { active: apiConfig.api_mode === 'responses' }]"
                      @click="updateApiMode('responses')"
                    >
                      <span>Responses</span>
                      <small>OpenAI 新格式 /v1/responses</small>
                    </button>
                  </div>
                  <p class="dash-hint">Chat Completions 是主流兼容格式，Responses 是 OpenAI 2025 年推出的新格式。大多数情况选 Chat Completions。</p>
                </div>
                <div class="dash-form-group">
                  <label>预设一键填充</label>
                  <div class="dash-presets">
                    <button class="dash-preset-badge" @click="applyPreset('openai')">OpenAI</button>
                    <button class="dash-preset-badge" @click="applyPreset('deepseek')">DeepSeek</button>
                    <button class="dash-preset-badge" @click="applyPreset('qwen')">通义千问</button>
                    <button class="dash-preset-badge" @click="applyPreset('ollama')">Ollama本地</button>
                  </div>
                </div>

                <div class="dash-toggle-group">
                  <label>工具调用二次确认 (推荐)</label>
                  <input
                    type="checkbox"
                    class="dash-toggle"
                    :checked="apiConfig.execution_mode !== 'unreviewed'"
                    @change="apiConfig.execution_mode = ($event.target as HTMLInputElement).checked ? 'normal' : 'unreviewed'"
                  />
                </div>
                <p class="dash-hint" style="margin-top:-4px;">普通模式会在每类操作首次执行前请求确认；自定义模式按上方免确认列表执行。</p>

                <div v-if="testResult.message" :class="['dash-test-result', testResult.success ? 'success' : 'error']">
                  {{ testResult.success ? '连接成功！' : testResult.message }}
                </div>
                <div class="dash-api-actions">
                  <button class="dash-btn" @click="testConnection" :disabled="isTestingConnection || !apiConfig.base_url || !apiConfig.model">
                    {{ isTestingConnection ? '测试中...' : '测试连接' }}
                  </button>
                  <button class="dash-btn primary" @click="saveApiConfig()" :disabled="isSavingConfig">
                    {{ configSaved ? '保存成功' : (isSavingConfig ? '保存中...' : '保存配置') }}
                  </button>
                </div>
                <div class="dash-api-actions" style="margin-top: 8px;">
                  <button class="dash-btn deep-test-btn" @click="testCompatibility" :disabled="isTestingCompatibility || !apiConfig.base_url || !apiConfig.model">
                    {{ isTestingCompatibility ? '检测中...' : '深度测试兼容性' }}
                  </button>
                </div>

                <!-- 深度测试结果 -->
                <div
                  v-if="compatibilityResult"
                  class="dash-compat-result"
                  :class="compatibilityResult.http_ok ? 'success' : 'error'"
                >
                  <h4>兼容性检测报告</h4>
                  <div class="compat-grid">
                    <div class="compat-item">
                      <span class="compat-label">HTTP 状态:</span>
                      <span :class="compatibilityResult.http_ok ? 'pass' : 'fail'">
                        {{ compatibilityResult.http_ok ? '✓' : '✗' }} {{ compatibilityResult.http_status }}
                      </span>
                    </div>
                    <div class="compat-item">
                      <span class="compat-label">SSE 格式:</span>
                      <span :class="compatibilityResult.is_sse_format ? 'pass' : 'fail'">
                        {{ compatibilityResult.is_sse_format ? '✓ 支持' : '✗ 不支持' }}
                      </span>
                    </div>
                    <div class="compat-item">
                      <span class="compat-label">JSON 格式:</span>
                      <span :class="compatibilityResult.is_json_format ? 'pass' : 'fail'">
                        {{ compatibilityResult.is_json_format ? '✓ 支持' : '✗ 不支持' }}
                      </span>
                    </div>
                    <div class="compat-item">
                      <span class="compat-label">内容提取:</span>
                      <span :class="compatibilityResult.content_extracted ? 'pass' : 'fail'">
                        {{ compatibilityResult.content_extracted ? '✓ 成功' : '✗ 失败' }}
                      </span>
                    </div>
                    <div class="compat-item" v-if="compatibilityResult.content_extracted">
                      <span class="compat-label">提取内容:</span>
                      <span class="compat-content">{{ compatibilityResult.content_extracted }}</span>
                    </div>
                    <div class="compat-item" v-if="compatibilityResult.recommended_mode">
                      <span class="compat-label">推荐模式:</span>
                      <span class="compat-recommend">{{ compatibilityResult.recommended_mode === 'stream' ? '流式 (SSE)' : compatibilityResult.recommended_mode === 'non_stream' ? '非流式 (JSON)' : '自动' }}</span>
                    </div>
                    <div class="compat-item" v-if="compatibilityResult.errors && compatibilityResult.errors.length > 0">
                      <span class="compat-label">错误:</span>
                      <ul class="compat-errors">
                        <li v-for="(err, idx) in compatibilityResult.errors" :key="idx">{{ err }}</li>
                      </ul>
                    </div>
                  </div>
                  <details v-if="compatibilityResult.raw_preview" class="compat-raw">
                    <summary>查看原始响应 (前500字符)</summary>
                    <pre>{{ compatibilityResult.raw_preview }}</pre>
                  </details>
                  <p class="compat-time">响应时间: {{ compatibilityResult.response_time_ms }} ms</p>
                </div>
              </template>
            </div>

            <div class="dash-card weather-settings-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="weather" :size="13" /></span> 天气 API 与提醒</div>
              <div class="dash-toggle-group">
                <label>启用天气栏与每日天气问候</label>
                <input
                  type="checkbox"
                  class="dash-toggle"
                  v-model="weatherConfig.enabled"
                />
              </div>
              <div class="weather-settings-grid">
                <div class="dash-form-group">
                  <label>城市/地区</label>
                  <input
                    type="text"
                    v-model="weatherConfig.location"
                    placeholder="建议填写中文城市；例如 上海 / 北京"
                  />
                </div>
                <div class="dash-form-group">
                  <label>wttr.in 兼容 API 地址</label>
                  <input
                    type="text"
                    v-model="weatherConfig.api_url"
                    placeholder="http://wttr.in 或 https://example.com/{location}?format=j1"
                  />
                </div>
              </div>
              <div v-if="weatherInfo" class="weather-settings-preview">
                当前天气：{{ formatWeatherDisplay(weatherInfo) }} · {{ weatherInfo.description }}
              </div>
              <div
                v-if="weatherConfigResult.message"
                :class="['dash-test-result', 'compact', weatherConfigResult.success ? 'success' : 'error']"
              >
                {{ weatherConfigResult.message }}
              </div>
              <div class="dash-api-actions">
                <button
                  class="dash-btn"
                  type="button"
                  :disabled="isTestingWeatherConfig || !weatherConfig.api_url.trim()"
                  @click="testWeatherConfig"
                >
                  {{ isTestingWeatherConfig ? '测试中...' : '测试天气 API' }}
                </button>
                <button
                  class="dash-btn secondary"
                  type="button"
                  :disabled="isWeatherLoading || !weatherConfig.enabled"
                  @click="refreshWeather"
                >
                  {{ isWeatherLoading ? '刷新中...' : '立即刷新' }}
                </button>
                <button
                  class="dash-btn primary"
                  type="button"
                  :disabled="isSavingWeatherConfig"
                  @click="saveWeatherConfig"
                >
                  {{ isSavingWeatherConfig ? '保存中...' : '保存天气设置' }}
                </button>
              </div>
            </div>

            <!-- 性格 -->
            <div class="system-role-grid">
            <div class="dash-card role-card personality-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="smile" :size="13" /></span> 性格</div>
              <div class="dash-option-grid">
                <button
                  v-for="item in personalities" :key="item.id"
                  :class="['dash-option-item', { active: currentPersonality === item.id }]"
                  @click="selectPersonality(item.id)"
                >
                  <span class="dash-option-name">{{ item.name }}</span>
                  <span class="dash-option-desc">{{ item.desc }}</span>
                </button>
              </div>
              <p class="dash-hint" style="margin-top:12px;">切换后会重置 Claude 会话，让新设定立即生效</p>
            </div>

            <!-- 职业 -->
            <div class="dash-card role-card profession-panel">
              <div class="dash-card-title"><span class="card-icon"><AppIcon name="file" :size="13" /></span> 职业</div>
              <div class="dash-option-grid">
                <button
                  v-for="item in professions" :key="item.id"
                  :class="['dash-option-item', { active: currentProfession === item.id }]"
                  @click="selectProfession(item.id)"
                >
                  <span class="dash-option-name">{{ item.name }}</span>
                  <span class="dash-option-desc">{{ item.desc }}</span>
                </button>
              </div>
            </div>
            </div>
          </div>

          <!-- ========== 关于 ========== -->
          <div v-else-if="activePage === 'about'" key="about">
            <div class="page-header">
              <div class="page-kicker">About</div>
              <h1 class="page-title">关于</h1>
              <p class="page-subtitle">应用信息</p>
            </div>

            <div class="dash-card about-card">
              <div class="about-logo"><AppIcon name="paw" :size="24" /></div>
              <div class="about-name">AI Desktop Pet</div>
              <div class="about-version">v0.1.0</div>
              <p class="about-desc">
                一个住在你桌面上的 AI 伙伴：智能对话、语音交互、长期记忆与个性化外观。
                基于 Tauri + Vue 3 构建，本地存储，轻量高效。
              </p>
            </div>
          </div>
        </Transition>

        <!-- 工具确认模态层 (全局，任何页面可见) -->
        <Transition name="fade">
          <div
            v-if="pendingConfirm"
            class="dash-tool-confirm-overlay"
            @click.self="handleDashboardToolConfirm(false)"
          >
            <div class="dash-tool-confirm-card" style="max-width: 500px;">
              <div class="dash-tool-confirm-head">
                <div class="dash-tool-confirm-title">
                  <span class="dash-tool-confirm-badge" style="background: rgba(191, 122, 78, 0.15); color: var(--dash-accent);">
                    <AppIcon name="zap" :size="14" />
                  </span>
                  <span>工具执行授权</span>
                </div>
                <button
                  class="dash-icon-btn"
                  title="拒绝并关闭 (Esc)"
                  @click="handleDashboardToolConfirm(false)"
                >
                  <AppIcon name="x" :size="12" />
                </button>
              </div>

              <div class="dash-tool-confirm-body" style="gap: 10px;">
                <div class="summary-panel" style="padding: 10px 12px; border-radius: 9px; background: var(--dash-panel-sunken); border: 1px solid var(--dash-card-border);">
                  <span style="font-size: 10px; font-weight: 700; color: var(--dash-text-muted); text-transform: uppercase;">操作类别</span>
                  <strong style="font-size: 13px; color: var(--dash-text-primary); display: block; margin-top: 2px;">{{ pendingConfirm.summary || pendingConfirm.tool_name }}</strong>
                </div>

                <div v-if="pendingConfirm.command" class="detail-row" style="padding: 8px 12px; border-radius: 8px; background: var(--dash-panel-sunken); border: 1px solid var(--dash-card-border);">
                  <span style="font-size: 10px; font-weight: 700; color: var(--dash-text-muted); text-transform: uppercase;">执行命令</span>
                  <code style="display: block; font-family: var(--dash-font-mono); font-size: 11px; margin-top: 2px; color: var(--dash-text-primary); word-break: break-all;">{{ pendingConfirm.command }}</code>
                </div>

                <div v-if="pendingConfirm.path" class="detail-row" style="padding: 8px 12px; border-radius: 8px; background: var(--dash-panel-sunken); border: 1px solid var(--dash-card-border);">
                  <span style="font-size: 10px; font-weight: 700; color: var(--dash-text-muted); text-transform: uppercase;">目标路径</span>
                  <code style="display: block; font-family: var(--dash-font-mono); font-size: 11px; margin-top: 2px; color: var(--dash-text-primary); word-break: break-all;">{{ pendingConfirm.path }}</code>
                </div>

                <div v-if="pendingConfirm.arguments" class="dash-tool-args">
                  <span class="args-label" style="font-size: 10.5px; font-weight: 650; color: var(--dash-text-muted);">调用参数:</span>
                  <pre class="args-code" style="max-height: 140px; margin-top: 4px;"><code>{{ formatArguments(pendingConfirm.arguments) }}</code></pre>
                </div>
              </div>

              <div class="dash-tool-confirm-actions" style="margin-top: 16px;">
                <button class="dash-btn secondary" @click="handleDashboardToolConfirm(false)">
                  <AppIcon name="x" :size="12" /> 拒绝 (Esc)
                </button>
                <button class="dash-btn primary" @click="handleDashboardToolConfirm(true)">
                  <AppIcon name="check" :size="12" /> 允许运行
                </button>
              </div>
            </div>
          </div>
        </Transition>

        <!-- 智能体主动询问弹窗 (ask_user, 全局，任何页面可见) -->
        <Transition name="slide-up">
          <div v-if="pendingQuestion" class="dash-floating-confirm-overlay">
            <div class="dash-ask-user-card">
              <div class="dash-ask-user-header">
                <span class="dash-ask-user-kicker">AI 有个问题</span>
                <strong class="dash-ask-user-question">{{ pendingQuestion.question }}</strong>
              </div>

              <div v-if="pendingQuestion.options.length" class="dash-ask-user-options">
                <button
                  v-for="option in pendingQuestion.options"
                  :key="option.label"
                  type="button"
                  :class="['dash-ask-option', { selected: askUserSelected === option.label && !askUserAnswer.trim() }]"
                  @click="selectAskUserOption(option.label)"
                >
                  <span class="dash-ask-option-label">{{ option.label }}</span>
                  <span v-if="option.description" class="dash-ask-option-desc">{{ option.description }}</span>
                </button>
              </div>

              <div class="dash-ask-user-input">
                <input
                  v-model="askUserAnswer"
                  type="text"
                  placeholder="或自己填写答案…"
                  @keydown.enter.prevent="submitAskUserAnswer()"
                />
              </div>

              <div class="dash-tool-confirm-actions">
                <button
                  class="dash-btn secondary"
                  :disabled="askUserSubmitting"
                  @click="submitAskUserAnswer(true)"
                >
                  跳过
                </button>
                <button
                  class="dash-btn primary"
                  :disabled="askUserSubmitting || (!askUserAnswer.trim() && !askUserSelected)"
                  @click="submitAskUserAnswer()"
                >
                  提交回答
                </button>
              </div>
            </div>
          </div>
        </Transition>

        <!-- 编辑记忆模态框 -->
        <Transition name="fade">
          <div
            v-if="editMemoryModalOpen && editingMemoryDraft"
            class="dash-tool-confirm-overlay"
            @click.self="editMemoryModalOpen = false"
          >
            <div class="dash-tool-confirm-card" style="max-width: 520px;">
              <div class="dash-tool-confirm-head">
                <div class="dash-tool-confirm-title">
                  <span class="dash-tool-confirm-badge"><AppIcon name="edit" :size="13" /></span>
                  <span>编辑长期记忆</span>
                </div>
                <button
                  class="dash-icon-btn"
                  title="关闭"
                  @click="editMemoryModalOpen = false"
                >
                  <AppIcon name="x" :size="12" />
                </button>
              </div>

              <div class="dash-tool-confirm-body" style="gap: 14px;">
                <div class="dash-form-group">
                  <label>分类</label>
                  <select class="dash-select" v-model="editingMemoryDraft.category">
                    <option value="preference">偏好习惯</option>
                    <option value="fact">个人事实</option>
                    <option value="habit">作息习惯</option>
                    <option value="work_domain">工作领域</option>
                    <option value="note">随手备忘</option>
                    <option value="manual">手动设定</option>
                    <option value="conversation">对话摘要</option>
                    <option value="app_path">注册应用</option>
                    <option value="general">通用</option>
                  </select>
                </div>

                <div class="dash-form-group">
                  <label>标题 / 键名</label>
                  <input
                    type="text"
                    class="dash-input"
                    v-model="editingMemoryDraft.key"
                    placeholder="记忆标题..."
                  />
                </div>

                <div class="dash-form-group">
                  <label>记忆内容</label>
                  <textarea
                    class="dash-textarea"
                    v-model="editingMemoryDraft.value"
                    rows="5"
                    placeholder="详细记忆内容..."
                  ></textarea>
                </div>
              </div>

              <div class="dash-tool-confirm-actions">
                <button
                  class="dash-btn secondary"
                  @click="editMemoryModalOpen = false"
                >
                  取消
                </button>
                <button
                  class="dash-btn primary"
                  :disabled="isUpdatingMemory || !editingMemoryDraft.key.trim() || !editingMemoryDraft.value.trim()"
                  @click="saveEditedMemory"
                >
                  {{ isUpdatingMemory ? "保存中..." : "保存修改" }}
                </button>
              </div>
            </div>
          </div>
        </Transition>
      </div>
    </div>
  </div>
</template>
