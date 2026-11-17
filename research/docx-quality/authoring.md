# 直连 API 路径的 Word 生成质量审计：内容层 / 契约层 / 提示词层

> 审计对象：桌宠 App「直连 API」后端（`src-tauri/src/direct_api/*`）、系统提示拼装（`src-tauri/src/lib.rs`），以及真实产物
> `C:\Users\15188\Desktop\AI领域新闻报告汇总_2026年11月.docx`（2026-11-14 00:45 生成，9,407 字节）。
> 本次只读审计，不改源码；行号基于当前 checkout。所有结论带 `文件:行号` 或原文引用，确认不了的不断言。

**文件简称（下文 `xxx:NN` 均指下列文件）**

| 简称 | 路径 |
|---|---|
| `tools.rs` | `src-tauri/src/direct_api/tools.rs`（6,435 行） |
| `agent.rs` | `src-tauri/src/direct_api/agent.rs`（2,888 行） |
| `api_client.rs` | `src-tauri/src/direct_api/api_client.rs`（1,972 行） |
| `lib.rs` | `src-tauri/src/lib.rs`（7,996 行） |
| `client.rs` | `src-tauri/src/openclaw/client.rs`（902 行，Claude Code CLI 后端） |
| `db.rs` | `src-tauri/src/storage/db.rs`（2,665 行） |
| `tools.ts` | `src/services/tools.ts`（90 行） |
| `AgentActivity.vue` | `src/components/AgentActivity.vue`（433 行） |
| `BRIEF.md` | `research/docx-quality/BRIEF.md` |
| `产物` | 桌面 `AI领域新闻报告汇总_2026年11月.docx`（本次用 zipfile 重新解包核对） |

---

## 0. 结论摘要（先回答问题）

**为什么直连 API 这条链路特别容易生成低质量 Word**：不是模型不会写，而是「排版责任」「表达通道」「反馈闭环」三件事在架构层被同时锁死，任何一条单独修都只能拿到局部改善：

1. **排版责任分配错误（谁写内容 vs 谁决定版式）。**
   工具描述向模型承诺「正文用 Markdown 语法书写，**会自动转换为排版好的 Word 文档**」（`tools.rs:989`），系统提示再单独确认一次（`lib.rs:2230-2233` 的「【Word 文档生成能力】」段）。于是模型做了一个理性决策：**排版交给渲染器**，我只管内容。
   但渲染器是手写的最小实现：`build_docx_bytes` 只打包 3 个 part（`tools.rs:1464-1499`），`markdown_to_document_xml` 只有 122 行解析逻辑（`tools.rs:1835-1956`），全文档 `w:pStyle=0 / w:outlineLvl=0 / w:numPr=0 / w:hyperlink=0`（本次解包计数）。**被外包出去的排版，在渲染器里直接蒸发。**
   对比 DSH + office-docx 路径：模型自己就是排版作者（写 OOXML / 用 python-docx / 调校验技能），不存在「责任交接」这一跳。

2. **表达通道被压成「最低公共分母」。**
   契约只有 `path / title / content` 三个字段（`tools.rs:988-999`），Markdown 解码器又只认标题、粗斜体、行内代码、列表、引用、表格、代码块（`tools.rs:989`），并且实现层还有 5 处硬性保真损失（软换行合并 `tools.rs:1849`、`---` 未处理、假列表 `tools.rs:1654`、表格均分列宽 `tools.rs:1725`、无链接/样式/分节）。**模型能想到的文档元素，大部分在契约里没有字段可写**；能写的部分，又在解码器里继续掉。

3. **生成过程既不可见、也不可迭代。**
   整篇正文必须一次性塞进一个 `create_docx` 调用的 `arguments` 字符串；成功后的工具结果只有「Word 文档已生成: 路径（N 字节）」（`tools.rs:1456-1460`）——没有结构、没有页数、没有校验、没有回读；本仓库**没有**任何 docx 校验/渲染/巡检工具；历史库只存 `content/thinking`，不存 tool 调用（`db.rs:103-123`、`agent.rs:1148-1178`），所以下一轮「继续改」时模型看不到上一轮写过什么；参数一旦被输出长度上限截断，整个调用直接跳过（`agent.rs:839-862`）。**于是这份文档只能「一次生成，质量不可观测，错误无法自我纠正」。**

两条同向放大器：**提示词里没有文档方法论**（Word 段只有 20 行，`lib.rs:2226-2249`），且默认桌宠人设要求「回复简短自然，通常 2-3 句话」（`client.rs:40`）；**请求体从不设置输出预算**（`api_client.rs:1014-1037、1120-1141、1193-1214` 只有 model/messages/stream/reasoning_effort/tools，全目录 `max_tokens` 0 命中），长文输出上限不可控且不可感知。

**最小改造集（P0，详见 §6）**：契约双通道（`mode` + `blocks`/`markdown` + `frontmatter`/`options`）→ 提示词替换为《文档作者规范》→ 解析器 4 处硬修复（软换行、`---`、标题样式、表格列宽）→ `create_docx` 返回值改成「结构摘要 + warnings」。这四步做完，文档质量的下限由「渲染器碰运气」变成「每次生成都被结构校验驱动」。

---

## 1. 链路解剖

### 1.1 一次「帮我整理一份 AI 新闻报告，保存成 Word」的完整调用序列

下文 S1-S9 是一个**成功案例**（即桌面产物那一类）的完整路径；每一站的掉质点编号对应 §1.2。

| 步 | 代码位置 | 实际发生的事 | 掉质风险 |
|---|---|---|---|
| S0 | 用户输入 | 用户给一句话需求（受众、篇幅、结构都没说） | 提示词若不先问/先提纲，模型直接开写 |
| S1 | `lib.rs:4340-4400` | 系统提示 = 桌宠人设（`client.rs:40/101-108`）+ memory/apps/custom/skill + collaboration + 14 段能力注入（runtime identity、execution mode、quote、**Word 生成能力** `lib.rs:2226-2249`、agent factory、coding、computer use、weather、search） | **D13**：文档方法论只有 20 行，且与「2-3 句话」人设冲突；**D14**：提示词与 40 个工具定义同时挤占上下文 |
| S2 | `agent.rs:364-371、373-396` | 历史按 120,000 字符预算裁剪（`agent.rs:174、205-219`）；超出部分以一句「更早 N 条消息没有附上」提示模型 | **D12**：更早的检索结论可能被裁掉；tool 调用/结果本来就不入库 |
| S3 | `agent.rs:404-1049` | 第 1..N 轮工具调用：`web_search` / `read_webpage` 收集素材；每轮 tool result 原样回灌（`agent.rs:1038-1044`） | 每次网页结果都可能很长，挤占同一预算 |
| S4 | `agent.rs:425-428、805-828` | 模型决定收尾：**一条 assistant 消息**里放一个 `create_docx` 的完整 `arguments`（正文全文） | **D1**：全文必须一次吐完；**D2**：无 `max_tokens`，截断即作废；`agent.rs:601-615` 对「工具调用参数不完整」的处理是**整轮重试/放弃**，不是续写 |
| S5 | `tools.rs:1231-1236` → `tools.rs:1417-1461` | 30 秒超时（`tools.rs:1125`）；校验 512KB（`tools.rs:1422`）；补 `.docx` 后缀、建父目录、写盘 | **D3**：30s 对本地写盘足够，但它是「整篇生成」链路里唯一的硬时限；**D4**：512KB 是防呆上限，真正先撞的是模型单条输出上限 |
| S6 | `tools.rs:1464-1499` | 手写 zip：`[Content_Types].xml` + `_rels/.rels` + `word/document.xml`，**只有 3 个 part** | **D11**：无 styles/numbering/settings/header/footer/docProps/rels → 无样式、无页码、无目录、无文档属性 |
| S7 | `tools.rs:1835-1956` | 逐行解析 Markdown：空行 flush、标题、列表、引用、表格、代码块；普通行 `join(" ")` 合成一段；末尾拼 `sectPr` | **D5/D6/D7/D8/D9/D10**：Markdown 保真度损失集中在这 122 行 |
| S8 | `tools.rs:1456-1460` | 返回 `"Word 文档已生成: <path>（<bytes> 字节）"`，回合结束（`agent.rs:1056-1077`） | **D12**：模型看不到结构/页数/目录/链接，无法自我纠正 |
| S9 | 用户手动打开 Word | 用户看到「标题加粗、无页码、无目录、`---` 字面量、来源不可点、表格挤压」；若要求修改，模型只能整篇重新生成（提示词 `lib.rs:2237` 就是这么教的） | 迭代成本=全文重写，且第二轮上下文里没有上一版正文 |

### 1.2 掉质点清单（逐条给证据）

