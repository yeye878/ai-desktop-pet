<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";

type CliBackend = "claude_code" | "codex" | "dsh";

interface PermissionOption {
  value: string;
  label: string;
  detail: string;
  danger?: boolean;
}

interface PermissionSpec {
  key: string;
  title: string;
  hint: string;
  fallback: string;
  options: PermissionOption[];
}

const props = defineProps<{ backend: CliBackend }>();

// 与 src-tauri/src/cli_permission.rs 的设置键 / 档位名保持一致。
const SPECS: Record<CliBackend, PermissionSpec> = {
  claude_code: {
    key: "claude_permission_mode",
    title: "Claude Code 权限模式",
    hint: "这里是设置页的默认权限：没有单独设置权限的会话都跟随它。会话里可在输入框旁直接用权限选择器按会话覆盖，不需要回到设置页。修改在下一轮对话生效，不改本机 Claude Code 设置。",
    fallback: "",
    options: [
      {
        value: "",
        label: "跟随本机 Claude Code 配置",
        detail: "不传 --permission-mode，完全由本机 Claude Code 自己的配置决定。",
      },
      {
        value: "auto",
        label: "自动（仅危险操作询问）",
        detail: "安全操作自动放行；风险较高的工具调用仍会弹出桌宠确认窗口。",
      },
      {
        value: "manual",
        label: "每次询问（manual）",
        detail: "所有需要授权的工具调用都会弹出确认窗口，适合严格审查。",
      },
      {
        value: "acceptEdits",
        label: "自动接受文件编辑（acceptEdits）",
        detail: "文件编辑自动放行，命令执行等高风险操作仍然询问。",
      },
      {
        value: "plan",
        label: "只读规划（plan）",
        detail: "只读探索并输出计划，不执行修改。",
      },
      {
        value: "dontAsk",
        label: "不询问（dontAsk）",
        detail: "需要授权的操作直接拒绝，不弹窗；适合无人值守的只读任务。",
      },
      {
        value: "bypassPermissions",
        label: "完全放行（bypassPermissions）",
        detail: "跳过全部权限检查，可读写整台机器。仅在隔离环境或完全信任任务时使用。",
        danger: true,
      },
    ],
  },
  codex: {
    key: "codex_sandbox_mode",
    title: "Codex 沙箱权限",
    hint: "这里是设置页的默认档位：没有单独设置权限的会话都跟随它。会话里可在输入框旁直接覆盖。Codex 的 exec 模式没有审批通道：任何档位都不会弹窗，越界操作要么被拒绝（前两档），要么直接执行（完全访问）。修改在下一轮对话生效。",
    fallback: "workspace-write",
    options: [
      {
        value: "read-only",
        label: "只读",
        detail: "只能读取文件；任何写入或修改命令都会被沙箱拒绝。",
      },
      {
        value: "workspace-write",
        label: "工作区可写（默认）",
        detail: "可写 Codex 工作目录；工作区外的操作会被拒绝，不会弹窗。",
      },
      {
        value: "danger-full-access",
        label: "完全访问（不再询问）",
        detail: "关闭沙箱，可读写整台机器。仅建议在隔离环境或完全信任任务时使用。",
        danger: true,
      },
    ],
  },
  dsh: {
    key: "dsh_permission_mode",
    title: "DeepSeek Harness 权限",
    hint: "这里是设置页的默认档位：没有单独设置权限的会话都跟随它。会话里可在输入框旁直接覆盖。决定 DSH 的沙箱范围与升权确认方式，修改在下一轮对话生效；如本机 DSH 配置里另有权限覆盖，以 DSH 自己的设置为准。",
    fallback: "",
    options: [
      {
        value: "",
        label: "跟随本机 DSH 配置",
        detail: "不覆写 DSH_PERMISSION_MODE，沿用你在 DSH 里的权限预设（默认工作区可写 + 询问）。",
      },
      {
        value: "read-only",
        label: "只读（越界询问）",
        detail: "DSH 只能读取；需要写入时会向桌宠发起升权请求。",
      },
      {
        value: "workspace-write",
        label: "工作区可写（越界询问，默认）",
        detail: "DSH 可写工作目录；越界时弹出桌宠确认窗口。",
      },
      {
        value: "workspace-write-auto",
        label: "工作区可写 + 自动批准",
        detail: "保留工作区沙箱，但升权请求由桌宠自动允许，不再弹窗。",
      },
      {
        value: "danger-full-access",
        label: "完全访问（不再询问）",
        detail: "关闭 DSH 沙箱与审批，可读写整台机器。仅建议在隔离环境或完全信任任务时使用。",
        danger: true,
      },
    ],
  },
};

