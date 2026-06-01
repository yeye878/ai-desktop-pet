<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { usePetStore, THEMES, FONT_COLORS, resolveSkinId } from "../stores/pet";
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

const emit = defineEmits<{ close: [] }>();
const pet = usePetStore();
const currentWindow = getCurrentWindow();

const systemInfo = ref({ cpu: 0, memory: 0 });

type ApiConfig = {
  id: string;
  name: string;
  api_key: string;
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

type ApiProfile = ApiConfig;

// 模型
const currentModel = ref("");
const modelSaved = ref(false);

const backendType = ref("claude_code");
const apiConfig = ref<ApiConfig>({
  id: "",
  name: "",
  api_key: "",
  base_url: "",
  model: "",
  confirm_enabled: true,
  thinking_depth: "auto",
  execution_mode: "normal",
  search_provider: "bing",
  auto_approved_tools: [],
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

// 皮肤
const currentSkin = ref("default");
const themes = Object.values(THEMES);

// 字体颜色
const currentFontColor = ref("");
const avatarInputRef = ref<HTMLInputElement | null>(null);

const BG_KEY = "ai-desktop-pet.chat-bg";
const CUSTOM_BG_KEY = "ai-desktop-pet.chat-bg-custom";
const VOICE_SETTINGS_KEY = "voice_settings";
const TTS_SETTINGS_KEY = "tts_settings";
const chatBg = ref(localStorage.getItem(BG_KEY) || "none");
const customBgImage = ref(localStorage.getItem(CUSTOM_BG_KEY) || "");
const bgInputRef = ref<HTMLInputElement | null>(null);
const voiceSettings = ref<VoiceSettings>({ ...DEFAULT_VOICE_SETTINGS });
const availableVoices = ref<SpeechSynthesisVoice[]>([]);
const ttsSettings = ref<TtsSettings>({ ...DEFAULT_TTS_SETTINGS });
const edgeVoices = ref<TtsVoice[]>([]);
const edgeVoiceSearch = ref("");
const showAllEdgeVoices = ref(false);
const ttsPreviewPlayer = new TtsPlayer();
const isPreviewing = ref(false);

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
    return [
      voice.id,
      voice.name,
      voice.language,
      voice.gender,
    ].some((value) => value.toLowerCase().includes(query));
  });
});

const edgeVoiceSummary = computed(() => {
  if (edgeVoices.value.length === 0) return "正在加载人声列表";
  if (showAllEdgeVoices.value) return `全部 ${filteredEdgeVoices.value.length} / ${edgeVoices.value.length} 个`;
  return `当前语言 ${filteredEdgeVoices.value.length} / ${edgeVoices.value.length} 个`;
});

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

const thinkingDepthOptions = [
  { value: "auto", label: "自动" },
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高" },
];

const executionModeOptions = [
  { value: "plan", label: "计划模式" },
  { value: "normal", label: "普通模式" },
  { value: "unreviewed", label: "无审查模式" },
  { value: "custom", label: "自定义模式" },
];

const searchProviderOptions = [
  { value: "bing", label: "Bing", desc: "国内网络推荐使用" },
  { value: "duckduckgo", label: "DuckDuckGo", desc: "海外网络可选" },
];

function thinkingDepthLabel(value: string) {
  return thinkingDepthOptions.find((item) => item.value === value)?.label || "自动";
}

function searchProviderLabel(value: string) {
  return searchProviderOptions.find((item) => item.value === value)?.label || "Bing";
}

async function loadSystemInfo() {
  try {
    systemInfo.value = await invoke("get_system_info");
  } catch {}
}

async function loadCurrentModel() {
  try {
    currentModel.value = await invoke("get_current_model");
  } catch {}
}

async function loadCurrentSkin() {
  try {
    currentSkin.value = resolveSkinId(await invoke("get_skin"));
    pet.skin = currentSkin.value;
  } catch {}
}

async function loadCurrentFontColor() {
  try {
    currentFontColor.value = await invoke("get_font_color");
    pet.fontColor = currentFontColor.value;
  } catch {
    currentFontColor.value = pet.fontColor;
  }
}

async function loadUserAvatar() {
  try {
    pet.userAvatar = await invoke("get_user_avatar");
  } catch {}
}

async function loadPersonality() {
  try {
    currentPersonality.value = await invoke("get_personality");
  } catch {}
}

async function loadProfession() {
  try {
    currentProfession.value = await invoke("get_profession");
  } catch {}
}

async function loadVoiceSettings() {
  try {
    voiceSettings.value = parseVoiceSettings(await invoke<string>("get_setting_value", {
      key: VOICE_SETTINGS_KEY,
    }));
  } catch {
    voiceSettings.value = { ...DEFAULT_VOICE_SETTINGS };
  }

  availableVoices.value = await getAvailableVoices();

  try {
    const raw = await invoke<string>("get_setting_value", { key: TTS_SETTINGS_KEY });
    if (raw) ttsSettings.value = { ...DEFAULT_TTS_SETTINGS, ...JSON.parse(raw) };
  } catch {
    ttsSettings.value = { ...DEFAULT_TTS_SETTINGS };
  }

  try {
    edgeVoices.value = await listEdgeVoices();
  } catch {}

  if (ttsSettings.value.engine === "edge" && edgeVoices.value.length > 0) {
    const hasSavedVoice = edgeVoices.value.some((voice) => voice.id === ttsSettings.value.voice);
    if (!hasSavedVoice) {
      await updateTtsSettings({ voice: edgeVoices.value[0].id });
    }
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
    }
  } catch (e) {
    console.error("加载后端设置失败", e);
  }
}

