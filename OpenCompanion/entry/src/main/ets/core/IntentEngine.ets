import { bus, Ev } from '../commons/EventBus';
import { PerfTrace } from '../commons/PerfTrace';
import { Log } from '../commons/Log';
import { ToolRegistry } from './ToolRegistry';
import { KbService } from '../knowledge/KbService';
import { AgentRepo } from './AgentRepo';
import { AgentProfile } from './Types';
import { TtsController } from '../voice/TtsController';
import { AppContext } from '../commons/AppContext';

/** 本地直答命中项 */
export class IntentHit {
  action: string = '';
  phrase: string = '';
  corrected: boolean = false;
}

/** 复合指令子任务（带依赖关系，对应方案 4.1.1 子任务序列） */
export class SubTask {
  type: string = '';
  label: string = '';
  payload: string = '';
  dependsOn: number = -1;
}

/** 复合指令解析结果 */
export class CompositePlan {
  matched: boolean = false;
  planName: string = '';
  subtasks: SubTask[] = [];
}

interface PhraseEntry {
  action: string;
  phrases: string[];
}

/**
 * 端侧意图解析引擎（命题 F1 复合指令 + R4 语音≤2s + R5 识别纠偏的交汇点）：
 * ① 指令词表本地直答：高频短语毫秒级路由，不走 LLM（语音指令响应 <500ms 的关键路径）；
 * ② 编辑距离 ≤2 的指令纠偏：识别结果先对齐词表再路由（"打开支识库"→"打开知识库"）；
 * ③ 复合指令拆解：一句话 → 带依赖关系的子任务序列（拍照→识别→总结→入库）。
 * 全部为确定性规则，未命中时返回 null 交还原有 LLM 链路，行为向后兼容。
 */
export class IntentEngine {
  private static readonly PHRASES: PhraseEntry[] = [
    { 'action': 'open_kb', 'phrases': ['打开知识库', '进入知识库', '看看知识库', '我的知识库', '知识库页面'] },
    { 'action': 'open_home', 'phrases': ['回到首页', '打开首页', '看看伙伴', '回到伙伴'] },
    { 'action': 'open_agents', 'phrases': ['打开智能体', '智能体列表', '看看智能体'] },
    { 'action': 'new_chat', 'phrases': ['新对话', '开始新对话', '新建对话', '清空对话'] },
    { 'action': 'today_schedule', 'phrases': ['今天日程', '今日日程', '我的日程', '今天安排', '今天有什么安排', '查看日程'] },
    { 'action': 'now_time', 'phrases': ['现在几点', '几点了', '今天几号', '现在时间', '今天日期'] },
    { 'action': 'kb_status', 'phrases': ['知识库状态', '知识库还有多少', '知识库统计'] },
    { 'action': 'stop_speak', 'phrases': ['停止播报', '别说了', '停止朗读', '安静'] },
    { 'action': 'brief', 'phrases': ['早间简报', '今日简报', '早上简报', '给我简报', '今日概览'] },
    { 'action': 'help', 'phrases': ['你能做什么', '帮助', '使用帮助', '怎么用'] }
  ];

  private static hotCache: string[] = [];
  private static hotCacheAt: number = 0;

  // ---------------- 指令词表路由（含纠偏） ----------------

  /**
   * 尝试本地直答。返回 true 表示已处理（本地回复已推送），调用方直接 return。
   * allowCorrect=false 时只做精确匹配（用于高风险路径保守判断）。
   */
  static async tryLocalAnswer(text: string): Promise<boolean> {
    PerfTrace.mark('voice_cmd_in');
    const hit = IntentEngine.route(text);
    if (hit === null) {
      return false;
    }
    Log.info(`intent hit: ${hit.action}${hit.corrected ? ' (corrected)' : ''}`);
    const reply = await IntentEngine.executeAction(hit.action);
    const cost = Date.now() - PerfTrace.lastStamp('voice_cmd_in');
    PerfTrace.measure(`本地直答·${hit.action}`, 'voice_cmd_in');
    bus.emit(Ev.CHAT_CHANGED);
    IntentEngine.pushReply(`${reply}\n\n—— 本地指令直达，耗时 ${cost}ms，未调用大模型`);
    return true;
  }

