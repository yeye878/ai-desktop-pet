# B · 高质量 DOCX 参考生成器（raw OOXML POC）

> 研究项：直连 API 场景下 Word 文档生成质量 · 目标态参考实现
> 对应任务：task-2（B）；数据源：`../BRIEF.md`；真实输入：`../out/source.md`（Lead 提供）

## 1. 定位

本目录是「目标质量」参考生成器：只用 Python 标准库 `zipfile` + 手写 OOXML 字符串，把 Markdown 渲染成专业报告级 `.docx`。**生成链路刻意零 python-docx 依赖**，目的是让每个 part 的 XML 字符串与 Rust `src-tauri/src/direct_api/tools.rs` 的 `build_docx_bytes` / `markdown_to_document_xml` 一一对应，结论可 1:1 移植回 Rust；python-docx 只出现在外部验收脚本 `verify_docx.py` 中。

与现状（BRIEF 2.1）对比：原产物只有 3 个 part、0 个 pStyle/outlineLvl/numPr；参考产物有 **14 个 part**，pStyle/outlineLvl/numPr 全部 > 0。

## 2. 交付文件

| 文件 | 作用 |
|---|---|
| [gold_docx.py](gold_docx.py) | 生成器（1400 行）：CLI + Markdown 解析 + 14 个 part 的 OOXML 字符串 + zip 打包 |
| [feature_showcase.md](feature_showcase.md) | 特性样张（87 行），覆盖全部 P0/P1 特性 |
| [verify_docx.py](verify_docx.py) | 验收脚本（474 行）：zipfile + lxml + python-docx 三重校验，34 项断言 |
| [../out/gold_features.docx](../out/gold_features.docx) | 用 `feature_showcase.md` 生成的自检产物（11,550 bytes） |
| [../out/gold.docx](../out/gold.docx) | 用 `source.md` 生成的真实内容产物（16,191 bytes，估算 6 页） |

## 3. 用法

```bash
PY="C:/Users/15188/.dsh/dsh-runtimes/dsh-primary-runtime/dependencies/python/python.exe"

# 特性样张 → 自检产物
$PY research/docx-quality/poc/gold_docx.py \
  --src research/docx-quality/poc/feature_showcase.md \
  --out research/docx-quality/out/gold_features.docx \
  --title "高质量 DOCX 渲染特性样例" --author "参考实现工程师"

# 真实新闻源 → gold.docx
$PY research/docx-quality/poc/gold_docx.py \
  --src research/docx-quality/out/source.md \
  --out research/docx-quality/out/gold.docx \
  --title "AI 领域新闻报告汇总（2026 年 11 月）" --author "AI 桌宠 · 直连 API 生成"

# 验收（任一 docx）
$PY research/docx-quality/poc/verify_docx.py research/docx-quality/out/gold.docx
```

| 参数 | 必填 | 说明 |
|---|---|---|
| `--src` | 是 | 输入 Markdown 路径（UTF-8 优先，GB18030 兜底） |
| `--out` | 是 | 输出 `.docx` 路径 |
| `--title` | 否 | 文档标题；缺省取 front matter `title` 或正文首个 H1 |
| `--author` | 否 | 作者；缺省取 front matter `author` |
| `--subtitle` / `--date` / `--org` / `--subject` | 否 | 封面副标题 / 日期 / 出品方 / core.xml 主题 |
| `--no-toc` | 否 | 不插入目录页 |

front matter 约定（文件最前面的 `---` 块，只接受 `key: value`，否则整段按正文处理）：
`title` / `subtitle` / `author` / `date` / `subject` / `keywords` / `org`。

Markdown 语法约定：`#`~`######` 标题；`-`/`*`/`+` 与 `1.`/`1)` 列表（2 空格一级缩进）；`> ` 引用；三反引号代码块；`|…|` 表格；`**粗体**` / `*斜体*` / `` `等宽` ``；`[文字](http(s)://url)` 与 `<https://url>` 外链；`---` / `***` / `___` 独立行 → 水平分隔线；`图 1:` / `表 2：` 开头的行 → Caption 样式。

## 4. 已覆盖特性（对照 BRIEF §4）

