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
const currentWindow = getCurrentWindow();
const chat = useChatStore();
const pet = usePetStore();
const voice = new VoiceController();

const settings = ref<VoiceSettings>({ ...DEFAULT_VOICE_SETTINGS });
const status = ref<VoiceStatus>("idle");
const transcript = ref("");
const interimText = ref("");
const errorText = ref("");
const isSending = ref(false);
const waveSeed = ref(0);
let waveTimer: ReturnType<typeof setInterval> | null = null;
let unlistenAiFinished: UnlistenFn | null = null;
let unlistenAiError: UnlistenFn | null = null;

const canListen = computed(() => settings.value.enabled && isSpeechRecognitionSupported() && !chat.isLoading);
const statusLabel = computed(() => {
  if (!settings.value.enabled) return "语音功能已关闭";
  if (!isSpeechRecognitionSupported()) return "当前环境不支持语音识别";
  if (chat.isLoading || isSending.value) return "正在等待回复";
  if (status.value === "listening") return "正在聆听";
  if (status.value === "speaking") return "正在播报";
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

  voice.stopSpeaking();
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
  voice.stopSpeaking();
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
    status.value = settings.value.autoSpeak ? "speaking" : "idle";
    pet.setState("speaking");
    if (settings.value.enabled && settings.value.autoSpeak) {
      try {
        await voice.speak(event.payload.text, settings.value);
      } catch (err) {
        errorText.value = err instanceof Error ? err.message : String(err);
        status.value = "error";
      }
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
  voice.stopSpeaking();
});
</script>

<template>
  <section class="voice-panel">
    <div class="voice-header">
      <div>
        <div class="eyebrow">Voice Mode</div>
        <h2>语音对话</h2>
      </div>
      <button class="icon-btn" title="关闭" @click="cancelVoice">×</button>
    </div>

    <div class="orb-wrap" :class="{ active: status === 'listening', speaking: status === 'speaking' }">
      <div class="orb">
        <span>{{ status === "listening" ? "听" : status === "speaking" ? "说" : "AI" }}</span>
      </div>
      <div class="ring ring-a"></div>
      <div class="ring ring-b"></div>
    </div>

    <div class="waveform" aria-hidden="true">
      <span v-for="index in 28" :key="index" :style="waveStyle(index)" />
    </div>

    <div class="status-row">
      <span class="status-dot" :class="status"></span>
      <span>{{ statusLabel }}</span>
      <small>{{ settings.shortcut }}</small>
    </div>

    <textarea
      v-model="transcript"
      class="transcript"
      :placeholder="displayText"
      :disabled="status === 'listening' || isSending || chat.isLoading"
    />

    <div v-if="interimText || errorText" class="live-caption" :class="{ error: errorText }">
      {{ interimText || errorText }}
    </div>

    <div class="actions">
      <button class="secondary-btn" :disabled="status === 'listening'" @click="clearTranscript">清空</button>
      <button class="primary-btn listen" :disabled="chat.isLoading && status !== 'listening'" @click="startListening">
        {{ primaryLabel }}
      </button>
      <button class="primary-btn" :disabled="!transcript.trim() || chat.isLoading || isSending" @click="sendTranscript">
        发送
      </button>
    </div>
  </section>
</template>

<style scoped>
.voice-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  padding: 16px 18px 18px;
  color: #f8fafc;
  background:
    radial-gradient(circle at 20% 12%, rgba(20, 184, 166, 0.32), transparent 30%),
    radial-gradient(circle at 78% 20%, rgba(244, 114, 182, 0.28), transparent 26%),
    linear-gradient(145deg, rgba(15, 23, 42, 0.94), rgba(30, 41, 59, 0.88));
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: 22px;
  box-shadow: 0 28px 80px rgba(2, 6, 23, 0.44), inset 0 1px 0 rgba(255, 255, 255, 0.16);
  overflow: hidden;
  min-height: 0;
}

.voice-header,
.status-row,
.actions {
  display: flex;
  align-items: center;
}

