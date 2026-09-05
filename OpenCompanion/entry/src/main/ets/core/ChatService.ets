import fs from '@ohos.file.fs';
import { util } from '@kit.ArkTS';
import { RdbHelper } from '../commons/RdbHelper';
import { AppContext, SettingKey } from '../commons/AppContext';
import { bus, Ev } from '../commons/EventBus';
import { Log } from '../commons/Log';
import { LlmClient, LlmProfiles } from './LlmClient';
import { AgentLoop, LoopOptions, LoopCallbacks } from './AgentLoop';
import { TeamOrchestrator } from './TeamOrchestrator';
import { AgentRepo } from './AgentRepo';
import { EvolutionManager } from './EvolutionManager';
import { ToolRegistry } from './ToolRegistry';
import { Gates } from './Gates';
import { KbService } from '../knowledge/KbService';
import { TtsController } from '../voice/TtsController';
import { AgentProfile, CancelToken, ChatMessage, ContentPart, JsonUtil, LlmMessage, SkillProfile } from './Types';

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

  static async init(): Promise<void> {
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

    // 派发
    ChatService.busy = true;
    ChatService.token = new CancelToken();
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
        ChatService.emitChatChanged();
      },
      onToolTrace: (line: string) => {
        assistant.toolEvents.push(line);
        ChatService.emitChatChanged(true);
      }
    };

    try {
      if (TeamOrchestrator.isTeamCommand(rest) || mentioned.length >= 2) {
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
      bus.emit(Ev.AI_FINISHED, {
        'agentName': assistant.agentName,
        'agentAvatar': assistant.agentAvatar,
        'elapsedMs': `${Date.now() - t0}`
      } as Record<string, string>);
      // 自动播报
      if (AppContext.getBool(SettingKey.VOICE_AUTO_SPEAK, false) && assistant.content.length > 0) {
        TtsController.speak(assistant.content);
      }
    }
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
        '回答自然口语化、简洁；用户问题复杂时才结构化。中文回答。');
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
        '/team 任务 → 三智能体组队协作\n/kb 查看知识库状态\n/skills 查看技能\n提示：麦克风按钮可语音输入');
      return true;
    }
    if (cmd === '/kb') {
      ChatService.pushLocalAssistantMessage(await KbService.statusText());
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

  static abort(): void {
    if (ChatService.token !== null) {
      ChatService.token.cancel();
    }
    Gates.abortAll();
    LlmClient.cancelAll();
    bus.emit(Ev.AI_TOOL, { 'text': '⏹ 已请求中断' } as Record<string, string>);
  }

  static async newConversation(): Promise<void> {
    ChatService.messages = [];
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
