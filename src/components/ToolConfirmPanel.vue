<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

const emit = defineEmits<{ close: [] }>();
const TOOL_CONFIRM_PAYLOAD_KEY = "ai-desktop-pet.tool-confirm-payload";

type ToolConfirmPayload = {
  id: string;
  tool_name: string;
  arguments: string;
  summary?: string;
  command?: string | null;
  path?: string | null;
};

const currentWindow = getCurrentWindow();
const pendingConfirm = ref<ToolConfirmPayload | null>(readInitialPayload());
const isResolving = ref(false);
let unlistenPayload: UnlistenFn | null = null;
let unlistenResolved: UnlistenFn | null = null;

function readInitialPayload() {
  const stored = localStorage.getItem(TOOL_CONFIRM_PAYLOAD_KEY);
  if (stored) {
    localStorage.removeItem(TOOL_CONFIRM_PAYLOAD_KEY);
    try {
      return JSON.parse(stored) as ToolConfirmPayload;
    } catch {
      // Fall through to URL compatibility below.
    }
  }

  const raw = new URLSearchParams(window.location.search).get("payload");
  if (!raw) return null;
  try {
    return JSON.parse(raw) as ToolConfirmPayload;
  } catch {
    return null;
  }
}

async function resolveConfirm(approved: boolean) {
  if (!pendingConfirm.value) {
    emit("close");
    return;
  }
  if (isResolving.value) return;
  isResolving.value = true;
  try {
    await invoke("confirm_tool", {
      id: pendingConfirm.value.id,
      approved,
    });
  } finally {
    pendingConfirm.value = null;
    emit("close");
  }
}

onMounted(async () => {
  unlistenPayload = await listen<ToolConfirmPayload>("tool-confirm-payload", (event) => {
    pendingConfirm.value = event.payload;
    isResolving.value = false;
  });

  unlistenResolved = await listen<{ id: string; approved: boolean }>(
    "ai-tool-confirm-resolved",
    (event) => {
      if (pendingConfirm.value?.id === event.payload.id) {
        pendingConfirm.value = null;
        emit("close");
      }
    },
  );

  await currentWindow.setAlwaysOnTop(true).catch(() => {});
  await currentWindow.setFocus().catch(() => {});
});

onBeforeUnmount(() => {
  unlistenPayload?.();
  unlistenResolved?.();
});
</script>

<template>
  <section class="tool-confirm-window" data-tauri-drag-region>
    <div class="tool-confirm-titlebar" data-tauri-drag-region>
      <div>
        <span class="tool-confirm-kicker">Tool Request</span>
        <h1>确认工具调用</h1>
      </div>
      <button class="icon-btn" title="拒绝并关闭" @click="resolveConfirm(false)">×</button>
    </div>

    <div v-if="pendingConfirm" class="tool-confirm-body">
      <div class="summary-panel">
        <span class="summary-label">操作</span>
        <strong>{{ pendingConfirm.summary || pendingConfirm.tool_name }}</strong>
      </div>

      <div v-if="pendingConfirm.command" class="detail-row">
        <span>命令</span>
        <code>{{ pendingConfirm.command }}</code>
      </div>
      <div v-if="pendingConfirm.path" class="detail-row">
        <span>路径</span>
        <code>{{ pendingConfirm.path }}</code>
      </div>

      <div class="args-block">
        <span>参数</span>
        <pre>{{ pendingConfirm.arguments }}</pre>
      </div>
    </div>

    <div v-else class="empty-state">
      <strong>等待工具请求</strong>
      <span>如果请求已处理，此窗口会自动关闭。</span>
    </div>

    <footer class="tool-confirm-actions">
      <button class="deny-btn" :disabled="!pendingConfirm || isResolving" @click="resolveConfirm(false)">
        拒绝
      </button>
      <button class="approve-btn" :disabled="!pendingConfirm || isResolving" @click="resolveConfirm(true)">
        允许
      </button>
    </footer>
  </section>
</template>

<style scoped>
.tool-confirm-window {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background:
    linear-gradient(135deg, rgba(255, 255, 255, 0.98), rgba(239, 251, 255, 0.94)),
    #ffffff;
  color: #1d2536;
  font-family: "Inter", "Segoe UI", ui-sans-serif, system-ui, sans-serif;
  border: 1px solid rgba(24, 42, 72, 0.14);
  box-shadow: 0 18px 44px rgba(24, 42, 72, 0.18);
  user-select: none;
}

.tool-confirm-titlebar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 16px 12px;
  border-bottom: 1px solid rgba(24, 42, 72, 0.08);
  -webkit-app-region: drag;
}

.tool-confirm-kicker {
  display: block;
  margin-bottom: 2px;
  color: #64748b;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

h1 {
  margin: 0;
  font-size: 18px;
  line-height: 1.2;
}

.icon-btn {
  width: 30px;
  height: 30px;
  border: 1px solid rgba(24, 42, 72, 0.10);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.78);
  color: #64748b;
  cursor: pointer;
  font-size: 20px;
  line-height: 1;
  -webkit-app-region: no-drag;
}

.icon-btn:hover {
  border-color: rgba(239, 68, 68, 0.30);
  color: #dc2626;
}

