<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronUp, Check, Shield } from "@lucide/vue";
import { useSessionsStore } from "../stores/sessions";

/**
 * 会话级权限选择器：放在对话输入框 / 会话窗格里，改的是当前这条会话的
 * 本地 CLI 权限档位（chat_sessions.permission），不动设置页里的全局默认。
 * 空值 = 跟随设置页默认；设置页里的对应选择仍作为兜底。
 */
type CliBackend = "claude_code" | "codex" | "dsh";

interface PermissionOption {
  value: string;
  label: string;
  detail: string;
  danger?: boolean;
}

const props = withDefaults(
  defineProps<{
    sessionId: string;
    backend: string;
    disabled?: boolean;
    /** 弹层方向：输入框上方用 up，页头工具条用 down。 */
    placement?: "up" | "down";
    /** 窄窗格 / 小窗专用：只显示图标与当前值，省掉「权限」前缀。 */
    compact?: boolean;
    embedded?: boolean;
  }>(),
  { disabled: false, placement: "up", compact: false, embedded: false },
);

const sessions = useSessionsStore();
const open = ref(false);
const saving = ref(false);
const error = ref("");
const globalValue = ref("");
const rootRef = ref<HTMLDivElement | null>(null);

const SPECS: Record<CliBackend, { key: string; label: string; options: PermissionOption[] }> = {
  claude_code: {
    key: "claude_permission_mode",
    label: "权限",
    options: [
      { value: "auto", label: "自动（仅危险操作询问）", detail: "安全操作直接放行，风险较高的调用才弹确认。" },
      { value: "manual", label: "每次都询问", detail: "所有需要授权的工具调用都会弹确认。" },
      { value: "acceptEdits", label: "自动接受文件编辑", detail: "文件编辑免确认，命令执行仍询问。" },
      { value: "plan", label: "只读规划", detail: "只探索并输出计划，不执行修改。" },
      { value: "dontAsk", label: "不询问（越权直接拒绝）", detail: "需要授权的操作直接拒绝，不弹窗。" },
      { value: "bypassPermissions", label: "完全放行", detail: "跳过全部权限检查，可读写整台机器。", danger: true },
    ],
  },
  codex: {
    key: "codex_sandbox_mode",
    label: "沙箱",
    options: [
      { value: "read-only", label: "只读", detail: "只能读取文件，写入一律被沙箱拒绝。" },
      { value: "workspace-write", label: "工作区可写", detail: "可写 Codex 工作目录，工作区外拒绝。" },
      { value: "danger-full-access", label: "完全访问（不再询问）", detail: "关闭沙箱，可读写整台机器。", danger: true },
    ],
  },
  dsh: {
    key: "dsh_permission_mode",
    label: "权限",
    options: [
      { value: "read-only", label: "只读（越界询问）", detail: "只能读取，需要写入时升权确认。" },
      { value: "workspace-write", label: "工作区可写（越界询问）", detail: "可写工作目录，越界弹确认。" },
      { value: "workspace-write-auto", label: "工作区可写 + 自动批准", detail: "保留沙箱，升权请求直接放行，不再弹窗。" },
      { value: "danger-full-access", label: "完全访问（不再询问）", detail: "关闭沙箱与审批，可读写整台机器。", danger: true },
    ],
  },
};

const spec = computed(() => SPECS[props.backend as CliBackend] ?? null);
const session = computed(() => sessions.find(props.sessionId));
const value = computed(() => (session.value?.permission || "").trim());
const current = computed<PermissionOption | undefined>(() =>
  spec.value?.options.find((option) => option.value === value.value),
);
const globalLabel = computed(() => {
  if (!spec.value) return "";
  const option = spec.value.options.find((item) => item.value === globalValue.value.trim());
  if (option) return option.label;
  return globalValue.value.trim() ? globalValue.value.trim() : "跟随本机配置";
});
const triggerLabel = computed(() => current.value?.label || "跟随设置");
const canRender = computed(() => !!spec.value && !!props.sessionId && !!session.value);
const effectiveHint = computed(() => {
  if (!spec.value) return "";
  return current.value
    ? `本会话固定为「${current.value.label}」`
    : `本会话跟随设置页默认：${globalLabel.value}`;
});

