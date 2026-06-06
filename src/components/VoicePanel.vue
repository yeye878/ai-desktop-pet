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

async function startListening() {
  if (status.value === "listening") {
    voice.stopListening();
    return;
  }
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

function clearSendTimeout() {
  if (sendTimeoutId) {
    clearTimeout(sendTimeoutId);
    sendTimeoutId = null;
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
    await invoke<{ started: boolean }>("send_to_ai", { message, attachments: [] });
    transcript.value = "";
    await currentWindow.emit("voice-message-sent", message);
  } catch (err) {
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
    autoListenTimer = setTimeout(() => {
      autoListenTimer = null;
      startListening();
    }, 300);
  }

  // Start canvas fluid animation loop
  const canvas = fluidCanvasRef.value;
  if (canvas) {
    const ctx = canvas.getContext("2d");
    if (ctx) {
      const dpr = window.devicePixelRatio || 1;
      const W = 240;
      const H = 150;
      canvas.width = W * dpr;
      canvas.height = H * dpr;
      canvas.style.width = W + "px";
      canvas.style.height = H + "px";
      ctx.scale(dpr, dpr);

      const render = (timestamp: number) => {
        const time = timestamp * 0.001;
        ctx.clearRect(0, 0, W, H);

        const cx = W / 2;
        const cy = H / 2 - 12;
        const R = 32;

        // Particle generation in listening mode
        if (status.value === "listening" && Math.random() < 0.35) {
          const angle = Math.random() * Math.PI * 2;
          sparks.value.push({
            x: cx + Math.cos(angle) * R,
            y: cy + Math.sin(angle) * R,
            vx: Math.cos(angle) * (0.8 + Math.random() * 1.5),
            vy: Math.sin(angle) * (0.8 + Math.random() * 1.5),
            life: 1.0,
            size: 1.2 + Math.random() * 1.5,
            color: Math.random() < 0.5 ? "#4ade80" : "#22d3ee"
          });
        }

        // Draw sparks
        for (let i = sparks.value.length - 1; i >= 0; i--) {
          const s = sparks.value[i];
          s.x += s.vx;
          s.y += s.vy;
          s.life -= 0.025;
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

        // Render overlapping fluid layers
        const drawBlob = (color: string, speedMult: number, phaseOffset: number, scale: number) => {
          ctx.save();
          ctx.fillStyle = color;
          ctx.shadowColor = color;
          ctx.shadowBlur = 10;
          ctx.globalCompositeOperation = "screen";

          ctx.beginPath();
          const numPoints = 64;
          for (let i = 0; i < numPoints; i++) {
            const theta = (i / numPoints) * Math.PI * 2;
            let offset = 0;

            if (status.value === "listening") {
              offset += Math.sin(theta * 3 + time * 14 * speedMult + phaseOffset) * 8;
              offset += Math.cos(theta * 5 - time * 18 * speedMult) * 4;
            } else if (status.value === "speaking" || isGenerating.value) {
              const speakAmp = status.value === "speaking" ? 11 : 6;
              offset += Math.sin(theta * 2 + time * 9 * speedMult + phaseOffset) * speakAmp;
              offset += Math.cos(theta * 4 - time * 13 * speedMult) * (speakAmp * 0.4);
            } else {
              // Gentle breathing/morphing blob
              offset += Math.sin(theta * 2.5 + time * 2.2 * speedMult + phaseOffset) * 3;
              offset += Math.cos(theta * 3.5 - time * 2.8 * speedMult) * 1.5;
            }

            const r = (R + offset) * scale;
            const x = cx + Math.cos(theta) * r;
            const y = cy + Math.sin(theta) * r;

            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          }
          ctx.closePath();
          ctx.fill();
          ctx.restore();
        };

        // Draw background shadow layer
        ctx.globalAlpha = 0.16;
        drawBlob("#4338ca", 0.8, Math.PI, 1.1);

        // Core cyan fluid blob
        const cyanCol = status.value === "listening" ? "rgba(74, 222, 128, 0.42)" : "rgba(56, 189, 248, 0.45)";
        ctx.globalAlpha = 0.65;
        drawBlob(cyanCol, 1.0, 0, 1.0);

        // Core violet fluid blob
        const violetCol = status.value === "listening" ? "rgba(34, 211, 238, 0.38)" : "rgba(167, 139, 250, 0.4)";
        ctx.globalAlpha = 0.55;
        drawBlob(violetCol, 1.1, Math.PI * 0.5, 0.95);

        // Draw waves
        ctx.globalAlpha = 0.85;
        const wave1Col = status.value === "listening" ? "rgba(74, 222, 128, 0.45)" : "rgba(56, 189, 248, 0.45)";
        const wave2Col = status.value === "listening" ? "rgba(34, 211, 238, 0.35)" : "rgba(167, 139, 250, 0.35)";

        // Render 2 continuous wave channels
        const drawSpectrumWave = (color: string, ampMult: number, phase: number) => {
          ctx.strokeStyle = color;
          ctx.lineWidth = 1.8;
          ctx.beginPath();
          const wavePoints = 36;
          for (let i = 0; i <= wavePoints; i++) {
            const x = (i / wavePoints) * W;
            let y = H - 24;

            let amp = 2;
            let freq = 0.08;
            let speed = 4;
            if (status.value === "listening") {
              amp = 18; freq = 0.11; speed = 16;
            } else if (status.value === "speaking" || isGenerating.value) {
              amp = 12; freq = 0.09; speed = 11;
            } else {
              amp = 1.5; freq = 0.05; speed = 2.5;
            }

            y += Math.sin(i * freq + time * speed + phase) * amp * ampMult;
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          }
          ctx.stroke();
        };

        drawSpectrumWave(wave1Col, 1.0, 0);
        drawSpectrumWave(wave2Col, 0.8, Math.PI * 0.6);

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
});

onBeforeUnmount(() => {
  clearSendTimeout();
  if (autoListenTimer) {
    clearTimeout(autoListenTimer);
    autoListenTimer = null;
  }
  cancelAnimationFrame(animId);
  unlistenAiFinished?.();
  unlistenAiError?.();
  voice.abortListening();
  ttsPlayer.stop();
  isSending.value = false;
  chat.isLoading = false;
});
</script>

<template>
  <section class="voice-panel" :class="{ active: status === 'listening', speaking: status === 'speaking', generating: isGenerating }">
    <div class="cyber-scanlines"></div>
    <div class="panel-glow"></div>

    <!-- Tech telemetry readouts in the corners -->
    <div class="tech-tag top-left">[SYS_RECOG: WEB_SPEECH]</div>
    <div class="tech-tag top-right">[GAIN_FACTOR: 1.2X]</div>
    <div class="tech-tag bottom-left">[DECODE: ACTIVE]</div>
    <div class="tech-tag bottom-right">[SYS_SAMPLING: 44.1KHZ]</div>

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

    <div class="visual-core">
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

    <div class="content-area">
      <textarea
        v-model="transcript"
        class="transcript"
        :placeholder="displayText"
        :disabled="status === 'listening' || isSending || chat.isLoading"
        rows="2"
      />
      <div v-if="interimText || errorText" class="live-caption" :class="{ error: !!errorText }">
        {{ interimText || errorText }}
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
</style>
