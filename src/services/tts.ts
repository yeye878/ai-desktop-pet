import { invoke } from "@tauri-apps/api/core";

export interface TtsVoice {
  id: string;
  name: string;
  language: string;
  gender: string;
}

export interface TtsResult {
  audio_path: string;
  cached: boolean;
  audio_data_url?: string | null;
  mime_type?: string | null;
}

export interface TtsCacheStats {
  hits: number;
  misses: number;
  entries: number;
  total_bytes: number;
  hit_rate: number;
  legacy_mp3_purged: number;
}

export type TtsEngine = "system" | "edge";

export interface TtsSettings {
  engine: TtsEngine;
  voice: string;
  rate: number;
  pitch: number;
  volume: number;
}

export const DEFAULT_TTS_SETTINGS: TtsSettings = {
  engine: "edge",
  voice: "zh-CN-XiaoxiaoNeural",
  rate: 0,
  pitch: 0,
  volume: 100,
};

const FALLBACK_EDGE_VOICES: TtsVoice[] = [
  { id: "zh-CN-XiaoxiaoNeural", name: "晓晓 - 中文普通话", language: "zh-CN", gender: "Female" },
  { id: "zh-CN-XiaoyiNeural", name: "晓伊 - 中文普通话", language: "zh-CN", gender: "Female" },
  { id: "zh-CN-YunjianNeural", name: "云健 - 中文普通话", language: "zh-CN", gender: "Male" },
  { id: "zh-CN-YunxiNeural", name: "云希 - 中文普通话", language: "zh-CN", gender: "Male" },
  { id: "zh-CN-YunxiaNeural", name: "云夏 - 中文普通话", language: "zh-CN", gender: "Male" },
  { id: "zh-CN-YunyangNeural", name: "云扬 - 中文普通话", language: "zh-CN", gender: "Male" },
  { id: "zh-CN-liaoning-XiaobeiNeural", name: "晓北 - 东北方言", language: "zh-CN-liaoning", gender: "Female" },
  { id: "zh-CN-shaanxi-XiaoniNeural", name: "晓妮 - 陕西方言", language: "zh-CN-shaanxi", gender: "Female" },
  { id: "zh-HK-HiuGaaiNeural", name: "曉佳 - 粤语香港", language: "zh-HK", gender: "Female" },
  { id: "zh-HK-HiuMaanNeural", name: "曉曼 - 粤语香港", language: "zh-HK", gender: "Female" },
  { id: "zh-HK-WanLungNeural", name: "雲龍 - 粤语香港", language: "zh-HK", gender: "Male" },
  { id: "zh-TW-HsiaoChenNeural", name: "曉臻 - 中文台湾", language: "zh-TW", gender: "Female" },
  { id: "zh-TW-HsiaoYuNeural", name: "曉雨 - 中文台湾", language: "zh-TW", gender: "Female" },
  { id: "zh-TW-YunJheNeural", name: "雲哲 - 中文台湾", language: "zh-TW", gender: "Male" },
  { id: "en-US-JennyNeural", name: "Jenny - English US", language: "en-US", gender: "Female" },
  { id: "en-US-AriaNeural", name: "Aria - English US", language: "en-US", gender: "Female" },
  { id: "en-US-GuyNeural", name: "Guy - English US", language: "en-US", gender: "Male" },
  { id: "ja-JP-NanamiNeural", name: "Nanami - 日本語", language: "ja-JP", gender: "Female" },
  { id: "ja-JP-KeitaNeural", name: "Keita - 日本語", language: "ja-JP", gender: "Male" },
  { id: "ko-KR-SunHiNeural", name: "SunHi - 한국어", language: "ko-KR", gender: "Female" },
];
const MAX_EDGE_CHUNK_BYTES = 3_600;
let voicesCache: TtsVoice[] | null = null;

function splitTextForEdgeTts(text: string): string[] {
  const chunks: string[] = [];
  let current = "";
  const segments = text.match(/[^。！？!?；;\n]+[。！？!?；;\n]?|\n/g) || [text];

  const pushCurrent = () => {
    const value = current.trim();
    if (value) chunks.push(value);
    current = "";
  };

  for (const segment of segments) {
    for (const char of segment) {
      const candidate = current + char;
      if (new TextEncoder().encode(candidate).length > MAX_EDGE_CHUNK_BYTES) {
        pushCurrent();
      }
      current += char;
    }
  }
  pushCurrent();
  return chunks;
}

export async function listEdgeVoices(): Promise<TtsVoice[]> {
  if (voicesCache) return voicesCache;
  try {
    voicesCache = await invoke<TtsVoice[]>("tts_list_voices");
  } catch {
    voicesCache = FALLBACK_EDGE_VOICES;
  }
  return voicesCache;
}

/** 拉取后端累计的 TTS 缓存命中率指标，便于诊断 / 显示给用户。 */
export async function getTtsCacheStats(): Promise<TtsCacheStats> {
  return invoke<TtsCacheStats>("tts_cache_stats");
}

