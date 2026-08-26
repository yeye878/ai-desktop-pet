import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { DEFAULT_VOICE_SETTINGS } from "./services/voice";
import { DEFAULT_TTS_SETTINGS } from "./services/tts";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      metadata?: unknown;
    };
  }
}

type MockPayload = Record<string, unknown> | undefined;
type MockMemory = {
  id: number;
  category: string;
  key: string;
  value: string;
  created_at: number;
};
type MockScheduledTask = {
  id: number;
  title: string;
  note: string;
  due_at: number;
  repeat: string;
  enabled: boolean;
  last_triggered_at: number | null;
  created_at: number;
  updated_at: number;
};
type MockApiProfile = {
  id: string;
  name: string;
  api_key: string;
  base_url: string;
  model: string;
  confirm_enabled: boolean;
  thinking_depth: string;
  execution_mode: string;
  search_provider: string;
  auto_approved_tools: string[];
};
type MockCustomPetAsset = {
  id: string;
  name: string;
  kind: string;
  manifest: string;
  sprite_path: string;
  preview_path: string;
  created_at: number;
  updated_at: number;
};
type MockAgent = {
  id: string;
  name: string;
  avatar: string;
  description: string;
  system_prompt: string;
  model: string;
  allowed_tools: string[];
  created_at: number;
  updated_at: number;
};
type MockWeatherConfig = {
  enabled: boolean;
  location: string;
  api_url: string;
};

const settingStore = new Map<string, string>([
  ["voice_settings", JSON.stringify(DEFAULT_VOICE_SETTINGS)],
  ["tts_settings", JSON.stringify(DEFAULT_TTS_SETTINGS)],
]);

let backendType = "claude_code";
let nextMemoryId = 3;
let nextScheduledTaskId = 2;
let nextCustomPetId = 1;
let customPetAssets: MockCustomPetAsset[] = [];
let mockAgents: MockAgent[] = [
  {
    id: "mock-agent-code",
    name: "代码侠",
    avatar: "🧑‍💻",
    description: "分析报错、写代码、给修改方案",
    system_prompt: "你是代码搭档。先理解项目结构和约束，再给出最小可行修改；排查问题优先读取相关文件，不要大范围重构。",
    model: "",
    allowed_tools: ["read_file", "list_directory"],
    created_at: 1,
    updated_at: 1,
  },
  {
    id: "mock-agent-writer",
    name: "文案师",
    avatar: "✍️",
    description: "润色文案、写周报、做摘要",
    system_prompt: "你是文案助手。保留原意和语气，给出直接可用的改写结果，必要时附简短说明。",
    model: "",
    allowed_tools: [],
    created_at: 2,
    updated_at: 2,
  },
];
let weatherConfig: MockWeatherConfig = {
  enabled: true,
  location: "Tokyo",
  api_url: "http://wttr.in",
};
const memories: MockMemory[] = [
  {
    id: 1,
    category: "manual",
    key: "偏好",
    value: "喜欢简洁、温暖、低噪声的桌面体验",
    created_at: 1,
  },
  {
    id: 2,
    category: "conversation",
    key: "对话摘要",
    value: "希望控制台像原生桌面软件一样可靠，并保留关键上下文。",
    created_at: 2,
  },
];
const scheduledTasks: MockScheduledTask[] = [
  {
    id: 1,
    title: "整理今日待办",
    note: "预览数据：打开任务面板时可以看到这条定时任务。",
    due_at: Math.floor(Date.now() / 1000) + 3600,
    repeat: "daily",
    enabled: true,
    last_triggered_at: null,
    created_at: Math.floor(Date.now() / 1000),
    updated_at: Math.floor(Date.now() / 1000),
  },
];
let activeApiProfileId = "mock-openai";
let apiProfiles: MockApiProfile[] = [
  {
    id: "mock-openai",
    name: "OpenAI 预览",
    api_key: "",
    base_url: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    confirm_enabled: true,
    thinking_depth: "auto",
    execution_mode: "normal",
    search_provider: "bing",
    auto_approved_tools: [],
  },
  {
    id: "mock-local",
    name: "Ollama 本地",
    api_key: "",
    base_url: "http://localhost:11434/v1",
    model: "qwen2.5:7b",
    confirm_enabled: true,
    thinking_depth: "low",
    execution_mode: "custom",
    search_provider: "bing",
    auto_approved_tools: ["read_file", "list_directory"],
  },
];

