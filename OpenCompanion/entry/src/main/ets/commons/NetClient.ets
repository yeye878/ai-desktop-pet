import { http } from '@kit.NetworkKit';
import { util } from '@kit.ArkTS';
import { AppContext, SettingKey } from './AppContext';
import { Log } from './Log';

export interface StreamHandlers {
  onChunk: (text: string) => void;
  onDone: () => void;
  onError: (msg: string) => void;
}

/**
 * HTTP 客户端（对应桌面版 reqwest：@ohos.net.http 的 dataReceive 事件提供 SSE 流式读取能力）
 */
export class NetClient {
  /**
   * 代理连通性状态（本次进程内）：
   * 走代理的请求发生连接级失败（代理软件没开、端口不通等）时置 true，
   * 后续所有请求自动直连，不再反复撞死代理。用户改配置或重启应用后恢复尝试。
   */
  private static proxyDead: boolean = false;

  /**
   * 全局 HTTP 代理（可选）：配置了 host+port 即对所有请求生效。
   * 用于设备域名解析异常的环境（如部分模拟器镜像 SELinux 阻断 netmanager，DNS 全灭），
   * 显式走代理后域名解析在代理端完成，绕开本机 resolver。
   * 代理不可达时自动回退直连（见 proxyDead）。
   */
  static proxyOption(): http.HttpProxy | null {
    if (NetClient.proxyDead) {
      return null;
    }
    const host = AppContext.getSetting(SettingKey.PROXY_HOST, '').trim();
    const port = AppContext.getNumber(SettingKey.PROXY_PORT, 0);
    if (host.length === 0 || port <= 0 || port > 65535) {
      return null;
    }
    return { host: host, port: port, exclusionList: [] };
  }

  /** 给请求参数挂代理；返回本次是否实际走了代理 */
  private static applyProxy(opt: http.HttpRequestOptions): boolean {
    const px = NetClient.proxyOption();
    if (px !== null) {
      opt.usingProxy = px;
      return true;
    }
    return false;
  }

  /** 连接级错误（23000xx 段，DNS/拒连/超时等），区别于 HTTP 状态码错误——只有前者值得直连重试 */
  private static isConnectError(e: Object): boolean {
    const err = e as BusinessError;
    return typeof err.code === 'number' && err.code >= 2300000 && err.code < 2400000;
  }

  /** 代理请求连接失败 → 标记代理失效并打日志（一次进程内提示一次即可，避免刷屏） */
  private static proxyFailed(msg: string): void {
    if (!NetClient.proxyDead) {
      Log.warn(`proxy unreachable (${msg}), fallback to direct connection`);
    }
    NetClient.proxyDead = true;
  }

  /** GET JSON 文本 */
  static async getText(url: string, headers: Map<string, string> = new Map<string, string>()): Promise<string> {
    const res = await NetClient.requestWithFallback(url, (): http.HttpRequestOptions => {
      return {
        method: http.RequestMethod.GET,
        header: NetClient.headerObj(headers),
        readTimeout: 20000,
        connectTimeout: 10000
      } as http.HttpRequestOptions;
    });
    if (res.responseCode >= 400) {
      throw new Error(`HTTP ${res.responseCode}`);
    }
    return typeof res.result === 'string' ? res.result : JSON.stringify(res.result);
  }

  /** POST JSON，非流式（返回原始响应文本） */
  static async postText(url: string, headers: Map<string, string>, body: string, timeoutMs: number = 180000): Promise<string> {
    const res = await NetClient.requestWithFallback(url, (): http.HttpRequestOptions => {
      return {
        method: http.RequestMethod.POST,
        header: NetClient.headerObj(headers),
        extraData: body,
        readTimeout: timeoutMs,
        connectTimeout: 15000
      } as http.HttpRequestOptions;
    });
    return typeof res.result === 'string' ? res.result : JSON.stringify(res.result);
  }

  /** 单次请求（自建自销） */
  private static async requestOnce(url: string, opt: http.HttpRequestOptions): Promise<http.HttpResponse> {
    const req = http.createHttp();
    try {
      return await req.request(url, opt);
    } finally {
      req.destroy();
    }
  }

  /** 挂代理发起；代理连接失败时清掉代理位重建参数直连重试一次 */
  private static async requestWithFallback(url: string,
    buildOpt: () => http.HttpRequestOptions): Promise<http.HttpResponse> {
    const opt = buildOpt();
    const viaProxy = NetClient.applyProxy(opt);
    try {
      return await NetClient.requestOnce(url, opt);
    } catch (e) {
      if (viaProxy && NetClient.isConnectError(e)) {
        NetClient.proxyFailed((e as BusinessError).message);
        return await NetClient.requestOnce(url, buildOpt());
      }
      throw e as Error;
    }
  }

