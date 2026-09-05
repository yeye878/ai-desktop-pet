import { AppContext, SettingKey } from '../commons/AppContext';
import { AssetStore } from '../commons/AssetStore';
import { NetClient, NetStream } from '../commons/NetClient';
import { Log } from '../commons/Log';
import { LlmMessage, LlmProfile, ToolCallReq, ToolDef, ContentPart } from './Types';

/**
 * LLM 客户端（对应桌面版 direct_api/api_client.rs：OpenAI 兼容协议，SSE 流式 + 工具调用增量聚合）
 */
export class LlmProfiles {
  static cache: Map<string, string> = new Map<string, string>();

  static list(): LlmProfile[] {
    const raw = AppContext.getSetting(SettingKey.API_PROFILES, '[]');
    const out: LlmProfile[] = [];
    try {
      const arr: Array<Object> = JSON.parse(raw) as Array<Object>;
      arr.forEach((o: Object) => {
        const m = o as Record<string, Object>;
        const p = new LlmProfile();
        p.id = typeof m['id'] === 'string' ? m['id'] as string : '';
        p.name = typeof m['name'] === 'string' ? m['name'] as string : '未命名';
        p.baseUrl = typeof m['baseUrl'] === 'string' ? m['baseUrl'] as string : 'https://api.openai.com/v1';
        p.model = typeof m['model'] === 'string' ? m['model'] as string : '';
        p.temperature = typeof m['temperature'] === 'number' ? m['temperature'] as number : 0.7;
        p.useStream = typeof m['useStream'] === 'boolean' ? m['useStream'] as boolean : true;
        p.confirmEnabled = typeof m['confirmEnabled'] === 'boolean' ? m['confirmEnabled'] as boolean : true;
        out.push(p);
      });
    } catch (e) {
      Log.warn(`parse profiles failed: ${(e as Error).message}`);
    }
    return out;
  }

  static async saveAll(profiles: LlmProfile[]): Promise<void> {
    const arr: Array<Record<string, Object>> = [];
    profiles.forEach((p: LlmProfile) => {
      const m: Record<string, Object> = {
        'id': p.id, 'name': p.name, 'baseUrl': p.baseUrl, 'model': p.model,
        'temperature': p.temperature, 'useStream': p.useStream, 'confirmEnabled': p.confirmEnabled
      };
      arr.push(m);
      if (p.apiKey.length > 0) {
        // 密钥入系统资产存储，JSON 中永不落明文（与桌面版策略一致）
        LlmProfiles.cache.set(p.id, p.apiKey);
        AssetStore.setSecret(`llmkey_${p.id}`, p.apiKey);
      }
    });
    await AppContext.setSetting(SettingKey.API_PROFILES, JSON.stringify(arr));
  }

  static activeId(): string {
    return AppContext.getSetting(SettingKey.ACTIVE_PROFILE, '');
  }

  static async setActive(id: string): Promise<void> {
    await AppContext.setSetting(SettingKey.ACTIVE_PROFILE, id);
  }

  static async active(): Promise<LlmProfile | null> {
    const id = LlmProfiles.activeId();
    const profiles = LlmProfiles.list();
    let picked: LlmProfile | null = null;
    profiles.forEach((p: LlmProfile) => {
      if (p.id === id) {
        picked = p;
      }
    });
    if (picked === null && profiles.length > 0) {
      picked = profiles[0];
    }
    if (picked === null) {
      return null;
    }
    const key = await LlmProfiles.resolveKey(picked);
    const p: LlmProfile = picked;
    p.apiKey = key;
    return p;
  }

  static async resolveKey(p: LlmProfile): Promise<string> {
    const cached = LlmProfiles.cache.get(p.id);
    if (cached !== undefined && cached.length > 0) {
      return cached;
    }
    const secret = await AssetStore.getSecret(`llmkey_${p.id}`);
    if (secret.length > 0) {
      LlmProfiles.cache.set(p.id, secret);
    }
    return secret;
  }

  static async removeProfile(id: string): Promise<void> {
    const rest = LlmProfiles.list().filter((p: LlmProfile) => p.id !== id);
    await LlmProfiles.saveAll(rest);
    await AssetStore.removeSecret(`llmkey_${id}`);
    LlmProfiles.cache.delete(id);
  }
}