.tool-confirm-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 14px 16px;
  display: grid;
  gap: 10px;
}

.summary-panel,
.detail-row,
.args-block {
  border: 1px solid rgba(24, 42, 72, 0.08);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.72);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.70);
}

.summary-panel {
  padding: 12px;
  display: grid;
  gap: 5px;
}

.summary-label,
.detail-row span,
.args-block span {
  color: #64748b;
  font-size: 11px;
  font-weight: 700;
}

.summary-panel strong {
  font-size: 14px;
  line-height: 1.45;
}

.detail-row {
  padding: 9px 11px;
  display: grid;
  gap: 5px;
}

code,
pre {
  color: #1e293b;
  font-family: Consolas, Monaco, ui-monospace, monospace;
  font-size: 11px;
  line-height: 1.45;
  word-break: break-word;
  white-space: pre-wrap;
}

.args-block {
  min-height: 94px;
  padding: 10px 11px;
  display: grid;
  gap: 6px;
}

.args-block pre {
  max-height: 116px;
  overflow: auto;
  margin: 0;
  padding: 8px;
  border-radius: 6px;
  background: rgba(15, 23, 42, 0.05);
}

.empty-state {
  flex: 1;
  display: grid;
  place-content: center;
  gap: 6px;
  color: #64748b;
  text-align: center;
}

.empty-state strong {
  color: #1d2536;
}

.tool-confirm-actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  padding: 12px 16px 16px;
  border-top: 1px solid rgba(24, 42, 72, 0.08);
}

.tool-confirm-actions button {
  height: 40px;
  border-radius: 8px;
  border: 1px solid transparent;
  cursor: pointer;
  font-weight: 700;
}

.tool-confirm-actions button:disabled {
  opacity: 0.48;
  cursor: not-allowed;
}

.deny-btn {
  background: rgba(239, 68, 68, 0.08);
  border-color: rgba(239, 68, 68, 0.18) !important;
  color: #dc2626;
}

.approve-btn {
  background: #1f9d55;
  color: #ffffff;
  box-shadow: 0 10px 20px rgba(31, 157, 85, 0.20);
}

/* Product polish layer */
.tool-confirm-window {
  background:
    url("../assets/art/paper-grain.webp"),
    radial-gradient(circle at 8% 0%, rgba(255, 216, 92, 0.18), transparent 32%),
    linear-gradient(145deg, rgba(255, 255, 255, 0.98), rgba(241, 250, 255, 0.94));
  background-size: 420px 420px, auto, auto;
  border-color: rgba(33, 48, 74, 0.12);
  box-shadow:
    0 18px 42px rgba(33, 48, 74, 0.18),
    inset 0 1px 0 rgba(255, 255, 255, 0.78);
}

.tool-confirm-titlebar {
  padding: 15px 16px 12px;
  border-bottom-color: rgba(33, 48, 74, 0.09);
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.52), transparent);
}

.tool-confirm-kicker {
  color: #7a8698;
  letter-spacing: 0.06em;
}

h1 {
  color: #20293a;
  font-weight: 760;
}

.icon-btn {
  border-color: rgba(33, 48, 74, 0.10);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.76);
  box-shadow: 0 8px 18px rgba(33, 48, 74, 0.06);
}

.icon-btn:hover {
  background: #ffffff;
  box-shadow: 0 10px 20px rgba(239, 68, 68, 0.10);
}

.summary-panel,
.detail-row,
.args-block {
  border-color: rgba(33, 48, 74, 0.10);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.72);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.72),
    0 8px 18px rgba(33, 48, 74, 0.055);
}

.summary-panel strong {
  color: #20293a;
}

.args-block pre {
  border: 1px solid rgba(33, 48, 74, 0.08);
  border-radius: 10px;
  background: rgba(33, 48, 74, 0.045);
}

.tool-confirm-actions {
  border-top-color: rgba(33, 48, 74, 0.09);
  background: rgba(255, 255, 255, 0.36);
}

.tool-confirm-actions button {
  border-radius: 10px;
  transition:
    transform 0.16s ease,
    border-color 0.16s ease,
    background 0.16s ease,
    box-shadow 0.16s ease,
    opacity 0.16s ease;
}

.tool-confirm-actions button:hover:not(:disabled) {
  transform: translateY(-1px);
}

.tool-confirm-actions button:active:not(:disabled) {
  transform: translateY(1px);
}

.deny-btn {
  background: rgba(239, 68, 68, 0.08);
}

.deny-btn:hover:not(:disabled) {
  background: #ef4444;
  color: #ffffff;
  box-shadow: 0 10px 20px rgba(239, 68, 68, 0.16);
}

.approve-btn {
  background: linear-gradient(135deg, #1f9d55, #20b866);
  box-shadow: 0 10px 22px rgba(31, 157, 85, 0.18);
}

.approve-btn:hover:not(:disabled) {
  box-shadow: 0 14px 26px rgba(31, 157, 85, 0.24);
}

@media (prefers-reduced-motion: reduce) {
  .tool-confirm-actions button,
  .icon-btn {
    transition-duration: 1ms !important;
    transform: none !important;
  }
}
</style>
