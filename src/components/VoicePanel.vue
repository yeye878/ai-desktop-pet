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
let sendTimeoutId: ReturnType<typeof setTimeout> | null = null;
let autoListenTimer: ReturnType<typeof setTimeout> | null = null;
let speakGeneration = 0;

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

  unlistenAiFinished = await listen<AiFinishedPayload>("ai-finished", async (event) => {
    clearSendTimeout();
    isSending.value = false;
    chat.isLoading = false;
    pet.setState("speaking");
    if (settings.value.enabled && settings.value.autoSpeak) {
      const gen = ++speakGeneration;
      isGenerating.value = true;
      status.value = "speaking";
      try {
        await ttsPlayer.speak(event.payload.text, ttsSettings.value);
      } catch (err: any) {
        if (err?.message !== "Aborted") {
          errorText.value = err instanceof Error ? err.message : String(err);
          status.value = "error";
        }
      }
      // 若已被新的 TTS 请求取代，跳过状态恢复
      if (gen !== speakGeneration) return;
      isGenerating.value = false;
    }
    if (status.value === "speaking") status.value = "idle";
    if (pet.state === "speaking" && status.value !== "error") pet.setState("idle");
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
});

onBeforeUnmount(() => {
  clearSendTimeout();
  clearAutoListenTimer();
  cancelAnimationFrame(animId);
  unlistenAiFinished?.();
  unlistenAiError?.();
  unlistenStartListening?.();
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
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M1 1l12 12M13 1L1 13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
      </button>
    </div>

    <div class="visual-core voice-stage">
      <!-- Dynamic fluid canvas rendering both the blob core and the bottom waves -->
      <div class="canvas-container">
        <canvas ref="fluidCanvasRef" class="fluid-canvas"></canvas>
        <div class="core-overlay-icon" :class="status">
          <svg v-if="status === 'listening'" width="20" height="20" viewBox="0 0 24 24" fill="none">
            <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" fill="currentColor"/>
            <path d="M19 10v2a7 7 0 0 1-14 0v-2M12 19v4M8 23h8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
          </svg>
          <svg v-else-if="status === 'speaking' || isGenerating" width="20" height="20" viewBox="0 0 24 24" fill="none">
            <path d="M11 5L6 9H2v6h4l5 4V5zM19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.08" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
          </svg>
          <svg v-else width="18" height="18" viewBox="0 0 24 24" fill="none">
            <circle cx="12" cy="12" r="3" fill="currentColor"/>
            <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
          </svg>
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
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  padding: 22px 20px 16px;
  color: #e2e8f0;
  background: linear-gradient(165deg, rgba(8, 12, 28, 0.88), rgba(12, 18, 38, 0.94));
  border: 1px solid rgba(56, 189, 248, 0.15);
  box-shadow: 
    0 20px 40px rgba(0, 0, 0, 0.6),
    inset 0 0 24px rgba(56, 189, 248, 0.05);
  border-radius: 24px;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  backdrop-filter: blur(20px);
}

/* Cyber Telemetry grid & scanlines */
.cyber-scanlines {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: linear-gradient(rgba(18, 16, 16, 0) 50%, rgba(0, 0, 0, 0.25) 50%), linear-gradient(90deg, rgba(255, 0, 0, 0.06), rgba(0, 255, 0, 0.02), rgba(0, 0, 255, 0.06));
  background-size: 100% 4px, 6px 100%;
  z-index: 10;
  opacity: 0.45;
}

.tech-tag {
  position: absolute;
  font-family: monospace, monospace;
  font-size: 8px;
  font-weight: 700;
  color: rgba(56, 189, 248, 0.35);
  letter-spacing: 0.5px;
  pointer-events: none;
  z-index: 3;
}

.tech-tag.top-left { top: 8px; left: 14px; }
.tech-tag.top-right { top: 8px; right: 14px; }
.tech-tag.bottom-left { bottom: 8px; left: 14px; }
.tech-tag.bottom-right { bottom: 8px; right: 14px; }

.panel-glow {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(ellipse 60% 40% at 20% 0%, rgba(56, 189, 248, 0.12), transparent),
    radial-gradient(ellipse 50% 35% at 80% 10%, rgba(168, 85, 247, 0.08), transparent);
  transition: opacity 0.4s;
  z-index: 1;
}

.voice-panel.active .panel-glow {
  background:
    radial-gradient(ellipse 60% 40% at 20% 0%, rgba(74, 222, 128, 0.16), transparent),
    radial-gradient(ellipse 50% 35% at 80% 10%, rgba(56, 189, 248, 0.1), transparent);
}

.voice-panel.speaking .panel-glow,
.voice-panel.generating .panel-glow {
  background:
    radial-gradient(ellipse 60% 40% at 20% 0%, rgba(56, 189, 248, 0.16), transparent),
    radial-gradient(ellipse 50% 35% at 80% 10%, rgba(168, 85, 247, 0.14), transparent);
}

.voice-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: 0 0 auto;
  margin-bottom: 6px;
  z-index: 2;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.badge {
  font-size: 9px;
  font-weight: 700;
  letter-spacing: 1.2px;
  padding: 3px 8px;
  border-radius: 4px;
  background: rgba(56, 189, 248, 0.12);
  color: #7dd3fc;
  border: 1px solid rgba(56, 189, 248, 0.15);
  text-shadow: 0 0 4px rgba(56, 189, 248, 0.4);
}

.status-indicator {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #475569;
  transition: all 0.3s;
}

.status-indicator.listening {
  background: #4ade80;
  box-shadow: 0 0 8px rgba(74, 222, 128, 0.8);
}

.status-indicator.speaking {
  background: #38bdf8;
  box-shadow: 0 0 8px rgba(56, 189, 248, 0.8);
}

.status-indicator.error {
  background: #fb7185;
  box-shadow: 0 0 6px rgba(251, 113, 133, 0.6);
}

.status-text {
  font-size: 12px;
  color: #94a3b8;
}

.close-btn {
  width: 28px;
  height: 28px;
  display: grid;
  place-items: center;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
  color: #64748b;
  cursor: pointer;
  transition: all 0.2s;
}

.close-btn:hover {
  color: #e2e8f0;
  background: rgba(255, 255, 255, 0.08);
  border-color: rgba(255, 255, 255, 0.12);
}

.visual-core {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 2px 0;
  z-index: 2;
}

.canvas-container {
  position: relative;
  width: 240px;
  height: 150px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.fluid-canvas {
  position: absolute;
  inset: 0;
}

.core-overlay-icon {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -64%);
  width: 48px;
  height: 48px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(15, 23, 42, 0.45);
  border: 1px solid rgba(255, 255, 255, 0.08);
  color: #94a3b8;
  backdrop-filter: blur(4px);
  z-index: 5;
  transition: all 0.3s;
}

.core-overlay-icon.listening {
  color: #4ade80;
  border-color: rgba(74, 222, 128, 0.35);
  box-shadow: 0 0 16px rgba(74, 222, 128, 0.15);
}

.core-overlay-icon.speaking {
  color: #a78bfa;
  border-color: rgba(167, 139, 250, 0.35);
  box-shadow: 0 0 16px rgba(167, 139, 250, 0.15);
}

.content-area {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
  z-index: 2;
}

.transcript {
  flex: 1 1 auto;
  min-height: 48px;
  box-sizing: border-box;
  width: 100%;
  padding: 10px 12px;
  resize: none;
  border: 1px solid rgba(56, 189, 248, 0.12);
  border-radius: 12px;
  outline: none;
  color: #f1f5f9;
  background: rgba(15, 23, 42, 0.45);
  font: inherit;
  font-size: 13px;
  line-height: 1.5;
  transition: all 0.3s;
  backdrop-filter: blur(4px);
}

.transcript:focus {
  border-color: rgba(56, 189, 248, 0.35);
  box-shadow: 0 0 8px rgba(56, 189, 248, 0.12);
  background: rgba(15, 23, 42, 0.6);
}

.transcript::placeholder {
  color: #475569;
}

.live-caption {
  font-size: 11px;
  color: #7dd3fc;
  text-align: center;
  padding: 0 4px;
}

.live-caption.error {
  color: #fca5a5;
}

.actions {
  flex: 0 0 auto;
  display: flex;
  gap: 8px;
  padding-top: 4px;
  z-index: 2;
}

.action-btn {
  height: 36px;
  border: 0;
  border-radius: 10px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s, transform 0.1s, border-color 0.2s;
}

.action-btn:active:not(:disabled) {
  transform: scale(0.97);
}

.action-btn.primary {
  flex: 1;
  color: #0f172a;
  background: linear-gradient(135deg, #38bdf8, #a78bfa);
}

.action-btn.primary.mic {
  flex: 1.3;
}

.action-btn.primary.mic.recording {
  background: linear-gradient(135deg, #4ade80, #22d3ee);
}

.action-btn.secondary {
  width: 60px;
  color: #94a3b8;
  background: rgba(15, 23, 42, 0.45);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.action-btn.secondary:hover {
  background: rgba(15, 23, 42, 0.6);
  border-color: rgba(56, 189, 248, 0.2);
}

.action-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.shortcut-hint {
  text-align: center;
  font-size: 10px;
  color: #64748b;
  padding-top: 4px;
  z-index: 2;
}

/* Product polish layer */
.voice-panel {
  padding: 20px 18px 16px;
  color: #223047;
  background:
    url("../assets/art/paper-grain.webp"),
    radial-gradient(circle at 18% 0%, rgba(102, 200, 255, 0.22), transparent 34%),
    radial-gradient(circle at 88% 10%, rgba(142, 230, 168, 0.18), transparent 30%),
    linear-gradient(155deg, rgba(255, 255, 255, 0.90), rgba(242, 250, 255, 0.82));
  background-size: 420px 420px, auto, auto, auto;
  border: 1px solid rgba(33, 48, 74, 0.12);
  border-radius: 18px;
  box-shadow:
    0 18px 42px rgba(33, 48, 74, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.78);
  backdrop-filter: blur(24px) saturate(1.25);
  -webkit-backdrop-filter: blur(24px) saturate(1.25);
}

.cyber-scanlines,
.tech-tag {
  display: none;
}

.panel-glow {
  background:
    radial-gradient(circle at 50% 18%, rgba(102, 200, 255, 0.16), transparent 34%),
    linear-gradient(180deg, rgba(255, 255, 255, 0.12), transparent 42%);
  opacity: 0.84;
}

.voice-panel.active .panel-glow,
.voice-panel.speaking .panel-glow,
.voice-panel.generating .panel-glow {
  background:
    radial-gradient(circle at 50% 18%, rgba(142, 230, 168, 0.18), transparent 34%),
    radial-gradient(circle at 80% 0%, rgba(102, 200, 255, 0.16), transparent 30%);
}

.voice-header {
  min-height: 34px;
  margin-bottom: 8px;
  padding: 0 2px;
}

.header-left {
  min-width: 0;
}

.badge {
  border-color: rgba(102, 200, 255, 0.22);
  border-radius: 999px;
  background: rgba(102, 200, 255, 0.12);
  color: #24749d;
  text-shadow: none;
}

.status-text {
  min-width: 0;
  overflow: hidden;
  color: #64748b;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.close-btn {
  border-color: rgba(33, 48, 74, 0.10);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.72);
  color: #64748b;
  box-shadow: 0 6px 14px rgba(33, 48, 74, 0.06);
}

.close-btn:hover {
  border-color: rgba(239, 68, 68, 0.28);
  background: #ffffff;
  color: #dc2626;
}

.close-btn:active {
  transform: translateY(1px);
}

.visual-core {
  padding: 0;
}

.canvas-container {
  width: min(100%, 248px);
  height: 152px;
  border: 1px solid rgba(33, 48, 74, 0.08);
  border-radius: 18px;
  background:
    radial-gradient(circle at 50% 38%, rgba(255, 255, 255, 0.82), transparent 30%),
    linear-gradient(180deg, rgba(255, 255, 255, 0.42), rgba(238, 248, 255, 0.38));
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.72),
    0 12px 28px rgba(33, 48, 74, 0.08);
  overflow: hidden;
}

.core-overlay-icon {
  background: rgba(255, 255, 255, 0.70);
  border-color: rgba(33, 48, 74, 0.10);
  color: #64748b;
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.76),
    0 8px 18px rgba(33, 48, 74, 0.10);
}

.core-overlay-icon.listening {
  color: #168650;
  border-color: rgba(22, 134, 80, 0.24);
  box-shadow: 0 0 0 5px rgba(22, 134, 80, 0.08), 0 8px 18px rgba(33, 48, 74, 0.08);
}

.core-overlay-icon.speaking {
  color: #6d5bd0;
  border-color: rgba(109, 91, 208, 0.22);
  box-shadow: 0 0 0 5px rgba(109, 91, 208, 0.08), 0 8px 18px rgba(33, 48, 74, 0.08);
}

.content-area {
  gap: 8px;
  margin-top: 8px;
}

.transcript {
  min-height: 74px;
  border-color: rgba(33, 48, 74, 0.12);
  border-radius: 14px;
  color: #223047;
  background: rgba(255, 255, 255, 0.76);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.76),
    0 8px 20px rgba(33, 48, 74, 0.06);
}

