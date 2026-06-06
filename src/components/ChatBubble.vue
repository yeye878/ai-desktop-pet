<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useChatStore, type FileAttachment, type Message } from "../stores/chat";
import { usePetStore } from "../stores/pet";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, type DragDropEvent } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { UnlistenFn } from "@tauri-apps/api/event";
import BgCanvas from "./BgCanvas.vue";
import {
  DEFAULT_VOICE_SETTINGS,
  VoiceController,
  isSpeechRecognitionSupported,
  parseVoiceSettings,
  type VoiceSettings,
  type VoiceStatus,
} from "../services/voice";

const emit = defineEmits<{ close: [] }>();
const chat = useChatStore();
const pet = usePetStore();

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
};
type ActiveChat = {
  message: string;
  thinking: string;
  started_at: string | number;
};
type AiFinishedPayload = {
  text: string;
  thinking: string | null;
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

const ACTIVE_TAB_KEY = "ai-desktop-pet.active-chat-tab";
const currentWindow = getCurrentWindow();
const activeTab = ref<ActiveTab>(loadActiveTab());
const input = ref("");
const clipboardItems = ref<ClipboardItem[]>([]);
const clipboardSearch = ref("");
const chatBubbleRef = ref<HTMLDivElement | null>(null);
const messagesRef = ref<HTMLDivElement | null>(null);
const chatEndRef = ref<HTMLDivElement | null>(null);
const activeMessageMenuId = ref<number | null>(null);
const isFileOver = ref(false);
const BG_KEY = "ai-desktop-pet.chat-bg";
const CUSTOM_BG_KEY = "ai-desktop-pet.chat-bg-custom";
const VOICE_SETTINGS_KEY = "voice_settings";
const chatBg = ref(localStorage.getItem(BG_KEY) || "none");
const customBgImage = ref(localStorage.getItem(CUSTOM_BG_KEY) || "");
const bgCanvasRef = ref<InstanceType<typeof BgCanvas> | null>(null);
const voice = new VoiceController();
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
  if (voiceStatus.value === "listening") return "停止识别";
  if (voiceStatus.value === "speaking") return "停止播报并开始说话";
  return "开始语音输入";
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
    appendAssistantOnce(event.payload.text, event.payload.thinking ?? undefined);
    chat.isLoading = false;
    thinkingContent.value = "";
    streamingAnswer.value = "";
    isThinkingCollapsed.value = true;
    pendingConfirm.value = null;
    pet.setState("speaking");
    pet.updateMood({ happiness: 0.05 });
    void speakAssistantReply(event.payload.text);

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

  unlistenScheduledTaskTriggered = await listen<{ message: string }>("scheduled-task-triggered", () => {
    pet.setState("happy");
    scrollToBottomAfterRender();
    setTimeout(() => {
      if (pet.state === "happy") pet.setState("idle");
    }, 2600);
  });

  try {
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
  if (fileErrorTimer) {
    clearTimeout(fileErrorTimer);
    fileErrorTimer = null;
  }
  window.removeEventListener("storage", handleStorageChange);
  window.removeEventListener("voice-settings-changed", handleVoiceSettingsChanged);
  voice.abortListening();
  voice.stopSpeaking();
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
    });
  } catch (err) {
    chat.isLoading = false;
    isThinkingCollapsed.value = true;
    chat.addMessage("assistant", `出错了: ${err}`);
    pet.setState("confused");
  }
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

async function speakAssistantReply(text: string) {
  if (!voiceSettings.value.enabled || !voiceSettings.value.autoSpeak) return;

  voiceStatus.value = "speaking";
  try {
    await voice.speak(text, voiceSettings.value);
  } catch (err) {
    voiceError.value = err instanceof Error ? err.message : String(err);
  } finally {
    if (voiceStatus.value === "speaking") {
      voiceStatus.value = "idle";
    }
  }
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
  };
}

