/**
 * Markdown / 公式渲染回归测试：node --test scripts/verify-markdown.cjs
 *
 * 覆盖：Markdown 常用语法、@提及 高亮、[QUOTE] 分段、KaTeX 公式（定界符 /
 * 代码段保护 / 纯文本降级 / 注入面）、流式半截语法、HTML 转义。
 *
 * 也可以直接 node scripts/verify-markdown.cjs 跑（沙箱里 node --test 的子进程管道会被拒）。
 */
const fs = require("fs");
const path = require("path");
const assert = require("node:assert/strict");
const ts = require("typescript");

const { test } = require("node:test");

/** 反斜杠：LaTeX 里到处都是，集中在 B 上，避免用例被转义规则搞乱 */
const B = String.fromCharCode(92);
const TICK = String.fromCharCode(96);

const direct = new Map();
const it = (name, fn) => direct.set(name, fn);
test.after(() => {
  if (direct.size === 0) return;
  for (const [name, fn] of direct) fn(assert, name);
  direct.clear();
});

const projectRoot = path.join(__dirname, "..");
const cacheRoot = path.join(projectRoot, ".md-check", ".cache");
/**
 * 把 TS 编成 CommonJS 落到 .md-check/.cache 再用 require 加载：
 * 依赖（katex）按正常模块解析从 node_modules 找。
 */
function compileModule(file) {
  const absolute = path.resolve(projectRoot, file);
  const compiled = ts.transpileModule(fs.readFileSync(absolute, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021, esModuleInterop: true },
  }).outputText;
  const target = path.join(cacheRoot, path.relative(projectRoot, absolute).replace(/\.ts$/, ".js"));
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, compiled);
  return target;
}
function loadModule(file) {
  // 工程是 "type": "module"，缓存目录里放一份 commonjs 声明，
  // 编译出来的 .js 才会被当成 CJS，内部的 require("./math") 也能解析
  fs.writeFileSync(path.join(cacheRoot, "..", "package.json"), JSON.stringify({ type: "commonjs" }));
  // 依赖模块也要一起重新编译，否则 require("./math") 会命中上一次的旧产物
  for (const dependency of ["src/services/math.ts"]) compileModule(dependency);
  return require(compileModule(file));
}

const { renderMarkdown, markdownToPlainText, renderMessageSegments } = loadModule("src/services/markdown.ts");

/** 用户实际收到的那类回复：标题 + 粗体 + 列表 + 链接 */
const SAMPLE = [
  "先说明一下：你给的是「今天」，我按 2026-10-09（周五）当天的窗口去扒了一圈。",
  "",
  "**一、模型侧：两大巨头同日开打**",
  "- OpenAI 把 **GPT-6 全面开放**，免费用户也能直接用（[OpenAI 和 Anthropic 同日开打](https://finance.sina.com.cn/wm/2026-10-08/doc-iniupnvc7566628.shtml)）。",
  "- 谷歌 **Gemini 4 Argon** 已对部分用户开放。",
  "",
  "> 一句话锐评：这周 AI 圈的剧本是「模型免费送」。",
  "",
  "| 公司 | 动作 |",
  "| --- | --- |",
  "| OpenAI | 免费开放 |",
  "| Anthropic | 降价 |",
].join("\n");

it("粗体 / 斜体 / 行内代码不再以星号形式露出", () => {
  const html = renderMarkdown("这是 **重点** 与 *强调* 以及 " + TICK + "code" + TICK + " 的段落");
  assert.ok(html.includes("<strong>重点</strong>"), html);
  assert.ok(html.includes("<em>强调</em>"), html);
  assert.ok(html.includes("<code>code</code>"), html);
  assert.ok(!html.includes("**"), "不应残留星号：" + html);
});

it("AI 回复样本：标题、列表、链接、引用、表格都能结构化渲染", () => {
  const html = renderMarkdown(SAMPLE);
  assert.ok(html.includes("<strong>一、模型侧：两大巨头同日开打</strong>"), "加粗小标题");
  assert.ok(html.includes('<ul class="md-list">'), "无序列表");
  assert.ok(html.includes('<a class="md-link" href="https://finance.sina.com.cn'), "链接");
  assert.ok(html.includes(">OpenAI 和 Anthropic 同日开打</a>"), "链接文字");
  assert.ok(html.includes('<blockquote class="md-quote">'), "引用块");
  assert.ok(html.includes('<table class="md-table">'), "表格");
  assert.ok(html.includes("<td>降价</td>"), "表格单元格");
  assert.ok(!/\*\*/.test(html), "不应残留 Markdown 记号：" + html.slice(0, 400));
});

