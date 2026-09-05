import { RdbHelper } from '../commons/RdbHelper';
import { AppContext, SettingKey } from '../commons/AppContext';
import { bus, Ev } from '../commons/EventBus';
import { Log } from '../commons/Log';
import { LlmClient, LlmProfiles } from './LlmClient';
import { AgentRepo } from './AgentRepo';
import { AgentProfile, SkillProfile, JsonUtil, LlmProfile } from './Types';
import { ToolRegistry } from './ToolRegistry';

/**
 * 自进化系统（对应桌面版 evolution/：复盘 → 画像准则/进化提案 → 应用/回滚）。
 * 这是产品差异化核心：助理会从对话中沉淀对你的理解，并自主提出能力进化。
 */
export class EvolutionManager {
  private static lastAutoReflectAt: number = 0;
  private static userMsgsSinceReflect: number = 0;

  static isAutoEnabled(): boolean {
    return AppContext.getBool(SettingKey.EVOLUTION_AUTO, true);
  }

  static async setAutoEnabled(v: boolean): Promise<void> {
    await AppContext.setBool(SettingKey.EVOLUTION_AUTO, v);
  }

  /** 通知：用户发来一条消息（用于自动复盘触发判断） */
  static onUserMessage(text: string): void {
    EvolutionManager.userMsgsSinceReflect++;
    const corrections = ['不对', '不是这样', '别再', '禁止', '改用', '你应该', '记住'];
    let hit = false;
    corrections.forEach((k: string) => {
      if (text.includes(k)) {
        hit = true;
      }
    });
    if (EvolutionManager.isAutoEnabled() && hit &&
      Date.now() - EvolutionManager.lastAutoReflectAt > 3 * 60 * 1000) {
      EvolutionManager.lastAutoReflectAt = Date.now();
      EvolutionManager.userMsgsSinceReflect = 0;
      AppContext.postFrame(() => {
        EvolutionManager.reflect().then((s: string) => {
          Log.info(`auto evolution: ${s}`);
        }).catch((e: Error) => {
          Log.warn(`auto evolution failed: ${e.message}`);
        });
      }, 2000);
    }
  }

