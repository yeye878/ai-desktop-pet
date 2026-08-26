/**
 * AI 回复中的引用标记解析器。
 *
 * AI 被（system prompt）告知：当需要明确引用之前对话中的某段原文时，
 * 可以把原文放在 [QUOTE]...[/QUOTE] 之间。前端将标记渲染成微信式引用块。
 *
 * 解析规则：
 * - 标记大小写不敏感（[QUOTE] / [quote] 均可）
 * - 已闭合的 [QUOTE]...[/QUOTE] → quote 段
 * - 流式渲染（streaming = true）时，未闭合的 [QUOTE] 之后的内容也临时按
 *   quote 段渲染，等 [/QUOTE] 到达后自然闭合，视觉上是"引用块正在生长"
 * - 非流式渲染时，未闭合的孤立 [QUOTE] 标记直接剔除，内容按普通文本处理
 */

export interface QuoteSegment {
  type: "text" | "quote";
  content: string;
}

const QUOTE_PATTERN = /\[QUOTE\]([\s\S]*?)\[\/QUOTE\]/gi;
const OPEN_TAG_PATTERN = /\[QUOTE\]/i;

export function parseQuoteSegments(text: string, streaming = false): QuoteSegment[] {
  if (!text) return [];

  const segments: QuoteSegment[] = [];
  let lastIndex = 0;

  QUOTE_PATTERN.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = QUOTE_PATTERN.exec(text)) !== null) {
    if (match.index > lastIndex) {
      segments.push({ type: "text", content: text.slice(lastIndex, match.index) });
    }
    segments.push({ type: "quote", content: match[1] });
    lastIndex = QUOTE_PATTERN.lastIndex;
  }

  const rest = text.slice(lastIndex);
  if (rest) {
    const openTag = OPEN_TAG_PATTERN.exec(rest);
    if (openTag) {
      if (openTag.index > 0) {
        segments.push({ type: "text", content: rest.slice(0, openTag.index) });
      }
      const unclosed = rest.slice(openTag.index + openTag[0].length);
      if (streaming) {
        // 流式中：标记后的内容临时按引用块渲染，等待闭合
        segments.push({ type: "quote", content: unclosed });
      } else {
        // 非流式：孤立未闭合标记剔除，内容按普通文本展示
        if (unclosed) segments.push({ type: "text", content: unclosed });
      }
    } else {
      segments.push({ type: "text", content: rest });
    }
  }

  // 合并相邻 text 段，避免渲染出多余节点
  const merged: QuoteSegment[] = [];
  for (const seg of segments) {
    if (!seg.content) continue;
    const prev = merged[merged.length - 1];
    if (seg.type === "text" && prev?.type === "text") {
      prev.content += seg.content;
    } else {
      merged.push({ ...seg });
    }
  }
  return merged;
}

/** 剔除 [QUOTE] 标记后的纯文本（用于复制、TTS 朗读等场景） */
export function stripQuoteMarkers(text: string): string {
  return parseQuoteSegments(text)
    .map((seg) => seg.content)
    .join("");
}

/** 引用内容传给 AI 时的长度上限，避免超长消息挤占上下文 */
export const QUOTE_CONTENT_MAX_LENGTH = 2000;

/** 超长引用内容截断，尾部加省略提示 */
export function truncateQuoteContent(content: string): string {
  if (content.length <= QUOTE_CONTENT_MAX_LENGTH) return content;
  return `${content.slice(0, QUOTE_CONTENT_MAX_LENGTH)}\n…（原文过长已截断）`;
}