it("删除线带样式类", () => {
  assert.ok(renderMarkdown("这是 ~~划掉~~ 的内容").includes('<del class="md-del">划掉</del>'));
  assert.ok(renderMarkdown("两个 ~~ 之间没有内容").includes("<del") === false);
});

it("标题、分割线、围栏代码块", () => {
  const source = "# 大标题\n\n### 小标题\n\n---\n\n" + TICK + TICK + TICK + "js\nconst a = 1;\n" + TICK + TICK + TICK;
  const html = renderMarkdown(source);
  assert.ok(html.includes('<h1 class="md-h">大标题</h1>'), html);
  assert.ok(html.includes('<h3 class="md-h">小标题</h3>'), html);
  assert.ok(html.includes('<hr class="md-hr" />'), html);
  assert.ok(html.includes('<pre class="md-pre" data-lang="js"><code>const a = 1;</code></pre>'), html);
});

it("有序列表保留起始序号，嵌套列表保留层级", () => {
  const ordered = renderMarkdown("3. 第三\n4. 第四");
  assert.ok(ordered.includes('<ol class="md-list" start="3">'), ordered);
  const nested = renderMarkdown("- 第一层\n  - 第二层");
  assert.ok(nested.includes('<ul class="md-list"><li class="md-li"><p class="md-p">第一层</p><ul class="md-list">'), nested);
});

it("@提及 高亮：命中占位符输出高亮 span，其它 @ 保持原样", () => {
  const html = renderMarkdown("让 @代码助手 看一下，邮箱是 a@b.com", { names: ["代码助手"] });
  assert.ok(html.includes('<span class="md-mention">代码助手</span>'), html);
  assert.ok(html.includes("a@b.com"), html);
  const custom = renderMarkdown("@代码助手 收到", { names: ["代码助手"], mention: { htmlTag: '<span class="bubble-mention">' } });
  assert.ok(custom.includes('<span class="bubble-mention">代码助手</span>'), custom);
});

it("[QUOTE] 分段：引用块与正文分开渲染", () => {
  const segments = renderMessageSegments("[QUOTE]原始问题[/QUOTE]\n\n我的回答是 **这样** 的。");
  assert.equal(segments.length, 2);
  assert.equal(segments[0].type, "quote");
  assert.ok(segments[0].html.includes("原始问题"), segments[0].html);
  assert.equal(segments[1].type, "text");
  assert.ok(segments[1].html.includes("<strong>这样</strong>"), segments[1].html);
});

it("纯文本降级：TTS 不会念出星号、井号、链接地址", () => {
  const plain = markdownToPlainText(SAMPLE);
  assert.ok(!plain.includes("*"), plain);
  assert.ok(!plain.includes("#"), plain);
  assert.ok(!plain.includes("https://"), plain);
  assert.ok(plain.includes("OpenAI 和 Anthropic 同日开打"), plain);
  assert.ok(plain.includes("OpenAI | 免费开放"), "表格降级为文字行：" + plain);
  assert.ok(!/\n{2,}/.test(plain), "空行被归一");
});

it("流式半截语法不炸：未闭合的 ** 与围栏按普通字符输出", () => {
  const half = renderMarkdown("正在生成 **加粗还没结束");
  assert.ok(half.includes("正在生成"), half);
  assert.ok(!half.includes("<strong>"), half);
  const fence = renderMarkdown("说明：\n" + TICK + TICK + TICK + "js\nconst a = 1;");
  assert.ok(fence.includes("const a = 1;"), fence);
  const partialLink = renderMarkdown("见 [文档](https://example.com/gu");
  assert.ok(partialLink.includes("[文档](https://example.com/gu"), partialLink);
  assert.ok(!partialLink.includes("md-link"), partialLink);
});

