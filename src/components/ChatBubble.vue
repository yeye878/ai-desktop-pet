<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useChatStore } from "../stores/chat";
import { usePetStore } from "../stores/pet";
import { invoke } from "@tauri-apps/api/core";
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

const ACTIVE_TAB_KEY = "ai-desktop-pet.active-chat-tab";
const currentWindow = getCurrentWindow();
const activeTab = ref<ActiveTab>(loadActiveTab());
const input = ref("");
const clipboardItems = ref<ClipboardItem[]>([]);
const clipboardSearch = ref("");
const chatBubbleRef = ref<HTMLDivElement | null>(null);
const messagesRef = ref<HTMLDivElement | null>(null);
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
let unlistenDragDrop: UnlistenFn | null = null;
let unlistenThinking: UnlistenFn | null = null;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;
let unlistenChatCleared: UnlistenFn | null = null;

const thinkingContent = ref("");
const isThinkingCollapsed = ref(true);
const expandedThinking = reactive(new Set<number>());
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

onMounted(async () => {
  unlistenDragDrop = await currentWindow.onDragDropEvent((event) => {
    handleDragDropEvent(event.payload);
  });

  unlistenThinking = await listen<string>("ai-thinking", (event) => {
    chat.isLoading = true;
    thinkingContent.value += event.payload;
    scrollToBottomAfterRender();
  });

  unlistenAiFinished = await listen<AiFinishedPayload>("ai-finished", (event) => {
    appendAssistantOnce(event.payload.text, event.payload.thinking ?? undefined);
    chat.isLoading = false;
    thinkingContent.value = "";
    isThinkingCollapsed.value = true;
    pet.setState("speaking");
    pet.updateMood({ happiness: 0.05 });
    void speakAssistantReply(event.payload.text);

    setTimeout(() => {
      if (pet.state === "speaking") pet.setState("idle");
    }, 3000);
  });

  unlistenAiError = await listen<AiErrorPayload>("ai-error", (event) => {
    const text = event.payload.aborted ? "已中止" : `出错了: ${event.payload.message}`;
    appendAssistantOnce(text, event.payload.thinking ?? (thinkingContent.value || undefined));
    chat.isLoading = false;
    thinkingContent.value = "";
    isThinkingCollapsed.value = true;
    pet.setState(event.payload.aborted ? "idle" : "confused");
  });

  unlistenChatCleared = await listen("chat-history-cleared", () => {
    chat.clearMessages();
    chat.isLoading = false;
    thinkingContent.value = "";
  });

  try {
    await loadVoiceSettings();
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
  await scrollToBottomAfterRender();
});

onBeforeUnmount(() => {
  if (scrollFrame !== null) {
    cancelAnimationFrame(scrollFrame);
    scrollFrame = null;
  }
  if (unlistenDragDrop) {
    unlistenDragDrop();
    unlistenDragDrop = null;
  }
  if (unlistenThinking) {
    unlistenThinking();
    unlistenThinking = null;
  }
  if (unlistenAiFinished) {
    unlistenAiFinished();
    unlistenAiFinished = null;
  }
  if (unlistenAiError) {
    unlistenAiError();
    unlistenAiError = null;
  }
  if (unlistenChatCleared) {
    unlistenChatCleared();
    unlistenChatCleared = null;
  }
  window.removeEventListener("storage", handleStorageChange);
  window.removeEventListener("voice-settings-changed", handleVoiceSettingsChanged);
  voice.abortListening();
  voice.stopSpeaking();
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
  if (!text) return;

  if (activeTab.value === "clipboard") {
    await saveClipboardText(text);
    return;
  }
  if (chat.isLoading) return;

  chat.addMessage("user", text);
  input.value = "";
  chat.isLoading = true;
  thinkingContent.value = "";
  isThinkingCollapsed.value = false;
  pet.setState("thinking");

  await nextTick();
  scrollToBottom();

  try {
    await invoke<{ started: boolean }>("send_to_ai", {
      message: text,
    });
  } catch (err) {
    chat.isLoading = false;
    isThinkingCollapsed.value = true;
    chat.addMessage("assistant", `出错了: ${err}`);
    pet.setState("confused");
  }
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
  try {
    await invoke("abort_ai");
  } catch {
    // Ignore abort failures.
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

async function clearChat() {
  try {
    await invoke("clear_chat_history");
    chat.clearMessages();
    chat.isLoading = false;
    thinkingContent.value = "";
    await currentWindow.emit("chat-history-cleared");
  } catch (err) {
    chat.addMessage("assistant", `对话清空失败: ${err}`);
  }
}

function scrollToBottom() {
  if (messagesRef.value) {
    if (activeTab.value === "chat") {
      messagesRef.value.scrollTop = messagesRef.value.scrollHeight;
    } else {
      messagesRef.value.scrollTop = 0;
    }
  }
}

function handleDragDropEvent(event: DragDropEvent) {
  if (event.type === "enter" || event.type === "over") {
    isFileOver.value = isPositionInsideBubble(event.position);
    return;
  }
  isFileOver.value = false;
  if (event.type === "drop" && event.paths.length > 0 && isPositionInsideBubble(event.position)) {
    appendFilePaths(event.paths);
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

function appendFilePaths(paths: string[]) {
  const fileText = paths.map((path) => `"${path}"`).join(" ");
  const prompt = `请使用文件投喂分类流程处理这些本地文件路径: ${fileText}`;
  input.value = input.value.trim() ? `${input.value.trim()} ${prompt}` : prompt;
}

async function scrollToBottomAfterRender() {
  await nextTick();
  if (scrollFrame !== null) {
    cancelAnimationFrame(scrollFrame);
  }
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = null;
    scrollToBottom();
  });
}

function onBgMouseMove(e: MouseEvent) {
  if (bgCanvasRef.value) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    bgCanvasRef.value.onMouseMove(e.clientX - rect.left, e.clientY - rect.top);
  }
}

function onBgClick(e: MouseEvent) {
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
        >
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
            <div class="msg-time">{{ formatTime(msg.timestamp) }}</div>
          </div>

          <div v-if="msg.role === 'user'" class="avatar user-avatar">
            <img v-if="userAvatar" :src="userAvatar" alt="用户头像" />
            <span v-else>&#x1F464;</span>
          </div>
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
          </div>
        </div>
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
        :disabled="!input.trim() || (activeTab === 'chat' && chat.isLoading)"
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
      <div class="drop-icon">&#x1F4C1;</div>
      <div>松开添加文件路径</div>
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
    linear-gradient(180deg, rgba(255, 255, 255, 0.84), rgba(255, 255, 255, 0.66)),
    var(--pet-bg-glass, rgba(255,255,255,0.92));
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border-radius: 14px;
  box-shadow:
    0 18px 42px rgba(15, 23, 42, 0.16),
    0 4px 12px rgba(15, 23, 42, 0.08),
    inset 0 1px 0 rgba(255, 255, 255, 0.5);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  z-index: 100;
  border: 1px solid rgba(255, 255, 255, 0.58);
  transition: box-shadow 0.3s, border-color 0.3s;
}

.chat-bubble.file-over {
  box-shadow:
    0 8px 32px rgba(0, 0, 0, 0.12),
    0 0 0 2px var(--pet-primary, #ff6b6b);
}

.header-decor {
  height: 3px;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 240% 240%;
  animation: softGradientShift 10s ease-in-out infinite;
  flex-shrink: 0;
}

.chat-header {
  display: flex;
  gap: 6px;
  align-items: center;
  padding: 8px 10px;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 240% 240%;
  color: white;
  font-size: 13px;
  animation: softGradientShift 10s ease-in-out infinite;
}

.tabs {
  display: flex;
  gap: 4px;
  flex: 1;
}

.tab-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  border: none;
  border-radius: 8px;
  padding: 5px 10px;
  font-size: 12px;
  cursor: pointer;
  transition: all 0.2s;
  background: rgba(255, 255, 255, 0.15);
  color: rgba(255, 255, 255, 0.85);
}

.tab-btn:hover {
  background: rgba(255, 255, 255, 0.25);
  color: white;
}

.tab-btn.active {
  background: white;
  color: var(--pet-primary, #ff6b6b);
  font-weight: 600;
  box-shadow: 0 6px 16px rgba(15, 23, 42, 0.12);
}

.tab-icon {
  font-size: 13px;
}

.header-action-btn {
  background: rgba(255, 255, 255, 0.15);
  border: none;
  border-radius: 8px;
  padding: 4px 6px;
  font-size: 13px;
  cursor: pointer;
  color: white;
  transition: background 0.2s;
  line-height: 1;
}

.header-action-btn:hover {
  background: rgba(255, 255, 255, 0.3);
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

.messages-area {
  flex: 1;
  position: relative;
  overflow: hidden;
  min-height: 0;
}

.chat-messages {
  position: relative;
  z-index: 1;
  height: 100%;
  overflow-y: auto;
  padding: 13px 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  user-select: text;
  -webkit-user-select: text;
  scrollbar-width: thin;
  scrollbar-color: rgba(0, 0, 0, 0.1) transparent;
}

.chat-messages::-webkit-scrollbar {
  width: 4px;
}

.chat-messages::-webkit-scrollbar-thumb {
  background: rgba(0, 0, 0, 0.1);
  border-radius: 4px;
}

.empty-hint {
  text-align: center;
  color: #94a3b8;
  font-size: 13px;
  padding: 40px 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}

.empty-icon {
  font-size: 32px;
  opacity: 0.4;
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
  width: 28px;
  height: 28px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  flex-shrink: 0;
  overflow: hidden;
  box-shadow:
    0 5px 12px rgba(15, 23, 42, 0.14),
    inset 0 1px 0 rgba(255, 255, 255, 0.28);
}

.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.bot-avatar {
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 180% 180%;
  animation: softGradientShift 8s ease-in-out infinite;
}

.user-avatar {
  background: linear-gradient(135deg, #475569, #0f172a);
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
  padding: 9px 12px;
  border-radius: 14px;
  font-size: 13px;
  line-height: 1.5;
  word-break: break-word;
  cursor: text;
  user-select: text;
  -webkit-user-select: text;
  position: relative;
  z-index: 1;
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
}

.message.user .bubble {
  background: var(--pet-bubble-user, linear-gradient(135deg, #ff6b6b, #ff8e8e));
  color: white;
  border-bottom-right-radius: 4px;
  box-shadow: 0 8px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.18);
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
  background: var(--pet-bubble-bot, rgba(255, 107, 107, 0.08));
  color: var(--pet-font-color, var(--pet-bubble-bot-text, #4a4a4a));
  text-shadow: 0 0 1px rgba(255, 255, 255, 0.8), 0 0 4px rgba(255, 255, 255, 0.5);
  border-bottom-left-radius: 4px;
  border: 1px solid rgba(255, 255, 255, 0.4);
  box-shadow: 0 4px 12px rgba(15, 23, 42, 0.08);
}

.message.assistant .bubble::before {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: rgba(255, 255, 255, 0.45);
  z-index: -1;
  pointer-events: none;
}

.msg-time {
  font-size: 10px;
  color: #94a3b8;
  padding: 0 4px;
}

.message.user .msg-time {
  text-align: right;
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
  color: #64748b;
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
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.1);
}

.stop-btn:hover {
  background: var(--pet-primary-dark, #e55a5a);
  transform: scale(1.05);
}

.thinking-content {
  margin-top: 6px;
  padding: 8px 10px;
  background: rgba(15, 23, 42, 0.035);
  border-radius: 8px;
  font-size: 11px;
  line-height: 1.6;
  color: #64748b;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  border: 1px solid rgba(15, 23, 42, 0.05);
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
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.9);
  box-shadow: 0 4px 10px rgba(15, 23, 42, 0.04);
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
  background: linear-gradient(135deg, #fff9e6, #fff3cd) !important;
  color: var(--pet-font-color, #6b5a2e) !important;
  border-left: 3px solid var(--pet-accent, #fbbf24);
  border-bottom-left-radius: 4px;
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
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.88);
  color: #64748b;
  cursor: pointer;
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
  box-shadow: 0 3px 8px rgba(15, 23, 42, 0.04);
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
  display: flex;
  align-items: center;
  padding: 9px 10px 10px;
  gap: 8px;
  border-top: 1px solid rgba(15, 23, 42, 0.06);
  background: rgba(255, 255, 255, 0.62);
}

.chat-input input {
  flex: 1;
  min-width: 0;
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 18px;
  padding: 8px 14px;
  font-size: 13px;
  outline: none;
  background: rgba(255, 255, 255, 0.82);
  transition: all 0.2s;
  color: var(--pet-font-color, #333);
}

.chat-input input:focus {
  border-color: var(--pet-primary, #ff6b6b);
  box-shadow: 0 0 0 3px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.1);
  background: white;
}

.chat-input input::placeholder {
  color: #ccc;
}

.voice-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  border: 1px solid rgba(var(--pet-primary-rgb, 255, 107, 107), 0.22);
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.84);
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
  padding: 0 14px 9px 52px;
  margin-top: -4px;
  min-height: 14px;
  color: var(--pet-primary, #ff6b6b);
  font-size: 11px;
  line-height: 1.4;
  background: rgba(255, 255, 255, 0.62);
  word-break: break-word;
}

.send-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 220% 220%;
  color: white;
  border: none;
  border-radius: 50%;
  cursor: pointer;
  transition: all 0.2s;
  box-shadow: 0 8px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.22);
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
  background: rgba(255, 255, 255, 0.9);
  backdrop-filter: blur(8px);
  color: var(--pet-primary, #ff6b6b);
  font-size: 14px;
  font-weight: 600;
  pointer-events: none;
  border-radius: 14px;
  border: 2px dashed var(--pet-primary, #ff6b6b);
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
</style>