function appendAssistantOnce(content: string, thinking?: string) {
  const last = chat.messages[chat.messages.length - 1];
  if (last?.role === "assistant" && last.content === content) return;
  chat.addMessage("assistant", content, thinking);
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

function onKeyDown(e: KeyboardEvent) {
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
          <span class="tab-icon">&#x1F4AC;</span>
          <span>对话</span>
        </button>
        <button
          :class="['tab-btn', { active: activeTab === 'clipboard' }]"
          @click="switchTab('clipboard')"
        >
          <span class="tab-icon">&#x1F4CB;</span>
          <span>剪切板</span>
        </button>
      </div>
      <button
        v-if="activeTab === 'chat'"
        class="header-action-btn"
        title="新对话"
        @click="startNewConversation"
      >
        &#x2795;
      </button>
      <button
        v-if="activeTab === 'chat'"
        class="header-action-btn"
        title="清空对话"
        @click="clearChat"
      >
        &#x1F5D1;
      </button>
      <button
        v-else
        class="header-action-btn"
        title="清空剪切板"
        @click="clearClipboard"
      >
        &#x1F5D1;
      </button>
      <button class="close-btn" @click="emit('close')">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M1 1L13 13M13 1L1 13" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
      </button>
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
          <div class="empty-icon">&#x1F43E;</div>
          <div>点击输入框和我聊天吧~</div>
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
            <span>&#x1F43E;</span>
          </div>

          <div class="msg-body">
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

            <div class="bubble">{{ msg.content }}</div>
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
            <span v-else>&#x1F464;</span>
          </div>
          </template>
        </div>

        <div v-if="chat.isLoading" class="message assistant msg-enter">
          <div class="avatar bot-avatar">
            <span class="avatar-thinking">&#x1F43E;</span>
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
              {{ streamingAnswer }}
            </div>


          </div>
        </div>
        <div ref="chatEndRef" class="chat-end" aria-hidden="true"></div>
      </template>

      <template v-else>
        <div class="clipboard-search">
          <span class="clipboard-search-icon">&#x1F50D;</span>
          <input v-model="clipboardSearch" type="search" placeholder="搜索剪切板..." />
        </div>

        <div v-if="clipboardItems.length === 0" class="empty-hint">
          <div class="empty-icon">&#x1F4CB;</div>
          <div>把临时内容存在这里吧~</div>
        </div>
        <div v-else-if="filteredClipboardItems.length === 0" class="empty-hint">
          <div class="empty-icon">&#x1F50D;</div>
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
                  <span class="clipboard-action-icon">{{ item.pinned ? '&#x1F4CC;' : '&#x1F4CD;' }}</span>
                  <span class="clipboard-action-label">{{ item.pinned ? '取消置顶' : '置顶' }}</span>
                </button>
                <button
                  type="button"
                  class="clipboard-action-btn"
                  title="复制"
                  aria-label="复制"
                  @click="copyClipboardItem(item)"
                >
                  <span class="clipboard-action-icon">&#x2398;</span>
                  <span class="clipboard-action-label">复制</span>
                </button>
                <button
                  type="button"
                  class="clipboard-action-btn danger"
                  title="删除"
                  aria-label="删除"
                  @click="deleteClipboardItem(item)"
                >
                  <span class="clipboard-action-icon">&#x1F5D1;</span>
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
            <span class="confirm-card-icon">⚡</span>
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
            <div class="confirm-tool-args">
              <span class="info-label">核心参数:</span>
              <pre class="args-code"><code>{{ pendingConfirm.arguments }}</code></pre>
            </div>
          </div>

          <div class="confirm-card-actions">
            <button class="confirm-btn deny" @click="handleToolConfirm(false)">
              ❌ 拒绝
            </button>
            <button class="confirm-btn approve" @click="handleToolConfirm(true)">
              ✅ 允许
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
      <div class="drop-icon">&#x1F4CE;</div>
      <div>松开添加文件</div>
    </div>
  </div>
</template>

<style scoped>
.chat-bubble {
  position: absolute;
  left: var(--chat-bubble-left, 0);
  top: var(--chat-bubble-top, 0);
  width: var(--chat-bubble-width, 100%);
  height: var(--chat-bubble-height, 100%);
  box-sizing: border-box;
  pointer-events: auto;
  background:
    url("../assets/art/paper-grain.webp"),
    linear-gradient(145deg, rgba(255, 255, 255, 0.94), rgba(255, 249, 238, 0.88)),
    repeating-linear-gradient(0deg, rgba(24, 42, 72, 0.024) 0 1px, transparent 1px 24px),
    var(--pet-bg-glass, rgba(255,255,255,0.92));
  background-size: 480px 480px, auto, auto, auto;
  backdrop-filter: blur(24px) saturate(1.18);
  -webkit-backdrop-filter: blur(24px) saturate(1.18);
  border-radius: 18px;
  box-shadow:
    0 8px 0 rgba(24, 42, 72, 0.045),
    0 22px 54px rgba(24, 42, 72, 0.14),
    0 6px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.10),
    inset 0 1px 0 rgba(255, 255, 255, 0.78);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  z-index: 100;
  border: 2px solid rgba(255, 255, 255, 0.74);
  transition: box-shadow 0.3s, border-color 0.3s;
}

.chat-bubble::before {
  content: "";
  position: absolute;
  inset: 10px;
  border: 1px dashed rgba(var(--pet-primary-rgb, 255, 107, 107), 0.18);
  border-radius: 12px;
  pointer-events: none;
  z-index: 0;
}

.chat-bubble::after {
  content: "";
  position: absolute;
  top: -18px;
  right: 18px;
  width: 128px;
  height: 54px;
  background: url("../assets/art/chat-tape.png") center / contain no-repeat;
  opacity: 0.46;
  filter: drop-shadow(0 7px 10px rgba(24, 42, 72, 0.08));
  pointer-events: none;
  z-index: 1;
  transform: rotate(2deg);
}

.chat-bubble.file-over {
  box-shadow:
    0 8px 0 rgba(24, 42, 72, 0.04),
    0 22px 54px rgba(24, 42, 72, 0.14),
    0 0 0 3px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.26);
}

