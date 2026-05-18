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
const waveSeed = ref(0);
let waveTimer: ReturnType<typeof setInterval> | null = null;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;

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

function waveStyle(index: number) {
  const phase = waveSeed.value + index * 0.74;
  const active = status.value === "listening" || status.value === "speaking";
  const height = active ? 14 + Math.abs(Math.sin(phase)) * 38 : 12 + Math.abs(Math.sin(phase)) * 16;
  return {
    height: `${Math.round(height)}px`,
    opacity: active ? 0.72 + Math.abs(Math.cos(phase)) * 0.24 : 0.38,
  };
}

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

async function sendTranscript() {
  const message = transcript.value.trim();
  if (!message || chat.isLoading || isSending.value) return;

  chat.addMessage("user", message);
  transcript.value = "";
  errorText.value = "";
  isSending.value = true;
  chat.isLoading = true;
  pet.setState("thinking");

  try {
    await invoke<{ started: boolean }>("send_to_ai", { message });
    await currentWindow.emit("voice-message-sent", message);
  } catch (err) {
    chat.isLoading = false;
    isSending.value = false;
    errorText.value = `发送失败: ${err}`;
    status.value = "error";
    pet.setState("confused");
  }
}

function cancelVoice() {
  voice.abortListening();
  ttsPlayer.stop();
  transcript.value = "";
  interimText.value = "";
  errorText.value = "";
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

  waveTimer = setInterval(() => {
    waveSeed.value += 0.18;
  }, 80);

  unlistenAiFinished = await listen<AiFinishedPayload>("ai-finished", async (event) => {
    isSending.value = false;
    chat.isLoading = false;
    pet.setState("speaking");
    if (settings.value.enabled && settings.value.autoSpeak) {
      isGenerating.value = true;
      status.value = "speaking";
      try {
        await ttsPlayer.speak(event.payload.text, ttsSettings.value);
      } catch (err) {
        errorText.value = err instanceof Error ? err.message : String(err);
        status.value = "error";
      }
      isGenerating.value = false;
    }
    if (status.value === "speaking") status.value = "idle";
    if (pet.state === "speaking") pet.setState("idle");
  });

  unlistenAiError = await listen<AiErrorPayload>("ai-error", (event) => {
    isSending.value = false;
    chat.isLoading = false;
    errorText.value = event.payload.aborted ? "已中止" : event.payload.message;
    status.value = event.payload.aborted ? "idle" : "error";
    pet.setState(event.payload.aborted ? "idle" : "confused");
  });
});

onBeforeUnmount(() => {
  if (waveTimer) clearInterval(waveTimer);
  unlistenAiFinished?.();
  unlistenAiError?.();
  voice.abortListening();
  ttsPlayer.stop();
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

    <div class="visual-core">
      <div class="orb-container" :class="{ active: status === 'listening', speaking: status === 'speaking' || isGenerating }">
        <div class="orb-glow"></div>
        <div class="orb">
          <div class="orb-inner">
            <svg v-if="status === 'listening'" width="24" height="24" viewBox="0 0 24 24" fill="none">
              <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" fill="currentColor"/>
              <path d="M19 10v2a7 7 0 0 1-14 0v-2M12 19v4M8 23h8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
            <svg v-else-if="status === 'speaking' || isGenerating" width="24" height="24" viewBox="0 0 24 24" fill="none">
              <path d="M11 5L6 9H2v6h4l5 4V5zM19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.08" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
            <svg v-else width="22" height="22" viewBox="0 0 24 24" fill="none">
              <circle cx="12" cy="12" r="3" fill="currentColor"/>
              <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
          </div>
        </div>
        <div class="ring ring-1"></div>
        <div class="ring ring-2"></div>
        <div class="ring ring-3"></div>
      </div>

      <div class="waveform" aria-hidden="true">
        <span v-for="i in 32" :key="i" :style="waveStyle(i)" />
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

    <div class="shortcut-hint">{{ settings.shortcut }}</div>
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
  padding: 18px 20px 16px;
  color: #e2e8f0;
  background: linear-gradient(160deg, rgba(10, 15, 30, 0.96), rgba(15, 23, 42, 0.98));
  border: 1px solid rgba(100, 200, 255, 0.08);
  border-radius: 22px;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}

.panel-glow {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(ellipse 60% 40% at 20% 0%, rgba(56, 189, 248, 0.08), transparent),
    radial-gradient(ellipse 50% 35% at 80% 10%, rgba(168, 85, 247, 0.06), transparent);
  transition: opacity 0.4s;
}

.voice-panel.active .panel-glow {
  background:
    radial-gradient(ellipse 60% 40% at 20% 0%, rgba(34, 197, 94, 0.12), transparent),
    radial-gradient(ellipse 50% 35% at 80% 10%, rgba(56, 189, 248, 0.08), transparent);
}

.voice-panel.speaking .panel-glow,
.voice-panel.generating .panel-glow {
  background:
    radial-gradient(ellipse 60% 40% at 20% 0%, rgba(56, 189, 248, 0.12), transparent),
    radial-gradient(ellipse 50% 35% at 80% 10%, rgba(168, 85, 247, 0.1), transparent);
}

.voice-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: 0 0 auto;
  margin-bottom: 6px;
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
  box-shadow: 0 0 8px rgba(74, 222, 128, 0.6);
}

