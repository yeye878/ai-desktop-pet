import { deviceInfo } from '@kit.BasicServicesKit';
import { Log } from './Log';

/**
 * 设备形态检测（对应方案 2.2 / 3.3）：
 * DataAugmentation Kit 的 RAG 问答与端侧问答模型仅支持 PC / 2in1 设备，
 * 手机端降级为「仅检索 + 展示相关片段」，不触发任何生成回答链路；
 * 检索（Kit 倒排 + 本地 bigram 兜底）全设备形态可用。
 */
export class DeviceInfo {
  private static cachedType: string = '';

  /** 设备类型：phone / tablet / 2in1 / tv / wearable / car / default */
  static type(): string {
    if (DeviceInfo.cachedType.length === 0) {
      try {
        DeviceInfo.cachedType = deviceInfo.deviceType;
      } catch (e) {
        Log.warn(`deviceType read failed: ${(e as Error).message}`);
        DeviceInfo.cachedType = 'default';
      }
    }
    return DeviceInfo.cachedType;
  }

  static isPhone(): boolean {
    return DeviceInfo.type() === 'phone';
  }

  /**
   * RAG 生成回答能力（Kit rag 会话 / localChatModel 端侧模型）是否可用。
   * 手机端返回 false：走纯检索降级链（方案 3.3「手机端 RAG 生成回答 → 降级为仅检索 + 展示相关片段」）。
   */
  static ragGenerateSupported(): boolean {
    return !DeviceInfo.isPhone();
  }

  /** 中文形态名（UI 提示用） */
  static label(): string {
    const t = DeviceInfo.type();
    if (t === 'phone') {
      return '手机';
    }
    if (t === 'tablet') {
      return '平板';
    }
    if (t === '2in1') {
      return '二合一';
    }
    return t;
  }
}