export interface StreamCallbacks {
  onThinking: (delta: string) => void;
  onContent: (delta: string) => void;
  onToolCalls: (calls: ToolCallReq[]) => void;
  onDone: (fullContent: string, toolCalls: ToolCallReq[]) => void;
  onError: (msg: string) => void;
}

interface SseChoiceDelta {
  content?: string;
  reasoning_content?: string;
  tool_calls?: Array<Record<string, Object>>;
  finish_reason?: string;
}

/** 单次流式运行态 */
class StreamRun {
  buffer: string = '';
  fullContent: string = '';
  toolAcc: Map<number, ToolCallReq> = new Map<number, ToolCallReq>();
  sawSse: boolean = false;
  finished: boolean = false;
}

export class LlmClient {
  private static streams: Map<string, NetStream> = new Map<string, NetStream>();
  private static seq: number = 1;

  static joinUrl(baseUrl: string, path: string): string {
    const b = baseUrl.replace(/\/+$/, '');
    if (b.endsWith('/chat/completions')) {
      return b;
    }
    return `${b}${path}`;
  }

  static authHeaders(p: LlmProfile): Map<string, string> {
    const headers = new Map<string, string>();
    headers.set('Content-Type', 'application/json');
    if (p.apiKey.length > 0) {
      headers.set('Authorization', `Bearer ${p.apiKey}`);
    }
    return headers;
  }

  /** 消息序列化为 OpenAI 协议 JSON */
  static messagesToJson(messages: LlmMessage[]): Array<Object> {
    const out: Array<Object> = [];
    messages.forEach((m: LlmMessage) => {
      const o: Record<string, Object> = { 'role': m.role };
      if (m.parts !== null && m.role === 'user') {
        const parts: Array<Object> = [];
        m.parts.forEach((part: ContentPart) => {
          if (part.type === 'text') {
            parts.push({ 'type': 'text', 'text': part.text } as Record<string, Object>);
          } else if (part.imageUrl.length > 0) {
            const img: Record<string, Object> = { 'url': part.imageUrl };
            parts.push({ 'type': 'image_url', 'image_url': img } as Record<string, Object>);
          }
        });
        o['content'] = parts;
      } else {
        o['content'] = m.content;
      }
      if (m.toolCalls !== null && m.toolCalls.length > 0) {
        const calls: Array<Object> = [];
        m.toolCalls.forEach((tc: ToolCallReq) => {
          const fn: Record<string, Object> = { 'name': tc.name, 'arguments': tc.arguments };
          calls.push({ 'id': tc.id, 'type': 'function', 'function': fn } as Record<string, Object>);
        });
        o['tool_calls'] = calls;
      }
      if (m.toolCallId.length > 0) {
        o['tool_call_id'] = m.toolCallId;
      }
      out.push(o);
    });
    return out;
  }

  static toolsToJson(tools: ToolDef[]): Array<Object> {
    const out: Array<Object> = [];
    tools.forEach((t: ToolDef) => {
      const props: Record<string, Object> = {};
      const required: string[] = [];
      t.parameters.forEach((pp) => {
        const desc: Record<string, Object> = { 'type': pp.type, 'description': pp.description };
        props[pp.name] = desc;
        if (pp.required) {
          required.push(pp.name);
        }
      });
      const parameters: Record<string, Object> = { 'type': 'object', 'properties': props };
      if (required.length > 0) {
        parameters['required'] = required;
      }
      const fn: Record<string, Object> = { 'name': t.name, 'description': t.description, 'parameters': parameters };
      out.push({ 'type': 'function', 'function': fn } as Record<string, Object>);
    });
    return out;
  }

