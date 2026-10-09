/**
 * 数学公式渲染：把 AI 回复里的 LaTeX 交给 KaTeX 排版。
 *
 * 背景：模型经常直接输出 LaTeX（\frac、\begin{cases}、\boxed…），
 * 之前只有 Markdown 渲染，公式会退化成一堆反斜杠命令。这里负责：
 *  - 从正文里识别公式片段（$…$、$$…$$、\(…\)、\[…\]，以及常见的裸 LaTeX）
 *  - 用 KaTeX 渲染成 HTML，并用占位符替换，保证 Markdown 解析器看不到这些命令
 *  - 纯文本模式（TTS / 摘要）下把公式降级成可读的文字
 *
 * 安全：tex 源码通过 new Function 传进 katex.renderToString 的实参里，
 * 不参与代码拼接（KaTeX 的信任模型就是"只渲染，不执行"），
 * 且渲染开启 strict 与 trust:false，\href、\htmlClass 之类一律不生效。
 */

import katex from "katex";

export interface MathToken {
  token: string;
  kind: "inline" | "display";
  latex: string;
}

const PLACEHOLDER_PREFIX = "\uE000MDMATH";
const PLACEHOLDER_SUFFIX = "\uE001";
const MAX_LATEX_LENGTH = 20000;
const MAX_FORMULAS = 400;

const KATEX_OPTIONS = {
  throwOnError: false,
  strict: "ignore" as const,
  trust: false,
  output: "html" as const,
  displayMode: false,
};

/**
 * 调用 KaTeX 的函数：源码用 new Function 生成、tex 与 katex 都走形参传入。
 * 为什么不直接 katex.renderToString(tex)？因为 tex 里全是反斜杠，
 * 一旦被拼进源码字符串就会变成转义符（"\r"、"\t"）把公式弄坏。
 */
const KATEX_RUNNER = new Function("katex", "tex", "options", "return katex.renderToString(tex, options);") as (
  katexModule: typeof katex,
  tex: string,
  options: Record<string, unknown>,
) => string;

let sequence = 0;

function renderLatex(latex: string, display: boolean): string {
  const trimmed = latex.trim();
  if (!trimmed) return "";
  try {
    const html = KATEX_RUNNER(katex, trimmed, { ...KATEX_OPTIONS, displayMode: display });
    if (display) {
      return '<span class="md-math md-math-display">' + html + "</span>";
    }
    return '<span class="md-math">' + html + "</span>";
  } catch {
    // 渲染失败就原样显示，公式源码至少不会丢
    return '<code class="md-math-error">' + escapeHtmlText(trimmed) + "</code>";
  }
}

