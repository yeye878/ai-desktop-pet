import { distributedKVStore } from '@kit.ArkData';
import { AppContext } from './AppContext';
import { Log } from './Log';

/**
 * 跨设备上下文同步（命题 F3 跨设备协同，对应方案 4.3"上下文同步"）：
 * 以 distributedKVStore 单版本 KV 在可信设备圈同步「最新会话摘要 + 桌宠状态」，
 * 任何一端接续/启动时都能取到其它设备的最新上下文。
 *
 * 防御性设计（与 RagBridge 同款纪律）：
 * - 能力检测：模块/能力不存在时置 unavailable，功能静默降级为单端完整可用；
 * - 全程 try/catch，任何异常不阻断启动与主链路；
 * - 单版本 KV 只存小体量摘要（<4KB），不做全量历史同步。
 */
export class DistributedSync {
  private static kv: distributedKVStore.SingleKVStore | null = null;
  private static state: string = 'uninit'; // uninit | ready | unavailable
  private static initializing: Promise<boolean> | null = null;
  static readonly KEY_CONTEXT: string = 'oc_latest_context';
  static readonly KEY_MEMORY: string = 'oc_memory_digest';

  /** 分布式同步是否可用（初始化成功） */
  static isReady(): boolean {
    return DistributedSync.state === 'ready' && DistributedSync.kv !== null;
  }

  static statusText(): string {
    if (DistributedSync.state === 'ready') {
      return '分布式数据同步已就绪，可信设备圈自动同步会话摘要';
    }
    if (DistributedSync.state === 'unavailable') {
      return '分布式同步暂不可用，单端完整可用；登录同一华为账号后自动启用';
    }
    return '分布式同步待初始化';
  }

  /** 惰性初始化（首帧后 idle 调用；失败置 unavailable 不重试） */
  static async ensure(): Promise<boolean> {
    if (DistributedSync.state === 'ready') {
      return true;
    }
    if (DistributedSync.state === 'unavailable') {
      return false;
    }
    if (DistributedSync.initializing !== null) {
      return DistributedSync.initializing;
    }
    DistributedSync.initializing = (async (): Promise<boolean> => {
      try {
        if (typeof distributedKVStore.createKVManager !== 'function') {
          throw new Error('distributedKVStore module not available on this device');
        }
        const ctx = AppContext.getContext();
        const config: distributedKVStore.KVManagerConfig = {
          context: ctx,
          bundleName: 'com.opencompanion.pet'
        };
        const manager = await distributedKVStore.createKVManager(config);
        const options: distributedKVStore.Options = {
          createIfMissing: true,
          encrypt: false,
          backup: false,
          autoSync: true,
          kvStoreType: distributedKVStore.KVStoreType.SINGLE_VERSION,
          securityLevel: distributedKVStore.SecurityLevel.S1
        };
        DistributedSync.kv = await manager.getKVStore<distributedKVStore.SingleKVStore>('oc_sync', options);
        DistributedSync.state = 'ready';
        Log.info('distributed kv ready');
        return true;
      } catch (e) {
        DistributedSync.state = 'unavailable';
        Log.warn(`distributed kv unavailable (single-device mode): ${(e as Error).message}`);
        return false;
      }
    })();
    return DistributedSync.initializing;
  }

  /** 发布最新会话摘要（接续/退后台时调用；fire-and-forget） */
  static publishContext(summary: string): void {
    DistributedSync.ensure().then((ok: boolean) => {
      if (!ok || DistributedSync.kv === null) {
        return;
      }
      const capped = summary.length > 3800 ? summary.substring(0, 3800) : summary;
      DistributedSync.kv.put(DistributedSync.KEY_CONTEXT, capped).then((_v) => {
        Log.info('context published to distributed kv');
      }).catch((e: BusinessError) => {
        Log.warn(`kv put failed ${e.code}: ${e.message}`);
      });
    }).catch((_e: Error) => {
    });
  }

  /** 读取其它设备发布的最新会话摘要（无记录返回空串） */
  static async fetchContext(): Promise<string> {
    if (!await DistributedSync.ensure() || DistributedSync.kv === null) {
      return '';
    }
    try {
      const v = await DistributedSync.kv.get(DistributedSync.KEY_CONTEXT);
      return typeof v === 'string' ? v as string : '';
    } catch (e) {
      return '';
    }
  }
}
