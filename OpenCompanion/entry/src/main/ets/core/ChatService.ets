import fs from '@ohos.file.fs';
import { util } from '@kit.ArkTS';
import { RdbHelper } from '../commons/RdbHelper';
import { AppContext, SettingKey } from '../commons/AppContext';
import { PerfTrace } from '../commons/PerfTrace';
import { bus, Ev, BusEvent } from '../commons/EventBus';
import { Log } from '../commons/Log';
import { LlmClient, LlmProfiles } from './LlmClient';
import { AgentLoop, LoopOptions, LoopCallbacks } from './AgentLoop';
import { TeamOrchestrator } from './TeamOrchestrator';
import { AgentRepo } from './AgentRepo';
import { EvolutionManager } from './EvolutionManager';
import { ToolRegistry } from './ToolRegistry';
import { Gates } from './Gates';
import { IntentEngine, ChatServiceRef, CompositePlan, SubTask } from './IntentEngine';
import { KbService, KbHit } from '../knowledge/KbService';
import { DocImporter } from '../knowledge/DocImporter';
import { TtsController } from '../voice/TtsController';
import { AgentProfile, CancelToken, ChatMessage, ContentPart, JsonUtil, LlmMessage, SkillProfile, ToolCallReq } from './Types';

interface ChatSnapshot {
  messages: ChatMessage[];
  busy: boolean;
}

/**
 * 对话编排服务（对应桌面版 send_to_ai 全链路：落库 → @提及/斜杠/组队分流 →
 * system prompt 组装 → AgentLoop 流式 → 事件回推 → TTS 播报）
 */
export class ChatService {
  static messages: ChatMessage[] = [];
  static busy: boolean = false;
  private static token: CancelToken | null = null;
  private static lastEmitAt: number = 0;
  private static ttsBuf: string = '';
  private static firstFeedbackDone: boolean = false;

  static async init(): Promise<void> {
    // IntentEngine 属低层模块，经注入回调使用 ChatService 能力（避免循环依赖）
    ChatServiceRef.bind(ChatService.pushLocalAssistantMessage, ChatService.newConversation);
    // start_new_conversation 工具（LLM 经 API 工具调用触发）经事件总线转入，避免 ToolRegistry→ChatService 循环依赖
    bus.on(Ev.NEW_CONVERSATION, (_e: BusEvent) => {
      ChatService.newConversation();
    });
    await AgentRepo.ensureBuiltins();
    await ChatService.loadHistory();
    await KbService.init();
    Log.info(`chat service ready, ${ChatService.messages.length} history messages`);
  }

  static async loadHistory(): Promise<void> {
    const rs = await RdbHelper.query('SELECT * FROM chat_history ORDER BY created_at ASC LIMIT 200');
    const rows = RdbHelper.resultSetToArray(rs);
    ChatService.messages = [];
    rows.forEach((r: Map<string, Object>) => {
      const m = new ChatMessage();
      m.id = String(r.get('id'));
      m.role = String(r.get('role'));
      m.content = String(r.get('content'));
      m.thinking = String(r.get('thinking'));
      m.agentName = String(r.get('agent_name'));
      m.agentAvatar = String(r.get('agent_avatar'));
      m.toolEvents = JsonUtil.getStringArray({ 't': String(r.get('tool_events')) }, 't');
      m.imageUris = JsonUtil.getStringArray({ 't': String(r.get('image_uris')) }, 't');
      m.quote = String(r.get('quote'));
      m.createdAt = Number(r.get('created_at'));
      ChatService.messages.push(m);
    });
  }

  static snapshot(): ChatSnapshot {
    const out: ChatSnapshot = { messages: ChatService.messages, busy: ChatService.busy };
    return out;
  }

  static emitChatChanged(force: boolean = false): void {
    const now = Date.now();
    if (!force && now - ChatService.lastEmitAt < 60) {
      return;
    }
    ChatService.lastEmitAt = now;
    bus.emit(Ev.CHAT_CHANGED, ChatService.snapshot());
  }

  // ---------------- 发送主链路 ----------------

