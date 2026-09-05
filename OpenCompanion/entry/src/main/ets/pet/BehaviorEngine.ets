import { bus, Ev, BusEvent } from '../commons/EventBus';
import { AppContext, SettingKey } from '../commons/AppContext';

export type PetState = 'idle' | 'waving' | 'happy' | 'sleeping' | 'thinking' | 'speaking' | 'confused' | 'sad';

/**
 * 桌宠行为状态机（对应桌面版 behavior/state.rs + mood.rs：
 * 短时状态自动回落 idle，情绪随事件联动，长时间无人理会进入睡眠）
 */
export class BehaviorEngine {
  static state: PetState = 'idle';
  static happiness: number = 0.6;          // -1 ~ 1
  static energy: number = 90;              // 0 ~ 100
  static moodText: string = '今天也要元气满满哦';

  private static stateUntil: number = 0;
  private static idleSince: number = Date.now();
  private static started: boolean = false;

  /** 状态变化时同步心情文案到设置存储（服务卡片 30 分钟定时刷新时读取） */
  private static persistMood(): void {
    AppContext.setSetting(SettingKey.PET_MOOD, BehaviorEngine.moodText).catch((_e: Error) => {
    });
  }

  static start(): void {
    if (BehaviorEngine.started) {
      return;
    }
    BehaviorEngine.started = true;
    bus.on(Ev.AI_STARTED, (_e: BusEvent) => {
      BehaviorEngine.setState('thinking', 15000);
    });
    bus.on(Ev.AI_DELTA, (_e: BusEvent) => {
      BehaviorEngine.setState('speaking', 1600);
    });
    bus.on(Ev.AI_FINISHED, (_e: BusEvent) => {
      BehaviorEngine.setState('happy', 3000);
      BehaviorEngine.happiness = Math.min(1, BehaviorEngine.happiness + 0.03);
    });
    bus.on(Ev.AI_ERROR, (_e: BusEvent) => {
      BehaviorEngine.setState('confused', 3000);
      BehaviorEngine.happiness = Math.max(-1, BehaviorEngine.happiness - 0.02);
    });
    setInterval(() => {
      BehaviorEngine.tick();
    }, 1000);
  }

  static setState(s: PetState, durationMs: number = 3000): void {
    BehaviorEngine.state = s;
    BehaviorEngine.stateUntil = Date.now() + durationMs;
    if (s === 'idle') {
      BehaviorEngine.idleSince = Date.now();
    }
  }

  static tick(): void {
    const now = Date.now();
    // 短时状态回落
    if (BehaviorEngine.state !== 'idle' && BehaviorEngine.state !== 'sleeping' && now > BehaviorEngine.stateUntil) {
      BehaviorEngine.state = 'idle';
      BehaviorEngine.idleSince = now;
    }
    // 长时间空闲 → 睡觉
    if (BehaviorEngine.state === 'idle' && now - BehaviorEngine.idleSince > 300000) {
      BehaviorEngine.state = 'sleeping';
      BehaviorEngine.moodText = 'zzZ…';
      BehaviorEngine.persistMood();
    }
    // 睡觉回能量
    if (BehaviorEngine.state === 'sleeping') {
      BehaviorEngine.energy = Math.min(100, BehaviorEngine.energy + 0.1);
    } else {
      BehaviorEngine.energy = Math.max(0, BehaviorEngine.energy - 0.02);
      BehaviorEngine.happiness = Math.max(-1, BehaviorEngine.happiness - 0.0005);
    }
  }

  static wake(): void {
    if (BehaviorEngine.state === 'sleeping') {
      BehaviorEngine.setState('waving', 2500);
      BehaviorEngine.moodText = '你回来啦！';
    }
  }

  static pet(): void {
    // 摸摸头
    if (BehaviorEngine.state === 'sleeping') {
      BehaviorEngine.wake();
      return;
    }
    BehaviorEngine.setState('happy', 2500);
    BehaviorEngine.happiness = Math.min(1, BehaviorEngine.happiness + 0.05);
    BehaviorEngine.moodText = BehaviorEngine.pickMoodLine();
  }

  static greet(): void {
    BehaviorEngine.setState('waving', 3000);
    BehaviorEngine.moodText = '嗨～今天过得怎么样？';
  }

  static sleepNow(): void {
    BehaviorEngine.setState('sleeping', 3600000);
    BehaviorEngine.moodText = '小睡一会儿…';
  }

  private static pickMoodLine(): string {
    const lines: string[] = ['被摸头了，好开心！', '嘿嘿～', '最喜欢你啦', '陪你一起加油！'];
    return lines[Math.floor(Math.random() * lines.length)];
  }

  static moodEmoji(): string {
    switch (BehaviorEngine.state) {
      case 'happy':
        return '😊';
      case 'sleeping':
        return '😴';
      case 'thinking':
        return '🤔';
      case 'speaking':
        return '💬';
      case 'waving':
        return '👋';
      case 'confused':
        return '😵‍💫';
      case 'sad':
        return '🥺';
      default:
        return BehaviorEngine.happiness > 0.3 ? '😊' : '🙂';
    }
  }
}
