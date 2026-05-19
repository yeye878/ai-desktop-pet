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

let voicesCache: TtsVoice[] | null = null;

export async function listEdgeVoices(): Promise<TtsVoice[]> {
  if (voicesCache) return voicesCache;
  voicesCache = await invoke<TtsVoice[]>("tts_list_voices");
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