function escapeHtmlText(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function makeToken(kind: "inline" | "display", latex: string): MathToken {
  sequence += 1;
  return { token: PLACEHOLDER_PREFIX + kind + sequence + PLACEHOLDER_SUFFIX, kind, latex };
}

type Segment = { type: "text" | "math"; value: string; display?: boolean };

/**
 * 找出正文里的公式片段。
 *
 * 每一轮取"最靠前的定界符"，按定界符类型决定是行内还是独立公式，
 * 避免 \( 被 Markdown 当成转义字符吃掉。支持 \$…\$、$$…$$、\(…\)、\[…\]
 * 与裸的 \begin{...}…\end{...}（模型经常不带定界符直接给 cases 环境）。
 *
 * 两条保守规则，都是被真实回复教出来的：
 * - \(…\) / $…$ 不跨行，起止必须落在同一行：否则「这件 100$ …… 200$ 都不便宜」
 *   这类正文会被整段吞进公式。
 * - 找不到闭合符时只把开头那个定界符当普通文本（不做跨块吞并）：
 *   模型偶尔会连开两块或漏掉一个 \]，宁可少渲染一个公式，也不能把正文吃掉。
 */
function scanSegments(text: string): Segment[] {
  const segments: Segment[] = [];
  let pos = 0;
  let guard = 0;

  while (pos < text.length && guard < MAX_FORMULAS) {
    guard += 1;
    type Spot = { index: number; length: number; open: string; close: string; display: boolean };
    let best: Spot | null = null;

    const consider = (pattern: RegExp, open: string, close: string, display: boolean) => {
      pattern.lastIndex = 0;
      const match = pattern.exec(text.slice(pos));
      if (!match) return;
      const index = pos + match.index;
      if (!best || index < best.index) {
        best = { index, length: match[0].length, open, close, display };
      }
    };

    consider(/\$\$/, "$$", "$$", true);
    consider(/\\\[/, "\\[", "\\]", true);
    consider(/\\\(/, "\\(", "\\)", false);
    consider(/\\begin\{(?:cases|aligned|align|array|matrix|pmatrix|bmatrix|vmatrix|gathered|split)\}/, "__env__", "__env__", true);
    consider(/\$/, "$", "$", false);

    if (!best) break;
    const spot: Spot = best;

    let body = "";
    let consumed = 0;
    let ok = false;

    if (spot.open === "__env__") {
      const envName = /^\\begin\{([a-zA-Z*]+)\}/.exec(text.slice(spot.index))?.[1] ?? "";
      const endTag = "\\end{" + envName + "}";
      const closeIndex = text.indexOf(endTag, spot.index + spot.length);
      if (closeIndex !== -1) {
        body = text.slice(spot.index, closeIndex + endTag.length);
        consumed = closeIndex + endTag.length - spot.index;
        ok = true;
      }
    } else {
      const contentStart = spot.index + spot.length;
      const closeIndex = text.indexOf(spot.close, contentStart);
      if (closeIndex !== -1) {
        const candidate = text.slice(contentStart, closeIndex);
        if (spot.open === "$") {
          if (!candidate.includes("\n") && candidate.trim().length > 0) {
            body = candidate;
            consumed = closeIndex + spot.close.length - spot.index;
            ok = true;
          }
        } else {
          body = candidate;
          consumed = closeIndex + spot.close.length - spot.index;
          ok = true;
        }
      }
    }

    if (!ok) {
      const nextStart = spot.index + spot.length;
      if (nextStart > pos) segments.push({ type: "text", value: text.slice(pos, nextStart) });
      pos = nextStart;
      continue;
    }

    if (spot.index > pos) segments.push({ type: "text", value: text.slice(pos, spot.index) });
    if (body.length <= MAX_LATEX_LENGTH) segments.push({ type: "math", value: body, display: spot.display });
    else segments.push({ type: "text", value: body });
    pos = spot.index + consumed;
  }

  if (pos < text.length) segments.push({ type: "text", value: text.slice(pos) });
  return segments;
}/** 去掉 LaTeX 命令，得到可以朗读/摘要的近似文字 */
function latexToPlainText(latex: string): string {
  return latex
    .replace(/\\begin\{[a-zA-Z*]+\}|\\end\{[a-zA-Z*]+\}/g, " ")
    .replace(/\\(?:dfrac|tfrac|frac)\s*\{([^{}]*)\}\s*\{([^{}]*)\}/g, "($1)/($2)")
    .replace(/\\(?:sqrt)\s*\{([^{}]*)\}/g, "√($1)")
    .replace(/\\(?:boxed|text|mathrm|mathbf|boldsymbol|operatorname|left|right|bigl|bigr|Bigl|Bigr)\s*/g, "")
    .replace(/\\[,;:!\s]/g, " ")
    .replace(/\\\\/g, "，")
    .replace(/\\[a-zA-Z]+/g, "")
    .replace(/[{}]/g, "")
    .replace(/\s*&\s*/g, " ")
    .replace(/\s{2,}/g, " ")
    .trim();
}

/**
 * 抽出正文里的公式，正文里的公式位置换成占位符。
 * 返回的 tokens 需要交给 restoreMathTokens 还原成 HTML。
 */
export function extractMathTokens(text: string): { text: string; tokens: MathToken[] } {
  if (!text.includes("\\") && !text.includes("$")) return { text, tokens: [] };
  const segments = scanSegments(text);
  if (!segments.some((segment) => segment.type === "math")) return { text, tokens: [] };

  const tokens: MathToken[] = [];
  let out = "";
  for (const segment of segments) {
    if (segment.type === "text") {
      out += segment.value;
      continue;
    }
    const token = makeToken(segment.display ? "display" : "inline", segment.value);
    tokens.push(token);
    // 独立公式前后补空行，让 Markdown 把它当独立块，不跟相邻行粘在一段里
    out += segment.display ? "\n\n" + token.token + "\n\n" : token.token;
  }
  return { text: out, tokens };
}

/** 把占位符换回渲染好的公式 HTML；HTML 模式直接渲染，纯文本模式降级朗读 */
export function restoreMathTokens(html: string, tokens: MathToken[], asHtml: boolean): string {
  if (tokens.length === 0) return html;
  let out = html;
  for (const token of tokens) {
    const replacement = asHtml
      ? renderLatex(token.latex, token.kind === "display")
      : latexToPlainText(token.latex);
    out = out.split(token.token).join(replacement);
  }
  return out;
}

/** Markdown → 纯文本时用的公式降级 */
export function mathToPlainText(latex: string): string {
  return latexToPlainText(latex);
}
