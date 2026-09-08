import fs from '@ohos.file.fs';
import { util } from '@kit.ArkTS';
import { reminderAgentManager } from '@kit.BackgroundTasksKit';
import { deviceInfo, pasteboard, batteryInfo } from '@kit.BasicServicesKit';
import { Want } from '@kit.AbilityKit';
import { picker } from '@kit.CoreFileKit';
import { AppContext } from '../commons/AppContext';
import { RdbHelper } from '../commons/RdbHelper';
import { NetClient } from '../commons/NetClient';
import { Log } from '../commons/Log';
import { TtsController } from '../voice/TtsController';
import { BehaviorEngine } from '../pet/BehaviorEngine';
import { ToolDef, ToolParam, JsonUtil } from './Types';

type ToolExecutor = (args: Record<string, Object>) => Promise<string>;

/**
 * 工具注册中心（对应桌面版 tools.rs 的 tool_definitions/execute_tool）。
 * 基础工具内置于此；kb_* / ocr_* 等依赖知识库层的工具由对应模块在初始化时注册。
 */
export class ToolRegistry {
  private static executors: Map<string, ToolExecutor> = new Map<string, ToolExecutor>();
  private static defs: Map<string, ToolDef> = new Map<string, ToolDef>();
  private static inited: boolean = false;

  /** 高风险工具：需确认后执行（对应 is_high_risk_tool） */
  static readonly HIGH_RISK: string[] = ['delete_memory', 'delete_file', 'read_file'];

  static ensureInited(): void {
    if (ToolRegistry.inited) {
      return;
    }
    ToolRegistry.inited = true;
    ToolRegistry.registerBasicTools();
  }

  static register(def: ToolDef, executor: ToolExecutor): void {
    ToolRegistry.ensureInited();
    ToolRegistry.defs.set(def.name, def);
    ToolRegistry.executors.set(def.name, executor);
  }

  static all(): ToolDef[] {
    ToolRegistry.ensureInited();
    const out: ToolDef[] = [];
    ToolRegistry.defs.forEach((d: ToolDef) => {
      out.push(d);
    });
    return out;
  }

  static defsFor(whitelist: string[]): ToolDef[] {
    ToolRegistry.ensureInited();
    if (whitelist.length === 0) {
      return ToolRegistry.all();
    }
    const out: ToolDef[] = [];
    whitelist.forEach((n: string) => {
      const d = ToolRegistry.defs.get(n);
      if (d) {
        out.push(d);
      }
    });
    return out;
  }

  static isHighRisk(name: string): boolean {
    return ToolRegistry.HIGH_RISK.indexOf(name) >= 0;
  }

  static async execute(name: string, argsJson: string): Promise<string> {
    ToolRegistry.ensureInited();
    const executor = ToolRegistry.executors.get(name);
    if (!executor) {
      return `错误: 工具 ${name} 不存在`;
    }
    let args: Record<string, Object> = {};
    if (argsJson.trim().length > 0) {
      const parsed = JsonUtil.parseObj(argsJson);
      if (parsed) {
        args = parsed;
      }
    }
    try {
      return await executor(args);
    } catch (e) {
      const err = e as Error;
      if (err.message === '__ABORTED__') {
        throw err;
      }
      Log.warn(`tool ${name} failed: ${err.message}`);
      return `执行失败: ${err.message}`;
    }
  }

  private static str(args: Record<string, Object>, key: string, def: string): string {
    const v = args[key];
    return typeof v === 'string' ? v as string : def;
  }

  private static num(args: Record<string, Object>, key: string, def: number): number {
    const v = args[key];
    return typeof v === 'number' ? v as number : def;
  }

  static docsDir(): string {
    const dir = `${AppContext.getContext().filesDir}/documents`;
    if (!fs.accessSync(dir)) {
      fs.mkdirSync(dir, true);
    }
    return dir;
  }