### 4.1 A 包完整性（14 个 part，原产物 3 个）

| part | 内容要点 |
|---|---|
| `[Content_Types].xml` | Default rels/xml + **11 个 Override**（原文件只有 `word/document.xml` 一条） |
| `_rels/.rels` | officeDocument + core-properties + extended-properties 三条关系 |
| `word/document.xml` | 封面节 + 正文节；TOC 域；页眉页脚引用；pStyle/numPr/hyperlink |
| `word/styles.xml` | docDefaults + 23 个命名样式（含 outlineLvl / 字体 / 缩进 / 行距 / 边框） |
| `word/numbering.xml` | 2 个 abstractNum（bullet / decimal，各 4 级）+ 每个列表一个 `w:num` 实例 |
| `word/settings.xml` | **`w:updateFields w:val="true"`** + compat + themeFontLang + characterSpacingControl |
| `word/theme/theme1.xml` | 完整 clrScheme / fontScheme / fmtScheme（最小合法结构） |
| `word/fontTable.xml` | Times New Roman/Arial/Calibri/Consolas/宋体/黑体/微软雅黑（含 altName） |
| `word/webSettings.xml` | optimizeForBrowser + allowPNG |
| `word/header1.xml` | 页眉文档名（Header 样式：右对齐 + 下框线） |
| `word/footer1.xml` | 「第 X 页 / 共 Y 页」= `PAGE` + `NUMPAGES` 域 |
| `word/_rels/document.xml.rels` | 固定 rId1-8（styles/numbering/settings/theme/fontTable/webSettings/header/footer）+ 超链接 rId9 起（同 URL 去重） |
| `docProps/core.xml` | title / subject / creator / lastModifiedBy / created / modified（W3CDTF） |
| `docProps/app.xml` | Application / Pages / Words / Characters / Lines / Paragraphs / Company |

### 4.2 B 排版系统（样式驱动）

`document.xml` 只写 `pStyle` / `numPr` / 关系 id，外观全部集中在 `styles.xml`：

| 样式 | 关键属性 |
|---|---|
| Title | 微软雅黑 26pt 粗体、深蓝 1F3864、居中、outlineLvl=9 |
| Subtitle | 微软雅黑 14pt、灰色 595959、居中、outlineLvl=9 |
| Heading1 / Heading2 / Heading3 | 黑体 + Arial、16/14/12pt、**outlineLvl = 0/1/2**、keepNext + keepLines、teal/blue/dark 三级色 |
| BodyText | 宋体 + Times New Roman 12pt、**首行缩进 `firstLineChars="200"` + `w:firstLine="480"`**、**1.5 倍行距 `line=360 lineRule=auto`**、`jc=both`、widowControl、outlineLvl=9 |
| Caption | 黑体 10.5pt 粗体、居中、keepNext（题注与表格同页） |
| Quote | 左竖线 pBdr + 浅蓝底纹 + 11pt 灰字、outlineLvl=9 |
| CodeBlock | Consolas 10pt + 四边细框 + 灰底 + 悬挂缩进 |
| ListParagraph | 无首行缩进、段后 60、行距 320，编号由 numPr 提供 |
| TableHeader / TableText | 表头黑体加粗居中、正文 10.5pt 左对齐 |
| TOCHeading / TOC1-3 | 目录标题与三级目录项样式（Word 更新域后直接套用） |
| Divider / Header / Footer / CoverMeta | 分隔线 / 页眉 / 页脚 / 封面信息行 |
| Hyperlink（字符样式） | 蓝色 0563C1 + 下划线 |

`styles.xml` 中 `w:outlineLvl` 共 **20 个**：Heading1/2/3 = 0/1/2；其余命名样式显式 9（body text level），避免被 TOC 收录。

### 4.3 C 长文档结构

