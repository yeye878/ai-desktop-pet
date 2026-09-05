/**
 * 核心数据模型（对应桌面版 direct_api/agent.rs 与 storage 的数据结构）
 */

/** 一条聊天消息（UI 展示与 RDB chat_history 同构） */
export class ChatMessage {
  id: string = '';
  role: string = 'user';            // user | assistant
  content: string = '';
  thinking: string = '';
  agentName: string = '';
  agentAvatar: string = '';
  toolEvents: string[] = [];
  imageUris: string[] = [];
  quote: string = '';
  createdAt: number = 0;

  static newId(): string {
    return `m${Date.now()}_${Math.floor(Math.random() * 100000)}`;
  }
}

/** 子智能体定义（对应 agents 表） */
export class AgentProfile {
  id: string = '';
  name: string = '';
  avatar: string = '🤖';
  description: string = '';
  systemPrompt: string = '';
  allowedTools: string[] = [];
  isBuiltin: boolean = false;
}

/** 技能定义（对应 skills 表） */
export class SkillProfile {
  id: string = '';
  name: string = '';
  description: string = '';
  systemPrompt: string = '';
  allowedTools: string[] = [];
  isActive: boolean = false;
}

/** LLM API 配置（对应 direct_api DirectApiConfig；密钥仅运行期持有，持久化在系统资产存储） */
export class LlmProfile {
  id: string = '';
  name: string = '';
  baseUrl: string = 'https://api.openai.com/v1';
  model: string = 'gpt-4o-mini';
  temperature: number = 0.7;
  useStream: boolean = true;
  confirmEnabled: boolean = true;
  apiKey: string = '';
}

/** 工具参数 schema */
export class ToolParam {
  name: string = '';
  type: string = 'string';          // string | number | boolean
  description: string = '';
  required: boolean = false;
}

/** 工具定义 */
export class ToolDef {
  name: string = '';
  description: string = '';
  parameters: ToolParam[] = [];

  constructor(name: string, description: string, parameters: ToolParam[]) {
    this.name = name;
    this.description = description;
    this.parameters = parameters;
  }
}

/** OpenAI 工具调用请求（流式增量聚合后的完整对象） */
export class ToolCallReq {
  id: string = '';
  name: string = '';
  arguments: string = '';
}

/** 多模态内容分片 */
export class ContentPart {
  type: string = 'text';            // text | image_url
  text: string = '';
  imageUrl: string = '';
}

/** 发送给 LLM 的消息（OpenAI chat 协议同构） */
export class LlmMessage {
  role: string = 'user';            // system | user | assistant | tool
  content: string = '';
  parts: ContentPart[] | null = null;
  toolCalls: ToolCallReq[] | null = null;
  toolCallId: string = '';
}

export class AskOption {
  label: string = '';
  description: string = '';

  constructor(label: string, description: string = '') {
    this.label = label;
    this.description = description;
  }
}

/** 取消令牌（对应桌面版 CancellationToken / run_id 机制） */
export class CancelToken {
  cancelled: boolean = false;
  private hooks: Array<() => void> = [];

  cancel(): void {
    if (this.cancelled) {
      return;
    }
    this.cancelled = true;
    this.hooks.forEach((f: () => void) => {
      try {
        f();
      } catch (e) {
        // ignore
      }
    });
  }

  onCancel(hook: () => void): void {
    this.hooks.push(hook);
  }

  /** 抛出中断异常 */
  checkThrow(): void {
    if (this.cancelled) {
      throw new Error('__ABORTED__');
    }
  }
}

/** JSON 容错工具 */
export class JsonUtil {
  /** 从模型输出中提取第一个平衡的 JSON 对象文本 */
  static extractJson(text: string): string | null {
    const start = text.indexOf('{');
    if (start < 0) {
      return null;
    }
    let depth = 0;
    let inStr = false;
    let escape = false;
    for (let i = start; i < text.length; i++) {
      const ch = text.charAt(i);
      if (escape) {
        escape = false;
        continue;
      }
      if (ch === '\\') {
        escape = true;
        continue;
      }
      if (ch === '"') {
        inStr = !inStr;
        continue;
      }
      if (inStr) {
        continue;
      }
      if (ch === '{') {
        depth++;
      } else if (ch === '}') {
        depth--;
        if (depth === 0) {
          return text.substring(start, i + 1);
        }
      }
    }
    return null;
  }

  static parseObj(text: string): Record<string, Object> | null {
    try {
      return JSON.parse(text) as Record<string, Object>;
    } catch (e) {
      return null;
    }
  }

  static getString(obj: Record<string, Object> | null, key: string, def: string): string {
    if (!obj) {
      return def;
    }
    const v = obj[key];
    if (typeof v === 'string') {
      return v as string;
    }
    return def;
  }

  static getNumber(obj: Record<string, Object> | null, key: string, def: number): number {
    if (!obj) {
      return def;
    }
    const v = obj[key];
    if (typeof v === 'number') {
      return v as number;
    }
    return def;
  }

  static getStringArray(obj: Record<string, Object> | null, key: string): string[] {
    const out: string[] = [];
    if (!obj) {
      return out;
    }
    const v = obj[key];
    if (Array.isArray(v)) {
      (v as Array<Object>).forEach((it: Object) => {
        if (typeof it === 'string') {
          out.push(it as string);
        }
      });
    }
    return out;
  }
}
