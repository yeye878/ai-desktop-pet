<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import PetCanvas from "./PetCanvas.vue";
import { usePetStore, THEMES, FONT_COLORS, resolveSkinId } from "../stores/pet";
import { useChatStore } from "../stores/chat";
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
import "../assets/dashboard.css";

const emit = defineEmits<{ openPet: []; closePet: [] }>();
const pet = usePetStore();
const chat = useChatStore();
const currentWindow = getCurrentWindow();

// === 导航 ===
type NavPage = "home" | "chat" | "memory" | "appearance" | "voice" | "system" | "about";
const activePage = ref<NavPage>("home");
const isPetActive = ref(false);

type MemoryItem = {
  id: number;
  category: string;
  key: string;
  value: string;
  created_at: number;
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

type AnswerDeltaPayload = {
  text: string;
};

// === 系统信息 ===
const systemInfo = ref({ cpu: 0, memory: 0 });
let sysInfoTimer: ReturnType<typeof setInterval> | null = null;

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

// === 字体颜色 ===
const currentFontColor = ref("");
const avatarInputRef = ref<HTMLInputElement | null>(null);

// === 对话背景 ===
const BG_KEY = "ai-desktop-pet.chat-bg";
const CUSTOM_BG_KEY = "ai-desktop-pet.chat-bg-custom";
const VOICE_SETTINGS_KEY = "voice_settings";
const TTS_SETTINGS_KEY = "tts_settings";
const chatBg = ref(localStorage.getItem(BG_KEY) || "none");
const customBgImage = ref(localStorage.getItem(CUSTOM_BG_KEY) || "");
const bgInputRef = ref<HTMLInputElement | null>(null);

// === 语音 ===
const voiceSettings = ref<VoiceSettings>({ ...DEFAULT_VOICE_SETTINGS });
const availableVoices = ref<SpeechSynthesisVoice[]>([]);
const ttsSettings = ref<TtsSettings>({ ...DEFAULT_TTS_SETTINGS });
const edgeVoices = ref<TtsVoice[]>([]);
const edgeVoiceSearch = ref("");
const showAllEdgeVoices = ref(false);
const ttsPreviewPlayer = new TtsPlayer();
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
  category: "manual",
});
const isSavingMemory = ref(false);
const memorySaved = ref(false);
const memoryDeletingId = ref<number | null>(null);

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

const toolPermissionOptions = [
  { id: "read_file", label: "读取文件" },
  { id: "write_file", label: "写入文件" },
  { id: "list_directory", label: "列出目录" },
  { id: "run_command", label: "执行命令" },
  { id: "web_search", label: "网页搜索" },
  { id: "open_app", label: "打开应用" },
];

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

function thinkingDepthLabel(value: string) {
  return thinkingDepthOptions.find((item) => item.value === value)?.label || "自动";
}

function searchProviderLabel(value: string) {
  return searchProviderOptions.find((item) => item.value === value)?.label || "Bing";
}

const quickActions = [
  { id: "wave", icon: "↗", label: "打招呼", desc: "挥挥手，进入陪伴状态", tone: "sky" },
  { id: "happy", icon: "◎", label: "开心一下", desc: "给小家伙加一点元气", tone: "lemon" },
  { id: "sleep", icon: "Zz", label: "去睡觉", desc: "切到安静休息状态", tone: "lavender" },
  { id: "wake", icon: "⏱", label: "叫醒它", desc: "回到待命，随时响应", tone: "mint" },
  { id: "new-chat", icon: "+", label: "新对话", desc: "保存摘要并重开上下文", tone: "mint" },
  { id: "clear", icon: "⌫", label: "清空对话", desc: "整理上下文和记忆摘要", tone: "coral" },
];

