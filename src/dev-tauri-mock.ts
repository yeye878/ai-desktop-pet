import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
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
  backend: string;
  api_profile_id: string;
  is_builtin: boolean;
  allowed_tools: string[];
  created_at: number;
  updated_at: number;
};
type MockWeatherConfig = {
  enabled: boolean;
  location: string;
  api_url: string;
};
type MockChatSlot = "main" | "claude" | "codex" | "dsh";
type MockChatSession = {
  id: string;
  title: string;
  kind: "main" | "agent";
  slot: MockChatSlot;
  agent_id: string;
  agent_name: string;
  agent_avatar: string;
  model: string;
  /** 模型所属渠道 provider id；空串 = 跟随本机渠道（与后端 chat_sessions.model_provider 一致）。 */
  provider: string;
  /** 思考强度 id；空串 = 跟随本机默认（与后端 chat_sessions.reasoning_effort 一致）。 */
  reasoning_effort: string;
  /** 本会话的本地 CLI 权限档位；空串 = 跟随设置页默认（chat_sessions.permission）。 */
  permission: string;
  created_at: number;
  updated_at: number;
};
type MockChatMessage = {
  id: number;
  session_id: string;
  role: "user" | "assistant";
  content: string;
  thinking?: string | null;
  created_at: number;
  quoted_role?: string | null;
  quoted_content?: string | null;
  agent_id?: string | null;
  agent_name?: string | null;
  agent_avatar?: string | null;
};
type MockActiveChat = {
  message: string;
  thinking: string;
  started_at: number;
  session_id: string;
};
type MockAgentIdentity = { id: string; name: string; avatar: string };

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
  ...[
    { name: "claude", label: "Claude Code", backend: "claude_code" },
    { name: "codex", label: "Codex", backend: "codex" },
    { name: "dsh", label: "DeepSeek Harness", backend: "dsh" },
  ].map(({ name, label, backend }) => ({
    id: `builtin-${name}`, name, avatar: "⌘", description: `本机 ${label} 独立智能体`,
    system_prompt: "独立执行，读取共享会话。", model: "", backend,
    api_profile_id: "", is_builtin: true, allowed_tools: [], created_at: 0, updated_at: 0,
  })),
  {
    id: "mock-agent-code",
    backend: "direct_api", api_profile_id: "", is_builtin: false,
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
    backend: "direct_api", api_profile_id: "", is_builtin: false,
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

const mockPetState = { state: "idle", happiness: 0.2, energy: 64 };

// === 多会话 / 三个 agent 窗口的内存版数据（浏览器、无 Tauri 环境用） ===
const MOCK_AGENT_SLOTS: Array<Exclude<MockChatSlot, "main">> = ["claude", "codex", "dsh"];
let mockChatSessions: MockChatSession[] = [];
let mockChatMessages: MockChatMessage[] = [];
let mockActiveMainSessionId = "";
let nextChatSessionId = 1;
let nextChatMessageId = 1;
let mockChatSeeded = false;
const mockActiveChats = new Map<string, MockActiveChat>();
const mockChatTimers: Array<ReturnType<typeof setTimeout>> = [];

function nowSeconds() {
  return Math.floor(Date.now() / 1000);
}

function normalizeMockSlot(value: unknown): MockChatSlot {
  const slot = String(value || "main").trim();
  return slot === "claude" || slot === "codex" || slot === "dsh" ? slot : "main";
}

function mockSlotAgent(slot: Exclude<MockChatSlot, "main">): MockAgentIdentity {
  const agent = mockAgents.find((item) => item.id === `builtin-${slot}`);
  return { id: agent?.id || `builtin-${slot}`, name: agent?.name || slot, avatar: agent?.avatar || "⌘" };
}

function findMockChatSession(id: string) {
  return mockChatSessions.find((session) => session.id === id);
}

function mockSessionOrder(id: string) {
  const match = /(\d+)$/.exec(id);
  return match ? Number(match[1]) : 0;
}

function publicMockChatSession(session: MockChatSession) {
  const messages = mockChatMessages.filter((message) => message.session_id === session.id);
  const last = messages[messages.length - 1];
  return {
    ...session,
    message_count: messages.length,
    preview: (last?.content || "").replace(/\s+/g, " ").trim().slice(0, 60),
  };
}

function sortedMockChatSessions() {
  return mockChatSessions
    .map(publicMockChatSession)
    .sort((left, right) => (right.updated_at - left.updated_at) || (mockSessionOrder(right.id) - mockSessionOrder(left.id)));
}

function nextMockConversationTitle() {
  const count = mockChatSessions.filter((session) => session.slot === "main").length;
  return count === 0 ? "默认对话" : `新对话 ${count}`;
}

function createMockChatSession(options: {
  slot: MockChatSlot;
  title?: string;
  agentId?: string;
  model?: string;
  provider?: string;
  reasoningEffort?: string;
  permission?: string;
  createdAt?: number;
  updatedAt?: number;
}): MockChatSession {
  const isMain = options.slot === "main";
  const fallbackAgent = isMain ? null : mockSlotAgent(options.slot as Exclude<MockChatSlot, "main">);
  const agentId = options.agentId || fallbackAgent?.id || "";
  const boundAgent = mockAgents.find((agent) => agent.id === agentId);
  const createdAt = options.createdAt ?? nowSeconds();
  const session: MockChatSession = {
    id: `mock-session-${nextChatSessionId++}`,
    title: options.title || (isMain ? "新对话" : `${boundAgent?.name || fallbackAgent?.name || options.slot} 对话`),
    kind: isMain ? "main" : "agent",
    slot: options.slot,
    agent_id: isMain ? "" : agentId,
    agent_name: isMain ? "" : (boundAgent?.name || fallbackAgent?.name || ""),
    agent_avatar: isMain ? "" : (boundAgent?.avatar || fallbackAgent?.avatar || ""),
    model: options.model || "",
    provider: options.provider || "",
    reasoning_effort: options.reasoningEffort || "",
    permission: options.permission || "",
    created_at: createdAt,
    updated_at: options.updatedAt ?? createdAt,
  };
  mockChatSessions.push(session);
  return session;
}

function appendMockChatMessage(
  sessionId: string,
  role: MockChatMessage["role"],
  content: string,
  extra: Partial<MockChatMessage> = {},
) {
  const message: MockChatMessage = {
    id: nextChatMessageId++,
    session_id: sessionId,
    role,
    content,
    created_at: nowSeconds(),
    ...extra,
  };
  mockChatMessages.push(message);
  return message;
}

function touchMockChatSession(session: MockChatSession) {
  session.updated_at = nowSeconds();
}

function createDefaultMockMainSession(): MockChatSession {
  const session = createMockChatSession({ slot: "main", title: "默认对话" });
  mockActiveMainSessionId = session.id;
  return session;
}

function resolveMockActiveMainSession(): MockChatSession {
  seedMockChatState();
  const active = findMockChatSession(mockActiveMainSessionId);
  if (active && active.slot === "main") return active;
  const newest = sortedMockChatSessions().find((session) => session.slot === "main");
  if (newest) {
    mockActiveMainSessionId = newest.id;
    return findMockChatSession(newest.id) as MockChatSession;
  }
  return createDefaultMockMainSession();
}

function emitMockActiveSession(session: MockChatSession) {
  void emit("chat-session-changed", { session: publicMockChatSession(session) });
}

function emitMockSessionsChanged() {
  void emit("chat-sessions-changed", {});
}

/** 预览夹具事件统一补 session_id，保证和真实后端的事件形状一致。 */
function emitMockPreviewEvent(event: string, payload: unknown) {
  if (event.startsWith("ai-") || event === "sync-chat-message") {
    const sessionId = resolveMockActiveMainSession().id;
    if (payload && typeof payload === "object") {
      void emit(event, { session_id: sessionId, ...(payload as Record<string, unknown>) });
    } else {
      void emit(event, { session_id: sessionId, text: payload });
    }
    return;
  }
  void emit(event, payload);
}

function seedMockChatState() {
  if (mockChatSeeded) return;
  mockChatSeeded = true;
  const now = nowSeconds();
  const main = createMockChatSession({ slot: "main", title: "默认对话", createdAt: now - 7200, updatedAt: now - 60 });
  mockActiveMainSessionId = main.id;
  appendMockChatMessage(
    main.id,
    "assistant",
    "预览模式已就绪：主对话和 claude / codex / dsh 三个窗口各自保存独立的会话与历史。",
    { created_at: now - 60 },
  );
  MOCK_AGENT_SLOTS.forEach((slot, index) => {
    const agent = mockSlotAgent(slot);
    const session = createMockChatSession({
      slot,
      title: `${agent.name} 对话`,
      agentId: agent.id,
      createdAt: now - 3600 + index,
      updatedAt: now - 180 + index,
    });
    appendMockChatMessage(session.id, "assistant", `这里是 ${agent.name} 独立窗口的预览会话，可以继续提问。`, {
      created_at: now - 180 + index,
      agent_id: agent.id,
      agent_name: agent.name,
      agent_avatar: agent.avatar,
    });
  });
}

function resetMockChatState() {
  mockChatTimers.splice(0).forEach((timer) => clearTimeout(timer));
  mockActiveChats.clear();
  mockChatSessions = [];
  mockChatMessages = [];
  mockActiveMainSessionId = "";
  nextChatSessionId = 1;
  nextChatMessageId = 1;
  // 清空后只重建默认主会话，与后端 clear_chat_history 的行为一致。
  mockChatSeeded = true;
  createDefaultMockMainSession();
}

function scheduleMockChatTurn(
  session: MockChatSession,
  active: MockActiveChat,
  reply: string,
  agent: MockAgentIdentity | null,
) {
  const sessionId = session.id;
  const stillActive = () => mockActiveChats.get(sessionId) === active && Boolean(findMockChatSession(sessionId));
  const later = (delay: number, action: () => void) => {
    mockChatTimers.push(setTimeout(action, delay));
  };

  later(420, () => {
    if (!stillActive()) return;
    const text = "正在检索当前会话的上下文……";
    active.thinking += text;
    void emit("ai-thinking", { session_id: sessionId, text });
  });

  if (agent) {
    later(760, () => {
      if (!stillActive()) return;
      void emit("ai-tool-event", {
        id: `mock-tool-${Date.now().toString(36)}`,
        tool_name: "run_command",
        status: "completed",
        summary: "预览模式：模拟一次工具调用",
        arguments: JSON.stringify({ command: "echo preview-session" }),
        command: "echo preview-session",
        output: "preview",
        exit_code: 0,
        duration_ms: 32,
        session_id: sessionId,
      });
    });
  }

  const splitAt = Math.ceil(reply.length * 0.55);
  later(1000, () => {
    if (!stillActive()) return;
    void emit("ai-answer-delta", { session_id: sessionId, text: reply.slice(0, splitAt) });
  });

  later(1450, () => {
    if (!stillActive()) return;
    void emit("ai-answer-delta", { session_id: sessionId, text: reply.slice(splitAt) });
  });

  later(1900, () => {
    if (!stillActive()) return;
    mockActiveChats.delete(sessionId);
    appendMockChatMessage(sessionId, "assistant", reply, {
      thinking: active.thinking || null,
      agent_id: agent?.id ?? null,
      agent_name: agent?.name ?? null,
      agent_avatar: agent?.avatar ?? null,
    });
    touchMockChatSession(session);
    void emit("ai-finished", {
      text: reply,
      thinking: active.thinking || null,
      session_id: sessionId,
      agent_id: agent?.id ?? null,
      agent_name: agent?.name ?? null,
      agent_avatar: agent?.avatar ?? null,
    });
    emitMockSessionsChanged();
  });
}

// === agent 模型目录夹具（浏览器 dev 模式）：形状严格对齐 docs/agent-model-catalog-contract.md 第 2 节 ===
type MockAgentEffortOption = { id: string; name: string; description?: string };
type MockAgentModelOption = {
  id: string;
  name: string;
  description?: string;
  /** 仅 dsh：provider 路由 id / 展示名。 */
  provider?: string;
  provider_label?: string;
  default_effort: string;
  efforts: MockAgentEffortOption[];
};
type MockAgentModelCatalog = {
  slot: string;
  backend: string;
  local_provider: string;
  local_model: string;
  local_model_label: string;
  local_effort: string;
  models: MockAgentModelOption[];
  error: string;
  fetched_at: number;
};

/**
 * 返回某个 agent 窗口的模型目录夹具（保持自包含，verify 脚本可直接抽取该函数做断言）：
 * - dsh：多渠道（opendesgin/deepseek-official/together/qwen），local_provider=opendesgin，
 *   同一 model id 出现在多个渠道（deepseek-v4.1-flash），模型总数 > 12 以覆盖搜索框；
 * - codex：6 个模型，efforts low..ultra；
 * - claude：opus/sonnet/haiku 别名 + low/medium/high/xhigh/max 五档。
 */
function mockAgentModelCatalog(slot: string): MockAgentModelCatalog {
  const fetchedAt = Date.now();
  if (slot === "dsh") {
    const opendesginEfforts: MockAgentEffortOption[] = [
      { id: "off", name: "Off", description: "关闭思考" },
      { id: "low", name: "Low", description: "轻量思考" },
      { id: "medium", name: "Medium", description: "标准思考" },
      { id: "high", name: "High", description: "深度思考" },
      { id: "xhigh", name: "XHigh", description: "极高思考预算" },
      { id: "max", name: "Max", description: "最大思考预算" },
    ];
    const deepseekEfforts: MockAgentEffortOption[] = [
      { id: "off", name: "Off", description: "关闭思考" },
      { id: "low", name: "Low", description: "轻量思考" },
      { id: "high", name: "High", description: "深度思考" },
      { id: "max", name: "Max", description: "最大思考预算" },
    ];
    const togetherEfforts: MockAgentEffortOption[] = [
      { id: "low", name: "Low" },
      { id: "medium", name: "Medium" },
      { id: "high", name: "High" },
      { id: "xhigh", name: "XHigh" },
      { id: "max", name: "Max" },
    ];
    return {
      slot: "dsh",
      backend: "dsh",
      // 本机默认渠道是聚合渠道 opendesgin；同一个 model id 会出现在多个渠道，靠 (provider,id) 区分。
      local_provider: "opendesgin",
      local_model: "deepseek-v4.1-flash",
      local_model_label: "opendesgin / deepseek-v4.1-flash",
      local_effort: "high",
      models: [
        {
          id: "claude-opus-5",
          name: "Claude Opus 5",
          description: "Anthropic 旗舰，长上下文推理",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "high",
          efforts: opendesginEfforts,
        },
        {
          id: "claude-sonnet-5.5",
          name: "Claude Sonnet 5.5",
          description: "速度与质量平衡",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "high",
          efforts: opendesginEfforts,
        },
        {
          id: "deepseek-v4.1-flash",
          name: "DeepSeek V4.1 Flash",
          description: "本机默认模型",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "high",
          efforts: opendesginEfforts,
        },
        {
          id: "gemini-3.7-flash",
          name: "Gemini 3.7 Flash",
          description: "多模态快速模型",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "medium",
          efforts: opendesginEfforts,
        },
        {
          id: "glm-5.2",
          name: "GLM 5.2",
          description: "中文对话与工具调用",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "medium",
          efforts: opendesginEfforts,
        },
        {
          id: "gpt-6.1",
          name: "GPT-6.1",
          description: "OpenAI 通用模型",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "medium",
          efforts: opendesginEfforts,
        },
        {
          id: "kimi-k3",
          name: "Kimi K3",
          description: "超长上下文",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "medium",
          efforts: opendesginEfforts,
        },
        {
          id: "qwen3.7-max",
          name: "Qwen 3.7 Max",
          description: "阿里旗舰模型",
          provider: "opendesgin",
          provider_label: "opendesgin",
          default_effort: "medium",
          efforts: opendesginEfforts,
        },
        {
          id: "deepseek-v4-flash",
          name: "DeepSeek-V4-Flash",
          description: "低延迟日常对话",
          provider: "deepseek-official",
          provider_label: "DeepSeek",
          default_effort: "high",
          efforts: deepseekEfforts,
        },
        {
          id: "deepseek-v4-pro",
          name: "DeepSeek-V4-Pro",
          description: "复杂推理与长任务",
          provider: "deepseek-official",
          provider_label: "DeepSeek",
          default_effort: "high",
          efforts: deepseekEfforts,
        },
        {
          id: "deepseek-v4-flash-vision-exp",
          name: "DeepSeek-V4-Flash-Vision-Exp",
          description: "多模态实验模型",
          provider: "deepseek-official",
          provider_label: "DeepSeek",
          default_effort: "medium",
          efforts: deepseekEfforts,
        },
        {
          id: "deepseek-v4.1-flash",
          name: "DeepSeek V4.1 Flash",
          description: "Together 渠道托管",
          provider: "together",
          provider_label: "Together",
          default_effort: "medium",
          efforts: togetherEfforts,
        },
        {
          id: "deepseek-v4.1-flash",
          name: "DeepSeek V4.1 Flash",
          description: "Qwen 渠道托管",
          provider: "qwen",
          provider_label: "Qwen",
          default_effort: "max",
          efforts: togetherEfforts,
        },
        {
          id: "qwen3.7-max",
          name: "Qwen 3.7 Max",
          description: "Qwen 官方渠道",
          provider: "qwen",
          provider_label: "Qwen",
          default_effort: "medium",
          efforts: togetherEfforts,
        },
      ],
      error: "",
      fetched_at: fetchedAt,
    };
  }
  if (slot === "codex") {
    const codexEfforts: MockAgentEffortOption[] = [
      { id: "low", name: "Low" },
      { id: "medium", name: "Medium" },
      { id: "high", name: "High" },
      { id: "xhigh", name: "XHigh" },
      { id: "max", name: "Max" },
      { id: "ultra", name: "Ultra" },
    ];
    return {
      slot: "codex",
      backend: "codex",
      local_provider: "",
      local_model: "gpt-6.1-terra",
      local_model_label: "GPT-6.1 Terra",
      local_effort: "high",
      models: [
        { id: "gpt-6.1-terra", name: "GPT-6.1 Terra", description: "默认编码模型", default_effort: "high", efforts: codexEfforts },
        { id: "gpt-6.1-pro", name: "GPT-6.1 Pro", description: "高难度推理", default_effort: "high", efforts: codexEfforts },
        { id: "gpt-6.1", name: "GPT-6.1", description: "通用模型", default_effort: "medium", efforts: codexEfforts },
        { id: "gpt-5.4-codex", name: "GPT-5.4 Codex", description: "上一代编码模型", default_effort: "high", efforts: codexEfforts },
        { id: "gpt-5.4", name: "GPT-5.4", description: "上一代通用模型", default_effort: "medium", efforts: codexEfforts },
        { id: "gpt-5-codex-mini", name: "GPT-5 Codex Mini", description: "轻量快速", default_effort: "low", efforts: codexEfforts },
      ],
      error: "",
      fetched_at: fetchedAt,
    };
  }
  if (slot === "claude") {
    const claudeEfforts: MockAgentEffortOption[] = [
      { id: "low", name: "低", description: "更快、更省 token" },
      { id: "medium", name: "中", description: "平衡速度与质量" },
      { id: "high", name: "高", description: "默认深度" },
      { id: "xhigh", name: "极高", description: "复杂任务" },
      { id: "max", name: "最高", description: "最大思考预算" },
    ];
    const alias = (id: string, name: string, mapped: string): MockAgentModelOption => ({
      id,
      name,
      description: "本机映射到 " + mapped,
      default_effort: "high",
      efforts: claudeEfforts,
    });
    return {
      slot: "claude",
      backend: "claude_code",
      local_provider: "",
      local_model: "sonnet",
      local_model_label: "Sonnet",
      local_effort: "high",
      models: [
        alias("opus", "Opus", "claude-opus-5"),
        alias("sonnet", "Sonnet", "claude-sonnet-5"),
        alias("haiku", "Haiku", "claude-haiku-5"),
      ],
      error: "",
      fetched_at: fetchedAt,
    };
  }
  return {
    slot: slot || "unknown",
    backend: "",
    local_provider: "",
    local_model: "",
    local_model_label: "",
    local_effort: "",
    models: [],
    error: "未知的 agent slot：" + (slot || "(空)"),
    fetched_at: fetchedAt,
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
    case "get_agent_model_catalog": {
      // 浏览器 dev 模式没有真实后端探测，直接返回三 slot 夹具；force 刷新同样返回夹具。
      const slot = readArg(args, "slot", "slot", "dsh").trim() || "dsh";
      return mockAgentModelCatalog(slot);
    }
    case "get_pet_state":
    case "tick":
      return { ...mockPetState };
    case "set_pet_state":
      mockPetState.state = String(args?.newState || "idle");
      if (mockPetState.state === "happy") mockPetState.happiness = Math.min(1, mockPetState.happiness + 0.2);
      if (mockPetState.state === "sleeping") mockPetState.energy = Math.min(100, mockPetState.energy + 15);
      return null;
    case "list_chat_sessions": {
      seedMockChatState();
      const slotFilter = readArg(args, "slot", "slot", "").trim();
      return sortedMockChatSessions().filter((session) => !slotFilter || session.slot === slotFilter);
    }
    case "get_chat_session": {
      seedMockChatState();
      const id = readArg(args, "id", "id");
      const session = findMockChatSession(id);
      return session ? publicMockChatSession(session) : null;
    }
    case "create_chat_session": {
      seedMockChatState();
      const slot = normalizeMockSlot(args?.slot);
      const session = createMockChatSession({
        slot,
        title: readArg(args, "title", "title", "").trim() || (slot === "main" ? nextMockConversationTitle() : undefined),
        agentId: readArg(args, "agentId", "agent_id", "").trim() || undefined,
        model: readArg(args, "model", "model", "").trim(),
        provider: readArg(args, "provider", "model_provider", "").trim(),
        reasoningEffort: readArg(args, "reasoningEffort", "reasoning_effort", "").trim(),
        permission: readArg(args, "permission", "permission", "").trim(),
      });
      if (slot === "main" && args?.activate !== false) {
        mockActiveMainSessionId = session.id;
        emitMockActiveSession(session);
      }
      emitMockSessionsChanged();
      return publicMockChatSession(session);
    }
    case "get_chat_session_messages": {
      seedMockChatState();
      const sessionId = readArg(args, "sessionId", "session_id", "") || resolveMockActiveMainSession().id;
      const requestedLimit = Number(args?.limit);
      const limit = Number.isFinite(requestedLimit) && requestedLimit > 0 ? requestedLimit : 500;
      const messages = mockChatMessages.filter((message) => message.session_id === sessionId);
      const selected = messages.length > limit ? messages.slice(-limit) : messages;
      return selected.map((message) => ({ ...message }));
    }
    case "rename_chat_session": {
      seedMockChatState();
      const session = findMockChatSession(readArg(args, "id", "id"));
      if (!session) throw new Error("会话不存在");
      const title = readArg(args, "title", "title").trim();
      if (!title) throw new Error("会话标题不能为空");
      session.title = title;
      touchMockChatSession(session);
      emitMockSessionsChanged();
      return publicMockChatSession(session);
    }
    case "delete_chat_session": {
      seedMockChatState();
      const id = readArg(args, "id", "id");
      const session = findMockChatSession(id);
      if (!session) throw new Error("会话不存在");
      mockChatSessions = mockChatSessions.filter((item) => item.id !== id);
      mockChatMessages = mockChatMessages.filter((message) => message.session_id !== id);
      mockActiveChats.delete(id);
      let activeChanged = false;
      if (mockActiveMainSessionId === id) {
        const newest = sortedMockChatSessions().find((item) => item.slot === "main");
        if (newest) mockActiveMainSessionId = newest.id;
        else createDefaultMockMainSession();
        activeChanged = true;
      }
      emitMockSessionsChanged();
      if (activeChanged) {
        const activeSession = findMockChatSession(mockActiveMainSessionId);
        if (activeSession) emitMockActiveSession(activeSession);
      }
      return { active_main_session_id: mockActiveMainSessionId };
    }
    case "set_chat_session_permission": {
      seedMockChatState();
      const session = findMockChatSession(readArg(args, "id", "id"));
      if (!session) throw new Error("会话不存在");
      // 与后端一致：只改权限档位，不刷新 updated_at（不让会话被顶到列表最前）。
      session.permission = readArg(args, "permission", "permission", "").trim();
      emitMockSessionsChanged();
      return publicMockChatSession(session);
    }
    case "set_chat_session_binding": {
      seedMockChatState();
      const session = findMockChatSession(readArg(args, "id", "id"));
      if (!session) throw new Error("会话不存在");
      const agentId = args?.agentId ?? args?.agent_id;
      const agentName = args?.agentName ?? args?.agent_name;
      const agentAvatar = args?.agentAvatar ?? args?.agent_avatar;
      const model = args?.model;
      const provider = args?.provider ?? args?.model_provider;
      const reasoningEffort = args?.reasoningEffort ?? args?.reasoning_effort;
      if (agentId !== undefined) session.agent_id = String(agentId);
      if (agentName !== undefined) session.agent_name = String(agentName);
      if (agentAvatar !== undefined) session.agent_avatar = String(agentAvatar);
      if (model !== undefined) session.model = String(model);
      if (provider !== undefined) session.provider = String(provider);
      if (reasoningEffort !== undefined) session.reasoning_effort = String(reasoningEffort);
      touchMockChatSession(session);
      emitMockSessionsChanged();
      return publicMockChatSession(session);
    }
    case "set_active_chat_session": {
      seedMockChatState();
      const session = findMockChatSession(readArg(args, "id", "id"));
      if (!session) throw new Error("会话不存在");
      if (session.slot !== "main") throw new Error("只有主对话会话可以设为活跃会话");
      mockActiveMainSessionId = session.id;
      touchMockChatSession(session);
      emitMockActiveSession(session);
      emitMockSessionsChanged();
      return publicMockChatSession(session);
    }
    case "get_active_chat_session":
      return publicMockChatSession(resolveMockActiveMainSession());
    case "start_new_conversation": {
      seedMockChatState();
      const session = createMockChatSession({ slot: "main", title: nextMockConversationTitle() });
      mockActiveMainSessionId = session.id;
      emitMockActiveSession(session);
      emitMockSessionsChanged();
      return publicMockChatSession(session);
    }
    case "get_chat_history": {
      seedMockChatState();
      const requested = readArg(args, "sessionId", "session_id", "");
      const session = requested ? findMockChatSession(requested) : undefined;
      const sessionId = session?.id || resolveMockActiveMainSession().id;
      const requestedLimit = Number(args?.limit);
      const limit = Number.isFinite(requestedLimit) && requestedLimit > 0 ? requestedLimit : 50;
      const messages = mockChatMessages.filter((message) => message.session_id === sessionId);
      const selected = messages.length > limit ? messages.slice(-limit) : messages;
      return selected.map((message) => ({ ...message }));
    }
    case "get_active_chat": {
      const requested = readArg(args, "sessionId", "session_id", "");
      if (requested) {
        const hit = mockActiveChats.get(requested);
        return hit ? { ...hit } : null;
      }
      const mainActive = mockActiveMainSessionId ? mockActiveChats.get(mockActiveMainSessionId) : undefined;
      if (mainActive) return { ...mainActive };
      const activeChats = [...mockActiveChats.values()];
      return activeChats.length > 0 ? { ...activeChats[activeChats.length - 1] } : null;
    }
    case "abort_ai": {
      const requested = readArg(args, "sessionId", "session_id", "");
      if (requested) {
        mockActiveChats.delete(requested);
      } else if (mockActiveMainSessionId && mockActiveChats.has(mockActiveMainSessionId)) {
        mockActiveChats.delete(mockActiveMainSessionId);
      } else {
        mockActiveChats.clear();
      }
      return null;
    }
    case "clear_chat_history": {
      resetMockChatState();
      const session = resolveMockActiveMainSession();
      void emit("chat-history-cleared", {});
      emitMockSessionsChanged();
      emitMockActiveSession(session);
      return { active_main_session_id: session.id };
    }
    case "send_to_ai": {
      seedMockChatState();
      const message = readArg(args, "message", "message", "").trim();
      if (!message) throw new Error("消息不能为空");
      const requestedSessionId = readArg(args, "sessionId", "session_id", "");
      const session = (requestedSessionId && findMockChatSession(requestedSessionId)) || resolveMockActiveMainSession();
      if (mockActiveChats.has(session.id)) throw new Error("AI 正在思考中，请稍等当前回复完成后");
      const targetAgentId = readArg(args, "targetAgentId", "target_agent_id", "").trim();
      const targetAgent = targetAgentId ? mockAgents.find((agent) => agent.id === targetAgentId) : undefined;
      const agentInfo: MockAgentIdentity | null = targetAgentId
        ? { id: targetAgent?.id || targetAgentId, name: targetAgent?.name || targetAgentId, avatar: targetAgent?.avatar || "🤖" }
        : null;
      const model = readArg(args, "model", "model", "").trim();
      const quotedRole = readArg(args, "quotedRole", "quoted_role", "").trim();
      const quotedContent = readArg(args, "quotedContent", "quoted_content", "").trim();
      appendMockChatMessage(session.id, "user", message, {
        quoted_role: quotedRole || null,
        quoted_content: quotedContent || null,
      });
      touchMockChatSession(session);
      if (!session.title) {
        session.title = message.replace(/\s+/g, " ").trim().slice(0, 24);
      }
      emitMockSessionsChanged();
      // 与真实后端一致：用户消息广播给所有视图，各端按最后一条消息去重。
      void emit("sync-chat-message", {
        session_id: session.id,
        role: "user",
        content: message,
        quote: quotedRole && quotedContent ? { role: quotedRole, content: quotedContent } : undefined,
      });
      const active: MockActiveChat = { message, thinking: "", started_at: nowSeconds(), session_id: session.id };
      mockActiveChats.set(session.id, active);
      void emit("ai-started", { session_id: session.id, message });
      const label = agentInfo ? `「${agentInfo.name}」` : "主对话";
      const modelHint = model ? `（${model}）` : "";
      const compactMessage = message.replace(/\s+/g, " ").slice(0, 140);
      const reply = agentInfo
        ? `${label}${modelHint}收到：${compactMessage}\n\n这是无 Tauri 的预览回复，真实 CLI 接入后这里会显示 ${agentInfo.name} 的实际输出。`
        : `预览模式收到：${compactMessage}${modelHint}\n\n这是浏览器 mock 的模拟回复，可以在会话列表里切换会话验证历史隔离。`;
      scheduleMockChatTurn(session, active, reply, agentInfo);
      return { started: true };
    }
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
      if (backendType === "codex") return "Codex / 本机配置";
      if (backendType === "dsh") return "DeepSeek Harness / 本机配置";
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
    case "check_codex_status":
      return {
        installed: true,
        logged_in: true,
        version: "codex-cli / preview",
        executable: "C:\\Users\\preview\\AppData\\Roaming\\npm\\codex.cmd",
        workspace: "C:\\Users\\preview\\AppData\\Roaming\\com.ai-desktop-pet.app\\codex-workspace",
        message: "预览模式：已连接本机 Codex。",
      };
    case "open_codex_config":
      return null;
    case "check_dsh_status":
      return {
        installed: true,
        logged_in: true,
        version: "dsh / preview",
        executable: "C:\\Users\\preview\\AppData\\Roaming\\npm\\node_modules\\@deepseek-ai\\dsh\\lib\\bin.js",
        model: "deepseek-flash",
        provider: "deepseek-official",
        profile: "C:\\Users\\preview\\.dsh\\profiles\\dsh-pet",
        workspace: "C:\\Users\\preview\\AppData\\Roaming\\com.ai-desktop-pet.app\\dsh-workspace",
        message: "预览模式：已连接本机 DeepSeek Harness。",
      };
    case "open_dsh_config":
      return null;
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
    case "get_collaboration_state":
      return null;
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
        backend: String(draft.backend || "direct_api"),
        api_profile_id: String(draft.api_profile_id || ""),
        is_builtin: false,
        allowed_tools: Array.isArray(draft.allowed_tools) ? (draft.allowed_tools as string[]) : [],
        created_at: now,
        updated_at: now,
      };
      const existing = mockAgents.findIndex((item) => item.id === next.id);
      if (existing >= 0) mockAgents[existing] = next;
      else mockAgents = [next, ...mockAgents];
      void emit("agents-changed");
      return null;
    }
    case "delete_agent": {
      const id = readArg(args, "id", "id");
      mockAgents = mockAgents.filter((item) => item.id !== id);
      void emit("agents-changed");
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
  const previewQuery = new URLSearchParams(window.location.search);
  if (previewQuery.get("activity") === "markdown") {
    // 验收 Markdown 排版：直接把一条带完整语法的回复塞进对话
    const reply = [
      "**一、模型侧：两大巨头同日开打**",
      "- OpenAI 把 **GPT-6 全面开放**，免费用户也能直接用（[报道](https://example.com/news)）。",
      "- 谷歌 **Gemini 4 Argon** 已对部分用户开放。",
      "",
      "1. 先看编码能力",
      "2. 再看推理芯片",
      "   - 子条目 `d-Matrix`",
      "",
      "> 一句话锐评：模型免费送、人才互相挖。",
      "",
      "| 公司 | 动作 |",
      "| --- | --- |",
      "| OpenAI | 免费开放 |",
      "| Anthropic | 降价 |",
      "",
      "行内公式：连续性方程 \\(u_x+v_y=0\\)，分数 \\(\\frac{a}{b}\\)。",
      "",
      "\\[",
      "\\begin{cases}",
      "\\rho(u u_x+v u_y)=-p_x+\\mu(u_{xx}+u_{yy}) \\\\",
      "u_x+v_y=0",
      "\\end{cases}",
      "\\]",
    ].join("\n");
    setTimeout(() => {
      emitMockPreviewEvent("ai-finished", { text: reply, thinking: null });
    }, 300);
  }
  if (previewQuery.get("activity") === "demo") {
    // Codex 后端轨迹夹具：思考增量、命令生命周期、文件变更、任务清单、
    // 非 0 退出码、配置提示与最终回复，用来在没有真实 CLI 的预览环境里
    // 验收 "思考过程 + 工具轨迹" 的完整展示链路。
    const frames: [string, unknown][] = [
      ["sync-chat-message", { role: "user", content: "检查项目入口并运行测试" }],
      ["ai-thinking", { text: "**确认入口与事件链路**\n\n先看入口文件怎么装配，再确认活动流事件的名字。" }],
      ["ai-thinking", { text: "\n\n运行前需要知道回归用例的命令，读一下 package.json。" }],
      ["ai-tool-event", { id: "preview-notice", tool_name: "codex_notice", status: "notice", summary: "Codex is ignoring 3 unrecognized configuration settings.", arguments: "{}", output: "Codex is ignoring 3 unrecognized configuration settings.\n  user (C:\\Users\\demo\\.codex\\config.toml): `disable_response_storage` is ignored." }],
      ["ai-tool-event", { id: "preview-plan", tool_name: "update_plan", status: "completed", summary: "更新任务清单", arguments: JSON.stringify({ plan: [{ step: "读取入口文件", status: "completed" }, { step: "运行回归用例", status: "in_progress" }, { step: "汇总结果", status: "pending" }] }, null, 2), duration_ms: 12 }],
      ["ai-tool-event", { id: "preview-read", tool_name: "run_command", status: "running", summary: "Get-Content src/main.ts", command: "Get-Content src/main.ts", arguments: JSON.stringify({ command: "Get-Content src/main.ts" }) }],
      ["ai-tool-event", { id: "preview-read", tool_name: "run_command", status: "completed", summary: "Get-Content src/main.ts", command: "Get-Content src/main.ts", arguments: JSON.stringify({ command: "Get-Content src/main.ts" }), output: "import { createApp } from \"vue\";\nimport { createPinia } from \"pinia\";", exit_code: 0, duration_ms: 412 }],
      ["ai-thinking", { text: "入口使用 Vue 3 + Pinia，接下来跑回归用例确认活动流没有回归。" }],
      ["ai-tool-event", { id: "preview-test", tool_name: "run_command", status: "running", summary: "npm run verify:agent-activity", command: "npm run verify:agent-activity", arguments: JSON.stringify({ command: "npm run verify:agent-activity" }) }],
      ["ai-tool-event", { id: "preview-test", tool_name: "run_command", status: "failed", summary: "npm run verify:agent-activity", command: "npm run verify:agent-activity", arguments: JSON.stringify({ command: "npm run verify:agent-activity" }), output: "not ok 4 - 活动流顺序断言失败\n# 预期 reasoning 在 command 之前", exit_code: 1, duration_ms: 2640 }],
      ["ai-thinking", { text: "回归用例失败，需要修正顺序后再跑一次。" }],
      ["ai-tool-event", { id: "preview-fix", tool_name: "edit_file", status: "completed", summary: "src/services/agentActivity.ts", path: "src/services/agentActivity.ts", arguments: JSON.stringify({ changes: [{ path: "src/services/agentActivity.ts", kind: "update" }] }), changes: [{ path: "src/services/agentActivity.ts", kind: "update" }], duration_ms: 96 }],
      ["ai-tool-event", { id: "preview-retest", tool_name: "run_command", status: "completed", summary: "npm run verify:agent-activity", command: "npm run verify:agent-activity", arguments: JSON.stringify({ command: "npm run verify:agent-activity" }), output: "ok 1 - 轨迹顺序\nok 2 - 思考增量\nok 3 - 终态不回退", exit_code: 0, duration_ms: 2180 }],
      ["ai-tool-event", { id: "preview-plan", tool_name: "update_plan", status: "completed", summary: "更新任务清单", arguments: JSON.stringify({ plan: [{ step: "读取入口文件", status: "completed" }, { step: "运行回归用例", status: "completed" }, { step: "汇总结果", status: "completed" }] }, null, 2), duration_ms: 9 }],
      ["ai-answer-delta", { text: "入口使用 Vue 3 + Pinia，回归用例先失败后修复通过。" }],
      ["ai-finished", { text: "入口使用 Vue 3 + Pinia，回归用例先失败后修复通过。", thinking: "**确认入口与事件链路**\n\n先看入口文件怎么装配，再确认活动流事件的名字。\n\n运行前需要知道回归用例的命令，读一下 package.json。入口使用 Vue 3 + Pinia，接下来跑回归用例确认活动流没有回归。回归用例失败，需要修正顺序后再跑一次。" }],
    ];
    frames.forEach(([event, payload], index) => setTimeout(() => { emitMockPreviewEvent(event, payload); }, 1000 + index * 420));
  }
}
