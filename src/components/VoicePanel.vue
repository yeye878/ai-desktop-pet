<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useChatStore } from "../stores/chat";
import { usePetStore } from "../stores/pet";
import {
  DEFAULT_VOICE_SETTINGS,
  VoiceController,
  isSpeechRecognitionSupported,
  parseVoiceSettings,
  type VoiceSettings,
  type VoiceStatus,
} from "../services/voice";
import { TtsPlayer, DEFAULT_TTS_SETTINGS, type TtsSettings } from "../services/tts";
import AppIcon from "./AppIcon.vue";

const emit = defineEmits<{ close: [] }>();

type AiFinishedPayload = {
  text: string;
  thinking: string | null;
};
type AiErrorPayload = {
  message: string;
  thinking: string | null;
  aborted: boolean;
};

const VOICE_SETTINGS_KEY = "voice_settings";
const TTS_SETTINGS_KEY = "tts_settings";
const currentWindow = getCurrentWindow();
const chat = useChatStore();
const pet = usePetStore();
const voice = new VoiceController();
const ttsPlayer = new TtsPlayer();

const settings = ref<VoiceSettings>({ ...DEFAULT_VOICE_SETTINGS });
const ttsSettings = ref<TtsSettings>({ ...DEFAULT_TTS_SETTINGS });
const status = ref<VoiceStatus>("idle");
const transcript = ref("");
const interimText = ref("");
const errorText = ref("");
const isSending = ref(false);
const isGenerating = ref(false);

const fluidCanvasRef = ref<HTMLCanvasElement | null>(null);
let animId = 0;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;
let unlistenStartListening: UnlistenFn | null = null;
let unlistenVoiceSettingsChanged: UnlistenFn | null = null;
let unlistenTtsSettingsChanged: UnlistenFn | null = null;
let sendTimeoutId: ReturnType<typeof setTimeout> | null = null;
let autoListenTimer: ReturnType<typeof setTimeout> | null = null;

interface Spark {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  size: number;
  color: string;
}
const sparks = ref<Spark[]>([]);

const canListen = computed(() => settings.value.enabled && isSpeechRecognitionSupported() && !chat.isLoading);
const statusLabel = computed(() => {
  if (!settings.value.enabled) return "语音功能已关闭";
  if (!isSpeechRecognitionSupported()) return "当前环境不支持语音识别";
  if (isGenerating.value) return "正在生成语音";
  if (chat.isLoading || isSending.value) return "AI 思考中";
  if (status.value === "listening") return "正在聆听";
  if (status.value === "speaking") return "正在播放";
  if (status.value === "error") return "需要重试";
  if (transcript.value) return "确认后发送";
  return "准备就绪";
});
const displayText = computed(() => interimText.value || transcript.value || errorText.value || "按下按钮后直接说话");
const copyStateLabel = computed(() => {
  if (errorText.value) return "需要处理";
  if (interimText.value) return "实时识别";
  if (transcript.value) return "待发送";
  return "语音输入";
});
const stageLabel = computed(() => {
  if (status.value === "listening") return "LIVE INPUT";
  if (status.value === "speaking" || isGenerating.value) return "VOICE OUT";
  if (chat.isLoading || isSending.value) return "THINKING";
  if (errorText.value) return "ATTENTION";
  return "READY";
});
const primaryLabel = computed(() => {
  if (status.value === "listening") return "停止";
  if (transcript.value) return "重说";
  return "开始说话";
});

async function loadSettings() {
  try {
    settings.value = parseVoiceSettings(await invoke<string>("get_setting_value", {
      key: VOICE_SETTINGS_KEY,
    }));
  } catch {
    settings.value = { ...DEFAULT_VOICE_SETTINGS };
  }
  try {
    const raw = await invoke<string>("get_setting_value", { key: TTS_SETTINGS_KEY });
    if (raw) ttsSettings.value = { ...DEFAULT_TTS_SETTINGS, ...JSON.parse(raw) };
  } catch {
    ttsSettings.value = { ...DEFAULT_TTS_SETTINGS };
  }
}

