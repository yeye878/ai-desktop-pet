<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import AppIcon from "./AppIcon.vue";

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
const isCopied = ref(false);
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

const formattedArguments = computed(() => {
  if (!pendingConfirm.value?.arguments) return "";
  const raw = pendingConfirm.value.arguments.trim();
  if (!raw) return "";
  try {
    const parsed = JSON.parse(raw);
    return JSON.stringify(parsed, null, 2);
  } catch {
    return raw;
  }
});

const toolCategoryInfo = computed(() => {
  const name = pendingConfirm.value?.tool_name?.toLowerCase() || "";
  if (name.includes("command") || name.includes("shell") || name.includes("exec")) {
    return { icon: "terminal", label: "系统命令执行", isHighRisk: true };
  }
  if (name.includes("file") || name.includes("write") || name.includes("edit") || name.includes("delete")) {
    return { icon: "file", label: "文件写入 / 修改", isHighRisk: true };
  }
  if (name.includes("browser")) {
    return { icon: "globe", label: "浏览器控制", isHighRisk: false };
  }
  if (name.includes("subprocess")) {
    return { icon: "cpu", label: "子进程管理", isHighRisk: false };
  }
  return { icon: "zap", label: "敏感操作", isHighRisk: false };
});

async function copyParameters() {
  const content = pendingConfirm.value?.command || formattedArguments.value || pendingConfirm.value?.arguments || "";
  if (!content) return;
  try {
    await navigator.clipboard.writeText(content);
    isCopied.value = true;
    setTimeout(() => {
      isCopied.value = false;
    }, 1500);
  } catch {}
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
  } catch (err) {
    console.warn("confirm_tool failed:", err);
  } finally {
    pendingConfirm.value = null;
    isResolving.value = false;
    emit("close");
  }
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    void resolveConfirm(false);
  } else if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    void resolveConfirm(true);
  }
}

onMounted(async () => {
  window.addEventListener("keydown", handleKeydown);

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
  window.removeEventListener("keydown", handleKeydown);
  unlistenPayload?.();
  unlistenResolved?.();
});
</script>

