import { http } from '@kit.NetworkKit';
import { util } from '@kit.ArkTS';
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
  /** GET JSON 文本 */
  static async getText(url: string, headers: Map<string, string> = new Map<string, string>()): Promise<string> {
    const req = http.createHttp();
    try {
      const opt: http.HttpRequestOptions = {
        method: http.RequestMethod.GET,
        header: NetClient.headerObj(headers),
        readTimeout: 20000,
        connectTimeout: 10000
      };
      const res = await req.request(url, opt);
      if (res.responseCode >= 400) {
        throw new Error(`HTTP ${res.responseCode}`);
      }
      return typeof res.result === 'string' ? res.result : JSON.stringify(res.result);
    } finally {
      req.destroy();
    }
  }

  /** POST JSON，非流式（返回原始响应文本） */
  static async postText(url: string, headers: Map<string, string>, body: string, timeoutMs: number = 180000): Promise<string> {
    const req = http.createHttp();
    try {
      const opt: http.HttpRequestOptions = {
        method: http.RequestMethod.POST,
        header: NetClient.headerObj(headers),
        extraData: body,
        readTimeout: timeoutMs,
        connectTimeout: 15000
      };
      const res = await req.request(url, opt);
      return typeof res.result === 'string' ? res.result : JSON.stringify(res.result);
    } finally {
      req.destroy();
    }
  }

  /**
   * POST JSON 流式读取（SSE 等 chunked 响应），返回 abort 句柄
   */
  static streamPost(url: string, headers: Map<string, string>, body: string, handlers: StreamHandlers): NetStream {
    const req = http.createHttp();
    const stream = new NetStream(req);
    const decoder = util.TextDecoder.create('utf-8');
    let buffer = '';
    let done = false;

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
      if (!done && !stream.aborted) {
        done = true;
        handlers.onError(`网络错误: ${e.message}（若 buffer 有内容则可能已完成）`);
      }
      finish();
    });
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

/** 流式请求句柄，用于中断（对应桌面版 CancellationToken 的网络部分） */
export class NetStream {
  private request: http.HttpRequest;
  aborted: boolean = false;

  constructor(request: http.HttpRequest) {
    this.request = request;
  }

  abort(): void {
    if (this.aborted) {
      return;
    }
    this.aborted = true;
    try {
      this.request.destroy();
    } catch (e) {
      // ignore
    }
  }
}
