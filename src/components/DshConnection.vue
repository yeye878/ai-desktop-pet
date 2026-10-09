<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

defineProps<{ modelSaved: boolean }>();
defineEmits<{ reset: [] }>();

interface DshStatus {
  installed: boolean;
  logged_in: boolean;
  version: string | null;
  executable: string | null;
  model: string | null;
  provider: string | null;
  profile: string;
  workspace: string;
  message: string;
}

const status = ref<DshStatus | null>(null);
const loading = ref(false);
const opening = ref(false);
const error = ref("");
const statusLabel = computed(() => {
  if (loading.value) return "检测中…";
  if (error.value) return "检测失败";
  if (!status.value?.installed) return "未安装 / 无法启动";
  return status.value.logged_in ? "已连接" : "未登录";
});
const modelLabel = computed(() => {
  const current = status.value;
  if (!current || !current.model) return "跟随本机 DSH 配置";
  return current.provider ? current.model + " · " + current.provider : current.model;
});

async function refreshStatus() {
  if (loading.value) return;
  loading.value = true;
  error.value = "";
  try {
    status.value = await invoke<DshStatus>("check_dsh_status");
  } catch (cause) {
    status.value = null;
    error.value = "检测 DeepSeek Harness 失败：" + cause;
  } finally {
    loading.value = false;
  }
}

async function openConfig() {
  opening.value = true;
  error.value = "";
  try {
    await invoke("open_dsh_config");
  } catch (cause) {
    error.value = "打开 DSH 目录失败：" + cause;
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
  <div class="dsh-connection">
    <div class="dsh-status" aria-live="polite">
      <span>本机 DeepSeek Harness</span>
      <strong :class="{ connected: status?.logged_in && !error && !loading }">{{ statusLabel }}</strong>
    </div>
    <p class="dsh-hint">{{ error || status?.message || "正在检查本机 dsh…" }}</p>
    <dl v-if="status?.installed" class="dsh-details">
      <div><dt>CLI 版本</dt><dd>{{ status.version }}</dd></div>
      <div><dt>模型</dt><dd>{{ modelLabel }}</dd></div>
      <div><dt>程序入口</dt><dd>{{ status.executable }}</dd></div>
      <div><dt>桌宠 profile</dt><dd>{{ status.profile }}</dd></div>
      <div><dt>工作目录</dt><dd>{{ status.workspace }}</dd></div>
    </dl>
    <p class="dsh-hint">
      复用本机 dsh 的登录信息、模型与权限配置，无需在桌宠中填写 API Key。
      桌宠使用专属 profile：挂载 dsh-base 与 dsh-headless，并用自带 runner 替换随包的一次性应用，
      因此推理过程、工具轨迹和会话续接都和 Codex / Claude Code 一致；你自己 profile 里的实验插件不会被加载。
    </p>
    <p class="dsh-hint">
      沙箱内的命令直接执行；当模型要越出沙箱时（升权请求），桌宠会弹出与直连 API 相同的确认窗口，
      显示命令、参数和升权理由，允许后才放行。审批策略为 never 的预设不会询问，需要授权的一律直接拒绝。
    </p>
    <p v-if="status && !status.installed" class="dsh-hint">
      安装命令：<code>npm install -g @deepseek-ai/dsh</code>；配置模型：运行 <code>dsh web</code> 后在 Models 页面添加。
    </p>
    <div class="dsh-actions">
      <button type="button" @click="$emit('reset')">{{ modelSaved ? "已重置" : "重置会话" }}</button>
      <!-- 重置会清空桌宠侧的会话，下一轮 dsh 会重新开始一段对话 -->
      <button type="button" :disabled="opening || !status?.installed" @click="openConfig">
        {{ opening ? "正在打开…" : "打开 DSH 目录" }}
      </button>
      <button type="button" :disabled="loading" @click="refreshStatus">刷新状态</button>
    </div>
  </div>
</template>

<style scoped>
.dsh-connection { color: var(--dash-text, #334155); font-size: 13px; }
.dsh-status { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.dsh-status strong { color: #64748b; font-size: 12px; }
.dsh-status strong.connected { color: #15803d; }
.dsh-hint { margin: 10px 0; color: var(--dash-text-secondary, #64748b); font-size: 12px; line-height: 1.7; overflow-wrap: anywhere; }
.dsh-details { display: grid; gap: 10px; padding: 14px; margin: 12px 0; border: 1px solid rgba(24, 42, 72, 0.08); border-radius: 10px; background: rgba(255, 255, 255, 0.4); }
.dsh-details div { display: grid; grid-template-columns: 76px minmax(0, 1fr); gap: 10px; }
.dsh-details dt { color: #64748b; }
.dsh-details dd { margin: 0; overflow-wrap: anywhere; }
.dsh-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 12px; }
.dsh-actions button { padding: 8px 12px; border: 1px solid rgba(24, 42, 72, 0.12); border-radius: 8px; color: inherit; background: rgba(255, 255, 255, 0.7); font: inherit; cursor: pointer; }
.dsh-actions button:hover:not(:disabled) { border-color: var(--pet-primary, #38bdf8); }
.dsh-actions button:disabled { cursor: default; opacity: 0.5; }
</style>
