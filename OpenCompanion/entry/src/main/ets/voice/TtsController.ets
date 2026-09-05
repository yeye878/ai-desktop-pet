import { textToSpeech } from '@kit.CoreSpeechKit';
import { Log } from '../commons/Log';

/**
 * 语音合成控制器（@kit.CoreSpeechKit 端侧 TTS，替换桌面版 Edge TTS WSS——移动端合规且零外网依赖）。
 * 长文本按句切块依序播报，降低首响时延。
 */
export class TtsController {
  private static engine: textToSpeech.TextToSpeechEngine | null = null;
  private static creating: boolean = false;
  private static queue: string[] = [];
  private static speaking: boolean = false;
  private static seq: number = 0;

  static async preheat(): Promise<void> {
    if (TtsController.engine !== null || TtsController.creating) {
      return;
    }
    TtsController.creating = true;
    try {
      const params: textToSpeech.CreateEngineParams = {
        language: 'zh-CN',
        person: 0,
        online: 1
      };
      TtsController.engine = await textToSpeech.createEngine(params);
      TtsController.engine.setListener({
        onStart: (_requestId: string, _response: textToSpeech.StartResponse) => {
          TtsController.speaking = true;
        },
        onComplete: (_requestId: string, _response: textToSpeech.CompleteResponse) => {
          TtsController.speaking = false;
          TtsController.pump();
        },
        onStop: (_requestId: string, _response: textToSpeech.StopResponse) => {
          TtsController.speaking = false;
        },
        onError: (_requestId: string, errorCode: number, errorMessage: string) => {
          Log.error(`tts error ${errorCode}: ${errorMessage}`);
          TtsController.speaking = false;
          TtsController.queue = [];
        }
      });
      Log.info('tts engine ready');
    } catch (e) {
      const err = e as BusinessError;
      Log.error(`tts createEngine failed ${err.code}: ${err.message}`);
      TtsController.engine = null;
    } finally {
      TtsController.creating = false;
    }
  }

  /** 播报文本（自动分句排队） */
  static speak(text: string): void {
    const clean = text.replace(/[#*`>\[\]]/g, '').trim();
    if (clean.length === 0) {
      return;
    }
    TtsController.queue = TtsController.queue.concat(TtsController.splitSentences(clean));
    TtsController.pump();
  }

  static stop(): void {
    TtsController.queue = [];
    if (TtsController.engine && TtsController.speaking) {
      try {
        TtsController.engine.stop();
      } catch (e) {
        // ignore
      }
    }
    TtsController.speaking = false;
  }

  static isSpeaking(): boolean {
    return TtsController.speaking || TtsController.queue.length > 0;
  }

  private static pump(): void {
    if (TtsController.speaking || TtsController.queue.length === 0) {
      return;
    }
    const engine = TtsController.engine;
    if (!engine) {
      TtsController.preheat().then(() => {
        if (TtsController.engine) {
          TtsController.pump();
        }
      });
      return;
    }
    const next = TtsController.queue.shift();
    if (next === undefined) {
      return;
    }
    TtsController.seq++;
    const speakParams: textToSpeech.SpeakParams = {
      requestId: `tts_${TtsController.seq}`
    };
    try {
      TtsController.speaking = true;
      engine.speak(next, speakParams);
    } catch (e) {
      Log.error(`tts speak failed: ${(e as Error).message}`);
      TtsController.speaking = false;
    }
  }

  private static splitSentences(text: string): string[] {
    const out: string[] = [];
    let buf = '';
    for (let i = 0; i < text.length; i++) {
      const ch = text.charAt(i);
      buf += ch;
      if ((ch === '。' || ch === '！' || ch === '？' || ch === '；' || ch === '\n') && buf.length >= 12) {
        out.push(buf.trim());
        buf = '';
      } else if (buf.length >= 100) {
        out.push(buf.trim());
        buf = '';
      }
    }
    if (buf.trim().length > 0) {
      out.push(buf.trim());
    }
    return out.filter((s: string) => s.length > 0);
  }
}