- **封面节**：封面内容末尾的段落内 `w:sectPr`（`w:type=nextPage` + `w:titlePg`），且**不引用任何 header/footer part** → 封面完全无页眉页脚；封面结构为 spacer → Title → Subtitle → Divider → CoverMeta（作者/日期/出品）。
- **正文节**：body 末尾 `w:sectPr` 引用 `rId7`(header)/`rId8`(footer)，`w:pgNumType w:start="1"` → 正文页码从 1 开始；页脚 `第 PAGE 页 / 共 NUMPAGES 页`，页眉文档名。
- **TOC 域**：` TOC \o "1-3" \h \z \u `，位于封面后的第一页；`--no-toc` 可关闭。
- 页面：A4 11906×16838、页边距上下/左右 1440、页眉 851、页脚 992、`w:docGrid linePitch=312`。

### 4.4 D 内容元素

| 特性 | 实现 |
|---|---|
| 真编号列表 | `numbering.xml` + `w:numPr(ilvl,numId)`；bullet/decimal 各 4 级；**每个列表实例独立 numId**，ordered 用 `w:startOverride` 从 1（或 Markdown 里的起始数字）重新编号 |
| 表格列宽按内容分配 | 中文宽字符 2 单位 / 西文 1 单位；MIN 620 保底 + 其余按权重等比；总宽精确 **9026 twips**；每格 `w:tcW`；`w:cantSplit` 全行；首行 `w:tblHeader` 跨页重复；数据行斑马纹 `F4F6FA`；`w:vAlign=center`；`tblLayout=fixed` + cellMar |
| 超链接 | `w:hyperlink r:id` + `<Relationship TargetMode="External">` + Hyperlink 字符样式（蓝色 + 下划线）；同 URL 复用同一 rId |
| `---` 水平分隔线 | 独立行 → Divider 段落 + **直接 `w:pBdr/w:bottom` 下框线**（不是字面量 `---`） |
| 软换行 | 同段落的连续 Markdown 行 → 行间 `<w:br/>`，不合并成一个句子 |
| 文档属性 | core.xml（title/creator/created…）+ app.xml（Pages/Words/…）+ header 文档名来自 title |

## 5. 关键实现细节（移植时照抄）

1. **样式驱动而非直接格式**：`_p(style_id, runs)` 只输出 `<w:pStyle>`；所有字体/字号/缩进/边框在 `STYLES_XML_BODY` 中一次定义。原 Rust 的 `run_xml(size)` 直接写 `<w:rFonts>/<w:sz>`，移植后 run 只保留 b/i/等宽/超链接等增量属性。
2. **表格列宽算法**（`allocate_table_widths`，gold_docx.py L891）：

   ```
   cols = max(每行单元格数)
   unit_i = max(4, 该列最长内容显示宽度 + 1)   # CJK=2, 其他=1
   min_width = min(620, 9026 // cols)
   width_i = min_width + (9026 - min_width*cols) * unit_i / Σunit
   最大余数法取整，保证 Σwidth_i == 9026
   ```

   实测 `gold.docx` 三张表列宽（均为 9026）：4 列 `[1814, 3893, 2080, 1239]`、5 列 `[894, 963, 2039, 3480, 1650]`、3 列 `[1204, 3433, 4389]`——长中文列明显更宽，不再是原实现的均分。
3. **编号实例登记**：`_list` 每遇到一个列表块就 `number_instances.append((kind, start))`，渲染结束时 `numbering_part()` 按顺序生成 `<w:num w:numId=N>`；ordered 额外写 `<w:lvlOverride><w:startOverride>`。这解决原实现「假列表」的两个问题：增删自动重排 + 多列表独立编号。
4. **节结构**（`_cover_section_break` / `_final_section_props`）：封面节不引用 header/footer 且带 `titlePg`（首页不同）；正文节 `headerReference`/`footerReference` + `pgNumType start=1`。单节无法同时做到「封面无页脚 + 正文从 1 计页」，所以必须双节。
5. **关系 id 约定**（`document_rels`）：rId1-6 基础设施、rId7 页眉、rId8 页脚、rId9+ 超链接（`_rid_for_href` 按 URL 去重）。移植到 Rust 时建议同样用 `HashMap<String,String>` 维持 target→rid 映射。
6. **标题层级归一**（`rebase_heading_levels` + `_drop_duplicate_title`）：front matter 已给文档标题、正文用 `##`/`###` 表示一/二级章节时，正文最小标题级别上移为 H1（`##`→Heading1、`###`→Heading2）；若正文首个 H1 与封面标题同名则删除，保证标题只出现一次。
7. **中文标点/字距**：`kinsoku` / `overflowPunct` / `autoSpaceDE` / `autoSpaceDN` 写在 `styles.xml` 的 `docDefaults/pPrDefault/pPr`；`settings.xml` 提供 `characterSpacingControl=compressPunctuation`、`compat compatibilityMode=15`、`themeFontLang zh-CN`。
8. **TOC 占位设计（有意为之）**：静态 OOXML 无法预知真实标题页码，因此目录用真 `TOC` 域，域结果（separate→end 之间）放灰色提示文本「（目录将在 Word 打开时自动更新；若未更新，请右键选择“更新域”。）」，并设 `updateFields=true`，Word 打开时更新域后即替换为真实目录；未更新时用户也知道如何操作。这不是缺省内容，是手写 OOXML 的必然设计。

