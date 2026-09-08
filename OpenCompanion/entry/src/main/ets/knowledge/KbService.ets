import { relationalStore } from '@kit.ArkData';
import { retrieval } from '@kit.DataAugmentationKit';
import { AppContext } from '../commons/AppContext';
import { DeviceInfo } from '../commons/DeviceInfo';
import { RdbHelper } from '../commons/RdbHelper';
import { Log } from '../commons/Log';
import { ToolRegistry } from '../core/ToolRegistry';
import { LlmClient, LlmProfiles } from '../core/LlmClient';
import { ToolDef, ToolParam, CancelToken, LlmMessage, LlmProfile, ToolCallReq } from '../core/Types';
import { RagBridge } from './RagBridge';
import { DocImporter } from './DocImporter';

export class KbHit {
  docTitle: string = '';
  content: string = '';
  score: number = 0;
}

export interface AskCallbacks {
  onDelta: (text: string) => void;
  onSources: (hits: KbHit[]) => void;
}

interface ChunkRow {
  title: string;
  content: string;
}

/**
 * 端侧 RAG 知识库（对应命题 F2）：
 * - 数据全部端侧存储（应用私有 RDB kb.db，知识不出设备）
 * - 检索主路径：@kit.DataAugmentationKit retrieval 倒排索引通道（BM25）
 * - 问答主路径：@kit.DataAugmentationKit rag.createRagSession（检索增强 + LLM 融合生成）
 * - 离线模式：localChatModel 端侧问答模型生成，断网可用
 * - 任何一层不可用时自动降级到本地大词表检索，功能不缺席
 */
export class KbService {
  private static store: relationalStore.RdbStore | null = null;
  private static retriever: retrieval.Retriever | null = null;
  private static ftsOk: boolean = false;
  private static chunkCache: ChunkRow[] = [];
  private static inited: boolean = false;

  static kbConfig(): relationalStore.StoreConfig {
    return {
      name: 'kb.db',
      securityLevel: relationalStore.SecurityLevel.S1
    } as relationalStore.StoreConfig;
  }

  static async init(): Promise<void> {
    if (KbService.inited) {
      return;
    }
    KbService.inited = true;
    const ctx = AppContext.getContext();
    KbService.store = await relationalStore.getRdbStore(ctx, KbService.kbConfig());
    const ddls: string[] = [
      `CREATE TABLE IF NOT EXISTS kb_docs (
        id TEXT PRIMARY KEY, kb_id TEXT DEFAULT 'default', title TEXT NOT NULL,
        source TEXT DEFAULT 'import', char_count INTEGER DEFAULT 0, created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS kb_chunks (
        id INTEGER PRIMARY KEY AUTOINCREMENT, doc_id TEXT, kb_id TEXT DEFAULT 'default',
        title TEXT DEFAULT '', content TEXT NOT NULL)`
    ];
    for (const ddl of ddls) {
      await KbService.store.executeSql(ddl);
    }
    // FTS 倒排表（Kit 检索通道的数据载体）
    try {
      await KbService.store.executeSql(
        'CREATE VIRTUAL TABLE IF NOT EXISTS kb_fts USING fts4(content, doc_title, doc_id UNINDEXED)');
      KbService.ftsOk = true;
    } catch (e) {
      Log.warn(`fts4 unavailable, fallback to LIKE search: ${(e as Error).message}`);
      KbService.ftsOk = false;
    }
    // Kit 检索器（倒排索引通道）
    try {
      if (typeof retrieval.getRetriever !== 'function') {
        throw new Error('retrieval module not available on this device');
      }
      const config: retrieval.RetrievalConfig = {
        channelConfigs: [{
          channelType: retrieval.ChannelType.INVERTED_INDEX_DATABASE,
          context: ctx,
          dbConfig: KbService.kbConfig()
        } as retrieval.ChannelConfig]
      };
      KbService.retriever = await retrieval.getRetriever(config);
      Log.info('retrieval kit retriever ready');
    } catch (e) {
      Log.warn(`retrieval kit unavailable: ${(e as Error).message}`);
      KbService.retriever = null;
    }
    await KbService.refreshCache();
    await RagBridge.init();
    KbService.registerTools();
    Log.info(`kb ready (fts=${KbService.ftsOk}, kit=${KbService.retriever !== null}, chunks=${KbService.chunkCache.length})`);
  }

