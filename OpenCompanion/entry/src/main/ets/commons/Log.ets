import { hilog } from '@kit.PerformanceAnalysisKit';

/**
 * 统一日志封装（对应桌面版 tracing/println 日志习惯）
 */
export class Log {
  static readonly DOMAIN: number = 0x0C0C;
  static readonly TAG: string = 'OpenCompanion';

  static info(msg: string): void {
    hilog.info(Log.DOMAIN, Log.TAG, '%{public}s', msg);
  }

  static warn(msg: string): void {
    hilog.warn(Log.DOMAIN, Log.TAG, '%{public}s', msg);
  }

  static error(msg: string): void {
    hilog.error(Log.DOMAIN, Log.TAG, '%{public}s', msg);
  }

  static debug(msg: string): void {
    hilog.debug(Log.DOMAIN, Log.TAG, '%{public}s', msg);
  }
}