function applyApiConfig(config: Partial<ApiConfig>) {
  apiConfig.value = {
    id: config.id || "",
    name: config.name || config.model || "",
    api_key: "", // Keep empty to prevent browser autofill/overwrite bugs
    api_key_mask: config.api_key_mask || "",
    has_api_key: config.has_api_key || Boolean(config.api_key),
    base_url: config.base_url || "https://api.openai.com/v1",
    model: config.model || "gpt-4o-mini",
    confirm_enabled: config.confirm_enabled !== false,
    thinking_depth: config.thinking_depth || "auto",
    execution_mode: config.execution_mode || (config.confirm_enabled === false ? "unreviewed" : "normal"),
    search_provider: config.search_provider || "bing",
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

async function selectBackend(type: string) {
  try {
    await invoke("set_backend_type", { backend: type });
    backendType.value = type;
    if (type === "claude_code") {
      await loadCurrentModel();
    } else {
      currentModel.value = `直连 API: ${apiConfig.value.model}`;
    }
  } catch (e) {
    alert("切换后端失败: " + e);
  }
}

const PRESETS = {
  openai: {
    base_url: "https://api.openai.com/v1",
    model: "gpt-4o-mini"
  },
  deepseek: {
    base_url: "https://api.deepseek.com/v1",
    model: "deepseek-chat"
  },
  qwen: {
    base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus"
  },
  ollama: {
    base_url: "http://localhost:11434/v1",
    model: "qwen2.5:7b"
  }
};

function applyPreset(key: keyof typeof PRESETS) {
  const preset = PRESETS[key];
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
    const saved = await invoke<ApiConfig>("set_api_config", {
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
    setTimeout(() => {
      configSaved.value = false;
    }, 2000);
  } catch (e) {
    if (!options.quiet) alert("保存失败: " + e);
    if (options.quiet) throw e;
  } finally {
    isSavingConfig.value = false;
  }
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
  } catch (e: any) {
    testResult.value = { success: false, message: e.toString() };
  } finally {
    isTestingConnection.value = false;
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
      message: `已获取 ${models.length} 个模型，请选择后加入模型列表。`,
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
    const profile = await invoke<ApiConfig>("set_active_api_profile", { id });
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
    await loadCurrentModel();
    modelSaved.value = true;
    setTimeout(() => (modelSaved.value = false), 2000);
  } catch (e) {
    alert("重置失败: " + e);
  }
}

async function startClaudeConfig() {
  try {
    await invoke("open_claude_config");
  } catch (e) {
    alert("启动配置终端失败: " + e);
  }
}

async function selectSkin(skinId: string) {
  try {
    await invoke("set_skin", { skin: skinId });
    currentSkin.value = skinId;
    pet.skin = skinId;
    // 发送到宠物窗口
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) {
    alert("皮肤切换失败: " + e);
  }
}

async function selectFontColor(value: string) {
  try {
    await invoke("set_font_color", { fontColor: value });
    currentFontColor.value = value;
    pet.fontColor = value;
    // 发送到宠物窗口
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) {
    alert("字体颜色保存失败: " + e);
  }
}

function chooseAvatar() {
  avatarInputRef.value?.click();
}

async function onAvatarSelected(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;

  if (!file.type.startsWith("image/")) {
    alert("请选择图片文件");
    return;
  }

  const avatar = await readAvatarFile(file);
  try {
    await invoke("set_user_avatar", { avatar });
    pet.userAvatar = avatar;
    // 发送到宠物窗口
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) {
    alert("头像保存失败: " + e);
  }
}

async function clearAvatar() {
  try {
    await invoke("set_user_avatar", { avatar: "" });
    pet.userAvatar = "";
    // 发送到宠物窗口
    const petWin = await WebviewWindow.getByLabel("pet");
    if (petWin) await petWin.emit("appearance-changed");
  } catch (e) {
    alert("头像清除失败: " + e);
  }
}

function readAvatarFile(file: File): Promise<string> {
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

function chooseBgImage() {
  bgInputRef.value?.click();
}

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
  try {
    await invoke("set_personality", { personality });
    currentPersonality.value = personality;
  } catch (e) {
    alert("性格切换失败: " + e);
  }
}

async function selectProfession(profession: string) {
  try {
    await invoke("set_profession", { profession });
    currentProfession.value = profession;
  } catch (e) {
    alert("职业切换失败: " + e);
  }
}

async function updateTtsSettings(patch: Partial<TtsSettings>) {
  ttsSettings.value = { ...ttsSettings.value, ...patch };
  try {
    await invoke("set_setting_value", {
      key: TTS_SETTINGS_KEY,
      value: JSON.stringify(ttsSettings.value),
    });
  } catch {}
}

async function previewVoice() {
  if (isPreviewing.value) {
    ttsPreviewPlayer.stop();
    isPreviewing.value = false;
    return;
  }
  isPreviewing.value = true;
  try {
    await ttsPreviewPlayer.speak("你好，我是你的桌宠伙伴，很高兴认识你。", ttsSettings.value);
  } catch (e: any) {
    if (e.message !== "Aborted") {
      alert("试听失败: " + (e.message || e));
    }
  }
  isPreviewing.value = false;
}

async function updateVoiceSettings(patch: Partial<VoiceSettings>) {
  voiceSettings.value = {
    ...voiceSettings.value,
    ...patch,
  };

  try {
    await invoke("set_setting_value", {
      key: VOICE_SETTINGS_KEY,
      value: serializeVoiceSettings(voiceSettings.value),
    });
    window.dispatchEvent(new CustomEvent("voice-settings-changed"));
    await currentWindow.emit("voice-settings-changed");
  } catch (e) {
    alert("语音设置保存失败: " + e);
  }
}

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

const activeTab = ref("appearance");
let unlistenSwitchTab: UnlistenFn | null = null;

onMounted(async () => {
  loadSystemInfo();
  loadCurrentModel();
  loadCurrentSkin();
  loadCurrentFontColor();
  loadUserAvatar();
  loadPersonality();
  loadProfession();
  loadVoiceSettings();
  loadBackendSettings();

  const params = new URLSearchParams(window.location.search);
  const tabParam = params.get("tab");
  if (tabParam) {
    activeTab.value = tabParam;
  }

  unlistenSwitchTab = await listen<string>("switch-tab", (event) => {
    activeTab.value = event.payload;
  });
});

onUnmounted(() => {
  unlistenSwitchTab?.();
});
</script>

<template>
  <div class="settings-mask" @click="emit('close')" @mousedown.stop />
  <div class="settings-panel">
    <div class="header-decor"></div>
    <div class="settings-header">
      <span>&#x2699;&#xFE0F; 设置</span>
      <button class="close-btn" @click="emit('close')">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M1 1L13 13M13 1L1 13" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
      </button>
    </div>

    <!-- Tab navigation -->
    <div class="settings-tabs">
      <button :class="['tab-btn', { active: activeTab === 'appearance' }]" @click="activeTab = 'appearance'">
        <span>🎨</span><span>外观</span>
      </button>
      <button :class="['tab-btn', { active: activeTab === 'voice' }]" @click="activeTab = 'voice'">
        <span>🎙️</span><span>语音</span>
      </button>
      <button :class="['tab-btn', { active: activeTab === 'system' }]" @click="activeTab = 'system'">
        <span>⚙️</span><span>系统</span>
      </button>
    </div>

    <div class="settings-body">
      <!-- APPEARANCE TAB -->
      <template v-if="activeTab === 'appearance'">
        <!-- 主题皮肤 -->
        <div class="section">
          <h3>&#x1F3A8; 主题皮肤</h3>
          <div class="skin-grid">
            <div
              v-for="t in themes"
              :key="t.id"
              class="skin-item"
              :class="{ active: currentSkin === t.id, animated: t.animated }"
              @click="selectSkin(t.id)"
            >
              <div class="skin-preview" :style="{ background: t.headerGradient }">
                <span class="skin-check" v-if="currentSkin === t.id">&#x2714;</span>
              </div>
              <span>{{ t.name }}</span>
              <small v-if="t.animated">随时间变色</small>
            </div>
          </div>
        </div>

        <!-- 字体颜色 -->
        <div class="section">
          <h3>&#x1F58A;&#xFE0F; 字体颜色</h3>
          <div class="font-color-grid">
            <div
              v-for="fc in FONT_COLORS"
              :key="fc.id"
              class="font-color-item"
              :class="{ active: currentFontColor === fc.value }"
              @click="selectFontColor(fc.value)"
            >
              <div
                class="font-color-preview"
                :style="{
                  color: fc.value || 'var(--pet-bubble-bot-text, #4a4a4a)',
                }"
              >
                Aa
              </div>
              <span>{{ fc.name }}</span>
            </div>
          </div>
        </div>

        <!-- 对话背景 -->
        <div class="section">
          <h3>&#x1F5BC;&#xFE0F; 对话背景</h3>
          <div class="bg-grid">
            <button :class="['bg-opt', { active: chatBg === 'none' }]" @click="selectBg('none')">
              <div class="bg-opt-preview bg-prev-none">&#x2716;</div>
              <span>无</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'cute' }]" @click="selectBg('cute')">
              <div class="bg-opt-preview bg-prev-cute">&#x1F43E;</div>
              <span>可爱</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'scifi' }]" @click="selectBg('scifi')">
              <div class="bg-opt-preview bg-prev-scifi">&#x1F680;</div>
              <span>科幻</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'minimal' }]" @click="selectBg('minimal')">
              <div class="bg-opt-preview bg-prev-minimal">&#x25CB;</div>
              <span>简洁</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'custom' }]" @click="chooseBgImage">
              <div class="bg-opt-preview bg-prev-custom">
                <img v-if="customBgImage" :src="customBgImage" alt="" />
                <span v-else>&#x1F4F7;</span>
              </div>
              <span>自定义</span>
            </button>
            <button v-if="chatBg === 'custom' && customBgImage" class="bg-opt danger" @click="clearCustomBg">
              <div class="bg-opt-preview bg-prev-none">&#x1F5D1;</div>
              <span>清除</span>
            </button>
          </div>
          <input ref="bgInputRef" class="avatar-input" type="file" accept="image/*" @change="onBgImageSelected" />
        </div>

        <!-- 用户头像 -->
        <div class="section">
          <h3>&#x1F464; 用户头像</h3>
          <div class="avatar-setting">
            <div class="avatar-preview">
              <img v-if="pet.userAvatar" :src="pet.userAvatar" alt="用户头像" />
              <span v-else>&#x1F464;</span>
            </div>
            <div class="avatar-actions">
              <button class="avatar-btn primary" @click="chooseAvatar">选择图片</button>
              <button
                class="avatar-btn"
                :disabled="!pet.userAvatar"
                @click="clearAvatar"
              >
                恢复默认
              </button>
            </div>
            <input
              ref="avatarInputRef"
              class="avatar-input"
              type="file"
              accept="image/*"
              @change="onAvatarSelected"
            />
          </div>
        </div>
      </template>

      <!-- VOICE TAB -->
      <template v-else-if="activeTab === 'voice'">
        <!-- 语音交互 -->
        <div class="section">
          <h3>语音交互</h3>
          <div class="voice-options">
            <label class="switch-row">
              <span>
                <strong>启用语音</strong>
                <small>允许麦克风输入和回复播报</small>
              </span>
              <input
                type="checkbox"
                :checked="voiceSettings.enabled"
                @change="updateVoiceSettings({ enabled: ($event.target as HTMLInputElement).checked })"
              />
            </label>
            <label class="switch-row">
              <span>
                <strong>识别后自动发送</strong>
                <small>关闭后会先填入输入框</small>
              </span>
              <input
                type="checkbox"
                :checked="voiceSettings.autoSend"
                @change="updateVoiceSettings({ autoSend: ($event.target as HTMLInputElement).checked })"
              />
            </label>
            <label class="switch-row">
              <span>
                <strong>自动朗读回复</strong>
                <small>AI 回复完成后直接播报</small>
              </span>
              <input
                type="checkbox"
                :checked="voiceSettings.autoSpeak"
                @change="updateVoiceSettings({ autoSpeak: ($event.target as HTMLInputElement).checked })"
              />
            </label>

            <label class="field-row">
              <span>快捷键</span>
              <input
                type="text"
                :value="voiceSettings.shortcut"
                placeholder="Alt+V"
                @change="updateVoiceSettings({ shortcut: ($event.target as HTMLInputElement).value.trim() || DEFAULT_VOICE_SETTINGS.shortcut })"
              />
            </label>

            <label class="field-row">
              <span>语言</span>
              <select
                :value="voiceSettings.language"
                @change="updateVoiceSettings({ language: ($event.target as HTMLSelectElement).value })"
              >
                <option value="zh-CN">中文普通话</option>
                <option value="en-US">English (US)</option>
                <option value="ja-JP">日本語</option>
              </select>
            </label>

            <label class="field-row">
              <span>声音引擎</span>
              <select
                :value="ttsSettings.engine"
                @change="updateTtsSettings({ engine: ($event.target as HTMLSelectElement).value as any })"
              >
                <option value="edge">微软自然语音</option>
                <option value="system">系统语音</option>
              </select>
            </label>

            <template v-if="ttsSettings.engine === 'edge'">
              <div class="voice-picker-tools">
                <input
                  class="voice-search"
                  type="search"
                  v-model="edgeVoiceSearch"
                  placeholder="搜索人声、地区或编号"
                />
                <label class="mini-check">
                  <input type="checkbox" v-model="showAllEdgeVoices" />
                  <span>显示全部</span>
                </label>
              </div>
              <label class="field-row">
                <span>
                  声音角色
                  <small>{{ edgeVoiceSummary }}</small>
                </span>
                <select
                  :value="ttsSettings.voice"
                  @change="updateTtsSettings({ voice: ($event.target as HTMLSelectElement).value })"
                  :disabled="filteredEdgeVoices.length === 0"
                >
                  <option
                    v-for="v in filteredEdgeVoices"
                    :key="v.id"
                    :value="v.id"
                  >
                    {{ v.name }} · {{ v.language }} · {{ v.gender === 'Female' ? '女' : '男' }}
                  </option>
                </select>
              </label>
              <div v-if="filteredEdgeVoices.length === 0" class="voice-empty">
                没找到匹配的人声，可以清空搜索或打开“显示全部”。
              </div>
              <div class="preview-row">
                <button class="preview-btn" :disabled="filteredEdgeVoices.length === 0" @click="previewVoice">
                  {{ isPreviewing ? '停止' : '试听' }}
                </button>
              </div>
            </template>

            <template v-else>
              <label class="field-row">
                <span>声音</span>
                <select
                  :value="voiceSettings.voiceName"
                  @change="updateVoiceSettings({ voiceName: ($event.target as HTMLSelectElement).value })"
                >
                  <option value="">自动选择</option>
                  <option
                    v-for="voice in availableVoices"
                    :key="voice.name"
                    :value="voice.name"
                  >
                    {{ voice.name }} · {{ voice.lang }}
                  </option>
                </select>
              </label>
            </template>

            <template v-if="ttsSettings.engine === 'edge'">
              <label class="range-row">
                <span>语速 {{ ttsSettings.rate >= 0 ? '+' : '' }}{{ ttsSettings.rate }}%</span>
                <input
                  type="range"
                  min="-50"
                  max="100"
                  step="10"
                  :value="ttsSettings.rate"
                  @input="updateTtsSettings({ rate: Number(($event.target as HTMLInputElement).value) })"
                />
              </label>
              <label class="range-row">
                <span>音调 {{ ttsSettings.pitch >= 0 ? '+' : '' }}{{ ttsSettings.pitch }}Hz</span>
                <input
                  type="range"
                  min="-50"
                  max="50"
                  step="5"
                  :value="ttsSettings.pitch"
                  @input="updateTtsSettings({ pitch: Number(($event.target as HTMLInputElement).value) })"
                />
              </label>
              <label class="range-row">
                <span>音量 {{ ttsSettings.volume }}%</span>
                <input
                  type="range"
                  min="0"
                  max="100"
                  step="10"
                  :value="ttsSettings.volume"
                  @input="updateTtsSettings({ volume: Number(($event.target as HTMLInputElement).value) })"
                />
              </label>
            </template>
            <template v-else>
              <label class="range-row">
                <span>语速 {{ voiceSettings.rate.toFixed(1) }}</span>
                <input
                  type="range"
                  min="0.6"
                  max="1.5"
                  step="0.1"
                  :value="voiceSettings.rate"
                  @input="updateVoiceSettings({ rate: Number(($event.target as HTMLInputElement).value) })"
                />
              </label>
              <label class="range-row">
                <span>音调 {{ voiceSettings.pitch.toFixed(1) }}</span>
                <input
                  type="range"
                  min="0.6"
                  max="1.6"
                  step="0.1"
                  :value="voiceSettings.pitch"
                  @input="updateVoiceSettings({ pitch: Number(($event.target as HTMLInputElement).value) })"
                />
              </label>
              <label class="range-row">
                <span>音量 {{ Math.round(voiceSettings.volume * 100) }}%</span>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.1"
                  :value="voiceSettings.volume"
                  @input="updateVoiceSettings({ volume: Number(($event.target as HTMLInputElement).value) })"
                />
              </label>
            </template>
          </div>
        </div>
      </template>

      <!-- SYSTEM TAB -->
      <template v-else-if="activeTab === 'system'">
        <!-- 系统状态 -->
        <div class="section">
          <h3>&#x1F4CA; 系统状态</h3>
          <div class="stat-row">
            <span class="stat-label">CPU</span>
            <div class="stat-bar">
              <div
                class="stat-fill"
                :style="{ width: systemInfo.cpu + '%', background: cpuBarColor(systemInfo.cpu) }"
              />
            </div>
            <span class="stat-value">{{ systemInfo.cpu.toFixed(1) }}%</span>
          </div>
          <div class="stat-row">
            <span class="stat-label">内存</span>
            <div class="stat-bar">
              <div
                class="stat-fill"
                :style="{ width: systemInfo.memory + '%', background: memBarColor(systemInfo.memory) }"
              />
            </div>
            <span class="stat-value">{{ systemInfo.memory.toFixed(1) }}%</span>
          </div>
        </div>

        <!-- AI 后端与模型 -->
        <div class="section">
          <h3>&#x1F916; AI 后端与模型</h3>
          
          <div class="backend-selector">
            <button 
              class="backend-btn" 
              :class="{ active: backendType === 'claude_code' }"
              @click="selectBackend('claude_code')"
            >
              <span>🐚</span> Claude Code
            </button>
            <button 
              class="backend-btn" 
              :class="{ active: backendType === 'direct_api' }"
              @click="selectBackend('direct_api')"
            >
              <span>🌐</span> 直连 API Agent
            </button>
          </div>

          <!-- Claude Code 模式 -->
          <template v-if="backendType === 'claude_code'">
            <p class="current-model">{{ currentModel || '加载中...' }}</p>
            <p class="hint">使用本地 Claude Code CLI 子进程，会话自动保持上下文。</p>
            <div class="model-actions">
              <button class="action-btn" @click="resetModel">
                {{ modelSaved ? "&#x2705; 已重置" : "&#x1F504; 重置会话" }}
              </button>
              <button class="action-btn config-btn" @click="startClaudeConfig">
                &#x1F527; 配置/登录 Claude Code
              </button>
            </div>
          </template>

          <!-- 直连 API Agent 模式 -->
          <template v-else>
            <div class="profile-toolbar">
              <div>
                <div class="profile-toolbar-title">已保存模型配置</div>
                <div class="profile-toolbar-subtitle">列表中隐藏 API Key，可点选后立即切换。</div>
              </div>
              <button class="mini-action-btn" type="button" @click="newApiProfileDraft">新建</button>
            </div>

            <div v-if="apiProfiles.length > 0" class="profile-list">
              <div
                v-for="profile in apiProfiles"
                :key="profile.id"
                class="profile-item"
                :class="{ active: profile.id === activeApiProfileId }"
              >
                <button class="profile-main" type="button" @click="activateApiProfile(profile.id)">
                  <span class="profile-name">{{ profile.name || profile.model }}</span>
                  <span class="profile-meta">{{ profile.model }} · {{ profile.base_url }}</span>
                  <span class="profile-badges">
                    <span>思考 {{ thinkingDepthLabel(profile.thinking_depth) }}</span>
                    <span>搜索 {{ searchProviderLabel(profile.search_provider) }}</span>
                    <span>{{ profile.has_api_key ? 'API Key 已隐藏' : '无 API Key' }}</span>
                  </span>
                </button>
                <button
                  class="profile-delete-btn"
                  type="button"
                  title="删除模型配置"
                  :disabled="profileDeletingId === profile.id || apiProfiles.length <= 1"
                  @click="deleteApiProfile(profile.id)"
                >
                  ×
                </button>
              </div>
            </div>

            <div class="api-form">
              <div class="form-group">
                <label>配置名称</label>
                <input
                  type="text"
                  v-model="apiConfig.name"
                  placeholder="OpenAI 工作模型"
                />
              </div>

              <div class="form-group">
                <label>接口地址 (Base URL)</label>
                <div class="inline-control">
                  <input
                    type="text"
                    v-model="apiConfig.base_url"
                    placeholder="https://api.openai.com/v1"
                    @input="clearFetchedModels"
                  />
                  <button
                    class="mini-action-btn"
                    type="button"
                    :disabled="isFetchingModels || !apiConfig.base_url.trim()"
                    @click="fetchApiModels"
                  >
                    {{ isFetchingModels ? '获取中' : '获取模型' }}
                  </button>
                </div>
              </div>

              <div class="form-group">
                <label>API 密钥 (API Key)</label>
                <input 
                  type="password" 
                  v-model="apiConfig.api_key" 
                  :placeholder="apiConfig.has_api_key ? '•••••••••••••••• (已保存)' : '请输入 sk-... 格式的 API Key'" 
                  autocomplete="new-password"
                  @input="clearFetchedModels"
                />
                <p class="hint compact">已保存模型列表只显示“API Key 已隐藏”，不会明文展示密钥。</p>
              </div>

              <div class="form-group">
                <label>模型名称 (Model)</label>
                <input 
                  type="text" 
                  v-model="apiConfig.model" 
                  placeholder="gpt-4o-mini" 
                />
              </div>

              <div
                v-if="modelFetchResult.message"
                class="test-result compact"
                :class="modelFetchResult.success ? 'success' : 'error'"
              >
                {{ modelFetchResult.message }}
              </div>

              <div v-if="fetchedModels.length > 0" class="model-picker">
                <div class="model-picker-row">
                  <span>选择模型</span>
                  <select
                    :value="selectedFetchedModel"
                    @change="selectFetchedModel(($event.target as HTMLSelectElement).value)"
                  >
                    <option v-for="model in fetchedModels" :key="model" :value="model">
                      {{ model }}
                    </option>
                  </select>
                </div>
                <button
                  class="action-btn"
                  type="button"
                  :disabled="isAddingFetchedModel || !selectedFetchedModel"
                  @click="addFetchedModelProfile"
                >
                  {{ isAddingFetchedModel ? '加入中...' : '加入模型列表' }}
                </button>
              </div>

              <div class="form-group">
                <label>思考深度</label>
                <div class="thinking-switch">
                  <button
                    v-for="item in thinkingDepthOptions"
                    :key="item.value"
                    type="button"
                    class="thinking-option"
                    :class="{ active: apiConfig.thinking_depth === item.value }"
                    @click="updateThinkingDepth(item.value)"
                  >
                    {{ item.label }}
                  </button>
                </div>
                <p class="hint compact">切换后会保存到当前模型，并在下一次直连 API 请求中生效。</p>
              </div>

              <div class="form-group">
                <label>优先搜索源</label>
                <select
                  :value="apiConfig.search_provider"
                  @change="updateSearchProvider(($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="item in searchProviderOptions" :key="item.value" :value="item.value">
                    {{ item.label }} - {{ item.desc }}
                  </option>
                </select>
                <p class="hint compact">国内网络推荐使用 Bing；DuckDuckGo 在国内网络下容易超时。</p>
              </div>

              <div class="form-group">
                <label>执行模式</label>
                <select
                  v-model="apiConfig.execution_mode"
                  @change="apiConfig.confirm_enabled = apiConfig.execution_mode !== 'unreviewed'"
                >
                  <option v-for="item in executionModeOptions" :key="item.value" :value="item.value">
                    {{ item.label }}
                  </option>
                </select>
              </div>

              <div class="form-group">
                <label>预设一键填充</label>
                <div class="presets-container">
                  <span class="preset-badge" @click="applyPreset('openai')">OpenAI</span>
                  <span class="preset-badge" @click="applyPreset('deepseek')">DeepSeek</span>
                  <span class="preset-badge" @click="applyPreset('qwen')">通义千问</span>
                  <span class="preset-badge" @click="applyPreset('ollama')">Ollama本地</span>
                </div>
              </div>

              <div class="toggle-group">
                <label>工具调用二次确认 (推荐)</label>
                <input 
                  type="checkbox" 
                  class="toggle-input" 
                  :checked="apiConfig.execution_mode !== 'unreviewed'"
                  @change="apiConfig.execution_mode = ($event.target as HTMLInputElement).checked ? 'normal' : 'unreviewed'"
                />
              </div>
              <p class="hint" style="margin-top: -4px;">开启后，敏感工具（如命令执行、文件写入）在运行前需要您手动允许。</p>

              <!-- 测试连接结果 -->
              <div 
                v-if="testResult.message" 
                class="test-result" 
                :class="testResult.success ? 'success' : 'error'"
              >
                {{ testResult.success ? '✅ 连接成功！' : '❌ ' + testResult.message }}
              </div>

              <div class="api-actions">
                <button 
                  class="action-btn test-btn" 
                  @click="testConnection" 
                  :disabled="isTestingConnection || !apiConfig.base_url || !apiConfig.model"
                >
                  {{ isTestingConnection ? '⏳ 测试中...' : '🔌 测试连接' }}
                </button>
                <button 
                  class="action-btn" 
                  @click="saveApiConfig()"
                  :disabled="isSavingConfig"
                >
                  {{ configSaved ? '✅ 保存成功' : (isSavingConfig ? '⏳ 保存中...' : '💾 保存配置') }}
                </button>
              </div>
            </div>
          </template>
        </div>

        <!-- 性格 -->
        <div class="section">
          <h3>&#x2728; 性格</h3>
          <div class="option-grid personality-grid">
            <button
              v-for="item in personalities"
              :key="item.id"
              class="option-item"
              :class="{ active: currentPersonality === item.id }"
              @click="selectPersonality(item.id)"
            >
              <span class="option-name">{{ item.name }}</span>
              <span class="option-desc">{{ item.desc }}</span>
            </button>
          </div>
          <p class="hint">切换后会重置 Claude 会话，让新设定立即生效</p>
        </div>

        <!-- 职业 -->
        <div class="section">
          <h3>&#x1F9ED; 职业</h3>
          <div class="option-grid profession-grid">
            <button
              v-for="item in professions"
              :key="item.id"
              class="option-item"
              :class="{ active: currentProfession === item.id }"
              @click="selectProfession(item.id)"
            >
              <span class="option-name">{{ item.name }}</span>
              <span class="option-desc">{{ item.desc }}</span>
            </button>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.settings-mask {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: transparent;
  backdrop-filter: none;
  z-index: 300;
}