function handleSettingsChanged() {
  void loadSettings();
}

function clearAutoListenTimer() {
  if (autoListenTimer) {
    clearTimeout(autoListenTimer);
    autoListenTimer = null;
  }
}

function scheduleStartListening(delay = 0) {
  clearAutoListenTimer();
  autoListenTimer = setTimeout(() => {
    autoListenTimer = null;
    void beginListening();
  }, delay);
}

async function beginListening() {
  if (status.value === "listening") return;

  if (!canListen.value) {
    errorText.value = !settings.value.enabled
      ? "请先在设置里启用语音"
      : isSpeechRecognitionSupported()
        ? "AI 正在回复，稍后再说"
        : "当前 WebView 不支持语音识别";
    status.value = "error";
    return;
  }

  ttsPlayer.stop();
  transcript.value = "";
  interimText.value = "";
  errorText.value = "";
  status.value = "listening";
  pet.setState("listening");

  try {
    const result = await voice.listen(settings.value, (text) => {
      interimText.value = text;
    });
    transcript.value = result.trim();
    interimText.value = "";
    status.value = "idle";
    if (!transcript.value) {
      errorText.value = "没有听清楚，再说一次吧";
      status.value = "error";
      pet.setState("confused");
      return;
    }
    pet.setState("idle");
    if (settings.value.autoSend) {
      await sendTranscript();
    }
  } catch (err) {
    interimText.value = "";
    errorText.value = err instanceof Error ? err.message : String(err);
    status.value = "error";
    pet.setState("confused");
  }
}

async function startListening() {
  if (status.value === "listening") {
    clearAutoListenTimer();
    voice.stopListening();
    return;
  }
  await beginListening();
}

function clearSendTimeout() {
  if (sendTimeoutId) {
    clearTimeout(sendTimeoutId);
    sendTimeoutId = null;
  }
}

async function hideVoicePanelForTask() {
  clearAutoListenTimer();
  try {
    await currentWindow.hide();
  } catch {
    // Hiding is best-effort; failing to hide should not block the user's request.
  }
}

async function showVoicePanelForRetry() {
  try {
    await currentWindow.show();
    await currentWindow.setFocus();
  } catch {
    // If the window cannot be restored, the chat error state still records the failure.
  }
}

async function sendTranscript() {
  const message = transcript.value.trim();
  if (!message || chat.isLoading || isSending.value) return;

  if (settings.value.enabled && settings.value.autoSpeak) {
    ttsPlayer.preparePlayback();
  }
  chat.addMessage("user", message);
  errorText.value = "";
  isSending.value = true;
  chat.isLoading = true;
  pet.setState("thinking");

  // 60s 超时兜底：若 ai-finished/ai-error 事件丢失，自动恢复状态
  clearSendTimeout();
  sendTimeoutId = setTimeout(() => {
    if (isSending.value) {
      isSending.value = false;
      chat.isLoading = false;
      errorText.value = "AI 响应超时，请重试";
      status.value = "error";
      pet.setState("confused");
    }
  }, 60_000);

  try {
    await hideVoicePanelForTask();
    await invoke<{ started: boolean }>("send_to_ai", { message, attachments: [] });
    transcript.value = "";
    await currentWindow.emit("voice-message-sent", message);
  } catch (err) {
    await showVoicePanelForRetry();
    clearSendTimeout();
    chat.isLoading = false;
    isSending.value = false;
    errorText.value = `发送失败: ${err instanceof Error ? err.message : String(err)}`;
    status.value = "error";
    pet.setState("confused");
    // 保留文本，用户可直接重试发送
  }
}

function cancelVoice() {
  clearSendTimeout();
  voice.abortListening();
  ttsPlayer.stop();
  transcript.value = "";
  interimText.value = "";
  errorText.value = "";
  isSending.value = false;
  chat.isLoading = false;
  isGenerating.value = false;
  status.value = "idle";
  pet.setState("idle");
  emit("close");
}

function clearTranscript() {
  transcript.value = "";
  interimText.value = "";
  errorText.value = "";
  status.value = "idle";
}

