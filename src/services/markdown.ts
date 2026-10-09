/**
 * Markdown 渲染器：把 AI 回复渲染成带排版的 HTML。
 *
 * 背景：AI 回复几乎都是 Markdown（**粗体**、# 标题、- 列表、[链接](url)……），
 * 而气泡此前是纯文本插值，用户满屏看到的都是 * 和 #。这里把 Markdown 转成
 * 安全的 HTML，排版样式见 global.css 的 .md-body。
 *
 * 设计取舍：
 * - 不引第三方依赖：只生成下面这几种固定标签，文本一律转义、链接协议走白名单，
 *   因此不存在注入面（原始 HTML 会被转义成纯文本显示）。
 * - 流式安全：对"半截 Markdown"（未闭合的 ** 或代码围栏）保持宽松，未闭合的
 *   标记按普通字符输出，不会在逐字刷新时闪出奇怪的结构。
 * - @提及 高亮内建在行内解析里：命中占位符时输出 <span class="md-mention">，
 *   名字一定被转义，也不会被 Markdown 当成别的语法吃掉。
 * - 公式与代码段先换成占位符再解析（见 math.ts / protectText），
 *   这样 \frac、\begin{cases} 里的反斜杠、下划线和花括号不会被 Markdown 破坏。
 */

import { extractMathTokens, restoreMathTokens, type MathToken } from "./math";

/** 生成 HTML 时，命中占位符要输出的标签与高亮类名 */
export interface MentionTag {
  /** 高亮 @名字 用的 span 标签，默认气泡里的 .bubble-mention */
  htmlTag?: string;
  /** 纯文本模式下写在名字前面的符号，默认 "@" */
  plainPrefix?: string;
}

/** Markdown 解析上下文：被 @ 提及的名字 + HTML 渲染设置 */
export interface MarkdownContext {
  names?: string[];
  mention?: MentionTag;
}

/** 消息渲染分段：引用块与 Markdown 正文交替出现 */
export interface MessageRenderSegment {
  type: "text" | "quote";
  html: string;
}

interface ParsedContext {
  names: string[];
  boundaries: Set<number>;
  /** 提及名字 → HTML 实体形式的占位符，保证名字不会被当成 Markdown 语法 */
  protectedNames: Map<string, string>;
  mentionHtml: MentionTag | undefined;
}

/**
 * HTML 标签工厂：返回标签字符串，由行内解析器拼进结果里。
 * 纯文本模式下传 null，所有标签都不产生。
 */
interface Emitter {
  open(tag: string): string;
  close(tag: string): string;
}

type Block =
  | { kind: "p"; text: string }
  | { kind: "h"; level: number; text: string }
  | { kind: "hr" }
  | { kind: "code"; lang: string; lines: string[] }
  | { kind: "quote"; lines: string[] }
  | { kind: "list"; ordered: boolean; start: number; items: string[] }
  | { kind: "table"; header: string[]; rows: string[][] };

