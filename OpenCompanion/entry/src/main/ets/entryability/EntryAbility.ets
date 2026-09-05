import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { window } from '@kit.ArkUI';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { AppContext } from '../commons/AppContext';
import { bus, Ev } from '../commons/EventBus';

/**
 * 入口 Ability。
 * 冷启动纪律：onCreate/onWindowStageCreate 只做窗口与首帧加载，
 * 数据库/语音引擎/知识库等全部延迟到首帧后（目标冷启动 ≤3s 的关键实践）。
 * continuable=true：支持系统级跨设备任务接续（onContinue）。
 */
export default class EntryAbility extends UIAbility {
  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    AppContext.setContext(this.context);
    EntryAbility.handleWant(want);
    hilog.info(0x0C0C, 'OpenCompanion', 'ability created (cold start path)');
  }

  onNewWant(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    EntryAbility.handleWant(want);
  }

  private static handleWant(want: Want): void {
    // 服务卡片 / 跨设备接续的目标页跳转
    const params = want.parameters;
    if (params !== undefined && params !== null) {
      const page = params['page'];
      if (typeof page === 'string' && (page as string).length > 0) {
        AppContext.postFrame(() => {
          bus.emit(Ev.NAVIGATE, { 'page': page as string } as Record<string, string>);
        }, 500);
      }
      const draft = params['draft'];
      if (typeof draft === 'string' && (draft as string).length > 0) {
        AppContext.postFrame(() => {
          bus.emit(Ev.NAVIGATE, { 'page': 'chat' } as Record<string, string>);
        }, 500);
      }
    }
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index', (err) => {
      if (err.code) {
        hilog.error(0x0C0C, 'OpenCompanion', 'loadContent failed: %{public}s', JSON.stringify(err));
        return;
      }
      try {
        const win = windowStage.getMainWindowSync();
        win.on('windowSizeChange', (size: window.Size) => {
          AppContext.setViewport(size.width, size.height);
          bus.emit(Ev.VP_WIDTH);
        });
        win.getWindowProperties();
        AppContext.setViewport(win.getWindowProperties().windowRect.width, win.getWindowProperties().windowRect.height);
      } catch (e) {
        hilog.warn(0x0C0C, 'OpenCompanion', 'window size init failed');
      }
      hilog.info(0x0C0C, 'OpenCompanion', 'first frame loaded');
    });
  }

  /**
   * 跨设备任务接续（命题 F3）：手机上问一半 → 平板「接力」继续。
   * 系统流转时携带会话上下文。
   */
  onContinue(wantParam: Record<string, Object>): AbilityConstant.OnContinueResult {
    wantParam['oc'] = 'opencompanion_session';
    wantParam['page'] = 'chat';
    wantParam['draft'] = `接续时间 ${new Date().toLocaleString()}，请从上次话题继续`;
    wantParam['mood'] = `${Math.round((Date.now() % 100))}`;
    return AbilityConstant.OnContinueResult.AGREE;
  }

  onWindowStageDestroy(): void {
  }

  onForeground(): void {
  }

  onBackground(): void {
  }
}