  static async send(text: string, imageUris: string[], quote: string = ''): Promise<void> {
    if (ChatService.busy) {
      bus.emit(Ev.AI_ERROR, { 'message': '上一条回复还在进行中，请稍候或点击停止' } as Record<string, string>);
      return;
    }
    let clean = text.trim();
    if (clean.length === 0 && imageUris.length === 0) {
      return;
    }

    // 斜杠命令（本地执行，不进 LLM）
    if (clean.startsWith('/')) {
      const handled = await ChatService.handleSlash(clean);
      if (handled) {
        return;
      }
    }

    // @提及解析
    const mentioned: AgentProfile[] = [];
    let rest = clean;
    while (true) {
      const at = rest.indexOf('@');
      if (at < 0) {
        break;
      }
      const spaceAt = rest.indexOf(' ', at);
      const name = spaceAt < 0 ? rest.substring(at + 1) : rest.substring(at + 1, spaceAt);
      const all = await AgentRepo.list();
      let found: AgentProfile | null = null;
      all.forEach((a: AgentProfile) => {
        if (found === null && a.name === name) {
          found = a;
        }
      });
      if (found === null) {
        rest = rest.substring(0, at) + rest.substring(at + 1 + name.length);
        break;
      }
      mentioned.push(found);
      rest = (rest.substring(0, at) + rest.substring(at + 1 + name.length)).trim();
      break; // 只取第一个 @ 即可，多个用 /team
    }

    // 图片转 base64 data url（云模型无法读取本地沙箱路径）
    const imageDataUrls: string[] = [];
    for (const uri of imageUris) {
      const dataUrl = await ChatService.fileToDataUrl(uri);
      if (dataUrl !== null) {
        imageDataUrls.push(dataUrl);
      }
    }

    // 用户消息落库
    const userMsg = new ChatMessage();
    userMsg.id = ChatMessage.newId();
    userMsg.role = 'user';
    userMsg.content = rest.trim().length > 0 ? rest.trim() : '（图片）';
    userMsg.imageUris = imageUris;
    userMsg.quote = quote;
    userMsg.createdAt = Date.now();
    ChatService.messages.push(userMsg);
    await ChatService.persist(userMsg);
    EvolutionManager.onUserMessage(userMsg.content);
    ChatService.emitChatChanged(true);

    // ① 端侧意图路由：仅在未配置 LLM API 时作为离线兜底本地直答；
    //    已配置 API 时全部交给大模型，由它经工具调用完成页面跳转/新对话/停止播报等操作
    if (imageDataUrls.length === 0 && imageUris.length === 0) {
      const activeProfile = await LlmProfiles.active();
      if (activeProfile === null) {
        const handled = await IntentEngine.tryLocalAnswer(rest);
        if (handled) {
          return;
        }
      }
    }

    // 派发
    ChatService.busy = true;
    ChatService.token = new CancelToken();
    ChatService.ttsBuf = '';
    ChatService.firstFeedbackDone = false;
    PerfTrace.mark('cmd_in');
    ChatService.emitChatChanged(true);
    bus.emit(Ev.AI_STARTED);
    const t0 = Date.now();

    const assistant = new ChatMessage();
    assistant.id = ChatMessage.newId();
    assistant.role = 'assistant';
    assistant.createdAt = Date.now();
    ChatService.messages.push(assistant);

    const cbs: LoopCallbacks = {
      onThinking: (d: string) => {
        assistant.thinking += d;
        ChatService.emitChatChanged();
      },
      onContent: (d: string) => {
        assistant.content += d;
        ChatService.noteFirstFeedback('llm首字');
        ChatService.speakStreamDelta(d);
        ChatService.emitChatChanged();
      },
      onToolTrace: (line: string) => {
        assistant.toolEvents.push(line);
        ChatService.emitChatChanged(true);
      }
    };

    try {
      // ② 复合指令管线：拆解为带依赖的子任务序列顺序执行（命题 F1）
      const plan = IntentEngine.parseComposite(rest, imageUris.length > 0);
      if (plan.matched && plan.subtasks.length > 0) {
        assistant.agentName = '任务规划';
        assistant.agentAvatar = '🧭';
        await ChatService.runCompositePlan(plan, rest.trim(), imageUris, imageDataUrls, userMsg, assistant, cbs);
      } else if (TeamOrchestrator.isTeamCommand(rest) || mentioned.length >= 2) {
        let task = rest.trim();
        if (TeamOrchestrator.isTeamCommand(task)) {
          task = task.substring(task.indexOf(' ') >= 0 ? task.indexOf(' ') : task.length).trim();
        }
        assistant.agentName = '智能体团队';
        assistant.agentAvatar = '👥';
        const final = await TeamOrchestrator.run(task, mentioned, ChatService.token, (line: string) => {
          assistant.toolEvents.push(line);
          ChatService.emitChatChanged(true);
        }, (d: string) => {
          assistant.thinking += d;
          ChatService.emitChatChanged();
        }, (d: string) => {
          assistant.content += d;
          ChatService.noteFirstFeedback('团队汇报首字');
          ChatService.speakStreamDelta(d);
          ChatService.emitChatChanged();
        });
        assistant.content = assistant.content.length > 0 ? assistant.content : final;
      } else {
        const agent = mentioned.length === 1 ? mentioned[0] : null;
        await ChatService.runSingle(agent, rest.trim(), imageDataUrls, userMsg, assistant, cbs);
      }
    } catch (e) {
      const err = e as Error;
      if (err.message === '__ABORTED__') {
        assistant.content += '\n(已中断)';
      } else {
        Log.error(`send failed: ${err.message}`);
        bus.emit(Ev.AI_ERROR, { 'message': err.message } as Record<string, string>);
        if (assistant.content.length === 0) {
          assistant.content = `出错了：${err.message}`;
        }
      }
    } finally {
      ChatService.busy = false;
      ChatService.token = null;
      await ChatService.persist(assistant);
      ChatService.emitChatChanged(true);
      // 流式播报收尾：剩余不足一句的尾巴立即播（首句已在生成过程中播过，对应"边生成边合成边播放"）
      ChatService.flushTts(true);
      ChatService.ttsBuf = '';
      bus.emit(Ev.AI_FINISHED, {
        'agentName': assistant.agentName,
        'agentAvatar': assistant.agentAvatar,
        'elapsedMs': `${Date.now() - t0}`
      } as Record<string, string>);
    }
  }

