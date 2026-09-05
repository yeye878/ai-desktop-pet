/**
 * @ 提及解析：输入框中输入 @ 时弹出智能体列表，发送时提取被提及的智能体名字。
 * 智能体名字不含空白字符（后端已强校验），因此用空白和常见标点作为名字的边界。
 */

export interface MentionSegment {
  type: "text" | "mention";
  content: string;
}

export interface MentionQuery {
  /** 正在输入的查询串（@ 之后、尚未闭合的名字部分） */
  query: string;
  /** @ 在输入串中的字符下标 */
  start: number;
}

/**
 * 判断输入框末尾是否正在输入一个 @ 提及。
 * 仅当最后一个 @ 之后全是非空白且 @ 不在其它词中间时才生效。
 */
export function mentionQueryFromInput(value: string): MentionQuery | null {
  const match = /@([^\s@]*)$/.exec(value);
  if (!match) return null;
  return { query: match[1], start: match.index };
}

/** 常见标点字符集，用于把 @名字 后的标点（如逗号、句号、冒号等）剥离出名字范围 */
const TRAILING_PUNCTUATION_REGEX = /[，。、：；！？,:;!?]+$/;

/** 提取消息中所有 @名字 提及（去重、保持出现顺序）。 */
export function extractMentionNames(value: string, knownNames?: string[]): string[] {
  if (!value) return [];
  const names: string[] = [];
  const seen = new Set<string>();

  if (knownNames && knownNames.length > 0) {
    // 若提供了已知智能体名字列表，优先按已知名字匹配（最长匹配优先）
    const sorted = [...knownNames].sort((a, b) => b.length - a.length);
    for (const name of sorted) {
      if (!name) continue;
      const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const re = new RegExp(`@${escaped}(?=[\\s，。、：；！？,:;!?]|$)`, "g");
      if (re.test(value) && !seen.has(name)) {
        seen.add(name);
        names.push(name);
      }
    }
    return names;
  }

  const re = /@([^\s@]+)/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(value)) !== null) {
    const raw = match[1].replace(TRAILING_PUNCTUATION_REGEX, "").trim();
    if (raw && !seen.has(raw)) {
      seen.add(raw);
      names.push(raw);
    }
  }
  return names;
}

/**
 * 把选中的智能体名字插入输入框：替换掉当前正在输入的 @ 片段，
 * 保留其余内容不变。
 */
export function insertMentionAt(
  value: string,
  query: MentionQuery,
  agentName: string,
): string {
  return value.slice(0, query.start) + "@" + agentName + " " + value.slice(query.start + 1 + query.query.length);
}

/** 把消息内容按 @提及 切成渲染片段（用于在气泡中高亮 @名字）。 */
export function splitMentionSegments(content: string, knownNames?: string[]): MentionSegment[] {
  if (!content) return [];
  const segments: MentionSegment[] = [];

  if (knownNames && knownNames.length > 0) {
    const sorted = [...knownNames]
      .filter((n) => n.trim().length > 0)
      .sort((a, b) => b.length - a.length);
    if (sorted.length > 0) {
      const pattern = sorted.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|");
      const re = new RegExp(`@(${pattern})(?=[\\s，。、：；！？,:;!?]|$)`, "g");
      let last = 0;
      let match: RegExpExecArray | null;
      while ((match = re.exec(content)) !== null) {
        if (match.index > last) {
          segments.push({ type: "text", content: content.slice(last, match.index) });
        }
        segments.push({ type: "mention", content: match[1] });
        last = match.index + match[0].length;
      }
      if (last < content.length) {
        segments.push({ type: "text", content: content.slice(last) });
      }
      return segments;
    }
  }

  const re = /@([^\s@，。、：；！？,:;!?]+)/g;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(content)) !== null) {
    if (match.index > last) {
      segments.push({ type: "text", content: content.slice(last, match.index) });
    }
    segments.push({ type: "mention", content: match[1] });
    last = match.index + match[0].length;
  }
  if (last < content.length) {
    segments.push({ type: "text", content: content.slice(last) });
  }
  return segments;
}

export interface AgentBlock {
  type: "text" | "agent_header";
  agentName?: string;
  avatar?: string;
  content: string;
}

/**
 * 解析多智能体回复中的 【智能体名字】 分段标题行。
 * 让多智能体辩论/合作回复在界面中展现清晰的分段与专属头像徽章。
 */
export function parseAgentHeaders(
  text: string,
  agents: Array<{ name: string; avatar?: string }> = [],
): AgentBlock[] {
  if (!text) return [];
  const re = /(?:^|\n) *【([^】\n]+)】 *(?:\n|$)/g;
  const blocks: AgentBlock[] = [];
  let last = 0;
  let match: RegExpExecArray | null;

  while ((match = re.exec(text)) !== null) {
    if (match.index > last) {
      blocks.push({ type: "text", content: text.slice(last, match.index) });
    }
    const name = match[1].trim();
    const found = agents.find((a) => a.name === name);
    blocks.push({
      type: "agent_header",
      agentName: name,
      avatar: found?.avatar || "🤖",
      content: match[0],
    });
    last = match.index + match[0].length;
  }
  if (last < text.length) {
    blocks.push({ type: "text", content: text.slice(last) });
  }
  return blocks.length > 0 ? blocks : [{ type: "text", content: text }];
}


