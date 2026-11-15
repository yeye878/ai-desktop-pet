<script setup lang="ts">
import { computed } from "vue";
import PetCanvas from "./PetCanvas.vue";
import { usePetStore } from "../stores/pet";

defineProps<{
  /** 状态标签文案，由调用方从 petStateLabel 传入 */
  stateLabel: string;
  /** 心情百分比 0-100 */
  happinessPercent: number;
  /** 能量百分比 0-100 */
  energy: number;
  /** 当前后端/模型一行摘要 */
  modelLabel: string;
}>();

const emit = defineEmits<{ petted: [] }>();

const pet = usePetStore();

/** 舞台按 2 倍渲染：画布内部仍是 120x140，坐标体系不变 */
const STAGE_SCALE = 2;

// 每个状态一套语汇，用于灯带与状态气泡
const STATE_VIBES: Record<string, { hue: string; glow: string; bubble: string }> = {
  idle: { hue: "#66c8ff", glow: "rgba(102, 200, 255, 0.35)", bubble: "在发呆～" },
  listening: { hue: "#8ee6a8", glow: "rgba(142, 230, 168, 0.35)", bubble: "我在听" },
  thinking: { hue: "#a7a2ff", glow: "rgba(167, 162, 255, 0.35)", bubble: "让我想想…" },
  speaking: { hue: "#ff93c7", glow: "rgba(255, 147, 199, 0.35)", bubble: "说给你听" },
  working: { hue: "#66c8ff", glow: "rgba(102, 200, 255, 0.4)", bubble: "干活中" },
  sleeping: { hue: "#8792a3", glow: "rgba(135, 146, 163, 0.3)", bubble: "Zzz…" },
  happy: { hue: "#ffd85c", glow: "rgba(255, 216, 92, 0.45)", bubble: "好开心！" },
  confused: { hue: "#ff7a66", glow: "rgba(255, 122, 102, 0.35)", bubble: "咦？" },
  waving: { hue: "#8ee6a8", glow: "rgba(142, 230, 168, 0.45)", bubble: "嗨～" },
  hungry: { hue: "#ff7a66", glow: "rgba(255, 122, 102, 0.4)", bubble: "有点饿" },
  stuffed: { hue: "#ffd85c", glow: "rgba(255, 216, 92, 0.4)", bubble: "吃撑了…" },
  refusing: { hue: "#ff7a66", glow: "rgba(255, 122, 102, 0.5)", bubble: "不要！" },
  dragging: { hue: "#a7a2ff", glow: "rgba(167, 162, 255, 0.4)", bubble: "放我下来" },
};

const vibe = computed(() => STATE_VIBES[pet.state] ?? STATE_VIBES.idle);
const isAsleep = computed(() => pet.state === "sleeping");

// 心情环/能量环用 SVG 描边进度，避免再堆一层 div 进度条
const RING_R = 26;
const RING_C = 2 * Math.PI * RING_R;
const ringOffset = (percent: number) => RING_C * (1 - Math.max(0, Math.min(100, percent)) / 100);
</script>

<template>
  <div class="pet-stage" :style="{ '--stage-hue': vibe.hue, '--stage-glow': vibe.glow }">
    <!-- 背景光晕随状态变色 -->
    <div class="stage-aura" :class="{ asleep: isAsleep }" />

    <!-- 状态气泡 -->
    <div class="stage-bubble">
      <span class="stage-bubble-text">{{ vibe.bubble }}</span>
    </div>

    <!-- 桌宠本体：live + scale 让预览也参与状态同步与指针反馈 -->
    <div class="stage-pet">
      <PetCanvas preview live :scale="STAGE_SCALE" @petted="emit('petted')" />
    </div>

    <!-- 底部状态灯带 -->
    <div class="stage-strip" />

    <!-- 指标 -->
    <div class="stage-meters">
      <div class="stage-meter">
        <svg class="stage-ring" viewBox="0 0 64 64" aria-hidden="true">
          <circle class="stage-ring-track" cx="32" cy="32" :r="RING_R" />
          <circle
            class="stage-ring-fill mood"
            cx="32" cy="32" :r="RING_R"
            :stroke-dasharray="RING_C"
            :stroke-dashoffset="ringOffset(happinessPercent)"
          />
        </svg>
        <div class="stage-meter-copy">
          <span class="stage-meter-value">{{ happinessPercent }}%</span>
          <span class="stage-meter-label">心情</span>
        </div>
      </div>

      <div class="stage-meter">
        <svg class="stage-ring" viewBox="0 0 64 64" aria-hidden="true">
          <circle class="stage-ring-track" cx="32" cy="32" :r="RING_R" />
          <circle
            class="stage-ring-fill energy"
            cx="32" cy="32" :r="RING_R"
            :stroke-dasharray="RING_C"
            :stroke-dashoffset="ringOffset(energy)"
          />
        </svg>
        <div class="stage-meter-copy">
          <span class="stage-meter-value">{{ energy }}%</span>
          <span class="stage-meter-label">能量</span>
        </div>
      </div>
    </div>

    <!-- 状态与后端摘要 -->
    <div class="stage-caption">
      <span class="stage-state">{{ stateLabel }}</span>
      <span class="stage-model">{{ modelLabel }}</span>
    </div>
  </div>
