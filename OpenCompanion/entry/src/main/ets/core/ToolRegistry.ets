import fs from '@ohos.file.fs';
import { util } from '@kit.ArkTS';
import { reminderAgentManager } from '@kit.BackgroundTasksKit';
import { deviceInfo } from '@kit.BasicServicesKit';
import { AppContext } from '../commons/AppContext';
import { RdbHelper } from '../commons/RdbHelper';
import { NetClient } from '../commons/NetClient';
import { Log } from '../commons/Log';
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
  static readonly HIGH_RISK: string[] = ['delete_memory', 'read_file'];

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
      return decoder.decodeWithStream(new Uint8Array(buf));
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

    ToolRegistry.register(new ToolDef('save_note', '保存笔记到应用文档目录（Markdown）', [
      { name: 'title', type: 'string', description: '笔记标题', required: true } as ToolParam,
      { name: 'content', type: 'string', description: 'Markdown 内容', required: true } as ToolParam
    ]), async (args: Record<string, Object>) => {
      const title = ToolRegistry.str(args, 'title', `note_${Date.now()}`);
      const content = ToolRegistry.str(args, 'content', '');
      const path = await ToolRegistry.writeDocFile(`${title.replace(/[\\/:*?"<>|]/g, '_')}.md`, content);
      return `已保存笔记: ${path}`;
    });

    ToolRegistry.register(new ToolDef('list_files', '列出应用文档目录下的文件', []), async (): Promise<string> => {
      const dir = ToolRegistry.docsDir();
      const files = fs.listFileSync(dir);
      if (files.length === 0) {
        return '文档目录为空';
      }
      return files.join('\n');
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
  }
}