.transcript:hover:not(:disabled) {
  border-color: rgba(33, 48, 74, 0.18);
  background: rgba(255, 255, 255, 0.92);
}

.transcript:focus {
  border-color: rgba(102, 200, 255, 0.42);
  background: #ffffff;
  box-shadow:
    0 0 0 4px rgba(102, 200, 255, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.78);
}

.transcript::placeholder {
  color: #8b98aa;
}

.live-caption {
  min-height: 17px;
  color: #24749d;
  font-weight: 600;
}

.live-caption.error {
  color: #dc2626;
}

.actions {
  gap: 9px;
  padding-top: 8px;
}

.action-btn {
  height: 38px;
  border-radius: 11px;
  border: 1px solid rgba(33, 48, 74, 0.10);
  box-shadow: 0 8px 18px rgba(33, 48, 74, 0.08);
  transition:
    transform 0.16s ease,
    border-color 0.16s ease,
    background 0.16s ease,
    color 0.16s ease,
    box-shadow 0.16s ease,
    opacity 0.16s ease;
}

.action-btn:focus-visible,
.close-btn:focus-visible {
  outline: none;
  box-shadow: 0 0 0 4px rgba(102, 200, 255, 0.18), 0 8px 18px rgba(33, 48, 74, 0.08);
}

