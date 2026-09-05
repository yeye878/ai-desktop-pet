<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useChatStore, type FileAttachment, type Message, type MessageAgent } from "../stores/chat";
import { usePetStore } from "../stores/pet";
import { useSkillsStore, type Skill } from "../stores/skills";
import { useAgentsStore, type Agent } from "../stores/agents";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, type DragDropEvent } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import type { UnlistenFn } from "@tauri-apps/api/event";
import BgCanvas from "./BgCanvas.vue";
import AppIcon from "./AppIcon.vue";
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
import {
  DEFAULT_VOICE_SETTINGS,
  isSpeechRecognitionSupported,
  parseVoiceSettings,
  type VoiceSettings,
  type VoiceStatus,
} from "../services/voice";

const emit = defineEmits<{ close: [] }>();
const chat = useChatStore();
const pet = usePetStore();
const skillsStore = useSkillsStore();
const agentsStore = useAgentsStore();

type ActiveTab = "chat" | "clipboard";
type ClipboardItem = {
  id: string | number;
  content: string;
  pinned: boolean;
  created_at: string | number;
};
type ChatHistoryItem = {
  role: string;
  content: string;
  thinking?: string | null;
  created_at?: string | number;
  quoted_role?: string | null;
  quoted_content?: string | null;
  agent_id?: string | null;
  agent_name?: string | null;
  agent_avatar?: string | null;
};
type ActiveChat = {
  message: string;
  thinking: string;
  started_at: string | number;
};
type AiFinishedPayload = {
  text: string;
  thinking: string | null;
  agentId?: string | null;
  agentName?: string | null;
  agentAvatar?: string | null;
};
type AiErrorPayload = {
  message: string;
  thinking: string | null;
  aborted: boolean;
};
type AnswerDeltaPayload = {
  text: string;
};
type ApiProfile = {
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
type SlashPickerItem =
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

const ACTIVE_TAB_KEY = "ai-desktop-pet.active-chat-tab";
const currentWindow = getCurrentWindow();
const activeTab = ref<ActiveTab>(loadActiveTab());
const input = ref("");
const inputRef = ref<HTMLInputElement | null>(null);
const clipboardItems = ref<ClipboardItem[]>([]);
const clipboardSearch = ref("");
const chatBubbleRef = ref<HTMLDivElement | null>(null);
const messagesRef = ref<HTMLDivElement | null>(null);
const chatEndRef = ref<HTMLDivElement | null>(null);
const activeMessageMenuId = ref<number | null>(null);
const slashSelectedIndex = ref(0);
const mentionSelectedIndex = ref(0);
const isFileOver = ref(false);
const BG_KEY = "ai-desktop-pet.chat-bg";
const CUSTOM_BG_KEY = "ai-desktop-pet.chat-bg-custom";
const VOICE_SETTINGS_KEY = "voice_settings";
const chatBg = ref(localStorage.getItem(BG_KEY) || "none");
const customBgImage = ref(localStorage.getItem(CUSTOM_BG_KEY) || "");
const bgCanvasRef = ref<InstanceType<typeof BgCanvas> | null>(null);
const voiceStatus = ref<VoiceStatus>("idle");
const voiceInterimText = ref("");
const voiceError = ref("");
const voiceSettings = ref<VoiceSettings>({ ...DEFAULT_VOICE_SETTINGS });
let scrollFrame: number | null = null;
let scrollTimers: ReturnType<typeof setTimeout>[] = [];
let unlistenDragDrop: UnlistenFn | null = null;
let unlistenThinking: UnlistenFn | null = null;
let unlistenAnswerDelta: UnlistenFn | null = null;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;
let unlistenChatCleared: UnlistenFn | null = null;
let unlistenVoiceChanged: UnlistenFn | null = null;
let unlistenSyncMessage: UnlistenFn | null = null;
let unlistenApiConfigChanged: UnlistenFn | null = null;
let unlistenScheduledTaskTriggered: UnlistenFn | null = null;
let unlistenAgentsChanged: UnlistenFn | null = null;
let unlistenSkillsChanged: UnlistenFn | null = null;
let unlistenChatBg: UnlistenFn | null = null;
let unlistenAppearanceChanged: UnlistenFn | null = null;

interface ToolConfirmPayload {
  id: string;
  tool_name: string;
  arguments: string;
  summary?: string;
  command?: string | null;
  path?: string | null;
}
const pendingConfirm = ref<ToolConfirmPayload | null>(null);
let unlistenToolConfirm: UnlistenFn | null = null;
let unlistenToolConfirmResolved: UnlistenFn | null = null;
let unlistenAskUser: UnlistenFn | null = null;

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
let fileErrorTimer: ReturnType<typeof setTimeout> | null = null;

const thinkingContent = ref("");
const streamingAnswer = ref("");
const isThinkingCollapsed = ref(true);
const expandedThinking = reactive(new Set<number>());
const backendType = ref("claude_code");
const currentApiProfile = ref<ApiProfile | null>(null);
const apiProfiles = ref<ApiProfile[]>([]);
const activeApiProfileId = ref("");
const userAvatar = computed(() => {
  return pet.userAvatar.trim();
});
const voiceButtonTitle = computed(() => {
  if (!voiceSettings.value.enabled) return "语音交互已关闭";
  if (!isSpeechRecognitionSupported()) return "当前 WebView 不支持语音识别";
  return voiceStatus.value === "speaking" ? "打开语音面板" : "打开语音输入";
});
const filteredClipboardItems = computed(() => {
  const keyword = clipboardSearch.value.trim().toLowerCase();
  const items = keyword
    ? clipboardItems.value.filter((item) => item.content.toLowerCase().includes(keyword))
    : clipboardItems.value;

  return [...items].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return getClipboardTime(b.created_at) - getClipboardTime(a.created_at);
  });
});
const slashQuery = computed(() => {
  if (activeTab.value === "chat" && chat.isLoading) return null;
  return slashQueryFromInput(input.value);
});
const slashItems = computed<SlashPickerItem[]>(() => {
  const query = slashQuery.value;
  if (query === null) return [];

  const skillItems: SlashPickerItem[] =
    activeTab.value === "chat"
      ? skillsStore.skills
          .filter((skill) => skillMatchesSlashQuery(skill, query))
          .map((skill) => ({
            type: "skill",
            key: `skill:${skill.id}`,
            title: skill.name || "(未命名技能)",
            description: skill.description || "激活这个 skill 并继续输入你的需求",
            hint: skill.is_active ? "已激活" : "/skill",
            skill,
          }))
      : [];

  const commandItems: SlashPickerItem[] = SLASH_COMMANDS
    .filter((command) => slashCommandMatches(command, query, "pet"))
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
const showSlashMenu = computed(() => slashQuery.value !== null && slashItems.value.length > 0);

// === @ 提及智能体 ===
const mentionDismissed = ref(false);
watch(input, () => {
  mentionDismissed.value = false;
});

const mentionQuery = computed(() => {
  if (activeTab.value !== "chat" || chat.isLoading) return null;
  return mentionQueryFromInput(input.value);
});
const mentionItems = computed<Agent[]>(() => {
  const query = mentionQuery.value;
  if (query === null) return [];
  const q = query.query.toLowerCase();
  return agentsStore.agents
    .filter((agent) => {
      if (!q) return true;
      return [agent.name, agent.description].some((v) => v.toLowerCase().includes(q));
    })
    .slice(0, 8);
});
const showMentionMenu = computed(
  () => !mentionDismissed.value && mentionQuery.value !== null && mentionItems.value.length > 0,
);

const knownAgentNames = computed(() => agentsStore.agents.map((a) => a.name));

/** 用户消息里被 @ 的智能体名字（用于发送时传给后端） */
function petMentionNames(text: string): string[] {
  return extractMentionNames(text, knownAgentNames.value).filter((name) => agentsStore.findByName(name));
}

// Loading 超时保底机制
let loadingTimeout: ReturnType<typeof setTimeout> | null = null;
function resetLoadingTimeout() {
  if (loadingTimeout) clearTimeout(loadingTimeout);
  loadingTimeout = setTimeout(() => {
    if (chat.isLoading) {
      chat.isLoading = false;
      appendAssistantOnce("⚠️ 响应超时，请重试", thinkingContent.value || undefined);
      thinkingContent.value = "";
      streamingAnswer.value = "";
      isThinkingCollapsed.value = true;
      pendingConfirm.value = null;
      pet.setState("confused");
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
  unlistenDragDrop = await currentWindow.onDragDropEvent((event) => {
    handleDragDropEvent(event.payload);
  });

  unlistenThinking = await listen<string>("ai-thinking", (event) => {
    chat.isLoading = true;
    resetLoadingTimeout(); // 重置超时
    thinkingContent.value += event.payload;
    scrollToBottomAfterRender();
  });

  unlistenAnswerDelta = await listen<AnswerDeltaPayload>("ai-answer-delta", (event) => {
    chat.isLoading = true;
    resetLoadingTimeout(); // 重置超时
    streamingAnswer.value += event.payload.text;
    scrollToBottomAfterRender();
  });

  unlistenAiFinished = await listen<AiFinishedPayload>("ai-finished", (event) => {
    clearLoadingTimeout(); // 清除超时
    const name = event.payload.agentName || (event.payload as any).agent_name;
    const id = event.payload.agentId || (event.payload as any).agent_id;
    const avatar = event.payload.agentAvatar || (event.payload as any).agent_avatar;
    const replyAgent: MessageAgent | null = name
      ? {
          id: id || undefined,
          name,
          avatar: avatar || undefined,
        }
      : null;
    appendAssistantOnce(event.payload.text, event.payload.thinking ?? undefined, replyAgent);
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    isThinkingCollapsed.value = true;
    pendingConfirm.value = null;
    pet.setState("speaking");
    pet.updateMood({ happiness: 0.05 });

    setTimeout(() => {
      if (pet.state === "speaking") pet.setState("idle");
    }, 3000);
  });

  unlistenAiError = await listen<AiErrorPayload>("ai-error", (event) => {
    clearLoadingTimeout(); // 清除超时
    // 区分可恢复中断和致命错误
    const hadContent = !!streamingAnswer.value || !!thinkingContent.value;
    let text: string;
    if (event.payload.aborted) {
      text = "已中止";
    } else if (hadContent) {
      // 有部分内容时，显示警告而非错误
      text = `⚠️ ${event.payload.message}`;
    } else {
      text = `出错了: ${event.payload.message}`;
    }
    appendAssistantOnce(text, event.payload.thinking ?? (thinkingContent.value || undefined));
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    isThinkingCollapsed.value = true;
    pendingConfirm.value = null;
    // 有部分内容时保持 idle，无内容时才 confused
    pet.setState(event.payload.aborted ? "idle" : (hadContent ? "idle" : "confused"));
  });

  unlistenChatCleared = await listen("chat-history-cleared", () => {
    resetChatUi();
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
      pendingConfirm.value = null;  // Clear any pending tool confirmation
      thinkingContent.value = "";
      streamingAnswer.value = "";
      isThinkingCollapsed.value = false;
      pet.setState("thinking");
    }
  });

  unlistenToolConfirm = await listen<ToolConfirmPayload>("ai-tool-confirm", (event) => {
    pendingConfirm.value = event.payload;
    isThinkingCollapsed.value = false;
    scrollToBottomAfterRender();
  });

  unlistenToolConfirmResolved = await listen<{ id: string; approved: boolean }>("ai-tool-confirm-resolved", (event) => {
    if (pendingConfirm.value?.id === event.payload.id) {
      pendingConfirm.value = null;
    }
  });

  // ask_user 兜底: 小窗不做完整弹窗, 提示用户到控制台作答
  unlistenAskUser = await listen<{ id: string; question: string }>("ai-ask-user", (event) => {
    chat.addMessage(
      "assistant",
      `❓ ${event.payload.question}\n（智能体在等你的回答，请打开控制台对话页作答；不作答 5 分钟后它会自行继续。）`,
    );
    scrollToBottomAfterRender();
  });

  unlistenScheduledTaskTriggered = await listen<{ message: string }>("scheduled-task-triggered", () => {
    pet.setState("happy");
    scrollToBottomAfterRender();
    setTimeout(() => {
      if (pet.state === "happy") pet.setState("idle");
    }, 2600);
  });

  try {
    void skillsStore.load().catch(() => {});
    void agentsStore.load().catch(() => {});
    await loadVoiceSettings();
    await loadApiState();
    await refreshChatState();
    if (!pet.userAvatar) {
      pet.userAvatar = await invoke<string>("get_user_avatar");
    }
    await refreshClipboardItems();
  } catch {
    // Ignore initialization failures; the chat can still open.
  }

  window.addEventListener("storage", handleStorageChange);
  window.addEventListener("voice-settings-changed", handleVoiceSettingsChanged);
  unlistenVoiceChanged = await listen("voice-settings-changed", () => {
    void loadVoiceSettings();
  });
  unlistenApiConfigChanged = await listen("api-config-changed", () => {
    void loadApiState();
  });
  unlistenAgentsChanged = await listen("agents-changed", () => {
    void agentsStore.load().catch(() => {});
  });
  unlistenSkillsChanged = await listen("skills-changed", () => {
    void skillsStore.load().catch(() => {});
  });
  unlistenChatBg = await listen<{ bg?: string; customImage?: string }>("chat-bg-changed", (event) => {
    if (event.payload?.bg !== undefined) {
      chatBg.value = event.payload.bg;
    } else {
      chatBg.value = localStorage.getItem(BG_KEY) || "none";
    }
    if (event.payload?.customImage !== undefined) {
      customBgImage.value = event.payload.customImage;
    } else {
      customBgImage.value = localStorage.getItem(CUSTOM_BG_KEY) || "";
    }
  });
  unlistenAppearanceChanged = await listen("appearance-changed", () => {
    chatBg.value = localStorage.getItem(BG_KEY) || "none";
    customBgImage.value = localStorage.getItem(CUSTOM_BG_KEY) || "";
  });
  await scrollToBottomAfterRender();
});

onBeforeUnmount(() => {
  pendingConfirm.value = null;
  if (scrollFrame !== null) {
    cancelAnimationFrame(scrollFrame);
    scrollFrame = null;
  }
  scrollTimers.forEach(clearTimeout);
  scrollTimers = [];
  if (unlistenDragDrop) {
    unlistenDragDrop();
    unlistenDragDrop = null;
  }
  if (unlistenThinking) {
    unlistenThinking();
    unlistenThinking = null;
  }
  if (unlistenAnswerDelta) {
    unlistenAnswerDelta();
    unlistenAnswerDelta = null;
  }
  if (unlistenAiFinished) {
    unlistenAiFinished();
    unlistenAiFinished = null;
  }
  if (unlistenAiError) {
    unlistenAiError();
    unlistenAiError = null;
  }
  if (unlistenToolConfirm) {
    unlistenToolConfirm();
    unlistenToolConfirm = null;
  }
  if (unlistenToolConfirmResolved) {
    unlistenToolConfirmResolved();
    unlistenToolConfirmResolved = null;
  }
  if (unlistenAskUser) {
    unlistenAskUser();
    unlistenAskUser = null;
  }
  if (unlistenChatCleared) {
    unlistenChatCleared();
    unlistenChatCleared = null;
  }
  if (unlistenSyncMessage) {
    unlistenSyncMessage();
    unlistenSyncMessage = null;
  }
  if (unlistenVoiceChanged) {
    unlistenVoiceChanged();
    unlistenVoiceChanged = null;
  }
  if (unlistenApiConfigChanged) {
    unlistenApiConfigChanged();
    unlistenApiConfigChanged = null;
  }
  if (unlistenScheduledTaskTriggered) {
    unlistenScheduledTaskTriggered();
    unlistenScheduledTaskTriggered = null;
  }
  if (unlistenAgentsChanged) {
    unlistenAgentsChanged();
    unlistenAgentsChanged = null;
  }
  if (unlistenSkillsChanged) {
    unlistenSkillsChanged();
    unlistenSkillsChanged = null;
  }
  if (unlistenChatBg) {
    unlistenChatBg();
    unlistenChatBg = null;
  }
  if (unlistenAppearanceChanged) {
    unlistenAppearanceChanged();
    unlistenAppearanceChanged = null;
  }
  if (fileErrorTimer) {
    clearTimeout(fileErrorTimer);
    fileErrorTimer = null;
  }
  window.removeEventListener("storage", handleStorageChange);
  window.removeEventListener("voice-settings-changed", handleVoiceSettingsChanged);
});

watch(chatBg, () => {
  bgCanvasRef.value?.clearPointer();
});

function handleStorageChange(e: StorageEvent) {
  if (e.key === BG_KEY) {
    chatBg.value = e.newValue || "none";
  } else if (e.key === CUSTOM_BG_KEY) {
    customBgImage.value = e.newValue || "";
  }
}

function handleVoiceSettingsChanged() {
  void loadVoiceSettings();
}


watch(
  () => [
    activeTab.value,
    chat.messages.length,
    chat.isLoading,
    clipboardItems.value.length,
    clipboardSearch.value,
  ],
  () => {
    scrollToBottomAfterRender();
  },
  { flush: "post" },
);

watch(
  () => [slashQuery.value, slashItems.value.length],
  () => {
    slashSelectedIndex.value = 0;
  },
);

watch(
  () => [mentionQuery.value?.query ?? "", mentionItems.value.length],
  () => {
    mentionSelectedIndex.value = 0;
  },
);

async function sendMessage() {
  const text = input.value.trim();
  const files = [...pendingFiles.value];

  if (!text && files.length === 0) return;

  if (activeTab.value === "clipboard") {
    await saveClipboardText(text);
    return;
  }
  if (chat.isLoading) return;
  closeMessageMenu();

  const fullMessage = buildMessageWithFiles(text, files);

  const fileAttachments: FileAttachment[] | undefined =
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

  // Broadcast user message to other windows (like the dashboard)
  await currentWindow.emit("sync-chat-message", {
    role: "user",
    content: displayText,
    fileAttachments: fileAttachments
  });

  input.value = "";
  clearPendingFiles();
  chat.isLoading = true;
  resetLoadingTimeout(); // 启动超时保底
  pendingConfirm.value = null;  // Clear any pending tool confirmation
  thinkingContent.value = "";
  isThinkingCollapsed.value = false;
  pet.setState("thinking");

  await nextTick();
  scrollToBottom();

  try {
    await invoke<{ started: boolean }>("send_to_ai", {
      message: fullMessage,
      attachments: aiAttachments,
      mentionAgents: petMentionNames(text),
    });
  } catch (err) {
    chat.isLoading = false;
    isThinkingCollapsed.value = true;
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

function moveSlashSelection(delta: number) {
  const total = slashItems.value.length;
  if (total === 0) return;
  slashSelectedIndex.value = (slashSelectedIndex.value + delta + total) % total;
}

async function chooseSlashItem(item = slashItems.value[slashSelectedIndex.value]) {
  if (!item) return;
  if (item.type === "skill") {
    await activateSlashSkill(item.skill);
  } else {
    await runSlashCommand(item.command);
  }
}

async function activateSlashSkill(skill: Skill) {
  try {
    await skillsStore.setActive(skill.id);
    input.value = `请使用「${skill.name || "这个"}」技能：`;
    slashSelectedIndex.value = 0;
    pet.updateMood({ happiness: 0.02 });
    await nextTick();
    inputRef.value?.focus();
  } catch (err) {
    appendAssistantOnce(`切换技能失败：${err}`);
    pet.setState("confused");
  }
}

async function runSlashCommand(command: SlashCommand) {
  input.value = "";
  slashSelectedIndex.value = 0;

  switch (command.id) {
    case "clear-skill":
      try {
        await skillsStore.setActive(null);
        appendAssistantOnce("已清除当前激活的 skill。");
      } catch (err) {
        appendAssistantOnce(`清除技能失败：${err}`);
      }
      break;
    case "new-chat":
      await startNewConversation();
      break;
    case "clear-chat":
      await clearChat();
      break;
    case "skills":
      await openSkillsSettingsPanel();
      break;
    case "agents":
      await openAgentsWorkshop();
      break;
    case "evolution":
      await openEvolutionCenter();
      break;
    case "team":
      input.value = "/team ";
      break;
    case "chat":
      switchTab("chat");
      break;
    case "clipboard":
      switchTab("clipboard");
      break;
  }

  await nextTick();
  inputRef.value?.focus();
}

async function openAgentsWorkshop() {
  const mainWin = await WebviewWindow.getByLabel("main");
  if (mainWin) {
    await mainWin.show();
    await mainWin.setFocus();
    await mainWin.emit("navigate-to-page", "agents");
  }
}

async function openEvolutionCenter() {
  const mainWin = await WebviewWindow.getByLabel("main");
  if (mainWin) {
    await mainWin.show();
    await mainWin.setFocus();
    await mainWin.emit("navigate-to-page", "evolution");
  }
}

async function openSkillsSettingsPanel() {
  const size = new LogicalSize(380, 460);
  const pos = await currentWindow.outerPosition().catch(() => null);
  const scale = await currentWindow.scaleFactor().catch(() => window.devicePixelRatio || 1);
  const x = pos ? Math.round(pos.x / scale + 16) : 220;
  const y = pos ? Math.round(pos.y / scale) : 180;
  const position = new LogicalPosition(x, y);
  const existing = await WebviewWindow.getByLabel("settings");

  if (existing) {
    await existing.setSize(size);
    await existing.setPosition(position);
    await existing.setAlwaysOnTop(true);
    await existing.show();
    await existing.setFocus();
    await existing.emit("switch-tab", "system");
    return;
  }

  const baseUrl = window.location.href.split("#")[0].split("?")[0];
  new WebviewWindow("settings", {
    url: `${baseUrl}?window=settings&tab=system`,
    x: position.x,
    y: position.y,
    width: size.width,
    height: size.height,
    title: "AI Desktop Pet Settings",
    transparent: true,
    decorations: false,
    alwaysOnTop: true,
    resizable: false,
    skipTaskbar: true,
    shadow: false,
    focus: true,
    parent: "pet",
  });
}

function openMessageMenu(msg: Message) {
  activeMessageMenuId.value = activeMessageMenuId.value === msg.id ? null : msg.id;
}

function closeMessageMenu() {
  activeMessageMenuId.value = null;
}

async function copyMessage(msg: Message) {
  try {
    await navigator.clipboard.writeText(msg.content);
    pet.updateMood({ happiness: 0.01 });
  } catch (err) {
    appendAssistantOnce(`复制失败: ${err}`);
  } finally {
    closeMessageMenu();
  }
}

async function resendMessage(msg: Message) {
  const text = msg.content.trim();
  if (!text || chat.isLoading) return;
  closeMessageMenu();
  activeTab.value = "chat";
  localStorage.setItem(ACTIVE_TAB_KEY, "chat");
  input.value = text;
  await nextTick();
  await sendMessage();
}

async function toggleVoiceInput() {
  const mainWindow = await WebviewWindow.getByLabel("main");
  await (mainWindow ?? currentWindow).emit("open-voice-panel");
}

async function abortAi() {
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  pendingConfirm.value = null;  // Clear pending tool confirmation on abort
  try {
    await invoke("abort_ai");
  } catch {
    // Ignore abort failures.
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

async function handleToolConfirm(approved: boolean) {
  if (!pendingConfirm.value) return;
  try {
    await invoke("confirm_tool", {
      id: pendingConfirm.value.id,
      approved,
    });
  } catch (e) {
    console.error("确认工具失败:", e);
  } finally {
    pendingConfirm.value = null;
  }
}

async function loadVoiceSettings() {
  try {
    voiceSettings.value = parseVoiceSettings(await invoke<string>("get_setting_value", {
      key: VOICE_SETTINGS_KEY,
    }));
  } catch {
    voiceSettings.value = { ...DEFAULT_VOICE_SETTINGS };
  }
}

async function loadApiState() {
  try {
    backendType.value = await invoke<string>("get_backend_type");
    const config = await invoke<ApiProfile>("get_api_config");
    currentApiProfile.value = config;
    const result = await invoke<{ active_id: string; profiles: ApiProfile[] }>("list_api_profiles");
    apiProfiles.value = result.profiles || [];
    activeApiProfileId.value = result.active_id || config.id || "";
  } catch {
    apiProfiles.value = [];
  }
}

function toggleThinking(msgId: number) {
  if (expandedThinking.has(msgId)) {
    expandedThinking.delete(msgId);
  } else {
    expandedThinking.add(msgId);
  }
}

async function refreshClipboardItems() {
  clipboardItems.value = normalizeClipboardItems(
    await invoke<Array<ClipboardItem | string>>("get_clipboard_items"),
  );
}

async function refreshChatState() {
  const [history, active] = await Promise.all([
    invoke<ChatHistoryItem[]>("get_chat_history"),
    invoke<ActiveChat | null>("get_active_chat"),
  ]);

  chat.setMessages(history.map(normalizeChatHistoryItem));

  if (active) {
    if (!chat.messages.some((msg) => msg.role === "user" && msg.content === active.message)) {
      chat.addMessage("user", active.message);
    }
    chat.isLoading = true;
    thinkingContent.value = active.thinking || "";
    isThinkingCollapsed.value = false;
    pet.setState("thinking");
  } else {
    chat.isLoading = false;
    thinkingContent.value = "";
    isThinkingCollapsed.value = true;
  }

  await scrollToBottomAfterRender();
}

function normalizeChatHistoryItem(item: ChatHistoryItem) {
  return {
    role: item.role === "assistant" ? "assistant" as const : "user" as const,
    content: item.content,
    thinking: item.thinking || undefined,
    timestamp: getChatTime(item.created_at),
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
  };
}

function appendAssistantOnce(content: string, thinking?: string, agent?: MessageAgent | null) {
  const last = chat.messages[chat.messages.length - 1];
  if (last?.role === "assistant" && last.content === content) return;
  chat.addMessage("assistant", content, thinking, undefined, undefined, undefined, agent);
  scrollToBottomAfterRender();
}

async function saveClipboardText(text: string) {
  try {
    await invoke("save_clipboard_item", { content: text });
    await refreshClipboardItems();
    input.value = "";
    pet.updateMood({ happiness: 0.02 });
    await scrollToBottomAfterRender();
  } catch (err) {
    chat.addMessage("assistant", `剪切板保存失败: ${err}`);
    activeTab.value = "chat";
    pet.setState("confused");
  }
}

async function copyClipboardItem(item: ClipboardItem) {
  try {
    await navigator.clipboard.writeText(item.content);
    pet.updateMood({ happiness: 0.01 });
  } catch (err) {
    chat.addMessage("assistant", `剪切板复制失败: ${err}`);
    activeTab.value = "chat";
  }
}

async function deleteClipboardItem(item: ClipboardItem) {
  try {
    await invoke("delete_clipboard_item", { id: item.id });
    clipboardItems.value = clipboardItems.value.filter((current) => current.id !== item.id);
  } catch (err) {
    chat.addMessage("assistant", `剪切板删除失败: ${err}`);
    activeTab.value = "chat";
  }
}

async function toggleClipboardPinned(item: ClipboardItem) {
  const pinned = !item.pinned;
  try {
    await invoke("set_clipboard_item_pinned", { id: item.id, pinned });
    clipboardItems.value = clipboardItems.value.map((current) =>
      current.id === item.id ? { ...current, pinned } : current,
    );
  } catch (err) {
    chat.addMessage("assistant", `剪切板置顶失败: ${err}`);
    activeTab.value = "chat";
  }
}

async function clearClipboard() {
  if (clipboardItems.value.length === 0) return;
  const confirmed = window.confirm("确定要清空全部剪切板暂存内容吗？此操作不可撤销。");
  if (!confirmed) return;

  try {
    await invoke("clear_clipboard_items");
    clipboardItems.value = [];
    clipboardSearch.value = "";
  } catch (err) {
    chat.addMessage("assistant", `剪切板清空失败: ${err}`);
    activeTab.value = "chat";
  }
}

function resetChatUi() {
  chat.clearMessages();
  chat.isLoading = false;
  thinkingContent.value = "";
  streamingAnswer.value = "";
  isThinkingCollapsed.value = true;
  pendingConfirm.value = null;
}

async function startNewConversation() {
  try {
    await invoke("start_new_conversation");
    // 新对话：保留聊天记录显示，仅重置 AI 状态
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    isThinkingCollapsed.value = true;
    pendingConfirm.value = null;
    // 插入分割线，标记新对话开始
    if (chat.messages.length > 0) {
      chat.addSystemMessage("新对话");
    }
    await scrollToBottomAfterRender();
  } catch (err) {
    chat.addMessage("assistant", `新对话创建失败: ${err}`);
  }
}

async function clearChat() {
  const confirmed = window.confirm("确定要清空所有聊天记录吗？此操作不可撤销。");
  if (!confirmed) return;

  try {
    await invoke("clear_chat_history");
    resetChatUi();
    await currentWindow.emit("chat-history-cleared");
  } catch (err) {
    chat.addMessage("assistant", `对话清空失败: ${err}`);
  }
}

function scrollToBottom() {
  const container = messagesRef.value;
  if (!container) return;

  if (activeTab.value === "chat") {
    container.scrollTop = container.scrollHeight;
    chatEndRef.value?.scrollIntoView({ block: "end" });
  } else {
    container.scrollTop = 0;
  }
}

function handleDragDropEvent(event: DragDropEvent) {
  if (event.type === "enter" || event.type === "over") {
    isFileOver.value = isPositionInsideBubble(event.position);
    return;
  }
  isFileOver.value = false;
  if (event.type === "drop" && event.paths.length > 0 && isPositionInsideBubble(event.position)) {
    void addToPendingFiles(event.paths);
  }
}

function isPositionInsideBubble(position: { x: number; y: number }) {
  const rect = chatBubbleRef.value?.getBoundingClientRect();
  if (!rect) return false;
  const scale = window.devicePixelRatio || 1;
  const x = position.x / scale;
  const y = position.y / scale;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

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

async function scrollToBottomAfterRender() {
  await nextTick();
  scrollToBottom();
  if (scrollFrame !== null) {
    cancelAnimationFrame(scrollFrame);
  }
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = null;
    scrollToBottom();
  });
  scrollTimers.forEach(clearTimeout);
  scrollTimers = [80, 220, 420].map((delay) =>
    setTimeout(() => {
      scrollToBottom();
    }, delay),
  );
}

function onBgMouseMove(e: MouseEvent) {
  if (bgCanvasRef.value) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    bgCanvasRef.value.onMouseMove(e.clientX - rect.left, e.clientY - rect.top);
  }
}

function clearBgPointer() {
  bgCanvasRef.value?.clearPointer();
}

function onBgClick(e: MouseEvent) {
  closeMessageMenu();
  if (bgCanvasRef.value) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    bgCanvasRef.value.onClick(e.clientX - rect.left, e.clientY - rect.top);
  }
}

function moveMentionSelection(delta: number) {
  const total = mentionItems.value.length;
  if (total === 0) return;
  mentionSelectedIndex.value = (mentionSelectedIndex.value + delta + total) % total;
}

/** 选中一个智能体：把 @查询 替换成 @名字 并继续输入 */
function chooseMentionItem(agent = mentionItems.value[mentionSelectedIndex.value]) {
  const query = mentionQuery.value;
  if (!agent || !query) return;
  input.value = insertMentionAt(input.value, query, agent.name);
  mentionSelectedIndex.value = 0;
  void nextTick(() => inputRef.value?.focus());
}

function onKeyDown(e: KeyboardEvent) {
  if (showMentionMenu.value) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveMentionSelection(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveMentionSelection(-1);
      return;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      void chooseMentionItem();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      mentionDismissed.value = true;
      mentionSelectedIndex.value = 0;
      return;
    }
  }

  if (showSlashMenu.value) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveSlashSelection(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveSlashSelection(-1);
      return;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      void chooseSlashItem();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      input.value = "";
      slashSelectedIndex.value = 0;
      return;
    }
  }

  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
}

function switchTab(tab: ActiveTab) {
  closeMessageMenu();
  activeTab.value = tab;
  localStorage.setItem(ACTIVE_TAB_KEY, tab);
  scrollToBottomAfterRender();
}

function loadActiveTab(): ActiveTab {
  return localStorage.getItem(ACTIVE_TAB_KEY) === "clipboard" ? "clipboard" : "chat";
}

function normalizeClipboardItems(items: Array<ClipboardItem | string>): ClipboardItem[] {
  return items.map((item, index) => {
    if (typeof item !== "string") {
      return {
        id: item.id,
        content: item.content,
        pinned: Boolean(item.pinned),
        created_at: item.created_at,
      };
    }

    return {
      id: `legacy-${index}-${item}`,
      content: item,
      pinned: false,
      created_at: Date.now() - index,
    };
  });
}

function getClipboardTime(value: string | number): number {
  const rawTime = typeof value === "number" ? value : new Date(value).getTime();
  const time = rawTime > 0 && rawTime < 10_000_000_000 ? rawTime * 1000 : rawTime;
  return Number.isFinite(time) ? time : 0;
}

function getChatTime(value: string | number | undefined): number {
  if (value === undefined || value === null) return Date.now();
  return getClipboardTime(value);
}

function formatTime(ts: number): string {
  const d = new Date(ts);
  return `${d.getHours().toString().padStart(2, "0")}:${d.getMinutes().toString().padStart(2, "0")}`;
}

function formatClipboardTime(value: string | number): string {
  const time = getClipboardTime(value);
  return time > 0 ? formatTime(time) : "";
}
</script>

<template>
  <div
    ref="chatBubbleRef"
    class="chat-bubble"
    :class="{ 'file-over': isFileOver }"
    @mousedown.stop
  >
    <div class="header-decor"></div>

    <div class="chat-header">
      <div class="tabs">
        <button
          :class="['tab-btn', { active: activeTab === 'chat' }]"
          @click="switchTab('chat')"
        >
          <span class="tab-icon"><AppIcon name="chat" :size="13" /></span>
          <span>对话</span>
        </button>
        <button
          :class="['tab-btn', { active: activeTab === 'clipboard' }]"
          @click="switchTab('clipboard')"
        >
          <span class="tab-icon"><AppIcon name="clipboard" :size="13" /></span>
          <span>剪切板</span>
        </button>
      </div>
      <div class="header-actions">
        <button
          v-if="activeTab === 'chat'"
          class="header-action-btn"
          title="新对话"
          @click="startNewConversation"
        >
          <AppIcon name="plus" :size="13" />
        </button>
        <button
          v-if="activeTab === 'chat'"
          class="header-action-btn"
          title="清空对话"
          @click="clearChat"
        >
          <AppIcon name="trash" :size="13" />
        </button>
        <button
          v-else
          class="header-action-btn"
          title="清空剪切板"
          @click="clearClipboard"
        >
          <AppIcon name="trash" :size="13" />
        </button>
        <button class="close-btn" title="关闭" @click="emit('close')">
          <AppIcon name="x" :size="13" />
        </button>
      </div>
    </div>

    <div
      class="messages-area"
      @mousemove.passive="onBgMouseMove"
      @mouseleave="clearBgPointer"
      @blur.capture="clearBgPointer"
      @click.self="onBgClick"
    >
      <BgCanvas
        v-if="chatBg !== 'none'"
        ref="bgCanvasRef"
        :mode="chatBg"
        :custom-image="customBgImage"
      />
      <div ref="messagesRef" class="chat-messages" @click="onBgClick">
      <template v-if="activeTab === 'chat'">
        <div v-if="chat.messages.length === 0 && !chat.isLoading" class="empty-hint">
          <div class="empty-icon"><AppIcon name="paw" :size="26" /></div>
          <div>点击输入框，和它说点什么</div>
        </div>

        <div
          v-for="(msg, idx) in chat.messages"
          :key="msg.id"
          :class="['message', msg.role, { 'msg-enter': idx === chat.messages.length - 1 }]"
          @contextmenu.prevent.stop="msg.role !== 'system' && openMessageMenu(msg)"
        >
          <!-- 系统分割线 -->
          <div v-if="msg.role === 'system'" class="system-divider">
            <span class="system-divider-line"></span>
            <span class="system-divider-text">{{ msg.content }}</span>
            <span class="system-divider-line"></span>
          </div>

          <template v-else>
          <div v-if="msg.role === 'assistant'" class="avatar bot-avatar">
            <AppIcon name="paw" :size="13" />
          </div>

          <div class="msg-body">
            <div v-if="msg.agent && msg.role === 'assistant'" class="msg-agent-badge">
              <span class="msg-agent-avatar">{{ msg.agent.avatar || "🤖" }}</span>
              <span class="msg-agent-name">{{ msg.agent.name }}</span>
            </div>
            <div v-if="msg.thinking" class="thinking-section">
              <div class="thinking-header" @click="toggleThinking(msg.id)">
                <span class="thinking-toggle">{{ expandedThinking.has(msg.id) ? '&#x25BE;' : '&#x25B8;' }}</span>
                <span class="thinking-label">思考过程</span>
              </div>
              <Transition name="thinking-expand">
                <div v-if="expandedThinking.has(msg.id)" class="thinking-content">
                  {{ msg.thinking }}
                </div>
              </Transition>
            </div>

            <div class="bubble">
              <template v-for="(ablock, bi) in parseAgentHeaders(msg.content, agentsStore.agents)" :key="bi">
                <div v-if="ablock.type === 'agent_header'" class="bubble-msg-agent-section-header">
                  <span class="bubble-msg-agent-avatar">{{ ablock.avatar }}</span>
                  <span class="bubble-msg-agent-name">{{ ablock.agentName }}</span>
                </div>
                <template v-else>
                  <template v-for="(mseg, mi) in splitMentionSegments(ablock.content, knownAgentNames)" :key="bi + '-' + mi">
                    <span v-if="mseg.type === 'mention'" class="bubble-mention">@{{ mseg.content }}</span>
                    <template v-else>{{ mseg.content }}</template>
                  </template>
                </template>
              </template>
            </div>
            <div v-if="msg.files && msg.files.length > 0" class="msg-files">
              <div v-for="(file, fi) in msg.files" :key="fi" class="msg-file-tag">
                <span>{{ file.isImage ? "\u{1F4F7}" : getFileIcon(file.extension) }}</span>
                <span>{{ file.name }}</span>
              </div>
            </div>
            <div class="msg-time">{{ formatTime(msg.timestamp) }}</div>
            <div
              v-if="activeMessageMenuId === msg.id"
              class="message-action-menu"
              @click.stop
              @contextmenu.prevent.stop
            >
              <button type="button" @click="copyMessage(msg)">复制</button>
              <button type="button" :disabled="chat.isLoading" @click="resendMessage(msg)">重新发送</button>
            </div>
          </div>

          <div v-if="msg.role === 'user'" class="avatar user-avatar">
            <img v-if="userAvatar" :src="userAvatar" alt="用户头像" />
            <AppIcon v-else name="user" :size="13" />
          </div>
          </template>
        </div>

        <div v-if="chat.isLoading" class="message assistant msg-enter">
          <div class="avatar bot-avatar">
            <span class="avatar-thinking"><AppIcon name="paw" :size="13" /></span>
          </div>
          <div class="msg-body">
            <div class="bubble thinking-bubble">
              <div class="thinking-header" @click="isThinkingCollapsed = !isThinkingCollapsed">
                <span class="thinking-toggle">{{ isThinkingCollapsed ? '&#x25B8;' : '&#x25BE;' }}</span>
                <span class="thinking-label thinking-loading">思考中</span>
                <span class="thinking-dots"><span>.</span><span>.</span><span>.</span></span>
                <button class="stop-btn" title="停止思考" @click.stop="abortAi">
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
                    <rect width="10" height="10" rx="2"/>
                  </svg>
                  <span>停止</span>
                </button>
              </div>
              <Transition name="thinking-expand">
                <div v-if="!isThinkingCollapsed && thinkingContent" class="thinking-content thinking-streaming">
                  {{ thinkingContent }}
                </div>
              </Transition>
              <div v-if="!isThinkingCollapsed && !thinkingContent" class="thinking-content thinking-waiting">
                等待思考输出...
              </div>
            </div>
            <div v-if="streamingAnswer" class="bubble bot-bubble streaming-answer">
              <template v-for="(ablock, bi) in parseAgentHeaders(streamingAnswer, agentsStore.agents)" :key="bi">
                <div v-if="ablock.type === 'agent_header'" class="bubble-msg-agent-section-header">
                  <span class="bubble-msg-agent-avatar">{{ ablock.avatar }}</span>
                  <span class="bubble-msg-agent-name">{{ ablock.agentName }}</span>
                </div>
                <template v-else>{{ ablock.content }}</template>
              </template>
            </div>


          </div>
        </div>
        <div ref="chatEndRef" class="chat-end" aria-hidden="true"></div>
      </template>

      <template v-else>
        <div class="clipboard-search">
          <span class="clipboard-search-icon"><AppIcon name="search" :size="12" /></span>
          <input v-model="clipboardSearch" type="search" placeholder="搜索剪切板..." />
        </div>

        <div v-if="clipboardItems.length === 0" class="empty-hint">
          <div class="empty-icon"><AppIcon name="clipboard" :size="26" /></div>
          <div>把临时内容存在这里</div>
        </div>
        <div v-else-if="filteredClipboardItems.length === 0" class="empty-hint">
          <div class="empty-icon"><AppIcon name="search" :size="26" /></div>
          <div>没有匹配的剪切板内容</div>
        </div>
        <div
          v-for="item in filteredClipboardItems"
          v-else
          :key="item.id"
          class="clipboard-item"
          :class="{ pinned: item.pinned }"
        >
          <div class="clipboard-item-main">
            <div class="bubble clipboard-bubble">{{ item.content }}</div>
            <div class="clipboard-footer">
              <div class="clipboard-meta">
                <span v-if="item.pinned" class="pin-label">置顶</span>
                <span>{{ formatClipboardTime(item.created_at) }}</span>
              </div>
              <div class="clipboard-actions">
                <button
                  type="button"
                  class="clipboard-action-btn"
                  :class="{ active: item.pinned }"
                  :title="item.pinned ? '取消置顶' : '置顶'"
                  :aria-label="item.pinned ? '取消置顶' : '置顶'"
                  @click="toggleClipboardPinned(item)"
                >
                  <span class="clipboard-action-icon"><AppIcon name="pin" :size="11" /></span>
                  <span class="clipboard-action-label">{{ item.pinned ? '取消置顶' : '置顶' }}</span>
                </button>
                <button
                  type="button"
                  class="clipboard-action-btn"
                  title="复制"
                  aria-label="复制"
                  @click="copyClipboardItem(item)"
                >
                  <span class="clipboard-action-icon"><AppIcon name="copy" :size="11" /></span>
                  <span class="clipboard-action-label">复制</span>
                </button>
                <button
                  type="button"
                  class="clipboard-action-btn danger"
                  title="删除"
                  aria-label="删除"
                  @click="deleteClipboardItem(item)"
                >
                  <span class="clipboard-action-icon"><AppIcon name="trash" :size="11" /></span>
                  <span class="clipboard-action-label">删除</span>
                </button>
              </div>
            </div>
          </div>
        </div>
      </template>
      </div>
    </div>

    <Transition name="fade">
      <div v-if="fileValidationError" class="file-validation-error">
        {{ fileValidationError }}
      </div>
    </Transition>

    <Transition name="slide-up">
      <div v-if="pendingConfirm" class="floating-confirm-overlay">
        <div class="tool-confirm-card">
          <div class="confirm-card-header">
            <span class="confirm-card-icon"><AppIcon name="zap" :size="13" /></span>
            <span class="confirm-card-title">请求运行敏感操作</span>
          </div>

          <div class="confirm-card-body">
            <div class="confirm-tool-info">
              <span class="info-label">操作类别:</span>
              <span class="info-value">{{ pendingConfirm.summary || pendingConfirm.tool_name }}</span>
            </div>
            <div v-if="pendingConfirm.command" class="confirm-tool-info">
              <span class="info-label">命令:</span>
              <span class="info-value">{{ pendingConfirm.command }}</span>
            </div>
            <div v-if="pendingConfirm.path" class="confirm-tool-info">
              <span class="info-label">路径:</span>
              <span class="info-value">{{ pendingConfirm.path }}</span>
            </div>
            <div v-if="pendingConfirm.arguments" class="confirm-tool-args">
              <span class="info-label">核心参数:</span>
              <pre class="args-code"><code>{{ formatArguments(pendingConfirm.arguments) }}</code></pre>
            </div>
          </div>

          <div class="confirm-card-actions">
            <button class="confirm-btn deny" @click="handleToolConfirm(false)">
              <AppIcon name="x" :size="12" /> 拒绝
            </button>
            <button class="confirm-btn approve" @click="handleToolConfirm(true)">
              <AppIcon name="check" :size="12" /> 允许
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <Transition name="slide-up">
      <div v-if="pendingFiles.length > 0" class="file-preview-area">
        <div class="file-preview-header">
          <span>{{ pendingFiles.length }} 个文件待发送</span>
          <button class="clear-all-btn" @click="clearPendingFiles">全部移除</button>
        </div>
        <div class="file-preview-list">
          <TransitionGroup name="file-item">
            <div
              v-for="file in pendingFiles"
              :key="file.id"
              class="file-preview-item"
              :class="{ error: file.status === 'error' }"
            >
              <template v-if="file.isImage">
                <img
                  v-if="file.previewUrl && file.status !== 'error'"
                  :src="file.previewUrl"
                  class="preview-thumbnail"
                  @error="handleImageError(file)"
                />
                <div v-else class="preview-fallback">
                  <span class="file-icon">{{ getFileIcon(file.extension) }}</span>
                  <span class="fallback-text">预览不可用</span>
                </div>
              </template>
              <template v-else>
                <div class="preview-file-card">
                  <span class="file-icon-lg">{{ getFileIcon(file.extension) }}</span>
                  <span class="file-name" :title="file.name">{{ file.name }}</span>
                  <span class="file-size">{{ formatFileSize(file.size) }}</span>
                </div>
              </template>
              <button class="remove-file-btn" @click="removePendingFile(file.id)">&times;</button>
            </div>
          </TransitionGroup>
        </div>
      </div>
    </Transition>

    <Transition name="slide-up">
      <div v-if="showMentionMenu" class="mention-menu" @mousedown.prevent>
        <button
          v-for="(agent, index) in mentionItems"
          :key="agent.id"
          type="button"
          class="mention-item"
          :class="{ active: index === mentionSelectedIndex }"
          @mouseenter="mentionSelectedIndex = index"
          @click="chooseMentionItem(agent)"
        >
          <span class="mention-avatar">{{ agent.avatar || "🤖" }}</span>
          <span class="mention-main">
            <span class="mention-title">@{{ agent.name }}</span>
            <span class="mention-desc">{{ agent.description || "自定义智能体" }}</span>
          </span>
          <span class="mention-hint">Tab</span>
        </button>
      </div>
    </Transition>

    <Transition name="slide-up">
      <div v-if="showSlashMenu" class="slash-menu" @mousedown.prevent>
        <button
          v-for="(item, index) in slashItems"
          :key="item.key"
          type="button"
          class="slash-item"
          :class="{ active: index === slashSelectedIndex, skill: item.type === 'skill' }"
          @mouseenter="slashSelectedIndex = index"
          @click="chooseSlashItem(item)"
        >
          <span class="slash-item-mark">{{ item.type === "skill" ? "#" : "/" }}</span>
          <span class="slash-item-main">
            <span class="slash-item-title">{{ item.title }}</span>
            <span class="slash-item-desc">{{ item.description }}</span>
          </span>
          <span class="slash-item-hint">{{ item.hint }}</span>
        </button>
      </div>
    </Transition>

    <div class="chat-input">
      <button
        v-if="activeTab === 'chat'"
        class="voice-btn"
        :class="{ active: voiceStatus === 'listening', speaking: voiceStatus === 'speaking' }"
        :disabled="chat.isLoading || !voiceSettings.enabled || !isSpeechRecognitionSupported()"
        :title="voiceButtonTitle"
        :aria-label="voiceButtonTitle"
        @click="toggleVoiceInput"
      >
        <svg v-if="voiceStatus === 'listening'" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="M12 3V21M5 8V16M19 8V16" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
        <svg v-else width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="M12 3C10.3431 3 9 4.34315 9 6V11C9 12.6569 10.3431 14 12 14C13.6569 14 15 12.6569 15 11V6C15 4.34315 13.6569 3 12 3Z" stroke="currentColor" stroke-width="2"/>
          <path d="M5 10V11C5 14.866 8.13401 18 12 18M19 10V11C19 14.866 15.866 18 12 18M12 18V21" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
      </button>
      <input
        ref="inputRef"
        v-model="input"
        :placeholder="activeTab === 'chat' ? (voiceStatus === 'listening' ? '正在听你说话...' : '说点什么...') : '保存到剪切板...'"
        @keydown="onKeyDown"
      />
      <button
        class="send-btn"
        :disabled="(!input.trim() && pendingFiles.length === 0) || (activeTab === 'chat' && chat.isLoading)"
        @click="sendMessage"
      >
        <svg v-if="activeTab === 'chat'" width="18" height="18" viewBox="0 0 24 24" fill="none">
          <path d="M22 2L11 13" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          <path d="M22 2L15 22L11 13L2 9L22 2Z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <svg v-else width="18" height="18" viewBox="0 0 24 24" fill="none">
          <path d="M19 21H5C4.46957 21 3.96086 20.7893 3.58579 20.4142C3.21071 20.0391 3 19.5304 3 19V5C3 4.46957 3.21071 3.96086 3.58579 3.58579C3.96086 3.21071 4.46957 3 5 3H14L20 9V19C20 19.5304 19.7893 20.0391 19.4142 20.4142C19.0391 20.7893 18.5304 21 18 21" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          <path d="M12 17V11M9 14L12 11L15 14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>

    <div v-if="voiceInterimText || voiceError" class="voice-status">
      <span v-if="voiceInterimText">{{ voiceInterimText }}</span>
      <span v-else>{{ voiceError }}</span>
    </div>

    <div v-if="isFileOver" class="drop-hint">
      <div class="drop-icon"><AppIcon name="paperclip" :size="22" /></div>
      <div>松开添加文件</div>
    </div>
  </div>
</template>

<style scoped>
/* ============================================================
   ChatBubble — 暖调工作室 · 悬浮对话窗
   ============================================================ */
.chat-bubble {
  position: absolute;
  left: var(--chat-bubble-left, 0);
  top: var(--chat-bubble-top, 0);
  width: var(--chat-bubble-width, 100%);
  height: var(--chat-bubble-height, 100%);
  box-sizing: border-box;
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 16px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  background:
    radial-gradient(420px 200px at 20% -10%, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.06), transparent 60%),
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
.chat-header {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.tabs {
  flex: 0 0 auto;
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  border-radius: 9px;
  background: var(--dash-panel-soft, #f3f0e9);
}

.tab-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4.5px 11px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 11.5px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
  transition:
    background var(--dash-dur-fast, 140ms) var(--dash-ease-out, ease-out),
    color var(--dash-dur-fast, 140ms) var(--dash-ease-out, ease-out),
    box-shadow var(--dash-dur-fast, 140ms) var(--dash-ease-out, ease-out);
}

.tab-btn:hover {
  color: var(--dash-text-secondary, #6d6558);
}

.tab-btn.active {
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.06));
}

.tab-icon {
  display: inline-flex;
  align-items: center;
}

.tab-btn.active .tab-icon {
  color: var(--dash-accent, #bf7a4e);
}

.header-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}

.header-action-btn,
.close-btn {
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  cursor: pointer;
  transition:
    background var(--dash-dur-fast, 140ms) var(--dash-ease-out, ease-out),
    color var(--dash-dur-fast, 140ms) var(--dash-ease-out, ease-out);
}

.header-action-btn:hover,
.close-btn:hover {
  background: rgba(63, 54, 44, 0.06);
  color: var(--dash-text-primary, #2d2922);
}

.close-btn:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
}

/* ---------- 消息区 ---------- */
.messages-area {
  flex: 1;
  min-height: 0;
  position: relative;
  overflow: hidden;
}

.chat-messages {
  position: relative;
  z-index: 1;
  height: 100%;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  scrollbar-width: thin;
  scrollbar-color: rgba(63, 54, 44, 0.18) transparent;
}

.chat-messages::-webkit-scrollbar {
  width: 6px;
}

.chat-messages::-webkit-scrollbar-thumb {
  background: rgba(63, 54, 44, 0.16);
  border-radius: 6px;
}

.chat-messages::-webkit-scrollbar-track {
  background: transparent;
}

.empty-hint {
  margin: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 30px 16px;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 11.5px;
  text-align: center;
  line-height: 1.7;
}

.empty-icon {
  width: 44px;
  height: 44px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  color: var(--dash-accent, #bf7a4e);
}

/* 消息 */
.message {
  display: flex;
  gap: 7px;
  align-items: flex-start;
  position: relative;
}

.message.user {
  flex-direction: row-reverse;
}

.msg-enter {
  animation: cb-msg-in 0.28s var(--dash-ease-out, ease-out) both;
}

@keyframes cb-msg-in {
  from { opacity: 0; transform: translateY(6px); }
  to   { opacity: 1; transform: translateY(0); }
}

.avatar {
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  margin-top: 2px;
}

.bot-avatar {
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
  color: var(--dash-accent, #bf7a4e);
  border: 1px solid rgba(var(--pet-primary-rgb, 191, 122, 78), 0.22);
}

.user-avatar {
  background: var(--dash-panel-sunken, #edeae2);
  color: var(--dash-text-muted, #a29a8a);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
}

.user-avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.msg-body {
  max-width: calc(100% - 40px);
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
}

.message.user .msg-body {
  align-items: flex-end;
}

.msg-agent-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-bottom: 4px;
  font-size: 10.5px;
  font-weight: 650;
  color: var(--dash-accent, #bf7a4e);
}

.msg-agent-avatar {
  width: 16px;
  height: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
  font-size: 10px;
}

.bubble {
  padding: 8px 12px;
  border-radius: 12px;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.05));
  font-size: 12.5px;
  line-height: 1.7;
  color: var(--pet-font-color, var(--dash-text-primary, #2d2922));
  word-break: break-word;
  white-space: pre-wrap;
}

.message.assistant .bubble {
  border-top-left-radius: 4px;
}

.message.user .bubble {
  background: var(--pet-bubble-user, #bf7a4e);
  border: none;
  color: #fffaf4;
  border-top-right-radius: 4px;
}

.bubble-mention {
  color: var(--dash-accent, #bf7a4e);
  font-weight: 650;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
  border-radius: 4px;
  padding: 0 3px;
}

.message.user .bubble-mention {
  color: #fff;
  background: rgba(255, 255, 255, 0.2);
}

.bubble-msg-agent-section-header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 6px 0 4px;
  padding-bottom: 4px;
  border-bottom: 1px dashed var(--dash-divider, rgba(63, 54, 44, 0.1));
  font-size: 10.5px;
  font-weight: 650;
  color: var(--dash-accent, #bf7a4e);
}

.bubble-msg-agent-avatar {
  width: 16px;
  height: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
  font-size: 10px;
}

.msg-time {
  margin-top: 3px;
  font-size: 9.5px;
  color: var(--dash-text-muted, #a29a8a);
  opacity: 0.8;
}

.msg-files {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 6px;
}

.msg-file-tag {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  border-radius: 999px;
  background: rgba(63, 54, 44, 0.06);
  font-size: 10.5px;
  color: var(--dash-text-secondary, #6d6558);
}

.message.user .msg-file-tag {
  background: rgba(255, 255, 255, 0.2);
  color: rgba(255, 250, 244, 0.95);
}

/* 系统分割线 */
.system-divider {
  align-self: center;
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  margin: 2px 0;
}

.system-divider-line {
  flex: 1;
  height: 1px;
  background: var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.system-divider-text {
  font-size: 9.5px;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--dash-text-muted, #a29a8a);
  white-space: nowrap;
}

/* 思考 */
.thinking-section {
  margin-bottom: 5px;
  width: 100%;
}

.thinking-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-radius: 9px;
  background: var(--dash-panel-soft, #f3f0e9);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.07));
  cursor: pointer;
  user-select: none;
}

.thinking-toggle {
  font-size: 9px;
  color: var(--dash-text-muted, #a29a8a);
  width: 10px;
}

.thinking-label {
  font-size: 10.5px;
  font-weight: 650;
  color: var(--dash-text-secondary, #6d6558);
}

.thinking-loading {
  color: var(--dash-accent, #bf7a4e);
}

.thinking-dots {
  display: inline-flex;
  gap: 2px;
  margin-left: 2px;
}

.thinking-dots span {
  animation: cb-thinking-dot 1.2s ease-in-out infinite;
  font-size: 12px;
  line-height: 1;
  color: var(--dash-accent, #bf7a4e);
}

.thinking-dots span:nth-child(2) { animation-delay: 0.15s; }
.thinking-dots span:nth-child(3) { animation-delay: 0.3s; }

@keyframes cb-thinking-dot {
  0%, 60%, 100% { opacity: 0.25; }
  30%           { opacity: 1; }
}

.stop-btn {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2.5px 8px;
  border-radius: 999px;
  border: 1px solid rgba(192, 90, 77, 0.35);
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
  font-size: 9.5px;
  font-weight: 600;
  cursor: pointer;
  transition:
    background var(--dash-dur-fast, 140ms) ease-out,
    color var(--dash-dur-fast, 140ms) ease-out;
}

.stop-btn:hover {
  background: var(--dash-danger, #c05a4d);
  color: #fff;
}

.thinking-content {
  margin-top: 5px;
  padding: 7px 10px;
  border-left: 2px solid var(--dash-divider, rgba(63, 54, 44, 0.1));
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
  line-height: 1.65;
  white-space: pre-wrap;
  word-break: break-word;
}

.thinking-bubble {
  width: 100%;
  background: var(--dash-panel-soft, #f3f0e9);
}

.thinking-waiting {
  font-style: italic;
}

.streaming-answer {
  margin-top: 6px;
}

/* 思考展开动画 */
.thinking-expand-enter-active,
.thinking-expand-leave-active {
  transition: opacity 0.2s ease-out, transform 0.2s ease-out;
}

.thinking-expand-enter-from,
.thinking-expand-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

/* 消息右键菜单 */
.message-action-menu {
  position: absolute;
  top: calc(100% + 4px);
  z-index: 20;
  display: flex;
  gap: 2px;
  padding: 3px;
  border-radius: 9px;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  box-shadow: var(--dash-shadow-md, 0 8px 28px rgba(48, 42, 34, 0.14));
}

.message.user .message-action-menu {
  right: 0;
}

.message-action-menu button {
  border: none;
  background: transparent;
  padding: 4px 9px;
  border-radius: 6px;
  font-size: 10.5px;
  font-weight: 550;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: background 140ms ease-out, color 140ms ease-out;
}

.message-action-menu button:hover {
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-primary, #2d2922);
}

.message-action-menu button:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.chat-end {
  height: 1px;
}

/* ---------- 剪切板 ---------- */
.clipboard-search {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 7px 11px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  background: var(--dash-panel-solid, #fffefb);
  margin-bottom: 4px;
}

.clipboard-search-icon {
  color: var(--dash-text-muted, #a29a8a);
  display: inline-flex;
}

.clipboard-search input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  font-size: 12px;
  font-family: inherit;
  color: var(--dash-text-primary, #2d2922);
}

.clipboard-search input:focus {
  outline: none;
}

.clipboard-item {
  border-radius: 12px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.04));
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out;
}

.clipboard-item:hover {
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  box-shadow: var(--dash-shadow-sm, 0 4px 14px rgba(48, 42, 34, 0.06));
}

.clipboard-item.pinned {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.4);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.05));
}

.clipboard-bubble {
  border: none;
  background: transparent;
  box-shadow: none;
  border-radius: 0;
  padding: 9px 12px 4px;
  font-size: 12px;
  max-height: 120px;
  overflow: hidden;
}

.clipboard-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 5px 9px 8px 12px;
}

.clipboard-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 9.5px;
  color: var(--dash-text-muted, #a29a8a);
}

.pin-label {
  padding: 0.5px 6px;
  border-radius: 999px;
  background: var(--dash-accent, #bf7a4e);
  color: #fff;
  font-weight: 700;
  font-size: 9px;
}

.clipboard-actions {
  display: flex;
  gap: 3px;
}

.clipboard-action-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 10px;
  font-weight: 600;
  cursor: pointer;
  transition: background 140ms ease-out, color 140ms ease-out;
}

.clipboard-action-btn:hover {
  background: rgba(63, 54, 44, 0.06);
  color: var(--dash-text-primary, #2d2922);
}

.clipboard-action-btn.active {
  color: var(--dash-accent, #bf7a4e);
}

.clipboard-action-btn.danger:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
}

.clipboard-action-icon {
  display: inline-flex;
}

/* ---------- 输入区 ---------- */
.chat-input {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 9px 10px;
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.chat-input input {
  flex: 1;
  min-width: 0;
  padding: 7.5px 12px;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.14));
  background: var(--dash-panel-solid, #fffefb);
  font-size: 12.5px;
  font-family: inherit;
  color: var(--dash-text-primary, #2d2922);
  transition: border-color 140ms ease-out, box-shadow 140ms ease-out;
}

.chat-input input:focus {
  outline: none;
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
}

.chat-input input::placeholder {
  color: var(--dash-text-muted, #a29a8a);
}

.voice-btn {
  width: 32px;
  height: 32px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.14));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: all 140ms ease-out;
}

.voice-btn:hover:not(:disabled) {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.4);
  color: var(--dash-accent, #bf7a4e);
}

.voice-btn.active {
  background: var(--dash-accent, #bf7a4e);
  border-color: transparent;
  color: #fff;
  animation: cb-voice-pulse 1.6s ease-out infinite;
}

.voice-btn.speaking {
  border-color: var(--dash-accent, #bf7a4e);
  color: var(--dash-accent, #bf7a4e);
}

.voice-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

@keyframes cb-voice-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(var(--pet-primary-rgb, 191, 122, 78), 0.35); }
  50%      { box-shadow: 0 0 0 5px rgba(var(--pet-primary-rgb, 191, 122, 78), 0); }
}

.send-btn {
  width: 32px;
  height: 32px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 10px;
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  cursor: pointer;
  box-shadow: 0 2px 8px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.3);
  transition: all 140ms ease-out;
}

.send-btn:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a5643c);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.36);
}

.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  box-shadow: none;
}

.voice-status {
  flex-shrink: 0;
  padding: 5px 14px;
  font-size: 10.5px;
  color: var(--dash-text-secondary, #6d6558);
  background: var(--dash-panel-soft, #f3f0e9);
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.06));
}

/* ---------- 拖放提示 ---------- */
.drop-hint {
  position: absolute;
  inset: 0;
  z-index: 40;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background: rgba(255, 254, 251, 0.88);
  backdrop-filter: blur(6px);
  border-radius: 16px;
  font-size: 12px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.drop-icon {
  width: 46px;
  height: 46px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
  border: 1.5px dashed rgba(var(--pet-primary-rgb, 191, 122, 78), 0.5);
  color: var(--dash-accent, #bf7a4e);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
}

.file-validation-error {
  position: absolute;
  left: 12px;
  right: 12px;
  bottom: 60px;
  z-index: 30;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  border: 1px solid rgba(192, 90, 77, 0.3);
  color: var(--dash-danger, #c05a4d);
  font-size: 11px;
}

/* ---------- 文件预览 ---------- */
.file-preview-area {
  position: absolute;
  left: 10px;
  right: 10px;
  bottom: 58px;
  z-index: 25;
  border-radius: 12px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  background: var(--dash-panel-solid, #fffefb);
  box-shadow: var(--dash-shadow-md, 0 12px 32px rgba(48, 42, 34, 0.12));
  overflow: hidden;
}

.file-preview-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 11px;
  font-size: 10.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
  border-bottom: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.07));
}

.clear-all-btn {
  border: none;
  background: transparent;
  color: var(--dash-danger, #c05a4d);
  font-size: 10.5px;
  font-weight: 600;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 5px;
}

.clear-all-btn:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
}

.file-preview-list {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  padding: 9px 11px;
  max-height: 140px;
  overflow-y: auto;
}

.file-preview-item {
  position: relative;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  overflow: hidden;
}

.file-preview-item.error {
  border-color: rgba(192, 90, 77, 0.4);
}

.preview-thumbnail {
  width: 60px;
  height: 60px;
  object-fit: cover;
  display: block;
}

.preview-fallback {
  width: 60px;
  height: 60px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  color: var(--dash-text-muted, #a29a8a);
}

.file-icon {
  font-size: 15px;
}

.fallback-text {
  font-size: 8.5px;
}

.preview-file-card {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 8px 10px;
  max-width: 130px;
}

.file-icon-lg {
  font-size: 15px;
  color: var(--dash-accent, #bf7a4e);
}

.file-name {
  font-size: 10.5px;
  font-weight: 600;
  color: var(--dash-text-primary, #2d2922);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.file-size {
  font-size: 9px;
  color: var(--dash-text-muted, #a29a8a);
}

.remove-file-btn {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: 50%;
  background: rgba(45, 41, 34, 0.62);
  color: #fff;
  font-size: 10px;
  line-height: 1;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  transition: opacity 140ms ease-out;
}

.file-preview-item:hover .remove-file-btn {
  opacity: 1;
}

/* ---------- @ 与 / 菜单 ---------- */
.mention-menu,
.slash-menu {
  position: absolute;
  left: 10px;
  right: 10px;
  bottom: 58px;
  z-index: 25;
  max-height: 220px;
  overflow-y: auto;
  border-radius: 12px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  background: var(--dash-panel-solid, #fffefb);
  box-shadow: var(--dash-shadow-md, 0 12px 32px rgba(48, 42, 34, 0.12));
  padding: 4px;
}

.mention-item,
.slash-item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 7px 9px;
  border: none;
  border-radius: 8px;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background 120ms ease-out;
}

.mention-item.active,
.slash-item.active {
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
}

.mention-avatar,
.slash-item-mark {
  width: 25px;
  height: 25px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  background: var(--dash-panel-soft, #f3f0e9);
  font-size: 12px;
  color: var(--dash-accent, #bf7a4e);
  font-weight: 650;
}

.mention-main,
.slash-item-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.mention-title,
.slash-item-title {
  font-size: 11.5px;
  font-weight: 620;
  color: var(--dash-text-primary, #2d2922);
}

.mention-desc,
.slash-item-desc {
  font-size: 10px;
  color: var(--dash-text-muted, #a29a8a);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mention-hint,
.slash-item-hint {
  font-size: 9.5px;
  color: var(--dash-text-muted, #a29a8a);
  flex-shrink: 0;
  padding: 1.5px 6px;
  border-radius: 5px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
}

/* ---------- 工具确认卡 ---------- */
.floating-confirm-overlay {
  position: absolute;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  background: rgba(45, 41, 34, 0.3);
  backdrop-filter: blur(5px);
  border-radius: 16px;
}

.tool-confirm-card {
  width: 100%;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  border-radius: 14px;
  box-shadow: var(--dash-shadow-lg, 0 24px 56px rgba(48, 42, 34, 0.16));
  padding: 14px 16px;
  animation: cb-msg-in 0.26s var(--dash-ease-spring, ease-out) both;
}

.confirm-card-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

.confirm-card-icon {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.12));
  color: var(--dash-accent, #bf7a4e);
}

.confirm-card-title {
  font-size: 12.5px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
}

.confirm-card-body {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.confirm-tool-info {
  display: flex;
  gap: 8px;
  font-size: 11px;
  line-height: 1.5;
}

.info-label {
  flex-shrink: 0;
  width: 60px;
  color: var(--dash-text-muted, #a29a8a);
  font-weight: 600;
}

.info-value {
  color: var(--dash-text-primary, #2d2922);
  word-break: break-all;
  min-width: 0;
}

.args-code {
  width: 100%;
  box-sizing: border-box;
  margin-top: 4px;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--dash-panel-sunken, #edeae2);
  font-size: 10px;
  font-family: var(--dash-font-mono, Consolas, monospace);
  color: var(--dash-text-secondary, #6d6558);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 120px;
  overflow-y: auto;
}

.confirm-card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 14px;
}

.confirm-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6.5px 15px;
  border-radius: 9px;
  font-size: 11.5px;
  font-weight: 650;
  cursor: pointer;
  transition: all 140ms ease-out;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-secondary, #6d6558);
}

.confirm-btn.deny:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  border-color: rgba(192, 90, 77, 0.35);
  color: var(--dash-danger, #c05a4d);
}

.confirm-btn.approve {
  background: var(--dash-accent, #bf7a4e);
  border-color: transparent;
  color: #fffaf4;
  box-shadow: 0 2px 8px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.3);
}

.confirm-btn.approve:hover {
  background: var(--dash-accent-dark, #a5643c);
}

/* ---------- 过渡 ---------- */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease-out;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.slide-up-enter-active,
.slide-up-leave-active {
  transition: opacity 0.22s ease-out, transform 0.22s ease-out;
}

.slide-up-enter-from,
.slide-up-leave-to {
  opacity: 0;
  transform: translateY(10px);
}

.file-item-enter-active,
.file-item-leave-active {
  transition: opacity 0.2s ease-out, transform 0.2s ease-out;
}

.file-item-enter-from,
.file-item-leave-to {
  opacity: 0;
  transform: scale(0.9);
}
</style>
