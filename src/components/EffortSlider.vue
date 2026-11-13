<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { effortLabel, type AgentEffortOption } from "../services/agentModels";

const props = withDefaults(defineProps<{
  options: AgentEffortOption[];
  modelValue: string;
  disabled?: boolean;
}>(), { disabled: false });
const emit = defineEmits<{ change: [effortId: string] }>();

const stops = computed(() => [
  { id: "", label: "自动", description: "跟随本机默认强度" },
  ...props.options.map((option) => ({
    id: option.id,
    label: effortLabel(option.id, option.name),
    description: option.description || "",
  })),
]);
const selectedIndex = computed(() => Math.max(0,
  stops.value.findIndex((stop) => stop.id === (props.modelValue || "")),
));
const unavailable = computed(() => props.disabled || stops.value.length < 2);
const activePointerId = ref<number | null>(null);
const isDragging = computed(() => activePointerId.value !== null);
const dragRatio = ref<number | null>(null);
const shownIndex = computed(() => dragRatio.value === null ? selectedIndex.value : indexFromRatio(dragRatio.value));
const shownStop = computed(() => stops.value[shownIndex.value] ?? stops.value[0]);
const targetRatio = computed(() => dragRatio.value ?? ratioAtIndex(selectedIndex.value));
const trackInset = 24;
const trackRef = ref<HTMLDivElement | null>(null);
const thumbRef = ref<HTMLDivElement | null>(null);
const trackWidth = ref(1);
const reducedMotion = ref(false);
const position = ref(targetRatio.value);
const trailPosition = ref(targetRatio.value);
type FluidParticle = {
  id: number;
  origin: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  age: number;
  life: number;
  size: number;
  phase: number;
};
const particles = shallowRef<FluidParticle[]>([]);
let particleId = 0;
let particleDistance = 0;
let positionVelocity = 0;
let trailVelocity = 0;
let lastPointerTime = 0;
let pointerOffset = 0;
let lastFrameTime = 0;
let frameId: number | null = null;
let mounted = false;
let motionQuery: MediaQueryList | null = null;

const lag = computed(() => reducedMotion.value ? 0 : clamp((position.value - trailPosition.value) * trackWidth.value, -34, 34));
const strain = computed(() => lag.value / 34);
const thumbStyle = computed(() => ({
  left: trackPosition(position.value),
  transform: `translate(-50%, -50%) scale(${1 + Math.abs(strain.value) * 0.1}, ${1 - Math.abs(strain.value) * 0.06}) skewX(${-strain.value * 1.5}deg)`,
}));
const trailStyle = computed(() => ({
  left: trackPosition(position.value, -lag.value * 0.7),
  width: `${30 + Math.abs(lag.value) * 1.8}px`,
  opacity: Math.min(0.85, Math.abs(lag.value) / 22),
  transform: `translate(-50%, -50%) scale(${lag.value < 0 ? -1 : 1}, ${1 - Math.abs(strain.value) * 0.15})`,
}));
const tint = computed(() => {
  const palette = [
    { at: 0, rgb: [128, 185, 197] },
    { at: 0.36, rgb: [130, 159, 206] },
    { at: 0.68, rgb: [163, 151, 201] },
    { at: 1, rgb: [215, 158, 155] },
  ];
  const right = palette.findIndex((color) => color.at >= position.value);
  const end = palette[Math.max(1, right)];
  const start = palette[Math.max(1, right) - 1];
  const mix = clamp((position.value - start.at) / (end.at - start.at));
  return start.rgb.map((channel, index) => Math.round(channel + (end.rgb[index] - channel) * mix)).join(" ");
});