| # | 掉质点 | 证据 | 直接后果 |
|---|---|---|---|
| **D1** | 单次全量输出 | 契约只有一个 `content` 字符串（`tools.rs:995`），没有 append/finalize；提示词把「修改已有文档」教成「重新生成覆盖」（`lib.rs:2237`）；错误提示还教「内容很长时分多次写入」（`agent.rs:844-846`），但每次 `create_docx` 都是整写覆盖（`tools.rs:1451-1454` `tokio::fs::write`）——对 docx 来说这条建议是空头支票 | 正文越长，截断/超时/JSON 非法的概率越接近 1；修改=全文重写 |
| **D2** | 无输出预算参数 | 三个请求体构造点都只有 `model/input 或 messages/stream/reasoning_effort/tools`（`api_client.rs:1014-1037、1120-1141、1193-1214`）；`src-tauri/src/direct_api` 全目录 `max_tokens` 0 命中 | 长文能否一次吐完取决于用户配的模型默认上限；链路既不知道也不控制 |
| **D3** | 30 秒工具超时 | `tools.rs:1125` 定义 `timeout_duration = 30s`；`tools.rs:1231-1236` 专用于 create_docx，超时只回一句「生成 Word 文档超时 (30秒)」 | 大文档或慢盘上，重试仍是「重新生成全文」 |
| **D4** | 512KB 内容上限 | `tools.rs:1422-1429` | 与 D1/D2 叠加：真正卡住的是模型输出 token，而非 512KB；但错误提示会引导模型「缩短内容」而非「分块写」 |
| **D5** | Markdown 子集即全部表达力 | 描述与实现：`tools.rs:989` vs `parse_inline_runs`（行内格式只认反引号、`**`、`*`，`tools.rs:1528-1559`）；无图片、无链接、无脚注、无表格合并、无列对齐 | 封面/作者/目录/页码/超链接/图片/多级编号在契约里**无字段可写** |
| **D6** | 软换行被合并 | `markdown_to_document_xml` 的 `flush_para`：`para_lines.join(" ")`（`tools.rs:1846-1853`） | 模型「一行一个字段」的写法被压成一整段（产物实证 E1，§5） |
| **D7** | `---` 未处理 | 解析循环（`tools.rs:1872-1945`）没有任何水平分隔线分支；产物尾部 `w:p` 文本就是字面量 `---`（本次解包 `literal_dashline=[121]`） | 分隔线变成正文垃圾字符（产物实证 E2） |
| **D8** | 假列表 | `list_paragraph` 手写 `"• "` / `"{num}. "` 前缀 + `w:ind` 悬挂缩进（`tools.rs:1654-1675`）；没有 `numPr` | 增删条目不重排、无法多级、复制到其他系统变纯文本（产物实证 E5） |
| **D9** | 表格均分列宽 + 无 cell 级控制 | `col_w = total_width / cols`（`tools.rs:1724-1725`）；`w:tc` 里没有 `w:tcW`（`tools.rs:1757-1759`）；只有 `gridCol` | 「关键要求」长列和「时间」短列等宽，中文被挤压换行；跨页行可能被劈开（无 `cantSplit`） |
| **D10** | 超链接不解析 | `build_docx_bytes` 只写 3 个 part，没有 `word/document.xml.rels`（`tools.rs:1475-1495`）；产物 `w:hyperlink=0`，正文 `https?://` 命中 0 | 来源不可点击、URL 在读回时彻底丢失；报告的可核验性归零（实证 E3） |
| **D11** | 无样式/目录/页码/封面/属性 | `heading_paragraph` 只设字号加粗，不设 `w:pStyle`/`w:outlineLvl`（`tools.rs:1628-1651`）；`sectPr` 只有 `pgSz/pgMar`（`tools.rs:1951-1952`）；无 `headerReference/footerReference`、无 TOC 域、无 `docProps/core.xml` | BRIEF：29-33 的 5 条核心缺陷全部成立；Word 导航窗格空、无法插目录、无页码、无封面 |
| **D12** | 结果不可见 + 不可迭代 | 工具结果 = 路径+字节数（`tools.rs:1456-1460`）；无校验/巡检工具；历史不保存 tool 调用（`db.rs:103-123`，重建历史只看 `content`/`thinking`：`agent.rs:1148-1178`）；`read_file` 读 docx 只回纯文本（`tools.rs:1370-1395、2059-2104`） | 「生成→看见→修正」闭环缺失；这是质量不可收敛的根本原因 |
| **D13** | 提示词没有文档方法论 | Word 段 20 行、只讲参数格式（`lib.rs:2226-2249`）；基础人设要求短回复（`client.rs:40`） | 模型不会「先提纲再写」「字段独立成段」「表格列意图」「写完自检」 |
| **D14** | 工具面过宽 + 提示词膨胀 | `tool_definitions()` 共 40 条工具（`tools.rs:471-1104`，`"type":"function"` 计数 40），另有协作/技能/记忆等 14 段注入（`lib.rs:4340-4400`） | 文档任务没有专属工具/专属规则，注意力被通用工具摊薄；对比 DSH 路径有 `office-docx` 技能专门描述整条工作流 |
| **D15** | 标题与 title 语义重叠 | 工具描述 `title` =「文档大标题，居中显示在正文开头」（`tools.rs:994`）；提示词又说「title 参数可选，作为文档首页大标题」（`lib.rs:2235`）；正文还能用 `#` | 模型可能把 `#` 留给标题、正文从 `##` 开始（产物实证：一级章节是 `##`，`title` 单独成段），也可能重复写标题；契约无明确规则 |
| **D16** | 覆盖式编辑破坏用户手工排版 | 提示词 `lib.rs:2237` 明确让模型「用 create_docx 重新生成覆盖」；`read_file` 回读只给纯文本（`tools.rs:1370-1395`），样式信息全丢 | 用户只要在 Word 里调过一次格式，下次「再改改」就全部丢失 |

### 1.3 结构性归因：为什么这些掉质点是「必然」而不是「偶发」

把上面 16 条按性质分成三组，可以看到它们不是 16 个独立 bug，而是同一个架构决定的三个簇：

- **簇 A（D1/D2/D3/D4）＝「一次性、不可分割的生成事务」。** 契约里没有「分段提交」的概念，工具调用是一次原子事务：要么整篇成功，要么整篇失败。这类链路天然不适合长文——长文需要的是「多次小写入 + 收尾合并」。
- **簇 B（D5/D6/D7/D8/D9/D10/D15）＝「表达通道与渲染器之间的保真度缺口」。** 模型侧表达（Markdown/结构意图）→ 解码器（122 行手写逻辑）→ OOXML（3 part 极简包）。链路中**没有任何一环能承载「文档工程」**（样式、节、域、关系、属性）。这就是「内容不差但文档很差」的直接机制：内容在模型这一侧是完好的，损失发生在后两环。
- **簇 C（D11/D12/D13/D14/D16）＝「文档工程完全没有托管方」。** 没有样式系统、没有校验、没有巡检、没有作者规范、没有迭代入口。对比 CLI/DSH 路径，这些能力要么由模型脚本直接实现，要么由技能/工具提供；直连 API 路径两者都没有。

一句话：**直连 API 路径把「写内容」和「做文档」拆给了两个能力极不对称的组件，而它们之间只有一根 122 行的 Markdown 解码器相连。**

### 1.4 与 Claude Code / DSH + office-docx 路径的能力差

本仓库同时存在两条后端：直连 API（进程内固定工具）与 Claude Code CLI（`client.rs`，子进程）。本次研究环境则是 DSH + `office-docx` / `office-render` 技能。

| 维度 | 直连 API（本仓库实现） | Claude Code CLI（本仓库另一后端） | DSH + office-docx 技能（本次环境） |
|---|---|---|---|
| 工具面 | 40 条固定 JSON Schema（`tools.rs:471-1104`），增删工具要改 Rust + 前端清单 + 契约测试 | Claude Code 自带 Bash/Read/Edit/Write + MCP 桥（`client.rs:380-394、419-434`）；提示词用 `--append-system-prompt-file` 落盘传入 | 技能即工具：`office-docx`（“Create, read, edit, and validate Word documents”）+ `office-render`（“Render requested document images or inspect visual layout”）；可用 python-docx / 手写 OOXML / Word COM（BRIEF:118） |
| 表达通道 | Markdown 子集 → 3-part 极简 docx | 任意脚本/任意库，模型可自己产出完整 OOXML 包 | 技能规定全流程：part 集、样式、校验、渲染；模型可直接控制 XML |
| 会话记忆 | 只存 `content/thinking`，tool 调用不入库（`db.rs:103-123`）；重建历史见 `agent.rs:1148-1178` | `--resume <session>` 复用 CLI 自己的完整轨迹（`client.rs:123-124`） | 多轮会话 + 工具轨迹（DSH 会话） |
| 验证能力 | 无（唯一「验证」是 roundtrip 测试里断言文本存在：`tools.rs:5747-5755`） | 可跑测试、可打开产物检查 | `validate` + 渲染截图，可看到真实版面 |
| 迭代方式 | 整篇重新生成（`lib.rs:2237`），无局部编辑 | `edit_file` 局部替换，可多轮修 | 可改 XML part、可局部重排、可反复渲染比对 |
| 规划 | 无 `update_plan`（`src-tauri/src/direct_api` 全目录 0 命中）；前端却为它专门渲染清单（`AgentActivity.vue:81、136-158`，那是 CLI 路径工具） | 有 plan 工具 | 有任务清单/多步规划 |
| 失败模式 | 截断=整篇作废（`agent.rs:839-862`），重试=重新生成 | 失败可局部重来 | 脚本报错可读、可修、可重跑 |
| 文档类提示词 | 20 行参数说明（`lib.rs:2226-2249`） | 桌宠人设 + 技能注入（`lib.rs:2348-2395`） | SKILL.md 级工作流 + 校验清单 |
| 用户可干预点 | 授权弹窗（每类工具首次确认，`lib.rs:1880`） | 权限模式（`client.rs:130-138`） | 技能/工具权限 |