## 6. 与 Rust 移植的对应关系

> 目标文件：`src-tauri/src/direct_api/tools.rs`（下称 tools.rs）。Python 侧行号为 `gold_docx.py` 当前版本。

| Python（gold_docx.py） | tools.rs 现状 | 移植动作 |
|---|---|---|
| `write_package` L387 | `build_docx_bytes` L1464-1499 | 保留 `zip::ZipWriter` + Deflate；写入列表从 3 个扩到 14 个 part（`[Content_Types].xml` 仍第一个写） |
| `package_content_types` L126 | `DOCX_CONTENT_TYPES_XML` L1400-1407 | 保留两个 Default，新增 11 个 `<Override>`（styles/numbering/settings/theme/fontTable/webSettings/header/footer/theme/docProps×2） |
| `package_rels` L154 | `DOCX_RELS_XML` L1409-1415 | 新增 core-properties、extended-properties 两条关系 |
| `document_rels` L166 | **不存在**（原 docx 无 document.xml.rels） | 新增 `word/_rels/document.xml.rels`：rId1-8 固定 + 超链接动态分配 |
| `styles_part` L879 | **不存在** | 新增 `word/styles.xml`（P0 核心，约 11KB XML 常量；`STYLES_XML_BODY` 可直接转成 Rust `concat!`/`format!`） |
| `numbering_part` L437、`_bullet_level` L406、`_ordered_level` L421 | **不存在**；列表靠 `list_paragraph` L1654 手写前缀 | 新增 `word/numbering.xml`；`list_paragraph` 改为只写 `pStyle=ListParagraph` + `numPr(ilvl,numId)` |
| `document_settings` L247 | **不存在** | 新增 `settings.xml`（`updateFields=true` 是关键） |
| `theme_part` L265 / `font_table_part` L317 / `web_settings_part` L342 | **不存在** | 新增三个 part，可选但建议补齐（Word 正常文档都有） |
| `header_part` L352 / `footer_part` L363 | **不存在**（sectPr L1951 无 header/footerReference） | 新增 header1.xml / footer1.xml；页脚写 `PAGE`/`NUMPAGES` 复合域，sectPr 补 `headerReference`/`footerReference` |
| `core_properties` L194 / `extended_properties` L216 | **不存在**；`doc_title_paragraph` L1612 把 title 变成正文第一段 | 新增 docProps/core.xml + app.xml；title 改走封面 Title 样式 + core.xml（`create_docx` 的 title 参数就此真正写进属性） |
| `_cover`（DocxRenderer）/ `_cover_section_break` / `_toc` / `_final_section_props` | `markdown_to_document_xml` L1835-1956（末尾单 sectPr L1951） | 解析循环外包一层「封面 → 封面节 sectPr → TOC 域页 → 内容 → 正文节 sectPr」；`sectPr` 由单节改双节 |
| `parse_markdown` L626 | `markdown_to_document_xml` L1835-1956 | 保留 `for line` 状态机骨架；改动：① front matter 预扫描；② 列表块归并（不再逐行 `list_paragraph`）；③ 软换行保留（删除 `para_lines.join(" ")` L1849，改多行 `w:br`）；④ 新增 `---`→Divider 分支；⑤ 标题层级归一；⑥ 节结构输出 |
| `split_paired_marker` L518 | `split_paired_marker` L1510-1525 | **原样照搬**（语义已对齐，含奇数标记容忍） |
| `_styled_tokens` L540 / `inline_tokens` L555 | `parse_inline_runs` L1528-1560 | 在 `InlineRun` 后追加链接切分（新增 `href: Option<String>` 字段）；顺序保持 `` ` ``→`**`→`*`；新增合法 URL 判断 |
| `_run_xml`（DocxRenderer） | `run_xml` L1571-1598 | 删除直接 `rFonts`/`sz`（交给样式）；新增 `<w:hyperlink>` 包裹 + `rStyle=Hyperlink` + 蓝字/下划线 |
| `_body` / `_line_runs` | `body_paragraph` L1601-1609 | 改 `pStyle=BodyText`，删除直接 spacing；多行用 `w:br` |
| `_heading` | `heading_paragraph` L1628-1651 | 改 `pStyle=Heading1/2/3`；删除直接 size/keep（移入样式，outlineLvl 由样式提供） |
| `_list` | `list_paragraph` L1654-1675 | 删除 `• `/`n. ` 前缀与 `ind left/hanging`；改 `numPr`；列表实例登记 |
| `_quote` | `quote_paragraph` L1678-1692 | 外观（pBdr/shd/color/italic/ind）移入 Quote 样式，段落只留 `pStyle` |
| `_code` | `code_block_paragraph` L1695-1711 | 外观移入 CodeBlock 样式；保留逐行 `w:br` + 前导空格 |
| `_table` + `allocate_table_widths` L891 | `table_xml` L1714-1773 | 关键替换 `col_w = total/cols`（L1725）→ 内容权重分配；新增 `tcW`、`cantSplit`、斑马纹 `shd`、`vAlign=center`；保留 `tblHeader` |
| `_divider` | 无（解析循环无 `---` 分支，BRIEF 缺陷 1） | 新增：独立 `---` 行 → `<w:p>` + `pStyle=Divider` + `w:pBdr/w:bottom` |
| `split_table_row` L612 / `is_table_separator` L607 | L1775-1784 | **原样照搬** |
| `_strip_trailing_hashes` L622 / `parse_heading`（内联于 parse_markdown） | `parse_heading` L1786-1798 | 复用；标题文本去尾部 `#`，层级 clamp 到 3 |
| `rebase_heading_levels` L826 | **不存在** | 新增：正文最小标题级别上移为 1 |
| `extract_front_matter` L582 | **不存在**（工具只有 content/title） | 新增；同时建议工具 schema 增加 `author`/`subtitle`/`date` 可选参数（见 BRIEF D 份 authoring 审计） |
| `build_docx` L1273 / `main` L1352 | `exec_create_docx` L1417-1461 | 入口保留 512KB/后缀纠正；新增元数据解析与组装顺序：`markdown_to_document_xml` 之前先抽 front matter，之后按 part 表打包 |
| `content_stats` L934 | **不存在** | 新增 app.xml 统计（可选，不阻塞渲染） |
| `verify_docx.py`（34 项断言） | 测试 `docx_generation_roundtrip` L5702、`dump_docx_for_external_validation` L5771（`#[ignore]`） | Rust 侧单测可对齐同一组断言：part 存在性、彩色样式计数、表宽和 9026、`---` 无字面量、双节引用等；建议把 dump 测试保留做 Word/python-docx 外部验收 |