it("安全：原始 HTML 被转义，危险协议链接被丢弃", () => {
  const html = renderMarkdown("<img src=x onerror=alert(1)> 与 <script>alert(2)</script>");
  assert.ok(!html.includes("<img"), html);
  assert.ok(!html.includes("<script"), html);
  assert.ok(html.includes("&lt;img src=x onerror=alert(1)&gt;"), html);
  const js = renderMarkdown("[点我](javascript:alert(1))");
  assert.ok(!js.includes("href"), js);
  assert.ok(js.includes("点我"), js);
  const data = renderMarkdown("[x](data:text/html;base64,PHNjcmlwdD4=)");
  assert.ok(!data.includes("href"), data);
});

it("裸链接自动变可点，尾随中文标点不入链接", () => {
  const html = renderMarkdown("来源 https://example.com/a/b 。");
  assert.ok(html.includes('href="https://example.com/a/b"'), html);
  assert.ok(!html.includes('href="https://example.com/a/b。"'), html);
});

it("块级结构：连续行合成一段（内部软换行），空行切段", () => {
  const html = renderMarkdown("第一行\n第二行\n\n第三段");
  assert.equal((html.match(/<p class="md-p">/g) || []).length, 2, html);
  assert.ok(html.includes("第一行<br />第二行"), html);
});

it("缓存命中不改变结果", () => {
  const once = renderMarkdown("**重复** 渲染同一段");
  const twice = renderMarkdown("**重复** 渲染同一段");
  assert.equal(once, twice);
});

// ===== 公式（KaTeX）=====

it("公式：\\[…\\] 与 $$…$$ 渲染成独立公式", () => {
  const source = "先看方程：\n\n" + B + "[\n" + B + "rho(u u_x+v u_y)=-p_x+" + B + "mu(u_{xx}+u_{yy})\n" + B + "]";
  const html = renderMarkdown(source);
  assert.ok(html.includes("katex-display"), html.slice(0, 400));
  assert.ok(html.includes("md-math"), html.slice(0, 400));
  assert.ok(!html.includes(B + "rho"), "LaTeX 源码不应原样露出：" + html.slice(0, 400));
  const dollars = renderMarkdown("$$E=mc^2$$");
  assert.ok(dollars.includes("katex-display"), dollars);
});

it("公式：\\(…\\) 与 $…$ 渲染成行内公式，跨行美元不当公式", () => {
  const html = renderMarkdown("下标 " + B + "(u_x=" + B + "partial u/" + B + "partial x" + B + ") 表示偏导");
  assert.ok(html.includes("katex"), html);
  assert.ok(!html.includes(B + "partial"), html);
  assert.ok(html.includes("表示偏导"), html);
  // 段落里的两个 $ 之间跨了行，不该被当成公式（否则正文会被整段吞掉）
  const currency = renderMarkdown("这件 100$ 那件\n200$ 都不便宜");
  assert.ok(!currency.includes("katex"), "跨行的美元不该被当成公式：" + currency);
  assert.ok(currency.includes("这件 100$"), currency);
  const closed = renderMarkdown("价格是 $x+1$ 元");
  assert.ok(closed.includes("katex"), "成对的行内 $ 公式要正常渲染：" + closed);
});

it("公式里的下划线与花括号不会被 Markdown 吃掉", () => {
  const cases = B + "[\n" + B + "begin{cases}\n" + B + "rho(u u_x+v u_y)=-p_x+" + B + "mu(u_{xx}+u_{yy})" + B + B + "\n u_x+v_y=0\n" + B + "end{cases}\n" + B + "]";
  const html = renderMarkdown(cases);
  assert.ok(html.includes("katex-display"), html.slice(0, 300));
  assert.ok(!html.includes("<em>"), "u_x 之类的下标不该变成斜体：" + html.slice(0, 300));
  assert.ok(!html.includes(B + "begin"), html.slice(0, 300));
  const boxed = renderMarkdown(B + "[\n" + B + "boxed{u_x=0," + B + "qquad p_x=" + B + "mu u_{yy}}\n" + B + "]");
  assert.ok(boxed.includes("katex-display"), boxed.slice(0, 300));
});

