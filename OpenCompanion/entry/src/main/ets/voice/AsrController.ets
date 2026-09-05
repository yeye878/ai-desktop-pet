import { speechRecognizer } from '@kit.CoreSpeechKit';
import { abilityAccessCtrl } from '@kit.AbilityKit';
import { common } from '@kit.AbilityKit';
import { AppContext } from '../commons/AppContext';
import { bus, Ev, BusEvent } from '../commons/EventBus';
import { Log } from '../commons/Log';
import { IntentEngine } from '../core/IntentEngine';

/**
 * 语音识别控制器（@kit.CoreSpeechKit 端侧离线识别，替换桌面版 WebView SpeechRecognition）。
 * - online:1 离线模式，断网可用，隐私友好
 * - sysGeneralLexicon 动态热词表（用户设置 ∪ 智能体名 ∪ 知识库文档标题）提升指令识别精度（命题 R5）
 * - 应用启动即预创建引擎（语音响应 ≤2s 的关键路径）
 * - 知识库变更后自动重建引擎刷新热词（聆听中不重建）
 */
export class AsrController {
  private static engine: speechRecognizer.SpeechRecognitionEngine | null = null;
  private static creating: boolean = false;
  private static sessionSeq: number = 0;
  private static kbHooked: boolean = false;
  static listening: boolean = false;
  static lastPartial: string = '';
  private static finalSentences: string = '';

  /** 兼容保留：设置页展示/编辑的基础热词 */
  static hotWords(): string[] {
    const raw = AppContext.getSetting('asr_hot_words', '打开知识库,新对话,今天日程,帮我总结,存入知识库,组队,学习助手,日程管家,文档处理');
    const out: string[] = [];
    raw.split(/[,，]/).forEach((w: string) => {
      const t = w.trim();
      if (t.length >= 2 && t.length <= 20 && out.length < 200) {
        out.push(t);
      }
    });
    return out;
  }

  /** 预创建引擎（冷启动后 idle 阶段调用）；force=true 时用最新热词重建 */
  static async preheat(force: boolean = false): Promise<boolean> {
    if (AsrController.engine !== null && !force) {
      return true;
    }
    if (AsrController.listening) {
      return AsrController.engine !== null; // 识别进行中不重建，避免截断
    }
    if (AsrController.creating) {
      return false;
    }
    AsrController.creating = true;
    try {
      if (AsrController.engine !== null) {
        try {
          AsrController.engine.shutdown();
        } catch (e) {
          // ignore
        }
        AsrController.engine = null;
      }
      // 动态热词：设置热词 ∪ 指令词表 ∪ 智能体名 ∪ 知识库文档标题
      let hotWords: string[] = AsrController.hotWords();
      try {
        hotWords = await IntentEngine.dynamicHotwords();
      } catch (e) {
        // 动态表失败时退回静态设置
      }
      const params: speechRecognizer.CreateEngineParams = {
        language: 'zh-CN',
        online: 1,
        extraParams: {
          'locate': 'CN',
          'recognizerMode': 'short',
          'sysGeneralLexicon': hotWords
        } as Record<string, Object>
      };
      AsrController.engine = await speechRecognizer.createEngine(params);
      AsrController.attachListener();
      AsrController.hookKbRefresh();
      Log.info(`asr engine ready (offline + ${hotWords.length} dynamic hotwords)`);
      return true;
    } catch (e) {
      const err = e as BusinessError;
      Log.error(`asr createEngine failed ${err.code}: ${err.message}`);
      AsrController.engine = null;
      return false;
    } finally {
      AsrController.creating = false;
    }
  }

  /** 知识库变更 → 延迟重建引擎刷新热词（一次即可） */
  private static hookKbRefresh(): void {
    if (AsrController.kbHooked) {
      return;
    }
    AsrController.kbHooked = true;
    bus.on(Ev.KB_CHANGED, (_e: BusEvent) => {
      setTimeout(() => {
        AsrController.preheat(true).then((_ok: boolean) => {
        }).catch((_e: Error) => {
        });
      }, 3000);
    });
  }