  static async writeDocFile(name: string, content: string): Promise<string> {
    const path = `${ToolRegistry.docsDir()}/${name}`;
    const f = fs.openSync(path, fs.OpenMode.READ_WRITE | fs.OpenMode.CREATE | fs.OpenMode.TRUNC);
    try {
      fs.writeSync(f.fd, content);
    } finally {
      fs.closeSync(f);
    }
    return path;
  }

  /** 标题 → 安全文件名（去分隔符，补 .md 后缀） */
  static sanitizeDocName(title: string): string {
    const t = title.endsWith('.md') ? title.substring(0, title.length - 3) : title;
    return `${t.replace(/[\\/:*?"<>|]/g, '_')}.md`;
  }

  static async appendDocFile(name: string, content: string): Promise<string> {
    const path = `${ToolRegistry.docsDir()}/${name}`;
    const f = fs.openSync(path, fs.OpenMode.READ_WRITE | fs.OpenMode.CREATE | fs.OpenMode.APPEND);
    try {
      fs.writeSync(f.fd, content);
    } finally {
      fs.closeSync(f);
    }
    return path;
  }

  static readTextFile(path: string, maxBytes: number = 512 * 1024): string {
    const stat = fs.statSync(path);
    if (stat.size > maxBytes) {
      throw new Error(`文件过大(${stat.size}B)，上限 ${maxBytes}B`);
    }
    const f = fs.openSync(path, fs.OpenMode.READ_ONLY);
    try {
      const buf = new ArrayBuffer(stat.size);
      fs.readSync(f.fd, buf);
      const decoder = util.TextDecoder.create('utf-8');
      return decoder.decodeToString(new Uint8Array(buf));
    } finally {
      fs.closeSync(f);
    }
  }

  // ---------------- 基础工具集 ----------------