.header-decor {
  position: relative;
  z-index: 2;
  height: 4px;
  background:
    linear-gradient(90deg, rgba(255, 216, 92, 0.9), rgba(102, 200, 255, 0.82), rgba(255, 147, 199, 0.82)),
    var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 240% 240%;
  animation: softGradientShift 10s ease-in-out infinite;
  flex-shrink: 0;
}

.chat-header {
  position: relative;
  z-index: 2;
  display: flex;
  gap: 6px;
  align-items: center;
  padding: 10px 12px 9px;
  background:
    url("../assets/art/paper-grain.webp"),
    linear-gradient(180deg, rgba(255, 255, 255, 0.88), rgba(255, 249, 238, 0.72)),
    linear-gradient(90deg, rgba(var(--pet-primary-rgb, 255, 107, 107), 0.10), rgba(var(--pet-accent-rgb, 255, 160, 122), 0.06));
  background-size: 420px 420px, auto, auto;
  color: #334155;
  font-size: 13px;
  border-bottom: 1px solid rgba(24, 42, 72, 0.07);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.70);
}

.chat-header > * {
  position: relative;
  z-index: 4;
}

.tabs {
  display: flex;
  gap: 4px;
  flex: 1;
  min-width: 0;
  max-width: 178px;
  padding: 3px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.58);
  border: 1px solid rgba(24, 42, 72, 0.06);
}

.tab-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  border: none;
  border-radius: 9px;
  padding: 6px 10px;
  font-size: 12px;
  cursor: pointer;
  transition: all 0.2s;
  background: transparent;
  color: #64748b;
  font-weight: 650;
  white-space: nowrap;
}

.tab-btn:hover {
  background: rgba(255, 255, 255, 0.76);
  color: #334155;
}

.tab-btn.active {
  background:
    linear-gradient(135deg, rgba(var(--pet-primary-rgb, 255, 107, 107), 0.16), rgba(var(--pet-accent-rgb, 255, 160, 122), 0.12)),
    rgba(255, 255, 255, 0.96);
  color: var(--pet-primary, #ff6b6b);
  font-weight: 760;
  box-shadow:
    0 2px 0 rgba(var(--pet-primary-rgb, 255, 107, 107), 0.08),
    0 8px 18px rgba(24, 42, 72, 0.08);
}

.tab-icon {
  font-size: 13px;
}

.header-action-btn {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: rgba(255, 255, 255, 0.72);
  border: 1px solid rgba(24, 42, 72, 0.06);
  border-radius: 9px;
  padding: 0;
  font-size: 13px;
  cursor: pointer;
  color: var(--pet-primary, #ff6b6b);
  transition: background 0.2s, color 0.2s, transform 0.2s, box-shadow 0.2s;
  line-height: 1;
  box-shadow: 0 3px 8px rgba(24, 42, 72, 0.04);
}

.header-action-btn:hover {
  background: white;
  color: var(--pet-primary-dark, #e55a5a);
  transform: translateY(-1px);
  box-shadow: 0 6px 14px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.12);
}

.close-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(255, 255, 255, 0.72);
  border: 1px solid rgba(24, 42, 72, 0.06);
  border-radius: 9px;
  width: 26px;
  height: 26px;
  color: #94a3b8;
  cursor: pointer;
  transition: all 0.2s;
  box-shadow: 0 3px 8px rgba(24, 42, 72, 0.04);
}

.close-btn:hover {
  background: rgba(239, 68, 68, 0.10);
  border-color: rgba(239, 68, 68, 0.18);
  color: #ef4444;
  transform: rotate(90deg);
}

.messages-area {
  flex: 1;
  position: relative;
  overflow: hidden;
  min-height: 0;
  background:
    url("../assets/art/paper-grain.webp"),
    linear-gradient(135deg, rgba(102, 200, 255, 0.08), transparent 42%),
    linear-gradient(225deg, rgba(255, 216, 92, 0.10), transparent 44%);
  background-size: 520px 520px, auto, auto;
}

.chat-messages {
  position: relative;
  z-index: 1;
  height: 100%;
  overflow-y: auto;
  padding: 14px 14px 13px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  user-select: text;
  -webkit-user-select: text;
  scrollbar-width: thin;
  scrollbar-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.20) transparent;
}

