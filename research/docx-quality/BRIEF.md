# 共享简报：直连 API 场景下 Word 文档生成质量研究

> Lead 维护，所有 teammate 先读这份文件，再读源码。**只读源码，不改源码**（本次只做研究与 POC，落地补丁由 Lead 汇总）。

## 1. 研究对象（真实产物）

- 用户桌面上的产物：`C:\Users\15188\Desktop\AI领域新闻报告汇总_2026年11月.docx`（9,407 字节，2026-11-14 00:45 生成）
- 它由桌宠 App「直连 API」路径下的 `create_docx` 工具生成（非 Claude Code / DSH 技能路径）。
- 内容质量本身不差（123 段，含 3 张表，观点与可信度分级齐全），**差的是"Word 排版与文档工程"**。

## 2. 已核实证据（Lead 用 python/zipfile 直接验过，可直接引用）

### 2.1 docx 包结构：只有 3 个 part

```
232 ->    429  [Content_Types].xml
172 ->    297  _rels/.rels
8659 -> 65825  word/document.xml
```

缺失的 part（对比 Word 正常文档）：`word/styles.xml`、`word/numbering.xml`、`word/settings.xml`、
`word/theme/theme1.xml`、`word/fontTable.xml`、`word/webSettings.xml`、`word/header*.xml`、
`word/footer*.xml`、`word/_rels/document.xml.rels`、`docProps/core.xml`、`docProps/app.xml`。

### 2.2 document.xml 关键指标（用 grep 计数）

| 指标 | 计数 | 含义 |
|---|---|---|
| `w:pStyle` | **0** | 没有一个段落引用命名样式 |
| `w:outlineLvl` | **0** | 标题没有大纲级别 → Word 导航窗格空白、无法自动生成目录 |
| `w:numPr` | **0** | 没有真实编号 → 列表是手写 `•` / `1.` 文本 |
| `w:sectPr` | 1（body 末尾） | 页面是 A4（11906×16838），页边距 1440，但下面 2.2 的缺陷仍存在 |
| `w:tblHeader` | 有 | 表头可跨页重复（这一点做对了） |

### 2.3 生成器代码位置

`src-tauri/src/direct_api/tools.rs`（共 6435 行）：

- 工具 schema：`L988-1010` 附近（`create_docx`，`path` + `content` + 可选 `title`）
- 分发：`L1231-1234`（30 秒超时）
- 入口：`exec_create_docx` `L1417-1461`（512KB 上限、自动补 `.docx` 后缀）
- 打包：`build_docx_bytes` `L1464-1499`（手写 zip，只写 3 个 part）
- 行内解析：`split_paired_marker` / `parse_inline_runs` `L1510-1560`
- XML 片段生成器：`run_xml` `L1571`、`body_paragraph` `L1601`、`doc_title_paragraph` `L1612`、
  `heading_paragraph` `L1628`、`list_paragraph` `L1654`、`quote_paragraph` `L1678`、
  `code_block_paragraph` `L1695`、`table_xml` `L1714`
- 主解析循环：`markdown_to_document_xml` `L1835-1956`（末尾拼 `sectPr`，`L1951`）
- 测试：`docx_generation_roundtrip` `L5702`；`dump_docx_for_external_validation` `L5770`（`#[ignore]`）

## 3. 已确认的真实排版缺陷（不是猜测）

1. **分隔线 `---` 被当普通文本输出**：产物最后一段正文是字面 `---`（见 document.xml 尾部
   `<w:t>---</w:t>`）。解析循环没有处理水平分隔线。
2. **Markdown 软换行被合并**：`markdown_to_document_xml` 里 `para_lines.join(" ")`（`L1849`），
   把连续多行合并成一个段落。产物第 3 段把「覆盖周期 / 成稿时间 / 信息性质 / 一句话结论」四项
   挤成了一段，可读性直接崩塌。模型只要写成「每行一个字段」，就会被合成一段。
3. **没有任何命名样式**：标题只是"字号加粗"，`heading_paragraph` 只设 `size`，不设
   `w:pStyle`/`w:outlineLvl`。后果：导航窗格空、无法插入目录、无法按样式统一改版、
   无法做「标题编号」、Word 的"样式"面板里全是默认。
4. **无页眉页脚 → 无页码**：`sectPr` 里没有 `w:headerReference`/`w:footerReference`，
   整本报告没有页码、没有页眉文档名。多页报告没有页码是很明显的"业余感"。
5. **无封面页**：标题就是正文第一段居中大字，不是独立封面；也没有分节符。
6. **无目录**：没有 `TOC` 域，长报告无法定位。
7. **列表是"假列表"**：`list_paragraph`（`L1654`）手写 `• ` / `1. ` 前缀 + 悬挂缩进，
   不是 `numbering.xml` 真编号。后果：增删条目不会自动重排、无法多级编号、无法调整编号格式。
8. **表格列宽均分**：`table_xml`（`L1714`）`col_w = total_width / cols`，5 列表格（3.4 节
   治理合规表）里"关键要求"这种长文本列和一列"地区"等宽，中文被迫挤压换行。
9. **表格无行级控制**：没有 `w:cantSplit`，跨页时一行可能被劈成两半；没有 `w:tcW`
   （只有 `gridCol`），Word 里拖动列宽行为不稳定。