function clamp(value: number, min = 0, max = 1): number {
  return Math.min(max, Math.max(min, value));
}
function ratioAtIndex(index: number): number {
  return stops.value.length > 1 ? index / (stops.value.length - 1) : 0;
}
function indexFromRatio(ratio: number): number {
  return Math.round(clamp(ratio) * (stops.value.length - 1));
}
function trackPosition(ratio: number, offset = 0): string {
  return `calc(${ratio * 100}% + ${trackInset * (1 - 2 * ratio) + offset}px)`;
}
function ratioFromClientX(clientX: number): number {
  const rect = trackRef.value?.getBoundingClientRect();
  if (!rect || rect.width <= trackInset * 2) return targetRatio.value;
  trackWidth.value = rect.width - trackInset * 2;
  return stops.value.length > 1 ? clamp((clientX - rect.left - trackInset) / trackWidth.value) : 0;
}
function emitParticles(from: number, to: number, velocity: number) {
  if (reducedMotion.value) return;
  const distance = (to - from) * trackWidth.value;
  particleDistance += Math.abs(distance);
  const count = Math.min(8, Math.floor(particleDistance / 5));
  particleDistance %= 5;
  if (!count) return;
  const direction = Math.sign(distance);
  const speed = clamp(Math.abs(velocity) * trackWidth.value / 700);
  const emitted = Array.from({ length: count }, (_, index): FluidParticle => {
    const id = ++particleId;
    const phase = id * 2.39996;
    return {
      id,
      origin: from + (to - from) * (index + 1) / count,
      x: -direction * (19 + speed * 4),
      y: Math.sin(phase) * 10,
      vx: direction * (24 + speed * 90),
      vy: Math.cos(phase) * 14,
      age: 0,
      life: 0.36 + (id % 5) * 0.055,
      size: 1.8 + (id % 4) * 0.55 + speed * 0.9,
      phase,
    };
  });
  particles.value = [...particles.value, ...emitted].slice(-32);
}
function advanceParticles(dt: number) {
  particles.value = particles.value.filter((particle) => particle.age + dt < particle.life).map((particle) => ({
    ...particle,
    age: particle.age + dt,
    x: particle.x + particle.vx * dt,
    y: clamp(particle.y + (particle.vy + Math.sin(particle.age * 12 + particle.phase) * 18) * dt, -14, 14),
    vx: particle.vx * Math.exp(-3.5 * dt),
  }));
}
function particleStyle(particle: FluidParticle) {
  const remaining = 1 - particle.age / particle.life;
  return {
    left: trackPosition(particle.origin, particle.x),
    top: `calc(50% + ${particle.y}px)`,
    width: `${particle.size}px`,
    height: `${particle.size}px`,
    opacity: Math.min(1, particle.age / 0.04) * remaining * 0.85,
    transform: `translate(-50%, -50%) scale(${(1 + Math.abs(particle.vx) / 100) * remaining}, ${0.6 + remaining * 0.4})`,
  };
}
function stopMotion() {
  if (frameId !== null) window.cancelAnimationFrame(frameId);
  frameId = null;
  lastFrameTime = 0;
}
function resetMotion() {
  stopMotion();
  particles.value = [];
  particleDistance = 0;
  positionVelocity = 0;
  trailVelocity = 0;
  position.value = targetRatio.value;
  trailPosition.value = targetRatio.value;
}
function startMotion() {
  if (!mounted) return;
  if (reducedMotion.value || props.disabled || stops.value.length < 2) {
    resetMotion();
    return;
  }
  if (frameId === null) {
    trackWidth.value = Math.max(1, (trackRef.value?.getBoundingClientRect().width || 0) - trackInset * 2);
    lastFrameTime = performance.now();
    frameId = window.requestAnimationFrame(animate);
  }
}
function animate(time: number) {
  frameId = null;
  const elapsed = clamp((time - lastFrameTime) / 1000, 0, 0.032);
  lastFrameTime = time;
  const steps = Math.max(1, Math.ceil(elapsed / (1 / 120)));
  const dt = elapsed / steps;
  let nextPosition = position.value;
  let nextTrail = trailPosition.value;
  // 主体松手吸附，尾迹独立追随；短步积分避免低帧率下弹簧发散。
  for (let step = 0; step < steps; step += 1) {
    if (isDragging.value) {
      nextPosition = targetRatio.value;
      positionVelocity *= Math.exp(-16 * dt);
    } else {
      positionVelocity += ((targetRatio.value - nextPosition) * 360 - positionVelocity * 25) * dt;
      const proposed = nextPosition + positionVelocity * dt;
      nextPosition = clamp(proposed);
      if (proposed !== nextPosition) positionVelocity = 0;
    }
    trailVelocity += ((nextPosition - nextTrail) * 180 - trailVelocity * 20) * dt;
    nextTrail += trailVelocity * dt;
  }
  position.value = nextPosition;
  trailPosition.value = nextTrail;
  if (particles.value.length) advanceParticles(elapsed);
  const settled = Math.abs(nextPosition - targetRatio.value) < 0.0002
    && Math.abs(nextTrail - targetRatio.value) < 0.0002
    && Math.abs(positionVelocity) < 0.002 && Math.abs(trailVelocity) < 0.002;
  if (settled && particles.value.length === 0) resetMotion();
  else frameId = window.requestAnimationFrame(animate);
}

