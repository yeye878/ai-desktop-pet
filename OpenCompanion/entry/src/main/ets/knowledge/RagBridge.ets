import { rag, retrieval, localChatModel } from '@kit.DataAugmentationKit';
import { relationalStore } from '@kit.ArkData';
import { AppContext } from '../commons/AppContext';
import { NetClient, NetStream } from '../commons/NetClient';
import { Log } from '../commons/Log';
import { LlmClient, LlmProfiles } from '../core/LlmClient';
import { CancelToken } from '../core/Types';
import { KbHit, AskCallbacks } from './KbService';

interface StrBox {
  text: string;
}

/**
 * 把自定义 LLM（任意 OpenAI 兼容云端服务）接入鸿蒙 RAG 框架：
 * rag.ChatLLM 由应用实现 streamChat/cancel，RAG 引擎负责「检索 → 拼 prompt → LLM 生成」。
 * 端侧离线链路：localChatModel（系统端侧问答模型）。
 */
class CloudChatLlm extends rag.ChatLLM {
  private seq: number = 1;
  private activeStreams: Map<number, NetStream> = new Map<number, NetStream>();

  streamChat(query: string, callback: (data: rag.LLMStreamAnswer) => void): Promise<rag.LLMRequestInfo> {
    const chatId = CloudChatLlm.baseId() + this.seq++;
    const task = (async (): Promise<rag.LLMRequestInfo> => {
      const profile = await LlmProfiles.active();
      if (profile === null) {
        callback({ isFinished: true, chunk: '' });
        return { chatId: chatId, status: rag.LLMRequestStatus.LLM_REQUEST_ERROR };
      }
      const body: Record<string, Object> = {
        'model': profile.model,
        'messages': [{ 'role': 'user', 'content': query } as Record<string, Object>] as Array<Object>,
        'stream': true,
        'temperature': 0.4
      };
      const box: StrBox = { text: '' };
      return await new Promise<rag.LLMRequestInfo>((resolve: (v: rag.LLMRequestInfo) => void) => {
        const stream = NetClient.streamPost(
          LlmClient.joinUrl(profile.baseUrl, '/chat/completions'),
          LlmClient.authHeaders(profile),
          JSON.stringify(body),
          {
            onChunk: (t: string) => {
              box.text += t;
              CloudChatLlm.drainSse(box, callback);
            },
            onDone: () => {
              callback({ isFinished: true, chunk: '' });
              this.activeStreams.delete(chatId);
            },
            onError: (msg: string) => {
              Log.warn(`rag llm stream error: ${msg}`);
              callback({ isFinished: true, chunk: '' });
              this.activeStreams.delete(chatId);
            }
          });
        this.activeStreams.set(chatId, stream);
        resolve({ chatId: chatId, status: rag.LLMRequestStatus.LLM_SUCCESS });
      });
    })();
    return task;
  }

  cancel(chatId: number): void {
    const s = this.activeStreams.get(chatId);
    if (s) {
      s.abort();
      this.activeStreams.delete(chatId);
    }
  }

  private static baseId(): number {
    return Math.floor(Date.now() % 2147483000);
  }

  /** 极简 SSE 解析（增量正文回传给 RAG 引擎；解析完的行从缓冲中剔除） */
  private static drainSse(box: StrBox, callback: (data: rag.LLMStreamAnswer) => void): void {
    const buffer = box.text;
    let idx = buffer.indexOf('\n');
    let consumed = 0;
    while (idx >= 0) {
      const line = buffer.substring(consumed, idx).replace(/\r$/, '');
      consumed = idx + 1;
      if (line.startsWith('data:')) {
        const payload = line.substring(5).trim();
        if (payload.length > 0 && payload !== '[DONE]') {
          try {
            const obj = JSON.parse(payload) as Record<string, Object>;
            const choices = obj['choices'] as Array<Object> | null;
            if (choices && choices.length > 0) {
              const first = choices[0] as Record<string, Object>;
              const delta = first['delta'] as Record<string, Object> | null;
              const content = delta ? delta['content'] : null;
              if (typeof content === 'string' && (content as string).length > 0) {
                callback({ isFinished: false, chunk: content as string });
              }
            }
          } catch (e) {
            // 半包忽略，等下一个 chunk
          }
        }
      }
      idx = buffer.indexOf('\n', consumed);
    }
    if (consumed > 0) {
      box.text = buffer.substring(consumed);
    }
  }
}

export class RagBridge {
  private static session: rag.RagSession | null = null;
  private static sessionProfileId: string = '';
  static localInited: boolean = false;
  private static questionSeq: number = 1;

  static async init(): Promise<void> {
    // 惰性：首次问答时才创建会话（冷启动优化）
  }

  static kitAvailable(): boolean {
    try {
      return canIUse('SystemCapability.DataAugmentation.RAG');
    } catch (e) {
      return false;
    }
  }

