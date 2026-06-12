<script setup lang="ts">
import { useChatStore } from "../stores/chat";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

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
      <span class="menu-icon">&#x1F44B;</span>
      <span>打个招呼</span>
    </div>
    <div class="menu-item" @click="action('happy')">
      <span class="menu-icon">&#x1F60A;</span>
      <span>开心一下</span>
    </div>
    <div class="menu-item" @click="action('sleep')">
      <span class="menu-icon">&#x1F634;</span>
      <span>去睡觉</span>
    </div>
    <div class="menu-item" @click="action('wake')">
      <span class="menu-icon">&#x23F0;</span>
      <span>叫醒它</span>
    </div>
    <div class="menu-divider" />
    <div class="menu-item" @click="action('new-chat')">
      <span class="menu-icon">＋</span>
      <span>新对话</span>
    </div>
    <div class="menu-item" @click="action('clear')">
      <span class="menu-icon">&#x1F5D1;&#xFE0F;</span>
      <span>清空对话</span>
    </div>
    <div class="menu-divider" />
    <div class="menu-item" @click="action('dashboard')">
      <span class="menu-icon">🏠</span>
      <span>打开控制台</span>
    </div>
    <div class="menu-item exit-item" @click="action('hide')">
      <span class="menu-icon">👋</span>
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
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.88), rgba(255, 255, 255, 0.72)),
    var(--pet-bg-glass, rgba(255, 255, 255, 0.95));
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border-radius: 13px;
  box-shadow:
    0 16px 36px rgba(15, 23, 42, 0.18),
    0 4px 12px rgba(15, 23, 42, 0.08),
    inset 0 1px 0 rgba(255, 255, 255, 0.5);
  border: 1px solid rgba(255, 255, 255, 0.58);
  padding: 6px;
  z-index: 201;
  min-width: 150px;
  overflow-y: auto;
  animation: menuPop 0.2s ease-out;
}

@keyframes menuPop {
  from {
    opacity: 0;
    transform: scale(0.92) translateY(-4px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

.menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  font-size: 13px;
  cursor: pointer;
  border-radius: 9px;
  transition: all 0.15s;
  color: var(--pet-font-color, #475569);
}

.menu-item:hover {
  background: var(--pet-header-gradient, linear-gradient(135deg, #ff6b6b, #ff8e53));
  background-size: 180% 180%;
  color: white;
  transform: translateX(3px);
  box-shadow: 0 8px 18px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.22);
}

.exit-item:hover {
  background: linear-gradient(135deg, #f59e0b, #d97706);
  box-shadow: 0 8px 18px rgba(217, 119, 6, 0.3);
}

.menu-icon {
  font-size: 15px;
  width: 20px;
  text-align: center;
}

.menu-divider {
  height: 1px;
  background: rgba(15, 23, 42, 0.07);
  margin: 4px 8px;
}

/* Product polish layer */
.context-menu {
  border-radius: 14px;
  border-color: rgba(33, 48, 74, 0.11);
  background:
    url("../assets/art/paper-grain.webp"),
    linear-gradient(180deg, rgba(255, 255, 255, 0.98), rgba(246, 252, 255, 0.92)),
    var(--pet-bg-glass, rgba(255, 255, 255, 0.95));
  background-size: 380px 380px, auto, auto;
  color: #334155;
  box-shadow:
    0 18px 38px rgba(33, 48, 74, 0.18),
    0 1px 0 rgba(255, 255, 255, 0.82) inset;
}

.menu-item {
  min-height: 34px;
  border: 1px solid transparent;
  border-radius: 10px;
  color: #475569;
  font-weight: 650;
  text-shadow: none;
}

.menu-item:hover {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.20);
  background:
    linear-gradient(135deg, rgba(var(--pet-primary-rgb, 255, 107, 107), 0.92), rgba(var(--pet-accent-rgb, 255, 142, 83), 0.82));
  transform: translateY(-1px);
  box-shadow: 0 10px 20px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.18);
}

.menu-item:active {
  transform: translateY(1px);
}

.exit-item:hover {
  background: linear-gradient(135deg, #f59e0b, #ef7f47);
}

.menu-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 20px;
  border-radius: 7px;
  background: rgba(255, 255, 255, 0.46);
  color: #64748b;
  font-size: 12px;
}

.menu-item:hover .menu-icon {
  background: rgba(255, 255, 255, 0.20);
}

.menu-divider {
  background: rgba(33, 48, 74, 0.08);
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