结论：**差的不是模型，而是「文档工程」在直连 API 链路里没有任何承担者**。这一点与本仓库把 CLI 路径和直连路径做成两套执行器直接相关（`client.rs:111-127、374-394` vs `agent.rs:404-1049`）。

---

## 2. 契约缺陷与改进草案

### 2.1 现有 `create_docx` 契约能表达 / 不能表达

现有 Schema（`tools.rs:985-1000`）：`path`（必填）、`content`（必填，Markdown）、可选 `title`。

| 能表达（但会继续在解码器里掉） | 完全无法表达（契约里没有字段） |
|---|---|
| `#/##/###/####` 标题（最多 4 级，`tools.rs:1789-1794`） | 封面（副标题/作者/机构/日期/密级/版本） |
| 段落、`**粗体**`、`*斜体*`、`` `行内代码` `` | 目录（TOC 域）、标题自动编号 |
| `-`/`1.` 列表（手写前缀，非真编号） | 页眉、页脚、页码、分节符（封面单独一节） |
| `> ` 引用 | 超链接（URL 可点击）、图片、脚注、书签/交叉引用 |
| Markdown 表格（均分列宽、无单元格宽/对齐/合并） | 表格列宽权重、列对齐、表题、跨页行控制 |
| 代码块（围栏语法） | 页面尺寸/方向/页边距/字体方案 |
| 文首大标题（只变成一段居中大字） | 文档属性（`docProps/core.xml`：title/author/created） |
| — | 分块写入（append）、收尾（finalize）、局部编辑 |
| — | 校验/巡检结果（结构统计、warnings） |

一句话：契约表达能力 ≈「一张打印纸」，而专业报告需要的是「一本装订好的文档」。

### 2.2 改进后的 JSON Schema 草案（`create_docx` v2，可直接替换 `tools.rs:985-1000`）

设计原则：

1. **双通道**：`markdown`（v2 解析器）或 `blocks`（结构化块）；两者都不传时回退到 v1 `content`。
2. **`mode` 三态**：`create`（一次成文）/ `append`（追加正文块到草稿）/ `finalize`（合并草稿、渲染完整包、返回结构摘要）。
3. **`frontmatter` 独立成字段**，不再靠正文里的 `---` 区块（避免与分隔线语义冲突）。
4. **`options` 把「文档工程开关」显式化**：toc / pageNumbers / cover / headingNumbering / page / body / table。
5. **块类型只放模型真会用的 10 种**（heading/paragraph/list/table/quote/code/divider/pageBreak/image/caption），行内仍允许 `**粗体** *斜体* `code` [链接](url)`，降低模型学习成本。

```json
{
  "name": "create_docx",
  "description": "创建、续写或完成一个 Word 文档（.docx）。支持封面、目录、页码、页眉、真编号列表、超链接、图片、按内容分配列宽的表格，并返回结构校验摘要。写入长文档时用 mode=append 分章提交，最后用 mode=finalize 合并；不要把所有内容塞进一次调用。读取已有文档内容请用 read_file。",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "保存路径，如 C:\\Users\\用户名\\Desktop\\报告.docx；不带 .docx 后缀会自动补上。append/finalize 时传同一个路径。"
      },
      "mode": {
        "type": "string",
        "enum": ["create", "append", "finalize"],
        "default": "create",
        "description": "create=一次成文（≤6 页推荐）；append=把本次的 markdown/blocks 追加到该路径的草稿（长文分章用）；finalize=合并草稿、生成完整 docx 并返回结构摘要。"
      },
      "title": { "type": "string", "description": "（v1 兼容，建议改用 frontmatter.title）文档大标题" },
      "content": { "type": "string", "description": "（v1 兼容）文档正文 Markdown；不传 blocks 时按 v1 语义解析" },
      "markdown": {
        "type": "string",
        "description": "v2 Markdown 正文。支持软换行（单换行=段内换行 w:br）、#/##/### 三级标题、- / 1. 列表、| 表格 |、> 引用、``` 代码块、**粗体** *斜体* `代码`、[文字](url) 超链接、单独一行 --- 作水平分隔线。"
      },
      "blocks": {
        "type": "array",
        "description": "结构化内容块，与 markdown 二选一。适合需要精确控制层级、列宽、链接、图片的场景。",
        "items": {
          "oneOf": [
            { "type": "object", "required": ["type", "text"], "properties": {
                "type": { "const": "heading" },
                "level": { "type": "integer", "minimum": 1, "maximum": 3 },
                "text": { "type": "string" } } },
            { "type": "object", "required": ["type", "text"], "properties": {
                "type": { "const": "paragraph" },
                "text": { "type": "string" },
                "style": { "type": "string", "enum": ["body", "lead", "note"] } } },
            { "type": "object", "required": ["type", "items"], "properties": {
                "type": { "const": "list" },
                "ordered": { "type": "boolean" },
                "start": { "type": "integer", "minimum": 1 },
                "items": { "type": "array", "items": { "type": "string" } } } },
            { "type": "object", "required": ["type", "columns", "rows"], "properties": {
                "type": { "const": "table" },
                "caption": { "type": "string" },
                "columns": { "type": "array", "minItems": 1, "maxItems": 5, "items": {
                  "type": "object", "required": ["label"], "properties": {
                    "label": { "type": "string", "description": "列名（≤6 字）" },
                    "width": { "type": "number", "minimum": 1, "maximum": 10, "description": "列宽权重，1-10；不传则按内容长度自动分配" },
                    "align": { "type": "string", "enum": ["left", "center", "right"] } } } },
                "rows": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } } } },
            { "type": "object", "required": ["type", "text"], "properties": {
                "type": { "const": "quote" }, "text": { "type": "string" } } },
            { "type": "object", "required": ["type", "text"], "properties": {
                "type": { "const": "code" }, "language": { "type": "string" }, "text": { "type": "string" } } },
            { "type": "object", "required": ["type"], "properties": { "type": { "const": "divider" } } },
            { "type": "object", "required": ["type"], "properties": { "type": { "const": "pageBreak" } } },
            { "type": "object", "required": ["type", "path"], "properties": {
                "type": { "const": "image" }, "path": { "type": "string" },
                "widthCm": { "type": "number", "minimum": 1, "maximum": 16 },
                "caption": { "type": "string" } } },
            { "type": "object", "required": ["type", "text"], "properties": {
                "type": { "const": "caption" }, "text": { "type": "string" } } }
          ]
        }
      },
      "frontmatter": {
        "type": "object",
        "description": "文档元信息，写入封面、页眉与 docProps/core.xml。",
        "properties": {
          "title": { "type": "string" },
          "subtitle": { "type": "string" },
          "author": { "type": "string" },
          "org": { "type": "string" },
          "date": { "type": "string", "description": "YYYY-MM-DD" },
          "subject": { "type": "string" },
          "version": { "type": "string" },
          "confidentiality": { "type": "string", "description": "如 公开 / 内部资料 / 机密" },
          "keywords": { "type": "array", "items": { "type": "string" } }
        }
      },
      "options": {
        "type": "object",
        "description": "文档工程开关；不传时使用推荐默认值（有 title 则出封面、二级标题≥3 则出目录、始终出页码）。",
        "properties": {
          "cover": { "type": "boolean", "description": "独立封面节 + 封面内容（默认 true）" },
          "toc": { "type": "boolean", "description": "插入 TOC 域（默认 true，标题≥3 时生效）" },
          "pageNumbers": { "type": "boolean", "description": "页脚「第 X 页 / 共 Y 页」（默认 true）" },
          "header": { "type": "string", "description": "页眉文字，默认取文档标题" },
          "headingNumbering": { "type": "boolean", "description": "标题自动编号（一、/1.1/1.1.1），默认 false" },
          "page": { "type": "object", "properties": {
              "size": { "type": "string", "enum": ["A4", "A5", "Letter"] },
              "orientation": { "type": "string", "enum": ["portrait", "landscape"] },
              "margin": { "type": "string", "enum": ["normal", "narrow", "wide"] } } },
          "body": { "type": "object", "properties": {
              "firstLineIndent": { "type": "number", "description": "首行缩进字符数，默认 2" },
              "lineSpacing": { "type": "number", "description": "行距倍数，默认 1.5" },
              "font": { "type": "string", "enum": ["微软雅黑", "宋体", "等线"] } } },
          "table": { "type": "object", "properties": {
              "widthMode": { "type": "string", "enum": ["auto", "equal", "weighted"], "description": "默认 auto：按内容长度分配" },
              "zebra": { "type": "boolean" },
              "headerRepeats": { "type": "boolean" },
              "cantSplit": { "type": "boolean" } } }
        }
      }
    },
    "required": ["path"],
    "anyOf": [
      { "required": ["content"] },
      { "required": ["markdown"] },
      { "required": ["blocks"] },
      { "required": ["mode"] }
    ]
  }
}
```

说明：

- `anyOf` 只做「至少给了一种内容载体」的软约束；`mode=append` 需要内容、`mode=finalize` 不需要，由运行时校验补足（模型传错时返回可读错误，而不是 schema 校验失败）。
- `api_client.rs` 的 `responses_tool_definitions` 会原样透传 `parameters`、不额外加 `strict`（`api_client.rs:965-1002`），因此 `oneOf/anyOf` 可安全使用，不会触发 provider 的 strict 校验拒绝。
- `blocks` 的 `table.columns.width` 是「权重」而非绝对值，最终 twips 由渲染器按 A4 内容宽 9026（`tools.rs:1724`）归一化，避免模型算数。

### 2.3 工具返回值：从「字节数」升级为「结构摘要 + warnings」

现状（`tools.rs:1456-1460`）：

```text
Word 文档已生成: C:\Users\15188\Desktop\AI领域新闻报告汇总_2026年11月.docx（9407 字节）
```

这正是 D12 的病灶：模型拿到的唯一反馈是「文件写出来了」。建议 v2 返回（人可读一行 + JSON，兼顾旧前端/旧模型）：

```text
Word 文档已生成: C:\...\报告.docx（48213 字节，约 7 页）
{"ok":true,"engine":"docxgen/2","path":"C:\\...\\报告.docx","bytes":48213,"parts":11,
 "stats":{"paragraphs":96,"headings":{"h1":6,"h2":5,"h3":18},"tables":3,"lists":5,"links":9,
          "toc":true,"pageNumbers":true,"cover":true,"estPages":7},
 "warnings":[]}