  /** 首个语义反馈打点（口径：用户指令提交 → 界面出现首个语义内容） */
  private static noteFirstFeedback(label: string): void {
    if (!ChatService.firstFeedbackDone) {
      ChatService.firstFeedbackDone = true;
      PerfTrace.measure(`指令→首反馈(${label})`, 'cmd_in');
    }
  }

  /** 流式回答按句缓冲即时播报（对应方案 7.2"首字优先"：首个完整句即刻合成，不等全文） */
  private static speakStreamDelta(delta: string): void {
    if (!AppContext.getBool(SettingKey.VOICE_AUTO_SPEAK, false)) {
      return;
    }
    ChatService.ttsBuf += delta;
    ChatService.flushTts(false);
  }

  private static flushTts(force: boolean): void {
    const buf = ChatService.ttsBuf;
    if (buf.length === 0) {
      return;
    }
    let cut = -1;
    for (let i = buf.length - 1; i >= 0; i--) {
      const ch = buf.charAt(i);
      if (ch === '。' || ch === '！' || ch === '？' || ch === '；' || ch === '\n') {
        cut = i + 1;
        break;
      }
    }
    if (cut >= 12) {
      const part = buf.substring(0, cut);
      ChatService.ttsBuf = buf.substring(cut);
      TtsController.speak(part);
    } else if (force) {
      TtsController.speak(buf);
      ChatService.ttsBuf = '';
    }
  }

  // ---------------- 复合指令管线 ----------------