.chat-messages::-webkit-scrollbar {
  width: 4px;
}

.chat-messages::-webkit-scrollbar-thumb {
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.20);
  border-radius: 4px;
}

.chat-end {
  width: 100%;
  height: 1px;
  flex: 0 0 1px;
}

.empty-hint {
  text-align: center;
  color: #7b8798;
  font-size: 13px;
  min-height: 210px;
  padding: 18px 18px 22px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  gap: 7px;
  border: 1px dashed rgba(var(--pet-primary-rgb, 255, 107, 107), 0.18);
  border-radius: 14px;
  background:
    url("../assets/art/chat-empty.png") center 12px / min(82%, 238px) auto no-repeat,
    linear-gradient(145deg, rgba(255, 255, 255, 0.62), rgba(255, 249, 238, 0.50));
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.58),
    0 5px 14px rgba(24, 42, 72, 0.05);
}

.empty-icon {
  display: none;
}

.message {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  animation: msgSlideIn 0.3s ease-out;
}

.message.user {
  justify-content: flex-end;
}

.message.system {
  justify-content: center;
  align-items: center;
  gap: 0;
}

.system-divider {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 4px 0;
}

.system-divider-line {
  flex: 1;
  height: 1px;
  background: rgba(15, 23, 42, 0.08);
}

.system-divider-text {
  font-size: 11px;
  color: #94a3b8;
  white-space: nowrap;
  flex-shrink: 0;
}

@keyframes msgSlideIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.avatar {
  width: 30px;
  height: 30px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  flex-shrink: 0;
  overflow: hidden;
  border: 2px solid rgba(255, 255, 255, 0.82);
  box-shadow:
    0 5px 12px rgba(24, 42, 72, 0.12),
    inset 0 1px 0 rgba(255, 255, 255, 0.38);
}

.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.bot-avatar {
  background:
    radial-gradient(circle at 35% 25%, rgba(255, 255, 255, 0.58), transparent 34%),
    var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 180% 180%;
  animation: softGradientShift 8s ease-in-out infinite;
}

.user-avatar {
  background: linear-gradient(135deg, #66c8ff, #a7a2ff);
  color: white;
}

.avatar-thinking {
  animation: avatarPulse 1.5s ease-in-out infinite;
}

@keyframes avatarPulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.6; transform: scale(0.9); }
}