const ESCAPE_PATTERN = /[&<>"']/g;
const ESCAPE_MAP: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};
/** 行内剩余文本里可能开启语法的字符，用于一次性跳过纯文本段 */
const INLINE_SPECIAL = /[\\`*_~\[!<@h\n]/;
/**
 * @提及 用的名字：中文/英数下划线点横线，首尾必须是这类字符。
 * 这样 "@代码助手，" 只会取到"代码助手"，中文标点不会被吃进名字里。
 */
const MENTION_PATTERN = /^@([A-Za-z0-9_.-]*[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff][A-Za-z0-9_.-\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]*)/;
const ORDERED_MARKER = /^(\d{1,9})[.)]\s+(.*)$/;
const UNORDERED_MARKER = /^([-+*])\s+(.*)$/;
const TABLE_DIVIDER = /^\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?$/;
const HR_PATTERN = /^(?:-{3,}|\*{3,}|_{3,})(?:\s+(?:-{3,}|\*{3,}|_{3,}))*$/;
const FENCE_PATTERN = /^\s{0,3}(`{3,}|~{3,})\s*([^\s`]*)\s*$/;
const CACHE_LIMIT = 200;
/** 单条缓存正文的长度上限，超过就每次现算（长回复不值得常驻内存） */
const CACHE_ENTRY_MAX_LENGTH = 4000;
const cache = new Map<string, string>();

export const MARKDOWN_MENTION_CLASS = "md-mention";

interface ProtectedText {
  text: string;
  /** 公式占位符：HTML 模式渲染成 KaTeX，纯文本模式降级成可读文字 */
  mathTokens: MathToken[];
  /** 把占位符还原回代码段 / 行内代码，并把公式渲染成 HTML */
  restore: (chunk: string) => string;
}

/**
 * 抽出公式并保护代码段。
 * 代码段与公式都换成占位符，Markdown 解析器只会看到占位符：
 * 代码里的 * 不会被当成强调，公式里的反斜杠命令、下划线和花括号也不会被吃掉。
 */
function protectText(source: string): ProtectedText {
  const blocks: string[] = [];
  const inlines: string[] = [];
  const stash = (list: string[], value: string) => {
    list.push(value);
    return "\uE000" + (list === blocks ? "B" : "I") + (list.length - 1) + "\uE001";
  };

  // 逐行扫描而不是整体正则：围栏代码块整块替换，块内的 $ 和反斜杠不会被误判成公式
  const kept: string[] = [];
  const pending: string[] = [];
  let fence: string | null = null;
  for (const line of source.split("\n")) {
    const marker = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence === null && marker) {
      fence = marker[1][0];
      pending.length = 0;
      pending.push(line);
      continue;
    }
    if (fence !== null) {
      pending.push(line);
      if (marker && marker[1][0] === fence) {
        kept.push(stash(blocks, pending.join("\n")));
        fence = null;
        pending.length = 0;
      }
      continue;
    }
    kept.push(line);
  }
  if (fence !== null) kept.push(stash(blocks, pending.join("\n")));

  const withCode = kept
    .join("\n")
    // 行内代码整段换掉：反引号里的 * 和 _ 不再被当作 Markdown 语法
    .replace(/`[^`\n]+`/g, (value) => stash(inlines, value));

  // 公式先抽出来（此时围栏代码还是占位符，块内的 $ 和反斜杠不会被误判）
  const math = extractMathTokens(withCode);
  // 围栏代码块要还原成原文再交给块级解析，这样代码块仍然渲染成 <pre><code>
  const text = math.text
    .split(/\uE000B(\d+)\uE001/)
    .map((part, index) => (index % 2 === 1 ? blocks[Number(part)] ?? "" : part))
    .join("");

  return {
    text,
    mathTokens: math.tokens,
    restore: (chunk) =>
      restoreMathTokens(chunk, math.tokens, true)
        .split(/\uE000I(\d+)\uE001/)
        .map((part, index) => {
          if (index % 2 === 0) return part;
          // 行内代码在这里才变成 <code>，块级解析阶段它只是一个不透明的占位符
          return "<code>" + escapeHtml(inlines[Number(part)]?.slice(1, -1) ?? "") + "</code>";
        })
        .join(""),
  };
}

function escapeHtml(value: string): string {
  return value.replace(ESCAPE_PATTERN, (char) => ESCAPE_MAP[char] ?? char);
}

/** 只允许 http/https/mailto 与相对链接；其余协议（javascript: data: …）一律丢弃 */
function safeUrl(raw: string): string | null {
  const url = raw.trim();
  if (!url) return null;
  if (url.startsWith("//")) return null;
  const scheme = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(url);
  if (scheme) {
    const name = scheme[1].toLowerCase();
    if (name !== "http" && name !== "https" && name !== "mailto") return null;
  }
  return url;
}

/** 解析链接目标：<...> 形式优先，否则吃括号平衡的裸串，末尾标点不计入 */
function parseUrl(raw: string, start: number): { url: string; end: number } | null {
  let i = start;
  while (i < raw.length && (raw[i] === " " || raw[i] === "\t")) i += 1;
  if (raw[i] === "<") {
    const close = raw.indexOf(">", i + 1);
    if (close === -1) return null;
    return { url: raw.slice(i + 1, close), end: close + 1 };
  }
  let depth = 0;
  let cursor = i;
  while (cursor < raw.length) {
    const char = raw[cursor];
    if (char === "(") depth += 1;
    else if (char === ")") {
      if (depth === 0) break;
      depth -= 1;
    } else if (char === " " || char === "\t" || char === "\n") break;
    cursor += 1;
  }
  if (cursor === i) return null;
  let end = cursor;
  while (end > i && /[.,;:!?，。；：！？]/.test(raw[end - 1])) end -= 1;
  if (end === i) return null;
  return { url: raw.slice(i, end), end: cursor };
}

function isBlank(char: string | undefined): boolean {
  return char === undefined || char === " " || char === "\t" || char === "\n";
}

/** 强调符内外必须有内容：** 后面是空白、闭合 ** 前面是空白都不算 */
function findDelimiter(raw: string, marker: string, from: number): { start: number; end: number } | null {
  let index = from;
  while (index < raw.length) {
    const found = raw.indexOf(marker, index);
    if (found === -1) return null;
    if (found > from && !isBlank(raw[found - 1])) return { start: found, end: found + marker.length };
    index = found + marker.length;
  }
  return null;
}

function buildContext(text: string, context: MarkdownContext | undefined): ParsedContext {
  const names = (context?.names ?? []).filter((name) => name.length > 0);
  const boundaries = new Set<number>();
  if (names.length > 0) {
    const pattern = /(^|\s)@/g;
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(text)) !== null) {
      boundaries.add(match.index + match[1].length);
    }
  }
  const protectedNames = new Map<string, string>();
  for (const name of names) {
    if (!protectedNames.has(name)) protectedNames.set(name, escapeHtml(name));
  }
  return { names, boundaries, protectedNames, mentionHtml: context?.mention };
}