**移植工作量粗估**：P0 = package/rels（约 100 行）+ styles.xml（约 11KB XML 常量）+ numbering.xml（约 80 行）+ settings + header/footer + core/app（约 150 行）+ 双节与 TOC 域（约 120 行）≈ 600-700 行；P1 = 表格列宽（约 80 行）+ 超链接关系表（约 60 行）+ `---`/软换行/标题归一（约 80 行）≈ 200 行。原 1400 行里可直接复用的解析/转义/关系解析代码约 300 行。

## 7. 验收（命令与实测输出）

### 7.1 生成（自检产物）

```bash
$PY research/docx-quality/poc/gold_docx.py --src research/docx-quality/poc/feature_showcase.md --out research/docx-quality/out/gold_features.docx --title "高质量 DOCX 渲染特性样例" --author "参考实现工程师"
```

```text
已生成：research/docx-quality/out/gold_features.docx  (11.3 KB, 14 个 part)
标题：高质量 DOCX 渲染特性样例
副标题：从 Markdown 到 Word 的专业报告排版覆盖清单
作者：参考实现工程师    日期：2026 年 11 月 14 日
内容统计：段落 83 / 标题 12 / 表格 2 / 列表 3 / 超链接 4 / pStyle 78 / numPr 10 / 软换行 3 / 分隔线 2
估算：约 4 页 / 989 词 / 1608 字符
```