```

`warnings` 的建议预置（每条 ≤60 字，整体 ≤800 字，避免工具结果反过来挤爆上下文）：

- `"标题未使用命名样式（pStyle=0），导航窗格与目录不可用"`
- `"有 3 个段落超过 200 字，疑似多个字段被合并，建议改 blocks/markdown 软换行"`
- `"表格 3 最长单元格 78 字 / 最短 7 字，列宽不均分已自动加权，建议拆分或改列"`
- `"检测到字面量 ---（已按水平分隔线渲染）"`
- `"缺少 docProps/core.xml，文档属性为空"`
- `"页脚未检测到 PAGE 域"`

这样每次 `create/finalize` 都自带「零成本校验回合」：模型不需要额外工具调用就能知道哪里需要修，修正发生在**同一次对话**里，而不是等用户打开 Word 才发现。

### 2.4 向后兼容策略与落地清单

**兼容性设计（4 条硬规则）：**

1. **字段探测，不升级必崩**：`exec_create_docx` 先看 `mode / blocks / frontmatter / options / markdown` 是否出现；任一出现 → v2 流程；否则 → v1 流程（`content` + `title`）。`content` 永不废弃，作为 v1 别名长期保留。
2. **`title` 语义收敛**：v1 中 `title` 只是「正文开头居中大字」（`tools.rs:1612-1625`）；v2 把它同时写入封面与 `docProps/core.xml`，正文不再重复插入标题段。这是输出改善，不是接口破坏。
3. **返回值向后兼容**：保留「Word 文档已生成: 路径（N 字节）」这行人类可读前缀（旧前端只展示字符串，`AgentActivity.vue:314-320`），JSON 另起一行；旧调用方不受影响。
4. **错误契约不变**：参数被截断时仍返回「工具参数不是完整的 JSON……请重新调用并给出完整参数；内容很长时分多次写入」这句（`agent.rs:844-846` 既有文案），v2 只是在后面补一句「可改用 mode=append 分章提交」。注意这条文案在 v1 里是空头支票：`create_docx` 每次整写覆盖（`tools.rs:1451-1454`），模型没有「分多次写入」的手段。

**落地清单（Rust / 前端 / 测试）：**

| 落点 | 改动 | 备注 |
|---|---|---|
| `tools.rs:985-1000` | 替换 schema 为 §2.2 | 只改 JSON 定义，零行为变化 |
| `tools.rs:1417-1461` | `exec_create_docx` 增加 v2 分支 + `mode` 状态机 + 草稿侧车文件（`<path>.docx.draft.json`） | `append` 只写草稿，`finalize` 渲染完整包 |
| `tools.rs:1464-1499` | `build_docx_bytes` 扩为完整 part 集（细项由 task-1/2 的 XML 规格提供） | 本审计只定契约接口，不定 XML 细节 |
| `tools.rs:1835-1956` | 解析器 4 处硬修复：软换行、`---`、标题 pStyle/outlineLvl、表格列宽加权 + `tcW`/`cantSplit` | 见 §6 P0-2 |
| `tools.rs` 新增 | `inspect_docx` 工具（§4） | 复用 `extract_open_xml_text`/`parse_xml_tag` 的扫描思路（`tools.rs:2059-2128`） |
| `tools.ts:8-45、49-86` | 新增 `inspect_docx` 到 `AVAILABLE_TOOL_NAMES` 与 `TOOL_LABELS` | **必须同步**：不在清单里的工具会被工坊静默丢弃（`tools.ts:1-7`） |
| `AgentActivity.vue:59-66` | 别名表加 `inspect_docx: "检查文档结构"`；可在 tool 详情里渲染 warnings 列表（可选） | 现有 `<pre>` 已能显示 JSON（`AgentActivity.vue:313-320`） |
| `lib.rs:2226-2249` | 用 §3.2《文档作者规范》替换 Word 段 | 与 `tools.rs:989` description 同步改写 |
| `tools.rs:6240-6245` | 前后端工具清单契约测试必然失败 → 更新测试里的期望清单 | 这是有意的契约变更，不是绕过 |
| `tools.rs:5702-5756` | 扩展 roundtrip 测试：断言 `pStyle>0`、`outlineLvl>0`、`hyperlink` 存在、`docProps/core.xml` 存在、v1 调用仍成功 | 现在的测试只断言「3 个 part + 文本存在」（`tools.rs:5725-5727、5747-5755`），样式回归完全测不出来 |

---

## 3. 提示词改造

### 3.1 现状与缺口

当前直连 API 路径里，与 Word 有关的提示词**全部内容**是（`lib.rs:2230-2237`，原文引用）：

```text
【Word 文档生成能力】
你可以使用 create_docx 工具把内容写到 .docx 文件（Word 文档）。当用户要求生成 Word 文档、
导出报告、写论文/方案并保存为文档时，优先用 create_docx 而不用 write_file 写纯文本。用法：
- content 参数是 Markdown 内容，支持：# ## ### 多级标题、段落、**粗体** *斜体* `行内代码`、- 无序列表、1. 有序列表、引用块、围栏代码块（三个反引号）、| 表格 | 语法。
- path 参数是保存位置；用户未指定路径时保存到桌面，文件名根据内容主题拟定。
- title 参数可选，作为文档首页大标题。
- 生成成功后在回复中告知用户完整保存路径。
- 若用户要求修改已有 .docx 的内容，先用 read_file 读取，确认需求后用 create_docx 重新生成覆盖。
```

缺口（逐条对照任务要求的能力）：

1. 没有「先提纲再写正文」；
2. 没有「字段独立成段、`**字段名：**` 开头」；
3. 没有标题层级约束（模型可以写 `####`、可以不用 `#` 而全用 `##`）；
4. 没有篇幅/段落长度约束（一屏/一段多少字）；
5. 没有表格列数与「列意图」约束；
6. 没有来源写法（`[来源](url)` 可点击）；
7. 没有「不要用 `---` 分隔」；
8. 没有长文分段写入（当前也没有 append 契约可写）；
9. 没有写完自检清单；
10. 与基础人设冲突：`client.rs:40` 要求「默认回复简短自然，通常 2-3 句话即可」，模型在「聊天回复要短」与「文档要长」之间没有任何仲裁规则；
11. 反而教了最坏的一条：修改已有文档 = 全文重新生成覆盖（`lib.rs:2237`）。

