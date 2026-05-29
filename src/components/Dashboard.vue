<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
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
type NavPage = "home" | "appearance" | "voice" | "system" | "about";
const activePage = ref<NavPage>("home");
const isPetActive = ref(false);

// === 系统信息 ===
const systemInfo = ref({ cpu: 0, memory: 0 });
let sysInfoTimer: ReturnType<typeof setInterval> | null = null;

// === 模型 ===
const currentModel = ref("");
const modelSaved = ref(false);

// === 后端 ===
const backendType = ref("claude_code");
const apiConfig = ref({
  api_key: "",
  base_url: "",
  model: "",
  confirm_enabled: true
});
const isTestingConnection = ref(false);
const testResult = ref({ success: false, message: "" });
const isSavingConfig = ref(false);
const configSaved = ref(false);

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
const memories = ref<Array<{ key: string; value: string }>>([]);

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
  try { memories.value = await invoke("get_memories", { category: "general" }); } catch {}
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
    apiConfig.value = {
      api_key: config.api_key || "",
      base_url: config.base_url || "https://api.openai.com/v1",
      model: config.model || "gpt-4o-mini",
      confirm_enabled: config.confirm_enabled !== false,
    };
    if (backendType.value === "direct_api") {
      currentModel.value = `直连 API: ${apiConfig.value.model}`;
    } else {
      await loadClaudeStatus();
    }
  } catch {}
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
  testResult.value = { success: false, message: "" };
}

async function saveApiConfig() {
  isSavingConfig.value = true;
  configSaved.value = false;
  try {
    await invoke("set_api_config", {
      apiKey: apiConfig.value.api_key,
      baseUrl: apiConfig.value.base_url,
      model: apiConfig.value.model,
      confirmEnabled: apiConfig.value.confirm_enabled,
    });
    configSaved.value = true;
    currentModel.value = `直连 API: ${apiConfig.value.model}`;
    setTimeout(() => { configSaved.value = false; }, 2000);
  } catch (e) { alert("保存失败: " + e); }
  finally { isSavingConfig.value = false; }
}

async function testConnection() {
  isTestingConnection.value = true;
  testResult.value = { success: false, message: "" };
  try {
    const res = await invoke<string>("test_api_connection", {
      apiKey: apiConfig.value.api_key,
      baseUrl: apiConfig.value.base_url,
      model: apiConfig.value.model,
    });
    testResult.value = { success: true, message: res };
  } catch (e: any) { testResult.value = { success: false, message: e.toString() }; }
  finally { isTestingConnection.value = false; }
}