### 7.2 校验 gold_features.docx

```bash
$PY research/docx-quality/poc/verify_docx.py research/docx-quality/out/gold_features.docx
```

```text
=== docx 包校验：research/docx-quality/out/gold_features.docx ===
[zip] 条目 14 个，原始大小 50415 bytes
[part] 必需 part 14/14：
  [Content_Types].xml                  1699 bytes
  _rels/.rels                           588 bytes
  word/document.xml                   25681 bytes
  word/styles.xml                     11038 bytes
  word/numbering.xml                   2972 bytes
  word/settings.xml                     470 bytes
  word/theme/theme1.xml                2081 bytes
  word/fontTable.xml                   1003 bytes
  word/webSettings.xml                  193 bytes
  word/header1.xml                      347 bytes
  word/footer1.xml                      903 bytes
  word/_rels/document.xml.rels         1860 bytes
  docProps/core.xml                     850 bytes
  docProps/app.xml                      730 bytes
[xml] 14/14 个 XML/rels part 解析成功
[ct] Default ['rels', 'xml']；Override 11 个；未覆盖 XML part 0 个
[rels] 包/文档关系内部 11 条、外部 4 条，坏链 0 条
[stats] pStyle=78  outlineLvl(styles)=20  numPr=10  hyperlink=4  br(软换行)=3  pageBreak=1  pBdr(文档)=1  pBdr(样式)=4  tcW=37  cantSplit=9  tblHeader=2  vAlign=37  headerReference=1  footerReference=1  TOC field=1  PAGE field=1  NUMPAGES field=1
[table] 2 张表，gridCol/tcW 合计均为 9026 twips，表头重复 + cantSplit + vAlign 已覆盖
[hr] Divider 段落 2 个；独立字面量 --- 残留 0 处
[python-docx 1.2.0] 重新打开成功：paragraphs=46, tables=2, sections=2, styles=23, header='高质量 DOCX 渲染特性样例', footer='第 1 页 / 共 1 页'
[python-docx] 段落样式分布（前 6）：List Paragraph:10, Heading 1:7, Body Text:7, Normal:5, Heading 2:5, Divider:2

结果：PASS（34/34 项检查全部通过）
[error] 0 个
```

### 7.3 生成真实内容

```bash
$PY research/docx-quality/poc/gold_docx.py --src research/docx-quality/out/source.md --out research/docx-quality/out/gold.docx --title "AI 领域新闻报告汇总（2026 年 11 月）" --author "AI 桌宠 · 直连 API 生成"
```

```text
已生成：research/docx-quality/out/gold.docx  (15.8 KB, 14 个 part)
标题：AI 领域新闻报告汇总（2026 年 11 月）
副标题：模型、资本与治理的三条主线
作者：AI 桌宠 · 直连 API 生成    日期：2026 年 11 月 14 日
内容统计：段落 135 / 标题 11 / 表格 3 / 列表 10 / 超链接 4 / pStyle 129 / numPr 32 / 软换行 0 / 分隔线 2
估算：约 6 页 / 3208 词 / 4597 字符
```

### 7.4 校验 gold.docx

```bash
$PY research/docx-quality/poc/verify_docx.py research/docx-quality/out/gold.docx
```