async function loadGlobal() {
  if (!spec.value) return;
  try {
    const raw = await invoke<string>("get_setting_value", { key: spec.value.key });
    globalValue.value = raw || "";
  } catch {
    globalValue.value = "";
  }
}

function toggle() {
  if (props.embedded || props.disabled || saving.value || !canRender.value) return;
  open.value = !open.value;
  if (open.value) void loadGlobal();
}

async function choose(option: PermissionOption) {
  if (props.disabled || saving.value || !canRender.value) return;
  if (option.value === value.value) {
    if (!props.embedded) open.value = false;
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    await sessions.setPermission(props.sessionId, option.value);
    if (!props.embedded) open.value = false;
  } catch (cause) {
    error.value = "保存失败：" + cause;
  } finally {
    saving.value = false;
  }
}

async function resetToDefault() {
  if (props.disabled || saving.value || !canRender.value) return;
  if (!value.value) {
    if (!props.embedded) open.value = false;
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    await sessions.setPermission(props.sessionId, "");
    if (!props.embedded) open.value = false;
  } catch (cause) {
    error.value = "保存失败：" + cause;
  } finally {
    saving.value = false;
  }
}

function onDocumentPointerDown(event: PointerEvent) {
  if (props.embedded || saving.value || !open.value) return;
  const target = event.target as Node | null;
  if (rootRef.value && target && !rootRef.value.contains(target)) open.value = false;
}
function onKeydown(event: KeyboardEvent) {
  if (props.embedded || saving.value) return;
  if (event.key === "Escape" && open.value) {
    event.stopPropagation();
    open.value = false;
  }
}
watch(
  () => [props.sessionId, props.backend],
  () => { open.value = false; error.value = ""; void loadGlobal(); },
  { immediate: true },
);
onMounted(() => {
  watch(() => props.embedded, (embedded, _, onCleanup) => {
    if (embedded) return;
    document.addEventListener("pointerdown", onDocumentPointerDown, true);
    window.addEventListener("keydown", onKeydown);
    onCleanup(() => {
      document.removeEventListener("pointerdown", onDocumentPointerDown, true);
      window.removeEventListener("keydown", onKeydown);
    });
  }, { immediate: true });
});
</script>

<template>
  <div v-if="canRender" ref="rootRef" class="session-access" :class="[`placement-${placement}`, { 'is-embedded': embedded }]">
    <button
      v-if="!embedded"
      type="button"
      class="session-access-trigger"
      :class="{ open, danger: !!current?.danger, saving, compact }"
      :disabled="disabled || saving"
      :aria-expanded="open"
      aria-haspopup="listbox"
      :title="`本会话权限（${effectiveHint}）`"
      @click="toggle"
    >
      <Shield :size="12" class="session-access-icon" />
      <span v-if="!compact" class="session-access-label">{{ spec?.label }}</span>
      <span class="session-access-value">{{ saving ? "保存中…" : triggerLabel }}</span>
      <component :is="placement === 'up' ? ChevronUp : ChevronDown" :size="11" class="session-access-caret" />
    </button>

    <div v-if="embedded || open" class="session-access-pop" role="listbox" :aria-label="`本会话${spec?.label}`" :aria-busy="saving">
      <div class="session-access-head">
        <span>本会话{{ spec?.label }}</span>
        <small>只影响这条会话的下一轮 CLI 回合</small>
      </div>
      <button
        v-for="option in (spec?.options || [])"
        :key="option.value"
        type="button"
        :class="['session-access-row', { active: option.value === value, danger: option.danger }]"
        role="option"
        :aria-selected="option.value === value"
        :disabled="disabled || saving"
        @click="choose(option)"
      >
        <span class="session-access-row-main">
          <span class="session-access-row-label">{{ option.label }}</span>
          <span class="session-access-row-detail">{{ option.detail }}</span>
        </span>
        <Check v-if="option.value === value" :size="13" class="session-access-check" />
      </button>
      <button
        type="button"
        :class="['session-access-row', 'session-access-row-reset', { active: value === '' }]"
        role="option"
        :aria-selected="value === ''"
        :disabled="disabled || saving"
        @click="resetToDefault"
      >
        <span class="session-access-row-main">
          <span class="session-access-row-label">跟随设置页默认</span>
          <span class="session-access-row-detail">设置页当前：{{ globalLabel }}</span>
        </span>
        <Check v-if="value === ''" :size="13" class="session-access-check" />
      </button>
      <p v-if="saving" class="session-access-saving" role="status">保存中…</p>
      <p v-if="error" class="session-access-error" role="alert">{{ error }}</p>
    </div>
  </div>
