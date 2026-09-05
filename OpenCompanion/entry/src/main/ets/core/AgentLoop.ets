import { LlmClient } from './LlmClient';
import { ToolRegistry } from './ToolRegistry';
import { Gates } from './Gates';
import { Log } from '../commons/Log';
import { CancelToken, ChatMessage, ContentPart, LlmMessage, ToolCallReq, ToolDef, LlmProfile } from './Types';

export interface LoopCallbacks {
  onThinking: (delta: string) => void;
  onContent: (delta: string) => void;
  onToolTrace: (line: string) => void;
}

/** 一次智能体运行的参数 */
export class LoopOptions {
  profile: LlmProfile | null = null;
  systemPrompt: string = '';
  history: ChatMessage[] = [];
  userText: string = '';
  imageUris: string[] = [];
  tools: ToolDef[] = [];
  allowConfirm: boolean = true;
  maxTurns: number = 24;
  token: CancelToken = new CancelToken();
  callbacks: LoopCallbacks | null = null;
}

export class LoopResult {
  content: string = '';
  trace: string[] = [];
  aborted: boolean = false;
}

/**
 * ReAct 工具循环（对应桌面版 agent.rs run_direct_api_agent：
 * 每轮调模型 → 返回 tool_calls 则执行并回填 → 直到纯文本结束）
 */
export class AgentLoop {
  static run(o: LoopOptions): Promise<LoopResult> {
    return new Promise<LoopResult>((resolve: (r: LoopResult) => void) => {
      AgentLoop.runInner(o, resolve);
    });
  }

  private static runInner(o: LoopOptions, resolve: (r: LoopResult) => void): void {
    const result = new LoopResult();
    const profile = o.profile;
    const cbs = o.callbacks;
    if (profile === null) {
      result.content = '未配置 LLM API：请到「设置」添加 API 配置（任意 OpenAI 兼容服务），激活后即可对话。';
      if (cbs) {
        cbs.onToolTrace('未配置 LLM API，请到「设置」中添加并激活 API 配置');
      }
      resolve(result);
      return;
    }

    // 组装消息：system + 历史 + 本轮输入
    const msgs: LlmMessage[] = [];
    const sys = new LlmMessage();
    sys.role = 'system';
    sys.content = o.systemPrompt;
    msgs.push(sys);
    o.history.forEach((h: ChatMessage) => {
      const m = new LlmMessage();
      if (h.role === 'user') {
        m.role = 'user';
        m.content = h.quote.length > 0 ? `（引用：${h.quote}）\n${h.content}` : h.content;
        if (h.imageUris.length > 0) {
          m.parts = AgentLoop.buildParts(h.content, h.imageUris);
          m.content = '';
        }
      } else {
        m.role = 'assistant';
        m.content = h.content;
      }
      msgs.push(m);
    });
    const user = new LlmMessage();
    user.role = 'user';
    user.content = o.userText;
    if (o.imageUris.length > 0) {
      user.parts = AgentLoop.buildParts(o.userText, o.imageUris);
      user.content = '';
    }
    msgs.push(user);

    const approved: Set<string> = new Set<string>();
    let turn = 0;

    /** 单轮：调模型 → 工具 或 结束 */
    const step = (): void => {
      if (o.token.cancelled) {
        result.aborted = true;
        resolve(result);
        return;
      }
      turn++;
      if (turn > o.maxTurns) {
        if (cbs) {
          cbs.onToolTrace(`已达到最大工具轮次(${o.maxTurns})，停止`);
        }
        resolve(result);
        return;
      }
      const runId = `loop_${Date.now()}_${turn}`;
      o.token.onCancel(() => {
        LlmClient.cancel(runId);
      });
      LlmClient.chatStream(profile, msgs, o.tools.length > 0 ? o.tools : null, {
        onThinking: (d: string) => {
          if (cbs) {
            cbs.onThinking(d);
          }
        },
        onContent: (d: string) => {
          if (cbs) {
            cbs.onContent(d);
          }
        },
        onToolCalls: (_calls: ToolCallReq[]) => {
          // 增量聚合期间无需打扰 UI
        },
        onDone: (full: string, calls: ToolCallReq[]) => {
          if (o.token.cancelled) {
            result.aborted = true;
            if (full.length > 0 && result.content.length === 0) {
              result.content = full;
            }
            resolve(result);
            return;
          }
          if (calls.length > 0) {
            const asst = new LlmMessage();
            asst.role = 'assistant';
            asst.content = full;
            asst.toolCalls = calls;
            msgs.push(asst);
            AgentLoop.executeCalls(o, calls, result, approved, msgs, step);
            return;
          }
          result.content = full;
          resolve(result);
        },
        onError: (msg: string) => {
          if (o.token.cancelled) {
            result.aborted = true;
            resolve(result);
            return;
          }
          Log.error(`agent loop llm error: ${msg}`);
          if (cbs) {
            cbs.onToolTrace(`模型调用失败: ${msg}`);
          }
          resolve(result);
        }
      }, runId);
    };
    step();
  }