function parseInline(state: ParsedContext, emitter: Emitter | null, source: string): string {
  const raw = source;
  let out = "";
  let pos = 0;

  /** 标签只在 HTML 模式下落地，纯文本模式直接返回内容 */
  const wrap = (inner: string, tagName: string) =>
    emitter ? emitter.open(tagName) + inner + emitter.close(tagName) : inner;

  while (pos < raw.length) {
    const char = raw[pos];

    if (char === "\\" && pos + 1 < raw.length && /[\\`*_~\[\]()#>!|.-]/.test(raw[pos + 1])) {
      out += emitter ? escapeHtml(raw[pos + 1]) : raw[pos + 1];
      pos += 2;
      continue;
    }

    if (char === "`") {
      const fence = /^`+/.exec(raw.slice(pos))?.[0] ?? "`";
      const close = raw.indexOf(fence, pos + fence.length);
      if (close !== -1) {
        const code = raw.slice(pos + fence.length, close);
        out += wrap(escapeHtml(code), "code");
        pos = close + fence.length;
        continue;
      }
      out += emitter ? escapeHtml(fence) : fence;
      pos += fence.length;
      continue;
    }

    if (char === "!" || char === "[") {
      const isImage = char === "!";
      const bracket = pos + (isImage ? 1 : 0);
      if (raw[bracket] === "[") {
        const labelEnd = raw.indexOf("]", bracket + 1);
        if (labelEnd !== -1 && raw[labelEnd + 1] === "(") {
          const target = parseUrl(raw, labelEnd + 2);
          if (target && raw[target.end] === ")") {
            const label = raw.slice(bracket + 1, labelEnd);
            const href = isImage ? null : safeUrl(target.url);
            if (isImage) {
              // 不加载远程图片，退化成 alt 文本
              out += emitter ? '<span class="md-image">' + escapeHtml(label) + "</span>" : label;
            } else if (href) {
              out += wrap(parseInline(state, emitter, label), 'a class="md-link" href="' + escapeHtml(href) + '" target="_blank" rel="noreferrer noopener"');
            } else {
              out += parseInline(state, emitter, label);
            }
            pos = target.end + 1;
            continue;
          }
        }
      }
    }

    if (char === "h" && raw.startsWith("http", pos)) {
      // 前面还有没闭合的 [ 或 ( 时不自动链接：链接标签写到一半时地址不能先变成链接，
      // 否则流式渲染会把 "[文档](https://…" 撕成两截
      const prefix = raw.slice(0, pos);
      const bracketsOpen = (prefix.match(/\[/g)?.length ?? 0) - (prefix.match(/\]/g)?.length ?? 0);
      const parensOpen = (prefix.match(/\(/g)?.length ?? 0) - (prefix.match(/\)/g)?.length ?? 0);
      const insideUnclosed = bracketsOpen > 0 || parensOpen > 0;
      const match = insideUnclosed ? null : /^https?:\/\/[^\s<>()\[\]"'，。；！？]+/.exec(raw.slice(pos));
      if (match) {
        const href = safeUrl(match[0]);
        out += href ? wrap(escapeHtml(match[0]), 'a class="md-link" href="' + escapeHtml(href) + '" target="_blank" rel="noreferrer noopener"') : emitter ? escapeHtml(match[0]) : match[0];
        pos += match[0].length;
        continue;
      }
    }

    if (char === "@" && state.names.length > 0 && state.boundaries.has(pos)) {
      const match = MENTION_PATTERN.exec(raw.slice(pos));
      const name = match?.[1];
      const token = name && state.names.includes(name) ? state.protectedNames.get(name) : undefined;
      if (name && token) {
        if (emitter) {
          out += wrap(token, state.mentionHtml?.htmlTag ?? '<span class="' + MARKDOWN_MENTION_CLASS + '">');
        } else {
          out += (state.mentionHtml?.plainPrefix ?? "@") + name;
        }
        pos += 1 + name.length;
        continue;
      }
    }

    if (char === "*" || char === "_") {
      const triple = raw.startsWith(char.repeat(3), pos);
      const double = !triple && raw.startsWith(char.repeat(2), pos);
      const marker = triple ? char.repeat(3) : double ? char.repeat(2) : char;
      if (!isBlank(raw[pos + marker.length])) {
        const close = findDelimiter(raw, marker, pos + marker.length);
        if (close) {
          const inner = parseInline(state, emitter, raw.slice(pos + marker.length, close.start));
          out += triple ? wrap(wrap(inner, "em"), "strong") : wrap(inner, double ? "strong" : "em");
          pos = close.end;
          continue;
        }
      }
    }

    if (char === "~" && raw.startsWith("~~", pos)) {
      const close = raw.indexOf("~~", pos + 2);
      if (close > pos + 2) {
        out += wrap(parseInline(state, emitter, raw.slice(pos + 2, close)), 'del class="md-del"');
        pos = close + 2;
        continue;
      }
    }

    if (char === "<") {
      if (emitter && /^<\/?[a-zA-Z][^>\n]*>/.test(raw.slice(pos))) {
        const close = raw.indexOf(">", pos);
        out += escapeHtml(raw.slice(pos, close + 1));
        pos = close + 1;
        continue;
      }
      if (/^https?:\/\/[^\s<>]+>/.test(raw.slice(pos))) {
        const close = raw.indexOf(">", pos);
        const href = safeUrl(raw.slice(pos + 1, close));
        if (href) {
          out += wrap(escapeHtml(href), 'a class="md-link" href="' + escapeHtml(href) + '" target="_blank" rel="noreferrer noopener"');
        }
        pos = close + 1;
        continue;
      }
      out += emitter ? "&lt;" : "<";
      pos += 1;
      continue;
    }

    if (char === "\n") {
      out += emitter ? "<br />" : "\n";
      pos += 1;
      continue;
    }

    const next = raw.slice(pos).search(INLINE_SPECIAL);
    // search 命中 0 表示当前字符本身是语法字符，但上面所有分支都没接受它
    // （孤立的 * 、未命中的 @ 、落单的 <），此时必须原样输出再前进一位，
    // 否则这个字符会被静默吞掉。
    if (next === 0) {
      out += emitter ? escapeHtml(char) : char;
      pos += 1;
      continue;
    }
    const end = next === -1 ? raw.length : pos + next;
    const chunk = raw.slice(pos, end);
    out += emitter ? escapeHtml(chunk) : chunk;
    pos = next === -1 ? raw.length : end;
  }

  return out;
}

function stripTrailingBlank(lines: string[]): string[] {
  const result = [...lines];
  while (result.length > 0 && !result[result.length - 1].trim()) result.pop();
  return result;
}

function isTableSeparator(line: string): boolean {
  return line.includes("-") && TABLE_DIVIDER.test(line.trim());
}

function splitTableRow(line: string): string[] {
  let value = line.trim();
  if (value.startsWith("|")) value = value.slice(1);
  if (value.endsWith("|")) value = value.slice(0, -1);
  if (!value.includes("|")) return [value.trim()];
  return value.split("|").map((cell) => cell.trim());
}

/** 表格分隔行必须和表头列数一致，否则不按表格解析（此时段落里必须有表头行） */
function matchTableDivider(raw: string, paragraph: string[]): boolean {
  if (paragraph.length === 0) return false;
  const header = splitTableRow(paragraph[paragraph.length - 1]);
  const divider = splitTableRow(raw);
  return header.length > 1 && header.length === divider.length;
}

function parseList(lines: string[]): Block {
  const ordered = ORDERED_MARKER.test(lines[0]);
  const widths: number[] = [];
  for (const line of lines) {
    const match = /^(\s*)(?:[-+*]|\d{1,9}[.)])\s+/.exec(line);
    if (match) widths.push(match[1].length);
  }
  const base = widths.length > 0 ? Math.min(...widths) : 0;
  const items: string[] = [];
  let current: string[] | null = null;
  for (const line of lines) {
    const match = /^(\s*)(?:[-+*]|\d{1,9}[.)])\s+(.*)$/.exec(line);
    if (match && match[1].length <= base + 1) {
      // 同级新条目：行的缩进不超过基准宽度，就是新的一项
      if (current) items.push(stripTrailingBlank(current).join("\n"));
      current = [match[2]];
    } else if (current) {
      current.push(match ? line.slice(Math.min(line.length, base + 2)) : line);
    } else {
      current = [line];
    }
  }
  if (current) items.push(stripTrailingBlank(current).join("\n"));
  return { kind: "list", ordered, start: ordered ? Number.parseInt(lines[0], 10) || 1 : 1, items };
}