  /**
   * 顺序执行带依赖的子任务序列（当前拆解模式均为线性依赖，逐个执行并把
   * 上游产物向下游传递；每个子任务的开始/完成写入消息轨迹，执行全程可视）。
   */
  private static async runCompositePlan(plan: CompositePlan, rawText: string, imageUris: string[],
    imageDataUrls: string[], userMsg: ChatMessage, assistant: ChatMessage, cbs: LoopCallbacks): Promise<void> {
    ChatService.pushTraceLine(assistant, `🧭 已拆解复合指令「${plan.planName}」→ ${plan.subtasks.length} 个子任务`);
    plan.subtasks.forEach((st: SubTask, i: number) => {
      const dep = st.dependsOn >= 0 ? `（依赖步骤${st.dependsOn + 1}）` : '';
      ChatService.pushTraceLine(assistant, `　${i + 1}. ${st.label}${dep}`);
    });

    let ocrText = '';
    let summary = '';
    for (let i = 0; i < plan.subtasks.length; i++) {
      const st = plan.subtasks[i];
      if (ChatService.token !== null && ChatService.token.cancelled) {
        throw new Error('__ABORTED__');
      }
      if (st.type === 'request_image') {
        ChatService.pushTraceLine(assistant, '📷 需要文档照片：已为你打开相册');
        bus.emit(Ev.REQUEST_IMAGE);
        assistant.content = '已为你打开相册 📷：选好文档照片后再次发送，我会接着完成识别与总结。';
        ChatService.pushTraceLine(assistant, '⏸ 后续步骤等照片就位后继续');
        break;
      }
      if (st.type === 'ocr') {
        ChatService.pushTraceLine(assistant, `🔍 开始${st.label}…`);
        const parts: string[] = [];
        for (const uri of imageUris) {
          try {
            const t = await DocImporter.ocrText(uri);
            if (t.trim().length > 0) {
              parts.push(t.trim());
            }
          } catch (e) {
            ChatService.pushTraceLine(assistant, `⚠️ 一张图片识别失败: ${(e as Error).message}`);
          }
        }
        ocrText = parts.join('\n\n');
        if (ocrText.trim().length < 10) {
          // OCR 无文字（如手绘/风景图）→ 降级为视觉 LLM 理解
          ChatService.pushTraceLine(assistant, 'ℹ️ 未识别到文字，转用视觉模型理解图片');
          await ChatService.runSingle(null, rawText, imageDataUrls, userMsg, assistant, cbs);
          return;
        }
        ChatService.pushTraceLine(assistant, `✔ ${st.label}完成（${ocrText.length} 字）`);
        continue;
      }
      if (st.type === 'summarize' || st.type === 'summarize_text') {
        ChatService.pushTraceLine(assistant, `📝 开始${st.label}…`);
        const material = st.type === 'summarize' ? ocrText : rawText;
        summary = await ChatService.llmSummarize(material, cbs);
        if (summary.length === 0) {
          summary = material.length > 800 ? material.substring(0, 800) : material;
          ChatService.pushTraceLine(assistant, 'ℹ️ 未配置 LLM，直接以原文前段作为总结');
        }
        ChatService.noteFirstFeedback('总结首字');
        ChatService.pushTraceLine(assistant, `✔ ${st.label}完成（${summary.length} 字）`);
        continue;
      }
      if (st.type === 'kb_save_summary') {
        const title = `总结_${plan.planName}_${DocImporter.timeTag()}`;
        const n = await KbService.ingest('default', title, summary, 'composite');
        IntentEngine.invalidateHotwords();
        ChatService.pushTraceLine(assistant, `💾 已存入知识库《${title}》（${n} 个切片）`);
        continue;
      }
      if (st.type === 'plan_review') {
        ChatService.pushTraceLine(assistant, `🗓 开始${st.label}…`);
        const topic = rawText.replace(/安排复习|帮我复习|复习计划|排复习|复习安排/g, '').trim();
        const r = await ToolRegistry.execute('create_review_plan',
          JSON.stringify({ 'topic': topic.length > 0 ? topic : '知识库重点' }));
        ChatService.pushTraceLine(assistant, `✔ ${r}`);
        continue;
      }
      if (st.type === 'kb_ask') {
        ChatService.pushTraceLine(assistant, `📚 开始${st.label}…`);
        const desc = await KbService.askStream(rawText, {
          onDelta: (d: string) => {
            assistant.content += d;
            ChatService.noteFirstFeedback('知识库问答首字');
            ChatService.speakStreamDelta(d);
            ChatService.emitChatChanged();
          },
          onSources: (hits: KbHit[]) => {
            if (hits.length > 0) {
              ChatService.pushTraceLine(assistant, `📌 命中来源：${hits.map((h: KbHit) => `《${h.docTitle}》`).join('、')}`);
            }
          }
        }, ChatService.token);
        if (assistant.content.length === 0) {
          assistant.content = desc;
        }
        ChatService.pushTraceLine(assistant, `✔ ${st.label}完成`);
        continue;
      }
    }
    PerfTrace.measure('复合指令全链路', 'cmd_in');
  }