  /** 顺序执行本轮所有工具调用，然后继续下一轮 */
  private static executeCalls(o: LoopOptions, calls: ToolCallReq[], result: LoopResult,
    approved: Set<string>, msgs: LlmMessage[], next: () => void): void {
    const cbs = o.callbacks;
    const confirmEnabled = o.profile !== null && o.profile.confirmEnabled;

    const runOne = (i: number): void => {
      if (i >= calls.length) {
        next();
        return;
      }
      const call = calls[i];
      const argsPreview = call.arguments.length > 160 ? `${call.arguments.substring(0, 160)}…` : call.arguments;
      const traceLine = `🔧 ${call.name}(${argsPreview})`;
      if (cbs) {
        cbs.onToolTrace(traceLine);
      }
      result.trace.push(traceLine);

      const needConfirm = o.allowConfirm && confirmEnabled &&
        ToolRegistry.isHighRisk(call.name) && !approved.has(call.name);

      const doExecute = (approvedNow: boolean): void => {
        if (!approvedNow) {
          const denied = new LlmMessage();
          denied.role = 'tool';
          denied.toolCallId = call.id;
          denied.content = '用户拒绝执行该工具，请改用其它方式或直接用文字回答。';
          msgs.push(denied);
          runOne(i + 1);
          return;
        }
        ToolRegistry.execute(call.name, call.arguments).then((out: string) => {
          if (cbs) {
            cbs.onToolTrace(`✅ ${call.name} 完成`);
          }
          const truncated = out.length > 6000 ? `${out.substring(0, 6000)}…(已截断)` : out;
          const tm = new LlmMessage();
          tm.role = 'tool';
          tm.toolCallId = call.id;
          tm.content = truncated;
          msgs.push(tm);
          runOne(i + 1);
        }).catch((e: Error) => {
          if (e.message === '__ABORTED__') {
            result.aborted = true;
            return;
          }
          const tm = new LlmMessage();
          tm.role = 'tool';
          tm.toolCallId = call.id;
          tm.content = `工具执行异常: ${e.message}`;
          msgs.push(tm);
          runOne(i + 1);
        });
      };

      if (needConfirm) {
        Gates.requestToolConfirm(call.name, argsPreview).then((ok: boolean) => {
          if (ok) {
            approved.add(call.name); // 本会话同类工具免重复确认
          }
          doExecute(ok);
        }).catch((_e: Error) => {
          result.aborted = true;
        });
      } else {
        doExecute(true);
      }
    };
    runOne(0);
  }

  private static buildParts(text: string, imageUris: string[]): ContentPart[] {
    const parts: ContentPart[] = [];
    const tp = new ContentPart();
    tp.type = 'text';
    tp.text = text;
    parts.push(tp);
    imageUris.forEach((uri: string) => {
      const ip = new ContentPart();
      ip.type = 'image_url';
      // 本地沙箱图片需转 base64 data url（file:// 不可被云服务访问）
      if (uri.startsWith('data:')) {
        ip.imageUrl = uri;
      } else {
        ip.imageUrl = uri; // 由 LlmClient 层处理本地文件
      }
      parts.push(ip);
    });
    return parts;
  }
}