/** 解析块级结构：段落、标题、围栏代码、引用、列表、表格、分割线 */
function parseBlocks(source: string): Block[] {
  const lines = source.split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];

  const flushParagraph = () => {
    if (paragraph.length > 0) {
      blocks.push({ kind: "p", text: paragraph.join("\n").trim() });
      paragraph = [];
    }
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];

    const fence = FENCE_PATTERN.exec(line);
    if (fence) {
      flushParagraph();
      const marker = fence[1][0];
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !new RegExp("^\\s{0,3}" + marker + "{3,}\\s*$").test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      blocks.push({ kind: "code", lang: fence[2] ?? "", lines: body });
      continue;
    }

    if (!line.trim()) {
      flushParagraph();
      continue;
    }

    const heading = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (heading) {
      flushParagraph();
      blocks.push({ kind: "h", level: heading[1].length, text: heading[2] });
      continue;
    }

    const setext = /^\s{0,3}(=+|-+)\s*$/.exec(line);
    if (setext && paragraph.length > 0) {
      const text = paragraph.join("\n").trim();
      paragraph = [];
      blocks.push({ kind: "h", level: setext[1][0] === "=" ? 1 : 2, text });
      continue;
    }

    if (isTableSeparator(line) && matchTableDivider(line, paragraph)) {
      // 表头是段落最后一行（还留在缓冲区里），前面几行仍按普通段落保留
      const headerLine = paragraph.pop() ?? "";
      flushParagraph();
      blocks.push({ kind: "table", header: splitTableRow(headerLine), rows: [] });
      continue;
    }

    if (HR_PATTERN.test(line.trim())) {
      flushParagraph();
      blocks.push({ kind: "hr" });
      continue;
    }

    const quote = /^\s{0,3}>\s?(.*)$/.exec(line);
    if (quote) {
      flushParagraph();
      const collected = [quote[1]];
      while (index + 1 < lines.length && /^\s{0,3}>/.test(lines[index + 1])) {
        index += 1;
        collected.push(/^\s{0,3}>\s?(.*)$/.exec(lines[index])?.[1] ?? "");
      }
      blocks.push({ kind: "quote", lines: collected });
      continue;
    }

    const bullet = UNORDERED_MARKER.test(line) || ORDERED_MARKER.test(line);
    if (bullet) {
      flushParagraph();
      const collected = [line];
      while (index + 1 < lines.length && lines[index + 1].trim()) {
        index += 1;
        collected.push(lines[index]);
      }
      blocks.push(parseList(collected));
      continue;
    }

    const table = blocks[blocks.length - 1];
    if (table?.kind === "table" && line.includes("|")) {
      table.rows.push(splitTableRow(line));
      continue;
    }

    paragraph.push(line);
  }

  flushParagraph();
  return blocks;
}