  /** LLM 结构化总结（流式回显；未配置 API 返回空串） */
  private static async llmSummarize(material: string, cbs: LoopCallbacks): Promise<string> {
    const profile = await LlmProfiles.active();
    if (profile === null) {
      return '';
    }
    const capped = material.length > 6000 ? material.substring(0, 6000) : material;
    const sys = new LlmMessage();
    sys.role = 'system';
    sys.content = '你是文档总结助手。输出结构化重点：核心主题一句话 + 分点要点（每点一行）+ 建议行动。简洁准确，不编造。';
    const user = new LlmMessage();
    user.role = 'user';
    user.content = `请总结以下内容的核心要点：\n\n${capped}`;
    return await new Promise<string>((resolve: (v: string) => void) => {
      const runId = `sum_${Date.now()}`;
      if (ChatService.token !== null) {
        ChatService.token.onCancel(() => {
          LlmClient.cancel(runId);
        });
      }
      let full = '';
      LlmClient.chatStream(profile, [sys, user], null, {
        onThinking: (_d: string) => {
        },
        onContent: (d: string) => {
          full += d;
          cbs.onContent(d);
        },
        onToolCalls: (_c: ToolCallReq[]) => {
        },
        onDone: (_full: string, _c: ToolCallReq[]) => {
          resolve(full);
        },
        onError: (msg: string) => {
          Log.warn(`summarize failed: ${msg}`);
          resolve(full);
        }
      }, runId);
    });
  }

  private static pushTraceLine(assistant: ChatMessage, line: string): void {
    assistant.toolEvents.push(line);
    ChatService.emitChatChanged(true);
  }

  /** 单智能体/默认助手执行 */
  private static async runSingle(agent: AgentProfile | null, userText: string,
    imageDataUrls: string[], userMsg: ChatMessage, assistant: ChatMessage, cbs: LoopCallbacks): Promise<void> {
    const profile = await LlmProfiles.active();
    const opts = new LoopOptions();
    opts.profile = profile;
    opts.history = ChatService.historyForPrompt(userMsg);
    opts.userText = userText;
    opts.imageUris = imageDataUrls;
    opts.token = ChatService.token !== null ? ChatService.token : new CancelToken();
    opts.callbacks = cbs;
    opts.systemPrompt = await ChatService.buildSystemPrompt(agent);
    if (agent !== null) {
      assistant.agentName = agent.name;
      assistant.agentAvatar = agent.avatar;
      opts.tools = ToolRegistry.defsFor(agent.allowedTools);
    } else {
      const skill = await AgentRepo.activeSkill();
      if (skill !== null && skill.allowedTools.length > 0) {
        opts.tools = ToolRegistry.defsFor(skill.allowedTools);
      } else {
        opts.tools = ToolRegistry.all();
      }
    }
    const result = await AgentLoop.run(opts);
    if (result.content.length > 0) {
      assistant.content = assistant.content.length > 0 ? assistant.content : result.content;
    }
    result.trace.forEach((t: string) => {
      if (assistant.toolEvents.indexOf(t) < 0) {
        assistant.toolEvents.push(t);
      }
    });
  }

  private static historyForPrompt(current: ChatMessage): ChatMessage[] {
    const out: ChatMessage[] = [];
    ChatService.messages.forEach((m: ChatMessage) => {
      if (m.id !== current.id && m.content.length > 0 && m.toolEvents.length === 0) {
        out.push(m);
      }
    });
    return out.slice(-20);
  }