10. **无超链接**：`[文字](url)` 不解析，新闻报告里的来源全是纯文本，无法点击。
11. **正文无首行缩进**：中文公文/报告惯例是首行缩进 2 字符，现在只有段间距。
12. **无文档属性**：没有 `docProps/core.xml`，文件属性里标题/作者/日期全空，`title` 参数
    只变成了第一段文字，没写进属性。
13. **无孤行/段中抑制控制**：标题没有 `keepNext` 以外的孤行控制（部分有），正文无 `widowControl`。
14. **字体内外混搭**：`w:ascii="Calibri"` + `w:eastAsia="微软雅黑"`。中文报告更常见的组合是
    中文标题黑体/正文宋体或全微软雅黑；且没有 `w:cs`、没有 `hint`。

## 4. "高质量 Word" 的验收定义（本次研究的目标态）

按专业报告/白皮书标准，拆成 4 组能力：

**A. 包完整性（文档工程）**
- 完整 part 集：styles/numbering/settings/theme/fontTable/webSettings/header/footer/docProps/rels
- `docProps/core.xml`（title/subject/creator/lastModifiedBy/created/modified）
- `settings.xml`：`<w:updateFields w:val="true"/>`（让 Word 打开时提示更新目录/页码域）
- 中文字距/标点：`w:compat`、`w:kinsoku`、`w:overflowPunct`、`w:doNotExpandShiftReturn`

**B. 排版系统（样式驱动）**
- 命名样式：Title / Subtitle / Heading1-3（带 `w:outlineLvl`）/ BodyText / TableHeader /
  Caption / Quote / CodeBlock / ListParagraph
- 正文：首行缩进 2 字符、行距 1.5、段后间距、`w:widowControl`、中文 `w:eastAsia` 字体
- 标题：keepNext + keepLines + 段前段后 + 大纲级别 + 编号可选
- 页面：A4、页边距（上下 2.54cm / 左右 3.17cm 或 2.5cm）、页眉页脚距

**C. 文档结构（长文档特征）**
- 封面节（首页不同，无页眉页脚）+ 正文节（页码从 1 开始）
- 目录域 `TOC \o "1-3" \h \z \u` + 一处 `w:updateFields`
- 页脚：`第 X 页 / 共 Y 页`（`PAGE` / `NUMPAGES` 域）
- 页眉：文档名（可选 + 分隔线）
- 分节符 `w:sectPr`（封面/正文）、`w:type=nextPage`

**D. 内容元素（渲染质量）**
- 真编号列表（numbering.xml，多级 `w:ilvl` + `w:numId`）
- 表格：按内容分配列宽 + `tcW` + `cantSplit` + 表头重复 + 斑马纹 + 垂直居中 + 单元格边距
- 超链接（`w:hyperlink` + `r:id` + external relationship + `w:color` + 下划线）
- 水平分隔线（`---`）→ 段落 `pBdr` 下框线，或 `w:type="page"` 分页
- 软换行 → 段落内 `w:br`（而不是拼成一个段落）
- 图片（`w:drawing` + media，P2，可选）
- 交叉引用/书签（P2）

## 5. 环境事实（很重要，别踩坑）

| 资源 | 状态 |
|---|---|
| system `python` | 3.12.6，有 `python-docx 1.2.0`、`PyMuPDF(fitz)`、`Pillow` |
| bundled python | `C:\Users\15188\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe`（有 python-docx/lxml/Pillow，**无 fitz**） |
| LibreOffice / `soffice` | **不可用**（`C:\Program Files\LibreOffice` 是残缺安装，只有 `program/logs`）；DSH office sidecar (`dsoffice`) 未安装 |
| Microsoft Word | **已安装**：`C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE`（可用 COM 导出 PDF） |
| cargo | 1.95.0，但 `src-tauri/target/release/.cargo-lock` 写入被拒（沙箱/ACL 问题）→ **本次不要求编译 Rust** |

## 6. 硬约束

- **不要修改** `src/`、`src-tauri/` 里的任何源码；**不要跑 `cargo`**（会踩 ACL 坑 + 与 Lead 冲突）。
- 只在各自 write scope 内写文件；不要写用户桌面。
- 所有结论必须带证据：`文件:行号` 或命令输出；不要写"可能/大概"当结论。
- 输出中文（报告用简体中文，专业、克制、不堆形容词）。
- 不要下载/安装新软件包（无 sidecar 授权）；渲染用本机 Word COM。
- 不要用 `du` 这类无界命令（上一轮把输出打爆了）。

## 7. 目录约定

```
research/docx-quality/
  BRIEF.md          # 本文件（Lead）
  audit.md          # A：OOXML/排版缺陷深度审计
  authoring.md      # D：工具契约 + 提示词 + 内容层
  poc/              # B：参考生成器（raw OOXML，Python）
  render/           # C：渲染与兼容性验证脚本 + 产物
  out/              # 共享产物：source.md / gold.docx / before.pdf / 报告.docx
```

## 8. 写作口径（最终报告会用）

研究结论要能回答三个问题：
1. **现在差在哪**（可量化、可复现：包缺几个 part、缺哪几个样式、Word 里具体表现是什么）
2. **为什么直连 API 特别容易差**（没有技能/工具链、单次 tool call 一次性吐全文、无渲染反馈闭环、
   Markdown 是"最低公共分母"格式、模型会把排版决策推给渲染器而渲染器是手写的）
3. **怎么改**（分 P0/P1/P2，给出可直接落地的 XML/Rust 改动点、契约变更、验收标准）