.msg-body {
  max-width: 78%;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.bubble {
  padding: 10px 13px;
  border-radius: 16px;
  font-size: 13px;
  line-height: 1.5;
  word-break: break-word;
  cursor: text;
  user-select: text;
  -webkit-user-select: text;
  position: relative;
  z-index: 1;
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
}

.message.user .bubble {
  background:
    linear-gradient(145deg, rgba(255, 255, 255, 0.16), transparent 36%),
    var(--pet-bubble-user, linear-gradient(135deg, #ff6b6b, #ff8e8e));
  color: white;
  border-bottom-right-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.28);
  box-shadow:
    0 4px 0 rgba(var(--pet-primary-rgb, 255, 107, 107), 0.10),
    0 10px 20px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.16);
}

.message.user .bubble::before {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: rgba(255, 255, 255, 0.15);
  z-index: -1;
  pointer-events: none;
}

.message.assistant .bubble {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.90), rgba(255, 251, 244, 0.78)),
    var(--pet-bubble-bot, rgba(255, 107, 107, 0.08));
  color: var(--pet-font-color, var(--pet-bubble-bot-text, #4a4a4a));
  text-shadow: 0 1px 0 rgba(255, 255, 255, 0.64);
  border-bottom-left-radius: 6px;
  border: 1px solid rgba(24, 42, 72, 0.07);
  box-shadow:
    0 3px 0 rgba(24, 42, 72, 0.035),
    0 10px 18px rgba(24, 42, 72, 0.08);
}

.message.assistant .bubble::before {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background:
    linear-gradient(90deg, rgba(var(--pet-primary-rgb, 255, 107, 107), 0.14), transparent 5px);
  z-index: -1;
  pointer-events: none;
}

.streaming-answer {
  margin-top: 4px;
  white-space: pre-wrap;
}

.msg-time {
  font-size: 10px;
  color: #94a3b8;
  padding: 0 4px;
}

.message.user .msg-time {
  text-align: right;
}

.message-action-menu {
  display: inline-flex;
  align-self: flex-start;
  gap: 6px;
  margin-top: 5px;
  padding: 4px;
  border: 1px solid rgba(24, 42, 72, 0.08);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.92);
  box-shadow: 0 8px 18px rgba(24, 42, 72, 0.12);
  user-select: none;
  -webkit-user-select: none;
}

.message.user .message-action-menu {
  align-self: flex-end;
}

.message-action-menu button {
  min-width: 58px;
  height: 28px;
  padding: 0 9px;
  border: 1px solid rgba(24, 42, 72, 0.08);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.86);
  color: #475569;
  cursor: pointer;
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
}

.message-action-menu button:hover:not(:disabled) {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.30);
  color: var(--pet-primary, #ff6b6b);
  background: #ffffff;
}

.message-action-menu button:disabled {
  opacity: 0.48;
  cursor: not-allowed;
}

.thinking-section {
  margin-bottom: 4px;
}

.thinking-bubble {
  max-width: 100%;
}

.thinking-header {
  display: flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  user-select: none;
  -webkit-user-select: none;
  font-size: 12px;
  color: #65758a;
  padding: 2px 0;
  transition: color 0.2s;
}