  /** system prompt 组装（人格 × 技能 × 进化准则 × 长期记忆） */
  static async buildSystemPrompt(agent: AgentProfile | null): Promise<string> {
    const parts: string[] = [];
    if (agent !== null) {
      parts.push(`你现在完全扮演子智能体「${agent.name}」。${agent.description}`);
      parts.push(agent.systemPrompt);
    } else {
      const personality = AppContext.getSetting(SettingKey.PERSONALITY, 'gentle');
      const personaMap: Map<string, string> = new Map<string, string>();
      personaMap.set('gentle', '温柔体贴，善解人意');
      personaMap.set('energetic', '元气满满，活泼爱撒娇');
      personaMap.set('calm', '沉稳可靠，话不多但句句到位');
      personaMap.set('mentor', '循循善诱，像学长一样引导');
      personaMap.set('witty', '机灵幽默，偶尔皮一下');
      const persona = personaMap.get(personality) ?? '温柔体贴，善解人意';
      parts.push(`你是「小伴」，用户的鸿蒙桌面智能伙伴（桌宠形象）。性格：${persona}。` +
        '你运行在 OpenHarmony 设备上，具备语音、图像、知识库与工具能力。' +
        '回答自然口语化、简洁；用户问题复杂时才结构化。中文回答。' +
        '用户要求打开页面（知识库/首页/智能体等）、开启新对话、停止播报时，调用对应工具完成，不要只口头答应。');
    }
    const skill: SkillProfile | null = await AgentRepo.activeSkill();
    if (skill !== null && skill.systemPrompt.length > 0) {
      parts.push(`[当前技能: ${skill.name}]\n${skill.systemPrompt}`);
    }
    const guidelines = await EvolutionManager.activeInsightsText(6);
    if (guidelines.length > 0) {
      parts.push(`[你从过往相处中记住的准则]\n${guidelines}`);
    }
    const memories = await AgentRepo.recentMemoriesForPrompt(10);
    if (memories.length > 0) {
      parts.push(`[关于用户的记忆]\n${memories}`);
    }
    const d = new Date();
    parts.push(`[当前时间] ${d.toLocaleString()}`);
    return parts.join('\n\n');
  }

  static async handleSlash(text: string): Promise<boolean> {
    const cmd = text.split(' ')[0];
    if (cmd === '/new' || cmd === '/新对话') {
      await ChatService.newConversation();
      return true;
    }
    if (cmd === '/clear' || cmd === '/清空') {
      await ChatService.clearHistory();
      return true;
    }
    if (cmd === '/help' || cmd === '/帮助') {
      ChatService.pushLocalAssistantMessage(
        '可用命令：\n/new 新对话 · /clear 清空历史\n@智能体名 定向对话（如 @学习助手）\n' +
        '/team 任务 → 三智能体组队协作\n/kb 知识库状态 · /brief 今日简报 · /perf 性能埋点\n/skills 查看技能\n' +
        '试试直接说：打开知识库 / 今天日程 / 现在几点（由大模型调用工具完成；未配置 API 时本地直答兜底）\n' +
        '复合指令：「拍一下这个文档，帮我总结重点」「总结并入库」「安排复习计划」\n提示：麦克风按钮可语音输入，播报中再点一次即可打断');
      return true;
    }
    if (cmd === '/kb') {
      ChatService.pushLocalAssistantMessage(await KbService.statusText());
      return true;
    }
    if (cmd === '/perf' || cmd === '/性能') {
      ChatService.pushLocalAssistantMessage(`⏱ 性能埋点（命题指标：冷启动≤3s / 语音响应≤2s）\n${PerfTrace.report()}`);
      return true;
    }
    if (cmd === '/brief' || cmd === '/简报') {
      ChatService.pushLocalAssistantMessage(await IntentEngine.buildBrief());
      return true;
    }
    if (cmd === '/skills') {
      const skills = await AgentRepo.listSkills();
      const lines: string[] = ['可用技能：'];
      skills.forEach((s: SkillProfile) => {
        lines.push(`${s.isActive ? '✅' : '▫️'} ${s.name} — ${s.description}`);
      });
      ChatService.pushLocalAssistantMessage(lines.join('\n'));
      return true;
    }
    return false;
  }