### 3.2 可直接粘贴的《文档作者规范》（替换 `lib.rs:2226-2249` 的 Word 段）

> 使用方式：整段替换 `【Word 文档生成能力】` 那一块；旧的 5 条参数说明只保留 path 默认值一句（或整体删除，参数语义已由工具 schema 的 `description` 承担）。规范里的字段名/工具名与 §2.2 的 v2 契约一一对应。

```text
【文档作者规范】本规范适用于「用户要求写报告 / 论文 / 方案 / 简报 / 白皮书，并保存为 Word 文档」的任务；它优先级高于桌宠「默认回复简短」的人设——聊天回复仍然简短，但写入工具的正文必须完整。

一、写之前（先想清楚，再动笔）
1. 先用一句话复述任务：受众是谁、这份文档要解决什么问题、大概几页。缺关键信息（受众、篇幅、截止信息、对外/对内）时用 ask_user 一次问清 2-3 个点；能合理推断的直接决定，不要反复打断。
2. 先列提纲再写正文：用 3-7 个二级标题搭骨架，每个标题下先想 2-4 个要点。不要在提纲确定前就开始成段写作。
3. 素材不足先检索：涉及最新动态、数字、金额、法规、产品发布的信息，必须先 web_search + read_webpage 核实；每个关键事实至少要有一个可引用来源，无法核实的明确标「待核验」。

二、结构规范
1. 标题只用三级：#（一级，建议写成「一、报告说明」这类中文序号）、##（二级）、###（三级）。不要用 ####，不要混用两套编号体系。文档大标题走 frontmatter.title，不要在正文里再写一遍。
2. 每个二级章节写 3-8 段；每段只讲一个观点，200 字以内（约 4-6 行）；超过 200 字就拆成两段。一屏（约 20 行）内必须能读到一个完整论点。
3. 字段型信息一行一个字段，用 **字段名：** 开头，例如：
   **覆盖周期：** 2026 年 11 月 1 日 — 11 月 14 日。
   **成稿时间：** 2026 年 11 月 14 日。
   不要把多个字段塞进同一段，也不要用软换行把字段挂在段落末尾。
4. 列表：无序条目用「- 」开头，一条一行、一条不超过 2 行；有序条目用「1. 」开头。不要手打「•」「·」「①」当列表符号，也不要用缩进做多级列表（渲染器只支持单级；需要层级就拆成小节）。
5. 表格：列数 ≤ 5，第一行是表头；表头名词化（≤6 字）；必须给每列一个明确意图（这列回答读者什么问题）。长文本列（如「关键要求」）不要和短列（如「时间」「#」）等权：写完后检查「最长单元格字数 ÷ 最短单元格字数」，大于 3 就拆表或改成条目段落。表格只放有比较价值的信息，不要把整段话塞进单元格。
6. 来源必须可点击：写成 [来源名称](https://完整URL)；不要把裸域名当来源（例如只写「来源：name.ai」）。聚合站来源要同时写明「待核验」。全文最后一节放「来源与可信度分级」表格。
7. 不要使用 ---、***、___ 作为分隔线（当前渲染器会输出字面量；改用新的二级标题或空行分段）。同理不要用 emoji、HTML、连续特殊符号做视觉分隔。
8. 数字与单位之间留空格（110 亿美元）；同一指标全文只用一个口径（金额/估值/同比不要混写）；不确定的数字写「约」并标来源。

三、写入方式（工具协议）
1. 篇幅 ≤ 6 页（约 6000 字）时：一次调用 create_docx，传 frontmatter（title/subtitle/author/date）+ options（cover=true、toc=true、pageNumbers=true）+ markdown（或 blocks）。
2. 篇幅 > 6 页时必须分块：create 先写封面/目录/第一节；之后每次 append 写 1-2 个章节（单次 ≤ 1500 字）；最后 finalize 合并并触发结构校验。绝不把整本塞进同一个 content。
3. 长文正文优先用 markdown 通道：一段一行、字段一行一个、空行分段；需要精确控制表格列宽/图片/链接时改用 blocks。
4. 生成后必须读工具返回的 stats 与 warnings：
   - warnings 非空 → 按提示修一版再交付（例如「标题未使用命名样式」说明结构与渲染不匹配、「疑似字段被合并」说明段落太挤、「表格列宽异常」说明表该拆）；
   - warnings 为空也要检查 stats：页数为 0 或标题层级为 0 属于异常，重试。
5. 修改已有文档：先 read_file 读回纯文本确认内容，再整篇 create（当前不支持局部样式保留，要在回复里说明这一点）。同一路径再次 create 会覆盖。
6. 参数被截断/接口报「工具参数不是完整的 JSON」时：不要原样重试整篇；改成 append 一个子章节后重试。

四、交付前自检（逐条打勾，不合格先改，不要发给用户）
- [ ] 标题层级 ≤ 3 级，且每级都有内容
- [ ] 每个字段独立成段，并以 **字段名：** 开头
- [ ] 全文没有 ---、***、####、手打「•」
- [ ] 表格 ≤ 5 列；最长/最短单元格字数比 ≤ 3；每列表头有明确意图
- [ ] 每个关键数字/法规/日期都有来源，且来源是 [名称](URL) 可点击
- [ ] 没有「据说」「可能」「业内人士称」被当作结论；不确定项已标「待核验」
- [ ] 结尾有「报告边界 / 免责声明」段
- [ ] create_docx 返回的 warnings 为空（或已逐条处理）

五、回复用户
给完整保存路径、页数/章节数估计、3 句以内的要点摘要；不要把正文复述一遍。
```

### 3.3 接入与冲突处理

- **替换位置**：`lib.rs:2226-2249` 的 `attach_word_and_ask_prompt`；`ask_user` 段（`lib.rs:2239-2246`）保留不动。工具描述 `tools.rs:989` 与 `tools.rs:994` 同步改成 v2 描述（`title` 不再是「居中显示在正文开头」，而是「文档标题，写入封面与文档属性」）。
- **人设冲突**：在规范开头显式声明优先级（已写进草稿第一段）。不要把整段规范塞进 `BASE_SYSTEM_PROMPT`（`client.rs:40`），否则闲聊也会变啰嗦；只在 direct API 的 Word 段注入。
- **每回合都在**：`lib.rs:4395` 每次会话都 attach `attach_word_and_ask_prompt`，所以这段规范是常驻的，不依赖用户激活技能。**不建议**把它做成内置技能：内置技能同一时间只能激活一个（`db.rs:425-446` 的 `skills_single_active_*` 触发器），会把「文档阅读」等其他技能顶掉；而且技能只有被手动激活才注入（`lib.rs:2348-2395`）。
- **上下文成本**：草稿约 2,000 字（≈3k tokens），与 120,000 字符历史预算（`agent.rs:174`）相比占比可控；如果担心挤占，可把「五、回复用户」并进输出风格，保留四节主体。
- **与 v1 的兼容期**：v1 通道仍接受旧式 Markdown（模型老写法不会报错），但会在 warnings 里提示「建议改用 frontmatter/options」；规范第三节让模型优先走 v2。

---

## 4. 反馈闭环：三种低成本方案对比与推荐

目标：让模型**在交付前**看到「生成的文档长什么样」，从而自我修正。三个候选：

### 4.1 三方案对比

| 维度 | A. `read_file` 回读纯文本 | B. `inspect_docx` 结构摘要工具 | C. 渲染截图进多模态 |
|---|---|---|---|
| 实现基础 | 已有：`read_file` 支持 `.docx`（`tools.rs:477、1271`），提取纯文本（`tools.rs:1370-1395、2059-2104`） | 新增：复用 zip + 手写 XML 扫描（`extract_open_xml_text`/`parse_xml_tag`，`tools.rs:2059-2128`）+ 结构探针 | 本机 Word COM 导出 PDF→PNG（BRIEF:111-119）；或 `open_app` + `computer_screenshot`（工具表已有） |
| 能看到什么 | 字面量 `---`、漏段/错序、残字符、表格单元格逐行文本（无列边界，每个 cell 的 `</w:p>` 被转成换行，`tools.rs:2092-2093`）；**看不到**样式/大纲/列宽/页码/页眉/目录/URL（链接 URL 在 rels 里，提取器只取 `w:t`，URL 直接丢失） | part 集完整性、`pStyle/outlineLvl/numPr/hyperlink` 计数、页眉页脚/TOC 域、表格列宽与最长单元格、超长段落、字面量残留、core.xml 字段、`sectPr` 数量 | 真实版面：分页、表格是否跨页断裂、孤行/寡行、目录页码、封面观感——唯一能看「排版」的方案 |
| 看不到什么 | 样式、版面、链接、页数 | 真实分页（Word 的自动排版无法静态计算），孤行/寡行 | 文本错误（截图不读字，模型只能目测） |
| 额外成本 | 0（已有工具） | ~150-300 行 Rust + 1 个工具 schema + 前端清单同步（`tools.ts:8-45、49-86`、`AgentActivity.vue:59-66`） | 依赖 Word COM/PowerShell（需用户授权 `run_command`，`lib.rs:1880`）；5-15s/份；图片 token ≈1-2k/页；`MAX_VISION_IMAGES=5`（`agent.rs:22`）；焦点抢占（`agent.rs:963-968` 已有焦点恢复逻辑，说明这类问题真实存在） |
| Token 成本/次 | ≈ 文档正文字数（可到全文；512KB 提取上限 `tools.rs:2211-2220`） | ≈400-800 字符 JSON | ≈1-2k/页 × 页数 |
| 对模型的可用性 | 低：内容大多已经正确，回读只能确认「没丢字」 | **高**：直接给「缺什么、差在哪、怎么改」的清单，能驱动自我修正 | 高但非确定性（模型对截图的判断不稳定） |
| 工作量 | S（0 新增，写提示词即可） | **M** | L |
| 风险 | 回读以为「没问题」（文本正确≠排版正确），反而强化错误信心 | 探针误报/漏报（需与渲染器版本绑定）；返回值变长 | Word 弹窗/焦点/超时/环境缺失（LibreOffice 不可用，BRIEF:117）；成本高、不可靠 |