it("多个公式混排：正文与公式顺序不乱", () => {
  const html = renderMarkdown([
    "（1）写出并化简方程",
    "",
    B + "[",
    "u_x+v_y=0",
    B + "]",
    "",
    "代入 " + B + "(v=0" + B + ") 得到：",
    "",
    B + "[",
    "u_x=0," + B + "qquad p_y=0",
    B + "]",
  ].join("\n"));
  assert.equal((html.match(/katex-display/g) || []).length, 2, html.slice(0, 600));
  assert.ok(html.indexOf("写出并化简方程") < html.indexOf("得到"), "顺序不能乱");
  assert.ok(html.includes("katex"), "行内公式也要渲染");
});

it("定界符错配时按块边界切分，不会把两段公式并成一块", () => {
  // 模型偶尔会连开两块、或漏掉一个 \] ，这里必须按"下一个 \[ 就是边界"处理
  const two = renderMarkdown([B + "[", "u_x+v_y=0", B + "]", "", "中间还有一段话", "", B + "[", "p_y=0", B + "]"].join("\n"));
  assert.equal((two.match(/katex-display/g) || []).length, 2, two.slice(0, 400));
  assert.ok(two.includes("中间还有一段话"), "中间正文不能被吞进公式：" + two.slice(0, 400));
  const messy = renderMarkdown([B + "[", B + "begin{cases}", "u_x=0", B + "end{cases}", B + "]", "", "代入 " + B + "(v=0" + B + ") 得：", "", B + "[", "p_x=0", B + "]"].join("\n"));
  assert.equal((messy.match(/katex-display/g) || []).length, 2, messy.slice(0, 400));
  assert.ok(!messy.includes("MDMATH"), "不该泄漏占位符");
});

it("代码块里的 $ 和反斜杠不会被当成公式", () => {
  const source = TICK + TICK + TICK + "sh\nawk '{print $1}' file\n" + TICK + TICK + TICK;
  const html = renderMarkdown(source);
  assert.ok(!html.includes("katex"), "代码块内容不应被 KaTeX 处理：" + html);
  assert.ok(html.includes("awk"), html);
  assert.ok(html.includes("<pre"), html);
});

it("公式在 TTS 纯文本里降级成可读文字", () => {
  const plain = markdownToPlainText("根号：" + B + "(" + B + "sqrt{x}" + B + ") 与分数 " + B + "(" + B + "frac{a}{b}" + B + ")");
  assert.ok(!plain.includes(B), "不应残留 LaTeX 命令：" + plain);
  assert.ok(plain.includes("√(x)"), plain);
  assert.ok(plain.includes("(a)/(b)"), plain);
  const display = markdownToPlainText(B + "[\nu_x+v_y=0\n" + B + "]");
  assert.ok(!display.includes(B), display);
});

it("公式不提供注入面：KaTeX 关闭信任与 HTML 命令", () => {
  const href = renderMarkdown(B + "(" + B + "href{javascript:alert(1)}{点我}" + B + ")");
  assert.ok(!href.includes("javascript:"), href);
  assert.ok(!href.includes("<a "), href);
  const htmlCmd = renderMarkdown(B + "(" + B + "htmlClass{evil}{x}" + B + ")");
  assert.ok(!htmlCmd.includes("evil"), htmlCmd);
});

it("渲染失败的公式原样展示而不是崩掉", () => {
  const html = renderMarkdown(B + "(" + B + "notacommand{1}" + B + ")");
  assert.ok(html.includes("md-p"), html);
  assert.ok(!html.includes("\uE000"), "不该泄漏占位符：" + html);
});

// ===== 直接运行入口 =====
if (require.main === module) {
  let passed = 0;
  const failures = [];
  for (const [name, fn] of direct) {
    try {
      fn(assert, name);
      passed += 1;
      console.log("  ok   " + name);
    } catch (error) {
      failures.push({ name, error });
      console.log("  FAIL " + name);
    }
  }
  console.log("");
  for (const failure of failures) {
    console.log("--- " + failure.name);
    console.log(String(failure.error && failure.error.message ? failure.error.message : failure.error).slice(0, 1200));
    console.log("");
  }
  console.log(passed + " passed, " + failures.length + " failed");
  process.exit(failures.length ? 1 : 0);
}