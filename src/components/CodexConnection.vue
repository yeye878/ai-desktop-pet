<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

defineProps<{ modelSaved: boolean }>();
defineEmits<{ reset: [] }>();

interface CodexStatus {
  installed: boolean;
  logged_in: boolean;
  version: string | null;
  executable: string | null;
  workspace: string;
  message: string;
}

const status = ref<CodexStatus | null>(null);
const loading = ref(false);
const opening = ref(false);
const error = ref("");
const statusLabel = computed(() => {
  if (loading.value) return "检测中…";
  if (error.value) return "检测失败";
  if (!status.value?.installed) return "未安装 / 无法启动";
  return status.value.logged_in ? "已连接" : "未登录";
});

async function refreshStatus() {
  if (loading.value) return;
  loading.value = true;
  error.value = "";
  try {
    status.value = await invoke<CodexStatus>("check_codex_status");
  } catch (cause) {
    status.value = null;
    error.value = `检测 Codex 失败：${cause}`;
  } finally {
    loading.value = false;
  }
}

async function openLogin() {
  opening.value = true;
  error.value = "";
  try {
    await invoke("open_codex_config");
  } catch (cause) {
    error.value = `打开登录终端失败：${cause}`;
  } finally {
    opening.value = false;
  }
}

onMounted(() => {
  void refreshStatus();
  window.addEventListener("focus", refreshStatus);
});
onUnmounted(() => window.removeEventListener("focus", refreshStatus));
</script>

<template>
  <div class="codex-connection">
    <div class="codex-status" aria-live="polite">
      <span>本机 Codex</span>
      <strong :class="{ connected: status?.logged_in && !error && !loading }">{{ statusLabel }}</strong>
    </div>
    <p class="codex-hint">{{ error || status?.message || "正在检查本机 Codex CLI…" }}</p>
    <dl v-if="status?.installed" class="codex-details">
      <div><dt>CLI 版本</dt><dd>{{ status.version }}</dd></div>
      <div><dt>模型</dt><dd>跟随本机 Codex 配置</dd></div>
      <div><dt>程序路径</dt><dd>{{ status.executable }}</dd></div>
      <div><dt>工作目录</dt><dd>{{ status.workspace }}</dd></div>
    </dl>
    <p class="codex-hint">自动保持对话上下文，复用本机登录，无需在桌宠中填写 API Key。使用工作目录写入沙箱；需要额外权限的命令会被拒绝。</p>
    <p v-if="status && !status.installed" class="codex-hint">安装命令：<code>npm install -g @openai/codex</code></p>
    <div class="codex-actions">
      <button type="button" @click="$emit('reset')">{{ modelSaved ? "已重置" : "重置会话" }}</button>
      <button type="button" :disabled="opening || !status?.installed" @click="openLogin">{{ opening ? "正在打开…" : "登录 Codex" }}</button>
      <button type="button" :disabled="loading" @click="refreshStatus">刷新状态</button>
    </div>
  </div>
</template>

<style scoped>
.codex-connection { color: var(--dash-text, #334155); font-size: 13px; }
.codex-status { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.codex-status strong { color: #64748b; font-size: 12px; }
.codex-status strong.connected { color: #15803d; }
.codex-hint { margin: 10px 0; color: var(--dash-text-secondary, #64748b); font-size: 12px; line-height: 1.7; overflow-wrap: anywhere; }
.codex-details { display: grid; gap: 10px; padding: 14px; margin: 12px 0; border: 1px solid rgba(24, 42, 72, 0.08); border-radius: 10px; background: rgba(255, 255, 255, 0.4); }
.codex-details div { display: grid; grid-template-columns: 64px minmax(0, 1fr); gap: 10px; }
.codex-details dt { color: #64748b; }
.codex-details dd { margin: 0; overflow-wrap: anywhere; }
.codex-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 12px; }
.codex-actions button { padding: 8px 12px; border: 1px solid rgba(24, 42, 72, 0.12); border-radius: 8px; color: inherit; background: rgba(255, 255, 255, 0.7); font: inherit; cursor: pointer; }
.codex-actions button:hover:not(:disabled) { border-color: var(--pet-primary, #38bdf8); }
.codex-actions button:disabled { cursor: default; opacity: 0.5; }
</style>