### 4.2 推荐：B 为主 + A 兜底 + C 只做终稿视觉验收

1. **第一步（P0，必做）：把结构摘要内嵌进 `create_docx`/`finalize` 的返回值（§2.3）。** 这不是「多一个工具」，而是把「校验」变成写文档的**副作用**：零额外回合、零额外授权、模型必看得到。这是投入产出比最高的一步。
2. **第二步（P1）：独立 `inspect_docx` 工具**，用于（a）模型对 `append` 之后的中间态复查；（b）前端「检查文档」按钮，让用户/模型对旧文档做体检；（c）回归测试脚本直接调用。
3. **第三步（P2，可选）：C 方案仅用于「正式对外文档」的终稿验收**，且只在用户明确要求或 ≥5 页时启用；失败时静默跳过，不影响交付。
4. **A 方案不作为独立步骤**，但保留在提示词里作为兜底话术（「交付前可用 read_file 回读确认没有丢段」）；明确它的局限（样式/版面不可见），避免模型误以为「回读通过=文档合格」。

**推荐理由（一句话）**：B 能在**同一回合内**把「生成→检查→修正」闭合，成本是 M 级一次性投入，且不引入任何外部运行时依赖；A 太弱、C 太贵且不稳定，适合做兜底/验收而不是主循环。

### 4.3 `inspect_docx` 探针清单（与 BRIEF §3 的 14 条缺陷对齐）

| 探针 | 检测内容 | 静态可测 | 对应 BRIEF 缺陷 |
|---|---|---|---|
| PS-01 | 必需 part 是否齐全（styles/numbering/settings/header/footer/docProps/rels/theme/fontTable/webSettings） | ✅ | 1-13（包完整性） |
| PS-02 | `w:pStyle` 计数 > 0，且 Heading1-3 都被引用 | ✅ | 3、4 |
| PS-03 | `w:outlineLvl` 覆盖全部标题段落，级别 0-2 | ✅ | 3、6 |
| PS-04 | `w:numPr` 计数与列表条目数一致；无手写「•」残留 | ✅ | 7 |
| PS-05 | `w:hyperlink` 数 ≥ 正文 URL 数；`document.xml.rels` 存在 external 关系 | ✅ | 10 |
| PS-06 | 页眉/页脚引用存在；页脚含 `PAGE`/`NUMPAGES` 域 | ✅ | 4 |
| PS-07 | `sectPr` ≥ 2（封面节/正文节），封面节首页不同 | ✅ | 5 |
| PS-08 | 目录域 `TOC \o "1-3" \h \z \u` + `settings.xml` 的 `updateFields` | ✅ | 6 |
| PS-09 | 表格 `gridCol` 宽比与最长单元格字数比偏差 > 2 倍告警；无 `w:tcW` 告警；无 `cantSplit` 告警 | ✅ | 8、9 |
| PS-10 | 字面量残留扫描：`---`、`####`、`**`、`](`、裸竖线泄漏进文本 | ✅ | 1、2 |
| PS-11 | 超长段落（>200 字）+ 「同段多个 `**字段名：**`」模式 | ✅ | 2 |
| PS-12 | `docProps/core.xml` 存在且 title/creator/created 非空 | ✅ | 12 |
| PS-13 | 首行缩进/行距/中文字体（`w:ind firstLine`、`line=360`、`eastAsia`） | ✅ | 11、14 |
| PS-14 | 分页观感（表格跨页断裂、孤行、目录页码） | ❌（需渲染） | 归 C 方案 |

静态探针能覆盖 14 条里的 **13 条**；唯一必须渲染才能判断的是「分页观感」。这正好说明「B 方案为主、C 方案兜底」的组合是合理的。

### 4.4 实现要点（给落地的人）

- **复用而非重写**：`read_docx_text` 已经会解 zip、取 `document.xml`（`tools.rs:1370-1395`）；`inspect_docx` 只需要把同一份 XML 用字符串扫描统计（`extract_open_xml_text`、`parse_xml_tag`，`tools.rs:2059-2128`），**不需要引入新依赖**。
- **输出有界**：`warnings` 最多 8 条、每条 ≤60 字；`stats` 只放计数，不放标题全文（避免工具结果反过来占用上下文）；总长 ≤800 字。
- **与渲染器版本绑定**：探针阈值（如「字体 eastAsia」）是 v2 渲染器的验收口径，放进同一个 PR，避免探针与实现漂移。
- **前端**：AgentActivity 的 tool 详情本来就是 `<pre>` 展示输出（`AgentActivity.vue:313-320`），JSON 可直接读；若要更好体验，可把 `warnings` 单独渲染成列表（可选，P1）。
- **测试**：把 before 产物（3 part、无样式）作为 fixture，断言 `inspect_docx` 至少报出 8 条 warning；gold 产物（task-2）断言 warnings 为空——这就是回归基线。

---

## 5. 内容层实证：桌面 docx 的 5 个具体例子

### 5.1 产物画像（本次重新解包核对）

| 指标 | 实测 | 含义 |
|---|---|---|
| zip part | 3 个：`[Content_Types].xml`、`_rels/.rels`、`word/document.xml` | 无 styles/settings/header/footer/docProps/rels |
| `w:p` | 123（含表格单元格 59 个） | 正文约 64 段 |
| `w:pStyle` / `w:outlineLvl` / `w:numPr` / `w:hyperlink` | 0 / 0 / 0 / 0 | 无样式、无大纲、无真编号、无链接 |
| `w:sectPr` | 1（body 末尾） | 无分节，封面/正文不可分 |
| 表格 | 3 张：6×4、4×5、5×3；`gridCol` 均匀 1805（5 列表）/2256（4 列表） | 列宽均分，与内容长度无关 |
| 标题 | 11 个（6 个 `##` + 5 个 `###`），`w:keepNext/keepLines` 各 6 | 只是「加粗大字」，不是样式 |
| 手写列表前缀 | 12 条 `•`、9 条 `1.`（如 w:p #33-35、#37-39、#95-97） | 假列表 |
| 超长段落 | 5 段 >150 字，最长 264 字（w:p #2） | 字段被合并 |
| 字面量 `-`/`---` | w:p #121 整段就是 `---` | 分隔线未处理 |
| 正文中的 URL | 0（`https?://` 在 `w:t` 中 0 命中，1 处命中是 XML 命名空间） | 来源全部是裸域名/纯文本 |

同时确认：正文里**没有** `**` / `](` / `|` 泄漏（0 命中）——说明这份产物的 Markdown 原文写得是干净的，**损失发生在模型→渲染器这一段，不是模型写错了。**

### 5.2 五个具体例子（原文引用 → 损失 → 对应修复）

**E1：四个字段被合并成 264 字一段（软换行合并）**

- 原文（`w:p` #2，`<w:t>` 拼接原文，节选）：
  > 覆盖周期：2026 年 11 月 1 日 — 11 月 14 日（以 11 月 12—13 日的一周为重点） 成稿时间：2026 年 11 月 14 日 信息性质：本报告以官方公告、官方博客、权威媒体为主要依据；…… 一句话结论：这个月的 AI 行业同时踩了两脚油门——……
- 现在的渲染：`para_lines.join(" ")`（`tools.rs:1849`），四个字段被空格连成一段，**读者要在一大段里找字段**。
- 损失：报告开头的「五个关键信息」丧失了可扫读性；打印后第一屏读感直接降级。这是 BRIEF §3 缺陷 2 的直接证据。
- 修复：v2 markdown 软换行 → `w:br`；或模型改 blocks `[{heading},{paragraph×4}]`；提示词 §3.2 要求「字段独立成段」。探针 PS-11。

**E2：`---` 变成字面量正文（水平分隔线未处理）**