onMounted(async () => {
  await loadSettings();
  await currentWindow.center();
  await currentWindow.setFocus();

  // 自动开始语音识别（如果条件允许）
  if (canListen.value) {
    // 短暂延时确保UI渲染完成
    scheduleStartListening(300);
  }

  // Start canvas fluid animation loop
  const canvas = fluidCanvasRef.value;
  if (canvas) {
    const ctx = canvas.getContext("2d");
    if (ctx) {
      const dpr = window.devicePixelRatio || 1;
      const W = Math.max(320, Math.round(canvas.parentElement?.clientWidth || 336));
      const H = Math.max(208, Math.round(canvas.parentElement?.clientHeight || 214));
      canvas.width = W * dpr;
      canvas.height = H * dpr;
      canvas.style.width = "100%";
      canvas.style.height = "100%";
      ctx.scale(dpr, dpr);

      const render = (timestamp: number) => {
        const time = timestamp * 0.001;
        ctx.clearRect(0, 0, W, H);

        const cx = W / 2;
        const cy = H / 2 - 4;
        const R = 46;
        const isActive = status.value === "listening";
        const isOutput = status.value === "speaking" || isGenerating.value;
        const isBusy = chat.isLoading || isSending.value;
        const amp = isActive ? 1 : isOutput ? 0.72 : isBusy ? 0.46 : 0.18;

        const accent = isActive
          ? { a: "#31d489", b: "#51c7ff", soft: "rgba(49, 212, 137, 0.18)" }
          : isOutput
            ? { a: "#7c6cf2", b: "#52c3ff", soft: "rgba(124, 108, 242, 0.17)" }
            : isBusy
              ? { a: "#ffb15c", b: "#ff6f6f", soft: "rgba(255, 177, 92, 0.16)" }
              : { a: "#7b8ca5", b: "#c5d5e8", soft: "rgba(123, 140, 165, 0.12)" };

        const bg = ctx.createLinearGradient(0, 0, W, H);
        bg.addColorStop(0, "rgba(255, 255, 255, 0.34)");
        bg.addColorStop(0.48, accent.soft);
        bg.addColorStop(1, "rgba(255, 255, 255, 0.16)");
        ctx.fillStyle = bg;
        ctx.fillRect(0, 0, W, H);

        ctx.save();
        ctx.strokeStyle = "rgba(33, 48, 74, 0.055)";
        ctx.lineWidth = 1;
        for (let x = 24; x < W; x += 24) {
          ctx.beginPath();
          ctx.moveTo(x, 18);
          ctx.lineTo(x, H - 18);
          ctx.stroke();
        }
        for (let y = 28; y < H; y += 28) {
          ctx.beginPath();
          ctx.moveTo(18, y);
          ctx.lineTo(W - 18, y);
          ctx.stroke();
        }
        ctx.restore();

        if ((isActive || isOutput) && Math.random() < 0.22) {
          const angle = Math.random() * Math.PI * 2;
          const speed = 0.6 + Math.random() * 1.2;
          sparks.value.push({
            x: cx + Math.cos(angle) * (R + 16),
            y: cy + Math.sin(angle) * (R + 16),
            vx: Math.cos(angle) * speed,
            vy: Math.sin(angle) * speed,
            life: 1.0,
            size: 1 + Math.random() * 1.2,
            color: Math.random() < 0.5 ? accent.a : accent.b,
          });
        }

        for (let i = sparks.value.length - 1; i >= 0; i--) {
          const s = sparks.value[i];
          s.x += s.vx;
          s.y += s.vy;
          s.life -= 0.020;
          if (s.life <= 0) {
            sparks.value.splice(i, 1);
            continue;
          }
          ctx.save();
          ctx.globalAlpha = s.life;
          ctx.fillStyle = s.color;
          ctx.shadowBlur = 4;
          ctx.shadowColor = s.color;
          ctx.beginPath();
          ctx.arc(s.x, s.y, s.size, 0, Math.PI * 2);
          ctx.fill();
          ctx.restore();
        }

        const drawOrb = (radius: number, colorA: string, colorB: string, phase: number) => {
          ctx.save();
          ctx.beginPath();
          const numPoints = 96;
          for (let i = 0; i < numPoints; i++) {
            const theta = (i / numPoints) * Math.PI * 2;
            const offset =
              Math.sin(theta * 3 + time * (2.2 + amp * 7) + phase) * (3 + amp * 9) +
              Math.cos(theta * 6 - time * (1.6 + amp * 4.5) + phase) * (1.4 + amp * 4);
            const r = radius + offset;
            const x = cx + Math.cos(theta) * r;
            const y = cy + Math.sin(theta) * r;

            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          }
          ctx.closePath();
          const fill = ctx.createRadialGradient(cx - 18, cy - 24, 6, cx, cy, radius + 28);
          fill.addColorStop(0, "rgba(255,255,255,0.78)");
          fill.addColorStop(0.38, colorA);
          fill.addColorStop(1, colorB);
          ctx.fillStyle = fill;
          ctx.shadowBlur = 28;
          ctx.shadowColor = colorA;
          ctx.globalAlpha = 0.76;
          ctx.fill();
          ctx.restore();
        };

        ctx.save();
        ctx.translate(cx, cy);
        for (let i = 0; i < 3; i++) {
          const radius = R + 26 + i * 22 + Math.sin(time * 1.2 + i) * (2 + amp * 4);
          ctx.beginPath();
          ctx.ellipse(0, 0, radius * 1.55, radius * 0.62, -0.08 + i * 0.18, 0, Math.PI * 2);
          ctx.strokeStyle = i === 0 ? "rgba(255,255,255,0.50)" : `rgba(33, 48, 74, ${0.10 - i * 0.02})`;
          ctx.lineWidth = 1;
          ctx.stroke();
        }
        ctx.restore();

        drawOrb(R + 10, `${accent.a}99`, `${accent.b}4d`, 0);
        drawOrb(R - 8, "rgba(255,255,255,0.72)", `${accent.a}42`, Math.PI * 0.7);

        const drawSpectrumWave = (color: string, ampMult: number, phase: number, yBase: number) => {
          ctx.strokeStyle = color;
          ctx.lineWidth = 1.35;
          ctx.beginPath();
          const wavePoints = 72;
          for (let i = 0; i <= wavePoints; i++) {
            const x = (i / wavePoints) * W;
            const y = yBase +
              Math.sin(i * 0.22 + time * (2.2 + amp * 7) + phase) * (2 + amp * 18) * ampMult +
              Math.cos(i * 0.09 - time * (1.8 + amp * 5) + phase) * (1 + amp * 6) * ampMult;
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          }
          ctx.stroke();
        };

        ctx.globalAlpha = 0.82;
        drawSpectrumWave(`${accent.a}99`, 1, 0, H - 42);
        ctx.globalAlpha = 0.54;
        drawSpectrumWave(`${accent.b}88`, 0.78, Math.PI * 0.8, H - 32);

        const bars = 34;
        for (let i = 0; i < bars; i++) {
          const x = 24 + i * ((W - 48) / (bars - 1));
          const pulse = Math.sin(time * (2.8 + amp * 8) + i * 0.62) * 0.5 + 0.5;
          const barH = 5 + pulse * (10 + amp * 34);
          const grd = ctx.createLinearGradient(x, H - 18 - barH, x, H - 18);
          grd.addColorStop(0, `${accent.a}b8`);
          grd.addColorStop(1, "rgba(255,255,255,0.22)");
          ctx.fillStyle = grd;
          ctx.fillRect(x, H - 18 - barH, 2, barH);
        }

        animId = requestAnimationFrame(render);
      };
      animId = requestAnimationFrame(render);
    }
  }

  unlistenAiFinished = await listen<AiFinishedPayload>("ai-finished", async () => {
    clearSendTimeout();
    isSending.value = false;
    chat.isLoading = false;
    pet.setState("speaking");
    status.value = "idle";
    isGenerating.value = false;
    if (pet.state === "speaking") pet.setState("idle");
  });

  unlistenAiError = await listen<AiErrorPayload>("ai-error", (event) => {
    clearSendTimeout();
    isSending.value = false;
    chat.isLoading = false;
    errorText.value = event.payload.aborted ? "已中止" : event.payload.message;
    status.value = event.payload.aborted ? "idle" : "error";
    pet.setState(event.payload.aborted ? "idle" : "confused");
  });

  unlistenStartListening = await listen("voice-start-listening", () => {
    scheduleStartListening();
  });
  window.addEventListener("voice-settings-changed", handleSettingsChanged);
  window.addEventListener("tts-settings-changed", handleSettingsChanged);
  unlistenVoiceSettingsChanged = await listen("voice-settings-changed", () => {
    handleSettingsChanged();
  });
  unlistenTtsSettingsChanged = await listen("tts-settings-changed", () => {
    handleSettingsChanged();
  });
});

