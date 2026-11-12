<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ChevronUp, Check } from "@lucide/vue";

/**
 * 底部信息条用的小弹层控件：触发 pill 显示「标签 · 当前值」，
 * 向上弹出。默认渲染带描述的选项列表；也可用 #panel 插槽放自定义内容（如滑块）。
 */
export interface MetaPickerOption {
  value: string;
  label: string;
  desc?: string;
}

const props = withDefaults(defineProps<{
  label: string;
  options?: MetaPickerOption[];
  modelValue?: string;
  /** 选项列表之外的展示值（如滑块场景）。 */
  currentLabel?: string;
  disabled?: boolean;
}>(), { options: () => [], modelValue: "", currentLabel: "", disabled: false });
const emit = defineEmits<{ change: [value: string] }>();

const rootRef = ref<HTMLDivElement | null>(null);
const open = ref(false);
const displayValue = computed(() =>
  props.currentLabel || props.options.find((option) => option.value === props.modelValue)?.label || "自动",
);

function toggle() {
  if (!props.disabled) open.value = !open.value;
}
function choose(value: string) {
  open.value = false;
  if (value !== props.modelValue) emit("change", value);
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
  <div ref="rootRef" class="meta-picker">
    <button
      type="button"
      class="meta-picker-trigger"
      :class="{ open }"
      :disabled="disabled"
      :aria-expanded="open"
      aria-haspopup="dialog"
      @click="toggle"
    >
      <span class="meta-picker-label">{{ label }}</span>
      <span class="meta-picker-value">{{ displayValue }}</span>
      <ChevronUp :size="11" class="meta-picker-caret" />
    </button>
    <div v-if="open" class="meta-picker-pop" role="dialog" :aria-label="label">
      <slot name="panel">
        <button
          v-for="option in options"
          :key="option.value"
          type="button"
          :class="['meta-picker-row', { active: option.value === modelValue }]"
          @click="choose(option.value)"
        >
          <span class="meta-picker-row-main">
            <span class="meta-picker-row-label">{{ option.label }}</span>
            <span v-if="option.desc" class="meta-picker-row-desc">{{ option.desc }}</span>
          </span>
          <Check v-if="option.value === modelValue" :size="13" class="meta-picker-check" />
        </button>
      </slot>
    </div>
  </div>
</template>

<style scoped>
.meta-picker { position: relative; }
.meta-picker-trigger { display: inline-flex; align-items: center; gap: 5px; padding: 4px 8px; border: 1px solid transparent; border-radius: 8px; background: transparent; cursor: pointer; font-size: 11px; transition: background .12s, border-color .12s; }
.meta-picker-trigger:hover:not(:disabled), .meta-picker-trigger.open { background: rgba(255,255,255,.85); border-color: rgba(120,110,100,.16); }
.meta-picker-trigger:disabled { opacity: .5; cursor: default; }
.meta-picker-label { color: #968b7f; }
.meta-picker-value { color: #3d3630; font-weight: 600; }
.meta-picker-caret { color: #b6aca1; transition: transform .15s; }
.meta-picker-trigger.open .meta-picker-caret { transform: rotate(180deg); }
.meta-picker-pop { position: absolute; bottom: calc(100% + 8px); right: 0; z-index: 60; min-width: 230px; padding: 8px; background: #fff; border: 1px solid rgba(120,110,100,.14); border-radius: 12px; box-shadow: 0 10px 32px rgba(60,50,40,.16); }
.meta-picker-row { display: flex; align-items: center; gap: 8px; width: 100%; padding: 8px 9px; border: none; border-radius: 8px; background: none; cursor: pointer; text-align: left; }
.meta-picker-row:hover { background: rgba(120,110,100,.07); }
.meta-picker-row.active { background: rgba(var(--pet-primary-rgb,255,107,107),.08); }
.meta-picker-row-main { flex: 1; display: flex; flex-direction: column; gap: 2px; }
.meta-picker-row-label { font-size: 12px; font-weight: 600; color: #3d3630; }
.meta-picker-row-desc { font-size: 11px; color: #968b7f; }
.meta-picker-check { color: var(--pet-primary, #ff6b6b); flex-shrink: 0; }
</style>
