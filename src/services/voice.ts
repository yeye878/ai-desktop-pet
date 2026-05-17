export type VoiceStatus = "idle" | "listening" | "speaking" | "error";

export interface VoiceSettings {
  enabled: boolean;
  autoSend: boolean;
  autoSpeak: boolean;
  shortcut: string;
  language: string;
  rate: number;
  pitch: number;
  volume: number;
  voiceName: string;
}

export const DEFAULT_VOICE_SETTINGS: VoiceSettings = {
  enabled: true,
  autoSend: true,
  autoSpeak: true,
  shortcut: "Ctrl+Alt+V",
  language: "zh-CN",
  rate: 1,
  pitch: 1,
  volume: 1,
  voiceName: "",
};

type SpeechRecognitionCtor = new () => SpeechRecognitionLike;

interface SpeechRecognitionResultLike {
  isFinal: boolean;
  [index: number]: { transcript: string };
}

interface SpeechRecognitionEventLike extends Event {
  resultIndex: number;
  results: {
    length: number;
    [index: number]: SpeechRecognitionResultLike;
  };
}

interface SpeechRecognitionErrorEventLike extends Event {
  error?: string;
  message?: string;
}

interface SpeechRecognitionLike extends EventTarget {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  maxAlternatives: number;
  onstart: ((event: Event) => void) | null;
  onend: ((event: Event) => void) | null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null;
  onresult: ((event: SpeechRecognitionEventLike) => void) | null;
  start: () => void;
  stop: () => void;
  abort: () => void;
}

declare global {
  interface Window {
    SpeechRecognition?: SpeechRecognitionCtor;
    webkitSpeechRecognition?: SpeechRecognitionCtor;
  }
}

export function isSpeechRecognitionSupported() {
  return Boolean(window.SpeechRecognition || window.webkitSpeechRecognition);
}

export function isSpeechSynthesisSupported() {
  return typeof window.speechSynthesis !== "undefined" && typeof SpeechSynthesisUtterance !== "undefined";
}

export function normalizeVoiceSettings(value: Partial<VoiceSettings> | null | undefined): VoiceSettings {
  return {
    ...DEFAULT_VOICE_SETTINGS,
    ...(value || {}),
    rate: clampNumber(value?.rate, 0.6, 1.5, DEFAULT_VOICE_SETTINGS.rate),
    pitch: clampNumber(value?.pitch, 0.6, 1.6, DEFAULT_VOICE_SETTINGS.pitch),
    volume: clampNumber(value?.volume, 0, 1, DEFAULT_VOICE_SETTINGS.volume),
  };
}

export function parseVoiceSettings(raw: string): VoiceSettings {
  if (!raw) return { ...DEFAULT_VOICE_SETTINGS };

  try {
    return normalizeVoiceSettings(JSON.parse(raw) as Partial<VoiceSettings>);
  } catch {
    return { ...DEFAULT_VOICE_SETTINGS };
  }
}

export function serializeVoiceSettings(settings: VoiceSettings) {
  return JSON.stringify(normalizeVoiceSettings(settings));
}

export function getAvailableVoices(): Promise<SpeechSynthesisVoice[]> {
  if (!isSpeechSynthesisSupported()) return Promise.resolve([]);

  const voices = window.speechSynthesis.getVoices();
  if (voices.length > 0) return Promise.resolve(voices);

  return new Promise((resolve) => {
    const timeout = window.setTimeout(() => {
      window.speechSynthesis.onvoiceschanged = null;
      resolve(window.speechSynthesis.getVoices());
    }, 600);

    window.speechSynthesis.onvoiceschanged = () => {
      window.clearTimeout(timeout);
      window.speechSynthesis.onvoiceschanged = null;
      resolve(window.speechSynthesis.getVoices());
    };
  });
}

export class VoiceController {
  private recognition: SpeechRecognitionLike | null = null;

  async listen(
    settings: VoiceSettings,
    onInterim?: (text: string) => void,
  ): Promise<string> {
    const Recognition = window.SpeechRecognition || window.webkitSpeechRecognition;
    if (!Recognition) {
      throw new Error("当前 WebView 不支持语音识别");
    }

    this.abortListening();

    return new Promise((resolve, reject) => {
      let finalText = "";
      let interimText = "";
      let settled = false;
      const recognition = new Recognition();
      this.recognition = recognition;

      const finish = (error?: Error) => {
        if (settled) return;
        settled = true;
        this.recognition = null;
        if (error) {
          reject(error);
        } else {
          resolve(finalText.trim() || interimText.trim());
        }
      };

      recognition.lang = settings.language || DEFAULT_VOICE_SETTINGS.language;
      recognition.continuous = false;
      recognition.interimResults = true;
      recognition.maxAlternatives = 1;
      recognition.onresult = (event) => {
        interimText = "";
        for (let index = event.resultIndex; index < event.results.length; index += 1) {
          const result = event.results[index];
          const transcript = result[0]?.transcript || "";
          if (result.isFinal) {
            finalText += transcript;
          } else {
            interimText += transcript;
          }
        }
        onInterim?.((finalText + interimText).trim());
      };
      recognition.onerror = (event) => {
        const message = formatSpeechError(event.error, event.message);
        try {
          recognition.abort();
        } catch {
          // Ignore abort failures after a recognition error.
        }
        finish(new Error(message));
      };
      recognition.onend = () => finish();

      try {
        recognition.start();
      } catch (error) {
        finish(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  stopListening() {
    try {
      this.recognition?.stop();
    } catch {
      this.recognition = null;
    }
  }

  abortListening() {
    try {
      this.recognition?.abort();
    } catch {
      // Ignore abort failures; the old recognition instance is no longer useful.
    } finally {
      this.recognition = null;
    }
  }

  async speak(text: string, settings: VoiceSettings) {
    if (!settings.autoSpeak || !text.trim()) return;
    if (!isSpeechSynthesisSupported()) {
      throw new Error("当前 WebView 不支持语音播报");
    }

    window.speechSynthesis.cancel();
    const utterance = new SpeechSynthesisUtterance(text);
    utterance.lang = settings.language || DEFAULT_VOICE_SETTINGS.language;
    utterance.rate = settings.rate;
    utterance.pitch = settings.pitch;
    utterance.volume = settings.volume;

    const voices = await getAvailableVoices();
    const selectedVoice = settings.voiceName
      ? voices.find((voice) => voice.name === settings.voiceName)
      : voices.find((voice) => voice.lang.toLowerCase().startsWith(settings.language.toLowerCase()));
    if (selectedVoice) {
      utterance.voice = selectedVoice;
    }

    await new Promise<void>((resolve, reject) => {
      utterance.onend = () => resolve();
      utterance.onerror = () => reject(new Error("语音播报失败"));
      window.speechSynthesis.speak(utterance);
    });
  }

  stopSpeaking() {
    if (isSpeechSynthesisSupported()) {
      window.speechSynthesis.cancel();
    }
  }
}

function clampNumber(value: unknown, min: number, max: number, fallback: number) {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, value));
}

function formatSpeechError(error?: string, message?: string) {
  switch (error) {
    case "not-allowed":
    case "service-not-allowed":
      return "麦克风权限被拒绝，请在系统或 WebView 权限里允许麦克风访问";
    case "no-speech":
      return "没有听到有效语音";
    case "audio-capture":
      return "没有检测到可用麦克风";
    case "network":
      return "语音识别服务暂时不可用";
    case "aborted":
      return "语音识别已取消";
    default:
      return message || "语音识别失败";
  }
}