onBeforeUnmount(() => {
  clearSendTimeout();
  clearAutoListenTimer();
  cancelAnimationFrame(animId);
  unlistenAiFinished?.();
  unlistenAiError?.();
  unlistenStartListening?.();
  unlistenVoiceSettingsChanged?.();
  unlistenTtsSettingsChanged?.();
  window.removeEventListener("voice-settings-changed", handleSettingsChanged);
  window.removeEventListener("tts-settings-changed", handleSettingsChanged);
  voice.abortListening();
  ttsPlayer.stop();
  isSending.value = false;
  chat.isLoading = false;
});
</script>

<template>
  <section class="voice-panel" :class="{ active: status === 'listening', speaking: status === 'speaking', generating: isGenerating }">
    <div class="panel-glow"></div>

    <div class="voice-header">
      <div class="header-left">
        <span class="badge">VOICE</span>
        <span class="status-indicator" :class="status"></span>
        <span class="status-text">{{ statusLabel }}</span>
      </div>
      <button class="close-btn" title="关闭" @click="cancelVoice">
        <AppIcon name="x" :size="13" />
      </button>
    </div>

    <div class="visual-core voice-stage">
      <!-- Dynamic fluid canvas rendering both the blob core and the bottom waves -->
      <div class="canvas-container">
        <canvas ref="fluidCanvasRef" class="fluid-canvas"></canvas>
        <div class="core-overlay-icon" :class="status">
          <AppIcon v-if="status === 'listening'" name="voice" :size="20" />
          <AppIcon v-else-if="status === 'speaking' || isGenerating" name="sparkles" :size="20" />
          <AppIcon v-else name="sun" :size="18" />
        </div>
      </div>
    </div>

    <div class="content-area voice-overlay">
      <div class="voice-copy-card" :class="{ error: !!errorText, active: !!interimText, ready: !!transcript }">
        <div class="voice-copy-head">
          <span>{{ copyStateLabel }}</span>
          <b>{{ stageLabel }}</b>
        </div>
        <p class="voice-copy-text">{{ displayText }}</p>
      </div>
    </div>

    <div class="actions">
      <button class="action-btn secondary" :disabled="status === 'listening'" @click="clearTranscript">清空</button>
      <button class="action-btn primary mic" :class="{ recording: status === 'listening' }" :disabled="chat.isLoading && status !== 'listening'" @click="startListening">
        {{ primaryLabel }}
      </button>
      <button class="action-btn primary" :disabled="!transcript.trim() || chat.isLoading || isSending" @click="sendTranscript">发送</button>
    </div>

    <div class="shortcut-hint">快捷键: {{ settings.shortcut }}</div>
  </section>
