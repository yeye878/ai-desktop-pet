<script setup lang="ts">
import Dashboard from "./components/Dashboard.vue";
import PetCanvas from "./components/PetCanvas.vue";
import ChatBubble from "./components/ChatBubble.vue";
import ContextMenu from "./components/ContextMenu.vue";
import Settings from "./components/Settings.vue";
import ToolConfirmPanel from "./components/ToolConfirmPanel.vue";
import VoicePanel from "./components/VoicePanel.vue";
import { onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { cursorPosition, getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { resolveSkinId, usePetStore } from "./stores/pet";
import {
  DEFAULT_VOICE_SETTINGS,
  parseVoiceSettings,
  type VoiceSettings,
} from "./services/voice";
import {
  getActiveCustomPixelPetAsset,
  setActiveCustomPixelPetAsset,
} from "./services/customPixelPetAssets";
import { isPetCanvasPoint } from "./services/petHitTest";
import { PET_CHARACTER_SETTING_KEY, resolvePetCharacterId } from "./services/petCharacters";

type AppWindowLabel = "main" | "pet" | "chat" | "context-menu" | "settings" | "voice" | "tool-confirm";
type PanelLabel = Exclude<AppWindowLabel, "main" | "pet">;
type WindowHandle = ReturnType<typeof getCurrentWindow> | WebviewWindow;
type ToolConfirmPayload = {
  id: string;
  tool_name: string;
  arguments: string;
  summary?: string;
  command?: string | null;
  path?: string | null;
};

const PET_W = 120;
const PANEL_GAP = 8;
const PANEL_SPECS: Record<PanelLabel, { width: number; height: number; title: string }> = {
  chat: { width: 340, height: 400, title: "AI Desktop Pet Chat" },
  "context-menu": { width: 170, height: 320, title: "AI Desktop Pet Menu" },
  settings: { width: 380, height: 460, title: "AI Desktop Pet Settings" },
  "tool-confirm": { width: 390, height: 350, title: "AI Desktop Pet Tool Confirm" },
  voice: { width: 430, height: 520, title: "AI Desktop Pet Voice" },
};
const VOICE_SETTINGS_KEY = "voice_settings";
const TOOL_CONFIRM_PAYLOAD_KEY = "ai-desktop-pet.tool-confirm-payload";
const APP_WINDOW_LABELS = new Set<AppWindowLabel>([
  "main",
  "pet",
  "chat",
  "context-menu",
  "settings",
  "voice",
  "tool-confirm",
]);

const currentWindow = getCurrentWindow();
const currentLabel: AppWindowLabel = APP_WINDOW_LABELS.has(currentWindow.label as AppWindowLabel)
  ? currentWindow.label as AppWindowLabel
  : "main";
const petStore = usePetStore();
const isDraggingPet = ref(false);
let cursorHitTestTimer: ReturnType<typeof setInterval> | null = null;
let lastIgnoreCursorEvents: boolean | null = null;
let unlistenAppearanceChanged: UnlistenFn | null = null;
let unlistenFocusChanged: UnlistenFn | null = null;
let unlistenMoved: UnlistenFn | null = null;
let unlistenOpenVoice: UnlistenFn | null = null;
let unlistenVoiceSettingsChanged: UnlistenFn | null = null;
let unlistenToolConfirm: UnlistenFn | null = null;
let registeredVoiceShortcut = "";
let voiceShortcutRegistration = Promise.resolve();

function panelUrl(label: PanelLabel) {
  const baseUrl = window.location.href.split("#")[0].split("?")[0];
  return `${baseUrl}?window=${label}`;
}

async function getPetWindow(): Promise<WindowHandle | null> {
  return await WebviewWindow.getByLabel("pet");
}

async function getLogicalOuterPosition(win: WindowHandle) {
  const [pos, scale] = await Promise.all([
    win.outerPosition(),
    win.scaleFactor().catch(() => window.devicePixelRatio || 1),
  ]);
  return { x: pos.x / scale, y: pos.y / scale };
}

async function getPetAnchor() {
  const petWindow = await getPetWindow();
  if (!petWindow) return { x: 200, y: 200 };
  return getLogicalOuterPosition(petWindow);
}

async function getCursorAnchor() {
  const [cursor, scale] = await Promise.all([
    cursorPosition(),
    currentWindow.scaleFactor().catch(() => window.devicePixelRatio || 1),
  ]);
  return { x: cursor.x / scale, y: cursor.y / scale };
}

async function openPanel(label: PanelLabel, x: number, y: number) {
  const spec = PANEL_SPECS[label];
  const position = new LogicalPosition(Math.round(x), Math.round(y));
  const size = new LogicalSize(spec.width, spec.height);
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.setSize(size);
    if (label === "voice") {
      await existing.setIgnoreCursorEvents(false).catch(() => {});
      await existing.center();
    } else {
      await existing.setPosition(position);
    }
    await existing.setAlwaysOnTop(true);
    await existing.show();
    await existing.setFocus();
    return existing;
  }

  return new WebviewWindow(label, {
    url: panelUrl(label),
    ...(label === "voice" ? { center: true } : { x: position.x, y: position.y }),
    width: spec.width,
    height: spec.height,
    title: spec.title,
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

async function closePanel(label: PanelLabel) {
  const win = await WebviewWindow.getByLabel(label);
  if (label === "context-menu") {
    await win?.hide();
  } else {
    await win?.close();
  }
}

async function toggleChat() {
  const existing = await WebviewWindow.getByLabel("chat");
  if (existing) {
    await existing.close();
    return;
  }

  const pet = await getPetAnchor();
  await openPanel("chat", pet.x + PET_W + PANEL_GAP, pet.y);
}

async function openContextMenu(e: MouseEvent) {
  e.preventDefault();
  e.stopPropagation();

  const cursor = await getCursorAnchor();
  await openPanel("context-menu", cursor.x, cursor.y);
}

async function openSettingsPanel(tab = "appearance") {
  const pet = await getPetAnchor();
  const spec = PANEL_SPECS["settings"];
  const position = new LogicalPosition(Math.round(pet.x + PET_W + PANEL_GAP), Math.round(pet.y));
  const size = new LogicalSize(spec.width, spec.height);

  const existing = await WebviewWindow.getByLabel("settings");
  if (existing) {
    await existing.setSize(size);
    await existing.setPosition(position);
    await existing.setAlwaysOnTop(true);
    await existing.show();
    await existing.setFocus();
    await existing.emit("switch-tab", tab);
    await closePanel("context-menu");
    return;
  }

  const baseUrl = window.location.href.split("#")[0].split("?")[0];
  const url = `${baseUrl}?window=settings&tab=${tab}`;

  new WebviewWindow("settings", {
    url,
    x: position.x,
    y: position.y,
    width: spec.width,
    height: spec.height,
    title: spec.title,
    transparent: true,
    decorations: false,
    alwaysOnTop: true,
    resizable: false,
    skipTaskbar: true,
    shadow: false,
    focus: true,
    parent: "pet",
  });

  await closePanel("context-menu");
}

async function openVoicePanel() {
  const voicePanel = await openPanel("voice", 0, 0);
  await closePanel("context-menu");
  window.setTimeout(() => {
    void voicePanel.emit("voice-start-listening").catch(() => {});
  }, 150);
}

async function openToolConfirmPanel(payload: ToolConfirmPayload) {
  const spec = PANEL_SPECS["tool-confirm"];
  const baseUrl = window.location.href.split("#")[0].split("?")[0];
  const url = `${baseUrl}?window=tool-confirm`;
  const size = new LogicalSize(spec.width, spec.height);
  localStorage.setItem(TOOL_CONFIRM_PAYLOAD_KEY, JSON.stringify(payload));
  // 🔥 智能弹窗: 仅在 chat/main 窗口获焦时才置顶+抢焦, 否则仅显示+通知
  const focused = await isChatWindowFocused();
  const existing = await WebviewWindow.getByLabel("tool-confirm");
  if (existing) {
    await existing.setSize(size);
    await existing.center();
    if (focused) {
      await existing.setAlwaysOnTop(true);
      await existing.setFocus();
    } else {
      await existing.setAlwaysOnTop(false);
    }
    await existing.show();
    if (!focused) {
      // 后台: 任务栏闪烁 + 系统通知
      await requestUserAttentionSafely(existing);
      await sendToolConfirmNotification(payload);
    }
    await existing.emit("tool-confirm-payload", payload);
    return;
  }

  new WebviewWindow("tool-confirm", {
    url,
    width: spec.width,
    height: spec.height,
    title: spec.title,
    center: true,
    transparent: false,
    decorations: false,
    alwaysOnTop: focused,
    resizable: false,
    skipTaskbar: !focused,
    shadow: true,
    focus: focused,
  });
  // 通知必须等窗口建好后调用, 否则 IPC 会因目标窗口不存在而失败
  if (!focused) {
    setTimeout(async () => {
      const win = await WebviewWindow.getByLabel("tool-confirm");
      if (win) {
        await requestUserAttentionSafely(win);
        await sendToolConfirmNotification(payload);
      }
    }, 200);
  }
}

/// 当前窗口是否是 chat 或 main (用户在使用 AI 对话)
async function isChatWindowFocused(): Promise<boolean> {
  try {
    const win = getCurrentWindow();
    const focused = await win.isFocused();
    if (!focused) return false;
    return win.label === "chat" || win.label === "main";
  } catch {
    return false;
  }
}

async function requestUserAttentionSafely(win: WebviewWindow) {
  try {
    await win.requestUserAttention(UserAttentionType.Critical);
  } catch {
    // 某些平台/窗口不支持, 静默失败
  }
}

async function sendToolConfirmNotification(payload: ToolConfirmPayload) {
  try {
    await invoke("send_tool_confirm_notification", {
      summary: payload.summary,
      toolName: payload.tool_name,
    });
  } catch (err) {
    console.warn("send_tool_confirm_notification failed:", err);
  }
}

async function closeCurrentWindow() {
  if (currentLabel === "voice") {
    await currentWindow.setIgnoreCursorEvents(true).catch(() => {});
  }
  if (currentLabel === "context-menu") {
    await currentWindow.hide();
  } else {
    await currentWindow.close();
  }
}

async function repositionOpenPanels() {
  if (currentLabel !== "pet") return;

  const pet = await getPetAnchor();
  const [chat, settings] = await Promise.all([
    WebviewWindow.getByLabel("chat"),
    WebviewWindow.getByLabel("settings"),
  ]);

  const panelPosition = new LogicalPosition(pet.x + PET_W + PANEL_GAP, pet.y);
  await Promise.all([
    chat?.setPosition(panelPosition),
    settings?.setPosition(panelPosition),
  ]);
}

async function hydrateAppearance() {
  try {
    petStore.skin = resolveSkinId(await invoke<string>("get_skin"));
  } catch {
    // Keep local defaults if persisted settings are unavailable.
  }

  try {
    petStore.fontColor = await invoke<string>("get_font_color");
  } catch {
    // Keep local defaults if persisted settings are unavailable.
  }

  try {
    petStore.userAvatar = await invoke<string>("get_user_avatar");
  } catch {
    // Keep local defaults if persisted settings are unavailable.
  }

  try {
    petStore.character = resolvePetCharacterId(await invoke<string>("get_setting_value", {
      key: PET_CHARACTER_SETTING_KEY,
    }));
  } catch {
    // Keep local defaults if persisted settings are unavailable.
  }

  try {
    petStore.customPixelPetAsset = await getActiveCustomPixelPetAsset();
    if (petStore.character === "custom-pixel" && !petStore.customPixelPetAsset) {
      petStore.character = "classic";
      await setActiveCustomPixelPetAsset(null).catch(() => null);
    }
  } catch {
    petStore.customPixelPetAsset = null;
    if (petStore.character === "custom-pixel") {
      petStore.character = "classic";
    }
  }
}

async function loadVoiceSettings(): Promise<VoiceSettings> {
  try {
    return parseVoiceSettings(await invoke<string>("get_setting_value", {
      key: VOICE_SETTINGS_KEY,
    }));
  } catch {
    return { ...DEFAULT_VOICE_SETTINGS };
  }
}

async function registerVoiceShortcut() {
  if (currentLabel !== "main") return;

  const settings = await loadVoiceSettings();
  const shortcut = (settings.shortcut || DEFAULT_VOICE_SETTINGS.shortcut).trim();
  if (shortcut === registeredVoiceShortcut) return;

  if (!shortcut) {
    if (registeredVoiceShortcut) {
      await unregister(registeredVoiceShortcut).catch(() => {});
      registeredVoiceShortcut = "";
    }
    return;
  }

  const previousShortcut = registeredVoiceShortcut;

  try {
    await register(shortcut, (event) => {
      if (event.state === "Pressed") {
        void openVoicePanel();
      }
    });
    registeredVoiceShortcut = shortcut;
    if (previousShortcut) {
      await unregister(previousShortcut).catch(() => {});
    }
  } catch (err) {
    console.warn("Failed to register voice shortcut", err);
  }
}

function queueVoiceShortcutRegistration() {
  voiceShortcutRegistration = voiceShortcutRegistration
    .catch(() => {})
    .then(() => registerVoiceShortcut());
  void voiceShortcutRegistration;
}

async function unregisterVoiceShortcut() {
  await voiceShortcutRegistration.catch(() => {});
  if (registeredVoiceShortcut) {
    await unregister(registeredVoiceShortcut).catch(() => {});
    registeredVoiceShortcut = "";
  }
}

function isPetBodyPoint(px: number, py: number) {
  return isPetCanvasPoint(px, py, petStore.character, petStore.state, petStore.position);
}

async function setPetWindowIgnoresCursorEvents(ignore: boolean) {
  if (lastIgnoreCursorEvents === ignore) return;

  try {
    await currentWindow.setIgnoreCursorEvents(ignore);
    lastIgnoreCursorEvents = ignore;
  } catch {
    lastIgnoreCursorEvents = null;
  }
}

async function updatePetWindowHitTest() {
  if (currentLabel !== "pet") return;

  if (isDraggingPet.value) {
    await setPetWindowIgnoresCursorEvents(false);
    return;
  }

  const scale = window.devicePixelRatio || 1;
  const [cursor, position] = await Promise.all([cursorPosition(), currentWindow.outerPosition()]);
  const px = (cursor.x - position.x) / scale;
  const py = (cursor.y - position.y) / scale;
  await setPetWindowIgnoresCursorEvents(!isPetBodyPoint(px, py));
}

function setPetDragging(dragging: boolean) {
  isDraggingPet.value = dragging;
  updatePetWindowHitTest();
}

// === 桌宠窗口管理 (从 Dashboard 触发) ===
async function openPetWindow() {
  const existing = await WebviewWindow.getByLabel("pet");
  if (existing) {
    await existing.show();
    await existing.setFocus();
    return;
  }

  const baseUrl = window.location.href.split("#")[0].split("?")[0];
  const url = `${baseUrl}?window=pet`;

  new WebviewWindow("pet", {
    url,
    width: 120,
    height: 140,
    transparent: true,
    decorations: false,
    alwaysOnTop: true,
    resizable: false,
    skipTaskbar: true,
    shadow: false,
    focus: false,
    title: "AI Desktop Pet",
  });
}

async function closePetWindow() {
  // Close child panels first
  await Promise.all([
    closePanel("chat"),
    closePanel("context-menu"),
    closePanel("settings"),
  ]);
  const petWin = await WebviewWindow.getByLabel("pet");
  if (petWin) {
    await petWin.close();
  }
  // 通知控制台
  await currentWindow.emit("pet-window-closed");
}

async function openDashboard() {
  const mainWin = await WebviewWindow.getByLabel("main");
  if (mainWin) {
    await mainWin.show();
    await mainWin.setFocus();
  }
  await closePanel("context-menu");
}

onMounted(async () => {
  await hydrateAppearance();
  unlistenAppearanceChanged = await currentWindow.listen("appearance-changed", () => {
    void hydrateAppearance();
  });

  if (currentLabel === "context-menu") {
    unlistenFocusChanged = await currentWindow.onFocusChanged(({ payload: focused }) => {
      if (!focused) currentWindow.hide();
    });
  }

  // Pet window: hit test + panel repositioning
  if (currentLabel === "pet") {
    unlistenOpenVoice = await listen("open-voice-panel", () => {
      void openVoicePanel();
    });

    unlistenMoved = await currentWindow.onMoved(() => {
      repositionOpenPanels();
    });
    updatePetWindowHitTest();
    cursorHitTestTimer = setInterval(updatePetWindowHitTest, 50);
  }

  // Main (Dashboard) window: voice shortcut registration
  if (currentLabel === "main") {
    unlistenOpenVoice = await listen("open-voice-panel", () => {
      void openVoicePanel();
    });
    unlistenVoiceSettingsChanged = await listen("voice-settings-changed", () => {
      queueVoiceShortcutRegistration();
    });
    unlistenToolConfirm = await listen<ToolConfirmPayload>("ai-tool-confirm", (event) => {
      void openToolConfirmPanel(event.payload);
    });
    window.addEventListener("voice-settings-changed", queueVoiceShortcutRegistration);
    queueVoiceShortcutRegistration();

    // Auto-launch pet on startup
    setTimeout(() => {
      openPetWindow();
    }, 500);
  }
});

onUnmounted(() => {
  if (cursorHitTestTimer) clearInterval(cursorHitTestTimer);
  unlistenAppearanceChanged?.();
  unlistenFocusChanged?.();
  unlistenMoved?.();
  unlistenOpenVoice?.();
  unlistenVoiceSettingsChanged?.();
  unlistenToolConfirm?.();
  if (currentLabel === "main") {
    window.removeEventListener("voice-settings-changed", queueVoiceShortcutRegistration);
    void unregisterVoiceShortcut();
  }
  if (currentLabel === "pet") {
    setPetWindowIgnoresCursorEvents(false);
  }
});
</script>

<template>
  <div :class="['window-root', `window-${currentLabel}`]">
    <!-- Dashboard 控制台 (main 窗口) -->
    <Dashboard
      v-if="currentLabel === 'main'"
      @open-pet="openPetWindow"
      @close-pet="closePetWindow"
    />

    <!-- 桌宠本体 (pet 子窗口) -->
    <PetCanvas
      v-else-if="currentLabel === 'pet'"
      @click="toggleChat"
      @contextmenu="openContextMenu"
      @dragging="setPetDragging"
    />

    <ChatBubble
      v-else-if="currentLabel === 'chat'"
      @close="closeCurrentWindow"
    />

    <ContextMenu
      v-else-if="currentLabel === 'context-menu'"
      @close="closeCurrentWindow"
      @open-settings="openSettingsPanel"
      @open-dashboard="openDashboard"
    />

    <Settings
      v-else-if="currentLabel === 'settings'"
      @close="closeCurrentWindow"
    />

    <VoicePanel
      v-else-if="currentLabel === 'voice'"
      @close="closeCurrentWindow"
    />

    <ToolConfirmPanel
      v-else-if="currentLabel === 'tool-confirm'"
      @close="closeCurrentWindow"
    />
  </div>
</template>

<style>
html,
body,
#app {
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: transparent;
}

.window-root {
  width: 100vw;
  height: 100vh;
  position: relative;
  user-select: none;
  overflow: hidden;
}

/* Dashboard 窗口不需要透明背景 */
.window-main {
  background: var(--dash-shell-bg, #f8fbff);
}
</style>
