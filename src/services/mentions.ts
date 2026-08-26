/**
 * @ 提及解析：输入框中输入 @ 时弹出智能体列表，发送时提取被提及的智能体名字。
 * 智能体名字不含空白字符（后端已强校验），因此用空白作为名字的边界。
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
  const match = /@([^s@]*)$/.exec(value);
  if (!match) return null;
  return { query: match[1], start: match.index };
}

/** 提取消息中所有 @名字 提及（去重、保持出现顺序）。 */
export function extractMentionNames(value: string): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  const re = /@([^s@]+)/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(value)) !== null) {
    const name = match[1];
    if (!seen.has(name)) {
      seen.add(name);
      names.push(name);
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
export function splitMentionSegments(content: string): MentionSegment[] {
  const segments: MentionSegment[] = [];
  const re = /@([^s@]+)/g;
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