</template>

<style scoped>
.voice-panel {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  padding: 18px 18px 14px;
  overflow: hidden;
  isolation: isolate;
  color: var(--dash-text-primary, #2d2922);
  font-family: var(--dash-font-sans, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif);
  font-size: 13px;
  line-height: 1.6;
  background:
    radial-gradient(120% 55% at 50% 0%, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.06), transparent 60%),
    var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: var(--dash-radius-xl, 16px);
  box-shadow:
    0 2px 6px rgba(48, 42, 34, 0.06),
    0 20px 48px rgba(48, 42, 34, 0.16);
}

.voice-panel > * {
  position: relative;
  z-index: 1;
}

.panel-glow {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  background:
    radial-gradient(ellipse 72% 46% at 50% 0%, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.08), transparent 70%);
  opacity: 0.85;
  transition: opacity 0.3s cubic-bezier(0.22, 1, 0.36, 1);
}

.voice-panel.active .panel-glow,
.voice-panel.speaking .panel-glow,
.voice-panel.generating .panel-glow {
  opacity: 1;
  background:
    radial-gradient(ellipse 72% 46% at 50% 0%, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.14), transparent 70%);
}

/* ---------- header ---------- */
.voice-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 30px;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.badge {
  flex-shrink: 0;
  padding: 3px 9px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.16em;
  color: var(--dash-accent, #bf7a4e);
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  border: 1px solid rgba(var(--pet-primary-rgb, 191, 122, 78), 0.18);
}

.status-indicator {
  flex-shrink: 0;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--dash-text-muted, #a29a8a);
  transition: background 0.25s ease, box-shadow 0.25s ease;
}

.status-indicator.listening {
  background: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  animation: indicator-breathe 1.6s ease-in-out infinite;
}

.status-indicator.speaking {
  background: var(--dash-success, #5e8f6a);
  box-shadow: 0 0 0 3px var(--dash-success-soft, rgba(94, 143, 106, 0.12));
}

.status-indicator.error {
  background: var(--dash-danger, #c05a4d);
  box-shadow: 0 0 0 3px var(--dash-danger-soft, rgba(192, 90, 77, 0.12));
}

@keyframes indicator-breathe {
  0%, 100% { box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1)); }
  50% { box-shadow: 0 0 0 5px var(--dash-accent-softer, rgba(191, 122, 78, 0.06)); }
}

.status-text {
  min-width: 0;
  overflow: hidden;
  font-size: 12px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.close-btn {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  display: grid;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  cursor: pointer;
  transition: background 0.16s cubic-bezier(0.22, 1, 0.36, 1), color 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.close-btn:hover {
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-primary, #2d2922);
}

.close-btn:active {
  transform: scale(0.94);
}

/* ---------- sound stage ---------- */
.visual-core {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
}

.canvas-container {
  position: relative;
  flex: 1;
  width: 100%;
  min-height: 0;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: var(--dash-radius-lg, 13px);
  background: var(--dash-panel-soft, #f3f0e9);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.65),
    var(--dash-shadow-sm, 0 1px 3px rgba(48, 42, 34, 0.05));
  overflow: hidden;
}

.fluid-canvas {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
}

.core-overlay-icon {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  z-index: 5;
  width: 56px;
  height: 56px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  color: var(--dash-text-secondary, #6d6558);
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.7),
    var(--dash-shadow-md, 0 8px 20px rgba(48, 42, 34, 0.1));
  transition: color 0.25s ease, border-color 0.25s ease, box-shadow 0.25s ease;
}

.core-overlay-icon.listening {
  color: var(--dash-accent, #bf7a4e);
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.35);
  animation: mic-pulse 1.8s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}

.core-overlay-icon.speaking {
  color: var(--dash-accent, #bf7a4e);
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.24);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.7),
    0 0 0 5px var(--dash-accent-softer, rgba(191, 122, 78, 0.06)),
    var(--dash-shadow-md, 0 8px 20px rgba(48, 42, 34, 0.1));
}

@keyframes mic-pulse {
  0% {
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.7),
      var(--dash-shadow-md, 0 8px 20px rgba(48, 42, 34, 0.1)),
      0 0 0 0 var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
  }
  70% {
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.7),
      var(--dash-shadow-md, 0 8px 20px rgba(48, 42, 34, 0.1)),
      0 0 0 16px rgba(var(--pet-primary-rgb, 191, 122, 78), 0);
  }
  100% {
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.7),
      var(--dash-shadow-md, 0 8px 20px rgba(48, 42, 34, 0.1)),
      0 0 0 0 rgba(var(--pet-primary-rgb, 191, 122, 78), 0);
  }
}