.settings-panel {
  position: absolute;
  left: var(--settings-panel-left, 0);
  top: var(--settings-panel-top, 0);
  width: var(--settings-panel-width, 100%);
  height: var(--settings-panel-height, 100%);
  pointer-events: auto;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.82), rgba(255, 255, 255, 0.62)),
    var(--pet-bg-glass, rgba(255, 255, 255, 0.92));
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border-radius: 14px;
  box-shadow:
    0 18px 42px rgba(15, 23, 42, 0.16),
    0 4px 12px rgba(15, 23, 42, 0.08),
    inset 0 1px 0 rgba(255, 255, 255, 0.5);
  border: 1px solid rgba(255, 255, 255, 0.6);
  z-index: 301;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.header-decor {
  height: 3px;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  flex-shrink: 0;
}

.settings-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 11px 14px;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 240% 240%;
  color: white;
  font-size: 14px;
  font-weight: 600;
  animation: softGradientShift 10s ease-in-out infinite;
}

.close-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(255, 255, 255, 0.15);
  border: none;
  border-radius: 50%;
  width: 26px;
  height: 26px;
  color: white;
  cursor: pointer;
  transition: all 0.2s;
}

.close-btn:hover {
  background: rgba(255, 255, 255, 0.35);
  transform: rotate(90deg);
}

