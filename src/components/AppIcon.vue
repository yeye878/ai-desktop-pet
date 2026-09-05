<script setup lang="ts">
import { computed } from "vue";
import { ICON_PATHS } from "./iconPaths";

const props = withDefaults(
  defineProps<{ name: string; size?: number | string; strokeWidth?: number }>(),
  { size: 16, strokeWidth: 1.7 }
);

const shapes = computed(() => ICON_PATHS[props.name] ?? []);
</script>

<template>
  <svg
    class="app-icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <path
      v-for="(shape, i) in shapes"
      :key="i"
      :d="shape.d"
      :fill="shape.f ? 'currentColor' : 'none'"
      :stroke="shape.f ? 'none' : 'currentColor'"
    />
  </svg>
</template>

<style scoped>
.app-icon {
  display: inline-block;
  flex-shrink: 0;
  vertical-align: middle;
}
</style>
