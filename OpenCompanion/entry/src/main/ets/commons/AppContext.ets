import { common } from '@kit.AbilityKit';
import { preferences } from '@kit.ArkData';
import { Log } from './Log';

/**
 * 全局上下文与轻量 KV 设置（对应桌面版 pet_settings 万能 KV + AppState）
 */
export class AppContext {
  private static ctx: common.UIAbilityContext | null = null;
  private static prefs: preferences.Preferences | null = null;
  private static cache: Map<string, string> = new Map<string, string>();
  static vpWidth: number = 360;
  static vpHeight: number = 780;

  static setContext(c: common.UIAbilityContext): void {
    AppContext.ctx = c;
  }

  static getContext(): common.UIAbilityContext {
    if (!AppContext.ctx) {
      throw new Error('AppContext not initialized');
    }
    return AppContext.ctx;
  }

  static hasContext(): boolean {
    return AppContext.ctx !== null;
  }

  static setViewport(w: number, h: number): void {
    AppContext.vpWidth = w;
    AppContext.vpHeight = h;
  }

  /** 首帧后延迟调用（冷启动优化：首帧前不做任何重初始化） */
  static postFrame(task: () => void, delayMs: number = 30): void {
    setTimeout(() => {
      try {
        task();
      } catch (e) {
        Log.error(`postFrame task failed: ${(e as Error).message}`);
      }
    }, delayMs);
  }

  static async initSettings(): Promise<void> {
    const ctx = AppContext.getContext();
    AppContext.prefs = await preferences.getPreferences(ctx, { name: 'companion_settings' });
    Log.info('settings ready');
  }

  static getSetting(key: string, def: string): string {
    const c = AppContext.cache.get(key);
    if (c !== undefined) {
      return c;
    }
    if (AppContext.prefs !== null) {
      try {
        const v = AppContext.prefs.getSync(key, def);
        const s = typeof v === 'string' ? v as string : JSON.stringify(v);
        AppContext.cache.set(key, s);
        return s;
      } catch (e) {
        // fallthrough
      }
    }
    return def;
  }

  static getBool(key: string, def: boolean): boolean {
    const v = AppContext.cache.get(key);
    if (v !== undefined) {
      return v === 'true';
    }
    // 未命中缓存时回源 prefs（getSetting 会顺带填 cache），否则重启后所有布尔设置失效
    return AppContext.getSetting(key, def ? 'true' : 'false') === 'true';
  }

  static getNumber(key: string, def: number): number {
    const v = AppContext.cache.get(key);
    if (v === undefined) {
      return Number(AppContext.getSetting(key, String(def)));
    }
    const n = Number(v);
    return Number.isNaN(n) ? def : n;
  }

  static async setSetting(key: string, value: string): Promise<void> {
    AppContext.cache.set(key, value);
    if (AppContext.prefs) {
      await AppContext.prefs.put(key, value);
      await AppContext.prefs.flush();
    }
  }

  static async setBool(key: string, value: boolean): Promise<void> {
    await AppContext.setSetting(key, value ? 'true' : 'false');
  }
}

/** 常用设置键 */
export class SettingKey {
  static readonly API_PROFILES: string = 'api_profiles';
  static readonly ACTIVE_PROFILE: string = 'active_profile';
  static readonly VOICE_AUTO_SPEAK: string = 'voice_auto_speak';
  static readonly VOICE_AUTO_SEND: string = 'voice_auto_send';
  static readonly TOOL_CONFIRM: string = 'tool_confirm_enabled';
  static readonly KB_OFFLINE: string = 'kb_offline_mode';
  static readonly EVOLUTION_AUTO: string = 'evolution_auto_enabled';
  static readonly HOT_WORDS: string = 'asr_hot_words';
  static readonly WEATHER_CITY: string = 'weather_city';
  static readonly PET_MOOD: string = 'pet_mood_text';
  static readonly ACTIVE_SKILL: string = 'active_skill_id';
  static readonly PERSONALITY: string = 'personality';
  static readonly PROXY_HOST: string = 'proxy_host';
  static readonly PROXY_PORT: string = 'proxy_port';
}