.settings-body {
  padding: 14px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
  scrollbar-width: thin;
  scrollbar-color: rgba(0, 0, 0, 0.1) transparent;
}

.settings-body::-webkit-scrollbar {
  width: 4px;
}

.settings-body::-webkit-scrollbar-thumb {
  background: rgba(0, 0, 0, 0.1);
  border-radius: 4px;
}

.section {
  padding: 11px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.48);
  border: 1px solid rgba(15, 23, 42, 0.05);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.52);
}

.section h3 {
  font-size: 13px;
  color: var(--pet-font-color, #475569);
  margin-bottom: 10px;
  font-weight: 600;
}

.current-model {
  font-size: 12px;
  color: #475569;
  background: rgba(15, 23, 42, 0.04);
  padding: 8px 10px;
  border-radius: 8px;
  margin-bottom: 8px;
  border: 1px solid rgba(15, 23, 42, 0.05);
}

.hint {
  font-size: 11px;
  color: #94a3b8;
  margin-bottom: 8px;
  line-height: 1.5;
}

.action-btn {
  width: 100%;
  padding: 8px;
  background: var(--pet-primary, #ff6b6b);
  color: white;
  border: none;
  border-radius: 10px;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;
  box-shadow: 0 8px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.22);
}

.action-btn:hover {
  background: var(--pet-primary-dark, #e55a5a);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}

.model-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.config-btn {
  background: linear-gradient(135deg, #a78bfa, #8b5cf6) !important;
  box-shadow: 0 8px 18px rgba(139, 92, 246, 0.22) !important;
}

.config-btn:hover {
  background: linear-gradient(135deg, #8b5cf6, #7c3aed) !important;
}

/* ===== 性格 / 职业 ===== */
.option-grid {
  display: grid;
  gap: 8px;
}

.personality-grid {
  grid-template-columns: repeat(2, 1fr);
}

.profession-grid {
  grid-template-columns: repeat(2, 1fr);
}

.option-item {
  min-height: 58px;
  padding: 8px 9px;
  border: 1px solid rgba(15, 23, 42, 0.06);
  border-radius: 9px;
  background: rgba(255, 255, 255, 0.6);
  color: var(--pet-font-color, #555);
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  justify-content: center;
  gap: 3px;
  cursor: pointer;
  text-align: left;
  transition: all 0.2s;
}

.option-item:hover {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.32);
  background: rgba(255, 255, 255, 0.8);
  transform: translateY(-1px);
}

.option-item.active {
  border-color: var(--pet-primary, #ff6b6b);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.08);
  box-shadow: 0 8px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.12);
}

.option-name {
  font-size: 12px;
  font-weight: 700;
  color: #475569;
}

.option-desc {
  font-size: 10px;
  line-height: 1.35;
  color: #94a3b8;
}

.option-item.active .option-name {
  color: var(--pet-primary, #ff6b6b);
}

.voice-options {
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.switch-row,
.field-row,
.range-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 9px;
  border: 1px solid rgba(15, 23, 42, 0.06);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.58);
}

.switch-row span {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.field-row span {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.switch-row strong,
.field-row span,
.range-row span {
  font-size: 12px;
  color: #475569;
}

.field-row small {
  font-size: 9px;
  line-height: 1.25;
  color: #94a3b8;
  font-weight: 500;
}

.voice-picker-tools {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 1px;
}

.voice-search {
  flex: 1;
  min-width: 0;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  padding: 7px 9px;
  background: rgba(255, 255, 255, 0.86);
  color: #475569;
  font-size: 11px;
  outline: none;
}

.voice-search:focus {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.32);
}

.mini-check {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
  color: #64748b;
  font-size: 11px;
  white-space: nowrap;
}

.mini-check input {
  width: 14px;
  height: 14px;
  accent-color: var(--pet-primary, #ff6b6b);
}

.voice-empty {
  padding: 6px 9px 2px;
  color: #94a3b8;
  font-size: 10px;
  line-height: 1.4;
}

.preview-row {
  display: flex;
  justify-content: flex-end;
  padding: 4px 9px;
}

.preview-btn {
  padding: 6px 16px;
  border: 1px solid rgba(15, 23, 42, 0.12);
  border-radius: 8px;
  background: linear-gradient(135deg, #e0f2fe, #f0e6ff);
  color: #334155;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s;
}

.preview-btn:hover {
  opacity: 0.8;
}

.preview-btn:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.switch-row small {
  font-size: 10px;
  line-height: 1.35;
  color: #94a3b8;
}

.switch-row input[type="checkbox"] {
  width: 16px;
  height: 16px;
  accent-color: var(--pet-primary, #ff6b6b);
  flex-shrink: 0;
}

.field-row select {
  min-width: 0;
  width: 170px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  padding: 6px 8px;
  background: rgba(255, 255, 255, 0.86);
  color: #475569;
  font-size: 11px;
  outline: none;
}

.field-row input[type="text"] {
  min-width: 0;
  width: 170px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  padding: 6px 8px;
  background: rgba(255, 255, 255, 0.86);
  color: #475569;
  font-size: 11px;
  outline: none;
}

.range-row {
  align-items: flex-start;
  flex-direction: column;
}

.range-row input[type="range"] {
  width: 100%;
  accent-color: var(--pet-primary, #ff6b6b);
}

/* ===== 皮肤网格 ===== */
.skin-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
}

.skin-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  min-height: 82px;
  padding: 8px 4px 7px;
  border: 2px solid transparent;
  border-radius: 11px;
  cursor: pointer;
  transition: all 0.25s;
  background: rgba(255, 255, 255, 0.32);
}

.skin-item:hover {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.25);
  background: rgba(255, 255, 255, 0.72);
  transform: translateY(-2px);
}

.skin-item.active {
  border-color: var(--pet-primary, #ff6b6b);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.09);
}

.skin-preview {
  width: 42px;
  height: 42px;
  border-radius: 14px;
  background-size: 220% 220%;
  box-shadow:
    0 8px 18px rgba(15, 23, 42, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 0.2s, box-shadow 0.2s;
}

.skin-item.animated .skin-preview {
  animation: softGradientShift 5.6s ease-in-out infinite;
}

.skin-item:hover .skin-preview {
  transform: scale(1.1);
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.2);
}

.skin-check {
  color: white;
  font-size: 16px;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
}

.skin-item span {
  font-size: 11px;
  color: #64748b;
  text-align: center;
  line-height: 1.2;
}

.skin-item small {
  font-size: 9px;
  color: var(--pet-primary, #ff6b6b);
  line-height: 1;
}

.skin-item.active span {
  color: var(--pet-primary, #ff6b6b);
  font-weight: 600;
}

/* ===== 字体颜色网格 ===== */
.font-color-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
}

.font-color-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 6px;
  border: 2px solid transparent;
  border-radius: 9px;
  cursor: pointer;
  transition: all 0.2s;
}

.font-color-item:hover {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.25);
  background: rgba(255, 255, 255, 0.64);
}

.font-color-item.active {
  border-color: var(--pet-primary, #ff6b6b);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.08);
}

.font-color-preview {
  width: 36px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 16px;
  font-weight: 700;
  background: rgba(255, 255, 255, 0.62);
  border-radius: 8px;
  border: 1px solid rgba(15, 23, 42, 0.06);
}

.font-color-item span {
  font-size: 10px;
  color: #999;
}

.font-color-item.active span {
  color: var(--pet-primary, #ff6b6b);
  font-weight: 600;
}

/* ===== 用户头像 ===== */
.avatar-setting {
  display: grid;
  grid-template-columns: 54px 1fr;
  gap: 10px;
  align-items: center;
}

.avatar-preview {
  width: 54px;
  height: 54px;
  border-radius: 50%;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 180% 180%;
  box-shadow:
    0 8px 18px rgba(15, 23, 42, 0.14),
    inset 0 1px 0 rgba(255, 255, 255, 0.28);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
  font-size: 22px;
  animation: softGradientShift 8s ease-in-out infinite;
}

.avatar-preview img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.avatar-actions {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.avatar-btn {
  min-height: 32px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 9px;
  background: rgba(255, 255, 255, 0.62);
  color: var(--pet-font-color, #475569);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.2s;
}

.avatar-btn:hover:not(:disabled) {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.32);
  background: rgba(255, 255, 255, 0.84);
  transform: translateY(-1px);
}

.avatar-btn.primary {
  background: var(--pet-primary, #ff6b6b);
  color: white;
  border-color: transparent;
}

.avatar-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.avatar-input {
  display: none;
}

/* ===== 系统状态 ===== */
.stat-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  margin-bottom: 8px;
}

.stat-label {
  width: 30px;
  color: #64748b;
  font-weight: 500;
}

/* ===== 对话背景 ===== */
.bg-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
}

.bg-opt {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 6px 4px;
  border: 2px solid transparent;
  border-radius: 10px;
  background: rgba(241, 245, 249, 0.6);
  cursor: pointer;
  transition: all 0.2s;
  font-size: 11px;
  color: #64748b;
}

.bg-opt:hover {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.25);
  background: rgba(255, 255, 255, 0.9);
  transform: translateY(-1px);
}

.bg-opt.active {
  border-color: var(--pet-primary, #ff6b6b);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.08);
}

.bg-opt.active span {
  color: var(--pet-primary, #ff6b6b);
  font-weight: 600;
}

.bg-opt.danger:hover {
  border-color: rgba(239, 68, 68, 0.3);
  color: #ef4444;
}

.bg-opt-preview {
  width: 38px;
  height: 38px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 16px;
  overflow: hidden;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4);
  transition: transform 0.2s;
}

.bg-opt:hover .bg-opt-preview {
  transform: scale(1.08);
}

.bg-prev-none {
  background: rgba(241, 245, 249, 0.8);
  color: #94a3b8;
  font-size: 14px;
}

.bg-prev-cute {
  background: linear-gradient(135deg, #fce4ec, #f8bbd0);
}

.bg-prev-scifi {
  background: linear-gradient(135deg, #0f172a, #1e3a5f);
  color: #38bdf8;
  box-shadow: inset 0 0 8px rgba(56, 189, 248, 0.3);
}

.bg-prev-minimal {
  background: #f1f5f9;
  color: #94a3b8;
  background-image: radial-gradient(circle, #cbd5e1 1px, transparent 1px);
  background-size: 8px 8px;
}

.bg-prev-custom {
  background: linear-gradient(135deg, #e0e7ff, #c7d2fe);
}

.bg-prev-custom img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: 8px;
}

.stat-value {
  width: 45px;
  text-align: right;
  color: #475569;
  font-variant-numeric: tabular-nums;
}

.stat-bar {
  flex: 1;
  height: 7px;
  background: rgba(15, 23, 42, 0.06);
  border-radius: 999px;
  overflow: hidden;
}

.stat-fill {
  height: 100%;
  border-radius: 999px;
  transition: width 0.5s ease;
}

@keyframes softGradientShift {
  0%, 100% {
    background-position: 0% 50%;
  }
  50% {
    background-position: 100% 50%;
  }
}

/* ===== Tabs ===== */
.settings-tabs {
  display: flex;
  background: rgba(15, 23, 42, 0.03);
  border-bottom: 1px solid rgba(15, 23, 42, 0.06);
  padding: 4px;
  gap: 4px;
  flex-shrink: 0;
}

.settings-tabs .tab-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px;
  border: none;
  background: transparent;
  color: #64748b;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  border-radius: 8px;
  transition: all 0.2s;
}

.settings-tabs .tab-btn:hover {
  background: rgba(15, 23, 42, 0.04);
  color: #334155;
}

.settings-tabs .tab-btn.active {
  background: white;
  color: var(--pet-primary, #ff6b6b);
  box-shadow: 0 2px 8px rgba(15, 23, 42, 0.08);
}

/* 直连 API 设置样式 */
.backend-selector {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}

.backend-btn {
  flex: 1;
  padding: 8px 12px;
  background: rgba(15, 23, 42, 0.04);
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  font-size: 12px;
  color: #475569;
  cursor: pointer;
  transition: all 0.2s;
  font-weight: 500;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
}

.backend-btn:hover {
  background: rgba(15, 23, 42, 0.08);
}

.backend-btn.active {
  background: var(--pet-primary, #ff6b6b);
  color: white;
  border-color: transparent;
  box-shadow: 0 4px 12px rgba(255, 107, 107, 0.2);
}

.profile-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin: 2px 0 10px;
}

.profile-toolbar-title {
  color: #475569;
  font-size: 12px;
  font-weight: 700;
}

.profile-toolbar-subtitle {
  margin-top: 2px;
  color: #94a3b8;
  font-size: 10px;
  line-height: 1.35;
}

.profile-list {
  display: grid;
  gap: 8px;
  margin-bottom: 12px;
}

.profile-item {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 28px;
  align-items: center;
  gap: 7px;
  padding: 8px;
  border: 1px solid rgba(15, 23, 42, 0.07);
  border-radius: 9px;
  background: rgba(255, 255, 255, 0.56);
}

.profile-item.active {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.32);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.07);
}

.profile-main {
  min-width: 0;
  border: none;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.profile-name {
  overflow: hidden;
  color: #475569;
  font-size: 12px;
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.profile-meta {
  overflow: hidden;
  color: #94a3b8;
  font-size: 10px;
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.profile-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 3px;
}

.profile-badges span {
  min-height: 20px;
  display: inline-flex;
  align-items: center;
  padding: 2px 7px;
  border-radius: 999px;
  background: rgba(15, 23, 42, 0.05);
  color: #64748b;
  font-size: 10px;
  font-weight: 700;
}

.profile-delete-btn,
.mini-action-btn {
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.72);
  color: #64748b;
  cursor: pointer;
  transition: all 0.2s;
}

.profile-delete-btn {
  width: 28px;
  height: 28px;
  font-size: 16px;
  line-height: 1;
}

.mini-action-btn {
  min-height: 30px;
  padding: 6px 9px;
  font-size: 11px;
  font-weight: 700;
  white-space: nowrap;
}

.profile-delete-btn:hover:not(:disabled),
.mini-action-btn:hover:not(:disabled) {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.28);
  background: white;
  color: var(--pet-primary, #ff6b6b);
  transform: translateY(-1px);
}

.profile-delete-btn:disabled,
.mini-action-btn:disabled,
.action-btn:disabled {
  cursor: not-allowed;
  opacity: 0.52;
  transform: none;
}

.inline-control {
  display: flex;
  align-items: center;
  gap: 8px;
}

.inline-control input {
  min-width: 0;
  flex: 1;
}

.model-picker {
  display: grid;
  gap: 8px;
  padding: 10px;
  border: 1px solid rgba(56, 189, 248, 0.18);
  border-radius: 9px;
  background: rgba(240, 249, 255, 0.58);
}

.model-picker-row {
  display: grid;
  grid-template-columns: 66px minmax(0, 1fr);
  align-items: center;
  gap: 8px;
}

.model-picker-row span {
  color: #64748b;
  font-size: 11px;
  font-weight: 700;
}

.thinking-switch {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 5px;
  padding: 4px;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 9px;
  background: rgba(15, 23, 42, 0.04);
}

.thinking-option {
  min-height: 32px;
  border: 1px solid transparent;
  border-radius: 7px;
  background: transparent;
  color: #64748b;
  cursor: pointer;
  font-size: 12px;
  font-weight: 700;
  transition: all 0.2s;
}

.thinking-option:hover {
  background: rgba(255, 255, 255, 0.72);
  color: #475569;
}

.thinking-option.active {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.32);
  background: white;
  color: var(--pet-primary, #ff6b6b);
  box-shadow: 0 3px 10px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.11);
}

.api-form {
  margin-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.form-group label {
  font-size: 11px;
  color: #64748b;
  font-weight: 600;
}

.form-group input,
.form-group select {
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid rgba(15, 23, 42, 0.12);
  background: rgba(255, 255, 255, 0.7);
  font-size: 12px;
  color: #334155;
  transition: all 0.2s;
  outline: none;
}

.form-group input:focus,
.form-group select:focus {
  border-color: var(--pet-primary, #ff6b6b);
  background: white;
  box-shadow: 0 0 0 2px rgba(255, 107, 107, 0.12);
}

.toggle-group {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 0;
}

.toggle-group label {
  font-size: 12px;
  color: #475569;
  font-weight: 500;
}

.toggle-input {
  width: 16px;
  height: 16px;
  accent-color: var(--pet-primary, #ff6b6b);
  cursor: pointer;
}

.presets-container {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 4px;
}

.preset-badge {
  padding: 4px 8px;
  background: rgba(15, 23, 42, 0.04);
  border: 1px solid rgba(15, 23, 42, 0.06);
  border-radius: 6px;
  font-size: 10.5px;
  color: #64748b;
  cursor: pointer;
  transition: all 0.2s;
}

.preset-badge:hover {
  background: rgba(15, 23, 42, 0.08);
  color: #334155;
  transform: translateY(-1px);
}

.api-actions {
  display: flex;
  gap: 8px;
  margin-top: 6px;
}

.api-actions .action-btn {
  flex: 1;
}

.test-btn {
  background: rgba(15, 23, 42, 0.06);
  color: #475569;
  box-shadow: none;
  border: 1px solid rgba(15, 23, 42, 0.08);
}

.test-btn:hover:not(:disabled) {
  background: rgba(15, 23, 42, 0.1);
}

.test-result {
  font-size: 11px;
  padding: 6px 10px;
  border-radius: 8px;
  line-height: 1.4;
  margin-top: 4px;
}

.test-result.compact,
.hint.compact {
  margin-top: 2px;
  margin-bottom: 0;
}

.test-result.success {
  background: rgba(34, 197, 94, 0.1);
  color: #15803d;
  border: 1px solid rgba(34, 197, 94, 0.15);
}

.test-result.error {
  background: rgba(239, 68, 68, 0.1);
  color: #b91c1c;
  border: 1px solid rgba(239, 68, 68, 0.15);
  word-break: break-all;
}
</style>