  /** 流式对话（SSE）。runId 用于中断。 */
  static chatStream(p: LlmProfile, messages: LlmMessage[], tools: ToolDef[] | null,
    cb: StreamCallbacks, runId: string): void {
    const body: Record<string, Object> = {
      'model': p.model,
      'messages': LlmClient.messagesToJson(messages),
      'stream': true,
      'temperature': p.temperature
    };
    if (tools !== null && tools.length > 0) {
      body['tools'] = LlmClient.toolsToJson(tools);
    }
    const run = new StreamRun();
    const url = LlmClient.joinUrl(p.baseUrl, '/chat/completions');
    const stream = NetClient.streamPost(url, LlmClient.authHeaders(p), JSON.stringify(body), {
      onChunk: (text: string) => {
        if (run.finished) {
          return;
        }
        run.buffer += text;
        LlmClient.feedSse(run, cb);
      },
      onDone: () => {
        if (run.finished) {
          return;
        }
        // 兼容非流式返回（某些网关忽略 stream=true）
        if (!run.sawSse && run.buffer.trim().startsWith('{')) {
          LlmClient.parseFullCompletion(run, cb);
        }
        run.finished = true;
        const calls = LlmClient.accumulatedCalls(run);
        cb.onDone(run.fullContent, calls);
      },
      onError: (msg: string) => {
        if (run.finished) {
          return;
        }
        run.finished = true;
        cb.onError(msg);
      }
    });
    LlmClient.streams.set(runId, stream);
  }

  /** 非流式单轮对话（规划器/复盘/测试连接用） */
  static async chatOnce(p: LlmProfile, system: string, user: string, maxTokens: number = 1200): Promise<string> {
    const msgs: LlmMessage[] = [];
    if (system.length > 0) {
      const sys = new LlmMessage();
      sys.role = 'system';
      sys.content = system;
      msgs.push(sys);
    }
    const um = new LlmMessage();
    um.role = 'user';
    um.content = user;
    msgs.push(um);
    const body: Record<string, Object> = {
      'model': p.model,
      'messages': LlmClient.messagesToJson(msgs),
      'temperature': 0.3,
      'max_tokens': maxTokens
    };
    const url = LlmClient.joinUrl(p.baseUrl, '/chat/completions');
    const raw = await NetClient.postText(url, LlmClient.authHeaders(p), JSON.stringify(body));
    const obj = JSON.parse(raw) as Record<string, Object>;
    const choices = obj['choices'] as Array<Object>;
    if (!choices || choices.length === 0) {
      const errObj = obj['error'] as Record<string, Object> | null;
      throw new Error(errObj ? `LLM错误: ${JSON.stringify(errObj)}` : `LLM响应异常: ${raw.substring(0, 200)}`);
    }
    const first = choices[0] as Record<string, Object>;
    const message = first['message'] as Record<string, Object>;
    const content = message['content'];
    return typeof content === 'string' ? content as string : '';
  }

  static async listModels(p: LlmProfile): Promise<string[]> {
    const b = p.baseUrl.replace(/\/+$/, '').replace(/\/chat\/completions$/, '');
    const raw = await NetClient.getText(`${b}/models`, LlmClient.authHeaders(p));
    const obj = JSON.parse(raw) as Record<string, Object>;
    const data = obj['data'] as Array<Object>;
    const out: string[] = [];
    if (data) {
      data.forEach((o: Object) => {
        const m = o as Record<string, Object>;
        const id = m['id'];
        if (typeof id === 'string') {
          out.push(id as string);
        }
      });
    }
    return out;
  }

  static async testConnection(p: LlmProfile): Promise<string> {
    const reply = await LlmClient.chatOnce(p, '你是连通性测试器。只回复两个字：正常', 'ping', 16);
    return reply.trim().length > 0 ? reply.trim() : '(空回复)';
  }

  static cancel(runId: string): void {
    const s = LlmClient.streams.get(runId);
    if (s) {
      s.abort();
      LlmClient.streams.delete(runId);
    }
  }

  static cancelAll(): void {
    LlmClient.streams.forEach((s: NetStream) => {
      s.abort();
    });
    LlmClient.streams.clear();
  }

  // ---------- SSE 解析（对应桌面版 SseParser） ----------

  private static feedSse(run: StreamRun, cb: StreamCallbacks): void {
    let idx = run.buffer.indexOf('\n');
    while (idx >= 0) {
      const line = run.buffer.substring(0, idx).replace(/\r$/, '');
      run.buffer = run.buffer.substring(idx + 1);
      LlmClient.handleSseLine(run, line, cb);
      idx = run.buffer.indexOf('\n');
    }
  }

