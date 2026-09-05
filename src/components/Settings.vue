<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import CustomPixelPetWorkshop from "./CustomPixelPetWorkshop.vue";
import PetCanvas from "./PetCanvas.vue";
import AppIcon from "./AppIcon.vue";
import { usePetStore, THEMES, FONT_COLORS, resolveSkinId } from "../stores/pet";
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
import { useSkillsStore, type Skill } from "../stores/skills";
import { AVAILABLE_TOOL_NAMES, toolLabel } from "../services/tools";

const emit = defineEmits<{ close: [] }>();
const pet = usePetStore();
const skillsStore = useSkillsStore();
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
  stream_mode: string;
  api_mode: string;
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

// 皮肤
const currentSkin = ref("default");
const themes = Object.values(THEMES);
const petCharacters = PET_CHARACTERS;
const currentCharacter = ref<PetCharacterId>("classic");

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
const voiceErrorMsg = ref("");

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
    stream_mode: config.stream_mode || "auto",
    api_mode: config.api_mode || "chat_completions",
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

async function saveStreamMode() {
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    console.error("流式模式保存失败:", e);
  }
}

async function saveApiMode() {
  try {
    await saveApiConfig({ quiet: true });
  } catch (e) {
    console.error("API 模式保存失败:", e);
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
    stream_mode: "auto",
    api_mode: "chat_completions",
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

// ===== 自定义系统提示 + 技能 =====

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

function newSkillDraft(): Skill {
  const id =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `sk_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
  return {
    id,
    name: "",
    description: "",
    system_prompt: "",
    allowed_tools: [],
    keywords: [],
    is_active: false,
    created_at: Math.floor(Date.now() / 1000),
    updated_at: Math.floor(Date.now() / 1000),
  };
}

const editingSkill = ref<Skill | null>(null);
const skillToolFilter = ref("");

const filteredSkillTools = computed(() => {
  const q = skillToolFilter.value.trim().toLowerCase();
  if (!q) return [...AVAILABLE_TOOL_NAMES];
  return AVAILABLE_TOOL_NAMES.filter((t) => t.toLowerCase().includes(q) || toolLabel(t).toLowerCase().includes(q));
});

function startNewSkill() {
  editingSkill.value = newSkillDraft();
  skillToolFilter.value = "";
}

function startEditSkill(skill: Skill) {
  editingSkill.value = { ...skill };
  skillToolFilter.value = "";
}

function cancelEditSkill() {
  editingSkill.value = null;
  skillToolFilter.value = "";
}

function toggleSkillTool(name: string) {
  if (!editingSkill.value) return;
  const set = new Set(editingSkill.value.allowed_tools);
  if (set.has(name)) set.delete(name);
  else set.add(name);
  editingSkill.value.allowed_tools = AVAILABLE_TOOL_NAMES.filter((t) => set.has(t));
}

function isSkillToolChecked(name: string): boolean {
  return !!editingSkill.value?.allowed_tools.includes(name);
}

async function saveEditingSkill() {
  if (!editingSkill.value) return;
  if (!editingSkill.value.name.trim()) {
    alert("请填写技能名称");
    return;
  }
  const updated: Skill = {
    ...editingSkill.value,
    keywords: editingSkill.value.keywords ?? [],
    updated_at: Math.floor(Date.now() / 1000),
  };
  try {
    await skillsStore.upsert(updated);
    editingSkill.value = null;
  } catch (e) {
    alert("保存技能失败: " + e);
  }
}

async function removeSkill(id: string) {
  if (!confirm("确认删除该技能？")) return;
  try {
    await skillsStore.remove(id);
    if (editingSkill.value?.id === id) cancelEditSkill();
  } catch (e) {
    alert("删除技能失败: " + e);
  }
}

async function restoreBuiltinSkills() {
  if (!confirm("将恢复所有被删除的内置技能，自定义技能不受影响。继续？")) return;
  try {
    await skillsStore.resetBuiltin();
  } catch (e) {
    alert("恢复内置技能失败: " + e);
  }
}

async function activateSkill(id: string | null) {
  try {
    await skillsStore.setActive(id);
  } catch (e) {
    alert("切换激活技能失败: " + e);
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
  } catch (e) {
    alert("本体形象切换失败: " + e);
  }
}

function onCustomPixelSelected() {
  currentCharacter.value = pet.character;
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

function chooseBgImage() {
  bgInputRef.value?.click();
}

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
    window.dispatchEvent(new CustomEvent("tts-settings-changed"));
    await currentWindow.emit("tts-settings-changed");
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
      voiceErrorMsg.value = "试听失败: " + (e.message || e);
      setTimeout(() => { voiceErrorMsg.value = ""; }, 5000);
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
    voiceErrorMsg.value = "语音设置保存失败: " + (e instanceof Error ? e.message : String(e));
    setTimeout(() => { voiceErrorMsg.value = ""; }, 5000);
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
  loadCurrentCharacter();
  loadCurrentFontColor();
  loadUserAvatar();
  loadPersonality();
  loadProfession();
  loadVoiceSettings();
  loadBackendSettings();
  skillsStore.load();

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
      <span class="settings-header-title"><AppIcon name="system" :size="14" /> 设置</span>
      <button class="close-btn" @click="emit('close')">
        <AppIcon name="x" :size="13" />
      </button>
    </div>

    <!-- Tab navigation -->
    <div class="settings-tabs">
      <button :class="['tab-btn', { active: activeTab === 'appearance' }]" @click="activeTab = 'appearance'">
        <AppIcon name="appearance" :size="13" /><span>外观</span>
      </button>
      <button :class="['tab-btn', { active: activeTab === 'voice' }]" @click="activeTab = 'voice'">
        <AppIcon name="voice" :size="13" /><span>语音</span>
      </button>
      <button :class="['tab-btn', { active: activeTab === 'system' }]" @click="activeTab = 'system'">
        <AppIcon name="system" :size="13" /><span>系统</span>
      </button>
    </div>

    <div class="settings-body">
      <!-- APPEARANCE TAB -->
      <template v-if="activeTab === 'appearance'">
        <div class="section">
          <h3><AppIcon name="paw" :size="13" /> 本体形象</h3>
          <div class="character-grid">
            <button
              v-for="character in petCharacters"
              :key="character.id"
              class="character-item"
              :class="{ active: currentCharacter === character.id }"
              @click="selectCharacter(character.id)"
            >
              <div class="character-preview">
                <PetCanvas
                  v-if="character.id === 'classic' || character.id === 'custom-pixel'"
                  preview
                  :character="character.id"
                  style="width: 72px; height: 82px; pointer-events: none;"
                />
                <img v-else src="../assets/pets/daimao-batiao/stills/still-03.png" alt="" />
              </div>
              <span>{{ character.name }}</span>
              <small>{{ character.description }}</small>
            </button>
          </div>
        </div>

        <div class="section">
          <h3><AppIcon name="image" :size="13" /> 自定义像素桌宠</h3>
          <CustomPixelPetWorkshop @selected="onCustomPixelSelected" />
        </div>

        <!-- 主题皮肤 -->
        <div class="section">
          <h3><AppIcon name="appearance" :size="13" /> 主题皮肤</h3>
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
          <h3><span class="h3-serif">Aa</span> 字体颜色</h3>
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
          <h3><AppIcon name="image" :size="13" /> 对话背景</h3>
          <div class="bg-grid">
            <button :class="['bg-opt', { active: chatBg === 'none' }]" @click="selectBg('none')">
              <div class="bg-opt-preview bg-prev-none"><AppIcon name="minus" :size="15" /></div>
              <span>无</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'cute' }]" @click="selectBg('cute')">
              <div class="bg-opt-preview bg-prev-cute"><AppIcon name="paw" :size="15" /></div>
              <span>可爱</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'scifi' }]" @click="selectBg('scifi')">
              <div class="bg-opt-preview bg-prev-scifi"><AppIcon name="zap" :size="15" /></div>
              <span>科幻</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'minimal' }]" @click="selectBg('minimal')">
              <div class="bg-opt-preview bg-prev-minimal"><AppIcon name="check" :size="15" /></div>
              <span>简洁</span>
            </button>
            <button :class="['bg-opt', { active: chatBg === 'custom' }]" @click="chooseBgImage">
              <div class="bg-opt-preview bg-prev-custom">
                <img v-if="customBgImage" :src="customBgImage" alt="" />
                <AppIcon v-else name="image" :size="15" />
              </div>
              <span>自定义</span>
            </button>
            <button v-if="chatBg === 'custom' && customBgImage" class="bg-opt danger" @click="clearCustomBg">
              <div class="bg-opt-preview bg-prev-none"><AppIcon name="trash" :size="15" /></div>
              <span>清除</span>
            </button>
          </div>
          <input ref="bgInputRef" class="avatar-input" type="file" accept="image/*" @change="onBgImageSelected" />
        </div>

        <!-- 用户头像 -->
        <div class="section">
          <h3><AppIcon name="user" :size="13" /> 用户头像</h3>
          <div class="avatar-setting">
            <div class="avatar-preview">
              <img v-if="pet.userAvatar" :src="pet.userAvatar" alt="用户头像" />
              <AppIcon v-else name="user" :size="22" />
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

        <!-- 性格 -->
        <div class="section">
          <h3><AppIcon name="smile" :size="13" /> 性格</h3>
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
          <h3><AppIcon name="file" :size="13" /> 职业</h3>
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
              <div v-if="voiceErrorMsg" class="voice-error-msg">{{ voiceErrorMsg }}</div>
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
        <!-- 自定义系统提示 -->
        <div class="section">
          <h3><AppIcon name="edit" :size="13" /> 自定义系统提示</h3>
          <p class="hint">
            追加到系统提示末尾，对所有对话与两个后端生效。留空则不追加。
          </p>
          <textarea
            class="settings-textarea"
            rows="4"
            v-model="customPromptDraft"
            placeholder="例如：你每次回复都以『汪！』开头，并且使用简洁的短句。"
          />
          <div class="action-row">
            <button class="action-btn" @click="saveCustomPrompt">
              {{ customPromptSaved ? "已保存" : "保存" }}
            </button>
            <span v-if="customPromptDraft.trim().length === 0" class="muted-hint">
              （留空将不追加任何内容）
            </span>
          </div>
        </div>

        <!-- 技能 -->
        <div class="section">
          <h3><AppIcon name="cpu" :size="13" /> 技能</h3>
          <p class="hint">
            把一组“提示追加 + 免确认工具 + 触发关键词”打包成可复用技能。
            手动激活后会持续生效；未激活时，用户消息含关键词会自动激活本轮。
            技能不影响可用工具列表，只影响是否需要用户确认。
          </p>

          <div class="action-row">
            <button class="action-btn primary" @click="startNewSkill">＋ 新建技能</button>
            <button class="action-btn" @click="restoreBuiltinSkills">
              恢复内置技能
            </button>
            <span v-if="skillsStore.activeSkill" class="muted-hint">
              当前激活：<b>{{ skillsStore.activeSkill.name }}</b>
            </span>
            <button
              v-else-if="skillsStore.skills.length > 0"
              class="action-btn"
              @click="activateSkill(null)"
            >
              清除激活
            </button>
          </div>

          <div v-if="skillsStore.skills.length === 0" class="empty-hint">
            还没有任何技能，点击“新建技能”开始。
          </div>

          <div v-else class="skill-list">
            <div
              v-for="s in skillsStore.skills"
              :key="s.id"
              class="skill-card"
              :class="{ active: s.is_active }"
            >
              <div class="skill-card-main">
                <div class="skill-card-header">
                  <span class="skill-card-name">{{ s.name || "(未命名)" }}</span>
                  <span v-if="s.is_active" class="skill-active-badge">已激活</span>
                </div>
                <div v-if="s.description" class="skill-card-desc">{{ s.description }}</div>
                <div class="skill-card-meta">
                  <span>工具 {{ s.allowed_tools.length }} 个</span>
                  <span v-if="s.system_prompt.trim()">提示追加 ✓</span>
                </div>
              </div>
              <div class="skill-card-actions">
                <button class="mini-action-btn" @click="startEditSkill(s)">编辑</button>
                <button
                  v-if="!s.is_active"
                  class="mini-action-btn"
                  @click="activateSkill(s.id)"
                >
                  激活
                </button>
                <button
                  v-else
                  class="mini-action-btn"
                  @click="activateSkill(null)"
                >
                  取消激活
                </button>
                <button class="mini-action-btn danger" @click="removeSkill(s.id)">删除</button>
              </div>
            </div>
          </div>

          <!-- 编辑面板 -->
          <div v-if="editingSkill" class="skill-editor">
            <h4>编辑技能</h4>
            <label class="form-label">
              名称 *
              <input
                class="settings-input"
                v-model="editingSkill.name"
                placeholder="例如：翻译官"
              />
            </label>
            <label class="form-label">
              描述
              <input
                class="settings-input"
                v-model="editingSkill.description"
                placeholder="一句话说明这个技能做什么"
              />
            </label>
            <label class="form-label">
              系统提示追加
              <textarea
                class="settings-textarea"
                rows="4"
                v-model="editingSkill.system_prompt"
                placeholder="留空表示只影响工具确认策略，不追加提示。"
              />
              <span v-if="!editingSkill.system_prompt.trim()" class="muted-hint">
                （无提示追加）
              </span>
            </label>
            <div class="form-label">
              允许免确认使用的工具
              <input
                class="settings-input"
                v-model="skillToolFilter"
                placeholder="搜索工具..."
              />
            </div>
            <div class="tool-grid">
              <label
                v-for="t in filteredSkillTools"
                :key="t"
                class="tool-row"
              >
                <input
                  type="checkbox"
                  :checked="isSkillToolChecked(t)"
                  @change="toggleSkillTool(t)"
                />
                <span class="tool-row-label">{{ toolLabel(t) }}</span>
                <span class="tool-row-name">{{ t }}</span>
              </label>
              <div v-if="filteredSkillTools.length === 0" class="muted-hint">
                没有匹配的工具
              </div>
            </div>
            <div class="action-row">
              <button class="action-btn primary" @click="saveEditingSkill">保存技能</button>
              <button class="action-btn" @click="cancelEditSkill">取消</button>
            </div>
          </div>
        </div>

        <!-- 系统状态 -->
        <div class="section">
          <h3><AppIcon name="system" :size="13" /> 系统状态</h3>
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
          <h3><AppIcon name="terminal" :size="13" /> AI 后端与模型</h3>
          
          <div class="backend-selector">
            <button
              class="backend-btn"
              :class="{ active: backendType === 'claude_code' }"
              @click="selectBackend('claude_code')"
            >
              <AppIcon name="terminal" :size="14" /> Claude Code
            </button>
            <button
              class="backend-btn"
              :class="{ active: backendType === 'direct_api' }"
              @click="selectBackend('direct_api')"
            >
              <AppIcon name="globe" :size="14" /> 直连 API Agent
            </button>
          </div>

          <!-- Claude Code 模式 -->
          <template v-if="backendType === 'claude_code'">
            <p class="current-model">{{ currentModel || '加载中...' }}</p>
            <p class="hint">使用本地 Claude Code CLI 子进程，会话自动保持上下文。</p>
            <div class="model-actions">
              <button class="action-btn" @click="resetModel">
                {{ modelSaved ? "已重置" : "重置会话" }}
              </button>
              <button class="action-btn config-btn" @click="startClaudeConfig">
                配置/登录 Claude Code
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
                <label>流式模式 (Stream Mode)</label>
                <select
                  v-model="apiConfig.stream_mode"
                  @change="saveStreamMode"
                >
                  <option value="auto">自动检测（推荐）</option>
                  <option value="stream">强制流式 (SSE)</option>
                  <option value="non_stream">非流式 (JSON)</option>
                </select>
                <p class="hint">
                  自动：先尝试SSE，失败则回退到JSON解析。Auto Code等非标准接口选"非流式"。
                </p>
              </div>

              <div class="form-group">
                <label>API 格式</label>
                <select
                  v-model="apiConfig.api_mode"
                  @change="saveApiMode"
                >
                  <option value="chat_completions">Chat Completions（传统兼容）</option>
                  <option value="responses">Responses（OpenAI 新格式）</option>
                </select>
                <p class="hint">
                  Chat Completions 是主流兼容格式，Responses 是 OpenAI 2025 年推出的新格式。大多数情况选 Chat Completions。
                </p>
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
                {{ testResult.success ? '连接成功！' : testResult.message }}
              </div>

              <div class="api-actions">
                <button 
                  class="action-btn test-btn" 
                  @click="testConnection" 
                  :disabled="isTestingConnection || !apiConfig.base_url || !apiConfig.model"
                >
                  {{ isTestingConnection ? '测试中...' : '测试连接' }}
                </button>
                <button 
                  class="action-btn" 
                  @click="saveApiConfig()"
                  :disabled="isSavingConfig"
                >
                  {{ configSaved ? '保存成功' : (isSavingConfig ? '保存中...' : '保存配置') }}
                </button>
              </div>

              <div class="api-actions" style="margin-top: 8px;">
                <button 
                  class="action-btn deep-test-btn" 
                  @click="testCompatibility" 
                  :disabled="isTestingCompatibility || !apiConfig.base_url || !apiConfig.model"
                >
                  {{ isTestingCompatibility ? '检测中...' : '深度测试兼容性' }}
                </button>
              </div>

              <!-- 深度测试结果 -->
              <div 
                v-if="compatibilityResult" 
                class="compatibility-result"
                :class="compatibilityResult.http_ok ? 'success' : 'error'"
              >
                <h4>兼容性检测报告</h4>
                <div class="result-grid">
                  <div class="result-item">
                    <span class="result-label">HTTP 状态:</span>
                    <span :class="compatibilityResult.http_ok ? 'pass' : 'fail'">
                      {{ compatibilityResult.http_ok ? '✓' : '✗' }} {{ compatibilityResult.http_status }}
                    </span>
                  </div>
                  <div class="result-item">
                    <span class="result-label">SSE 格式:</span>
                    <span :class="compatibilityResult.is_sse_format ? 'pass' : 'fail'">
                      {{ compatibilityResult.is_sse_format ? '✓ 支持' : '✗ 不支持' }}
                    </span>
                  </div>
                  <div class="result-item">
                    <span class="result-label">JSON 格式:</span>
                    <span :class="compatibilityResult.is_json_format ? 'pass' : 'fail'">
                      {{ compatibilityResult.is_json_format ? '✓ 支持' : '✗ 不支持' }}
                    </span>
                  </div>
                  <div class="result-item">
                    <span class="result-label">内容提取:</span>
                    <span :class="compatibilityResult.content_extracted ? 'pass' : 'fail'">
                      {{ compatibilityResult.content_extracted ? '✓ 成功' : '✗ 失败' }}
                    </span>
                  </div>
                  <div class="result-item" v-if="compatibilityResult.content_extracted">
                    <span class="result-label">提取内容:</span>
                    <span class="content-preview">{{ compatibilityResult.content_extracted }}</span>
                  </div>
                  <div class="result-item" v-if="compatibilityResult.recommended_mode">
                    <span class="result-label">推荐模式:</span>
                    <span class="recommendation">{{ compatibilityResult.recommended_mode === 'stream' ? '流式 (SSE)' : compatibilityResult.recommended_mode === 'non_stream' ? '非流式 (JSON)' : '自动' }}</span>
                  </div>
                  <div class="result-item" v-if="compatibilityResult.errors && compatibilityResult.errors.length > 0">
                    <span class="result-label">错误信息:</span>
                    <ul class="error-list">
                      <li v-for="(err, idx) in compatibilityResult.errors" :key="idx">{{ err }}</li>
                    </ul>
                  </div>
                </div>
                <details v-if="compatibilityResult.raw_preview" class="raw-preview">
                  <summary>查看原始响应 (前500字符)</summary>
                  <pre>{{ compatibilityResult.raw_preview }}</pre>
                </details>
                <p class="response-time">响应时间: {{ compatibilityResult.response_time_ms }} ms</p>
              </div>
            </div>
          </template>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* ============================================================
   Settings — 暖调工作室 · 设置面板
   ============================================================ */
.settings-mask {
  position: absolute;
  inset: 0;
  z-index: 0;
}

.settings-panel {
  position: absolute;
  inset: 0;
  z-index: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 16px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  background:
    radial-gradient(480px 220px at 20% -10%, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.06), transparent 60%),
    var(--dash-panel-solid, #fffefb);
  box-shadow:
    0 2px 6px rgba(48, 42, 34, 0.06),
    0 20px 48px rgba(48, 42, 34, 0.16);
  color: var(--dash-text-primary, #2d2922);
  font-family: var(--dash-font-sans, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif);
  font-size: 12.5px;
  line-height: 1.6;
}

.header-decor {
  display: none;
}

/* ---------- 头部 ---------- */
.settings-header {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px 10px;
}

.settings-header-title {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 13px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.settings-header-title .app-icon {
  color: var(--dash-accent, #bf7a4e);
}

.close-btn {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  cursor: pointer;
  transition: background 140ms ease-out, color 140ms ease-out;
}

.close-btn:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
}

/* ---------- 选项卡 ---------- */
.settings-tabs {
  flex-shrink: 0;
  display: flex;
  gap: 2px;
  margin: 0 14px 10px;
  padding: 2px;
  border-radius: 10px;
  background: var(--dash-panel-soft, #f3f0e9);
}

.tab-btn {
  flex: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 5.5px 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 11.5px;
  font-weight: 600;
  cursor: pointer;
  transition: background 140ms ease-out, color 140ms ease-out, box-shadow 140ms ease-out;
}

.tab-btn:hover {
  color: var(--dash-text-secondary, #6d6558);
}

.tab-btn.active {
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.06));
}

.tab-btn.active .app-icon {
  color: var(--dash-accent, #bf7a4e);
}

/* ---------- 主体 ---------- */
.settings-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 2px 14px 18px;
  scrollbar-width: thin;
  scrollbar-color: rgba(63, 54, 44, 0.18) transparent;
}

.settings-body::-webkit-scrollbar {
  width: 6px;
}

.settings-body::-webkit-scrollbar-thumb {
  background: rgba(63, 54, 44, 0.16);
  border-radius: 6px;
}

.settings-body::-webkit-scrollbar-track {
  background: transparent;
}

.section {
  margin-bottom: 18px;
  padding-top: 14px;
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.section:first-child {
  border-top: none;
  padding-top: 4px;
}

.section h3 {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.04em;
  color: var(--dash-text-primary, #2d2922);
  margin: 0 0 10px;
}

.section h3 .app-icon {
  color: var(--dash-accent, #bf7a4e);
}

.h3-serif {
  font-family: var(--dash-font-serif, Georgia, serif);
  font-style: italic;
  font-weight: 700;
  font-size: 12px;
  color: var(--dash-accent, #bf7a4e);
}

.hint {
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
  line-height: 1.65;
  margin: 0 0 10px;
}

.hint.compact {
  margin-top: 4px;
  margin-bottom: 0;
}

.muted-hint {
  font-size: 10.5px;
  color: var(--dash-text-muted, #a29a8a);
}

.empty-hint {
  padding: 18px 14px;
  border-radius: 10px;
  border: 1px dashed var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  color: var(--dash-text-muted, #a29a8a);
  font-size: 11.5px;
  text-align: center;
  line-height: 1.7;
}

/* ---------- 形象选择 ---------- */
.character-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
}

.character-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  padding: 10px 6px 9px;
  border-radius: 12px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  cursor: pointer;
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out, transform 140ms ease-out;
}

.character-item:hover {
  transform: translateY(-1px);
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.character-item.active {
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 1px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.25), 0 6px 18px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.12);
}

.character-preview {
  height: 84px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.character-preview img {
  max-width: 72px;
  max-height: 82px;
  object-fit: contain;
  border-radius: 8px;
}

.character-item > span {
  font-size: 11px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.character-item > small {
  font-size: 9.5px;
  color: var(--dash-text-muted, #a29a8a);
  text-align: center;
  line-height: 1.4;
}

/* ---------- 皮肤 ---------- */
.skin-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}

.skin-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  cursor: pointer;
}

.skin-preview {
  width: 100%;
  height: 46px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 140ms ease-out, box-shadow 140ms ease-out;
}

.skin-item:hover .skin-preview {
  transform: translateY(-1px);
  box-shadow: var(--dash-shadow-sm, 0 4px 14px rgba(48, 42, 34, 0.08));
}

.skin-item.active .skin-preview {
  box-shadow: 0 0 0 2px var(--dash-panel-solid, #fffefb), 0 0 0 4px var(--dash-accent, #bf7a4e);
}

.skin-check {
  width: 17px;
  height: 17px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.94);
  color: var(--dash-accent-dark, #a5643c);
  font-size: 10px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.skin-item > span {
  font-size: 10.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.skin-item.active > span {
  color: var(--dash-accent-dark, #a5643c);
}

.skin-item > small {
  font-size: 8.5px;
  color: var(--dash-text-muted, #a29a8a);
}

/* ---------- 字体颜色 ---------- */
.font-color-grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 6px;
}

.font-color-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  padding: 8px 4px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  cursor: pointer;
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out;
}

.font-color-item:hover {
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.font-color-item.active {
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 1px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.25);
}

.font-color-preview {
  font-size: 16px;
  font-weight: 700;
  font-family: var(--dash-font-serif, Georgia, serif);
}

.font-color-item > span {
  font-size: 9.5px;
  color: var(--dash-text-secondary, #6d6558);
}

/* ---------- 对话背景 ---------- */
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
  border: none;
  background: transparent;
  cursor: pointer;
  padding: 0;
}

.bg-opt-preview {
  width: 100%;
  height: 48px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-soft, #f3f0e9);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--dash-text-muted, #a29a8a);
  overflow: hidden;
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out, transform 140ms ease-out;
}

.bg-opt-preview img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.bg-opt:hover .bg-opt-preview {
  transform: translateY(-1px);
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.bg-opt.active .bg-opt-preview {
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 1px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.25), 0 6px 18px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.12);
}

.bg-opt > span {
  font-size: 10.5px;
  font-weight: 550;
  color: var(--dash-text-secondary, #6d6558);
}

.avatar-input {
  display: none;
}

/* ---------- 用户头像 ---------- */
.avatar-setting {
  display: flex;
  align-items: center;
  gap: 14px;
}

.avatar-preview {
  width: 54px;
  height: 54px;
  border-radius: 50%;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-soft, #f3f0e9);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--dash-text-muted, #a29a8a);
  overflow: hidden;
  flex-shrink: 0;
}

.avatar-preview img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.avatar-actions {
  display: flex;
  gap: 7px;
}

.avatar-btn {
  padding: 6.5px 14px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  font-size: 11.5px;
  font-weight: 600;
  cursor: pointer;
  transition: all 140ms ease-out;
}

.avatar-btn:hover:not(:disabled) {
  background: var(--dash-panel-soft, #f3f0e9);
}

.avatar-btn.primary {
  background: var(--dash-accent, #bf7a4e);
  border-color: transparent;
  color: #fffaf4;
}

.avatar-btn.primary:hover {
  background: var(--dash-accent-dark, #a5643c);
}

.avatar-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

/* ---------- 性格/职业选项 ---------- */
.option-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 7px;
}

.option-item {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 8px 11px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  text-align: left;
  cursor: pointer;
  transition: border-color 140ms ease-out, background 140ms ease-out;
}

.option-item:hover {
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.option-item.active {
  border-color: var(--dash-accent, #bf7a4e);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  box-shadow: 0 0 0 1px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.22);
}

.option-name {
  font-size: 11.5px;
  font-weight: 620;
  color: var(--dash-text-primary, #2d2922);
}

.option-desc {
  font-size: 10px;
  color: var(--dash-text-muted, #a29a8a);
}

/* ---------- 语音 ---------- */
.voice-options {
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.switch-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 9px 12px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-soft, #f3f0e9);
  cursor: pointer;
}

.switch-row > span {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.switch-row strong {
  font-size: 12px;
  font-weight: 620;
  color: var(--dash-text-primary, #2d2922);
}

.switch-row small {
  font-size: 10px;
  color: var(--dash-text-muted, #a29a8a);
}

.switch-row input[type="checkbox"],
.toggle-input {
  appearance: none;
  -webkit-appearance: none;
  width: 36px;
  height: 21px;
  border-radius: 999px;
  background: rgba(63, 54, 44, 0.16);
  position: relative;
  cursor: pointer;
  flex-shrink: 0;
  transition: background 200ms ease-out;
}

.switch-row input[type="checkbox"]::after,
.toggle-input::after {
  content: "";
  position: absolute;
  top: 2.5px;
  left: 2.5px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fffefb;
  box-shadow: 0 1px 3px rgba(48, 42, 34, 0.25);
  transition: transform 200ms cubic-bezier(0.34, 1.3, 0.64, 1);
}

.switch-row input[type="checkbox"]:checked,
.toggle-input:checked {
  background: var(--dash-accent, #bf7a4e);
}

.switch-row input[type="checkbox"]:checked::after,
.toggle-input:checked::after {
  transform: translateX(15px);
}

.field-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 2px;
  border-bottom: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.07));
}

.field-row > span {
  font-size: 12px;
  font-weight: 550;
  color: var(--dash-text-primary, #2d2922);
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.field-row > span small {
  font-size: 10px;
  font-weight: 450;
  color: var(--dash-text-muted, #a29a8a);
}

.field-row input[type="text"],
.settings-input,
.form-group input[type="text"],
.form-group input[type="password"] {
  padding: 7px 10px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  font-size: 12px;
  font-family: inherit;
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out;
}

.field-row input[type="text"]:focus,
.settings-input:focus,
.form-group input:focus,
.settings-textarea:focus,
.field-row select:focus,
.form-group select:focus,
.voice-search:focus {
  outline: none;
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
}

.field-row select,
.form-group select,
.model-picker-row select {
  padding: 7px 28px 7px 10px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb)
    url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' fill='none' stroke='%23a29a8a' stroke-width='1.6' stroke-linecap='round'/%3E%3C/svg%3E")
    no-repeat right 10px center;
  color: var(--dash-text-primary, #2d2922);
  font-size: 12px;
  font-family: inherit;
  appearance: none;
  cursor: pointer;
  max-width: 220px;
}

.settings-textarea {
  width: 100%;
  box-sizing: border-box;
  padding: 9px 11px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  font-size: 12px;
  font-family: inherit;
  line-height: 1.6;
  resize: vertical;
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out;
}

.voice-picker-tools {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 4px 0 8px;
}

.voice-search {
  flex: 1;
  padding: 7px 10px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  font-size: 12px;
  font-family: inherit;
}

.mini-check {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 10.5px;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  white-space: nowrap;
}

.mini-check input[type="checkbox"],
.tool-row input[type="checkbox"] {
  accent-color: var(--dash-accent, #bf7a4e);
  width: 13px;
  height: 13px;
}

.voice-empty {
  padding: 12px;
  border-radius: 9px;
  border: 1px dashed var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  color: var(--dash-text-muted, #a29a8a);
  font-size: 11px;
  text-align: center;
  margin: 6px 0;
}

.preview-row {
  margin-top: 10px;
}

.preview-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 16px;
  border: none;
  border-radius: 9px;
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  font-size: 12px;
  font-weight: 650;
  cursor: pointer;
  box-shadow: 0 2px 8px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28);
  transition: background 140ms ease-out;
}

.preview-btn:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a5643c);
}

.preview-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.voice-error-msg {
  margin-top: 8px;
  padding: 7px 11px;
  border-radius: 9px;
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  border: 1px solid rgba(192, 90, 77, 0.28);
  color: var(--dash-danger, #c05a4d);
  font-size: 11px;
}

.range-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 7px 2px;
}

.range-row > span {
  font-size: 11.5px;
  font-weight: 550;
  color: var(--dash-text-primary, #2d2922);
  display: flex;
  justify-content: space-between;
}

.range-row input[type="range"] {
  appearance: none;
  -webkit-appearance: none;
  width: 100%;
  height: 4px;
  border-radius: 999px;
  background: rgba(63, 54, 44, 0.13);
  cursor: pointer;
}

.range-row input[type="range"]::-webkit-slider-thumb {
  appearance: none;
  -webkit-appearance: none;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: var(--dash-panel-solid, #fffefb);
  border: 2px solid var(--dash-accent, #bf7a4e);
  box-shadow: 0 1px 4px rgba(48, 42, 34, 0.2);
}

/* ---------- 动作按钮 ---------- */
.action-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
  flex-wrap: wrap;
}

.action-btn,
.mini-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  font-weight: 600;
  cursor: pointer;
  transition: all 140ms ease-out;
}

.action-btn {
  padding: 7px 15px;
  font-size: 12px;
}

.mini-action-btn {
  padding: 4.5px 11px;
  font-size: 11px;
  border-radius: 8px;
}

.action-btn:hover:not(:disabled),
.mini-action-btn:hover:not(:disabled) {
  background: var(--dash-panel-soft, #f3f0e9);
  border-color: rgba(63, 54, 44, 0.24);
}

.action-btn.primary {
  background: var(--dash-accent, #bf7a4e);
  border-color: transparent;
  color: #fffaf4;
  box-shadow: 0 2px 8px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28);
}

.action-btn.primary:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a5643c);
}

.action-btn:disabled,
.mini-action-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.mini-action-btn.danger {
  color: var(--dash-danger, #c05a4d);
}

.mini-action-btn.danger:hover:not(:disabled) {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  border-color: rgba(192, 90, 77, 0.3);
}

/* ---------- 技能 ---------- */
.skill-list {
  display: flex;
  flex-direction: column;
  gap: 7px;
  margin-top: 10px;
}

.skill-card {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 11px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  transition: border-color 140ms ease-out;
}

.skill-card.active {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.4);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.05));
}

.skill-card-main {
  flex: 1;
  min-width: 0;
}

.skill-card-header {
  display: flex;
  align-items: center;
  gap: 7px;
}

.skill-card-name {
  font-size: 12px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.skill-active-badge {
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--dash-accent, #bf7a4e);
  color: #fff;
  font-size: 9px;
  font-weight: 700;
}

.skill-card-desc {
  margin-top: 2px;
  font-size: 10.5px;
  color: var(--dash-text-secondary, #6d6558);
  line-height: 1.55;
}

.skill-card-meta {
  margin-top: 3px;
  display: flex;
  gap: 8px;
  font-size: 9.5px;
  color: var(--dash-text-muted, #a29a8a);
}

.skill-card-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.skill-editor {
  margin-top: 12px;
  padding: 13px 14px;
  border-radius: 12px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-soft, #f3f0e9);
}

.skill-editor h4 {
  margin: 0 0 10px;
  font-size: 12px;
  font-weight: 700;
  color: var(--dash-text-primary, #2d2922);
}

.form-label {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-bottom: 10px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.tool-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 5px;
  max-height: 180px;
  overflow-y: auto;
  margin-bottom: 10px;
}

.tool-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border-radius: 8px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.08));
  background: var(--dash-panel-solid, #fffefb);
  cursor: pointer;
  font-size: 10.5px;
}

.tool-row-label {
  font-weight: 600;
  color: var(--dash-text-primary, #2d2922);
}

.tool-row-name {
  color: var(--dash-text-muted, #a29a8a);
  font-family: var(--dash-font-mono, Consolas, monospace);
  font-size: 9px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ---------- 系统状态 ---------- */
.stat-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 0;
}

.stat-label {
  width: 36px;
  flex-shrink: 0;
  font-size: 11px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.stat-bar {
  flex: 1;
  height: 5px;
  border-radius: 999px;
  background: rgba(63, 54, 44, 0.09);
  overflow: hidden;
}

.stat-fill {
  height: 100%;
  border-radius: 999px;
  transition: width 0.7s var(--dash-ease-out, ease-out);
}

.stat-value {
  width: 46px;
  text-align: right;
  font-size: 11px;
  font-weight: 650;
  font-family: var(--dash-font-mono, Consolas, monospace);
  color: var(--dash-text-primary, #2d2922);
}

/* ---------- 后端与模型 ---------- */
.backend-selector {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
  margin-bottom: 12px;
}

.backend-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  padding: 10px;
  border-radius: 11px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  font-size: 11.5px;
  font-weight: 620;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: all 140ms ease-out;
}

.backend-btn.active {
  border-color: var(--dash-accent, #bf7a4e);
  color: var(--dash-text-primary, #2d2922);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  box-shadow: 0 0 0 1px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.22);
}

.current-model {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
  padding: 7px 11px;
  border-radius: 9px;
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  border: 1px dashed rgba(var(--pet-primary-rgb, 191, 122, 78), 0.3);
  margin: 0 0 8px;
  word-break: break-all;
}

.model-actions {
  display: flex;
  gap: 8px;
  margin-top: 10px;
  flex-wrap: wrap;
}

.profile-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}

.profile-toolbar-title {
  font-size: 12px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.profile-toolbar-subtitle {
  font-size: 10px;
  color: var(--dash-text-muted, #a29a8a);
  margin-top: 1px;
}

.profile-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 12px;
}

.profile-item {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 10px;
  border-radius: 11px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out;
}

.profile-item.active {
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 1px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.22);
}

.profile-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border: none;
  background: transparent;
  text-align: left;
  cursor: pointer;
  padding: 0;
}

.profile-name {
  font-size: 12px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.profile-meta {
  font-size: 10px;
  font-family: var(--dash-font-mono, Consolas, monospace);
  color: var(--dash-text-muted, #a29a8a);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.profile-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 2px;
}

.profile-badges span {
  font-size: 9px;
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--dash-panel-soft, #f3f0e9);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.08));
  color: var(--dash-text-secondary, #6d6558);
}

.profile-delete-btn {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 13px;
  cursor: pointer;
  transition: all 140ms ease-out;
}

.profile-delete-btn:hover:not(:disabled) {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
}

.profile-delete-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.api-form {
  display: flex;
  flex-direction: column;
}

.form-group {
  margin-bottom: 11px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.form-group > label {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.inline-control {
  display: flex;
  gap: 7px;
  align-items: center;
}

.inline-control input {
  flex: 1;
}

.model-picker {
  margin: 4px 0 12px;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  border: 1px dashed rgba(var(--pet-primary-rgb, 191, 122, 78), 0.35);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.model-picker-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.thinking-switch {
  display: flex;
  gap: 5px;
  flex-wrap: wrap;
}

.thinking-option {
  padding: 5px 12px;
  border-radius: 999px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  font-size: 11px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: all 140ms ease-out;
}

.thinking-option:hover {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.4);
  color: var(--dash-accent-dark, #a5643c);
}

.thinking-option.active {
  background: var(--dash-accent, #bf7a4e);
  border-color: transparent;
  color: #fffaf4;
}

.presets-container {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.preset-badge {
  padding: 5px 12px;
  border-radius: 999px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  font-size: 11px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: all 140ms ease-out;
}

.preset-badge:hover {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.45);
  color: var(--dash-accent-dark, #a5643c);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
}

.toggle-group {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 9px 0;
}

.toggle-group > label {
  font-size: 12px;
  font-weight: 550;
  color: var(--dash-text-primary, #2d2922);
}

.test-result {
  margin: 10px 0;
  padding: 9px 12px;
  border-radius: 10px;
  font-size: 11.5px;
  line-height: 1.55;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-secondary, #6d6558);
}

.test-result.compact {
  margin: 6px 0;
  padding: 7px 10px;
}

.test-result.success {
  background: var(--dash-success-soft, rgba(94, 143, 106, 0.12));
  border-color: rgba(94, 143, 106, 0.3);
  color: #41684b;
}

.test-result.error {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  border-color: rgba(192, 90, 77, 0.28);
  color: var(--dash-danger, #c05a4d);
}

.api-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}

.api-actions .action-btn {
  flex: 1;
}

.deep-test-btn {
  width: 100%;
  justify-content: center;
  border-style: dashed;
  color: var(--dash-text-secondary, #6d6558);
}

/* ---------- 兼容性报告 ---------- */
.compatibility-result {
  margin-top: 12px;
  border-radius: 12px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-soft, #f3f0e9);
  padding: 12px 14px;
}

.compatibility-result.success {
  border-color: rgba(94, 143, 106, 0.35);
}

.compatibility-result.error {
  border-color: rgba(192, 90, 77, 0.35);
}

.compatibility-result h4 {
  margin: 0 0 9px;
  font-size: 12px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.result-grid {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.result-item {
  display: flex;
  gap: 9px;
  font-size: 11px;
  align-items: baseline;
}

.result-label {
  flex-shrink: 0;
  width: 72px;
  color: var(--dash-text-muted, #a29a8a);
}

.result-item .pass {
  color: var(--dash-success, #5e8f6a);
  font-weight: 600;
}

.result-item .fail {
  color: var(--dash-danger, #c05a4d);
  font-weight: 600;
}

.content-preview {
  font-family: var(--dash-font-mono, Consolas, monospace);
  font-size: 10px;
  color: var(--dash-text-secondary, #6d6558);
  word-break: break-all;
}

.recommendation {
  font-weight: 650;
  color: var(--dash-accent-dark, #a5643c);
}

.error-list {
  margin: 0;
  padding-left: 15px;
  color: var(--dash-danger, #c05a4d);
}

.raw-preview {
  margin-top: 9px;
}

.raw-preview summary {
  cursor: pointer;
  font-size: 10.5px;
  font-weight: 600;
  color: var(--dash-text-muted, #a29a8a);
}

.raw-preview pre {
  margin-top: 5px;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--dash-panel-sunken, #edeae2);
  font-size: 10px;
  font-family: var(--dash-font-mono, Consolas, monospace);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
}

.response-time {
  margin: 8px 0 0;
  font-size: 10px;
  color: var(--dash-text-muted, #a29a8a);
}
</style>