  static isKitReady(): boolean {
    return KbService.retriever !== null;
  }

  static async statusText(): Promise<string> {
    const docs = await KbService.count('kb_docs');
    const chunks = await KbService.count('kb_chunks');
    const offline = AppContext.getBool('kb_offline_mode', false);
    const engine = KbService.retriever !== null && KbService.ftsOk ? '全文检索' : '关键词检索';
    const mode = DeviceInfo.ragGenerateSupported()
      ? (offline ? ' · 离线问答' : '')
      : ` · ${DeviceInfo.label()}端检索模式（仅展示片段）`;
    return `已入库 ${docs} 篇文档 · ${chunks} 个知识片段 · ${engine}${mode}`;
  }

  private static async count(table: string): Promise<number> {
    if (!KbService.store) {
      return 0;
    }
    const rs = await KbService.store.querySql(`SELECT COUNT(*) AS c FROM ${table}`);
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return 0;
    }
    const v = rows[0].get('c');
    return typeof v === 'number' ? v : Number(v);
  }

  private static async refreshCache(): Promise<void> {
    KbService.chunkCache = [];
    if (!KbService.store) {
      return;
    }
    const rs = await KbService.store.querySql('SELECT title, content FROM kb_chunks ORDER BY id ASC');
    const rows = RdbHelper.resultSetToArray(rs);
    rows.forEach((r: Map<string, Object>) => {
      const c: ChunkRow = { title: String(r.get('title')), content: String(r.get('content')) };
      KbService.chunkCache.push(c);
    });
  }

  // ---------------- 入库 ----------------

  static async ingest(kbId: string, title: string, text: string, source: string = 'import'): Promise<number> {
    if (!KbService.store) {
      return 0;
    }
    const docId = `doc_${Date.now()}_${Math.floor(Math.random() * 10000)}`;
    const docRow: Record<string, Object> = {
      'id': docId,
      'kb_id': kbId,
      'title': title,
      'source': source,
      'char_count': text.length,
      'created_at': Date.now()
    };
    await KbService.store.insert('kb_docs', docRow as relationalStore.ValuesBucket);
    // 切片：约 420 字符、重叠 60
    const chunks = KbService.splitChunks(text, 420, 60);
    for (const chunk of chunks) {
      const row: Record<string, Object> = {
        'doc_id': docId,
        'kb_id': kbId,
        'title': title,
        'content': chunk
      };
      const rowId = await KbService.store.insert('kb_chunks', row as relationalStore.ValuesBucket);
      if (KbService.ftsOk) {
        const ftsRow: Record<string, Object> = {
          'content': chunk,
          'doc_title': title,
          'doc_id': `${docId}_${rowId}`
        };
        await KbService.store.insert('kb_fts', ftsRow as relationalStore.ValuesBucket);
      }
    }
    await KbService.refreshCache();
    Log.info(`ingested ${title}: ${chunks.length} chunks`);
    return chunks.length;
  }

  static async listDocs(): Promise<Array<Map<string, Object>>> {
    if (!KbService.store) {
      return [];
    }
    const rs = await KbService.store.querySql('SELECT id, title, source, char_count, created_at FROM kb_docs ORDER BY created_at DESC');
    return RdbHelper.resultSetToArray(rs);
  }

  static async deleteDoc(docId: string): Promise<void> {
    if (!KbService.store) {
      return;
    }
    await KbService.store.executeSql('DELETE FROM kb_docs WHERE id = ?', [docId]);
    await KbService.store.executeSql('DELETE FROM kb_chunks WHERE doc_id = ?', [docId]);
    if (KbService.ftsOk) {
      // fts 表以 doc_id_rowid 前缀匹配删除
      const rs = await KbService.store.querySql("SELECT rowid FROM kb_fts WHERE doc_id LIKE ?", [`${docId}_%`]);
      const rows = RdbHelper.resultSetToArray(rs);
      rows.forEach((r: Map<string, Object>) => {
        const rowid = Number(r.get('rowid'));
        KbService.store?.executeSql('DELETE FROM kb_fts WHERE rowid = ?', [rowid]);
      });
    }
    await KbService.refreshCache();
  }

  static splitChunks(text: string, size: number, overlap: number): string[] {
    const clean = text.replace(/\r/g, '').trim();
    const out: string[] = [];
    if (clean.length <= size) {
      if (clean.length > 0) {
        out.push(clean);
      }
      return out;
    }
    let i = 0;
    while (i < clean.length) {
      let end = Math.min(clean.length, i + size);
      // 尽量在句末断开
      if (end < clean.length) {
        const punct = clean.lastIndexOf('。', end);
        const nl = clean.lastIndexOf('\n', end);
        const cut = Math.max(punct, nl);
        if (cut > i + size / 2) {
          end = cut + 1;
        }
      }
      out.push(clean.substring(i, end).trim());
      if (end >= clean.length) {
        break;
      }
      i = end - overlap;
    }
    return out;
  }

  // ---------------- 检索 ----------------

  /**
   * 多路召回融合检索（对应方案 4.2"向量与倒排并行多路召回并重排"）：
   * 通道① Kit 倒排索引（BM25）与通道② 本地 bigram 重叠并行召回，
   * 以 RRF（倒数排名融合）合并重排——任一通道异常/空结果不影响另一通道。
   */
  static async search(query: string, topK: number = 4): Promise<KbHit[]> {
    if (KbService.chunkCache.length === 0) {
      return [];
    }
    const kitHits = await KbService.kitSearch(query, topK);
    const localHits = KbService.localSearch(query, topK);
    const fused = KbService.fuseRrf([kitHits, localHits], topK);
    if (fused.length === 0) {
      Log.info(`kb search empty (kit=${kitHits.length}, local=${localHits.length})`);
    }
    return fused;
  }

  /** 通道①：DataAugmentationKit 倒排索引检索 */
  private static async kitSearch(query: string, topK: number): Promise<KbHit[]> {
    if (KbService.retriever === null || !KbService.ftsOk) {
      return [];
    }
    try {
      const condition: retrieval.RetrievalCondition = {
        recallConditions: [{
          ftsTableName: 'kb_fts',
          fromClause: 'kb_fts',
          primaryKey: ['rowid'],
          responseColumns: ['content', 'doc_title'],
          recallName: 'kb_recall',
          invertedIndexStrategies: [{ 'bm25Weight': 1.0 } as retrieval.Bm25Strategy]
        } as retrieval.InvertedIndexRecallCondition],
        resultCount: topK
      };
      const res = await KbService.retriever.retrieveRdb(query, condition);
      const hits: KbHit[] = [];
      res.records.forEach((r: retrieval.ItemInfo) => {
        const hit = new KbHit();
        const contentV = r.columns['content'];
        const titleV = r.columns['doc_title'];
        hit.content = typeof contentV === 'string' ? contentV as string : '';
        hit.docTitle = typeof titleV === 'string' ? titleV as string : '';
        hit.score = r.score;
        if (hit.content.length > 0) {
          hits.push(hit);
        }
      });
      return hits;
    } catch (e) {
      const err = e as BusinessError;
      Log.warn(`kit retrieve failed(${err.code}), use local channel: ${err.message}`);
      return [];
    }
  }

  /** RRF 倒数排名融合：score = Σ 1/(60+rank)，内容前缀去重 */
  static fuseRrf(channels: KbHit[][], topK: number): KbHit[] {
    const K = 60;
    const merged: Map<string, KbHit> = new Map<string, KbHit>();
    const scores: Map<string, number> = new Map<string, number>();
    channels.forEach((hits: KbHit[]) => {
      hits.forEach((h: KbHit, rank: number) => {
        const key = h.content.length > 48 ? h.content.substring(0, 48) : h.content;
        const add = 1.0 / (K + rank + 1);
        if (merged.has(key)) {
          scores.set(key, (scores.get(key) ?? 0) + add);
        } else {
          merged.set(key, h);
          scores.set(key, add);
        }
      });
    });
    const keys: string[] = Array.from(merged.keys());
    const ranked: KbHit[] = [];
    keys.forEach((k: string) => {
      const hit = merged.get(k);
      if (hit !== undefined) {
        hit.score = scores.get(k) ?? 0;
        ranked.push(hit);
      }
    });
    ranked.sort((a: KbHit, b: KbHit) => b.score - a.score);
    return ranked.slice(0, topK);
  }

  static localSearch(query: string, topK: number): KbHit[] {
    const grams = KbService.bigrams(query);
    const hits: KbHit[] = [];
    KbService.chunkCache.forEach((c: ChunkRow) => {
      let score = 0;
      grams.forEach((g: string) => {
        let idx = c.content.indexOf(g);
        while (idx >= 0) {
          score++;
          idx = c.content.indexOf(g, idx + 1);
        }
      });
      if (score > 0) {
        const hit = new KbHit();
        hit.docTitle = c.title;
        hit.content = c.content;
        hit.score = score / grams.length;
        hits.push(hit);
      }
    });
    hits.sort((a: KbHit, b: KbHit) => b.score - a.score);
    return hits.slice(0, topK);
  }

  private static bigrams(s: string): string[] {
    const t = s.replace(/[，。！？、\s]/g, '');
    const out: string[] = [];
    for (let i = 0; i < t.length - 1; i++) {
      out.push(t.substring(i, i + 2));
    }
    if (t.length === 1) {
      out.push(t);
    }
    return out;
  }

  // ---------------- 问答 ----------------

  /** RAG 问答（流式）。手机端降级为仅检索；离线模式走端侧模型；否则走 RAG 会话，失败降级为 检索+LLM。 */
  static async askStream(question: string, cb: AskCallbacks, token: CancelToken | null): Promise<string> {
    // 方案 3.3：DataAugmentation Kit 问答不支持手机 → 手机端只做检索 + 展示相关片段，不生成回答
    if (!DeviceInfo.ragGenerateSupported()) {
      return await KbService.searchOnlyAnswer(question, cb);
    }
    const offline = AppContext.getBool('kb_offline_mode', false);
    if (offline) {
      return await KbService.offlineAnswer(question, cb, token);
    }
    try {
      const usedSources = await RagBridge.answer(question, cb, token);
      return usedSources;
    } catch (e) {
      const err = e as Error;
      Log.warn(`rag bridge failed, fallback to search+llm: ${err.message}`);
      return await KbService.searchAndLlmAnswer(question, cb, token);
    }
  }

  /** 手机端降级链（方案 2.2「手机端要么降级成纯检索」）：多路召回 → 直接展示相关片段，不做生成 */
  static async searchOnlyAnswer(question: string, cb: AskCallbacks): Promise<string> {
    const hits = await KbService.search(question, 5);
    cb.onSources(hits);
    if (hits.length === 0) {
      return '知识库中还没有与该问题相关的内容。请先在知识库页导入文档，或换个问法。';
    }
    const head = `手机端仅支持知识库检索（完整问答生成请在 PC / 二合一设备体验），为你找到 ${hits.length} 个相关片段：`;
    return `${head}\n\n${hits.map((h: KbHit, i: number) => `${i + 1}.【${h.docTitle}】${h.content}`).join('\n---\n')}`;
  }

  /** 降级链：本地检索 → 云端 LLM 融合回答 */
  static async searchAndLlmAnswer(question: string, cb: AskCallbacks, token: CancelToken | null): Promise<string> {
    const hits = await KbService.search(question, 4);
    cb.onSources(hits);
    if (hits.length === 0) {
      return '知识库中还没有与该问题相关的内容。请先在知识库页导入文档，或换个问法。';
    }
    const profile = await LlmProfiles.active();
    if (profile === null) {
      // 无 API 时直接返回检索到的原文
      return hits.map((h: KbHit) => `【${h.docTitle}】${h.content}`).join('\n---\n');
    }
    const context = hits.map((h: KbHit, i: number) => `资料${i + 1}（来自《${h.docTitle}》）：${h.content}`).join('\n\n');
    const prompt = `请仅依据以下资料回答问题。若资料不足以回答，请明确说明。\n\n${context}\n\n问题：${question}`;
    return await new Promise<string>((resolve: (v: string) => void) => {
      const runId = `kba_${Date.now()}`;
      if (token !== null) {
        token.onCancel(() => {
          LlmClient.cancel(runId);
        });
      }
      const sys = new LlmMessage();
      sys.role = 'system';
      sys.content = '你是知识库问答助手，依据资料简洁准确地回答。';
      const user = new LlmMessage();
      user.role = 'user';
      user.content = prompt;
      LlmClient.chatStream(profile, [sys, user], null, {
        onThinking: (_d: string) => {
        },
        onContent: (delta: string) => {
          cb.onDelta(delta);
        },
        onToolCalls: (_c: ToolCallReq[]) => {
        },
        onDone: (full: string, _c: ToolCallReq[]) => {
          resolve(full);
        },
        onError: (msg: string) => {
          resolve(`(回答生成失败: ${msg})\n\n检索到的相关资料：\n${context.substring(0, 1200)}`);
        }
      }, runId);
    });
  }

  /** 端侧模型离线回答（localChatModel，全程断网可用） */
  static async offlineAnswer(question: string, cb: AskCallbacks, token: CancelToken | null): Promise<string> {
    const hits = await KbService.search(question, 3);
    cb.onSources(hits);
    const context = hits.map((h: KbHit) => `《${h.docTitle}》:${h.content}`).join('\n');
    const prompt = KbService.trimBytes(`请依据资料回答问题，资料没有的信息回答“资料中未提及”。\n资料：${context}\n问题：${question}`, 4200);
    return await RagBridge.localAnswer(prompt, cb);
  }

  static trimBytes(s: string, maxBytes: number): string {
    const encoded = s.length * 3; // CJK 粗略按 3 字节估算
    if (encoded <= maxBytes) {
      return s;
    }
    const maxChars = Math.floor(maxBytes / 3);
    return s.substring(0, maxChars);
  }

  // ---------------- 工具注册 ----------------

  private static registerTools(): void {
    ToolRegistry.register(new ToolDef('kb_search', '在用户本地知识库中检索相关内容（课程笔记/文献/会议纪要等）', [
      { name: 'query', type: 'string', description: '检索问题或关键词', required: true } as ToolParam,
      { name: 'top_k', type: 'number', description: '返回条数，默认4', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const q = typeof args['query'] === 'string' ? args['query'] as string : '';
      if (q.length === 0) {
        return '缺少 query 参数';
      }
      const hits = await KbService.search(q, 4);
      if (hits.length === 0) {
        return '知识库中没有相关内容';
      }
      return hits.map((h: KbHit, i: number) => `[${i + 1}]《${h.docTitle}》${h.content}`).join('\n---\n');
    });

    ToolRegistry.register(new ToolDef('kb_ingest', '把一段文本存入用户本地知识库', [
      { name: 'title', type: 'string', description: '文档标题', required: true } as ToolParam,
      { name: 'text', type: 'string', description: '完整文本内容', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const title = typeof args['title'] === 'string' ? args['title'] as string : `doc_${Date.now()}`;
      const text = typeof args['text'] === 'string' ? args['text'] as string : '';
      if (text.length < 10) {
        return '文本太短，未入库';
      }
      const n = await KbService.ingest('default', title, text, 'agent');
      return `已入库《${title}》，共 ${n} 个知识切片`;
    });

    ToolRegistry.register(new ToolDef('kb_ask', '基于用户本地知识库回答问题（检索增强问答，带引用来源）', [
      { name: 'question', type: 'string', description: '问题', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const q = typeof args['question'] === 'string' ? args['question'] as string : '';
      if (q.length === 0) {
        return '缺少 question 参数';
      }
      const answer = await KbService.askStream(q, {
        onDelta: (_t: string) => {
        },
        onSources: (_h: KbHit[]) => {
        }
      }, null);
      return answer;
    });

    ToolRegistry.register(new ToolDef('ocr_image', '识别图片中的文字（OCR，支持中英文）', [
      { name: 'imageUri', type: 'string', description: '图片 file:// 或 /data/... 路径', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const uri = typeof args['imageUri'] === 'string' ? args['imageUri'] as string : '';
      if (uri.length === 0) {
        return '缺少 imageUri';
      }
      return await DocImporter.ocrText(uri);
    });

    // 遗忘曲线复习排期（方案 4.5/场景三：学习助手 × 日程管家协作的确定性工具）
    ToolRegistry.register(new ToolDef('create_review_plan',
      '按遗忘曲线（D+1/2/4/7/15 天）为一个主题或知识库最近文档生成 5 个系统级复习提醒（每晚 20:00）', [
      { name: 'topic', type: 'string', description: '复习主题；不填则自动取知识库最近一篇文档的标题', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      let topic = typeof args['topic'] === 'string' ? (args['topic'] as string).trim() : '';
      if (topic.length === 0) {
        const docs = await KbService.listDocs();
        if (docs.length === 0) {
          return '知识库为空：请先在知识库页导入学习资料，或在 topic 参数中指定复习主题';
        }
        topic = String(docs[0].get('title'));
      }
      if (topic.length > 30) {
        topic = topic.substring(0, 30);
      }
      const offsets: number[] = [1, 2, 4, 7, 15];
      const done: string[] = [];
      for (let i = 0; i < offsets.length; i++) {
        const off = offsets[i];
        const d = new Date(Date.now() + off * 86400000);
        try {
          await ToolRegistry.execute('create_reminder', JSON.stringify({
            'title': `📚 复习:${topic}（D+${off}）`,
            'year': d.getFullYear(),
            'month': d.getMonth() + 1,
            'day': d.getDate(),
            'hour': 20,
            'minute': 0,
            'note': `遗忘曲线第${i + 1}次复习：${topic}。建议先自测要点，再回知识库核对原文。`
          }));
          done.push(`D+${off}`);
        } catch (e) {
          Log.warn(`review reminder D+${off} failed: ${(e as Error).message}`);
        }
      }
      return done.length === 0
        ? '复习提醒创建失败（系统提醒通道不可用）'
        : `已为《${topic}》生成 ${done.length} 个遗忘曲线复习提醒（${done.join('、')}，当天 20:00 触发系统通知）`;
    });

    // 文档要点总结（文档处理智能体"总结重点"的确定性入口）
    ToolRegistry.register(new ToolDef('summarize_doc',
      '总结知识库中的一篇文档：检索该文档核心切片并生成结构化要点', [
      { name: 'title', type: 'string', description: '文档标题关键词（模糊匹配；不填取最近一篇）', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const kw = typeof args['title'] === 'string' ? (args['title'] as string).trim() : '';
      const docs = await KbService.listDocs();
      if (docs.length === 0) {
        return '知识库为空，请先导入文档';
      }
      let target: Map<string, Object> | null = null;
      docs.forEach((d: Map<string, Object>) => {
        if (target === null && kw.length > 0 && String(d.get('title')).indexOf(kw) >= 0) {
          target = d;
        }
      });
      if (target === null) {
        target = docs[0];
      }
      const t: Map<string, Object> = target;
      const title = String(t.get('title'));
      const hits = await KbService.search(title, 6);
      const mine = hits.filter((h: KbHit) => h.docTitle === title);
      const use = mine.length > 0 ? mine : hits;
      if (use.length === 0) {
        return `未在知识库中找到《${title}》的内容切片`;
      }
      const context = use.map((h: KbHit, i: number) => `片段${i + 1}：${h.content}`).join('\n');
      const profile = await LlmProfiles.active();
      if (profile === null) {
        return `《${title}》核心内容（未配置 LLM，返回原文要点）：\n${context.substring(0, 1500)}`;
      }
      return await LlmClient.chatOnce(profile,
        '你是文档总结助手。输出：一句话摘要 + 要点列表（每点一行）+ 建议行动。只依据给定材料，简洁准确。',
        `文档：《${title}》\n\n${context.substring(0, 6000)}`, 700);
    });
  }
}