const missionEntries = [
  { page: "chat" as NavPage, icon: "💬", title: "对话舱", desc: "直接派发任务或闲聊" },
  { page: "appearance" as NavPage, icon: "◐", title: "换装台", desc: "皮肤、字体和背景" },
  { page: "voice" as NavPage, icon: "◌", title: "声线站", desc: "快捷键、语音和试听" },
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

async function loadMemories() {
  try { memories.value = await invoke<MemoryItem[]>("get_memories"); } catch {}
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

async function updateTtsSettings(patch: Partial<TtsSettings>) {
  ttsSettings.value = { ...ttsSettings.value, ...patch };
  try {
    await invoke("set_setting_value", { key: TTS_SETTINGS_KEY, value: JSON.stringify(ttsSettings.value) });
  } catch {}
}

async function previewVoice() {
  if (isPreviewing.value) { ttsPreviewPlayer.stop(); isPreviewing.value = false; return; }
  isPreviewing.value = true;
  try { await ttsPreviewPlayer.speak("你好，我是你的桌宠伙伴，很高兴认识你。", ttsSettings.value); }
  catch (e: any) { if (e.message !== "Aborted") alert("试听失败: " + (e.message || e)); }
  isPreviewing.value = false;
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
const thinkingContent = ref("");
const streamingAnswer = ref("");
const toolEvents = ref<ToolEvent[]>([]);
const pendingConfirm = ref<ToolConfirmPayload | null>(null);
const dashChatMessagesRef = ref<HTMLDivElement | null>(null);
const dashChatEndRef = ref<HTMLDivElement | null>(null);
let dashChatScrollFrame: number | null = null;
let dashChatScrollTimers: ReturnType<typeof setTimeout>[] = [];

let unlistenThinking: UnlistenFn | null = null;
let unlistenAnswerDelta: UnlistenFn | null = null;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;
let unlistenChatCleared: UnlistenFn | null = null;
let unlistenSyncMessage: UnlistenFn | null = null;
let unlistenToolEvent: UnlistenFn | null = null;
let unlistenToolConfirm: UnlistenFn | null = null;
let unlistenToolConfirmResolved: UnlistenFn | null = null;
let unlistenDragDrop: UnlistenFn | null = null;

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
const MAX_FILE_SIZE = 10 * 1024 * 1024;
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
        showFileError(`文件过大: ${meta.name} (最大 10MB)`);
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
  if (activePage.value !== "chat") {
    isFileOver.value = false;
    return;
  }
  if (event.type === "enter" || event.type === "over") {
    isFileOver.value = true;
    return;
  }
  isFileOver.value = false;
  if (event.type === "drop" && event.paths.length > 0) {
    void addToPendingFiles(event.paths);
  }
}

async function refreshChatState() {
  try {
    const history = await invoke<any[]>("get_chat_history");
    chat.setMessages(history.map(item => ({
      role: item.role === "assistant" ? "assistant" as const : "user" as const,
      content: item.content,
      thinking: item.thinking || undefined,
      timestamp: typeof item.created_at === 'number' ? item.created_at : new Date(item.created_at).getTime(),
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
}

async function abortAi() {
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  pendingConfirm.value = null;  // Clear pending tool confirmation on abort
  try {
    await invoke("abort_ai");
  } catch {}
}

async function sendDashboardMessage() {

  const text = chatInput.value.trim();
  const files = [...pendingFiles.value];

  if (!text && files.length === 0) return;
  if (chat.isLoading) return;

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

  const displayText = text || "(已发送文件)";
  chat.addMessage("user", displayText, undefined, fileAttachments);
  chatInput.value = "";
  clearPendingFiles();
  chat.isLoading = true;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  toolEvents.value = [];
  pendingConfirm.value = null;
  pet.setState("thinking");

  // Broadcast user message to other windows (like the pet chat bubble)
  await currentWindow.emit("sync-chat-message", { role: "user", content: displayText, files: fileAttachments });

  try {
    await invoke("send_to_ai", { message: fullMessage, attachments: aiAttachments });
  } catch (err) {
    chat.isLoading = false;
    streamingAnswer.value = "";
    chat.addMessage("assistant", `出错了: ${err}`);
    pet.setState("confused");
  }
}

function upsertToolEvent(event: ToolEvent) {
  const index = toolEvents.value.findIndex((item) => item.id === event.id);
  if (index >= 0) {
    toolEvents.value[index] = { ...toolEvents.value[index], ...event };
  } else {
    toolEvents.value.push(event);
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
    // Silently handle timeout/race condition errors
    const errStr = String(e);
    if (!errStr.includes("无效的确认请求 ID")) {
      alert("确认工具操作失败: " + e);
    }
  } finally {
    pendingConfirm.value = null;
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
    dashChatScrollTimers = [80, 220].map((delay) =>
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

watch(activePage, (newPage) => {
  if (newPage === 'chat') {
    scrollDashChatToBottom();
  }
}, { flush: "post" });

// === 生命周期 ===
let unlistenPetStatus: UnlistenFn | null = null;

onMounted(async () => {
  loadSystemInfo();
  loadMemories();
  loadCurrentModel();
  loadCurrentSkin();
  loadCurrentFontColor();
  loadUserAvatar();
  loadPersonality();
  loadProfession();
  loadVoiceSettings();
  loadBackendSettings();
  loadClaudeStatus();

  sysInfoTimer = setInterval(loadSystemInfo, 5000);

  // 检查最大化状态
  isMaximized.value = await currentWindow.isMaximized();

  // 加载对话历史并监听实时同步
  await refreshChatState();

  unlistenThinking = await listen<string>("ai-thinking", (event) => {
    chat.isLoading = true;
    thinkingContent.value += event.payload;
  });

  unlistenAnswerDelta = await listen<AnswerDeltaPayload>("ai-answer-delta", (event) => {
    chat.isLoading = true;
    streamingAnswer.value += event.payload.text;
  });

  unlistenAiFinished = await listen<any>("ai-finished", (event) => {
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === event.payload.text)) {
      chat.addMessage("assistant", event.payload.text, event.payload.thinking || undefined);
    }
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    pendingConfirm.value = null;
  });

  unlistenAiError = await listen<any>("ai-error", (event) => {
    const text = event.payload.aborted ? "已中止" : `出错了: ${event.payload.message}`;
    const last = chat.messages[chat.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === text)) {
      chat.addMessage("assistant", text, event.payload.thinking || undefined);
    }
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    pendingConfirm.value = null;
  });

  unlistenChatCleared = await listen("chat-history-cleared", () => {
    resetDashChatUi();
    void loadMemories();
  });

  unlistenSyncMessage = await listen<any>("sync-chat-message", (event) => {
    const payload = event.payload;
    const last = chat.messages[chat.messages.length - 1];
    if (last && last.role === payload.role && last.content === payload.content) {
      return;
    }
    chat.addMessage(payload.role, payload.content, undefined, payload.files ?? payload.fileAttachments);
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

  // 检查桌宠窗口是否已存在
  const existing = await WebviewWindow.getByLabel("pet");
  isPetActive.value = !!existing;

  unlistenPetStatus = await listen("pet-window-closed", () => {
    isPetActive.value = false;
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
  unlistenToolConfirm?.();
  unlistenToolConfirmResolved?.();
  unlistenDragDrop?.();
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
      <div class="titlebar-charm-strip" aria-hidden="true">
        <span class="titlebar-charm charm-dot"></span>
        <span class="titlebar-charm charm-capsule"></span>
        <span class="titlebar-charm charm-star"></span>
        <span class="titlebar-charm charm-ring"></span>
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
            <span class="brand-logo">AP</span>
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
            <span class="sidebar-item-icon">⌂</span>
            <span>首页</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'chat' }]"
            @click="activePage = 'chat'"
          >
            <span class="sidebar-item-icon">💬</span>
            <span>对话互动</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'memory' }]"
            @click="activePage = 'memory'"
          >
            <span class="sidebar-item-icon">◫</span>
            <span>记忆库</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'appearance' }]"
            @click="activePage = 'appearance'"
          >
            <span class="sidebar-item-icon">◐</span>
            <span>个性外观</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'voice' }]"
            @click="activePage = 'voice'"
          >
            <span class="sidebar-item-icon">◌</span>
            <span>语音设置</span>
          </button>
          <button
            :class="['sidebar-item', { active: activePage === 'system' }]"
            @click="activePage = 'system'"
          >
            <span class="sidebar-item-icon">⌘</span>
            <span>系统设置</span>
          </button>

          <div class="sidebar-divider" />

          <button
            :class="['sidebar-item', { active: activePage === 'about' }]"
            @click="activePage = 'about'"
          >
            <span class="sidebar-item-icon">i</span>
            <span>关于</span>
          </button>
        </nav>

        <div class="sidebar-footer">
          <button
            :class="['sidebar-action-btn', 'primary', { active: isPetActive }]"
            @click="isPetActive ? emit('closePet') : emit('openPet'); isPetActive = !isPetActive"
          >
            <span class="sidebar-action-icon">{{ isPetActive ? '●' : '+' }}</span>
            <span>{{ isPetActive ? '收回桌宠' : '召唤桌宠' }}</span>
          </button>
          <button class="sidebar-action-btn danger" @click="exitApp">
            <span class="sidebar-action-icon">×</span>
            <span>退出应用</span>
          </button>
        </div>
      </div>

      <!-- 内容区 -->
      <div :class="['dashboard-content', `page-${activePage}`, { 'chat-page-active': activePage === 'chat' }]">
        <Transition name="page-fade" mode="out-in">
          <!-- ========== 首页 ========== -->
          <div v-if="activePage === 'home'" key="home" class="home-studio-page">
            <div class="page-header home-header studio-header">
              <div>
                <div class="page-kicker">Mission Playground</div>
                <h1 class="page-title">桌宠飞行舱</h1>
                <p class="page-subtitle">用任务轨道、动作卡片和状态贴纸管理你的小伙伴</p>
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
                  <span class="pet-spark spark-a">✦</span>
                  <span class="pet-spark spark-b">✧</span>
                  <div class="pet-preview-canvas-wrap">
                    <PetCanvas style="width: 140px; height: 140px; pointer-events: none;" />
                  </div>
                </div>
                <div class="pet-info">
                  <div class="pet-label">Current Companion</div>
                  <div class="pet-status-name">{{ pet.expression }} 桌宠伙伴</div>
                  <div class="pet-status-state">
                    <span class="pet-status-dot" />
                    <span>{{ petStateLabel }}</span>
                  </div>
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
                  <span class="pet-action-kicker">Action Echo</span>
                  <strong>{{ activeActionId ? '收到指令' : '待机中' }}</strong>
                  <p>{{ actionFeedback }}</p>
                </div>
              </div>


              <!-- 快捷操作 -->
              <div class="dash-card action-lab-card sticker-board-card">
                <div class="dash-card-title">
                  <span class="card-icon">⌁</span> 动作实验台
                </div>
                <div class="quick-actions">
                  <button
                    v-for="action in quickActions"
                    :key="action.id"
                    :class="['quick-action-btn', action.tone, { active: activeActionId === action.id }]"
                    @click="petAction(action.id)"
                  >
                    <span class="quick-action-icon">{{ action.icon }}</span>
                    <span class="quick-action-copy">
                      <span class="quick-action-label">{{ action.label }}</span>
                      <span class="quick-action-desc">{{ action.desc }}</span>
                    </span>
                  </button>
                </div>
              </div>

              <!-- 任务轨道 -->
              <div class="dash-card mission-rail-card mission-ribbon-card">
                <div class="dash-card-title">
                  <span class="card-icon">⌘</span> 任务轨道
                </div>
                <div class="mission-rail">
                  <button
                    v-for="entry in missionEntries"
                    :key="entry.page"
                    class="mission-node"
                    @click="activePage = entry.page"
                  >
                    <span class="mission-node-icon">{{ entry.icon }}</span>
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
                  <span class="card-icon">◷</span> 系统资源
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
            <!-- Glassmorphism drop zone overlay -->
            <div v-if="isFileOver" class="dash-drop-overlay">
              <div class="dash-drop-overlay-box">
                <span class="dash-drop-icon">📂</span>
                <span class="dash-drop-text">释放文件以添加为附件</span>
              </div>
            </div>

            <div class="dash-chat-messages" ref="dashChatMessagesRef">
              <div v-if="chat.messages.length === 0" class="dash-chat-empty">
                🐾 暂无对话历史，跟小家伙说点什么吧！
              </div>
              
              <div
                v-for="msg in chat.messages" :key="msg.id"
                :class="['dash-msg-wrapper', msg.role]"
              >
                <!-- 系统分割线 -->
                <div v-if="msg.role === 'system'" class="dash-system-divider">
                  <span class="dash-system-divider-line"></span>
                  <span class="dash-system-divider-text">{{ msg.content }}</span>
                  <span class="dash-system-divider-line"></span>
                </div>
                <div v-else class="dash-msg-bubble">
                  <div v-if="msg.thinking" class="dash-msg-thinking">
                    <details>
                      <summary>思考过程</summary>
                      <p>{{ msg.thinking }}</p>
                    </details>
                  </div>
                  <div class="dash-msg-text">{{ msg.content }}</div>

                  <!-- 显示已发送的本地文件信息 -->
                  <div v-if="msg.files && msg.files.length > 0" class="dash-msg-files">
                    <div v-for="(file, fi) in msg.files" :key="fi" class="dash-msg-file-tag">
                      <span>{{ file.isImage ? "\u{1F4F7}" : getFileIcon(file.extension) }}</span>
                      <span>{{ file.name }}</span>
                    </div>
                  </div>
                </div>
              </div>

              <div v-if="toolEvents.length > 0" class="dash-msg-wrapper assistant">
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
                  <div v-if="streamingAnswer" class="dash-msg-text">{{ streamingAnswer }}</div>
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

              <div class="dash-chat-input-area">
                <input
                  type="text"
                  v-model="chatInput"
                  @keydown.enter="sendDashboardMessage"
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
                  :disabled="(!chatInput.trim() && pendingFiles.length === 0) || chat.isLoading"
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
            <div class="page-header">
              <div class="page-kicker">Memory</div>
              <h1 class="page-title">长期记忆库</h1>
              <p class="page-subtitle">保存稳定偏好、重要背景和重置对话前的摘要</p>
            </div>

            <div class="dash-memory-layout">
              <div class="dash-card palette-panel skin-panel">
                <div class="dash-card-title"><span class="card-icon">＋</span> 新建记忆</div>
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

              <div class="dash-memory-list">
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
                      ×
                    </button>
                  </div>
                  <p>{{ memory.value }}</p>
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
              <div class="dash-card palette-panel skin-panel">
              <div class="dash-card-title"><span class="card-icon">◐</span> 主题皮肤</div>
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
              <div class="dash-card-title"><span class="card-icon">Aa</span> 字体颜色</div>
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
              <div class="dash-card-title"><span class="card-icon">▧</span> 对话背景</div>
              <div class="dash-bg-grid">
                <button :class="['dash-bg-opt', { active: chatBg === 'none' }]" @click="selectBg('none')">
                  <div class="dash-bg-preview">✖</div>
                  <span class="dash-bg-label">无</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'cute' }]" @click="selectBg('cute')">
                  <div class="dash-bg-preview">🐾</div>
                  <span class="dash-bg-label">可爱</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'scifi' }]" @click="selectBg('scifi')">
                  <div class="dash-bg-preview">🚀</div>
                  <span class="dash-bg-label">科幻</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'minimal' }]" @click="selectBg('minimal')">
                  <div class="dash-bg-preview">○</div>
                  <span class="dash-bg-label">简洁</span>
                </button>
                <button :class="['dash-bg-opt', { active: chatBg === 'custom' }]" @click="chooseBgImage">
                  <div class="dash-bg-preview">
                    <img v-if="customBgImage" :src="customBgImage" alt="" />
                    <span v-else>📷</span>
                  </div>
                  <span class="dash-bg-label">自定义</span>
                </button>
                <button v-if="chatBg === 'custom' && customBgImage" class="dash-bg-opt" @click="clearCustomBg">
                  <div class="dash-bg-preview">🗑️</div>
                  <span class="dash-bg-label">清除</span>
                </button>
              </div>
              <input ref="bgInputRef" class="hidden-input" type="file" accept="image/*" @change="onBgImageSelected" />
            </div>

            <!-- 用户头像 -->
            <div class="dash-card portrait-frame-panel avatar-panel">
              <div class="dash-card-title"><span class="card-icon">◉</span> 用户头像</div>
              <div class="dash-avatar-section">
                <div class="dash-avatar-preview">
                  <img v-if="pet.userAvatar" :src="pet.userAvatar" alt="用户头像" />
                  <span v-else>👤</span>
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
              <div class="dash-card-title"><span class="card-icon">◌</span> 语音交互</div>

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
              <div class="dash-card-title"><span class="card-icon">≋</span> 声音引擎</div>

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
                    {{ isPreviewing ? '⏹ 停止' : '▶ 试听' }}
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

            <!-- AI 后端 -->
            <div class="dash-card lab-module agent-backend-panel">
              <div class="dash-card-title"><span class="card-icon">⌘</span> AI 后端与模型</div>

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
                  <button class="dash-btn primary" @click="resetModel">{{ modelSaved ? "✅ 已重置" : "🔄 重置会话" }}</button>
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
                  {{ testResult.success ? '✅ 连接成功！' : '❌ ' + testResult.message }}
                </div>
                <div class="dash-api-actions">
                  <button class="dash-btn" @click="testConnection" :disabled="isTestingConnection || !apiConfig.base_url || !apiConfig.model">
                    {{ isTestingConnection ? '测试中...' : '测试连接' }}
                  </button>
                  <button class="dash-btn primary" @click="saveApiConfig()" :disabled="isSavingConfig">
                    {{ configSaved ? '保存成功' : (isSavingConfig ? '保存中...' : '保存配置') }}
                  </button>
                </div>
              </template>
            </div>

            <!-- 性格 -->
            <div class="system-role-grid">
            <div class="dash-card role-card personality-panel">
              <div class="dash-card-title"><span class="card-icon">✦</span> 性格</div>
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
              <div class="dash-card-title"><span class="card-icon">◇</span> 职业</div>
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
              <div class="about-logo">🐾</div>
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
                <button class="dash-btn secondary" @click="handleDashboardToolConfirm(false)">拒绝</button>
                <button class="dash-btn primary" @click="handleDashboardToolConfirm(true)">允许</button>
              </div>
            </div>
          </div>
        </Transition>
      </div>
    </div>
  </div>
</template>