  /** 路由：精确 → 词表纠偏（编辑距离≤2 且长度≥4 才容错，防误拦截） */
  static route(text: string): IntentHit | null {
    const norm = IntentEngine.normalize(text);
    if (norm.length < 2 || norm.length > 24) {
      return null;
    }
    for (let i = 0; i < IntentEngine.PHRASES.length; i++) {
      const entry = IntentEngine.PHRASES[i];
      for (let j = 0; j < entry.phrases.length; j++) {
        if (norm === entry.phrases[j]) {
          const hit = new IntentHit();
          hit.action = entry.action;
          hit.phrase = entry.phrases[j];
          return hit;
        }
      }
    }
    // 纠偏：仅当识别结果 ≥3 字时启用（短句误纠率过高）
    if (norm.length >= 3) {
      // 前缀补全：识别残缺（如断句截断"今天日"）时对齐到词表短语
      if (norm.length >= 3) {
        let prefixHit: IntentHit | null = null;
        for (let i = 0; i < IntentEngine.PHRASES.length && prefixHit === null; i++) {
          const entry = IntentEngine.PHRASES[i];
          for (let j = 0; j < entry.phrases.length; j++) {
            const p = entry.phrases[j];
            if (p.length > norm.length && p.startsWith(norm) && p.length - norm.length <= 2) {
              const hit = new IntentHit();
              hit.action = entry.action;
              hit.phrase = p;
              hit.corrected = true;
              prefixHit = hit;
              break;
            }
          }
        }
        if (prefixHit !== null) {
          return prefixHit;
        }
      }
      // 编辑距离纠偏（专有词误识）：距离 ≤2 且不超过词长 1/3
      if (norm.length >= 4) {
        let best: IntentHit | null = null;
        let bestDist = 3;
        for (let i = 0; i < IntentEngine.PHRASES.length; i++) {
          const entry = IntentEngine.PHRASES[i];
          for (let j = 0; j < entry.phrases.length; j++) {
            const p = entry.phrases[j];
            if (Math.abs(p.length - norm.length) > 2) {
              continue;
            }
            const d = IntentEngine.editDistance(norm, p);
            if (d < bestDist && d <= 2 && d <= Math.floor(p.length / 3)) {
              bestDist = d;
              const hit = new IntentHit();
              hit.action = entry.action;
              hit.phrase = p;
              hit.corrected = true;
              best = hit;
            }
          }
        }
        return best;
      }
    }
    return null;
  }

  // ---------------- 复合指令拆解 ----------------

  /**
   * 把一句话拆为带依赖的子任务序列。当前内置三类命题场景模式：
   * 文档总结（拍→识别→总结[→入库]）、复习计划（依据知识库排期）、先查后答（知识库检索问答）。
   */
  static parseComposite(text: string, hasImage: boolean): CompositePlan {
    const plan = new CompositePlan();
    const t = IntentEngine.normalize(text);
    if (t.length === 0) {
      return plan;
    }
    const wantImageDoc = IntentEngine.anyOf(t, ['拍一下', '拍这个', '拍这份', '拍这个文档', '看这个文档', '看一下这个文档',
      '看看这个文档', '这个文档', '这份文档', '这份讲义', '这个讲义', '拍张照', '拍照']);
    const wantSummary = IntentEngine.anyOf(t, ['总结', '重点', '要点', '概括', '摘要', '划重点', '归纳']);
    const wantIngest = IntentEngine.anyOf(t, ['入库', '存入知识库', '存到知识库', '放进知识库', '保存到知识库']);
    const wantReview = IntentEngine.anyOf(t, ['复习计划', '安排复习', '排复习', '复习安排', '帮我复习']);
    const wantAskKb = IntentEngine.anyOf(t, ['知识库里', '我的笔记', '查我的', '根据我的资料', '根据知识库']);

    if (wantImageDoc && wantSummary) {
      plan.matched = true;
      plan.planName = '文档重点总结';
      if (!hasImage) {
        const s1 = new SubTask();
        s1.type = 'request_image';
        s1.label = '获取文档照片';
        plan.subtasks.push(s1);
        const s2 = new SubTask();
        s2.type = 'ocr';
        s2.label = '识别文档文字';
        s2.dependsOn = 0;
        plan.subtasks.push(s2);
      } else {
        const s2 = new SubTask();
        s2.type = 'ocr';
        s2.label = '识别文档文字';
        plan.subtasks.push(s2);
      }
      const s3 = new SubTask();
      s3.type = 'summarize';
      s3.label = '生成结构化重点';
      s3.dependsOn = plan.subtasks.length - 1;
      plan.subtasks.push(s3);
      if (wantIngest) {
        const s4 = new SubTask();
        s4.type = 'kb_save_summary';
        s4.label = '总结存入知识库';
        s4.dependsOn = plan.subtasks.length - 1;
        plan.subtasks.push(s4);
      }
      return plan;
    }
    if (wantIngest && wantSummary && !wantImageDoc) {
      // 已有文字内容的"总结+入库"
      plan.matched = true;
      plan.planName = '总结并入库';
      const s1 = new SubTask();
      s1.type = 'summarize_text';
      s1.label = '总结给定内容';
      plan.subtasks.push(s1);
      const s2 = new SubTask();
      s2.type = 'kb_save_summary';
      s2.label = '总结存入知识库';
      s2.dependsOn = 0;
      plan.subtasks.push(s2);
      return plan;
    }
    if (wantReview) {
      plan.matched = true;
      plan.planName = '遗忘曲线复习排期';
      const s1 = new SubTask();
      s1.type = 'plan_review';
      s1.label = '按遗忘曲线生成复习提醒';
      plan.subtasks.push(s1);
      return plan;
    }
    if (wantAskKb) {
      plan.matched = true;
      plan.planName = '知识库溯源问答';
      const s1 = new SubTask();
      s1.type = 'kb_ask';
      s1.label = '检索个人知识库并回答';
      plan.subtasks.push(s1);
      return plan;
    }
    return plan;
  }

