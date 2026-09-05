import { RdbHelper } from '../commons/RdbHelper';
import { AppContext, SettingKey } from '../commons/AppContext';
import { bus, Ev } from '../commons/EventBus';
import { LlmClient, LlmProfiles } from './LlmClient';
import { AgentProfile, SkillProfile, JsonUtil, ToolDef } from './Types';
import { ToolRegistry } from './ToolRegistry';

/**
 * 智能体/技能仓库（对应桌面版 agents/skills 表 + 内置种子）
 */
export class AgentRepo {
  static readonly STUDY: string = 'agent_study';
  static readonly SCHEDULE: string = 'agent_schedule';
  static readonly DOC: string = 'agent_doc';

  static async ensureBuiltins(): Promise<void> {
    const rs = await RdbHelper.query('SELECT COUNT(*) AS c FROM agents');
    const rows = RdbHelper.resultSetToArray(rs);
    let count = 0;
    if (rows.length > 0) {
      const v = rows[0].get('c');
      count = typeof v === 'number' ? v : Number(v);
    }
    if (count > 0) {
      return;
    }
    const builtins: AgentProfile[] = AgentRepo.builtinAgents();
    for (const a of builtins) {
      await AgentRepo.save(a);
    }
    const skills = AgentRepo.builtinSkills();
    for (const s of skills) {
      await AgentRepo.saveSkill(s);
    }
  }

  static builtinAgents(): AgentProfile[] {
    const list: AgentProfile[] = [];
    const study = new AgentProfile();
    study.id = AgentRepo.STUDY;
    study.name = '学习助手';
    study.avatar = '📚';
    study.description = '基于端侧知识库答疑、整理笔记、出题检验，是你的学习搭子';
    study.systemPrompt = '你是「学习助手」，一名耐心严谨的学习伙伴。优先使用 kb_ask/kb_search 工具在用户的本地知识库中查证再回答，' +
      '回答给出要点结构（要点/展开/小结）。当用户学完一个主题，可主动提出用 1-2 个问题检验理解。' +
      '重要结论可 save_memory 记录，整理的笔记用 save_note 保存为 Markdown。';
    study.allowedTools = ['kb_ask', 'kb_search', 'kb_ingest', 'save_memory', 'search_memories', 'web_search', 'read_webpage', 'save_note', 'get_current_time'];
    list.push(study);

    const schedule = new AgentProfile();
    schedule.id = AgentRepo.SCHEDULE;
    schedule.name = '日程管家';
    schedule.avatar = '⏰';
    schedule.description = '管理提醒与日程、播报天气与每日简报';
    schedule.systemPrompt = '你是「日程管家」，可靠高效的生活秘书。创建提醒一律使用 create_reminder 工具（会生成系统级通知），' +
      '创建后向用户确认时间。用户询问安排时用 list_reminders 查询。每日简报 = 时间 + 天气 + 待办。' +
      '用户提到的偏好（如提醒提前量）用 save_memory 记住。';
    schedule.allowedTools = ['create_reminder', 'list_reminders', 'delete_reminder', 'get_weather', 'save_memory', 'search_memories', 'get_current_time'];
    list.push(schedule);

    const doc = new AgentProfile();
    doc.id = AgentRepo.DOC;
    doc.name = '文档处理';
    doc.avatar = '📄';
    doc.description = '图片文字提取、文档总结改写、知识入库';
    doc.systemPrompt = '你是「文档处理」专员。处理图片时先用 ocr_image 提取文字；长文本先分块总结再归纳；' +
      '用户说"存入知识库"时用 kb_ingest；整理成果用 save_note 导出 Markdown。总结输出：一句话摘要 + 要点列表 + 行动项。';
    doc.allowedTools = ['ocr_image', 'read_file', 'list_files', 'kb_ingest', 'kb_search', 'save_note', 'get_current_time'];
    list.push(doc);
    return list;
  }

  static builtinSkills(): SkillProfile[] {
    const s1 = new SkillProfile();
    s1.id = 'skill_companion';
    s1.name = '陪伴聊天';
    s1.description = '日常陪伴（默认）';
    s1.systemPrompt = '';
    s1.isActive = true;
    const s2 = new SkillProfile();
    s2.id = 'skill_deep';
    s2.name = '深度研究';
    s2.description = '先检索知识库与记忆，再联网查证，输出带依据的长答案';
    s2.systemPrompt = '回答前先 kb_search 查本地知识库、search_memories 查偏好；需要事实核查时 web_search。' +
      '回答采用「结论 → 依据 → 注意点」结构，标注信息来源（知识库文档名或网页链接）。';
    s2.isActive = false;
    return [s1, s2];
  }

  static async list(): Promise<AgentProfile[]> {
    const rs = await RdbHelper.query('SELECT * FROM agents ORDER BY is_builtin DESC, created_at ASC');
    const rows = RdbHelper.resultSetToArray(rs);
    const out: AgentProfile[] = [];
    rows.forEach((r: Map<string, Object>) => {
      const a = new AgentProfile();
      a.id = String(r.get('id'));
      a.name = String(r.get('name'));
      a.avatar = String(r.get('avatar'));
      a.description = String(r.get('description'));
      a.systemPrompt = String(r.get('system_prompt'));
      a.allowedTools = JsonUtil.getStringArray({ 't': String(r.get('allowed_tools')) }, 't');
      a.isBuiltin = Number(r.get('is_builtin')) === 1;
      out.push(a);
    });
    return out;
  }