/** 渲染 [QUOTE] 之外的正文：块级结构 + 行内语法 */
function renderBlocks(source: string, context: MarkdownContext | undefined, html: boolean): string {
  const state = buildContext(source, context);
  const out: string[] = [];
  const emitter: Emitter | null = html
    ? {
        open: (tag) => (tag.startsWith("<") ? tag : "<" + tag + ">"),
        close: (tag) => "</" + (tag.startsWith("<") ? /^<([a-zA-Z][a-zA-Z0-9-]*)/.exec(tag)?.[1] ?? "span" : tag.split(" ")[0]) + ">",
      }
    : null;

  const inline = (text: string) => parseInline(state, emitter, text);

  for (const block of parseBlocks(source)) {
    switch (block.kind) {
      case "p":
        out.push(html ? "<p class=\"md-p\">" + inline(block.text) + "</p>" : inline(block.text));
        break;
      case "h": {
        const tag = "h" + Math.min(6, Math.max(1, block.level));
        out.push(html ? "<" + tag + ' class="md-h">' + inline(block.text) + "</" + tag + ">" : inline(block.text));
        break;
      }
      case "hr":
        if (html) out.push('<hr class="md-hr" />');
        break;
      case "code":
        if (html) {
          const lang = block.lang ? ' data-lang="' + escapeHtml(block.lang) + '"' : "";
          out.push("<pre class=\"md-pre\"" + lang + "><code>" + escapeHtml(block.lines.join("\n")) + "</code></pre>");
        } else {
          out.push(block.lines.join("\n"));
        }
        break;
      case "quote":
        if (html) {
          out.push("<blockquote class=\"md-quote\">" + renderBlocks(block.lines.join("\n"), context, html) + "</blockquote>");
        } else {
          out.push(block.lines.join("\n"));
        }
        break;
      case "list": {
        const tag = block.ordered ? "ol" : "ul";
        const start = block.ordered && block.start !== 1 ? ' start="' + block.start + '"' : "";
        const items = block.items.map((item) => {
          const body = renderBlocks(item, context, html);
          return html ? "<li class=\"md-li\">" + body + "</li>" : "- " + body;
        });
        out.push(html ? "<" + tag + ' class="md-list"' + start + ">" + items.join("") + "</" + tag + ">" : items.join("\n"));
        break;
      }
      case "table":
        if (html) {
          const head = block.header.map((cell) => "<th>" + inline(cell) + "</th>").join("");
          const body = block.rows
            .map((row) => "<tr>" + block.header.map((_, cellIndex) => "<td>" + inline(row[cellIndex] ?? "") + "</td>").join("") + "</tr>")
            .join("");
          out.push('<table class="md-table"><thead><tr>' + head + "</tr></thead><tbody>" + body + "</tbody></table>");
        } else {
          out.push([block.header.join(" | "), ...block.rows.map((row) => row.join(" | "))].join("\n"));
        }
        break;
      default:
        break;
    }
  }

  return out.join(html ? "" : "\n");
}