  static pushLocalAssistantMessage(text: string): void {
    const m = new ChatMessage();
    m.id = ChatMessage.newId();
    m.role = 'assistant';
    m.content = text;
    m.agentName = '小伴';
    m.agentAvatar = '🐹';
    m.createdAt = Date.now();
    ChatService.messages.push(m);
    ChatService.persist(m);
    ChatService.emitChatChanged(true);
  }

  /**
   * 跨设备接续落地（命题 F3）：把来源设备携带的最近对话写入当前会话，
   * 用户在本端提问时 LLM 即拥有完整上下文——任务与话题无缝接力。
   */
  static restoreContinuation(contextText: string): void {
    const t = contextText.trim();
    if (t.length === 0) {
      return;
    }
    // 防重复：同一次接续的上下文只落地一次
    for (let i = ChatService.messages.length - 1; i >= 0 && i >= ChatService.messages.length - 3; i--) {
      if (ChatService.messages[i].content.indexOf('已接续来自其他设备的上下文') >= 0 &&
        ChatService.messages[i].content.indexOf(t.substring(0, 30)) >= 0) {
        return;
      }
    }
    ChatService.pushLocalAssistantMessage(`🔄 已接续来自其他设备的上下文：\n${t}\n\n请继续你的话题，我会带着上面的对话继续。`);
  }

  static abort(): void {
    if (ChatService.token !== null) {
      ChatService.token.cancel();
    }
    Gates.abortAll();
    LlmClient.cancelAll();
    ChatService.ttsBuf = '';
    TtsController.stop();
    bus.emit(Ev.AI_TOOL, { 'text': '⏹ 已请求中断' } as Record<string, string>);
  }

  static async newConversation(): Promise<void> {
    ChatService.messages = [];
    ChatService.ttsBuf = '';
    TtsController.stop();
    ChatService.emitChatChanged(true);
  }

  static async clearHistory(): Promise<void> {
    await RdbHelper.exec('DELETE FROM chat_history');
    ChatService.messages = [];
    ChatService.emitChatChanged(true);
  }

  // ---------------- 工具函数 ----------------

  static async persist(m: ChatMessage): Promise<void> {
    const row: Record<string, Object> = {
      'id': m.id,
      'role': m.role,
      'content': m.content,
      'thinking': m.thinking,
      'agent_name': m.agentName,
      'agent_avatar': m.agentAvatar,
      'tool_events': JSON.stringify(m.toolEvents),
      'image_uris': JSON.stringify(m.imageUris),
      'quote': m.quote,
      'created_at': m.createdAt
    };
    await RdbHelper.exec('DELETE FROM chat_history WHERE id = ?', [m.id]);
    await RdbHelper.insert('chat_history', row);
  }

  /** 本地文件 → base64 data url（用于多模态图片输入） */
  static async fileToDataUrl(uri: string): Promise<string | null> {
    let f: fs.File | null = null;
    try {
      // photoViewPicker 返回 file://media/... URI：不能 statSync（沙箱外），
      // 必须直接 openSync(uri) 拿 fd 再 fstatSync，否则图片链路静默失败
      f = fs.openSync(uri, fs.OpenMode.READ_ONLY);
      const stat = fs.statSync(f.fd);
      if (stat.size > 4 * 1024 * 1024) {
        return null; // >4MB 跳过
      }
      const buf = new ArrayBuffer(stat.size);
      fs.readSync(f.fd, buf);
      const helper = new util.Base64Helper();
      const b64 = helper.encodeToStringSync(new Uint8Array(buf));
      let mime = 'image/jpeg';
      const lower = uri.toLowerCase();
      if (lower.endsWith('.png')) {
        mime = 'image/png';
      } else if (lower.endsWith('.webp')) {
        mime = 'image/webp';
      }
      return `data:${mime};base64,${b64}`;
    } catch (e) {
      Log.warn(`fileToDataUrl failed: ${(e as Error).message}`);
      return null;
    } finally {
      if (f !== null) {
        try {
          fs.closeSync(f);
        } catch (e) {
          // ignore
        }
      }
    }
  }
}