- 原文（`w:p` #121，整段 `<w:t>---</w:t>`）：`---`
- 现在的渲染：没有分隔线分支（`tools.rs:1872-1945`），`---` 走普通段落 `body_paragraph`，打印出来就是三个减号。
- 损失：`核验建议` 与 `报告边界` 之间原本是「分割线」，现在变成一段乱码式文本；公文/报告观感被拉低，且在复制到其他系统时会成为残留字符。
- 修复：v2 markdown 中单独一行 `---` → 「段落下框线 `pBdr`」或「分页/分隔段落」；提示词仍要求优先用标题分段；探针 PS-10。

**E3：来源全是不可点击的裸域名（无超链接）**

- 原文（`w:p` #54-56，节选）：
  > ……AI 基建平台 fal 13 亿美元（来源：beststartup.us，待一手核验）；
  > ……Mercor 7.5 亿美元为最大单笔……（来源：aifundingtracker）；
  > ……记录到 73 条融资公告、披露总额约 224 亿美元（来源：name.ai）。
- 现在的渲染：`w:hyperlink=0`、无 `word/document.xml.rels`、正文 `https?://` 0 命中 —— 这些来源只能是纯文本，点击无效；`read_file` 回读时 URL 也不存在（提取器只读 `w:t`）。
- 损失：内容明明是「带可信度分级的新闻报告」，但**可核验性归零**：读者无法一键验证金额；模型的「来源意识」没有被渲染器兑现（BRIEF §3 缺陷 10）。
- 修复：解析 `[名称](url)` → `w:hyperlink` + external relationship（需要 `document.xml.rels`，当前打包器根本没有这个 part，`tools.rs:1475-1495`）；提示词 §3.2 要求 `[来源名称](https://完整URL)`；探针 PS-05。

**E4：5 列表格等宽，长文本列被挤压**

- 原文（3.4 治理合规表，`w:p` #63-82；引用「关键要求」单元格）：
  > 首次把预置智能体的手机、平板、可穿戴等终端纳入电信设备进网监管，围绕适配验证、进网检测、现网升级、分发管理四环节建全链条治理
  > （同表短列：`时间` =「11 月 6 日前后」，`地区` =「中国」）
- 现在的渲染：`col_w = total_width / cols`（`tools.rs:1724-1725`），5 列各 1805 twips（约 1.25 英寸）；长单元格约 39 个汉字宽，实际要换 6-8 行；而「时间」「地区」列大量留白。另一张 6×4 表里 `#`（序号，1 个字符）同样占 2256 twips。
- 损失：表格高度被拉长（可能跨页），且没有 `w:cantSplit`（`tools.rs:1757-1759` 只有 `shd/vAlign`），跨页时一行可能被劈成两半；阅读时「关键要求」反复换行打断节奏。
- 修复：按内容长度分配列宽（中文按 2 个长度单位）+ `w:tcW` + `cantSplit` + 表头重复保留；`blocks.table.columns[].width` 权重；探针 PS-09。

**E5：假列表（手写 `•` / `1.` 前缀）**

- 原文（`w:p` #33 / #37，节选）：
  > • 这是英伟达成立 33 年来第一次最高权力交接，且发生在毫无预告的时点……
  > 1. 11 月 17 日财报电话会上，临时 CEO 对 2027 年产能与订单可见度的表述；
- 现在的渲染：`list_paragraph` 手写前缀 `"• "` / `"{num}. "` + 悬挂缩进（`tools.rs:1654-1675`），没有 `numbering.xml`、没有 `w:numPr`（产物计数 0）。
- 损失：① 增删/重排条目时序号不会自动更新（模型手写的 `1./2./3.` 是死文本）；② 无法多级编号（「后续观察清单」下面再分层只能靠缩进硬凑）；③ 复制到其他应用/无障碍工具时只是普通段落，不是列表；④ Word 里无法一键调整编号格式（BRIEF §3 缺陷 7）。
- 修复：真 `numbering.xml` + `w:numPr`（多级 `w:ilvl`）；解析器按连续块生成 `numId`；提示词禁止手打 `•`；探针 PS-04。

### 5.3 小结：内容层是干净的，损失发生在「渲染 / 契约」层

- 该产物的**内容质量**：123 段、3 张表、可信度 A/B/C/D 分级、口径提醒、报告边界、观察清单齐全（BRIEF:9 的结论成立）。
- **模型侧没有写错**：无 `**` / `](` / `|` 残留（0 命中）；`---` 是唯一的格式残留（1 处）。
- 所有 5 个例子都能映射到「契约/渲染/提示词」某一层的具体修复项（§2/§3/§6）。**不需要重写内容，也不需要换模型**；需要的是把「文档工程」的表达通道和校验闭环补上。
- 对照 `research/docx-quality/out/source.md`（Lead 产出的高质量 Markdown 源）可以看到目标写法：frontmatter 独立（`source.md:1-6`）、字段一行一个（`source.md:10-16`）、来源写成 `[名称](URL)`（`source.md:50、70-72`）、5 列以内表格带明确列意图（`source.md:84-88`）。**这份源正是 v2 契约应该 1:1 承载的输入形态。**

---

## 6. 优先级表（P0 / P1 / P2）

工作量口径：**S** ≈ 半天以内、**M** ≈ 1-3 天、**L** ≈ 1 周以上（单人，含自测）。

| 优先级 | 编号 | 改造项 | 层级 | 具体做什么 | 工作量 | 风险 | 验收口径 |
|---|---|---|---|---|---|---|---|
| **P0** | P0-1 | `create_docx` 契约 v2（双通道） | 契约 | 按 §2.2 替换 schema；`mode` 状态机；`frontmatter/options`；字段探测兼容 v1 | M | 模型误用新字段、解析分支变多；缓解：v1 探测 + 运行时软校验 + warnings 提示 | v1 调用（path/content/title）结果仍可用；v2 blocks/markdown 各出一份样例；未知字段不报错 |
| **P0** | P0-2 | Markdown 解析器 4 处硬修复 | 渲染 | 软换行→`w:br`；`---`→下框线/分页；标题→`pStyle`+`outlineLvl`；表格列宽按内容加权 + `tcW`+`cantSplit` | M | Word 兼容性回归（错误的 pStyle 引用会触发修复提示）；需 Word COM 打开验证 | 同一份 `source.md` 生成后：`outlineLvl>0`、`hyperlink>0`、无字面量 `---`、最长列宽比≈内容字数比、Word 打开无「修复」提示 |
| **P0** | P0-3 | 文档工程 part 集 + 封面/目录/页码 | 渲染 | styles.xml、numbering.xml、settings.xml（`updateFields`）、header/footer（PAGE/NUMPAGES）、docProps/core、document.xml.rels、双节 sectPr | L | 与 task-1/2 的 XML 规格强耦合；缺 rels/Content_Types 会让 Word 报修复；量大 | Word 页面：封面独立、第 2 节页码从 1、目录域可更新、导航窗格出现 H1-H3、文件属性非空 |
| **P0** | P0-4 | 提示词替换为《文档作者规范》 | 提示词 | 用 §3.2 替换 `lib.rs:2226-2249`；同步改 `tools.rs:989/994` 描述 | S | 提示变长挤占上下文；与「2-3 句话」人设冲突；缓解：作用域声明 + 精简版 | 3 个真实任务抽样：先提纲后写、字段独立成段、无 `---`、来源写成链接、写后读 warnings |
| **P0** | P0-5 | 工具返回值 = 结构摘要 + warnings | 反馈 | §2.3；探针逻辑内嵌 `exec_create_docx`/`finalize` | M | token 增加（≤800 字）、误报；缓解：warning 上限 8 条 + 阈值跟渲染器同 PR | before 产物生成即报 ≥8 条 warning；gold 产物 warnings 为空；v1 返回值仍含「已生成: path（N 字节）」 |
| **P1** | P1-1 | `append` / `finalize` 分块写入 | 契约 | 草稿侧车（`<path>.docx.draft.json`）+ 合并渲染 + 中途失败可续 | M | 侧车文件残留/并发覆盖；缓解：路径绑定 + 时间戳 + finalize 清理 | 8 页报告 = create + 3×append + finalize，顺序/章节完整；中途 kill 后重试可续写 |
| **P1** | P1-2 | 独立 `inspect_docx` 工具 | 反馈 | §4.3 的 13 条静态探针；供模型复查与前端「检查文档」 | M | 与 P0-5 重复；需同步 `tools.ts`/`AgentActivity.vue` 否则工坊静默丢弃 | 对 before 产物输出 ≥8 条 warning；gold 产物 0 条；工坊勾选器里能看到该工具 |
| **P1** | P1-3 | 历史中保留「文档清单」 | 反馈 | 把已写章节的标题+摘要作为 assistant 文本落库（或轻量 doc manifest），让下一轮能续写/局部改 | S/M | 改动 `ChatMessage`/保存路径；缓解：只存 ≤20 行摘要 | 第二轮说「继续写第 4 章」无需用户复述；「把 3.2 节改短」能定位到章节 |
| **P1** | P1-4 | 表头/编号/表格默认值策略 | 渲染 | numbering.xml 多级；表格斑马纹/表头重复/垂直居中；表题 | S | 与 P0-3 同批做，避免重复打包 | 5 项列表增删后序号自动重排；3 张样例表观感一致 |
| **P1** | P1-5 | 文档属性 + 路径/覆盖语义 | 渲染/契约 | `frontmatter` → core.xml；覆盖前 warnings；同名文件提示 | S | 无 | 文件属性里 title/author/created 正确；重复 create 有明确提示 |
| **P2** | P2-1 | 渲染截图闭环（B 方案 C 项） | 反馈 | Word COM → PDF → PNG 进多模态；仅 ≥5 页/对外文档启用；失败静默降级 | L | 环境依赖（Word/权限/焦点/超时）、成本高、判断不稳定；BRIEF:117-118 已说明 LibreOffice 不可用 | 能看出分页断裂/孤行/目录页码；失败时不影响交付 |
| **P2** | P2-2 | 图片 / 脚注 / 交叉引用 | 渲染 | `w:drawing` + media part；脚注 part；书签+REF | M（图）/ M（脚注）/ M（交叉引用） | 媒体打包与 rels 复杂度；兼容性 | 含图/脚注样例 Word 打开正常、导出 PDF 无丢图 |
| **P2** | P2-3 | 局部编辑（`docx_edit`） | 契约 | 按「章节标题/段落锚点」替换或插入，保留用户手工样式 | L | OOXML 定位复杂、破坏样式风险 | 能按章节替换 1 段且其余样式/目录不变 |
| **P2** | P2-4 | 模板/主题体系 | 渲染 | 企业模板（页眉/字体/色彩），`options.template` | L | 模板治理、兼容 | 同一模板生成两篇文档样式一致 |