  private static registerBasicTools(): void {
    ToolRegistry.register(new ToolDef('get_current_time', '获取当前日期与时间', []), async (): Promise<string> => {
      const d = new Date();
      const week = ['日', '一', '二', '三', '四', '五', '六'][d.getDay()];
      return `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日 星期${week} ` +
        `${d.getHours()}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
    });

    ToolRegistry.register(new ToolDef('get_device_info', '获取设备信息（型号/系统版本）', []), async (): Promise<string> => {
      return `设备: ${deviceInfo.productModel} 品牌: ${deviceInfo.brand} 系统: ${deviceInfo.osFullName} API: ${deviceInfo.sdkApiVersion}`;
    });

    ToolRegistry.register(new ToolDef('get_weather', '查询天气', [
      { name: 'location', type: 'string', description: '城市名（如 北京），默认取设置中的城市', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const city = ToolRegistry.str(args, 'location', AppContext.getSetting('weather_city', '北京'));
      const url = `https://wttr.in/${encodeURIComponent(city)}?format=j1`;
      const raw = await NetClient.getText(url);
      const obj = JSON.parse(raw) as Record<string, Object>;
      const cur = obj['current_condition'] as Array<Object>;
      if (!cur || cur.length === 0) {
        return '未获取到天气数据';
      }
      const c = cur[0] as Record<string, Object>;
      const desc = c['weatherDesc'] as Array<Object>;
      const dText = desc && desc.length > 0 ? ((desc[0] as Record<string, Object>)['value'] as string) : '';
      return `${city} 天气: ${dText} 气温: ${c['temp_C']}°C 体感: ${c['FeelsLikeC']}°C 湿度: ${c['humidity']}%`;
    });

    ToolRegistry.register(new ToolDef('web_search', '联网搜索，返回结果标题与链接', [
      { name: 'query', type: 'string', description: '搜索关键词', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const q = ToolRegistry.str(args, 'query', '');
      const url = `https://html.duckduckgo.com/html/?q=${encodeURIComponent(q)}`;
      const headers = new Map<string, string>();
      headers.set('User-Agent', 'Mozilla/5.0 (HarmonyOS) OpenCompanion/1.0');
      const html = await NetClient.getText(url, headers);
      const results: string[] = [];
      const re = /<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g;
      let m: RegExpExecArray | null = re.exec(html);
      while (m !== null && results.length < 8) {
        const title = m[2].replace(/<[^>]+>/g, '').trim();
        if (title.length > 0) {
          results.push(`${results.length + 1}. ${title} — ${m[1]}`);
        }
        m = re.exec(html);
      }
      if (results.length === 0) {
        return '搜索无结果或网络不可用，请稍后重试';
      }
      return `「${q}」搜索结果:\n${results.join('\n')}`;
    });

    ToolRegistry.register(new ToolDef('read_webpage', '读取网页正文文本', [
      { name: 'url', type: 'string', description: '网页 URL', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const url = ToolRegistry.str(args, 'url', '');
      const headers = new Map<string, string>();
      headers.set('User-Agent', 'Mozilla/5.0 (HarmonyOS) OpenCompanion/1.0');
      const html = await NetClient.getText(url, headers);
      const text = html
        .replace(/<script[\s\S]*?<\/script>/g, '')
        .replace(/<style[\s\S]*?<\/style>/g, '')
        .replace(/<[^>]+>/g, ' ')
        .replace(/\s+/g, ' ');
      return text.length > 3000 ? text.substring(0, 3000) : text;
    });

    ToolRegistry.register(new ToolDef('save_memory', '保存长期记忆（用户偏好/事实/习惯）', [
      { name: 'key', type: 'string', description: '记忆标题', required: true } as ToolParam,
      { name: 'value', type: 'string', description: '记忆内容', required: true } as ToolParam,
      { name: 'category', type: 'string', description: '分类: preference/fact/habit/work_domain', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const id = `mem${Date.now()}`;
      const row: Record<string, Object> = {
        'id': id,
        'category': ToolRegistry.str(args, 'category', 'general'),
        'key': ToolRegistry.str(args, 'key', '未命名'),
        'value': ToolRegistry.str(args, 'value', ''),
        'created_at': Date.now()
      };
      await RdbHelper.insert('pet_memory', row);
      return `已保存记忆「${row['key'] as string}」`;
    });

    ToolRegistry.register(new ToolDef('search_memories', '检索长期记忆', [
      { name: 'keyword', type: 'string', description: '关键词', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const kw = ToolRegistry.str(args, 'keyword', '');
      const rs = await RdbHelper.query(
        'SELECT id, category, key, value FROM pet_memory WHERE key LIKE ? OR value LIKE ? ORDER BY created_at DESC LIMIT 10',
        [`%${kw}%`, `%${kw}%`]);
      const rows = RdbHelper.resultSetToArray(rs);
      if (rows.length === 0) {
        return '没有找到相关记忆';
      }
      const lines: string[] = [];
      rows.forEach((r: Map<string, Object>) => {
        lines.push(`[${r.get('id')}] (${r.get('category')}) ${r.get('key')}: ${r.get('value')}`);
      });
      return lines.join('\n');
    });

    ToolRegistry.register(new ToolDef('delete_memory', '删除一条长期记忆', [
      { name: 'id', type: 'string', description: '记忆 id（先 search_memories 获取）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const id = ToolRegistry.str(args, 'id', '');
      await RdbHelper.exec('DELETE FROM pet_memory WHERE id = ?', [id]);
      return `已删除记忆 ${id}`;
    });

    ToolRegistry.register(new ToolDef('create_reminder', '创建系统级提醒（到点通知，应用不在前台也会提醒）', [
      { name: 'title', type: 'string', description: '提醒标题', required: true } as ToolParam,
      { name: 'year', type: 'number', description: '年（如 2026）', required: true } as ToolParam,
      { name: 'month', type: 'number', description: '月 1-12', required: true } as ToolParam,
      { name: 'day', type: 'number', description: '日', required: true } as ToolParam,
      { name: 'hour', type: 'number', description: '时 0-23', required: true } as ToolParam,
      { name: 'minute', type: 'number', description: '分', required: true } as ToolParam,
      { name: 'note', type: 'string', description: '备注', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const title = ToolRegistry.str(args, 'title', '提醒');
      const dt: reminderAgentManager.LocalDateTime = {
        year: ToolRegistry.num(args, 'year', 2026),
        month: ToolRegistry.num(args, 'month', 1),
        day: ToolRegistry.num(args, 'day', 1),
        hour: ToolRegistry.num(args, 'hour', 9),
        minute: ToolRegistry.num(args, 'minute', 0),
        second: 0
      };
      const reminder: reminderAgentManager.ReminderRequestCalendar = {
        reminderType: reminderAgentManager.ReminderType.REMINDER_TYPE_CALENDAR,
        dateTime: dt,
        title: title,
        content: ToolRegistry.str(args, 'note', '来自智能伙伴的提醒')
      };
      const reminderId = await reminderAgentManager.publishReminder(reminder);
      const row: Record<string, Object> = {
        'id': `task${Date.now()}`,
        'title': title,
        'note': `${ToolRegistry.str(args, 'note', '')} #[reminder:${reminderId}]`,
        'due_at': new Date(dt.year, dt.month - 1, dt.day, dt.hour, dt.minute).getTime(),
        'repeat': 'once',
        'enabled': 1,
        'last_triggered_at': 0,
        'created_at': Date.now()
      };
      await RdbHelper.insert('scheduled_tasks', row);
      return `已创建提醒「${title}」（id=${reminderId}），到点会通过系统通知提醒`;
    });

    ToolRegistry.register(new ToolDef('list_reminders', '列出未过期的系统提醒', []), async (): Promise<string> => {
      const valid = await reminderAgentManager.getValidReminders();
      if (valid.length === 0) {
        return '当前没有生效的提醒';
      }
      const lines: string[] = [];
      valid.forEach((r: reminderAgentManager.ReminderRequest) => {
        let dt = '';
        const cal = r as reminderAgentManager.ReminderRequestCalendar;
        if (cal.dateTime) {
          dt = `${cal.dateTime.year}-${cal.dateTime.month}-${cal.dateTime.day} ${cal.dateTime.hour}:${cal.dateTime.minute}`;
        }
        lines.push(`${r.title ?? ''} @ ${dt}`);
      });
      return lines.join('\n');
    });

    ToolRegistry.register(new ToolDef('delete_reminder', '取消一个系统提醒', [
      { name: 'id', type: 'number', description: '提醒 id（create_reminder 返回的 id）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const id = ToolRegistry.num(args, 'id', -1);
      await reminderAgentManager.cancelReminder(id);
      return `已取消提醒 ${id}`;
    });

    ToolRegistry.register(new ToolDef('save_note', '保存笔记/文档到应用文档目录（Markdown，整篇覆盖写入；长文档分段生成配合 append_note 续写，导出用 export_document）', [
      { name: 'title', type: 'string', description: '笔记标题', required: true } as ToolParam,
      { name: 'content', type: 'string', description: 'Markdown 内容', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const title = ToolRegistry.str(args, 'title', `note_${Date.now()}`);
      const content = ToolRegistry.str(args, 'content', '');
      const path = await ToolRegistry.writeDocFile(ToolRegistry.sanitizeDocName(title), content);
      return `已保存笔记: ${path}（${content.length} 字符）`;
    });

    ToolRegistry.register(new ToolDef('append_note', '向已有笔记末尾追加一段内容（生成长文档时分节续写，避免整篇重写）', [
      { name: 'title', type: 'string', description: '笔记标题（与 save_note 用的同名）', required: true } as ToolParam,
      { name: 'content', type: 'string', description: '要追加的 Markdown 片段', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const title = ToolRegistry.str(args, 'title', '');
      const content = ToolRegistry.str(args, 'content', '');
      if (title.trim().length === 0 || content.length === 0) {
        return '错误: 标题与内容均不能为空';
      }
      const path = await ToolRegistry.appendDocFile(ToolRegistry.sanitizeDocName(title), `\n${content}`);
      return `已追加 ${content.length} 字符到 ${path}`;
    });

    ToolRegistry.register(new ToolDef('list_files', '列出应用文档目录下的文件（含大小与修改时间）', []), async (): Promise<string> => {
      const dir = ToolRegistry.docsDir();
      const files = fs.listFileSync(dir);
      if (files.length === 0) {
        return '文档目录为空';
      }
      const lines: string[] = [];
      files.forEach((name: string) => {
        const st = fs.statSync(`${dir}/${name}`);
        const d = new Date(Number(st.mtime) * 1000);
        lines.push(`${name}  ${(st.size / 1024).toFixed(1)}KB  ${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()} ${d.getHours()}:${String(d.getMinutes()).padStart(2, '0')}`);
      });
      return lines.join('\n');
    });

    ToolRegistry.register(new ToolDef('read_file', '读取应用文档目录中的文本文件（或 file:// 开头的已授权文件）', [
      { name: 'path', type: 'string', description: '文件名或 file:// uri', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      let path = ToolRegistry.str(args, 'path', '');
      if (!path.startsWith('file://') && !path.startsWith('/')) {
        path = `${ToolRegistry.docsDir()}/${path}`;
      }
      const text = ToolRegistry.readTextFile(path);
      return text.length === 0 ? '(空文件)' : text;
    });

    // ---------------- 实用工具：计算 / 设备 / 系统 ----------------

    ToolRegistry.register(new ToolDef('calculator', '数学计算：支持 + - * / % ^ 与括号的四则混合运算', [
      { name: 'expression', type: 'string', description: '算式，如 (3+4)*2^10 或 1024/8/60', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const expr = ToolRegistry.str(args, 'expression', '');
      if (expr.trim().length === 0) {
        return '错误: 表达式为空';
      }
      const value = new CalcParser(expr).parse();
      const rounded = Math.round(value * 1e10) / 1e10;
      return `${expr} = ${rounded}`;
    });

    ToolRegistry.register(new ToolDef('get_battery_info', '查询手机电量与充电状态', []), async (): Promise<string> => {
      const soc = batteryInfo.batterySOC;
      let status = '未知';
      switch (batteryInfo.chargingStatus) {
        case batteryInfo.BatteryChargeState.ENABLE: status = '充电中'; break;
        case batteryInfo.BatteryChargeState.DISABLE: status = '使用电池'; break;
        case batteryInfo.BatteryChargeState.FULL: status = '已充满'; break;
        default: break;
      }
      return `电量 ${soc}%，状态: ${status}`;
    });

    ToolRegistry.register(new ToolDef('copy_to_clipboard', '把文本复制到剪贴板，方便用户粘贴到别处', [
      { name: 'text', type: 'string', description: '要复制的文本', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const text = ToolRegistry.str(args, 'text', '');
      const data = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, text);
      await pasteboard.getSystemPasteboard().setData(data);
      const preview = text.length > 40 ? `${text.substring(0, 40)}…` : text;
      return `已复制到剪贴板：「${preview}」`;
    });

    ToolRegistry.register(new ToolDef('open_link', '在系统浏览器中打开一个网页链接', [
      { name: 'url', type: 'string', description: 'http/https 完整链接', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const url = ToolRegistry.str(args, 'url', '');
      if (!url.startsWith('http://') && !url.startsWith('https://')) {
        return '错误: 仅支持 http/https 链接';
      }
      const want: Want = {
        action: 'ohos.want.action.viewData',
        entities: ['entity.system.browsable'],
        uri: url
      };
      await AppContext.getContext().startAbility(want);
      return `已打开链接: ${url}`;
    });

    ToolRegistry.register(new ToolDef('speak_text', '让桌宠用语音朗读一段文本', [
      { name: 'text', type: 'string', description: '要朗读的文本（简洁口语化）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const text = ToolRegistry.str(args, 'text', '');
      if (text.trim().length === 0) {
        return '错误: 朗读内容为空';
      }
      TtsController.speak(text);
      return `开始朗读：「${text}」`;
    });

    // ---------------- 时间管理：倒计时 ----------------

    ToolRegistry.register(new ToolDef('set_timer', '设置倒计时（分钟），到点通过系统通知提醒，应用退后台也生效', [
      { name: 'minutes', type: 'number', description: '倒计时分钟数，1-1440', required: true } as ToolParam,
      { name: 'label', type: 'string', description: '计时用途（如 番茄钟/关火）', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const minutes = ToolRegistry.num(args, 'minutes', 0);
      if (minutes < 1 || minutes > 1440) {
        return '错误: 分钟数需在 1-1440 之间';
      }
      const label = ToolRegistry.str(args, 'label', '时间到啦');
      const req: reminderAgentManager.ReminderRequestTimer = {
        reminderType: reminderAgentManager.ReminderType.REMINDER_TYPE_TIMER,
        triggerTimeInSeconds: Math.round(minutes * 60),
        title: `倒计时 ${minutes} 分钟`,
        content: label
      };
      const id = await reminderAgentManager.publishReminder(req);
      return `已设置 ${minutes} 分钟倒计时「${label}」（提醒 id=${id}），到点会响`;
    });

    // ---------------- 待办清单（todos 表） ----------------

    ToolRegistry.register(new ToolDef('add_todo', '添加一条待办事项', [
      { name: 'title', type: 'string', description: '待办内容', required: true } as ToolParam,
      { name: 'note', type: 'string', description: '补充备注', required: false } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const title = ToolRegistry.str(args, 'title', '');
      if (title.trim().length === 0) {
        return '错误: 待办内容为空';
      }
      const row: Record<string, Object> = {
        'id': `todo${Date.now()}`,
        'title': title,
        'note': ToolRegistry.str(args, 'note', ''),
        'done': 0,
        'created_at': Date.now()
      };
      await RdbHelper.insert('todos', row);
      return `已添加待办「${title}」`;
    });

    ToolRegistry.register(new ToolDef('list_todos', '列出未完成的待办事项', []), async (): Promise<string> => {
      const rs = await RdbHelper.query(
        'SELECT id, title, note FROM todos WHERE done = 0 ORDER BY created_at ASC LIMIT 50');
      const rows = RdbHelper.resultSetToArray(rs);
      if (rows.length === 0) {
        return '待办清单是空的，一切搞定 ✨';
      }
      const lines: string[] = [];
      rows.forEach((r: Map<string, Object>) => {
        const note = String(r.get('note') ?? '');
        lines.push(`[${r.get('id')}] ${r.get('title')}${note.length > 0 ? `（${note}）` : ''}`);
      });
      return `未完成待办 ${rows.length} 条:\n${lines.join('\n')}`;
    });

    ToolRegistry.register(new ToolDef('complete_todo', '把一条待办标记为完成', [
      { name: 'id', type: 'string', description: '待办 id（先 list_todos 获取）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const id = ToolRegistry.str(args, 'id', '');
      const rs = await RdbHelper.query('SELECT title FROM todos WHERE id = ? AND done = 0', [id]);
      const rows = RdbHelper.resultSetToArray(rs);
      if (rows.length === 0) {
        return `没有找到未完成的待办 ${id}（先 list_todos 查一下）`;
      }
      await RdbHelper.exec('UPDATE todos SET done = 1 WHERE id = ?', [id]);
      return `已完成「${String(rows[0].get('title'))}」🎉`;
    });

    // ---------------- 桌宠联动 ----------------

    ToolRegistry.register(new ToolDef('get_pet_status', '查看桌宠当前的状态、心情、开心值与精力', []), async (): Promise<string> => {
      const happyPct = Math.round((BehaviorEngine.happiness + 1) * 50);
      return `状态: ${BehaviorEngine.state}｜心情: ${BehaviorEngine.moodText}｜开心值: ${happyPct}%｜精力: ${Math.round(BehaviorEngine.energy)}%`;
    });

    ToolRegistry.register(new ToolDef('pet_action', '让桌宠做一个动作表达情绪（greet 打招呼 / pat 被摸摸头 / dance 开心 / wave 挥手 / think 思考 / sleep 去睡觉 / wake 醒来 / sad 难过）', [
      { name: 'action', type: 'string', description: 'greet | pat | dance | wave | think | sleep | wake | sad', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const action = ToolRegistry.str(args, 'action', '');
      if (action === 'greet') {
        BehaviorEngine.greet();
      } else if (action === 'pat') {
        BehaviorEngine.pet();
      } else if (action === 'dance') {
        BehaviorEngine.setState('happy', 4000);
      } else if (action === 'wave') {
        BehaviorEngine.setState('waving', 3000);
      } else if (action === 'think') {
        BehaviorEngine.setState('thinking', 5000);
      } else if (action === 'sleep') {
        BehaviorEngine.sleepNow();
      } else if (action === 'wake') {
        BehaviorEngine.wake();
      } else if (action === 'sad') {
        BehaviorEngine.setState('sad', 4000);
      } else {
        return `错误: 未知动作「${action}」，支持 greet/pat/dance/wave/think/sleep/wake/sad`;
      }
      return `桌宠已执行动作「${action}」`;
    });

    // ---------------- 文件管理 ----------------

    ToolRegistry.register(new ToolDef('delete_file', '删除应用文档目录中的一个文件（不可撤销）', [
      { name: 'name', type: 'string', description: '文件名（list_files 里的名字）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const name = ToolRegistry.str(args, 'name', '');
      if (name.length === 0 || name.includes('/') || name.includes('..')) {
        return '错误: 文件名不合法';
      }
      const path = `${ToolRegistry.docsDir()}/${name}`;
      if (!fs.accessSync(path)) {
        return `没有找到文件 ${name}`;
      }
      fs.unlinkSync(path);
      return `已删除文件 ${name}`;
    });

    ToolRegistry.register(new ToolDef('rename_file', '重命名应用文档目录中的文件', [
      { name: 'from', type: 'string', description: '现有文件名', required: true } as ToolParam,
      { name: 'to', type: 'string', description: '新文件名（含扩展名）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const from = ToolRegistry.str(args, 'from', '');
      const to = ToolRegistry.str(args, 'to', '');
      if (from.length === 0 || to.length === 0 || from.includes('/') || to.includes('/') ||
        from.includes('..') || to.includes('..')) {
        return '错误: 文件名不合法';
      }
      const srcPath = `${ToolRegistry.docsDir()}/${from}`;
      const dstPath = `${ToolRegistry.docsDir()}/${to}`;
      if (!fs.accessSync(srcPath)) {
        return `没有找到文件 ${from}`;
      }
      if (fs.accessSync(dstPath)) {
        return `目标文件名 ${to} 已存在，请先换个名字`;
      }
      fs.renameSync(srcPath, dstPath);
      return `已重命名 ${from} → ${to}`;
    });

    ToolRegistry.register(new ToolDef('export_document', '把生成的文档导出到手机公共目录（弹出系统保存对话框，用户可选位置，之后可在文件管理器查看/分享）', [
      { name: 'name', type: 'string', description: '文件名（list_files 中的名字）', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const name = ToolRegistry.str(args, 'name', '');
      if (name.length === 0 || name.includes('/') || name.includes('..')) {
        return '错误: 文件名不合法';
      }
      const path = `${ToolRegistry.docsDir()}/${name}`;
      if (!fs.accessSync(path)) {
        return `没有找到文件 ${name}`;
      }
      const docPicker = new picker.DocumentViewPicker(AppContext.getContext());
      const opts = new picker.DocumentSaveOptions();
      opts.newFileNames = [name];
      const uris: string[] = await docPicker.save(opts);
      if (uris.length === 0) {
        return '用户取消了保存对话框';
      }
      const src = fs.openSync(path, fs.OpenMode.READ_ONLY);
      const dest = fs.openSync(uris[0], fs.OpenMode.READ_WRITE | fs.OpenMode.TRUNC);
      try {
        await fs.copyFile(src.fd, dest.fd);
      } finally {
        fs.closeSync(src);
        fs.closeSync(dest);
      }
      return `已导出「${name}」，可在文件管理器的"最近"里看到它`;
    });
  }
}

/**
 * 安全表达式解析器（递归下降）：数字/小数点、+ - * / % ^ 与括号，
 * 不经过 eval/Function，杜绝代码注入。
 */
class CalcParser {
  private src: string;
  private pos: number = 0;

  constructor(src: string) {
    this.src = src;
  }

  parse(): number {
    const v = this.expr();
    this.skipWs();
    if (this.pos < this.src.length) {
      throw new Error('表达式在 ' + this.src.substring(this.pos) + ' 处无法解析');
    }
    return v;
  }

  private skipWs(): void {
    while (this.pos < this.src.length && this.src.charAt(this.pos) === ' ') {
      this.pos++;
    }
  }

  private expr(): number {
    let v = this.term();
    for (;;) {
      this.skipWs();
      const c = this.src.charAt(this.pos);
      if (c === '+') {
        this.pos++;
        v += this.term();
      } else if (c === '-') {
        this.pos++;
        v -= this.term();
      } else {
        break;
      }
    }
    return v;
  }

  private term(): number {
    let v = this.power();
    for (;;) {
      this.skipWs();
      const c = this.src.charAt(this.pos);
      if (c === '*') {
        this.pos++;
        v *= this.power();
      } else if (c === '/') {
        this.pos++;
        const d = this.power();
        if (d === 0) {
          throw new Error('除数为 0');
        }
        v /= d;
      } else if (c === '%') {
        this.pos++;
        const d = this.power();
        if (d === 0) {
          throw new Error('除数为 0');
        }
        v %= d;
      } else {
        break;
      }
    }
    return v;
  }

  private power(): number {
    const base = this.unary();
    this.skipWs();
    if (this.src.charAt(this.pos) === '^') {
      this.pos++;
      return Math.pow(base, this.power());
    }
    return base;
  }

  private unary(): number {
    this.skipWs();
    const c = this.src.charAt(this.pos);
    if (c === '-') {
      this.pos++;
      return -this.unary();
    }
    if (c === '+') {
      this.pos++;
      return this.unary();
    }
    return this.primary();
  }

  private primary(): number {
    this.skipWs();
    if (this.src.charAt(this.pos) === '(') {
      this.pos++;
      const v = this.expr();
      this.skipWs();
      if (this.src.charAt(this.pos) !== ')') {
        throw new Error('缺少右括号');
      }
      this.pos++;
      return v;
    }
    const start = this.pos;
    while (this.pos < this.src.length &&
      (CalcParser.isDigit(this.src.charAt(this.pos)) || this.src.charAt(this.pos) === '.')) {
      this.pos++;
    }
    if (this.pos === start) {
      throw new Error('表达式格式错误');
    }
    const n = Number(this.src.substring(start, this.pos));
    if (Number.isNaN(n)) {
      throw new Error('数字解析失败');
    }
    return n;
  }

  private static isDigit(c: string): boolean {
    return c >= '0' && c <= '9';
  }
}