  private static attachListener(): void {
    const engine = AsrController.engine;
    if (!engine) {
      return;
    }
    engine.setListener({
      onStart: (sessionId: string, eventMessage: string) => {
        Log.info(`asr start: ${eventMessage}`);
        bus.emit(Ev.VOICE_STATE, { 'state': 'listening' } as Record<string, string>);
      },
      onEvent: (sessionId: string, eventCode: number, eventMessage: string) => {
        if (eventCode === 3) {
          // 语音段结束（vadEnd）
        }
      },
      onResult: (sessionId: string, result: speechRecognizer.SpeechRecognitionResult) => {
        AsrController.lastPartial = result.result;
        // 累积已确定的语音段：finish() 后 stop() 才能拿到完整多段文本（否则只余最后一段）
        if (result.isFinal) {
          AsrController.finalSentences += result.result;
          AsrController.lastPartial = '';
        }
        bus.emit(Ev.VOICE_PARTIAL, {
          'text': result.result,
          'isFinal': `${result.isFinal}`,
          'isLast': `${result.isLast}`
        } as Record<string, string>);
      },
      onComplete: (sessionId: string, eventMessage: string) => {
        AsrController.listening = false;
        Log.info(`asr complete: ${eventMessage}`);
        bus.emit(Ev.VOICE_STATE, { 'state': 'idle' } as Record<string, string>);
      },
      onError: (sessionId: string, errorCode: number, errorMessage: string) => {
        AsrController.listening = false;
        Log.error(`asr error ${errorCode}: ${errorMessage}`);
        bus.emit(Ev.VOICE_STATE, { 'state': 'error', 'message': `${errorCode}: ${errorMessage}` } as Record<string, string>);
      }
    });
  }

  static async ensureMicPermission(): Promise<boolean> {
    try {
      const ctx: common.UIAbilityContext = AppContext.getContext();
      const atManager = abilityAccessCtrl.createAtManager();
      const result = await atManager.requestPermissionsFromUser(ctx, ['ohos.permission.MICROPHONE']);
      let granted = false;
      result.authResults.forEach((r: number) => {
        if (r === 0) {
          granted = true;
        }
      });
      return granted;
    } catch (e) {
      Log.error(`mic permission failed: ${(e as Error).message}`);
      return false;
    }
  }

  /** 开始聆听。返回是否成功启动。 */
  static async start(): Promise<boolean> {
    const granted = await AsrController.ensureMicPermission();
    if (!granted) {
      bus.emit(Ev.VOICE_STATE, { 'state': 'error', 'message': '未获得麦克风权限' } as Record<string, string>);
      return false;
    }
    if (!await AsrController.preheat()) {
      bus.emit(Ev.VOICE_STATE, { 'state': 'error', 'message': '语音引擎不可用' } as Record<string, string>);
      return false;
    }
    if (AsrController.listening) {
      return true;
    }
    AsrController.lastPartial = '';
    AsrController.finalSentences = '';
    const engine = AsrController.engine;
    if (!engine) {
      return false;
    }
    if (engine.isBusy()) {
      engine.cancel(`s${AsrController.sessionSeq}`);
    }
    AsrController.sessionSeq++;
    const sessionId = `s${AsrController.sessionSeq}`;
    const startParams: speechRecognizer.StartParams = {
      sessionId: sessionId,
      audioInfo: {
        audioType: 'pcm',
        sampleRate: 16000,
        soundChannel: 1,
        sampleBit: 16
      }
    };
    try {
      engine.startListening(startParams);
      AsrController.listening = true;
      return true;
    } catch (e) {
      Log.error(`asr start failed: ${(e as Error).message}`);
      return false;
    }
  }

  /** 结束本次识别 */
  static stop(): string {
    const engine = AsrController.engine;
    if (engine && AsrController.listening) {
      try {
        engine.finish(`s${AsrController.sessionSeq}`);
      } catch (e) {
        // ignore
      }
      AsrController.listening = false;
    }
    return AsrController.finalSentences.length > 0 ? AsrController.finalSentences : AsrController.lastPartial;
  }

  static cancel(): void {
    const engine = AsrController.engine;
    if (engine) {
      try {
        engine.cancel(`s${AsrController.sessionSeq}`);
      } catch (e) {
        // ignore
      }
    }
    AsrController.listening = false;
  }
}