<template>
  <section class="tool-confirm-window" data-tauri-drag-region>
    <!-- 顶部标题栏 -->
    <header class="tool-confirm-titlebar" data-tauri-drag-region>
      <div class="titlebar-left">
        <span class="shield-badge">
          <AppIcon name="zap" :size="14" />
        </span>
        <div class="titlebar-text">
          <span class="tool-confirm-kicker">Security Authorization</span>
          <h1>工具执行授权</h1>
        </div>
      </div>
      <button class="close-btn" title="拒绝并关闭 (Esc)" @click="resolveConfirm(false)">
        <AppIcon name="x" :size="13" />
      </button>
    </header>

    <!-- 主内容区 -->
    <div v-if="pendingConfirm" class="tool-confirm-body">
      <!-- 摘要面板 -->
      <div class="summary-card">
        <div class="summary-meta">
          <span class="category-pill">
            <AppIcon :name="toolCategoryInfo.icon" :size="12" />
            <span>{{ toolCategoryInfo.label }}</span>
          </span>
          <span v-if="toolCategoryInfo.isHighRisk" class="risk-badge">需人工确认</span>
        </div>
        <strong class="summary-title">{{ pendingConfirm.summary || pendingConfirm.tool_name }}</strong>
      </div>

      <!-- 命令详情 -->
      <div v-if="pendingConfirm.command" class="detail-panel">
        <div class="detail-header">
          <span class="detail-label">执行命令</span>
          <button class="copy-inline-btn" @click="copyParameters">
            <AppIcon :name="isCopied ? 'check' : 'copy'" :size="11" />
            <span>{{ isCopied ? '已复制' : '复制' }}</span>
          </button>
        </div>
        <code class="code-box command-box">{{ pendingConfirm.command }}</code>
      </div>

      <!-- 路径详情 -->
      <div v-if="pendingConfirm.path" class="detail-panel">
        <span class="detail-label">目标路径</span>
        <code class="code-box path-box">{{ pendingConfirm.path }}</code>
      </div>

      <!-- 核心参数 -->
      <div v-if="formattedArguments" class="detail-panel">
        <div class="detail-header">
          <span class="detail-label">调用参数</span>
          <button v-if="!pendingConfirm.command" class="copy-inline-btn" @click="copyParameters">
            <AppIcon :name="isCopied ? 'check' : 'copy'" :size="11" />
            <span>{{ isCopied ? '已复制' : '复制' }}</span>
          </button>
        </div>
        <pre class="code-box args-box"><code>{{ formattedArguments }}</code></pre>
      </div>
    </div>

    <!-- 空状态 -->
    <div v-else class="empty-state">
      <div class="empty-icon-wrap">
        <AppIcon name="check" :size="20" />
      </div>
      <strong>等待工具请求</strong>
      <span>如果请求已处理，此窗口会自动关闭。</span>
    </div>

    <!-- 底部操作按钮 -->
    <footer class="tool-confirm-actions">
      <button
        class="deny-btn"
        :disabled="!pendingConfirm || isResolving"
        @click="resolveConfirm(false)"
      >
        <AppIcon name="x" :size="13" />
        <span>拒绝 (Esc)</span>
      </button>
      <button
        class="approve-btn"
        :disabled="!pendingConfirm || isResolving"
        @click="resolveConfirm(true)"
      >
        <AppIcon name="check" :size="13" />
        <span>允许运行 (Enter)</span>
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
  box-sizing: border-box;
  background:
    radial-gradient(520px 240px at 20% -10%, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.09), transparent 65%),
    var(--dash-shell-bg, #f6f4ef);
  color: var(--dash-text-primary, #2d2922);
  font-family: var(--dash-font-sans, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif);
  font-size: 12.5px;
  user-select: none;
}

.tool-confirm-titlebar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 18px 12px;
  border-bottom: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
  -webkit-app-region: drag;
  background: var(--dash-panel-solid, #fffefb);
}

.titlebar-left {
  display: flex;
  align-items: center;
  gap: 10px;
}

.shield-badge {
  width: 30px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 9px;
  background: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.14);
  color: var(--dash-accent, #bf7a4e);
}

.titlebar-text {
  display: flex;
  flex-direction: column;
}

.tool-confirm-kicker {
  color: var(--dash-accent, #bf7a4e);
  font-size: 9.5px;
  font-weight: 750;
  letter-spacing: 0.16em;
  text-transform: uppercase;
}

h1 {
  margin: 0;
  font-size: 14.5px;
  font-weight: 680;
  line-height: 1.25;
  color: var(--dash-text-primary, #2d2922);
}

.close-btn {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  cursor: pointer;
  -webkit-app-region: no-drag;
  transition: background 140ms ease-out, color 140ms ease-out;
}

.close-btn:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.12));
  color: var(--dash-danger, #c05a4d);
}

.tool-confirm-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 14px 18px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.tool-confirm-body::-webkit-scrollbar {
  width: 6px;
}
.tool-confirm-body::-webkit-scrollbar-thumb {
  background: rgba(63, 54, 44, 0.2);
  border-radius: 999px;
}

.summary-card {
  padding: 12px 14px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: 12px;
  background: var(--dash-panel-solid, #fffefb);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.05));
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.summary-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.category-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  font-weight: 600;
  color: var(--dash-accent, #bf7a4e);
}

.risk-badge {
  font-size: 10px;
  font-weight: 700;
  padding: 2px 7px;
  border-radius: 999px;
  background: rgba(192, 90, 77, 0.12);
  color: var(--dash-danger, #c05a4d);
}

.summary-title {
  font-size: 13.5px;
  font-weight: 650;
  color: var(--dash-text-primary, #2d2922);
  line-height: 1.45;
  word-break: break-all;
}

.detail-panel {
  padding: 10px 13px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: 11px;
  background: var(--dash-panel-solid, #fffefb);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.05));
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.detail-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.detail-label {
  color: var(--dash-text-muted, #a29a8a);
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.copy-inline-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: none;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 10.5px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 5px;
  transition: all 140ms ease-out;
}

.copy-inline-btn:hover {
  background: var(--dash-panel-soft, #edeae2);
  color: var(--dash-text-primary, #2d2922);
}

.code-box {
  margin: 0;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--dash-panel-sunken, #edeae2);
  color: var(--dash-text-primary, #2d2922);
  font-family: var(--dash-font-mono, Consolas, monospace);
  font-size: 11px;
  line-height: 1.5;
  word-break: break-all;
  white-space: pre-wrap;
}

.args-box {
  max-height: 130px;
  overflow-y: auto;
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  color: var(--dash-text-muted, #a29a8a);
  font-size: 11.5px;
  text-align: center;
  padding: 20px;
}

.empty-icon-wrap {
  width: 42px;
  height: 42px;
  border-radius: 50%;
  background: var(--dash-panel-soft, #edeae2);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--dash-accent, #bf7a4e);
  margin-bottom: 4px;
}

.empty-state strong {
  color: var(--dash-text-primary, #2d2922);
  font-size: 13.5px;
}

.tool-confirm-actions {
  display: grid;
  grid-template-columns: 1fr 1.3fr;
  gap: 10px;
  padding: 13px 18px 16px;
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
  background: var(--dash-panel-solid, #fffefb);
}

.tool-confirm-actions button {
  height: 38px;
  border-radius: 10px;
  border: 1px solid transparent;
  cursor: pointer;
  font-size: 12.5px;
  font-weight: 650;
  font-family: inherit;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  transition: all 140ms ease-out;
}

.tool-confirm-actions button:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.tool-confirm-actions button:active:not(:disabled) {
  transform: scale(0.98);
}

.deny-btn {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  border-color: rgba(192, 90, 77, 0.25) !important;
  color: var(--dash-danger, #c05a4d);
}

.deny-btn:hover:not(:disabled) {
  background: var(--dash-danger, #c05a4d);
  color: #fff;
}

.approve-btn {
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  box-shadow: 0 3px 12px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.3);
}

.approve-btn:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a5643c);
  box-shadow: 0 5px 16px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.36);
}

@media (prefers-reduced-motion: reduce) {
  .tool-confirm-actions button,
  .close-btn {
    transition-duration: 1ms !important;
    transform: none !important;
  }
}
</style>