export function clearVoicesCache() {
  voicesCache = null;
}

export class TtsPlayer {
  private systemUtterance: SpeechSynthesisUtterance | null = null;
  private playing = false;
  private activeReject: ((reason?: any) => void) | null = null;
  private playbackToken = 0;

  get isPlaying() {
    return this.playing;
  }

  preparePlayback() {
    // Native Edge playback no longer needs a browser media unlock.
  }

  async speak(text: string, settings: TtsSettings): Promise<void> {
    if (!text.trim()) return;
    if (settings.engine === "system") {
      this.stop();
    } else {
      this.stopLocal();
    }
    if (settings.engine === "system") {
      return this.speakWithSystem(text, settings);
    }

    const token = ++this.playbackToken;
    this.playing = true;
    return new Promise<void>(async (resolve, reject) => {
      this.activeReject = reject;
      try {
        for (const chunk of splitTextForEdgeTts(text)) {
          if (this.playbackToken !== token) return;
          await invoke<void>("tts_play", {
            text: chunk,
            voice: settings.voice,
            rate: settings.rate,
            pitch: settings.pitch,
            volume: settings.volume,
          });
        }
        if (this.playbackToken !== token) return;
        this.playing = false;
        this.activeReject = null;
        resolve();
      } catch (err) {
        if (this.playbackToken !== token) return;
        this.playing = false;
        this.activeReject = null;
        reject(this.toPlaybackError(err, "TTS playback failed"));
      }
    });
  }

  private speakWithSystem(text: string, settings: TtsSettings): Promise<void> {
    if (!this.canUseSystemSpeech()) {
      return Promise.reject(new Error("System TTS is not available"));
    }

    const synth = window.speechSynthesis;
    synth.cancel();

    const utterance = new SpeechSynthesisUtterance(text);
    const voice = this.selectSystemVoice(settings.voice);
    if (voice) utterance.voice = voice;
    utterance.lang = voice?.lang || this.inferLanguage(settings.voice);
    utterance.rate = this.clamp(1 + settings.rate / 100, 0.1, 2);
    utterance.pitch = this.clamp(1 + settings.pitch / 50, 0, 2);
    utterance.volume = this.clamp(settings.volume / 100, 0, 1);

    this.systemUtterance = utterance;
    this.playing = true;

    return new Promise<void>((resolve, reject) => {
      let settled = false;
      this.activeReject = reject;

      const finish = (callback: () => void) => {
        if (settled) return;
        settled = true;
        if (this.systemUtterance === utterance) {
          this.systemUtterance = null;
          this.playing = false;
          this.activeReject = null;
        }
        utterance.onend = null;
        utterance.onerror = null;
        callback();
      };

      utterance.onend = () => finish(resolve);
      utterance.onerror = (event) => {
        finish(() => reject(new Error(event.error || "System TTS playback failed")));
      };

      try {
        synth.speak(utterance);
      } catch (err) {
        finish(() => reject(this.toPlaybackError(err, "System TTS playback failed")));
      }
    });
  }

  private canUseSystemSpeech() {
    return typeof window !== "undefined"
      && typeof window.speechSynthesis !== "undefined"
      && typeof SpeechSynthesisUtterance !== "undefined";
  }

  private selectSystemVoice(voiceId: string) {
    if (!this.canUseSystemSpeech()) return null;
    const voices = window.speechSynthesis.getVoices();
    const normalized = voiceId.toLowerCase();
    const lang = this.inferLanguage(voiceId).toLowerCase();
    const baseLang = lang.split("-")[0];

    return voices.find((voice) => voice.name.toLowerCase() === normalized || voice.voiceURI.toLowerCase() === normalized)
      || voices.find((voice) => voice.lang.toLowerCase() === lang)
      || voices.find((voice) => voice.lang.toLowerCase().startsWith(baseLang))
      || null;
  }

  private inferLanguage(voiceId: string) {
    return voiceId.match(/^[a-z]{2}-[A-Z]{2}/)?.[0] || "zh-CN";
  }

  private toPlaybackError(err: unknown, fallback: string) {
    if (err instanceof DOMException && err.name === "NotAllowedError") {
      return new Error("Playback was blocked. Click preview once and try again.");
    }
    if (err instanceof Error) return err;
    return new Error(String(err || fallback));
  }

  private clamp(value: number, min: number, max: number) {
    return Math.min(max, Math.max(min, value));
  }

  private stopLocal() {
    this.playbackToken += 1;
    if (this.systemUtterance) {
      this.systemUtterance.onend = null;
      this.systemUtterance.onerror = null;
      this.systemUtterance = null;
    }
    if (this.canUseSystemSpeech()) {
      window.speechSynthesis.cancel();
    }
    if (this.activeReject) {
      const reject = this.activeReject;
      this.activeReject = null;
      reject(new Error("Aborted"));
    }
    this.playing = false;
  }

  stop() {
    this.stopLocal();
    invoke("tts_stop").catch(() => {});
  }
}