</template>

<style scoped>
.pet-stage {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  padding: 22px 20px 20px;
  border: 1px solid var(--dash-card-border);
  border-radius: var(--dash-radius-xl);
  background:
    radial-gradient(circle at 50% 34%, var(--stage-glow), transparent 62%),
    linear-gradient(180deg, rgba(255, 255, 255, 0.86), rgba(255, 255, 255, 0.52));
  box-shadow: var(--dash-shadow-sm);
  overflow: hidden;
  transition: background 600ms ease;
}

.stage-aura {
  position: absolute;
  top: 8%;
  left: 50%;
  width: 260px;
  height: 260px;
  transform: translateX(-50%);
  border-radius: 50%;
  background: radial-gradient(circle, var(--stage-glow), transparent 68%);
  filter: blur(6px);
  pointer-events: none;
  animation: stage-breathe 5s ease-in-out infinite;
}

.stage-aura.asleep {
  animation-duration: 9s;
  opacity: 0.6;
}

.stage-bubble {
  position: relative;
  z-index: 2;
  max-width: 100%;
  padding: 7px 14px;
  border: 1px solid var(--dash-card-border);
  border-radius: 14px;
  background: var(--dash-panel-solid);
  box-shadow: 0 1px 0 rgba(255, 255, 255, 0.8), 0 8px 18px rgba(33, 48, 74, 0.06);
  transition: transform 260ms cubic-bezier(0.22, 0.68, 0, 1);
}

.stage-bubble::after {
  content: "";
  position: absolute;
  left: 50%;
  bottom: -5px;
  width: 9px;
  height: 9px;
  transform: translateX(-50%) rotate(45deg);
  border-right: 1px solid var(--dash-card-border);
  border-bottom: 1px solid var(--dash-card-border);
  background: var(--dash-panel-solid);
}

.stage-bubble-text {
  color: var(--dash-text-primary);
  font-size: 13px;
  font-weight: 700;
}

.stage-pet {
  position: relative;
  z-index: 1;
  /* 画布本身 pointer-events: auto，容器保持穿透，避免遮住舞台 */
  pointer-events: none;
  animation: stage-float 5.5s ease-in-out infinite;
}

.stage-strip {
  width: 100%;
  height: 4px;
  border-radius: 999px;
  background: linear-gradient(90deg, transparent, var(--stage-hue), transparent);
  opacity: 0.75;
  transition: background 500ms ease;
}

.stage-meters {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  width: 100%;
}

.stage-meter {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border: 1px solid var(--dash-card-border);
  border-radius: var(--dash-radius-lg);
  background: var(--dash-panel-bg);
}

.stage-ring {
  width: 38px;
  height: 38px;
  flex-shrink: 0;
  transform: rotate(-90deg);
}

.stage-ring-track {
  fill: none;
  stroke: rgba(33, 48, 74, 0.1);
  stroke-width: 7;
}

.stage-ring-fill {
  fill: none;
  stroke-width: 7;
  stroke-linecap: round;
  transition: stroke-dashoffset 600ms cubic-bezier(0.22, 0.68, 0, 1);
}

.stage-ring-fill.mood {
  stroke: var(--dash-candy-coral, #ff7a66);
}

.stage-ring-fill.energy {
  stroke: var(--dash-candy-sky, #66c8ff);
}

.stage-meter-copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.stage-meter-value {
  color: var(--dash-text-primary);
  font-size: 15px;
  font-weight: 800;
  font-variant-numeric: tabular-nums;
}

.stage-meter-label {
  color: var(--dash-text-muted);
  font-size: 11px;
  font-weight: 600;
}

.stage-caption {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: 100%;
  min-width: 0;
}

.stage-state {
  flex-shrink: 0;
  padding: 3px 10px;
  border-radius: 999px;
  background: var(--stage-glow);
  color: var(--dash-text-primary);
  font-size: 12px;
  font-weight: 700;
}

.stage-model {
  min-width: 0;
  overflow: hidden;
  color: var(--dash-text-muted);
  font-size: 11px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@keyframes stage-breathe {
  0%, 100% { transform: translateX(-50%) scale(1); opacity: 0.9; }
  50% { transform: translateX(-50%) scale(1.08); opacity: 1; }
}

@keyframes stage-float {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-6px); }
}
</style>
