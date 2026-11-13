<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Check, ChevronDown } from "@lucide/vue";
import AppIcon from "./AppIcon.vue";

/** 智能体选择器：按钮显示当前头像+名称，弹层带描述，替代原生 select。 */
export interface AgentPickerOption {
  id: string;
  name: string;
  avatar: string;
  description?: string;
}

const props = withDefaults(defineProps<{
  agents: AgentPickerOption[];
  modelValue: string;
  subtitle?: string;
  disabled?: boolean;
}>(), { disabled: false });
const emit = defineEmits<{ change: [agentId: string] }>();

const rootRef = ref<HTMLDivElement | null>(null);
const open = ref(false);
const current = computed(() => props.agents.find((agent) => agent.id === props.modelValue));

function toggle() {
  if (!props.disabled) open.value = !open.value;
}
function choose(id: string) {
  open.value = false;
  if (id !== props.modelValue) emit("change", id);
}
function onDocumentPointerDown(event: PointerEvent) {
  if (!open.value) return;
  const target = event.target as Node | null;
  if (rootRef.value && target && !rootRef.value.contains(target)) open.value = false;
}
function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && open.value) {
    event.stopPropagation();
    open.value = false;
  }
}
watch(() => props.disabled, (value) => { if (value) open.value = false; });
onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  window.addEventListener("keydown", onKeydown);
});
onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div ref="rootRef" class="agent-picker">
    <button
      type="button"
      class="agent-picker-trigger"
      :class="{ open }"
      :disabled="disabled"
      :aria-expanded="open"
      aria-haspopup="listbox"
      title="选择智能体"
      @click="toggle"
    >
      <span class="agent-picker-avatar">
        <AppIcon v-if="current?.id === 'builtin-claude'" name="claude" :size="18" />
        <AppIcon v-else-if="current?.id === 'builtin-codex'" name="codex" :size="18" />
        <AppIcon v-else-if="current?.id === 'builtin-dsh'" name="dsh" :size="18" />
        <template v-else>{{ current?.avatar || "🤖" }}</template>
      </span>
      <span class="agent-picker-copy">
        <span class="agent-picker-name">{{ current?.name || "选择智能体" }}</span>
        <span v-if="subtitle" class="agent-picker-subtitle">{{ subtitle }}</span>
      </span>
      <ChevronDown :size="12" class="agent-picker-caret" />
    </button>
    <div v-if="open" class="agent-picker-pop" role="listbox" aria-label="智能体列表">
      <button
        v-for="agent in agents"
        :key="agent.id"
        type="button"
        :class="['agent-picker-row', { active: agent.id === modelValue }]"
        role="option"
        :aria-selected="agent.id === modelValue"
        @click="choose(agent.id)"
      >
        <span class="agent-picker-row-avatar">
          <AppIcon v-if="agent.id === 'builtin-claude'" name="claude" :size="15" />
          <AppIcon v-else-if="agent.id === 'builtin-codex'" name="codex" :size="15" />
          <AppIcon v-else-if="agent.id === 'builtin-dsh'" name="dsh" :size="15" />
          <template v-else>{{ agent.avatar || "🤖" }}</template>
        </span>
        <span class="agent-picker-row-main">
          <span class="agent-picker-row-name">{{ agent.name }}</span>
          <span v-if="agent.description" class="agent-picker-row-desc">{{ agent.description }}</span>
        </span>
        <Check v-if="agent.id === modelValue" :size="14" class="agent-picker-check" />
      </button>
      <div v-if="!agents.length" class="agent-picker-empty">暂无可用智能体</div>
    </div>
  </div>
</template>

<style scoped>
.agent-picker { position: relative; min-width: 0; }
.agent-picker-trigger { display: inline-flex; align-items: center; gap: 10px; max-width: 100%; padding: 5px; border: 1px solid transparent; border-radius: 9px; background: transparent; cursor: pointer; text-align: left; font: inherit; transition: background .15s; }
.agent-picker-trigger:hover:not(:disabled), .agent-picker-trigger.open { background: rgba(33,48,74,.045); }
.agent-picker-trigger:focus-visible { outline: 2px solid var(--pet-primary, #ff6b6b); outline-offset: 2px; }
.agent-picker-trigger:disabled { opacity: .55; cursor: default; }
.agent-picker-avatar { width: 34px; height: 34px; display: inline-flex; align-items: center; justify-content: center; font-size: 19px; background: linear-gradient(145deg, rgba(255,255,255,.85), rgba(222,232,245,.55)); box-shadow: inset 0 0 0 1px rgba(33,48,74,.05); border-radius: 10px; flex-shrink: 0; }
.agent-picker-copy { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
.agent-picker-name { font-size: 14px; font-weight: 650; color: var(--dash-text-primary, #293449); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.agent-picker-subtitle { max-width: 220px; color: var(--dash-text-muted, #7c8696); font-size: 10px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.agent-picker-caret { color: #968b7f; flex-shrink: 0; transition: transform .15s; }
.agent-picker-trigger.open .agent-picker-caret { transform: rotate(180deg); }
.agent-picker-pop { position: absolute; top: calc(100% + 6px); left: 0; z-index: 60; min-width: 220px; max-width: 300px; max-height: 320px; overflow-y: auto; padding: 5px; background: #fff; border: 1px solid rgba(120,110,100,.14); border-radius: 12px; box-shadow: 0 10px 32px rgba(60,50,40,.16); }
.agent-picker-row { display: flex; align-items: center; gap: 9px; width: 100%; padding: 7px 8px; border: none; border-radius: 8px; background: none; cursor: pointer; text-align: left; }
.agent-picker-row:hover { background: rgba(120,110,100,.07); }
.agent-picker-row.active { background: rgba(var(--pet-primary-rgb,255,107,107),.08); }
.agent-picker-row-avatar { width: 28px; height: 28px; display: inline-flex; align-items: center; justify-content: center; font-size: 16px; background: rgba(120,110,100,.08); border-radius: 8px; flex-shrink: 0; }
.agent-picker-row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.agent-picker-row-name { font-size: 12.5px; font-weight: 600; color: #3d3630; }
.agent-picker-row-desc { font-size: 11px; color: #968b7f; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.agent-picker-check { color: var(--pet-primary, #ff6b6b); flex-shrink: 0; }
.agent-picker-empty { padding: 14px; text-align: center; font-size: 12px; color: #968b7f; }
</style>
