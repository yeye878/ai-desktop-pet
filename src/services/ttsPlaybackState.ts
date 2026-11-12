import { listen } from "@tauri-apps/api/event";
import { ref } from "vue";

/**
 * 跨窗口共享的 TTS 播放状态。
 *
 * 后端在音频真正开始/结束播放时会广播 `tts-playback-started` / `tts-playback-ended`，
 * 这里把它收敛成一份所有窗口都能读的响应式状态，避免 Dashboard、VoicePanel 各自
 * 维护一套"我以为在播"的假状态（之前就是这样，导致波形对着空气跳舞）。
 */
export interface PlaybackEventPayload {
  playback_id: number;
  text: string;
}

/** 是否正在播放（有任意一句正在出声）。 */
export const isSpeaking = ref(false);
/** 正在播放的文本片段，可用于字幕/高亮。 */
export const speakingText = ref("");
/** 最近一次会话 id，供调用方丢弃过期结束事件。 */
export const activePlaybackId = ref(0);

let subscribed = false;
let unlistenStarted: (() => void) | null = null;
let unlistenEnded: (() => void) | null = null;

function applyStarted(payload: PlaybackEventPayload | undefined) {
  const id = payload?.playback_id ?? 0;
  activePlaybackId.value = id;
  speakingText.value = payload?.text ?? "";
  isSpeaking.value = true;
}

function applyEnded(payload: PlaybackEventPayload | undefined) {
  const id = payload?.playback_id ?? 0;
  // 非当前会话（被打断/被新一句顶替）的结束事件不应把新播放标成结束；
  // id 为 0 代表 `tts_stop` 主动打断，无条件复位。
  if (id !== 0 && id !== activePlaybackId.value) return;
  activePlaybackId.value = 0;
  speakingText.value = "";
  isSpeaking.value = false;
}

/**
 * 订阅播放事件。幂等：重复调用不会叠加监听。
 * 通常在应用的根组件 onMounted 里调一次即可。
 */
export async function ensurePlaybackSubscription() {
  if (subscribed) return;
  subscribed = true;
  unlistenStarted = await listen<PlaybackEventPayload>("tts-playback-started", (event) => {
    applyStarted(event.payload);
  });
  unlistenEnded = await listen<PlaybackEventPayload>("tts-playback-ended", (event) => {
    applyEnded(event.payload);
  });
}

export function resetPlaybackState() {
  isSpeaking.value = false;
  speakingText.value = "";
  activePlaybackId.value = 0;
}

export function disposePlaybackSubscription() {
  unlistenStarted?.();
  unlistenEnded?.();
  unlistenStarted = null;
  unlistenEnded = null;
  subscribed = false;
  resetPlaybackState();
}