/** 缓存键要带上渲染上下文，否则同一段文字在不同提及标签下会串味 */
function contextSignature(context: MarkdownContext | undefined): string {
  if (!context) return "";
  const names = context.names && context.names.length > 0 ? [...context.names].sort().join("|") : "";
  const tag = context.mention?.htmlTag ?? "";
  const prefix = context.mention?.plainPrefix ?? "";
  return names || tag || prefix ? names + "\u0000" + tag + "\u0000" + prefix : "";
}

/**
 * Markdown → 安全 HTML（带 .md-* 样式类）。
 * 结果做了 LRU 缓存：流式刷新时同一段文本不会反复解析。
 */
export function renderMarkdown(text: string, context?: MarkdownContext): string {
  if (!text) return "";
  const normalized = text.replace(/\r\n/g, "\n");
  const signature = contextSignature(context);
  // 超长正文不进缓存：避免缓存把长回复的 HTML 长期留住
  const cacheable = normalized.length <= CACHE_ENTRY_MAX_LENGTH;
  const key = signature + "\u0001" + normalized;
  if (cacheable) {
    const hit = cache.get(key);
    if (hit !== undefined) return hit;
  }
  const protectedText = protectText(normalized);
  const html = protectedText.restore(renderBlocks(protectedText.text, context, true));
  if (cacheable) {
    if (cache.size >= CACHE_LIMIT) {
      const oldest = cache.keys().next().value;
      if (oldest !== undefined) cache.delete(oldest);
    }
    cache.set(key, html);
  }
  return html;
}