function updatePointer(clientX: number) {
  const ratio = ratioFromClientX(clientX - pointerOffset);
  const time = performance.now();
  const elapsed = Math.max(0.008, (time - lastPointerTime) / 1000);
  const delta = ratio - (dragRatio.value ?? ratio);
  if (delta !== 0 && !reducedMotion.value) {
    positionVelocity = clamp(positionVelocity * 0.35 + delta / elapsed * 0.65, -3, 3);
    emitParticles(dragRatio.value ?? ratio, ratio, positionVelocity);
  }
  lastPointerTime = time;
  dragRatio.value = ratio;
}
function releasePointer() {
  const pointerId = activePointerId.value;
  activePointerId.value = null;
  pointerOffset = 0;
  const track = trackRef.value;
  if (pointerId !== null && track?.hasPointerCapture(pointerId)) {
    try { track.releasePointerCapture(pointerId); } catch { /* 捕获目标可能已被移除。 */ }
  }
}
function cancelDrag(immediate = false) {
  releasePointer();
  dragRatio.value = null;
  if (immediate) resetMotion();
}
function onPointerDown(event: PointerEvent) {
  if (unavailable.value || activePointerId.value !== null || event.button !== 0 || !event.isPrimary) return;
  const track = trackRef.value;
  if (!track) return;
  event.preventDefault();
  const rect = track.getBoundingClientRect();
  pointerOffset = event.target === thumbRef.value
    ? event.clientX - rect.left - trackInset - position.value * (rect.width - trackInset * 2)
    : 0;
  activePointerId.value = event.pointerId;
  dragRatio.value = ratioFromClientX(event.clientX - pointerOffset);
  lastPointerTime = performance.now();
  resetMotion();
  track.focus({ preventScroll: true });
  try { track.setPointerCapture(event.pointerId); } catch { cancelDrag(true); }
}
function onPointerMove(event: PointerEvent) {
  if (event.pointerId !== activePointerId.value) return;
  updatePointer(event.clientX);
}
function onPointerUp(event: PointerEvent) {
  if (event.pointerId !== activePointerId.value) return;
  updatePointer(event.clientX);
  const index = indexFromRatio(dragRatio.value ?? 0);
  releasePointer();
  commit(index);
  dragRatio.value = null;
  startMotion();
}
function onPointerCancel(event: PointerEvent) {
  if (event.pointerId === activePointerId.value) cancelDrag();
}
function commit(index: number) {
  if (unavailable.value) return;
  const stop = stops.value[index];
  if (stop && stop.id !== (props.modelValue || "")) emit("change", stop.id);
}
function selectStop(index: number) {
  if (unavailable.value) return;
  cancelDrag();
  commit(index);
}
function onKeydown(event: KeyboardEvent) {
  if (unavailable.value) return;
  let next: number | null = null;
  if (event.key === "ArrowLeft" || event.key === "ArrowDown") next = Math.max(0, shownIndex.value - 1);
  else if (event.key === "ArrowRight" || event.key === "ArrowUp") next = Math.min(stops.value.length - 1, shownIndex.value + 1);
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = stops.value.length - 1;
  if (next !== null) {
    event.preventDefault();
    event.stopPropagation();
    selectStop(next);
  }
}
function onMotionPreferenceChange() {
  reducedMotion.value = motionQuery?.matches ?? false;
  if (reducedMotion.value) resetMotion();
}

watch(targetRatio, () => {
  if (dragRatio.value !== null) position.value = targetRatio.value;
  startMotion();
}, { flush: "sync" });
watch(() => props.disabled, (disabled) => { if (disabled) cancelDrag(true); }, { flush: "sync" });
watch(() => props.options, () => cancelDrag(true), { deep: true, flush: "sync" });
watch(() => props.modelValue, () => { if (isDragging.value) cancelDrag(); }, { flush: "sync" });