.thinking-header:hover {
  color: var(--pet-primary, #ff6b6b);
}

.thinking-toggle {
  font-size: 10px;
  width: 12px;
  flex-shrink: 0;
  transition: transform 0.2s;
}

.thinking-label {
  font-size: 12px;
}

.thinking-loading {
  color: var(--pet-primary, #ff6b6b);
  font-weight: 600;
}

.thinking-dots {
  display: inline-flex;
  gap: 1px;
}

.thinking-dots span {
  animation: dotBounce 1.4s ease-in-out infinite;
  font-weight: bold;
  color: var(--pet-primary, #ff6b6b);
}

.thinking-dots span:nth-child(2) {
  animation-delay: 0.2s;
}

.thinking-dots span:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes dotBounce {
  0%, 60%, 100% { transform: translateY(0); opacity: 0.4; }
  30% { transform: translateY(-4px); opacity: 1; }
}

.stop-btn {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 3px;
  background: var(--pet-primary, #ff6b6b);
  color: white;
  border: none;
  border-radius: 6px;
  padding: 3px 8px;
  font-size: 11px;
  cursor: pointer;
  transition: all 0.2s;
  box-shadow: 0 4px 10px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.16);
}

.stop-btn:hover {
  background: var(--pet-primary-dark, #e55a5a);
  transform: scale(1.05);
}

.thinking-content {
  margin-top: 6px;
  padding: 8px 10px;
  background: rgba(255, 255, 255, 0.62);
  border-radius: 10px;
  font-size: 11px;
  line-height: 1.6;
  color: #64748b;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  border: 1px solid rgba(24, 42, 72, 0.07);
}

.thinking-streaming {
  border-left: 2px solid var(--pet-primary, #ff6b6b);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.045);
}

.thinking-waiting {
  color: #ccc;
  font-style: italic;
  text-align: center;
  padding: 12px;
}

.thinking-expand-enter-active,
.thinking-expand-leave-active {
  transition: all 0.25s ease;
  overflow: hidden;
}

.thinking-expand-enter-from,
.thinking-expand-leave-to {
  max-height: 0;
  opacity: 0;
  margin-top: 0;
  padding: 0 10px;
}

.clipboard-search {
  position: sticky;
  top: 0;
  z-index: 1;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 10px;
  border: 1px solid rgba(var(--pet-primary-rgb, 255, 107, 107), 0.14);
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.86);
  box-shadow: 0 5px 14px rgba(24, 42, 72, 0.06);
}

.clipboard-search-icon {
  color: #94a3b8;
  font-size: 12px;
}

.clipboard-search input {
  min-width: 0;
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  color: var(--pet-font-color, #333);
  font-size: 12px;
}

.clipboard-search input::placeholder {
  color: #cbd5e1;
}

.clipboard-item {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
  animation: msgSlideIn 0.3s ease-out;
}

.clipboard-item-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.clipboard-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 14px;
  color: #94a3b8;
  font-size: 10px;
}

.clipboard-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
  padding: 0 4px 0 6px;
}

.pin-label {
  color: var(--pet-primary, #ff6b6b);
  font-weight: 600;
}

.clipboard-bubble {
  background:
    linear-gradient(145deg, rgba(255, 255, 255, 0.76), rgba(255, 249, 230, 0.78)),
    linear-gradient(135deg, #fff9e6, #fff3cd) !important;
  color: var(--pet-font-color, #6b5a2e) !important;
  border-left: 3px solid var(--pet-accent, #fbbf24);
  border-bottom-left-radius: 6px;
}

.clipboard-item.pinned .clipboard-bubble {
  border-left-color: var(--pet-primary, #ff6b6b);
}

.clipboard-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  flex-wrap: wrap;
  margin-left: auto;
}

.clipboard-action-btn {
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  min-width: 0;
  padding: 0 9px;
  border: 1px solid rgba(24, 42, 72, 0.07);
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.82);
  color: #64748b;
  cursor: pointer;
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
  box-shadow: 0 3px 8px rgba(24, 42, 72, 0.05);
  transition:
    transform 0.2s,
    border-color 0.2s,
    background 0.2s,
    color 0.2s,
    box-shadow 0.2s;
}

.clipboard-action-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  line-height: 1;
}

.clipboard-action-label {
  white-space: nowrap;
}

.clipboard-action-btn:hover {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.28);
  color: var(--pet-primary, #ff6b6b);
  background: white;
  transform: translateY(-1px);
  box-shadow: 0 6px 12px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.12);
}

.clipboard-action-btn:hover .clipboard-action-icon {
  transform: scale(1.05);
}

.clipboard-action-btn.active {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.3);
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.12);
  color: var(--pet-primary, #ff6b6b);
}

.clipboard-action-btn.active:hover {
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.16);
}

.clipboard-action-btn.danger:hover {
  border-color: rgba(239, 68, 68, 0.28);
  color: #ef4444;
  background: rgba(239, 68, 68, 0.08);
  box-shadow: 0 6px 12px rgba(239, 68, 68, 0.1);
}

.chat-input {
  position: relative;
  z-index: 2;
  display: flex;
  align-items: center;
  padding: 10px 12px 12px;
  gap: 8px;
  border-top: 1px solid rgba(24, 42, 72, 0.07);
  background:
    url("../assets/art/paper-grain.webp"),
    linear-gradient(180deg, rgba(255, 255, 255, 0.54), rgba(255, 249, 238, 0.78));
  background-size: 420px 420px, auto;
}

.chat-input input {
  flex: 1;
  min-width: 0;
  border: 1px solid rgba(24, 42, 72, 0.08);
  border-radius: 16px;
  padding: 9px 14px;
  font-size: 13px;
  outline: none;
  background: rgba(255, 255, 255, 0.88);
  transition: all 0.2s;
  color: var(--pet-font-color, #333);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.72);
}

.chat-input input:focus {
  border-color: var(--pet-primary, #ff6b6b);
  box-shadow: 0 0 0 3px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.1);
  background: white;
}

.chat-input input::placeholder {
  color: #aab4c2;
}

.voice-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 35px;
  height: 35px;
  flex-shrink: 0;
  border: 1px solid rgba(var(--pet-primary-rgb, 255, 107, 107), 0.22);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.86);
  color: var(--pet-primary, #ff6b6b);
  cursor: pointer;
  transition: all 0.2s;
  box-shadow: 0 5px 12px rgba(15, 23, 42, 0.08);
}

.voice-btn:hover:not(:disabled) {
  transform: scale(1.06);
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.38);
  box-shadow: 0 8px 16px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.14);
}