  private static async ensureSession(): Promise<rag.RagSession> {
    const profile = await LlmProfiles.active();
    const profileId = profile !== null ? profile.id : '';
    if (RagBridge.session !== null && RagBridge.sessionProfileId === profileId) {
      return RagBridge.session;
    }
    if (RagBridge.session !== null) {
      try {
        await RagBridge.session.close();
      } catch (e) {
        // ignore
      }
      RagBridge.session = null;
    }
    const ctx = AppContext.getContext();
    const retrievalConfig: retrieval.RetrievalConfig = {
      channelConfigs: [{
        channelType: retrieval.ChannelType.INVERTED_INDEX_DATABASE,
        context: ctx,
        dbConfig: {
          name: 'kb.db',
          securityLevel: relationalStore.SecurityLevel.S1
        } as relationalStore.StoreConfig
      } as retrieval.ChannelConfig]
    };
    const condition: retrieval.RetrievalCondition = {
      recallConditions: [{
        ftsTableName: 'kb_fts',
        fromClause: 'kb_fts',
        primaryKey: ['rowid'],
        responseColumns: ['content', 'doc_title'],
        recallName: 'rag_recall',
        invertedIndexStrategies: [{ 'bm25Weight': 1.0 } as retrieval.Bm25Strategy]
      } as retrieval.InvertedIndexRecallCondition],
      resultCount: 5
    };
    const config: rag.Config = {
      llm: new CloudChatLlm(),
      retrievalConfig: retrievalConfig,
      retrievalCondition: condition
    };
    RagBridge.session = await rag.createRagSession(ctx, config);
    RagBridge.sessionProfileId = profileId;
    Log.info('rag session created');
    return RagBridge.session;
  }

  /**
   * RAG 流式问答：引擎检索知识库 → 拼 prompt → 我们的 ChatLLM 流式生成。
   * 返回使用的来源描述。
   */
  static async answer(question: string, cb: AskCallbacks, token: CancelToken | null): Promise<string> {
    if (!RagBridge.kitAvailable()) {
      throw new Error('SystemCapability.DataAugmentation.RAG 不可用');
    }
    const session = await RagBridge.ensureSession();
    const sources: KbHit[] = [];
    const runConfig: rag.RunConfig = {
      answerTypes: [rag.StreamType.REFERENCE, rag.StreamType.ANSWER]
    };
    return await new Promise<string>((resolve: (v: string) => void, reject: (e: Error) => void) => {
      session.streamRun(question, runConfig, (err: BusinessError<void>, stream: rag.Stream) => {
        if (err) {
          reject(new Error(`rag streamRun error ${err.code}: ${err.message}`));
          return;
        }
        if (stream.type === rag.StreamType.REFERENCE && stream.answer.data) {
          stream.answer.data.forEach((item: retrieval.ItemInfo) => {
            const hit = new KbHit();
            const c = item.columns['content'];
            const t = item.columns['doc_title'];
            hit.content = typeof c === 'string' ? c as string : '';
            hit.docTitle = typeof t === 'string' ? t as string : `来源${sources.length + 1}`;
            hit.score = item.score;
            sources.push(hit);
          });
          cb.onSources(sources);
          return;
        }
        if (stream.type === rag.StreamType.ANSWER) {
          if (stream.answer.chunk.length > 0) {
            cb.onDelta(stream.answer.chunk);
          }
        }
        if (stream.isFinished) {
          const desc = sources.length > 0
            ? `依据知识库：${sources.map((h: KbHit) => `《${h.docTitle}》`).join('、')}`
            : '（未检索到引用来源）';
          resolve(desc);
        }
      }).then((runId: number) => {
        if (token !== null) {
          token.onCancel(() => {
            session.cancel(runId);
          });
        }
      }).catch((e: BusinessError) => {
        reject(new Error(`rag create run failed ${e.code}: ${e.message}`));
      });
    });
  }

  /** 端侧问答模型（离线）。prompt 建议不超过 4500 字节。 */
  static async localAnswer(prompt: string, cb: AskCallbacks): Promise<string> {
    if (!localInited()) {
      const ok = await localChatModel.init();
      RagBridge.localInited = ok;
      if (!ok) {
        throw new Error('端侧问答模型初始化失败（可能设备不支持或模型资源未下载）');
      }
    }
    const questionId = RagBridge.questionSeq++ % 65536;
    return await new Promise<string>((resolve: (v: string) => void, reject: (e: Error) => void) => {
      const info: localChatModel.QuestionInfo = { questionId: questionId, content: prompt };
      const config: localChatModel.Config = { isStream: true };
      let full = '';
      localChatModel.chat(info, config, (err: BusinessError<void>, ans: localChatModel.Answer) => {
        if (err) {
          if (full.length > 0) {
            resolve(full);
            return;
          }
          reject(new Error(`端侧模型错误 ${err.code}: ${err.message}`));
          return;
        }
        if (ans.content.length > 0) {
          full += ans.content;
          cb.onDelta(ans.content);
        }
        if (ans.isFinished) {
          resolve(full);
        }
      });
    });
  }
}

function localInited(): boolean {
  return RagBridge.localInited;
}
