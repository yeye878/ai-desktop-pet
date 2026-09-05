<script setup lang="ts">
import { useChatStore } from "../stores/chat";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import AppIcon from "./AppIcon.vue";

const emit = defineEmits<{ close: []; openSettings: [tab: string]; openDashboard: [] }>();
const chat = useChatStore();
const currentWindow = getCurrentWindow();

async function action(name: string) {
  switch (name) {
    case "happy":
      await invoke("set_pet_state", { newState: "happy" });
      break;
    case "sleep":
      await invoke("set_pet_state", { newState: "sleeping" });
      break;
    case "wake":
      await invoke("set_pet_state", { newState: "idle" });
      break;
    case "wave":
      await invoke("set_pet_state", { newState: "waving" });
      break;
    case "new-chat":
      await invoke("start_new_conversation");
      chat.isLoading = false;
      if (chat.messages.length > 0) {
        chat.addSystemMessage("新对话");
      }
      break;
    case "clear":
      await invoke("clear_chat_history");
      chat.clearMessages();
      await currentWindow.emit("chat-history-cleared");
      break;
    case "dashboard":
      emit("openDashboard");
      return;
    case "hide":
      // 收起桌宠（关闭 pet 窗口，控制台保留）
      const petWin = await WebviewWindow.getByLabel("pet");
      if (petWin) {
        await currentWindow.emit("pet-window-closed");
        await petWin.close();
      }
      return;
  }
  emit("close");
}
</script>

<template>
  <div class="context-menu">
    <div class="menu-item" @click="action('wave')">
      <span class="menu-icon"><AppIcon name="wave" :size="13" /></span>
      <span>打个招呼</span>
    </div>
    <div class="menu-item" @click="action('happy')">
      <span class="menu-icon"><AppIcon name="smile" :size="13" /></span>
      <span>开心一下</span>
    </div>
    <div class="menu-item" @click="action('sleep')">
      <span class="menu-icon"><AppIcon name="moon" :size="13" /></span>
      <span>去睡觉</span>
    </div>
    <div class="menu-item" @click="action('wake')">
      <span class="menu-icon"><AppIcon name="bell" :size="13" /></span>
      <span>叫醒它</span>
    </div>
    <div class="menu-divider" />
    <div class="menu-item" @click="action('new-chat')">
      <span class="menu-icon"><AppIcon name="plus" :size="13" /></span>
      <span>新对话</span>
    </div>
    <div class="menu-item" @click="action('clear')">
      <span class="menu-icon"><AppIcon name="trash" :size="13" /></span>
      <span>清空对话</span>
    </div>
    <div class="menu-divider" />
    <div class="menu-item" @click="action('dashboard')">
      <span class="menu-icon"><AppIcon name="home" :size="13" /></span>
      <span>打开控制台</span>
    </div>
    <div class="menu-item exit-item" @click="action('hide')">
      <span class="menu-icon"><AppIcon name="minus" :size="13" /></span>
      <span>收起桌宠</span>
    </div>
  </div>
</template>

<style scoped>
.context-menu {
  position: absolute;
  pointer-events: auto;
  left: 0;
  top: 0;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  background: var(--dash-panel-solid, #fffefb);
  border-radius: 14px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.1));
  box-shadow:
    0 2px 6px rgba(48, 42, 34, 0.06),
    0 18px 44px rgba(48, 42, 34, 0.16);
  padding: 5px;
  z-index: 201;
  min-width: 150px;
  overflow-y: auto;
  animation: menuPop 0.18s cubic-bezier(0.22, 1, 0.36, 1);
  font-family: var(--dash-font-sans, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif);
  color: var(--dash-text-secondary, #6d6558);
}

@keyframes menuPop {
  from {
    opacity: 0;
    transform: scale(0.96) translateY(-4px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

.menu-item {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 7px 11px;
  font-size: 12px;
  font-weight: 550;
  cursor: pointer;
  border-radius: 9px;
  transition: background 140ms ease-out, color 140ms ease-out;
}

.menu-item:hover {
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  color: var(--dash-text-primary, #2d2922);
}

.menu-item:active {
  transform: scale(0.985);
}

.exit-item:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
}

.menu-icon {
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-muted, #a29a8a);
  flex-shrink: 0;
  transition: background 140ms ease-out, color 140ms ease-out;
}

.menu-item:hover .menu-icon {
  background: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.14);
  color: var(--dash-accent, #bf7a4e);
}

.exit-item:hover .menu-icon {
  background: rgba(192, 90, 77, 0.14);
  color: var(--dash-danger, #c05a4d);
}

.menu-divider {
  height: 1px;
  background: var(--dash-divider, rgba(63, 54, 44, 0.08));
  margin: 4px 8px;
}

@media (prefers-reduced-motion: reduce) {
  .context-menu,
  .menu-item {
    animation: none !important;
    transition-duration: 1ms !important;
    transform: none !important;
  }
}
</style>