const plainCache = new Map<string, string>();

/** Markdown → 纯文本：给 TTS 朗读 / 摘要等场景用，星号和链接地址都不会念出来 */
export function markdownToPlainText(text: string, context?: MarkdownContext): string {
  if (!text) return "";
  const plainKey = contextSignature(context) + "\u0001" + text;
  if (text.length <= CACHE_ENTRY_MAX_LENGTH) {
    const cached = plainCache.get(plainKey);
    if (cached !== undefined) return cached;
  }
  const normalized = text
    .replace(/\r\n/g, "\n")
    // 图片的地址不朗读，只留替代文字
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    // HTML 注释不留痕
    .replace(/<!--[\s\S]*?-->/g, "");
  // 纯文本模式：公式按 LaTeX 降级成可读文字，代码段占位符则直接丢掉
  const protectedText = protectText(normalized);
  const plain = restoreMathTokens(renderBlocks(protectedText.text, context, false), protectedText.mathTokens, false)
    .replace(/\uE000[BI]\d+\uE001/g, "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{2,}/g, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .trim();
  if (text.length <= CACHE_ENTRY_MAX_LENGTH) {
    if (plainCache.size >= CACHE_LIMIT) {
      const oldest = plainCache.keys().next().value;
      if (oldest !== undefined) plainCache.delete(oldest);
    }
    plainCache.set(plainKey, plain);
  }
  return plain;
}

const MESSAGE_CACHE_LIMIT = 40;
const messageCache = new Map<string, { source: string; segments: MessageRenderSegment[] }>();

/**
 * 把一条消息切成"引用块 + Markdown 正文"的渲染分段。
 * quote 段保持微信式引用块的原样渲染（内部也按行内语法处理），text 段走完整 Markdown。
 */
export function renderMessageSegments(
  raw: string,
  options: { names?: string[]; mention?: MentionTag; cacheKey?: string } = {},
): MessageRenderSegment[] {
  const text = (raw ?? "").replace(/\r\n/g, "\n");
  const key = options.cacheKey;
  if (key) {
    const hit = messageCache.get(key);
    if (hit && hit.source === text) return hit.segments;
  }

  const segments: MessageRenderSegment[] = [];
  const pattern = /\[QUOTE\]([\s\S]*?)\[\/QUOTE\]/gi;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) {
      segments.push({ type: "text", html: renderMarkdown(text.slice(last, match.index), options) });
    }
    segments.push({ type: "quote", html: renderMarkdown(match[1].trim(), options) });
    last = pattern.lastIndex;
  }
  const rest = text.slice(last).replace(/\[\/?QUOTE\]/gi, "");
  if (rest) segments.push({ type: "text", html: renderMarkdown(rest, options) });
  const visible = segments.filter((segment) => segment.html.trim().length > 0);

  if (key) {
    if (messageCache.size >= MESSAGE_CACHE_LIMIT) {
      const oldest = messageCache.keys().next().value;
      if (oldest !== undefined) messageCache.delete(oldest);
    }
    messageCache.set(key, { source: text, segments: visible });
  }
  return visible;
}
