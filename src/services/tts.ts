import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";

export interface TtsVoice {
  id: string;
  name: string;
  language: string;
  gender: string;
}

export interface TtsResult {
  audio_path: string;
  cached: boolean;
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

let voicesCache: TtsVoice[] | null = null;

export async function listEdgeVoices(): Promise<TtsVoice[]> {
  if (voicesCache) return voicesCache;
  try {
    voicesCache = await invoke<TtsVoice[]>("tts_list_voices");
  } catch {
    voicesCache = FALLBACK_EDGE_VOICES;
  }
  return voicesCache;
}

export function clearVoicesCache() {
  voicesCache = null;
}

export class TtsPlayer {
  private audio: HTMLAudioElement | null = null;
  private playing = false;
  private activeReject: ((reason?: any) => void) | null = null;

  get isPlaying() {
    return this.playing;
  }

  async speak(text: string, settings: TtsSettings): Promise<void> {
    if (!text.trim()) return;
    this.stop();

    const result = await invoke<TtsResult>("tts_synthesize", {
      text,
      voice: settings.voice,
      rate: settings.rate,
      pitch: settings.pitch,
      volume: settings.volume,
    });

    const audioUrl = convertFileSrc(result.audio_path);
    this.audio = new Audio(audioUrl);
    this.audio.volume = settings.volume / 100;
    this.playing = true;

    return new Promise<void>((resolve, reject) => {
      if (!this.audio) return resolve();
      this.activeReject = reject;

      this.audio.onended = () => {
        this.playing = false;
        this.activeReject = null;
        resolve();
      };
      this.audio.onerror = () => {
        this.playing = false;
        this.activeReject = null;
        reject(new Error("音频播放失败"));
      };
      this.audio.play().catch((e) => {
        this.playing = false;
        this.activeReject = null;
        reject(e);
      });
    });
  }

  stop() {
    if (this.audio) {
      this.audio.onended = null;
      this.audio.onerror = null;
      this.audio.pause();
      this.audio.src = "";
      this.audio = null;
    }
    if (this.activeReject) {
      const reject = this.activeReject;
      this.activeReject = null;
      reject(new Error("Aborted"));
    }
    this.playing = false;
  }
}