async function resetModel() {
  try {
    await invoke("switch_model");
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
async function petAction(name: string) {
  const stateMap: Record<string, string> = {
    wave: "waving", happy: "happy", sleep: "sleeping", wake: "idle",
  };
  const state = stateMap[name];
  if (state) {
    try { await invoke("set_pet_state", { newState: state }); } catch {}
  }
  if (name === "clear") {
    try {
      await invoke("clear_chat_history");
      chat.clearMessages();
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

  // 检查桌宠窗口是否已存在
  const existing = await WebviewWindow.getByLabel("pet");
  isPetActive.value = !!existing;

  unlistenPetStatus = await listen("pet-window-closed", () => {
    isPetActive.value = false;
  });
});

onUnmounted(() => {
  if (sysInfoTimer) clearInterval(sysInfoTimer);
  unlistenPetStatus?.();
});
</script>

<template>
  <div class="dashboard-root">
    <!-- 自定义标题栏 -->
    <div class="dashboard-titlebar" data-tauri-drag-region>
      <div class="titlebar-title">
        <span class="titlebar-mark">AI</span>
        <span>Desktop Pet</span>
      </div>
      <div class="titlebar-controls">
        <button class="titlebar-btn" @click="minimizeWindow" title="最小化">
          <svg width="12" height="2" viewBox="0 0 12 2"><rect width="12" height="2" rx="1" fill="currentColor"/></svg>
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
      <div class="dashboard-content">
        <Transition name="page-fade" mode="out-in">
          <!-- ========== 首页 ========== -->
          <div v-if="activePage === 'home'" key="home">
            <div class="page-header home-header">
              <div>
                <div class="page-kicker">Control Center</div>
                <h1 class="page-title">桌宠控制台</h1>
                <p class="page-subtitle">状态、外观和声音集中管理</p>
              </div>
              <div :class="['status-pill', isPetActive ? 'live' : 'idle']">
                <span class="status-pill-dot" />
                <span>{{ isPetActive ? '桌宠在线' : '桌宠未启动' }}</span>
              </div>
            </div>

            <div class="home-grid">
              <!-- 宠物状态预览 -->
              <div class="dash-card pet-preview-card">
                <div class="pet-preview-visual">
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
                <div class="pet-card-actions">
                  <button
                    :class="['dash-btn', 'primary', { active: isPetActive }]"
                    @click="isPetActive ? emit('closePet') : emit('openPet'); isPetActive = !isPetActive"
                  >
                    {{ isPetActive ? '收回桌宠' : '召唤桌宠' }}
                  </button>
                </div>
              </div>

              <!-- 快捷操作 -->
              <div class="dash-card">
                <div class="dash-card-title">
                  <span class="card-icon">⌁</span> 快捷操作
                </div>
                <div class="quick-actions">
                  <button class="quick-action-btn" @click="petAction('wave')">
                    <span class="quick-action-icon">↗</span>
                    <span class="quick-action-label">打招呼</span>
                  </button>
                  <button class="quick-action-btn" @click="petAction('happy')">
                    <span class="quick-action-icon">◎</span>
                    <span class="quick-action-label">开心一下</span>
                  </button>
                  <button class="quick-action-btn" @click="petAction('sleep')">
                    <span class="quick-action-icon">Zz</span>
                    <span class="quick-action-label">去睡觉</span>
                  </button>
                  <button class="quick-action-btn" @click="petAction('wake')">
                    <span class="quick-action-icon">⏱</span>
                    <span class="quick-action-label">叫醒它</span>
                  </button>
                  <button class="quick-action-btn" @click="petAction('clear')">
                    <span class="quick-action-icon">⌫</span>
                    <span class="quick-action-label">清空对话</span>
                  </button>
                </div>
              </div>

              <!-- 系统监控 -->
              <div class="dash-card">
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

          <!-- ========== 外观 ========== -->
          <div v-else-if="activePage === 'appearance'" key="appearance">
            <div class="page-header">
              <div class="page-kicker">Appearance</div>
              <h1 class="page-title">个性外观</h1>
              <p class="page-subtitle">自定义桌宠的主题、字体和外观风格</p>
            </div>

            <!-- 主题皮肤 -->
            <div class="dash-card">
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
            <div class="dash-card">
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
            <div class="dash-card">
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
            <div class="dash-card">
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

          <!-- ========== 语音 ========== -->
          <div v-else-if="activePage === 'voice'" key="voice">
            <div class="page-header">
              <div class="page-kicker">Voice</div>
              <h1 class="page-title">语音设置</h1>
              <p class="page-subtitle">配置语音交互和文字转语音引擎</p>
            </div>

            <div class="dash-card">
              <div class="dash-card-title"><span class="card-icon">◌</span> 语音交互</div>

              <div class="dash-switch-row">
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

              <div class="dash-switch-row">
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

              <div class="dash-switch-row">
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

            <div class="dash-card">
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

                <div class="dash-range-row">
                  <div class="dash-range-header">
                    <span class="dash-range-label">语速</span>
                    <span class="dash-range-value">{{ ttsSettings.rate >= 0 ? '+' : '' }}{{ ttsSettings.rate }}%</span>
                  </div>
                  <input class="dash-range" type="range" min="-50" max="100" step="10" :value="ttsSettings.rate"
                    @input="updateTtsSettings({ rate: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row">
                  <div class="dash-range-header">
                    <span class="dash-range-label">音调</span>
                    <span class="dash-range-value">{{ ttsSettings.pitch >= 0 ? '+' : '' }}{{ ttsSettings.pitch }}Hz</span>
                  </div>
                  <input class="dash-range" type="range" min="-50" max="50" step="5" :value="ttsSettings.pitch"
                    @input="updateTtsSettings({ pitch: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row">
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
                <div class="dash-range-row">
                  <div class="dash-range-header">
                    <span class="dash-range-label">语速</span>
                    <span class="dash-range-value">{{ voiceSettings.rate.toFixed(1) }}</span>
                  </div>
                  <input class="dash-range" type="range" min="0.6" max="1.5" step="0.1" :value="voiceSettings.rate"
                    @input="updateVoiceSettings({ rate: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row">
                  <div class="dash-range-header">
                    <span class="dash-range-label">音调</span>
                    <span class="dash-range-value">{{ voiceSettings.pitch.toFixed(1) }}</span>
                  </div>
                  <input class="dash-range" type="range" min="0.6" max="1.6" step="0.1" :value="voiceSettings.pitch"
                    @input="updateVoiceSettings({ pitch: Number(($event.target as HTMLInputElement).value) })" />
                </div>
                <div class="dash-range-row">
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
          <div v-else-if="activePage === 'system'" key="system">
            <div class="page-header">
              <div class="page-kicker">System</div>
              <h1 class="page-title">系统设置</h1>
              <p class="page-subtitle">配置 AI 后端、性格和职业定位</p>
            </div>

            <!-- AI 后端 -->
            <div class="dash-card">
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
                <div class="dash-form-group">
                  <label>接口地址 (Base URL)</label>
                  <input type="text" v-model="apiConfig.base_url" placeholder="https://api.openai.com/v1" />
                </div>
                <div class="dash-form-group">
                  <label>API 密钥 (API Key)</label>
                  <input type="password" v-model="apiConfig.api_key" placeholder="sk-••••••••••••••••" />
                </div>
                <div class="dash-form-group">
                  <label>模型名称 (Model)</label>
                  <input type="text" v-model="apiConfig.model" placeholder="gpt-4o-mini" />
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
                  <input type="checkbox" class="dash-toggle" v-model="apiConfig.confirm_enabled" />
                </div>
                <p class="dash-hint" style="margin-top:-4px;">开启后，敏感工具（如命令执行、文件写入）在运行前需要您手动允许。</p>

                <div v-if="testResult.message" :class="['dash-test-result', testResult.success ? 'success' : 'error']">
                  {{ testResult.success ? '✅ 连接成功！' : '❌ ' + testResult.message }}
                </div>
                <div class="dash-api-actions">
                  <button class="dash-btn" @click="testConnection" :disabled="isTestingConnection || !apiConfig.api_key">
                    {{ isTestingConnection ? '测试中...' : '测试连接' }}
                  </button>
                  <button class="dash-btn primary" @click="saveApiConfig" :disabled="isSavingConfig">
                    {{ configSaved ? '保存成功' : (isSavingConfig ? '保存中...' : '保存配置') }}
                  </button>
                </div>
              </template>
            </div>

            <!-- 性格 -->
            <div class="dash-card">
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
            <div class="dash-card">
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

            <!-- 记忆 -->
            <div class="dash-card" v-if="memories.length > 0">
              <div class="dash-card-title"><span class="card-icon">◫</span> 宠物记忆</div>
              <div v-for="m in memories" :key="m.key" class="dash-memory-item">
                <strong>{{ m.key }}:</strong> {{ m.value }}
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
      </div>
    </div>
  </div>
</template>