  static async activeInsightsText(limit: number): Promise<string> {
    const rs = await RdbHelper.query(
      "SELECT category, content, confidence FROM evolution_insights WHERE status = 'active' ORDER BY confidence DESC LIMIT ?",
      [limit]);
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return '';
    }
    const lines: string[] = [];
    rows.forEach((r: Map<string, Object>) => {
      lines.push(`- (${r.get('category')}) ${r.get('content')}`);
    });
    return lines.join('\n');
  }

  static async summary(): Promise<Record<string, Object>> {
    const insights = await EvolutionManager.countBy("SELECT COUNT(*) AS c FROM evolution_insights WHERE status = 'active'");
    const applied = await EvolutionManager.countBy("SELECT COUNT(*) AS c FROM evolution_logs");
    const level = Math.min(9, 1 + Math.floor(insights / 3) + Math.floor(applied / 2));
    const titles = ['初识', '熟悉', '默契', '心有灵犀', '老友', '知己', '灵魂搭档', '无可替代', '合二为一', '传说'];
    const title = titles[Math.min(titles.length - 1, level - 1)];
    const out: Record<string, Object> = {
      'level': level,
      'title': title,
      'insights': insights,
      'appliedLogs': applied
    };
    return out;
  }

  private static async countBy(sql: string): Promise<number> {
    const rs = await RdbHelper.query(sql);
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return 0;
    }
    const v = rows[0].get('c');
    return typeof v === 'number' ? v : Number(v);
  }

  /** 触发一次复盘：分析近期对话，沉淀准则 + 生成进化提案 */
  static async reflect(): Promise<string> {
    const profile: LlmProfile | null = await LlmProfiles.active();
    if (profile === null) {
      return '未配置 LLM API';
    }
    const rs = await RdbHelper.query(
      "SELECT role, content FROM chat_history ORDER BY created_at DESC LIMIT 30");
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length < 4) {
      return '对话还太少，多聊几句我才能更懂你';
    }
    rows.reverse();
    const transcript: string[] = [];
    rows.forEach((r: Map<string, Object>) => {
      const role = String(r.get('role')) === 'user' ? '用户' : '我';
      let content = String(r.get('content'));
      if (content.length > 200) {
        content = content.substring(0, 200);
      }
      transcript.push(`${role}: ${content}`);
    });
    const existing = await EvolutionManager.activeInsightsText(12);
    const agents = await AgentRepo.list();
    const agentNames = agents.map((a: AgentProfile) => a.name).join('、');

    const system = '你是个人助理的自我复盘引擎。分析对话记录，输出严格 JSON（不要其它文字）：' +
      '{"insights":[{"category":"preference|work_domain|habit|negative_constraint","content":"对用户的画像准则(一句话)","confidence":0.5到1.0}],' +
      '"proposals":[{"type":"create_agent|create_skill|create_reminder|update_guideline","name":"名称","payload":"该提案的完整参数JSON(字符串)"}]}。' +
      '注意：只提出确实有价值的洞察(0-4条)与提案(0-2条)。create_agent 的 payload: {"description":"...","system_prompt":"...","tools":["工具名"]}；' +
      'create_skill 的 payload: {"description":"...","system_prompt":"..."}；create_reminder 的 payload: {"title":"...","note":"...","hour":9,"minute":0}；' +
      'update_guideline 的 payload: {"content":"..."}。';
    const user = `已有画像准则:\n${existing.length > 0 ? existing : '（暂无）'}\n已有智能体: ${agentNames}\n\n近期对话:\n${transcript.join('\n')}`;
    let raw = '';
    try {
      raw = await LlmClient.chatOnce(profile, system, user, 1600);
    } catch (e) {
      return `复盘失败: ${(e as Error).message}`;
    }
    const jsonText = JsonUtil.extractJson(raw);
    if (!jsonText) {
      return '模型未返回有效复盘结果';
    }
    const obj = JsonUtil.parseObj(jsonText);
    if (!obj) {
      return '模型未返回有效复盘结果';
    }

    // 洞察去重合并（对应 bigram Jaccard ≥0.8 合并）
    let added = 0;
    let merged = 0;
    const insightArr = obj['insights'] as Array<Object> | null;
    if (insightArr) {
      for (const it of insightArr) {
        const m = it as Record<string, Object>;
        const content = typeof m['content'] === 'string' ? m['content'] as string : '';
        const category = typeof m['category'] === 'string' ? m['category'] as string : 'preference';
        const confidence = typeof m['confidence'] === 'number' ? m['confidence'] as number : 0.6;
        if (content.length === 0) {
          continue;
        }
        const dupId = await EvolutionManager.findDuplicate(content);
        if (dupId !== null) {
          await RdbHelper.exec(
            'UPDATE evolution_insights SET confidence = MIN(1.0, confidence + 0.05), hit_count = hit_count + 1 WHERE id = ?',
            [dupId]);
          merged++;
        } else {
          const row: Record<string, Object> = {
            'id': `ins${Date.now()}_${added}`,
            'category': category,
            'content': content,
            'confidence': confidence,
            'hit_count': 1,
            'status': 'active',
            'created_at': Date.now()
          };
          await RdbHelper.insert('evolution_insights', row);
          added++;
        }
      }
    }

    let proposals = 0;
    const propArr = obj['proposals'] as Array<Object> | null;
    if (propArr) {
      for (const it of propArr) {
        const m = it as Record<string, Object>;
        const ptype = typeof m['type'] === 'string' ? m['type'] as string : '';
        const name = typeof m['name'] === 'string' ? m['name'] as string : '';
        const payload = typeof m['payload'] === 'string' ? m['payload'] as string : '{}';
        if (ptype.length === 0) {
          continue;
        }
        const row: Record<string, Object> = {
          'id': `prp${Date.now()}_${proposals}`,
          'ptype': ptype,
          'name': name,
          'payload': payload,
          'status': 'pending',
          'created_at': Date.now()
        };
        await RdbHelper.insert('evolution_proposals', row);
        proposals++;
      }
    }
    EvolutionManager.lastAutoReflectAt = Date.now();
    EvolutionManager.userMsgsSinceReflect = 0;
    bus.emit(Ev.EVOLUTION_CHANGED);
    return `复盘完成：新增准则 ${added} 条${merged > 0 ? `，强化既有准则 ${merged} 条` : ''}，进化提案 ${proposals} 条`;
  }

  /** 字符 bigram Jaccard 相似度查重 */
  private static async findDuplicate(content: string): Promise<string | null> {
    const rs = await RdbHelper.query("SELECT id, content FROM evolution_insights WHERE status = 'active'");
    const rows = RdbHelper.resultSetToArray(rs);
    let dup: string | null = null;
    rows.forEach((r: Map<string, Object>) => {
      const other = String(r.get('content'));
      if (dup === null && EvolutionManager.bigramJaccard(content, other) >= 0.8) {
        dup = String(r.get('id'));
      }
    });
    return dup;
  }

  static bigramJaccard(a: string, b: string): number {
    const setA = EvolutionManager.bigrams(a);
    const setB = EvolutionManager.bigrams(b);
    if (setA.length === 0 || setB.length === 0) {
      return 0;
    }
    let inter = 0;
    setA.forEach((g: string) => {
      if (setB.indexOf(g) >= 0) {
        inter++;
      }
    });
    return inter / (setA.length + setB.length - inter);
  }

  private static bigrams(s: string): string[] {
    const out: string[] = [];
    for (let i = 0; i < s.length - 1; i++) {
      out.push(s.substring(i, i + 2));
    }
    return out;
  }

  // ---------------- 提案应用/回滚 ----------------

  static async listProposals(): Promise<Array<Map<string, Object>>> {
    const rs = await RdbHelper.query("SELECT * FROM evolution_proposals WHERE status = 'pending' ORDER BY created_at DESC");
    return RdbHelper.resultSetToArray(rs);
  }

  static async listInsights(): Promise<Array<Map<string, Object>>> {
    const rs = await RdbHelper.query('SELECT * FROM evolution_insights ORDER BY confidence DESC');
    return RdbHelper.resultSetToArray(rs);
  }

  static async listLogs(): Promise<Array<Map<string, Object>>> {
    const rs = await RdbHelper.query('SELECT * FROM evolution_logs ORDER BY created_at DESC LIMIT 50');
    return RdbHelper.resultSetToArray(rs);
  }

  static async applyProposal(proposalId: string): Promise<string> {
    const rs = await RdbHelper.query('SELECT * FROM evolution_proposals WHERE id = ?', [proposalId]);
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return '提案不存在';
    }
    const p = rows[0];
    const ptype = String(p.get('ptype'));
    const name = String(p.get('name'));
    const payloadRaw = String(p.get('payload'));
    const payload = JsonUtil.parseObj(payloadRaw);

    if (ptype === 'create_agent') {
      const a = new AgentProfile();
      a.id = `agent_evo_${Date.now()}`;
      a.name = name.length > 0 ? name : '进化智能体';
      a.avatar = '🌱';
      a.description = JsonUtil.getString(payload, 'description', '由进化系统创建');
      a.systemPrompt = JsonUtil.getString(payload, 'system_prompt', '你是用户的贴心助手。');
      a.allowedTools = JsonUtil.getStringArray(payload, 'tools');
      const before = JSON.stringify(await AgentRepo.list());
      await AgentRepo.save(a);
      await EvolutionManager.log('create_agent', before, JSON.stringify(a));
    } else if (ptype === 'create_skill') {
      const s = new SkillProfile();
      s.id = `skill_evo_${Date.now()}`;
      s.name = name.length > 0 ? name : '进化技能';
      s.description = JsonUtil.getString(payload, 'description', '由进化系统创建');
      s.systemPrompt = JsonUtil.getString(payload, 'system_prompt', '');
      const before = JSON.stringify(await AgentRepo.listSkills());
      await AgentRepo.saveSkill(s);
      await EvolutionManager.log('create_skill', before, JSON.stringify(s));
    } else if (ptype === 'create_reminder') {
      const args: Record<string, Object> = {};
      if (payload) {
        const keys = Object.keys(payload);
        keys.forEach((k: string) => {
          const v = payload[k];
          if (v !== null && v !== undefined) {
            args[k] = v;
          }
        });
      }
      const now = new Date();
      if (typeof args['year'] !== 'number') {
        args['year'] = now.getFullYear();
        args['month'] = now.getMonth() + 1;
        args['day'] = now.getDate();
      }
      if (typeof args['hour'] !== 'number') {
        args['hour'] = 9;
        args['minute'] = 0;
      }
      const result = await ToolRegistry.execute('create_reminder', JSON.stringify(args));
      await EvolutionManager.log('create_reminder', '{}', result);
    } else if (ptype === 'update_guideline') {
      const content = JsonUtil.getString(payload, 'content', name);
      const row: Record<string, Object> = {
        'id': `ins${Date.now()}`,
        'category': 'preference',
        'content': content,
        'confidence': 0.8,
        'hit_count': 1,
        'status': 'active',
        'created_at': Date.now()
      };
      await RdbHelper.insert('evolution_insights', row);
      await EvolutionManager.log('update_guideline', '{}', content);
    } else {
      return `未知提案类型: ${ptype}`;
    }
    await RdbHelper.exec("UPDATE evolution_proposals SET status = 'applied' WHERE id = ?", [proposalId]);
    bus.emit(Ev.EVOLUTION_CHANGED);
    bus.emit(Ev.AGENTS_CHANGED);
    return `已应用进化提案「${name}」`;
  }

  static async rejectProposal(proposalId: string): Promise<void> {
    await RdbHelper.exec("UPDATE evolution_proposals SET status = 'rejected' WHERE id = ?", [proposalId]);
    bus.emit(Ev.EVOLUTION_CHANGED);
  }

  /** 回滚一次进化日志（当前支持 create_agent / create_skill） */
  static async rollbackLog(logId: string): Promise<string> {
    const rs = await RdbHelper.query('SELECT * FROM evolution_logs WHERE id = ?', [logId]);
    const rows = RdbHelper.resultSetToArray(rs);
    if (rows.length === 0) {
      return '日志不存在';
    }
    const action = String(rows[0].get('action'));
    const after = String(rows[0].get('after_state'));
    const obj = JsonUtil.parseObj(after);
    if (action === 'create_agent' && obj) {
      const id = JsonUtil.getString(obj, 'id', '');
      if (id.length > 0) {
        await RdbHelper.exec('DELETE FROM agents WHERE id = ?', [id]);
        await RdbHelper.exec("UPDATE evolution_logs SET action = 'create_agent_rolled_back' WHERE id = ?", [logId]);
        bus.emit(Ev.AGENTS_CHANGED);
        bus.emit(Ev.EVOLUTION_CHANGED);
        return '已回滚智能体创建';
      }
    } else if (action === 'create_skill' && obj) {
      const id = JsonUtil.getString(obj, 'id', '');
      if (id.length > 0) {
        await RdbHelper.exec('DELETE FROM skills WHERE id = ?', [id]);
        await RdbHelper.exec("UPDATE evolution_logs SET action = 'create_skill_rolled_back' WHERE id = ?", [logId]);
        bus.emit(Ev.EVOLUTION_CHANGED);
        return '已回滚技能创建';
      }
    }
    return '该操作暂不支持回滚';
  }

  private static async log(action: string, before: string, after: string): Promise<void> {
    const row: Record<string, Object> = {
      'id': `log${Date.now()}`,
      'action': action,
      'before_state': before,
      'after_state': after,
      'created_at': Date.now()
    };
    await RdbHelper.insert('evolution_logs', row);
  }
}