.action-btn:active:not(:disabled) {
  transform: translateY(1px) scale(0.99);
}

.action-btn.primary {
  color: #ffffff;
  background:
    linear-gradient(135deg, rgba(var(--pet-primary-rgb, 204, 112, 82), 0.94), rgba(var(--pet-accent-rgb, 229, 154, 128), 0.86));
  border-color: rgba(var(--pet-primary-rgb, 204, 112, 82), 0.18);
  text-shadow: 0 1px 1px rgba(33, 48, 74, 0.18);
}

.action-btn.primary:hover:not(:disabled) {
  box-shadow: 0 12px 24px rgba(var(--pet-primary-rgb, 204, 112, 82), 0.18);
}

.action-btn.primary.mic.recording {
  color: #052a1a;
  background: linear-gradient(135deg, #8ee6a8, #66c8ff);
  text-shadow: none;
}

.action-btn.secondary {
  color: #566276;
  background: rgba(255, 255, 255, 0.74);
}

.action-btn.secondary:hover:not(:disabled) {
  border-color: rgba(33, 48, 74, 0.18);
  background: #ffffff;
  color: #223047;
}

.action-btn:disabled,
.action-btn.primary:disabled {
  opacity: 1;
  color: #8793a5;
  background: rgba(33, 48, 74, 0.07);
  border-color: rgba(33, 48, 74, 0.08);
  box-shadow: none;
  text-shadow: none;
}

.shortcut-hint {
  color: #7c8798;
  font-weight: 600;
}

@media (prefers-reduced-motion: reduce) {
  .voice-panel,
  .panel-glow,
  .action-btn,
  .close-btn,
  .core-overlay-icon {
    animation: none !important;
    transition-duration: 1ms !important;
  }
}

/* Voice window redesign: compact command surface */
.voice-panel {
  padding: 18px;
  gap: 12px;
  background:
    url("../assets/art/paper-grain.webp"),
    radial-gradient(circle at 18% 0%, rgba(102, 200, 255, 0.18), transparent 34%),
    radial-gradient(circle at 88% 6%, rgba(142, 230, 168, 0.14), transparent 31%),
    linear-gradient(155deg, rgba(255, 255, 255, 0.94), rgba(240, 248, 255, 0.86));
  background-size: 420px 420px, auto, auto, auto;
}

.voice-panel::before {
  content: "";
  position: absolute;
  inset: 10px;
  pointer-events: none;
  border: 1px solid rgba(255, 255, 255, 0.72);
  border-radius: 14px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.72);
  z-index: 1;
}

