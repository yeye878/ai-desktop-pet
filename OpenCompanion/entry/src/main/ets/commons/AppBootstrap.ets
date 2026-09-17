import { AppContext } from './AppContext';
import { RdbHelper } from './RdbHelper';
import { PerfTrace } from './PerfTrace';
import { DistributedSync } from './DistributedSync';
import { Log } from './Log';
import { BehaviorEngine } from '../pet/BehaviorEngine';
import { ChatService } from '../core/ChatService';
import { AsrController } from '../voice/AsrController';
import { TtsController } from '../voice/TtsController';
import { RagBridge } from '../knowledge/RagBridge';

/**
 * 冷启动编排：主链路的初始化顺序集中在此定义，UI 外壳只负责把就绪状态映射到界面。
 * 从 Index.ets 抽出，使启动顺序（尤其 RdbHelper 必须最先）成为可独立阅读的契约。
 */
export class AppBootstrap {
  /**
   * 执行冷启动初始化链（首帧后延迟触发）。
   * 失败按降级处理：记录错误后仍然 resolve，调用方在 resolve 后进入可交互态即可。
   */
  static init(delayMs: number = 60): Promise<void> {
    return new Promise<void>((resolve: () => void) => {
      AppContext.postFrame(() => {
        AppBootstrap.runChain()
          .then(() => {
            resolve();
          })
          .catch((e: Error) => {
            Log.error(`init chain failed: ${e.message}`);
            resolve();
          });
      }, delayMs);
    });
  }

  /**
   * 初始化链本体：顺序固定。
   * RdbHelper 必须最先——ChatService.init → AgentRepo/loadHistory 直接查主库，
   * 缺了这一步整条链静默失败（发消息不显示、页面刷新 unhandled rejection）。
   */
  private static async runChain(): Promise<void> {
    await RdbHelper.init();
    await AppContext.initSettings();
    await ChatService.init();
    BehaviorEngine.start();
    AsrController.preheat();
    TtsController.preheat();
    PerfTrace.mark('interactive');
    PerfTrace.measure('冷启动·可交互', 'ability_created', 'interactive');
    // idle 阶段预热：RAG 会话池 + 分布式同步（均有静默降级，不影响主链路）
    AppContext.postFrame(() => {
      RagBridge.preheat();
      DistributedSync.ensure();
    }, 600);
  }
}
