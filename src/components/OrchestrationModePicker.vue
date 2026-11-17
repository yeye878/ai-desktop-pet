<script setup lang="ts">
import { useOrchestrationStore } from "../stores/orchestration";
import type { OrchestrationMode } from "../services/orchestration";

const orchestration = useOrchestrationStore();
const options: { value: OrchestrationMode; label: string; title: string }[] = [
  { value: "direct", label: "普通对话", title: "单智能体直接回答" },
  { value: "collaborative", label: "自由协同", title: "现有 @ 角色自由协同" },
  { value: "adaptive", label: "智能调度", title: "独立评估 → 分级执行 → 独立验收与返工" },
];
async function pick(mode: OrchestrationMode) {
  await orchestration.setMode(mode);
}
</script>

<template>
  <div class="orch-mode-picker" role="radiogroup" aria-label="任务组织方式">
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      role="radio"
      :aria-checked="orchestration.mode === option.value"
      :class="['orch-mode-option', { active: orchestration.mode === option.value }]"
      :title="option.title"
      @click="pick(option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.orch-mode-picker { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid #d5d8de; border-radius: 6px; background: #f4f5f7; }
.orch-mode-option { border: 0; background: transparent; border-radius: 4px; padding: 3px 10px; font-size: 11px; color: #5f6368; cursor: pointer; }
.orch-mode-option.active { background: #fff; color: #267a9c; font-weight: 600; box-shadow: 0 1px 2px rgba(0, 0, 0, .08); }
</style>