.voice-panel > * {
  position: relative;
  z-index: 2;
}

.panel-glow {
  opacity: 0.62;
}

.voice-header {
  min-height: 32px;
  margin-bottom: 0;
}

.badge {
  min-height: 22px;
  display: inline-flex;
  align-items: center;
  letter-spacing: 0.08em;
}

.status-indicator {
  width: 7px;
  height: 7px;
}

.visual-core {
  flex: 1 1 auto;
  min-height: 0;
  align-items: stretch;
}

.canvas-container {
  width: 100%;
  height: 232px;
  border-color: rgba(33, 48, 74, 0.10);
  border-radius: 16px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.58), rgba(243, 250, 255, 0.38));
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.78),
    inset 0 -1px 0 rgba(33, 48, 74, 0.04),
    0 14px 34px rgba(33, 48, 74, 0.10);
}

.fluid-canvas {
  width: 100%;
  height: 100%;
}

.core-overlay-icon {
  width: 54px;
  height: 54px;
  transform: translate(-50%, -50%);
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.84), rgba(246, 250, 255, 0.62));
  border-color: rgba(33, 48, 74, 0.12);
  color: #526175;
  backdrop-filter: blur(12px) saturate(1.2);
  -webkit-backdrop-filter: blur(12px) saturate(1.2);
}

