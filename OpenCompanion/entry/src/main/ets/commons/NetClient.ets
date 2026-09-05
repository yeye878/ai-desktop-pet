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

    req.on('headersReceive', (h: Object) => {
      Log.debug(`stream headers: ${JSON.stringify(h)}`);
    });
    req.on('dataReceive', (chunk: ArrayBuffer) => {
      if (stream.aborted) {
        return;
      }
      try {
        const text = decoder.decodeWithStream(new Uint8Array(chunk), { stream: true });
        buffer += text;
        handlers.onChunk(text);
      } catch (e) {
        Log.error(`decode chunk failed: ${(e as Error).message}`);
      }
    });
    req.on('dataEnd', () => {
      if (!done && !stream.aborted) {
        done = true;
        handlers.onDone();
      }
    });

    const opt: http.HttpRequestOptions = {
      method: http.RequestMethod.POST,
      header: NetClient.headerObj(headers),
      extraData: body,
      readTimeout: 300000,
      connectTimeout: 15000
    };
    req.request(url, opt).then((res: http.HttpResponse) => {
      if (res.responseCode >= 400 && !done && !stream.aborted) {
        done = true;
        const raw = typeof res.result === 'string' ? res.result : '';
        handlers.onError(`HTTP ${res.responseCode}: ${NetClient.trimError(raw, buffer)}`);
      }
    }).catch((e: BusinessError) => {
      if (!done && !stream.aborted) {
        done = true;
        handlers.onError(`网络错误: ${e.message}（若 buffer 有内容则可能已完成）`);
      }
    });
    return stream;
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