  // ---------------- 动态热词（R5） ----------------

  /**
   * 动态热词表 = 用户设置热词 ∪ 内置智能体名 ∪ 知识库文档标题（截断上限 200 词）。
   * 知识库变更后调用 invalidateHotwords() 触发重建。
   */
  static async dynamicHotwords(): Promise<string[]> {
    if (Date.now() - IntentEngine.hotCacheAt < 30000 && IntentEngine.hotCache.length > 0) {
      return IntentEngine.hotCache;
    }
    const out: string[] = [];
    const pushWord = (w: string) => {
      const t = w.trim();
      if (t.length >= 2 && t.length <= 20 && out.length < 200 && out.indexOf(t) < 0) {
        out.push(t);
      }
    };
    const raw = AppContext.getSetting('asr_hot_words', '打开知识库,新对话,今天日程,帮我总结,存入知识库,组队,学习助手,日程管家,文档处理');
    raw.split(/[,，]/).forEach((w: string) => {
      pushWord(w);
    });
    IntentEngine.PHRASES.forEach((p: PhraseEntry) => {
      p.phrases.forEach((w: string) => {
        pushWord(w);
      });
    });
    try {
      const agents = await AgentRepo.list();
      agents.forEach((a: AgentProfile) => {
        pushWord(a.name);
      });
    } catch (e) {
      // 仓库未就绪时跳过（热词属增益项，不阻断）
    }
    try {
      const docs = await KbService.listDocs();
      docs.slice(0, 40).forEach((d: Map<string, Object>) => {
        const title = String(d.get('title'));
        if (title.length >= 2 && title.length <= 20) {
          pushWord(title);
        } else if (title.length > 20) {
          pushWord(title.substring(0, 20));
        }
      });
    } catch (e) {
      // ignore
    }
    IntentEngine.hotCache = out;
    IntentEngine.hotCacheAt = Date.now();
    return out;
  }

  static invalidateHotwords(): void {
    IntentEngine.hotCacheAt = 0;
    IntentEngine.hotCache = [];
  }

  // ---------------- 直答动作执行 ----------------

  static async executeAction(action: string): Promise<string> {
    if (action === 'open_kb' || action === 'open_home' || action === 'open_agents') {
      const page = action === 'open_kb' ? 'kb' : (action === 'open_home' ? 'home' : 'agents');
      bus.emit(Ev.NAVIGATE, { 'page': page } as Record<string, string>);
      return action === 'open_kb' ? '已打开知识库 📚' : (action === 'open_home' ? '回到伙伴首页 🐹' : '已打开智能体列表 🤖');
    }
    if (action === 'new_chat') {
      await ChatServiceRef.newConversation();
      return '已开启新对话 ✨';
    }
    if (action === 'stop_speak') {
      TtsController.stop();
      return '已停止播报 🤫';
    }
    if (action === 'now_time') {
      const r = await ToolRegistry.execute('get_current_time', '{}');
      return `🕒 ${r}`;
    }
    if (action === 'today_schedule') {
      const r = await ToolRegistry.execute('list_reminders', '{}');
      return `📅 今日与未来的安排：\n${r}`;
    }
    if (action === 'kb_status') {
      return await KbService.statusText();
    }
    if (action === 'brief') {
      return await IntentEngine.buildBrief();
    }
    return '可用指令：打开知识库 / 新对话 / 今天日程 / 现在几点 / 早间简报 / 知识库状态 / 停止播报；' +
      '复合指令如「拍一下这个文档，帮我总结重点」。';
  }