.content-area {
  flex: 0 0 auto;
  margin-top: 0;
}

.voice-copy-card {
  min-height: 72px;
  display: grid;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid rgba(33, 48, 74, 0.10);
  border-radius: 14px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.74), rgba(248, 252, 255, 0.50));
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.72),
    0 8px 22px rgba(33, 48, 74, 0.065);
}

.voice-copy-card.active {
  border-color: rgba(22, 134, 80, 0.18);
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.78), rgba(241, 253, 247, 0.58));
}

.voice-copy-card.ready {
  border-color: rgba(var(--pet-primary-rgb, 204, 112, 82), 0.20);
}

.voice-copy-card.error {
  border-color: rgba(220, 38, 38, 0.20);
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.78), rgba(255, 244, 244, 0.56));
}

.voice-copy-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  color: #718096;
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.voice-copy-head b {
  color: #42516a;
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 0.06em;
}

.voice-copy-text {
  margin: 0;
  max-height: 38px;
  overflow: hidden;
  color: #243047;
  font-size: 13px;
  font-weight: 650;
  line-height: 1.45;
  text-overflow: ellipsis;
}

.voice-copy-card:not(.active):not(.ready):not(.error) .voice-copy-text {
  color: #7c899a;
  font-weight: 600;
}

.voice-copy-card.error .voice-copy-text {
  color: #b91c1c;
}