onMounted(() => {
  mounted = true;
  motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  onMotionPreferenceChange();
  motionQuery.addEventListener("change", onMotionPreferenceChange);
});
onBeforeUnmount(() => {
  mounted = false;
  releasePointer();
  stopMotion();
  motionQuery?.removeEventListener("change", onMotionPreferenceChange);
});
</script>

<template>
  <div
    class="effort-slider"
    :class="{ disabled, dragging: isDragging }"
    :style="{ '--effort-tint': tint, '--position': trackPosition(position), '--label-intervals': Math.max(1, stops.length - 1), '--track-inset': `${trackInset}px` }"
  >
    <div class="effort-slider-head">
      <span class="effort-slider-caption">思考强度</span>
      <span class="effort-slider-value">{{ shownStop.label }}</span>
    </div>
    <div class="effort-slider-scale">
      <div
        ref="trackRef"
        class="effort-slider-track"
        role="slider"
        :tabindex="unavailable ? -1 : 0"
        aria-label="思考强度"
        aria-orientation="horizontal"
        :aria-disabled="unavailable"
        :aria-valuemin="0"
        :aria-valuemax="stops.length - 1"
        :aria-valuenow="shownIndex"
        :aria-valuetext="shownStop.label"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerCancel"
        @lostpointercapture="onPointerCancel"
        @keydown="onKeydown"
      >
        <div class="effort-slider-rail" aria-hidden="true">
          <div class="effort-slider-fill" />
          <div class="effort-slider-trail" :style="trailStyle" />
          <span v-for="particle in particles" :key="particle.id" class="effort-slider-particle" :style="particleStyle(particle)" />
        </div>
        <span
          v-for="(stop, index) in stops"
          :key="stop.id"
          class="effort-slider-dot"
          :class="{ passed: index <= shownIndex }"
          :style="{ left: trackPosition(ratioAtIndex(index)) }"
          aria-hidden="true"
        />
        <div ref="thumbRef" class="effort-slider-thumb" :style="thumbStyle" aria-hidden="true" />
      </div>
      <div class="effort-slider-labels" role="group" aria-label="选择思考强度档位">
        <button
          v-for="(stop, index) in stops"
          :key="stop.id"
          type="button"
          :class="{ active: index === shownIndex }"
          :style="{ left: trackPosition(ratioAtIndex(index)) }"
          :disabled="unavailable"
          :aria-pressed="index === shownIndex"
          :title="stop.description ? `${stop.label}：${stop.description}` : stop.label"
          @click="selectStop(index)"
        >
          {{ stop.label }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.effort-slider {
  --effort-spectrum: linear-gradient(90deg, #80b9c5 0%, #829fce 36%, #a397c9 68%, #d79e9b 100%);
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 10px;
  padding: 10px 10px 6px;
  border-radius: 12px;
  background: rgba(227, 233, 242, 0.22);
  color: #303d52;
}
.effort-slider.disabled { opacity: 0.6; }
.effort-slider-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding-inline: 2px; }
.effort-slider-caption { color: #354156; font-size: 12px; font-weight: 650; }
.effort-slider-value {
  max-width: 60%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: #536582;
  font-size: 12px;
  font-weight: 650;
}
.effort-slider-scale { min-width: 0; }
.effort-slider-track {
  position: relative;
  height: 44px;
  border-radius: 22px;
  outline: none;
  touch-action: none;
  user-select: none;
  cursor: grab;
}
.effort-slider-track:focus-visible { outline: 2px solid #647f9f; outline-offset: 3px; }
.effort-slider-track[aria-disabled="true"] { cursor: default; }
.dragging .effort-slider-track { cursor: grabbing; }
.effort-slider-rail {
  position: absolute;
  inset: 2px 0;
  overflow: hidden;
  border-radius: 999px;
  background: rgba(221, 229, 239, 0.65);
  -webkit-backdrop-filter: blur(12px) saturate(1.35);
  backdrop-filter: blur(12px) saturate(1.35);
  box-shadow: 0 1px 0 rgba(255, 255, 255, 0.92), 0 -1px 1px rgba(55, 76, 104, 0.08);
  pointer-events: none;
}
.effort-slider-rail::before {
  content: "";
  position: absolute;
  inset: 0;
  background: var(--effort-spectrum);
  opacity: 0.18;
}
.effort-slider-rail::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.5), transparent 42%, rgba(255, 255, 255, 0.08) 65%, rgba(255, 255, 255, 0.38)), linear-gradient(115deg, transparent 16%, rgba(255, 255, 255, 0.15) 24%, transparent 33%, transparent 66%, rgba(255, 255, 255, 0.17) 75%, transparent 86%);
  box-shadow: inset 0 1px 1px rgba(255, 255, 255, 0.8), inset 0 2px 5px rgba(41, 64, 94, 0.16), inset 0 -1px 2px rgba(255, 255, 255, 0.8), inset 0 0 0 1px rgba(103, 125, 155, 0.12);
}
.effort-slider-fill {
  position: absolute;
  inset: 0;
  background: var(--effort-spectrum);
  clip-path: inset(0 calc(100% - var(--position)) 0 0);
}
.effort-slider-dot {
  position: absolute;
  top: 50%;
  width: 3px;
  height: 3px;
  border-radius: 50%;
  transform: translate(-50%, -50%);
  background: rgba(69, 87, 115, 0.28);
  pointer-events: none;
}
.effort-slider-dot.passed { background: rgba(255, 255, 255, 0.7); }
.effort-slider-trail {
  position: absolute;
  top: 50%;
  height: 28px;
  border-radius: 50%;
  background: radial-gradient(ellipse at 85% 50%, rgba(255, 255, 255, 0.75), rgb(var(--effort-tint) / 0.48) 40%, transparent 72%);
  filter: blur(1px);
  pointer-events: none;
}
.effort-slider-trail::before, .effort-slider-trail::after {
  content: "";
  position: absolute;
  inset: 1px 5px 12px 0;
  border-radius: 50%;
  border-top: 1.5px solid rgba(255, 255, 255, 0.7);
  transform: rotate(-6deg);
}
.effort-slider-trail::after { inset: 12px 5px 1px 8px; border-top: 0; border-bottom: 1px solid rgba(255, 255, 255, 0.55); transform: rotate(7deg); }
.effort-slider-particle {
  position: absolute;
  border-radius: 50%;
  background: radial-gradient(circle at 30% 30%, #fff, rgba(255, 255, 255, 0.85) 35%, rgb(var(--effort-tint) / 0.5));
  box-shadow: 0 0 3px rgba(255, 255, 255, 0.5);
  pointer-events: none;
}
.effort-slider-thumb {
  position: absolute;
  top: 50%;
  box-sizing: border-box;
  width: 40px;
  height: 32px;
  border: 1px solid #fff;
  border-radius: 16px;
  background: linear-gradient(170deg, #ffffff 15%, #f5f7fa 55%, #e8edf4);
  box-shadow: 0 2px 4px rgba(44, 63, 91, 0.23), 0 0 1px rgba(44, 63, 91, 0.14), inset 0 -1px 2px rgba(105, 126, 153, 0.09);
  pointer-events: auto;
}
.effort-slider-thumb::after {
  content: "";
  position: absolute;
  top: 50%;
  left: 50%;
  width: 1.5px;
  height: 9px;
  border-radius: 1px;
  background: #a0aec1;
  box-shadow: -4px 0 0 #a0aec1, 4px 0 0 #a0aec1;
  transform: translate(-50%, -50%);
  opacity: 0.65;
  pointer-events: none;
}
.effort-slider-labels { position: relative; height: 30px; margin-top: 2px; }
.effort-slider-labels button {
  position: absolute;
  top: 0;
  box-sizing: border-box;
  width: calc((100% - var(--track-inset) * 2) / var(--label-intervals));
  max-width: 44px;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transform: translateX(-50%);
  padding: 5px 1px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: #657186;
  font: inherit;
  font-size: 11px;
  line-height: 16px;
  text-align: center;
  cursor: pointer;
}
.effort-slider-labels button:hover:not(:disabled) { color: #303d52; background: rgba(81, 104, 143, 0.06); }
.effort-slider-labels button.active { color: #2d3b53; font-weight: 750; }
.effort-slider-labels button:focus-visible { outline: 2px solid #647f9f; outline-offset: 1px; }
.effort-slider-labels button:disabled { cursor: default; }

@media (prefers-reduced-motion: reduce) {
  .effort-slider *, .effort-slider *::before, .effort-slider *::after { animation: none !important; transition: none !important; }
  .effort-slider-trail, .effort-slider-particle { display: none; }
  .effort-slider-thumb { transform: translate(-50%, -50%) !important; }
}
</style>