  /** 早间简报：时间 + 天气 + 待办三段本地拼装（无 LLM 也可用，对应方案 4.5） */
  static async buildBrief(): Promise<string> {
    const parts: string[] = ['🌅 今日简报'];
    try {
      const t = await ToolRegistry.execute('get_current_time', '{}');
      parts.push(`⏰ ${t}`);
    } catch (e) {
      // ignore
    }
    try {
      const w = await ToolRegistry.execute('get_weather', '{}');
      parts.push(`🌤 ${w}`);
    } catch (e) {
      parts.push('🌤 天气暂不可用');
    }
    try {
      const r = await ToolRegistry.execute('list_reminders', '{}');
      parts.push(`📅 待办：\n${r}`);
    } catch (e) {
      // ignore
    }
    try {
      const docs = await KbService.listDocs();
      if (docs.length > 0) {
        parts.push(`📚 知识库已有 ${docs.length} 篇文档，随时可以提问`);
      }
    } catch (e) {
      // ignore
    }
    return parts.join('\n\n');
  }

  // ---------------- 工具函数 ----------------

  static normalize(text: string): string {
    return text.replace(/[，。！？、,.!?\s：:；;~～·“”‘’"']/g, '').trim();
  }

  static anyOf(t: string, words: string[]): boolean {
    for (let i = 0; i < words.length; i++) {
      if (t.indexOf(words[i]) >= 0) {
        return true;
      }
    }
    return false;
  }

  /** 经典 DP 编辑距离（输入为短句，开销可忽略） */
  static editDistance(a: string, b: string): number {
    const m = a.length;
    const n = b.length;
    if (m === 0) {
      return n;
    }
    if (n === 0) {
      return m;
    }
    let prev: number[] = [];
    for (let j = 0; j <= n; j++) {
      prev.push(j);
    }
    for (let i = 1; i <= m; i++) {
      const cur: number[] = [i];
      for (let j = 1; j <= n; j++) {
        const cost = a.charAt(i - 1) === b.charAt(j - 1) ? 0 : 1;
        const ins = prev[j] + 1;
        const del = cur[j - 1] + 1;
        const rep = prev[j - 1] + cost;
        cur.push(Math.min(ins, del, rep));
      }
      prev = cur;
    }
    return prev[n];
  }

  static pushReply(text: string): void {
    // 延迟 import 会造成循环依赖，这里经 ChatServiceRef 注入（见文件尾）
    ChatServiceRef.pushLocal(text);
  }
}

/** 智能体名最小接口（避免与 AgentProfile 类型产生编译耦合） */
interface AgentProfileLike {
  name: string;
}

/**
 * ChatService 注入口：IntentEngine 属于 core 低层，直接 import ChatService 会形成
 * ChatService→IntentEngine→ChatService 循环依赖（ArkTS 运行期表现为字段 undefined），
 * 由 ChatService 在 init 时调用 ChatServiceRef.bind 完成注册。
 */
export class ChatServiceRef {
  static newConversationFn: (() => Promise<void>) | null = null;
  static pushLocalFn: ((text: string) => void) | null = null;

  static bind(pushLocal: (text: string) => void, newConversation: () => Promise<void>): void {
    ChatServiceRef.pushLocalFn = pushLocal;
    ChatServiceRef.newConversationFn = newConversation;
  }

  static pushLocal(text: string): void {
    if (ChatServiceRef.pushLocalFn !== null) {
      ChatServiceRef.pushLocalFn(text);
    }
  }

  static newConversation(): Promise<void> {
    if (ChatServiceRef.newConversationFn !== null) {
      return ChatServiceRef.newConversationFn();
    }
    return Promise.resolve();
  }
}