.actions {
  padding-top: 0;
}

.action-btn {
  height: 40px;
}

.action-btn.secondary {
  width: 68px;
}

.shortcut-hint {
  padding-top: 0;
}

/* Integrated page treatment: the sound field is the window, not a card. */
.voice-panel {
  padding: 18px 18px 16px;
  gap: 10px;
  isolation: isolate;
  background:
    url("../assets/art/paper-grain.webp"),
    radial-gradient(circle at 50% 32%, rgba(255, 255, 255, 0.72), transparent 26%),
    radial-gradient(circle at 24% 18%, rgba(102, 200, 255, 0.22), transparent 34%),
    radial-gradient(circle at 88% 18%, rgba(142, 230, 168, 0.16), transparent 32%),
    linear-gradient(150deg, #fbfdff 0%, #edf8ff 48%, #fff9ef 100%);
  background-size: 420px 420px, auto, auto, auto, auto;
}

.voice-panel::before {
  inset: 0;
  border: 0;
  border-radius: inherit;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.42), transparent 24%, rgba(255, 255, 255, 0.30)),
    linear-gradient(90deg, rgba(33, 48, 74, 0.045) 1px, transparent 1px),
    linear-gradient(180deg, rgba(33, 48, 74, 0.035) 1px, transparent 1px);
  background-size: auto, 28px 28px, 28px 28px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.84);
  opacity: 0.72;
  mask-image: linear-gradient(180deg, transparent 0, #000 36px, #000 calc(100% - 18px), transparent 100%);
}

.voice-panel::after {
  content: "";
  position: absolute;
  inset: 58px 18px 96px;
  z-index: 1;
  pointer-events: none;
  border-radius: 28px;
  background:
    radial-gradient(ellipse at center, rgba(255, 255, 255, 0.54), transparent 44%),
    radial-gradient(ellipse at center, rgba(102, 200, 255, 0.12), transparent 62%);
  filter: blur(2px);
}

.voice-header,
.voice-overlay,
.actions,
.shortcut-hint {
  z-index: 4;
}

.voice-stage {
  position: absolute;
  inset: 58px 18px 108px;
  z-index: 2;
  pointer-events: none;
}

.voice-stage .canvas-container {
  position: absolute;
  inset: 0;
  width: auto;
  height: auto;
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
  overflow: visible;
}

.voice-stage .fluid-canvas {
  inset: 0;
  width: 100%;
  height: 100%;
}

.voice-stage .core-overlay-icon {
  top: 47%;
  width: 58px;
  height: 58px;
  pointer-events: auto;
  background:
    radial-gradient(circle at 35% 22%, rgba(255, 255, 255, 0.98), rgba(255, 255, 255, 0.72) 46%, rgba(242, 248, 255, 0.52));
  border-color: rgba(255, 255, 255, 0.70);
  box-shadow:
    0 0 0 1px rgba(33, 48, 74, 0.08),
    0 18px 42px rgba(33, 48, 74, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.84);
}

.voice-overlay {
  margin-top: auto;
  padding-top: 250px;
}

.voice-copy-card {
  min-height: 68px;
  border-color: rgba(255, 255, 255, 0.58);
  border-radius: 16px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.54), rgba(255, 255, 255, 0.30));
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.76),
    0 12px 30px rgba(33, 48, 74, 0.08);
  backdrop-filter: blur(18px) saturate(1.18);
  -webkit-backdrop-filter: blur(18px) saturate(1.18);
}

.voice-copy-card.active,
.voice-copy-card.ready,
.voice-copy-card.error {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.62), rgba(255, 255, 255, 0.34));
}

.voice-copy-card.error {
  border-color: rgba(220, 38, 38, 0.18);
}

.actions {
  padding-top: 2px;
}

.action-btn {
  backdrop-filter: blur(12px) saturate(1.18);
  -webkit-backdrop-filter: blur(12px) saturate(1.18);
}

.shortcut-hint {
  margin-top: -2px;
}
</style>