.voice-btn.active {
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  color: white;
  animation: voicePulse 1.1s ease-in-out infinite;
}

.voice-btn.speaking {
  color: white;
  background: linear-gradient(135deg, #38bdf8, #8b5cf6);
}

.voice-btn:disabled {
  opacity: 0.42;
  cursor: not-allowed;
}

@keyframes voicePulse {
  0%, 100% {
    box-shadow: 0 0 0 0 rgba(var(--pet-primary-rgb, 255, 107, 107), 0.32);
  }
  50% {
    box-shadow: 0 0 0 7px rgba(var(--pet-primary-rgb, 255, 107, 107), 0);
  }
}

.voice-status {
  position: relative;
  z-index: 2;
  padding: 0 14px 9px 52px;
  margin-top: 0;
  min-height: 14px;
  color: var(--pet-primary, #ff6b6b);
  font-size: 11px;
  line-height: 1.4;
  background:
    url("../assets/art/paper-grain.webp"),
    rgba(255, 249, 238, 0.78);
  background-size: 420px 420px, auto;
  word-break: break-word;
}

.send-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 37px;
  height: 37px;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 220% 220%;
  color: white;
  border: none;
  border-radius: 13px;
  cursor: pointer;
  transition: all 0.2s;
  box-shadow:
    0 4px 0 rgba(var(--pet-primary-rgb, 255, 107, 107), 0.12),
    0 8px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.20);
  flex-shrink: 0;
}

.send-btn:hover:not(:disabled) {
  background-position: 100% 50%;
  transform: scale(1.08);
  box-shadow: 0 10px 22px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.28);
}

.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.drop-hint {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background:
    linear-gradient(135deg, rgba(255, 255, 255, 0.92), rgba(255, 249, 238, 0.88));
  backdrop-filter: blur(8px);
  color: var(--pet-primary, #ff6b6b);
  font-size: 14px;
  font-weight: 600;
  pointer-events: none;
  border-radius: 18px;
  border: 2px dashed rgba(var(--pet-primary-rgb, 255, 107, 107), 0.48);
  box-shadow: inset 0 0 0 8px rgba(255, 255, 255, 0.32);
}

.drop-icon {
  font-size: 32px;
  animation: dropBounce 0.6s ease infinite alternate;
}

@keyframes dropBounce {
  from { transform: translateY(0); }
  to { transform: translateY(-6px); }
}

@keyframes softGradientShift {
  0%, 100% {
    background-position: 0% 50%;
  }
  50% {
    background-position: 100% 50%;
  }
}

/* 交互式工具确认卡片样式 */
.floating-confirm-overlay {
  padding: 0 14px 10px;
  position: relative;
  z-index: 10;
}

.tool-confirm-card {
  background:
    linear-gradient(145deg, rgba(255, 255, 255, 0.96), rgba(255, 249, 238, 0.90));
  backdrop-filter: blur(12px);
  border: 1px solid rgba(245, 158, 11, 0.38);
  border-radius: 16px;
  padding: 12px;
  box-shadow:
    0 4px 12px rgba(245, 158, 11, 0.12),
    0 14px 30px rgba(24, 42, 72, 0.12),
    inset 0 1px 0 rgba(255, 255, 255, 0.8);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.confirm-card-header {
  display: flex;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid rgba(245, 158, 11, 0.12);
  padding-bottom: 6px;
}

.confirm-card-icon {
  font-size: 16px;
  animation: pulseLight 1.5s infinite;
}

.confirm-card-title {
  font-size: 13px;
  font-weight: 700;
  color: #b45309;
}

.confirm-card-body {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.confirm-tool-info {
  display: flex;
  gap: 8px;
  align-items: center;
  font-size: 12px;
}

.confirm-tool-args {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.info-label {
  font-weight: 600;
  color: #64748b;
  min-width: 60px;
  font-size: 11px;
}

.info-value {
  color: #1e293b;
  font-weight: 500;
}

.args-code {
  background: rgba(15, 23, 42, 0.06);
  border: 1px solid rgba(15, 23, 42, 0.08);
  padding: 8px 10px;
  border-radius: 8px;
  font-family: Consolas, Monaco, monospace;
  font-size: 11px;
  color: #0f172a;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-all;
  margin: 0;
}

.confirm-card-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}

.confirm-btn {
  flex: 1;
  padding: 8px;
  border: none;
  border-radius: 10px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
}

.confirm-btn.deny {
  background: rgba(239, 68, 68, 0.08);
  color: #dc2626;
  border: 1px solid rgba(239, 68, 68, 0.15);
}

.confirm-btn.deny:hover {
  background: #ef4444;
  color: white;
  box-shadow: 0 4px 12px rgba(239, 68, 68, 0.2);
}

.confirm-btn.approve {
  background: rgba(34, 197, 94, 0.08);
  color: #16a34a;
  border: 1px solid rgba(34, 197, 94, 0.15);
}

.confirm-btn.approve:hover {
  background: #22c55e;
  color: white;
  box-shadow: 0 4px 12px rgba(34, 197, 94, 0.2);
}

@keyframes slideUp {
  from { transform: translateY(12px); opacity: 0; }
  to { transform: translateY(0); opacity: 1; }
}

@keyframes pulseLight {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.6; transform: scale(0.92); }
}

/* ===== 文件预览区 ===== */
.file-preview-area {
  position: relative;
  z-index: 2;
  padding: 8px 10px 0;
  border-top: 1px solid rgba(24, 42, 72, 0.07);
  background: rgba(255, 249, 238, 0.58);
}

.file-preview-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11px;
  color: #64748b;
  margin-bottom: 6px;
}

.clear-all-btn {
  background: none;
  border: none;
  color: #94a3b8;
  font-size: 11px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 4px;
  transition: color 0.15s, background 0.15s;
}

.clear-all-btn:hover {
  color: #ef4444;
  background: rgba(239, 68, 68, 0.08);
}

.file-preview-list {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  padding-bottom: 8px;
  max-height: 120px;
  overflow-y: auto;
}

.file-preview-item {
  position: relative;
  width: 72px;
  height: 72px;
  border-radius: 12px;
  border: 1px solid rgba(24, 42, 72, 0.08);
  background: rgba(255, 255, 255, 0.84);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  transition: transform 0.2s, box-shadow 0.2s;
  flex-shrink: 0;
}

.file-preview-item:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 14px rgba(24, 42, 72, 0.08);
}