### 6.1 三个「不要做」

1. **不要只用提示词绕过 `---` / 软换行缺陷。** 提示词能降低触发率，但渲染器不修，任何用户/模型换个写法就复发；提示词是补丁，渲染器才是修复。
2. **不要让模型把 5,000 字以上正文塞进单次 `content`。** 这是 D1/D2 的必然失败点；v1 兼容期也必须靠 prompt §3.2 的分块规则 + P1-1 的 append 兜住。
3. **不要新增工具/字段却不更新前端契约清单。** `tools.ts:1-7` 明确说「不在此清单里的工具会被静默丢弃」，且 `tools.rs:6240-6245` 有前后端比对测试——不同步会直接测试失败或工坊里看不到。

### 6.2 如果只做一天（最小可交付）

按依赖顺序做 4 件事，就能把「内容不差→文档很差」变成「文档至少专业可读」：

1. `tools.rs:1849` 的 `join(" ")` 改为按行生成段落/`w:br`（修 E1）；
2. `tools.rs:1628-1651` 给标题补 `pStyle` + `outlineLvl`（配最小 `styles.xml`，修 BRIEF 缺陷 3/6）；
3. 解析 `---` 与 `[文字](url)`（`pBdr` + `w:hyperlink` + `document.xml.rels`，修 E2/E3）；
4. 换掉 `lib.rs:2226-2249` 的提示词 + `tools.rs:1456-1460` 的返回值（修 D12/D13，模型开始自我约束）。

其余 P0/P1 按表推进。**验收统一用 task-3 的 Word 渲染 + `compat_report.md`，不要靠肉眼猜。**

---

## 附录 A：v2 调用的三个最小示例（供提示词与集成测试共用）

**A1. 一次成文（≤6 页）**

```json
{
  "path": "C:\\Users\\15188\\Desktop\\AI领域新闻报告汇总_2026年11月.docx",
  "mode": "create",
  "frontmatter": {
    "title": "AI 领域新闻报告汇总（2026 年 11 月）",
    "subtitle": "模型、资本与治理的三条主线",
    "author": "AI 桌宠 · 直连 API 生成",
    "date": "2026-11-14"
  },
  "options": { "cover": true, "toc": true, "pageNumbers": true, "headingNumbering": false },
  "markdown": "## 一、报告说明\n\n**覆盖周期：** 2026 年 11 月 1 日 — 11 月 14 日。\n\n**成稿时间：** 2026 年 11 月 14 日。\n\n**一句话结论：** ……\n\n## 二、本期五条主线\n\n| 主线 | 一句话概括 | 可信度 | 关键节点 |\n| --- | --- | --- | --- |\n| 英伟达换帅 | …… | 高：官方 + 多家媒体 | 11 月 12 日 |\n"
}
```

**A2. 长文分章（create → append × N → finalize）**

```json
{ "path": "C:\\...\\长报告.docx", "mode": "create", "frontmatter": { "title": "……" }, "blocks": [ { "type": "heading", "level": 1, "text": "一、背景" }, { "type": "paragraph", "text": "……" } ] }
```

```json
{ "path": "C:\\...\\长报告.docx", "mode": "append", "blocks": [ { "type": "heading", "level": 2, "text": "1.1 ……" }, { "type": "paragraph", "text": "……" } ] }
```

```json
{ "path": "C:\\...\\长报告.docx", "mode": "finalize" }
```

**A3. 精确控制表格列宽**

```json
{
  "path": "C:\\...\\合规表.docx",
  "mode": "create",
  "frontmatter": { "title": "治理合规对照" },
  "blocks": [
    { "type": "heading", "level": 1, "text": "一、比较" },
    { "type": "table",
      "caption": "表 1：三地监管动作对照",
      "columns": [
        { "label": "地区", "width": 2 },
        { "label": "时间", "width": 2 },
        { "label": "动作", "width": 4 },
        { "label": "关键要求", "width": 10 },
        { "label": "影响对象", "width": 4 }
      ],
      "rows": [
        ["中国", "11 月 6 日前后", "工信部通知", "首次把预置智能体的终端纳入进网监管……", "终端厂商、智能体提供方"]
      ] }
  ]
}
```

---

## 附录 B：证据索引（本文引用的关键行号）

| 结论 | 证据 |
|---|---|
| 工具描述承诺「自动转换为排版好的 Word」 | `tools.rs:989` |
| 工具契约只有 path/title/content | `tools.rs:988-999` |
| 40 条工具定义 | `tools.rs:471-1104`（`"type": "function"` ×40） |
| 工具超时 30s | `tools.rs:1125`；`tools.rs:1231-1236` |
| 512KB 上限 / 返回字节数 | `tools.rs:1422-1429`、`tools.rs:1456-1460` |
| 只打 3 个 part | `tools.rs:1464-1499` |
| 软换行合并 `join(" ")` | `tools.rs:1846-1853`、`tools.rs:1849` |
| 标题无 pStyle/outlineLvl | `tools.rs:1628-1651` |
| 假列表 | `tools.rs:1654-1675` |
| 均分列宽 | `tools.rs:1724-1725`；无 tcW `tools.rs:1757-1759` |
| only pgSz/pgMar 的 sectPr | `tools.rs:1951-1952` |
| docx 纯文本回读 | `tools.rs:1370-1395`、`tools.rs:2059-2104` |
| roundtrip 测试只测文本 | `tools.rs:5702-5756` |
| 请求体无 max_tokens | `api_client.rs:1014-1037、1120-1141、1193-1214` |
| 流式重试/截断整轮重来 | `agent.rs:169、171、601-619` |
| 截断参数不执行 | `agent.rs:839-862` |
| 工具结果只回一句 | `agent.rs:1038-1044` + `tools.rs:1456-1460` |
| 历史不含 tool 调用 | `db.rs:103-123`、`agent.rs:1148-1178` |
| 历史预算 120k 字符 | `agent.rs:174、205-219` |
| Word 提示段 20 行 | `lib.rs:2226-2249` |
| 系统提示 14 段拼接 | `lib.rs:4340-4400`、`lib.rs:4395` |
| 基础人设「2-3 句话」 | `client.rs:40、101-108` |
| CLI 路径 `--resume` + 提示词文件 | `client.rs:111-127、374-394` |
| 内置技能无「文档写作」 | `db.rs:16-96`（仅「文档阅读」`db.rs:30-36`） |
| 技能单激活触发器 | `db.rs:425-446` |
| 前端工具清单/标签 | `tools.ts:8-45、49-86、84` |
| 前端只展示 args/output 文本 | `AgentActivity.vue:284-322、313-320` |
| 前端为 CLI 的 update_plan 渲染清单 | `AgentActivity.vue:81、136-158` |
| 产物指标（本次解包） | `w:pStyle=0 / outlineLvl=0 / numPr=0 / hyperlink=0 / sectPr=1 / 3 parts / gridCol 1805 / 264 字长段 / 字面量 --- (w:p #121)` |
