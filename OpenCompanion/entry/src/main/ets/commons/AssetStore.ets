import { asset } from '@kit.AssetStoreKit';
import { util } from '@kit.ArkTS';
import { Log } from './Log';

/**
 * API Key 等机密的安全存储（对应桌面版 keyring → Windows 凭据管理器。
 * 鸿蒙使用系统资产存储 Asset Store Kit；不可用时降级到内存/编码存储并明确告警。）
 */
export class AssetStore {
  private static fallback: Map<string, string> = new Map<string, string>();

  private static toBytes(s: string): Uint8Array {
    const encoder = new util.TextEncoder();
    return encoder.encodeInto(s);
  }

  private static toStr(b: Uint8Array): string {
    const decoder = util.TextDecoder.create('utf-8');
    return decoder.decodeToString(b);
  }

  /** 写入机密；返回是否使用了系统级安全存储 */
  static async setSecret(alias: string, secret: string): Promise<boolean> {
    const query: asset.AssetMap = new Map<asset.Tag, asset.Value>();
    query.set(asset.Tag.ALIAS, AssetStore.toBytes(alias));
    try {
      // 先移除旧值；不存在（24000002）属正常，继续写入
      await asset.remove(query);
    } catch (e) {
      // ignore：首次写入时旧值不存在
    }
    try {
      const attrs: asset.AssetMap = new Map<asset.Tag, asset.Value>();
      attrs.set(asset.Tag.ALIAS, AssetStore.toBytes(alias));
      attrs.set(asset.Tag.SECRET, AssetStore.toBytes(secret));
      attrs.set(asset.Tag.SYNC_TYPE, asset.SyncType.NEVER);
      await asset.add(attrs);
      return true;
    } catch (e) {
      const err = e as BusinessError;
      Log.warn(`asset store unavailable(${err.code}), fallback to memory: ${err.message}`);
      AssetStore.fallback.set(alias, secret);
      return false;
    }
  }

  static async getSecret(alias: string): Promise<string> {
    try {
      const query: asset.AssetMap = new Map<asset.Tag, asset.Value>();
      query.set(asset.Tag.ALIAS, AssetStore.toBytes(alias));
      query.set(asset.Tag.RETURN_TYPE, asset.ReturnType.ALL);
      const result: Array<asset.AssetMap> = await asset.query(query);
      if (result.length > 0) {
        const secretVal = result[0].get(asset.Tag.SECRET);
        if (secretVal instanceof Uint8Array) {
          return AssetStore.toStr(secretVal);
        }
      }
      return '';
    } catch (e) {
      const fb = AssetStore.fallback.get(alias);
      return fb === undefined ? '' : fb;
    }
  }

  static async removeSecret(alias: string): Promise<void> {
    AssetStore.fallback.delete(alias);
    try {
      const query: asset.AssetMap = new Map<asset.Tag, asset.Value>();
      query.set(asset.Tag.ALIAS, AssetStore.toBytes(alias));
      await asset.remove(query);
    } catch (e) {
      // ignore
    }
  }
}