  static async save(a: AgentProfile): Promise<void> {
    const row: Record<string, Object> = {
      'id': a.id.length > 0 ? a.id : `agent_${Date.now()}`,
      'name': a.name,
      'avatar': a.avatar,
      'description': a.description,
      'system_prompt': a.systemPrompt,
      'allowed_tools': JSON.stringify(a.allowedTools),
      'is_builtin': a.isBuiltin ? 1 : 0,
      'created_at': Date.now()
    };
    a.id = row['id'] as string;
    await RdbHelper.exec('DELETE FROM agents WHERE id = ?', [a.id]);
    await RdbHelper.insert('agents', row);
    bus.emit(Ev.AGENTS_CHANGED);
  }

  static async delete(id: string): Promise<string> {
    const all = await AgentRepo.list();
    let isBuiltin = false;
    all.forEach((a: AgentProfile) => {
      if (a.id === id && a.isBuiltin) {
        isBuiltin = true;
      }
    });
    if (isBuiltin) {
      return '内置子智能体不可删除';
    }
    await RdbHelper.exec('DELETE FROM agents WHERE id = ?', [id]);
    bus.emit(Ev.AGENTS_CHANGED);
    return 'ok';
  }

  /** AI 生成智能体定义（对应 generate_agent_spec） */
  static async generateSpec(userDesc: string): Promise<AgentProfile | null> {
    const profile = await LlmProfiles.active();
    if (profile === null) {
      return null;
    }
    const toolNames: string[] = [];
    ToolRegistry.all().forEach((t: ToolDef) => {
      toolNames.push(`${t.name}: ${t.description}`);
    });
    const system = '你是智能体配置生成器。根据用户描述输出严格 JSON（不要多余文字），字段：' +
      '{"name":"中文名(<=6字)","avatar":"一个emoji","description":"一句话职责","system_prompt":"人设与工作方式(200字内,说明优先用哪些工具)","tools":["工具名数组"]}。' +
      `可用工具列表:\n${toolNames.join('\n')}`;
    const raw = await LlmClient.chatOnce(profile, system, userDesc, 800);
    const jsonText = JsonUtil.extractJson(raw);
    if (!jsonText) {
      return null;
    }
    const obj = JsonUtil.parseObj(jsonText);
    if (!obj) {
      return null;
    }
    const a = new AgentProfile();
    a.name = JsonUtil.getString(obj, 'name', '自定义智能体');
    a.avatar = JsonUtil.getString(obj, 'avatar', '🤖');
    a.description = JsonUtil.getString(obj, 'description', userDesc);
    a.systemPrompt = JsonUtil.getString(obj, 'system_prompt', '');
    a.allowedTools = JsonUtil.getStringArray(obj, 'tools');
    return a;
  }

  // ---------------- 技能 ----------------

  static async listSkills(): Promise<SkillProfile[]> {
    const rs = await RdbHelper.query('SELECT * FROM skills');
    const rows = RdbHelper.resultSetToArray(rs);
    const out: SkillProfile[] = [];
    rows.forEach((r: Map<string, Object>) => {
      const s = new SkillProfile();
      s.id = String(r.get('id'));
      s.name = String(r.get('name'));
      s.description = String(r.get('description'));
      s.systemPrompt = String(r.get('system_prompt'));
      s.allowedTools = JsonUtil.getStringArray({ 't': String(r.get('allowed_tools')) }, 't');
      s.isActive = Number(r.get('is_active')) === 1;
      out.push(s);
    });
    return out;
  }

  static async saveSkill(s: SkillProfile): Promise<void> {
    const row: Record<string, Object> = {
      'id': s.id.length > 0 ? s.id : `skill_${Date.now()}`,
      'name': s.name,
      'description': s.description,
      'system_prompt': s.systemPrompt,
      'allowed_tools': JSON.stringify(s.allowedTools),
      'is_active': s.isActive ? 1 : 0
    };
    s.id = row['id'] as string;
    if (s.isActive) {
      await RdbHelper.exec('UPDATE skills SET is_active = 0');
      await AppContext.setSetting(SettingKey.ACTIVE_SKILL, s.id);
    }
    await RdbHelper.exec('DELETE FROM skills WHERE id = ?', [s.id]);
    await RdbHelper.insert('skills', row);
  }

  static async activeSkill(): Promise<SkillProfile | null> {
    const skills = await AgentRepo.listSkills();
    let picked: SkillProfile | null = null;
    skills.forEach((s: SkillProfile) => {
      if (s.isActive) {
        picked = s;
      }
    });
    return picked;
  }

  // ---------------- 长期记忆（供提示词注入） ----------------

  static async recentMemoriesForPrompt(limit: number): Promise<string> {
    const rs = await RdbHelper.query(
      'SELECT key, value FROM pet_memory ORDER BY created_at DESC LIMIT ?', [limit]);
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return '';
    }
    const lines: string[] = [];
    rows.forEach((r: Map<string, Object>) => {
      lines.push(`- ${r.get('key')}: ${r.get('value')}`);
    });
    return lines.join('\n');
  }

  static async memoryCount(): Promise<number> {
    const rs = await RdbHelper.query('SELECT COUNT(*) AS c FROM pet_memory');
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return 0;
    }
    const v = rows[0].get('c');
    return typeof v === 'number' ? v : Number(v);
  }
}