.status-indicator.speaking {
  background: #38bdf8;
  box-shadow: 0 0 8px rgba(56, 189, 248, 0.6);
}

.status-indicator.error {
  background: #fb7185;
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
  transition: color 0.2s, background 0.2s;
}

.close-btn:hover {
  color: #e2e8f0;
  background: rgba(255, 255, 255, 0.08);
}

.visual-core {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 8px 0;
}

.orb-container {
  position: relative;
  width: 100px;
  height: 100px;
  display: grid;
  place-items: center;
}

.orb-glow {
  position: absolute;
  inset: -10px;
  border-radius: 50%;
  background: radial-gradient(circle, rgba(56, 189, 248, 0.15), transparent 70%);
  opacity: 0;
  transition: opacity 0.5s;
}

.orb-container.active .orb-glow {
  background: radial-gradient(circle, rgba(74, 222, 128, 0.2), transparent 70%);
  opacity: 1;
}

.orb-container.speaking .orb-glow {
  background: radial-gradient(circle, rgba(139, 92, 246, 0.2), transparent 70%);
  opacity: 1;
}

.orb {
  width: 60px;
  height: 60px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  background: linear-gradient(135deg, #1e293b, #0f172a);
  border: 1.5px solid rgba(100, 200, 255, 0.15);
  z-index: 2;
  transition: border-color 0.3s, box-shadow 0.3s;
}

.orb-container.active .orb {
  border-color: rgba(74, 222, 128, 0.4);
  box-shadow: 0 0 20px rgba(74, 222, 128, 0.15);
}

.orb-container.speaking .orb {
  border-color: rgba(139, 92, 246, 0.4);
  box-shadow: 0 0 20px rgba(139, 92, 246, 0.15);
}

.orb-inner {
  color: #94a3b8;
  transition: color 0.3s;
}

.orb-container.active .orb-inner { color: #4ade80; }
.orb-container.speaking .orb-inner { color: #a78bfa; }

.ring {
  position: absolute;
  border-radius: 50%;
  border: 1px solid rgba(56, 189, 248, 0.1);
  animation: pulse 3s ease-in-out infinite;
}

.ring-1 { inset: 8px; animation-delay: 0s; }
.ring-2 { inset: 0px; animation-delay: 0.8s; opacity: 0.6; }
.ring-3 { inset: -8px; animation-delay: 1.6s; opacity: 0.3; }

.orb-container.active .ring {
  border-color: rgba(74, 222, 128, 0.15);
  animation-duration: 1.5s;
}

.orb-container.speaking .ring {
  border-color: rgba(139, 92, 246, 0.15);
  animation-duration: 1.8s;
}

.waveform {
  height: 40px;
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 3px;
  padding: 0 16px;
}

.waveform span {
  width: 3px;
  border-radius: 999px;
  background: linear-gradient(180deg, rgba(56, 189, 248, 0.6), rgba(139, 92, 246, 0.4));
  transition: height 0.1s ease, opacity 0.1s ease;
}

.content-area {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
}

.transcript {
  flex: 1 1 auto;
  min-height: 48px;
  box-sizing: border-box;
  width: 100%;
  padding: 10px 12px;
  resize: none;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 12px;
  outline: none;
  color: #f1f5f9;
  background: rgba(255, 255, 255, 0.03);
  font: inherit;
  font-size: 13px;
  line-height: 1.5;
  transition: border-color 0.2s;
}

.transcript:focus {
  border-color: rgba(56, 189, 248, 0.2);
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
}

.action-btn {
  height: 36px;
  border: 0;
  border-radius: 10px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s, transform 0.1s;
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
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.action-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.shortcut-hint {
  text-align: center;
  font-size: 10px;
  color: #334155;
  padding-top: 4px;
}

@keyframes pulse {
  0%, 100% { transform: scale(0.95); opacity: 0.4; }
  50% { transform: scale(1.1); opacity: 0.8; }
}
</style>