```text
=== docx 包校验：research/docx-quality/out/gold.docx ===
[zip] 条目 14 个，原始大小 66557 bytes
[part] 必需 part 14/14：
  [Content_Types].xml                  1699 bytes
  _rels/.rels                           588 bytes
  word/document.xml                   41256 bytes
  word/styles.xml                     11038 bytes
  word/numbering.xml                   3498 bytes
  word/settings.xml                     470 bytes
  word/theme/theme1.xml                2081 bytes
  word/fontTable.xml                   1003 bytes
  word/webSettings.xml                  193 bytes
  word/header1.xml                      362 bytes
  word/footer1.xml                      903 bytes
  word/_rels/document.xml.rels         1860 bytes
  docProps/core.xml                     873 bytes
  docProps/app.xml                      733 bytes
[xml] 14/14 个 XML/rels part 解析成功
[ct] Default ['rels', 'xml']；Override 11 个；未覆盖 XML part 0 个
[rels] 包/文档关系内部 11 条、外部 4 条，坏链 0 条
[stats] pStyle=129  outlineLvl(styles)=20  numPr=32  hyperlink=4  br(软换行)=0  pageBreak=1  pBdr(文档)=1  pBdr(样式)=4  tcW=59  cantSplit=15  tblHeader=3  vAlign=59  headerReference=1  footerReference=1  TOC field=1  PAGE field=1  NUMPAGES field=1
[table] 3 张表，gridCol/tcW 合计均为 9026 twips，表头重复 + cantSplit + vAlign 已覆盖
[hr] Divider 段落 2 个；独立字面量 --- 残留 0 处
[python-docx 1.2.0] 重新打开成功：paragraphs=76, tables=3, sections=2, styles=23, header='AI 领域新闻报告汇总（2026 年 11 月）', footer='第 1 页 / 共 1 页'
[python-docx] 段落样式分布（前 6）：List Paragraph:32, Body Text:18, Normal:6, Heading 1:6, Heading 2:5, Divider:2

结果：PASS（34/34 项检查全部通过）
[error] 0 个
```

### 7.5 gold.docx 专项自检（含 Lead 要求的 `---` 判定）

lxml 直读 `word/document.xml`：

```text
pStyle=129（明细：TableText 47 / ListParagraph 32 / BodyText 18 / TableHeader 12 /
         Heading1 6 / Heading2 5 / Divider 2 / CoverMeta 2 / Title 1 / Subtitle 1 /
         TOCHeading 1 / TOC1 1）
outlineLvl(styles)=20（Heading1/2/3 = 0/1/2；BodyText 等 9=body text level）
numPr=32 个元素（每个列表项一个；document.xml 中字符串 "numPr" 出现 64 次 = 32 开标签 + 32 闭标签）
w:hyperlink=4（rId9→blog.google / rId10→anthropic.com / rId11→name.ai / rId12→beststartup.us）
sectPr=2（封面节：无 header/footer 引用 + titlePg；正文节：headerReference+footerReference + pgNumType start=1）
```

- **`---` 是否已变成分隔线**：是。独立成段 `---` 渲染为 Divider 段落（pStyle=Divider + **直接 `w:pBdr/w:bottom` 下框线**，lxml 检查该段 `pBdr/bottom` 存在 = True）；`---` 前的正文（「核验建议」）与之后「报告边界」段落未被误判为标题。
- **是否有字面量 `---` 残留**：0。`document.xml` 中「独立成段且文本为 `---`/`***`/`___`」的段落 0 个；任意 `<w:t>` 文本含 `---` 的节点 0 个（gold.docx 全文已无 `---` 字面量）。
- **页数（粗略）**：`app.xml` 计 6 页（封面 1 + 目录 1 + 正文约 4；按 1300 非空白字符/页估算）。精确页数需 Word 渲染 PDF 后确认，由 render-verifier 负责。python-docx 读到的页脚 `第 1 页 / 共 1 页` 是域占位值，Word 打开更新域后才会显示真实页码。
- **表格校正**：`source.md` 实际只有 **3 张表**（4/5/3 列：主线表、治理合规表、来源分级表；pipe 行共 18 = 3 组表头/分隔/数据行），不是 Lead 消息里说的 4 张（第二个 5 列表不存在于源文件）；gold.docx 按实际源渲染 3 张，三张表列宽分别为 9026 = `[1814,3893,2080,1239]` / `[894,963,2039,3480,1650]` / `[1204,3433,4389]`。
- **标题层级**：H1 = 一、二、三、四、五、六（6 个）；H2 = 3.1~3.5（5 个）；front matter 的文档标题只出现在封面与页眉，不在正文重复。