const spec = computed(() => SPECS[props.backend]);
const value = ref("");
const saved = ref("");
const loading = ref(true);
const saving = ref(false);
const error = ref("");
const savedFlash = ref(false);
let flashTimer: ReturnType<typeof setTimeout> | null = null;

function normalize(raw: string): string {
  const text = (raw || "").trim();
  return spec.value.options.some((option) => option.value === text)
    ? text
    : spec.value.fallback;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const raw = await invoke<string>("get_setting_value", { key: spec.value.key });
    value.value = normalize(raw);
    saved.value = value.value;
  } catch (cause) {
    error.value = "读取权限设置失败：" + cause;
  } finally {
    loading.value = false;
  }
}

async function change(event: Event) {
  const next = (event.target as HTMLSelectElement).value;
  const previous = saved.value;
  if (next === previous) return;
  value.value = next;
  saving.value = true;
  error.value = "";
  try {
    await invoke("set_setting_value", { key: spec.value.key, value: next });
    saved.value = next;
    savedFlash.value = true;
    if (flashTimer) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      savedFlash.value = false;
    }, 1800);
  } catch (cause) {
    value.value = previous;
    error.value = "保存权限设置失败：" + cause;
  } finally {
    saving.value = false;
  }
}

const current = computed(
  () => spec.value.options.find((option) => option.value === value.value) ?? spec.value.options[0],
);

onMounted(load);
onUnmounted(() => {
  if (flashTimer) clearTimeout(flashTimer);
});
watch(() => props.backend, load);
</script>

<template>
  <div class="permission-control">
    <div class="permission-head">
      <span class="permission-title">{{ spec.title }}</span>
      <span v-if="saving" class="permission-state">保存中…</span>
      <span v-else-if="savedFlash" class="permission-state saved">已保存</span>
    </div>
    <div class="permission-row">
      <select
        class="permission-select"
        :value="value"
        :disabled="loading || saving"
        :aria-label="spec.title"
        @change="change"
      >
        <option v-for="option in spec.options" :key="option.value" :value="option.value">
          {{ option.label }}
        </option>
      </select>
      <span class="permission-detail" :class="{ danger: current?.danger }">
        {{ loading ? "读取中…" : current?.detail }}
      </span>
    </div>
    <p v-if="error" class="permission-error">{{ error }}</p>
    <p class="permission-hint">{{ spec.hint }}</p>
  </div>
</template>

<style scoped>
.permission-control {
  margin: 12px 0;
  padding: 12px 14px;
  border: 1px solid rgba(24, 42, 72, 0.1);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.45);
  color: var(--dash-text-primary, var(--dash-text, #334155));
  font-size: 12px;
}
.permission-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.permission-title {
  font-weight: 600;
}
.permission-state {
  margin-left: auto;
  color: var(--dash-text-secondary, #64748b);
  font-size: 11px;
}
.permission-state.saved {
  color: #15803d;
}
.permission-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}
.permission-select {
  flex: 0 0 auto;
  min-width: 220px;
  max-width: 100%;
  padding: 6px 8px;
  border: 1px solid rgba(24, 42, 72, 0.14);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.9);
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.permission-select:disabled {
  cursor: default;
  opacity: 0.6;
}
.permission-detail {
  flex: 1 1 220px;
  color: var(--dash-text-secondary, #64748b);
  line-height: 1.6;
}
.permission-detail.danger {
  color: #b45309;
}
.permission-error {
  margin: 6px 0 0;
  color: #b91c1c;
  line-height: 1.5;
}
.permission-hint {
  margin: 6px 0 0;
  color: var(--dash-text-secondary, #64748b);
  font-size: 11px;
  line-height: 1.6;
}
</style>