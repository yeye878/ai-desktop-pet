import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { window } from '@kit.ArkUI';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { AppContext } from '../commons/AppContext';
import { DistributedSync } from '../commons/DistributedSync';
import { PerfTrace } from '../commons/PerfTrace';
import { bus, Ev } from '../commons/EventBus';
import { ChatService } from '../core/ChatService';
import { ChatMessage } from '../core/Types';

/**
 * 入口 Ability。
 * 冷启动纪律：onCreate/onWindowStageCreate 只做窗口与首帧加载，
 * 数据库/语音引擎/知识库等全部延迟到首帧后（目标冷启动 ≤3s 的关键实践）。
 * continuable=true：支持系统级跨设备任务接续（onContinue），并携带真实会话上下文。
 */
export default class EntryAbility extends UIAbility {
  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    AppContext.setContext(this.context);
    PerfTrace.mark('ability_created');
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
          bus.emit(Ev.NAVIGATE, { 'page': 'chat', 'draft': draft as string } as Record<string, string>);
        }, 500);
      }
      // 跨设备接续携带的真实会话上下文：落地到对话页恢复
      const context = params['context'];
      if (typeof context === 'string' && (context as string).length > 0) {
        AppContext.postFrame(() => {
          bus.emit(Ev.NAVIGATE, { 'page': 'chat', 'continuation': context as string } as Record<string, string>);
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
      PerfTrace.mark('first_frame');
      PerfTrace.measure('冷启动·首帧', 'ability_created', 'first_frame');
      try {
        const win = windowStage.getMainWindowSync();
        win.on('windowSizeChange', (size: window.Size) => {
          // size/windowRect 单位为 px，vpWidth/vpWidth 需要 vp（Index 的 720vp 断点依赖它）
          AppContext.setViewport(px2vp(size.width), px2vp(size.height));
          bus.emit(Ev.VP_WIDTH);
        });
        const rect = win.getWindowProperties().windowRect;
        AppContext.setViewport(px2vp(rect.width), px2vp(rect.height));
      } catch (e) {
        hilog.warn(0x0C0C, 'OpenCompanion', 'window size init failed');
      }
      hilog.info(0x0C0C, 'OpenCompanion', 'first frame loaded');
    });
  }

  /** 最近对话的接续摘要（手机→平板「接力」时随 Want 携带 + 发布到分布式 KV） */
  private static buildContextSummary(): string {
    try {
      const recent = ChatService.messages.slice(-6);
      const lines: string[] = [];
      recent.forEach((m: ChatMessage) => {
        if (m.content.trim().length === 0) {
          return;
        }
        const who = m.role === 'user' ? '用户' : '小伴';
        let text = m.content.replace(/\n/g, ' ');
        if (text.length > 60) {
          text = text.substring(0, 60) + '…';
        }
        lines.push(`${who}: ${text}`);
      });
      return lines.join('\n');
    } catch (e) {
      return '';
    }
  }

  /**
   * 跨设备任务接续（命题 F3）：手机上问一半 → 平板「接力」继续。
   * 携带真实会话上下文（最近 6 条对话摘要），并同时发布到分布式 KV 供对端拉取。
   */
  onContinue(wantParam: Record<string, Object>): AbilityConstant.OnContinueResult {
    const context = EntryAbility.buildContextSummary();
    wantParam['oc'] = 'opencompanion_session';
    wantParam['page'] = 'chat';
    wantParam['mood'] = `${Math.round((Date.now() % 100))}`;
    if (context.length > 0) {
      wantParam['context'] = context;
    }
    wantParam['draft'] = context.length > 0
      ? '（接续自另一台设备，请基于上面的上下文继续话题）'
      : `接续时间 ${new Date().toLocaleString()}，请从上次话题继续`;
    DistributedSync.publishContext(context);
    return AbilityConstant.OnContinueResult.AGREE;
  }

  onWindowStageDestroy(): void {
  }

  onForeground(): void {
  }

  onBackground(): void {
    // 退后台时把最新会话摘要同步到分布式 KV（恢复后任何一端可读取）
    DistributedSync.publishContext(EntryAbility.buildContextSummary());
  }
}
