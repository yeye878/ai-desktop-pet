<script setup lang="ts">
import { useChatStore } from "../stores/chat";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const emit = defineEmits<{ close: []; openSettings: [] }>();
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
    case "clear":
      await invoke("clear_chat_history");
      chat.clearMessages();
      await currentWindow.emit("chat-history-cleared");
      break;
    case "settings":
      emit("openSettings");
      return;
    case "exit":
      await invoke("exit_app");
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
    <div class="menu-item" @click="action('clear')">
      <span class="menu-icon">&#x1F5D1;&#xFE0F;</span>
      <span>清空对话</span>
    </div>
    <div class="menu-item" @click="action('settings')">
      <span class="menu-icon">&#x2699;&#xFE0F;</span>
      <span>设置</span>
    </div>
    <div class="menu-divider" />
    <div class="menu-item exit-item" @click="action('exit')">
      <span class="menu-icon">&#x274C;</span>
      <span>退出桌宠</span>
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
  background: linear-gradient(135deg, #f43f5e, #e11d48);
  box-shadow: 0 8px 18px rgba(225, 29, 72, 0.3);
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
</style>