.file-preview-item.error {
  opacity: 0.55;
}

.preview-thumbnail {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.preview-fallback,
.preview-file-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  padding: 4px;
  width: 100%;
  height: 100%;
}

.file-icon {
  font-size: 20px;
}

.file-icon-lg {
  font-size: 24px;
}

.fallback-text {
  font-size: 9px;
  color: #94a3b8;
}

.file-name {
  font-size: 10px;
  max-width: 62px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: center;
  color: #334155;
}

.file-size {
  font-size: 9px;
  color: #94a3b8;
}

.remove-file-btn {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 18px;
  height: 18px;
  border-radius: 8px;
  background: rgba(51, 65, 85, 0.68);
  color: white;
  border: none;
  font-size: 14px;
  line-height: 1;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  transition: opacity 0.15s, background 0.15s;
  padding: 0;
}

.file-preview-item:hover .remove-file-btn {
  opacity: 1;
}

.remove-file-btn:hover {
  background: rgba(239, 68, 68, 0.85);
}

/* 验证错误提示 */
.file-validation-error {
  padding: 5px 12px;
  font-size: 11px;
  color: #ef4444;
  background: rgba(239, 68, 68, 0.06);
  border-top: 1px solid rgba(239, 68, 68, 0.1);
}

/* 消息气泡中的文件标签 */
.msg-files {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 6px;
}

.msg-file-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 2px 8px;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.2);
  font-size: 11px;
  border: 1px solid rgba(15, 23, 42, 0.08);
}

/* 文件预览过渡动画 */
.slide-up-enter-active,
.slide-up-leave-active {
  transition: all 0.25s ease;
  overflow: hidden;
}

.slide-up-enter-from,
.slide-up-leave-to {
  opacity: 0;
  transform: translateY(8px);
  max-height: 0;
  padding-top: 0;
  padding-bottom: 0;
}

.file-item-enter-active,
.file-item-leave-active {
  transition: all 0.2s ease;
}

.file-item-enter-from {
  opacity: 0;
  transform: scale(0.8);
}

.file-item-leave-to {
  opacity: 0;
  transform: scale(0.8);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