</template>

<style scoped>
.session-access { position: relative; display: inline-flex; }
.session-access-trigger {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 240px;
  padding: 4px 8px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font: inherit;
  font-size: 11px;
  line-height: 1.4;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s;
}
.session-access-trigger:hover:not(:disabled),
.session-access-trigger.open {
  background: rgba(255, 255, 255, 0.85);
  border-color: rgba(120, 110, 100, 0.18);
}
.session-access-trigger:disabled { opacity: 0.5; cursor: default; }
.session-access-trigger.danger { color: #b45309; }
.session-access-trigger.compact { max-width: 170px; padding: 3px 6px; font-size: 10.5px; }
.session-access-icon { color: #968b7f; flex-shrink: 0; }
.session-access-trigger.danger .session-access-icon { color: #b45309; }
.session-access-label { color: #968b7f; flex-shrink: 0; }
.session-access-value {
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.session-access-caret { color: #b6aca1; flex-shrink: 0; transition: transform 0.15s; }
.session-access-trigger.open .session-access-caret { transform: rotate(180deg); }
.session-access-pop {
  position: absolute;
  right: 0;
  z-index: 70;
  width: 280px;
  max-width: 82vw;
  padding: 8px;
  border: 1px solid rgba(120, 110, 100, 0.16);
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 12px 34px rgba(60, 50, 40, 0.18);
}
.placement-up .session-access-pop { bottom: calc(100% + 8px); }
.placement-down .session-access-pop { top: calc(100% + 8px); }
.session-access-head {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px 8px 8px;
  border-bottom: 1px solid rgba(120, 110, 100, 0.1);
  margin-bottom: 4px;
}
.session-access-head span { font-size: 12px; font-weight: 600; color: #3d3630; }
.session-access-head small { font-size: 10px; color: #968b7f; }
.session-access-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  width: 100%;
  padding: 7px 9px;
  border: none;
  border-radius: 8px;
  background: none;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.session-access-row:hover:not(:disabled) { background: rgba(120, 110, 100, 0.07); }
.session-access-row:disabled { opacity: 0.6; cursor: default; }
.session-access-row.active { background: rgba(255, 107, 107, 0.08); }
.session-access-row.danger .session-access-row-label { color: #b45309; }
.session-access-row-reset { margin-top: 4px; border-top: 1px dashed rgba(120, 110, 100, 0.18); border-radius: 0 0 8px 8px; }
.session-access-row-main { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.session-access-row-label { font-size: 12px; font-weight: 600; color: #3d3630; }
.session-access-row-detail { font-size: 10.5px; color: #968b7f; line-height: 1.45; }
.session-access-check { color: var(--pet-primary, #ff6b6b); flex-shrink: 0; margin-top: 2px; }
.session-access-error { margin: 6px 8px 2px; font-size: 11px; color: #b91c1c; }
.session-access-saving { margin: 6px 8px 2px; font-size: 11px; color: #968b7f; }
.session-access.is-embedded {
  position: static;
  display: block;
  width: 100%;
  min-width: 0;
  max-width: 100%;
}
.session-access.is-embedded .session-access-pop {
  position: static;
  inset: auto;
  z-index: auto;
  box-sizing: border-box;
  width: 100%;
  min-width: 0;
  max-width: 100%;
  max-height: none;
  overflow: visible;
  overflow-wrap: anywhere;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
}
.session-access.is-embedded .session-access-row { box-sizing: border-box; min-width: 0; }
</style>