### 7.6 环境与约束

- bundled python：`C:\Users\15188\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe`（3.12.14，python-docx 1.2.0、lxml 6.1.3）。
- 生成器仅标准库；**不使用 LibreOffice**、不跑 cargo、不装新包；渲染（Word→PDF→PNG）属于 C 份 render-verifier。

## 8. 两个小缺口判断（Lead 指定）

| 项目 | 判断 | 说明 |
|---|---|---|
| `w:kinsoku` / `w:overflowPunct` / `w:autoSpaceDE/DN` 中文标点兼容 | **已实现**（位置与 BRIEF 建议略有不同） | BRIEF §4A 建议放 settings.xml，但 OOXML schema 中它们属于 **CT_PPrBase（段落属性）**，settings.xml 会因元素位置不合法被 Word 忽略。本实现写在 `styles.xml` 的 `docDefaults/pPrDefault/pPr`（对全文段落生效）；`settings.xml` 只放合法的 `characterSpacingControl=compressPunctuation`、`compat compatibilityMode=15`、`themeFontLang`、`updateFields`。 |
| `w:doNotExpandShiftReturn` | **未实现（建议谨慎补）** | 属 CT_Settings 元素，但元素在 settings 序列中的插入位置需真机验证；为避免 Word 打开时触发修复对话框，本轮未加。建议 Rust 落地时先在 Word 2016/365 真机确认顺序，再作为 P2 中文排版项补上。 |
| TOC 域结果是占位提示文本 | **有意设计（非缺陷）** | 见 §5.8：静态 OOXML 无法预知真实页码，用真 `TOC` 域 + `updateFields=true` + 灰色提示文案；Word 首次打开提示更新域，更新后即真实目录。占位文案避免「打开后目录空白且用户不知如何操作」。 |

## 9. 已知边界（P2 未实现）

- 图片（`w:drawing` + `word/media/`、`png` content-type）、脚注/尾注、书签 + `REF` 交叉引用：本轮不实现（BRIEF 标 P2；实现图片还需新增 media part 与关系，POC 价值低）。
- 不支持：表格单元格内多行 / 嵌套表格 / reference-style 链接 / HTML 注释 / 数学公式。
- TOC 首次打开需更新域（见 §8）；PDF 渲染与真实页数由 C 份在 Word 中完成，本 POC 不调用 Word COM（避免与 render-verifier 争用 Word 实例）。
- 字体内置假设：宋体 / 黑体 / 微软雅黑 / Consolas / Times New Roman / Arial 由用户 Word 环境提供（fontTable 已声明 altName，缺字体会回退）。
- 自动分页位置无法预知（OOXML 排版由 Word 引擎决定），因此无法在生成期插入「章节另起一页」之外的精确分页；当前只做封面/目录两处分页。

## 10. 给 Lead 的落地结论（可直接引用）

1. **P0（必须改）**：补齐 11 个 part（styles/numbering/settings/header/footer/docProps×2/document.xml.rels/theme/fontTable/webSettings）+ 双节结构 + `pgNumType` + `updateFields`。这五项决定「导航窗格、目录、页码、样式面板」是否可用，是专业感的分水岭。
2. **P1（高收益）**：表格列宽按内容分配（替换 `col_w = total/cols`，L1725）+ 真编号列表（替换 `list_paragraph` L1654）+ 超链接关系 + `---`→Divider + 软换行 `w:br`。前两项是 BRIEF 点名的真实缺陷 8/7。
3. **P2（可选）**：图片、脚注、书签/REF、`doNotExpandShiftReturn`、TOC 目录形式的更多定制。
4. 验收标准可直接复用 `verify_docx.py` 的 34 项断言（part 完整性 / XML 可解析 / 样式与 outlineLvl / 双节与域 / 表宽 9026 / `---` 无字面量 / python-docx 回读），对应 Rust 单测即可。