.voice-header {
  justify-content: space-between;
  flex: 0 0 auto;
}

.eyebrow {
  font-size: 10px;
  letter-spacing: 0;
  color: #67e8f9;
  text-transform: uppercase;
}

h2 {
  margin: 2px 0 0;
  font-size: 22px;
  font-weight: 750;
}

.icon-btn {
  width: 30px;
  height: 30px;
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 50%;
  color: #e2e8f0;
  background: rgba(255, 255, 255, 0.08);
  cursor: pointer;
  font-size: 20px;
  line-height: 1;
}

.orb-wrap {
  flex: 0 0 auto;
  position: relative;
  width: 118px;
  height: 118px;
  margin: 2px auto 0;
  display: grid;
  place-items: center;
}

.orb {
  width: 76px;
  height: 76px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  background: linear-gradient(145deg, #22d3ee, #a78bfa 52%, #fb7185);
  box-shadow: 0 18px 46px rgba(34, 211, 238, 0.32);
  z-index: 2;
}

.orb span {
  font-size: 20px;
  font-weight: 800;
}

.ring {
  position: absolute;
  inset: 22px;
  border-radius: 50%;
  border: 1px solid rgba(103, 232, 249, 0.28);
}

.ring-a {
  animation: breathe 2.4s ease-in-out infinite;
}

.ring-b {
  inset: 10px;
  border-color: rgba(251, 113, 133, 0.2);
  animation: breathe 2.4s ease-in-out infinite reverse;
}

.orb-wrap.active .ring,
.orb-wrap.speaking .ring {
  animation-duration: 1.1s;
}

.waveform {
  flex: 0 0 56px;
  height: 56px;
  padding: 0 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
}

.waveform span {
  width: 5px;
  border-radius: 999px;
  background: linear-gradient(180deg, #67e8f9, #a78bfa);
  transition: height 0.12s ease, opacity 0.12s ease;
}

.status-row {
  flex: 0 0 auto;
  gap: 8px;
  justify-content: center;
  color: #cbd5e1;
  font-size: 13px;
}

.status-row small {
  padding: 3px 7px;
  border-radius: 999px;
  color: #94a3b8;
  background: rgba(255, 255, 255, 0.08);
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #94a3b8;
}

.status-dot.listening {
  background: #22c55e;
  box-shadow: 0 0 16px rgba(34, 197, 94, 0.8);
}

.status-dot.speaking {
  background: #38bdf8;
}

.status-dot.error {
  background: #fb7185;
}

.transcript {
  flex: 1 1 auto;
  min-height: 58px;
  box-sizing: border-box;
  width: 100%;
  height: auto;
  padding: 12px;
  resize: none;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 14px;
  outline: none;
  color: #f8fafc;
  background: rgba(15, 23, 42, 0.42);
  font: inherit;
  line-height: 1.5;
}

.transcript::placeholder {
  color: #94a3b8;
}

.live-caption {
  flex: 0 0 auto;
  min-height: 18px;
  color: #a5f3fc;
  font-size: 12px;
  text-align: center;
}

.live-caption.error {
  color: #fecdd3;
}

.actions {
  flex: 0 0 auto;
  gap: 10px;
  margin-top: 2px;
}

.primary-btn,
.secondary-btn {
  height: 38px;
  border: 0;
  border-radius: 12px;
  cursor: pointer;
  font-weight: 700;
}

.primary-btn {
  flex: 1;
  color: #0f172a;
  background: linear-gradient(135deg, #67e8f9, #f0abfc);
}

.primary-btn.listen {
  flex: 1.25;
}

.secondary-btn {
  width: 70px;
  color: #e2e8f0;
  background: rgba(255, 255, 255, 0.1);
}

button:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

@keyframes breathe {
  0%, 100% {
    transform: scale(0.94);
    opacity: 0.42;
  }
  50% {
    transform: scale(1.18);
    opacity: 0.86;
  }
}
</style>