/* ---------- transcript card ---------- */
.content-area {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.voice-copy-card {
  display: grid;
  gap: 6px;
  min-height: 64px;
  padding: 11px 14px;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: var(--dash-radius-lg, 13px);
  box-shadow: var(--dash-shadow-sm, 0 1px 3px rgba(48, 42, 34, 0.05));
  transition: border-color 0.2s ease, background 0.2s ease;
}

.voice-copy-card.active {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.22);
  background: linear-gradient(180deg, var(--dash-panel-solid, #fffefb), var(--dash-accent-softer, rgba(191, 122, 78, 0.06)));
}

.voice-copy-card.ready {
  border-color: rgba(var(--pet-primary-rgb, 191, 122, 78), 0.3);
}

.voice-copy-card.error {
  border-color: rgba(192, 90, 77, 0.25);
  background: linear-gradient(180deg, var(--dash-panel-solid, #fffefb), var(--dash-danger-soft, rgba(192, 90, 77, 0.08)));
}

.voice-copy-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--dash-text-muted, #a29a8a);
}

.voice-copy-head b {
  font-family: var(--dash-font-mono, ui-monospace, "SF Mono", Consolas, monospace);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.16em;
  color: var(--dash-accent, #bf7a4e);
}

.voice-copy-card.error .voice-copy-head b {
  color: var(--dash-danger, #c05a4d);
}

.voice-copy-text {
  margin: 0;
  max-height: 40px;
  overflow: hidden;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.5;
  color: var(--dash-text-primary, #2d2922);
  text-overflow: ellipsis;
}

.voice-copy-card:not(.active):not(.ready):not(.error) .voice-copy-text {
  color: var(--dash-text-muted, #a29a8a);
  font-weight: 500;
}

.voice-copy-card.error .voice-copy-text {
  color: var(--dash-danger, #c05a4d);
}

/* ---------- actions ---------- */
.actions {
  flex: 0 0 auto;
  display: flex;
  gap: 8px;
}

.action-btn {
  height: 38px;
  border: 1px solid transparent;
  border-radius: 10px;
  font-family: inherit;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition:
    background 0.18s cubic-bezier(0.22, 1, 0.36, 1),
    border-color 0.18s cubic-bezier(0.22, 1, 0.36, 1),
    color 0.18s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.18s cubic-bezier(0.22, 1, 0.36, 1),
    transform 0.18s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.18s ease;
}

.action-btn.primary {
  flex: 1;
  color: #fffaf4;
  background: var(--dash-accent, #bf7a4e);
  box-shadow: 0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28);
}

.action-btn.primary:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a8663c);
  transform: translateY(-1px);
}

.action-btn.primary.mic {
  flex: 1.3;
}

.action-btn.primary.mic.recording {
  animation: mic-btn-pulse 1.8s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}

@keyframes mic-btn-pulse {
  0% {
    box-shadow:
      0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28),
      0 0 0 0 var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
  }
  70% {
    box-shadow:
      0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28),
      0 0 0 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0);
  }
  100% {
    box-shadow:
      0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28),
      0 0 0 0 rgba(var(--pet-primary-rgb, 191, 122, 78), 0);
  }
}

.action-btn.secondary {
  width: 64px;
  color: var(--dash-text-secondary, #6d6558);
  background: var(--dash-panel-solid, #fffefb);
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.action-btn.secondary:hover:not(:disabled) {
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-primary, #2d2922);
}

.action-btn:active:not(:disabled) {
  transform: scale(0.98);
}

.action-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
  box-shadow: none;
}

.action-btn:focus-visible,
.close-btn:focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
}

.shortcut-hint {
  flex: 0 0 auto;
  text-align: center;
  font-size: 10px;
  color: var(--dash-text-muted, #a29a8a);
}

@media (prefers-reduced-motion: reduce) {
  .status-indicator.listening,
  .core-overlay-icon.listening,
  .action-btn.primary.mic.recording {
    animation: none !important;
  }

  .panel-glow,
  .status-indicator,
  .close-btn,
  .core-overlay-icon,
  .voice-copy-card,
  .action-btn {
    transition-duration: 1ms !important;
  }
}
</style>