const edgeVoices = [
  { id: "zh-CN-XiaoxiaoNeural", name: "晓晓 - 中文普通话", language: "zh-CN", gender: "Female" },
  { id: "zh-CN-YunxiNeural", name: "云希 - 中文普通话", language: "zh-CN", gender: "Male" },
  { id: "en-US-JennyNeural", name: "Jenny - English US", language: "en-US", gender: "Female" },
  { id: "ja-JP-NanamiNeural", name: "Nanami - 日本語", language: "ja-JP", gender: "Female" },
];

function base64FromBytes(bytes: Uint8Array) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}

function createMockTtsAudio(text = "") {
  const sampleRate = 22050;
  const durationSeconds = 0.72;
  const samples = Math.floor(sampleRate * durationSeconds);
  const bytes = new Uint8Array(44 + samples * 2);
  const view = new DataView(bytes.buffer);
  const writeText = (offset: number, value: string) => {
    for (let i = 0; i < value.length; i++) bytes[offset + i] = value.charCodeAt(i);
  };

  writeText(0, "RIFF");
  view.setUint32(4, 36 + samples * 2, true);
  writeText(8, "WAVE");
  writeText(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeText(36, "data");
  view.setUint32(40, samples * 2, true);

  const frequency = 520 + Math.min(text.length, 80) * 2;
  for (let i = 0; i < samples; i++) {
    const t = i / sampleRate;
    const fadeIn = Math.min(1, i / (sampleRate * 0.04));
    const fadeOut = Math.min(1, (samples - i) / (sampleRate * 0.08));
    const envelope = Math.min(fadeIn, fadeOut);
    const wave = Math.sin(2 * Math.PI * frequency * t) * 0.28
      + Math.sin(2 * Math.PI * frequency * 1.5 * t) * 0.08;
    const sample = Math.max(-1, Math.min(1, wave * envelope));
    view.setInt16(44 + i * 2, sample * 32767, true);
  }

  return `data:audio/wav;base64,${base64FromBytes(bytes)}`;
}

function getMockSetting(args: MockPayload) {
  const key = String(args?.key || "");
  return settingStore.get(key) || "";
}

function setMockSetting(args: MockPayload) {
  const key = String(args?.key || "");
  const value = String(args?.value || "");
  if (key) settingStore.set(key, value);
  return null;
}

function readArg(args: MockPayload, camel: string, snake: string, fallback = "") {
  return String(args?.[camel] ?? args?.[snake] ?? fallback);
}

function getActiveApiProfile() {
  return apiProfiles.find((profile) => profile.id === activeApiProfileId) || apiProfiles[0];
}

function maskApiKey(key: string) {
  const value = key.trim();
  if (!value) return "";
  return `${value.slice(0, 4)}••••${value.slice(-4)}`;
}

function publicApiProfile(profile: MockApiProfile) {
  return {
    ...profile,
    api_key: "",
    api_key_mask: maskApiKey(profile.api_key),
    has_api_key: profile.api_key.trim().length > 0,
  };
}

function upsertMockApiProfile(args: MockPayload) {
  const model = readArg(args, "model", "model", "gpt-4o-mini");
  const hasProfileIdArg = Boolean(
    args && (
      Object.prototype.hasOwnProperty.call(args, "profileId")
      || Object.prototype.hasOwnProperty.call(args, "profile_id")
    ),
  );
  const requestedProfileId = readArg(args, "profileId", "profile_id", "");
  const existingProfile = requestedProfileId
    ? apiProfiles.find((item) => item.id === requestedProfileId)
    : getActiveApiProfile();
  const apiKey = readArg(args, "apiKey", "api_key");
  const profile: MockApiProfile = {
    id: hasProfileIdArg
      ? (requestedProfileId || `mock-${Date.now()}`)
      : (activeApiProfileId || `mock-${Date.now()}`),
    name: readArg(args, "profileName", "profile_name", model) || model,
    api_key: apiKey || existingProfile?.api_key || "",
    base_url: readArg(args, "baseUrl", "base_url", "https://api.openai.com/v1"),
    model,
    confirm_enabled: Boolean(args?.confirmEnabled ?? args?.confirm_enabled ?? true),
    thinking_depth: readArg(args, "thinkingDepth", "thinking_depth", "auto"),
    execution_mode: readArg(args, "executionMode", "execution_mode", getActiveApiProfile()?.execution_mode || "normal"),
    search_provider: readArg(args, "searchProvider", "search_provider", getActiveApiProfile()?.search_provider || "bing"),
    auto_approved_tools: Array.isArray(args?.autoApprovedTools)
      ? (args?.autoApprovedTools as string[])
      : Array.isArray(args?.auto_approved_tools)
        ? (args?.auto_approved_tools as string[])
        : getActiveApiProfile()?.auto_approved_tools || [],
  };
  const existing = apiProfiles.findIndex((item) => item.id === profile.id);
  if (existing >= 0) apiProfiles[existing] = profile;
  else apiProfiles = [profile, ...apiProfiles];
  activeApiProfileId = profile.id;
  return profile;
}

function mockWeatherInfo(location = weatherConfig.location) {
  return {
    city: location.trim() || "Tokyo",
    temperature: 23,
    feels_like: 25,
    humidity: 61,
    description: "Partly cloudy",
    wind_speed: 9,
    icon: "⛅",
    timestamp: Math.floor(Date.now() / 1000),
  };
}

function handleMockCommand(cmd: string, args: MockPayload) {
  if (cmd.startsWith("plugin:")) {
    if (cmd === "plugin:window|get_all_windows") return ["main"];
    if (cmd === "plugin:window|scale_factor") return 1;
    if (cmd === "plugin:window|outer_position" || cmd === "plugin:window|inner_position") return { x: 120, y: 80 };
    if (cmd === "plugin:window|outer_size" || cmd === "plugin:window|inner_size") return { width: 1180, height: 760 };
    if (cmd === "plugin:window|cursor_position") return { x: 200, y: 200 };
    if (cmd === "plugin:global-shortcut|is_registered") return false;
    return null;
  }

  switch (cmd) {
    case "get_system_info":
      return { cpu: 18.6, memory: 42.3 };
    case "get_weather_config":
      return weatherConfig;
    case "set_weather_config": {
      const apiUrl = readArg(args, "apiUrl", "api_url", "http://wttr.in").trim();
      if (!/^https?:\/\//i.test(apiUrl)) throw new Error("天气API地址必须以 http:// 或 https:// 开头");
      weatherConfig = {
        enabled: Boolean(args?.enabled),
        location: readArg(args, "location", "location").trim(),
        api_url: apiUrl.replace(/\/+$/, "") || "http://wttr.in",
      };
      return weatherConfig;
    }
    case "test_weather_config": {
      const apiUrl = readArg(args, "apiUrl", "api_url", weatherConfig.api_url).trim();
      if (!/^https?:\/\//i.test(apiUrl)) throw new Error("天气API地址必须以 http:// 或 https:// 开头");
      return mockWeatherInfo(readArg(args, "location", "location", weatherConfig.location));
    }
    case "get_weather":
    case "refresh_weather":
      return weatherConfig.enabled ? mockWeatherInfo() : null;
    case "check_and_send_weather":
      return weatherConfig.enabled;
    case "get_current_model":
      return "Claude Code / preview";
    case "get_backend_type":
      return backendType;
    case "set_backend_type":
      backendType = readArg(args, "backend", "backend", "claude_code");
      return null;
    case "get_api_config":
      return getActiveApiProfile();
    case "set_api_config":
      return upsertMockApiProfile(args);
    case "list_api_profiles":
      return { active_id: activeApiProfileId, profiles: apiProfiles.map(publicApiProfile) };
    case "set_active_api_profile": {
      const id = readArg(args, "id", "id");
      const profile = apiProfiles.find((item) => item.id === id);
      if (profile) activeApiProfileId = profile.id;
      return profile || getActiveApiProfile();
    }
    case "delete_api_profile": {
      const id = readArg(args, "id", "id");
      apiProfiles = apiProfiles.filter((item) => item.id !== id);
      if (!apiProfiles.some((item) => item.id === activeApiProfileId)) {
        activeApiProfileId = apiProfiles[0]?.id || "";
      }
      return null;
    }
    case "check_claude_status":
      return {
        logged_in: true,
        token_preview: "mock-token-preview",
        model: "Claude 3.5 Sonnet",
        base_url: "local preview",
      };
    case "get_skin":
      return "default";
    case "get_font_color":
    case "get_user_avatar":
      return "";
    case "get_personality":
      return "gentle";
    case "get_profession":
      return "companion";
    case "get_memories":
      if (args?.category) {
        return memories.filter((item) => item.category === String(args.category));
      }
      return memories;
    case "save_memory": {
      const item = {
        id: nextMemoryId++,
        category: readArg(args, "category", "category", "manual"),
        key: readArg(args, "key", "key"),
        value: readArg(args, "value", "value"),
        created_at: Date.now(),
      };
      memories.unshift(item);
      return item;
    }
    case "delete_memory": {
      const id = Number(args?.id || 0);
      const index = memories.findIndex((item) => item.id === id);
      if (index >= 0) memories.splice(index, 1);
      return null;
    }
    case "get_scheduled_tasks":
      return scheduledTasks
        .slice()
        .sort((a, b) => Number(b.enabled) - Number(a.enabled) || a.due_at - b.due_at);
    case "save_scheduled_task": {
      const now = Math.floor(Date.now() / 1000);
      const item: MockScheduledTask = {
        id: nextScheduledTaskId++,
        title: readArg(args, "title", "title", "新任务"),
        note: readArg(args, "note", "note"),
        due_at: Number(args?.dueAt ?? args?.due_at ?? now + 3600),
        repeat: readArg(args, "repeat", "repeat", "once"),
        enabled: Boolean(args?.enabled ?? true),
        last_triggered_at: null,
        created_at: now,
        updated_at: now,
      };
      scheduledTasks.unshift(item);
      return item;
    }
    case "set_scheduled_task_enabled": {
      const id = Number(args?.id || 0);
      const task = scheduledTasks.find((item) => item.id === id);
      if (task) {
        task.enabled = Boolean(args?.enabled);
        task.updated_at = Math.floor(Date.now() / 1000);
      }
      return task;
    }
    case "delete_scheduled_task": {
      const id = Number(args?.id || 0);
      const index = scheduledTasks.findIndex((item) => item.id === id);
      if (index >= 0) scheduledTasks.splice(index, 1);
      return null;
    }
    case "clear_chat_history":
    case "start_new_conversation":
      memories.unshift({
        id: nextMemoryId++,
        category: "conversation",
        key: "对话摘要",
        value: "预览模式下自动保存的对话摘要。",
        created_at: Date.now(),
      });
      return { saved_memory_id: memories[0].id };
    case "get_setting_value":
      return getMockSetting(args);
    case "set_setting_value":
      return setMockSetting(args);
    case "save_custom_pet_asset": {
      const request = (args?.request || {}) as Record<string, unknown>;
      const now = Date.now();
      const asset: MockCustomPetAsset = {
        id: String(request.id || `mock-custom-${nextCustomPetId++}`),
        name: String(request.name || "Custom Pixel Pet"),
        kind: String(request.kind || "custom-pixel"),
        manifest: String(request.manifest || ""),
        sprite_path: String(request.spriteDataUrl || request.sprite_data_url || ""),
        preview_path: String(request.previewDataUrl || request.preview_data_url || ""),
        created_at: now,
        updated_at: now,
      };
      customPetAssets = [asset, ...customPetAssets.filter((item) => item.id !== asset.id)];
      return asset;
    }
    case "list_custom_pet_assets":
      return customPetAssets;
    case "get_custom_pet_asset": {
      const id = readArg(args, "id", "id");
      return customPetAssets.find((item) => item.id === id) || null;
    }
    case "get_active_custom_pet_asset": {
      const id = settingStore.get("custom_pixel_pet_asset_id") || "";
      const asset = customPetAssets.find((item) => item.id === id) || null;
      if (id && !asset) {
        settingStore.set("custom_pixel_pet_asset_id", "");
        if (settingStore.get("pet_character") === "custom-pixel") {
          settingStore.set("pet_character", "classic");
        }
      }
      return asset;
    }
    case "set_active_custom_pet_asset": {
      const id = readArg(args, "id", "id");
      if (!id) {
        settingStore.set("custom_pixel_pet_asset_id", "");
        if (settingStore.get("pet_character") === "custom-pixel") {
          settingStore.set("pet_character", "classic");
        }
        return null;
      }
      const asset = customPetAssets.find((item) => item.id === id) || null;
      if (!asset) throw new Error("Custom pet asset was not found");
      settingStore.set("custom_pixel_pet_asset_id", id);
      settingStore.set("pet_character", "custom-pixel");
      return asset;
    }
    case "delete_custom_pet_asset": {
      const id = readArg(args, "id", "id");
      customPetAssets = customPetAssets.filter((item) => item.id !== id);
      if (settingStore.get("custom_pixel_pet_asset_id") === id) {
        settingStore.set("custom_pixel_pet_asset_id", "");
        if (settingStore.get("pet_character") === "custom-pixel") {
          settingStore.set("pet_character", "classic");
        }
      }
      return null;
    }
    case "tts_list_voices":
      return edgeVoices;
    case "tts_play":
      return null;
    case "tts_stop":
      return null;
    case "tts_synthesize":
      return {
        audio_path: createMockTtsAudio(readArg(args, "text", "text")),
        cached: false,
      };
    case "test_api_connection":
      return "Preview connection ok";
    case "test_api_compatibility":
      return {
        http_ok: true,
        http_status: 200,
        is_sse_format: true,
        is_json_format: false,
        content_extracted: "Preview compatibility ok",
        recommended_mode: "stream",
        errors: [],
        raw_preview: "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}",
        response_time_ms: 120,
      };
    case "list_api_models":
      return [
        "gpt-4o-mini",
        "gpt-4.1-mini",
        "gpt-4.1",
        "qwen-plus",
        "qwen2.5:7b",
      ];
    case "list_agents":
      return mockAgents;
    case "save_agent": {
      const draft = (args?.agent || {}) as Record<string, unknown>;
      const now = Math.floor(Date.now() / 1000);
      const next: MockAgent = {
        id: String(draft.id || "mock-agent-" + Date.now().toString(36)),
        name: String(draft.name || "新智能体"),
        avatar: String(draft.avatar || "🤖"),
        description: String(draft.description || ""),
        system_prompt: String(draft.system_prompt || ""),
        model: String(draft.model || ""),
        allowed_tools: Array.isArray(draft.allowed_tools) ? (draft.allowed_tools as string[]) : [],
        created_at: now,
        updated_at: now,
      };
      const existing = mockAgents.findIndex((item) => item.id === next.id);
      if (existing >= 0) mockAgents[existing] = next;
      else mockAgents = [next, ...mockAgents];
      return null;
    }
    case "delete_agent": {
      const id = readArg(args, "id", "id");
      mockAgents = mockAgents.filter((item) => item.id !== id);
      return null;
    }
    case "generate_agent_spec": {
      const desc = readArg(args, "description", "description", "").trim();
      if (!desc) throw new Error("请先描述你想创建的智能体");
      return {
        name: "预览智能体",
        avatar: "🤖",
        description: "在预览模式下由描述生成的示例智能体",
        system_prompt:
          "你是「预览智能体」。用户的需求是：" + desc + "。请以专业、简洁的方式帮助用户完成相关任务。",
        allowed_tools: [] as string[],
      };
    }
    default:
      return null;
  }
}

export function installDevTauriMock() {
  if (!import.meta.env.DEV) return;
  if (typeof window === "undefined") return;
  if (window.__TAURI_INTERNALS__?.metadata) return;

  const previewWindow = new URLSearchParams(window.location.search).get("window") || "main";
  mockWindows(previewWindow);
  mockIPC((cmd, args) => handleMockCommand(cmd, args as MockPayload), {
    shouldMockEvents: true,
  });
}