  /**
   * POST JSON 流式读取（SSE 等 chunked 响应），返回 abort 句柄。
   * 代理链路连接失败时自动直连重发一次（与 getText/postText 行为一致）。
   */
  static streamPost(url: string, headers: Map<string, string>, body: string, handlers: StreamHandlers): NetStream {
    const stream = new NetStream();
    const decoder = util.TextDecoder.create('utf-8');
    let buffer = '';
    let done = false;

    const fire = (): void => {
      const req = http.createHttp();
      stream.attach(req);

      const finish = (): void => {
        // 正常结束也要销毁请求：dataEnd 后 req 不 destroy 会泄漏连接（仅 abort 时销毁不够）
        if (!done) {
          done = true;
        }
        try {
          req.destroy();
        } catch (e) {
          // ignore
        }
      };

      req.on('headersReceive', (h: Object) => {
        // 网关以 JSON 错误体响应时 dataEnd 仍会触发 onDone（空内容假成功），
        // 在 header 阶段拦截非 2xx 直接走错误路径
        const code = NetClient.statusCodeFromHeaders(h);
        if (code >= 400 && !done && !stream.aborted) {
          done = true;
          handlers.onError(`HTTP ${code}`);
          finish();
        }
      });
      req.on('dataReceive', (chunk: ArrayBuffer) => {
        if (stream.aborted || done) {
          return;
        }
        try {
          const text = decoder.decodeWithStream(new Uint8Array(chunk), { stream: true });
          buffer += text;
          // 隔离 UI 层回调链异常，避免中断后续 chunk
          try {
            handlers.onChunk(text);
          } catch (e) {
            Log.error(`onChunk handler failed: ${(e as Error).message}`);
          }
        } catch (e) {
          Log.error(`decode chunk failed: ${(e as Error).message}`);
        }
      });
      req.on('dataEnd', () => {
        if (!done && !stream.aborted) {
          done = true;
          handlers.onDone();
        }
        finish();
      });

      const opt: http.HttpRequestOptions = {
        method: http.RequestMethod.POST,
        header: NetClient.headerObj(headers),
        extraData: body,
        usingProtocol: http.HttpProtocol.HTTP1_1,
        readTimeout: 300000,
        connectTimeout: 15000
      };
      const viaProxy = NetClient.applyProxy(opt);
      req.request(url, opt).then((res: http.HttpResponse) => {
        if (done || stream.aborted) {
          return;
        }
        if (res.responseCode >= 400) {
          done = true;
          const raw = typeof res.result === 'string' ? res.result : '';
          handlers.onError(`HTTP ${res.responseCode}: ${NetClient.trimError(raw, buffer)}`);
          finish();
          return;
        }
        // 部分设备（模拟器）dataReceive 不增量回调，响应在 promise 完成时一次性给出
        if (buffer.length === 0 && typeof res.result === 'string' && (res.result as string).length > 0) {
          done = true;
          handlers.onChunk(res.result as string);
          handlers.onDone();
          finish();
        }
      }).catch((e: BusinessError) => {
        // 代理不可达（连接级错误且未收到任何数据）→ 标记代理失效，直连重发一次
        if (viaProxy && buffer.length === 0 && !done && !stream.aborted && NetClient.isConnectError(e)) {
          NetClient.proxyFailed(e.message);
          try {
            req.off('headersReceive');
            req.off('dataReceive');
            req.off('dataEnd');
          } catch (e2) {
            // ignore
          }
          req.destroy();
          fire();
          return;
        }
        if (!done && !stream.aborted) {
          done = true;
          handlers.onError(`网络错误: ${e.message}（若 buffer 有内容则可能已完成）`);
        }
        finish();
      });
    };

    fire();
    return stream;
  }

  private static statusCodeFromHeaders(h: Object): number {
    try {
      const rec = h as Record<string, Object>;
      // headersReceive 回调参数为 header 对象；状态码不在 header 里，兜底 200，
      // 真实状态码由 request promise 分支处理（headersReceive 只做能拿到的拦截）
      const keys = Object.keys(rec);
      for (let i = 0; i < keys.length; i++) {
        if (keys[i].toLowerCase() === 'content-type') {
          return 200;
        }
      }
    } catch (e) {
      // ignore
    }
    return 200;
  }

  private static trimError(raw: string, buffer: string): string {
    const src = raw.length > 0 ? raw : buffer;
    if (src.length > 400) {
      return src.substring(0, 400);
    }
    return src;
  }

  private static headerObj(headers: Map<string, string>): Record<string, string> {
    const obj: Record<string, string> = {};
    headers.forEach((v: string, k: string) => {
      obj[k] = v;
    });
    return obj;
  }
}

/** 流式请求句柄，用于中断（对应桌面版 CancellationToken 的网络部分）。
 * 代理失效直连重发时内部请求会被替换，故句柄与请求解耦。 */
export class NetStream {
  private request: http.HttpRequest | null = null;
  aborted: boolean = false;

  attach(req: http.HttpRequest): void {
    this.request = req;
  }

  abort(): void {
    if (this.aborted) {
      return;
    }
    this.aborted = true;
    try {
      this.request?.destroy();
    } catch (e) {
      // ignore
    }
  }
}