  private static handleSseLine(run: StreamRun, line: string, cb: StreamCallbacks): void {
    if (!line.startsWith('data:')) {
      return;
    }
    run.sawSse = true;
    const payload = line.substring(5).trim();
    if (payload === '[DONE]') {
      run.finished = true;
      cb.onDone(run.fullContent, LlmClient.accumulatedCalls(run));
      return;
    }
    if (payload.length === 0) {
      return;
    }
    let obj: Record<string, Object>;
    try {
      obj = JSON.parse(payload) as Record<string, Object>;
    } catch (e) {
      return;
    }
    const choices = obj['choices'] as Array<Object> | null;
    if (!choices || choices.length === 0) {
      return;
    }
    const first = choices[0] as Record<string, Object>;
    const delta = first['delta'] as Record<string, Object> | null;
    if (!delta) {
      return;
    }
    const reasoning = delta['reasoning_content'];
    if (typeof reasoning === 'string' && (reasoning as string).length > 0) {
      cb.onThinking(reasoning as string);
    }
    const content = delta['content'];
    if (typeof content === 'string' && (content as string).length > 0) {
      run.fullContent += content as string;
      cb.onContent(content as string);
    }
    const toolCalls = delta['tool_calls'] as Array<Record<string, Object>> | null;
    if (toolCalls) {
      toolCalls.forEach((tc: Record<string, Object>) => {
        const index = typeof tc['index'] === 'number' ? tc['index'] as number : 0;
        let acc = run.toolAcc.get(index);
        if (!acc) {
          acc = new ToolCallReq();
          run.toolAcc.set(index, acc);
        }
        const id = tc['id'];
        if (typeof id === 'string') {
          acc.id = id as string;
        }
        const fn = tc['function'] as Record<string, Object> | null;
        if (fn) {
          const name = fn['name'];
          if (typeof name === 'string' && (name as string).length > 0) {
            acc.name += name as string;
          }
          const args = fn['arguments'];
          if (typeof args === 'string') {
            acc.arguments += args as string;
          }
        }
      });
      cb.onToolCalls(LlmClient.accumulatedCalls(run));
    }
  }

  private static parseFullCompletion(run: StreamRun, cb: StreamCallbacks): void {
    try {
      const obj = JSON.parse(run.buffer) as Record<string, Object>;
      const choices = obj['choices'] as Array<Object> | null;
      if (choices && choices.length > 0) {
        const first = choices[0] as Record<string, Object>;
        const message = first['message'] as Record<string, Object> | null;
        if (message) {
          const content = message['content'];
          if (typeof content === 'string') {
            run.fullContent += content as string;
            cb.onContent(content as string);
          }
          const tcs = message['tool_calls'] as Array<Record<string, Object>> | null;
          if (tcs) {
            tcs.forEach((tc: Record<string, Object>, i: number) => {
              const fn = tc['function'] as Record<string, Object> | null;
              const acc = new ToolCallReq();
              acc.id = typeof tc['id'] === 'string' ? tc['id'] as string : `call_${i}`;
              acc.name = fn && typeof fn['name'] === 'string' ? fn['name'] as string : '';
              acc.arguments = fn && typeof fn['arguments'] === 'string' ? fn['arguments'] as string : '';
              run.toolAcc.set(i, acc);
            });
            cb.onToolCalls(LlmClient.accumulatedCalls(run));
          }
        }
      }
    } catch (e) {
      Log.warn(`parse full completion failed: ${(e as Error).message}`);
    }
  }

  private static accumulatedCalls(run: StreamRun): ToolCallReq[] {
    const out: ToolCallReq[] = [];
    const indexes: number[] = [];
    run.toolAcc.forEach((_v: ToolCallReq, k: number) => {
      indexes.push(k);
    });
    indexes.sort((a: number, b: number) => a - b);
    indexes.forEach((k: number) => {
      const v = run.toolAcc.get(k);
      if (v && v.name.length > 0) {
        out.push(v);
      }
    });
    return out;
  }
}
