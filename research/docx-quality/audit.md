# 直连 API `create_docx`：OOXML / 排版缺陷审计

> 任务：task-1（A：OOXML 与排版缺陷深度审计）
> 审计对象：
> - 生成器代码：`src-tauri/src/direct_api/tools.rs` 的 docx 段（L1398–1956，另含 schema L988–1010、分发与超时 L1231–1236、测试 L5701–5784）
> - 真实产物：`C:\Users\15188\Desktop\AI领域新闻报告汇总_2026年11月.docx`（9,407 B，2026-11-14 00:45）
> 取证环境：system python 3.12.6（`zipfile` / `lxml` / `python-docx 1.2.0`），只读产物，不修改 `src/`、`src-tauri/`，不跑 cargo。
> 结论口径：只写有证据的结论（`文件:行号` 或 XML/命令输出）；本审计不预设 Word 的实际打开行为——凡涉及"Word 打开后是否弹修复提示"的，均标注由渲染链路（task-3，Word COM）判定；本审计给出的是结构层面**必然**的后果与"改造时若不满足约束则必然触发修复"的清单。

---

## 0. 结论速览

### 0.1 三条最致命（P0）

| # | 致命点 | 代码证据 | 产物证据 | 直接后果 |
|---|---|---|---|---|
| 1 | **包只有 3 个 part，样式/编号/设置/页眉页脚/文档属性/关系全部缺失** | `build_docx_bytes` L1464–1499 只写 `[Content_Types].xml`、`_rels/.rels`、`word/document.xml`；`DOCX_CONTENT_TYPES_XML` L1400–1407 只有 1 个 Override | zip 清单只有 3 项（见 §1.3）；参考 docx（python-docx 默认模板，Word 生成）有 17 项，含 `styles.xml`、`numbering.xml`、`settings.xml`、`header*.xml`、`footer*.xml`、`docProps/core.xml`、`docProps/app.xml`、`word/_rels/document.xml.rels` | 没有样式系统、没有真编号、没有页码、没有目录、没有文件属性——这些不是"效果差"，是**物理上不存在** |
| 2 | **标题只是"字号加粗"，没有 `w:pStyle` / `w:outlineLvl`** | `heading_paragraph` L1628–1651 只设 `w:sz` +（level≤2 时）`keepNext/keepLines`；`doc_title_paragraph` L1612–1625 同理；全文无 `w:pStyle` 注入点 | `w:pStyle`=0、`w:outlineLvl`=0、`w:rStyle`=0（全文 grep 计数） | 导航窗格空白；「引用→目录」得到空目录/找不到目录项；样式面板所有段落都是「正文」；按样式统一改版、标题自动编号全部不可用 |
| 3 | **Markdown 软换行被 `join(" ")` 合并成一段** | `markdown_to_document_xml` 的 `flush_para!` L1846–1854，L1849 `para_lines.join(" ")` | 产物第 3 段（body 第 2 段）原文 264 字：`覆盖周期：… 成稿时间：… 信息性质：… 一句话结论：…` 四个字段被合成**一个**段落 | 模型最自然的"每行一个字段"写法直接崩掉：字段边界消失、可读性下降、无法按字段选取/引用；`w:br` 计数=0 说明连软换行都没有保留 |

### 0.2 数字总览

- BRIEF §3 的 14 条特征：**无 8 条 / 半吊子 4 条 / 有但做错 2 条**（详见 §3）。
- 本审计新增 12 条 BRIEF 未列的代码级缺陷（E01–E12，见 §5）。
- 产物 `document.xml` 解压后 65,825 B / 57,422 字符；body 级段落 64 个（python-docx 计数），`w:p` 总数 123（含 59 个表格单元格内段落）；3 张表（4/5/3 列，6/4/5 行）。
- 正文 4,784 字符，其中 CJK 3,276 字符；3 张表的 `gridCol` 之和分别为 9,024 / 9,025 / 9,024，而 `w:tblW` 均为 9,026（均分算法的整数除法丢余数）。

---

## 1. 审计对象与取证方法

### 1.1 代码段（行号已逐条核对）

| 功能 | 函数 / 常量 | 行号 |
|---|---|---|
| `create_docx` 工具 schema | `json!({...})` | L988–1010（`title` 描述 L994） |
| 分发 + 30 s 超时 | match 分支 | L1231–1236 |
| 入口（512 KB 上限、补后缀、写盘） | `exec_create_docx` | L1417–1461（上限 L1422） |
| 打包（手写 zip，3 个 part） | `build_docx_bytes` | L1464–1499 |
| 行内 run 数据结构 / 成对标记切分 / 行内解析 | `InlineRun` / `split_paired_marker` / `parse_inline_runs` | L1501–1560 |
| XML 转义 | `escape_xml` | L1562–1568 |
| 单个 run 生成（字体/字号/粗斜体） | `run_xml` | L1571–1598 |
| 正文段落 | `body_paragraph` | L1601–1609 |
| 文档大标题 | `doc_title_paragraph` | L1612–1625 |
| 章节标题 | `heading_paragraph` | L1628–1651（keep 控制仅 level≤2，L1643–1647） |
| 列表项（手写前缀） | `list_paragraph` | L1654–1675 |
| 引用块 | `quote_paragraph` | L1678–1692（灰色字色插在 `</w:rPr>` 前，L1685–1686） |
| 代码块 | `code_block_paragraph` | L1695–1711 |
| 表格 | `table_xml` | L1714–1773（列宽均分 L1724–1725；单元格不解析行内格式 L1748–1756；表头重复 L1761–1762） |
| 表格行切分 / 分隔线判定 / 标题 / 列表 / 引用解析 | `split_table_row` / `is_table_separator` / `parse_heading` / `parse_bullet` / `parse_ordered_item` / `parse_quote` | L1775–1832 |
| 主解析循环 + 根元素 + `sectPr` | `markdown_to_document_xml` | L1835–1956（`sectPr` L1951–1952） |
| 测试 | `docx_generation_roundtrip` / `dump_docx_for_external_validation` | L5701–5756 / L5768–5784 |

### 1.2 取证命令（可复现）

```bash
python - <<'PY'
import zipfile, re, html, unicodedata
import lxml.etree as ET, docx
p = r"C:\Users\15188\Desktop\AI领域新闻报告汇总_2026年11月.docx"
z = zipfile.ZipFile(p)
print([i.filename for i in z.infolist()])              # part 清单
xml = z.read("word/document.xml").decode("utf-8")
for tok in ["w:pStyle","w:outlineLvl","w:numPr","w:tcW","w:cantSplit","w:hyperlink",
            "w:firstLine","w:widowControl","w:keepNext","w:keepLines","w:updateFields",
            "w:headerReference","w:footerReference","w:br","w:lang","w:noProof"]:
    print(tok, xml.count(tok))                          # 特征计数
ET.fromstring(z.read("word/document.xml"))              # XML 良构性
d = docx.Document(p); print(len(d.paragraphs), len(d.tables))  # 第三方解析器
PY
```

**关键输出**（完整输出见附录 A）：

- part 清单：`[Content_Types].xml`（429 B）、`_rels/.rels`（297 B）、`word/document.xml`（65,825 B），共 3 项。
- `lxml.etree.fromstring` 解析通过，root = `{http://schemas.openxmlformats.org/wordprocessingml/2006/main}document`（XML 良构，无非法字符）。
- `python-docx` 可打开：body 级段落 64、表格 3。

### 1.3 产物基线指标

| 指标 | 值 | 说明 |
|---|---|---|
| zip part 数 | 3 | 正常 Word 文档 11–17 项（参考 docx 17 项，附录 A.3） |
| `w:pStyle` / `w:outlineLvl` / `w:numPr` / `w:rStyle` | 0 / 0 / 0 / 0 | 无命名样式、无大纲、无真编号 |
| `w:sectPr` | 1 个元素（body 末尾） | `pgSz` 11906×16838、`pgMar` 四周 1440、`header` 851、`footer` 992 |
| `w:headerReference` / `w:footerReference` | 0 / 0 | 无页眉页脚，故无页码 |
| `w:tblHeader` | 3 | 3 张表各 1 个，跨页表头重复**做对了** |
| `w:tcW` / `w:cantSplit` | 0 / 0 | 单元格无显式宽、行不禁止跨页拆开 |
| `w:hyperlink` | 0 | 无超链接（且无 rels part） |
| `w:firstLine` | 0 | 正文无首行缩进 |
| `w:widowControl` | 0 | **注意：缺省即启用**，不是缺陷（见 §11 C02） |
| `w:keepNext` / `w:keepLines` | 6 / 6 | 只覆盖 6 个二级标题；5 个三级标题没有 |
| `w:updateFields` / `w:lang` / `w:noProof` / `w:rsid` | 0 | 无域更新、无语言标记、无代码免校对 |
| `w:br` | 0 | 软换行完全丢失 |
| `w:pBdr` | 1 个元素 | 仅 1 处引用块（正文第 40 段），`---` 没有变成边框线 |
| 字面 `---` | 1 处 | 正文第 122/123 段（index 121），其后还有「报告边界」段 |
| 字面 `• ` / 手写序号 | 23 / 9 | 32 个"假列表"段落 |
| 字面 `](` | 0 | 输入 Markdown 没有链接语法（见 §11 C04） |
| `w:rPr` 总数 | 223 | 其中 72 个（32%）违反 `CT_RPr` 子元素顺序（见 E02）；151 个 `(rFonts,sz,szCs)` 顺序合法 |
| `w:gridCol` | 12（4 + 5 + 3） | 4 列表 2256×4=9024；5 列表 1805×5=9025；3 列表 3008×3=9024；`tblW`=9026 |
| 表格后空段落 | 3（index 28 / 83 / 119） | `table_xml` L1771 固定追加 |
| 文本量 | 4,784 字符（CJK 3,276） | 其中正文第 3 段单段 264 字 |

---

## 2. Gap Matrix A：包完整性（part / Content_Types / relationships）

参考基线：`python-docx` 包内默认模板 `templates/default.docx` 的 namelist（Word 2013 生成）：`[Content_Types].xml`、`_rels/.rels`、`customXml/*`、`docProps/app.xml`、`docProps/core.xml`、`docProps/thumbnail.jpeg`、`word/_rels/document.xml.rels`、`word/document.xml`、`word/fontTable.xml`、`word/numbering.xml`、`word/settings.xml`、`word/styles.xml`、`word/stylesWithEffects.xml`、`word/theme/theme1.xml`、`word/webSettings.xml`。

| part | 参考 docx | 本产物 | 缺它的后果 | 目标态生成位置 |
|---|---|---|---|---|
| `[Content_Types].xml` | 有 | 有（只有 1 个 Override） | 新增 part 未登记类型 → 类型不匹配，样式/设置/页眉页脚不生效或触发修复 | 改 `DOCX_CONTENT_TYPES_XML` L1400–1407（§7.1） |
| `_rels/.rels` | 有 | 有（只有 officeDocument 关系） | 无 core/app 关系 → 文档属性不显示 | 改 `DOCX_RELS_XML` L1409–1414（§7.2） |
| `word/document.xml` | 有 | 有 | — | — |
| `word/_rels/document.xml.rels` | 有 | **无** | 无法引用 styles/numbering/settings/header/footer/theme，也无法挂超链接外链；一旦写 `r:id` 必然悬空 → Word 修复提示 | 新增常量 `DOCX_DOCUMENT_RELS_XML`（§7.3） |
| `word/styles.xml` | 有 | **无** | 无命名样式、无 `outlineLvl`、无 docDefaults → 导航窗格空、目录空、样式面板全「正文」 | 新增 `DOCX_STYLES_XML`（§7.4） |
| `word/numbering.xml` | 有 | **无** | 无真编号 → 假列表、增删条目不重排、不能多级 | 新增 `DOCX_NUMBERING_XML`（§7.5） |
| `word/settings.xml` | 有 | **无** | 无 `updateFields`（目录/页码打开不更新）、无 `compatSetting`、无默认制表位、无标点压缩设置 | 新增 `DOCX_SETTINGS_XML`（§7.6） |
| `word/header1.xml` | 有 | **无** | 无页眉（文档名） | 新增 `DOCX_HEADER_XML`（§7.7） |
| `word/footer1.xml` | 有 | **无** | 无页码（`PAGE`/`NUMPAGES` 域） | 新增 `DOCX_FOOTER_XML`（§7.7） |
| `docProps/core.xml` | 有 | **无** | 标题/作者/主题/创建时间全空；Explorer「详细信息」、Word「文件→信息」空白；导出 PDF 的 Title 元数据空；`title` 参数只变成正文第一段（L1837–1839） | 新增 `DOCX_CORE_XML`（§7.8），时间用 `chrono`（`src-tauri/Cargo.toml:47`） |
| `docProps/app.xml` | 有 | **无** | 应用名/版本等元数据缺失 | 新增 `DOCX_APP_XML`（§7.8） |
| `word/theme/theme1.xml` | 有 | **无** | 主题字体/配色不可用；「设计→主题」无内容 | 可选（P2），新增常量 |
| `word/fontTable.xml` | 有 | **无** | 字体未声明；字体缺失时由 Word 做替代，PDF 嵌入不受控 | 可选（P2） |
| `word/webSettings.xml` | 有 | **无** | 网页视图设置缺失（影响很小） | 可选（P2） |
| `word/stylesWithEffects.xml` | 有 | **无** | Word 2010 效果样式（仅兼容用途） | 不需要 |
| `word/media/*` | 视内容 | **无** | 无图片能力（`w:drawing` 完全没实现） | P2 |

**结论**：包级缺口 8 个必需 part（styles/numbering/settings/header/footer/两个 docProps/文档 rels）。BRIEF §2.1 的缺失清单与本审计一致；补充一点：`docProps/thumbnail.jpeg` 不是 Word 正常文档的必需项，不必补。

---

## 3. Gap Matrix B：BRIEF §3 的 14 条特征逐条核对

判定口径：【无】= 完全没有实现；【半】= 做了一部分但达不到目标态；【有-错】= 有实现但行为错误。

| # | BRIEF 缺陷 | 判定 | 代码证据 | 产物证据 | Word 可观察后果（具体） |
|---|---|---|---|---|---|
| 1 | `---` 当普通文本 | 无 | `markdown_to_document_xml` 循环 L1872–1939 无 `---`/`***`/`___` 分支；仅识别表格/空行/标题/列表/引用/普通文本 | 字面 `---` 出现 1 次，是正文第 122 段（`paras[121]` 原文 `<w:t xml:space="preserve">---</w:t>`） | 页面中间出现一行三个半角连字符（12 pt 正文），不是分隔线、不是分页，视觉上就是一行误输入的三连字符 |
| 2 | 软换行被合并 | 有-错 | `flush_para!` L1846–1854；`let text = para_lines.join(" ")` **L1849** | 正文第 3 段把「覆盖周期 / 成稿时间 / 信息性质 / 一句话结论」4 个字段合成 1 段，长度 264 字符（`paras[2]` 原文见附录 B.1）；`w:br`=0 | 本来应 4 行的字段变成 1 整段，`覆盖周期：…（以 11 月 12—13 日的一周为重点） 成稿时间：…` 在行中断开处任意折行；用户无法按字段选择、复制出来是混行；文档首屏信息密度失控 |
| 3 | 无命名样式 | 无 | `heading_paragraph` L1628–1651 只设 `w:sz`/`keepNext`；`doc_title_paragraph` L1612–1625 只设字号+居中；`body_paragraph` L1601–1609 只有 spacing | `w:pStyle`=0、`w:rStyle`=0、`w:tblStyle`=0 | 导航窗格「标题」页签无任何条目；「引用→目录」显示"未找到目录项"或插入空目录；样式面板所有段落显示「正文」；点「标题 1」样式对现有标题无效；标题自动编号不可用；无法按样式一键改字号 |
| 4 | 无页眉页脚、无页码 | 无 | `sectPr` L1951–1952 只有 `pgSz`/`pgMar`，无 `headerReference`/`footerReference`；包内无 header/footer part | `w:headerReference`=0、`w:footerReference`=0 | 打印/导出 PDF 没有任何页码；页眉无文档名/日期；无法做「封面不编号、正文从 1 开始」；多页报告无法引用「第 X 页」 |
| 5 | 无封面 | 无 | 全文档只有 1 个 `sectPr`（L1951）；`title` 参数只生成第一段居中大字（L1837–1839），无分节符、无独立封面字段 | 第 1 段即标题，第 2 段即「一、报告说明」正文 | 第一页顶部直接是"标题+正文"，不是封面；无法放副标题/作者/日期/机构块；无法让封面与正文使用不同页边距/方向 |
| 6 | 无目录 | 无 | 无 `TOC` 域代码；`w:updateFields`=0 | 文档无目录页 | 无法导航长报告；即使手工插入目录，也会因 #3 无大纲级别而得到空目录 |
| 7 | 列表是假列表 | 半 | `list_paragraph` L1654–1675：`prefix` 为字面 `• ` / `{num}. `，作为普通 run 写入（L1655–1667），无 `w:numPr` | 字面 `• ` 23 个、字面序号 9 个，共 32 段；`w:numPr`=0 | 增删条目不会自动重排（序号是文本，会一起被复制/删除）；无法多级编号；无法改项目符号样式；把「列表」按钮用于这些段会得到"文字前缀 + 自动编号"双重符号；Ctrl+A 复制到别的文档时编号格式不跟随 |
| 8 | 表格列宽均分 | 有-错 | `table_xml` L1724–1725：`total_width=9026`，`col_w = total_width / cols` | 4 列表 `gridCol`=2256×4（和 9024）；5 列表 1805×5（和 9025）；3 列表 3008×3（和 9024）；均 ≠ `tblW` 9026 | 5 列表内宽 1805−216=1589 twips，10.5 pt 汉字宽 210 twips → 每行仅 **7.6** 个汉字；「关键要求」62 字格实测折 **9 行**，同表「地区」2 字格 1 行 → 该数据行行高由最长格决定（9 行），其他格大片空白（详见 §9.3） |
| 9 | 表格无行级控制 | 半 | `trPr` 只输出 `w:tblHeader` L1761–1762；无 `cantSplit`、无 `tcW`（单元格 `tcPr` 仅 `shd`+`vAlign` L1757–1759，`tcW` 仅在 `tblW`/`gridCol` 层面） | `w:cantSplit`=0、`w:tcW`=0；`tblHeader`=3（**做对了**）、`vAlign` 有 | 缺 `cantSplit` 即允许行跨页断开，内容会被分页从行中间劈开；`tblW`(9026) 与 `gridCol` 之和(9024/9025) 不一致，Word 编辑/自动调整时按 `tblW` 重算 → 列宽回弹 |
| 10 | 无超链接 | 无 | `parse_inline_runs` L1528–1560 只解析 `` ` ``、`**`、`*` 三种标记，无链接分支；包内无 rels part | `w:hyperlink`=0；字面 `](`=0；`http` 在 document.xml 出现 1 次但属于 `xmlns:w` 命名空间声明（见 §11 C04）——说明输入 Markdown 根本没写链接语法 | 来源、法规原文等都无法点击；导出 PDF 后链接不保留；Word 里这些只是普通文本，不能"编辑超链接" |
| 11 | 正文无首行缩进 | 无 | `body_paragraph` L1601–1609 的 `pPr` 只有 `spacing`，无 `w:ind`/`firstLineChars` | `w:firstLine`=0、`w:firstLineChars`=0 | 中文报告正文没有 2 字符首行缩进；段落起始只靠 6 pt 段后距区分，长文阅读缺少"段"的视觉边界；复制到中文公文模板后格式不匹配 |
| 12 | 无文档属性 | 无 | 包内无 `docProps/*`（`build_docx_bytes` L1464–1499）；`title` 只进正文 L1837–1839 | zip 清单无 `docProps` | Word「文件→信息」标题/作者/主题/创建时间全空；资源管理器「详细信息」的标题列空；导出 PDF 的 Title 元数据空；`title` 参数没有进入属性，用户改标题属性无从下手 |
| 13 | 无孤行/段中抑制 | 半 | `heading_paragraph` L1643–1647：只有 `level <= 2` 输出 `keepNext/keepLines`；`body_paragraph` 无 `widowControl` 元素；表格无 `cantSplit` | `keepNext`=6、`keepLines`=6 恰好等于 6 个二级标题；5 个三级标题（3.1–3.5）无；`widowControl`=0（**但缺省即启用，不算缺陷**，见 §11 C02） | 三级标题落在页底时，Word 会把标题与紧邻正文拆到两页（缺 `keepNext` 不阻止，即"孤行标题"）；表格行可被分页劈开；正文的孤行控制实际是开着的，BRIEF 这一半结论要修正 |
| 14 | 字体内外混搭 | 半 | `run_xml` L1575（等宽：Consolas + eastAsia 微软雅黑）、L1583（正文：Calibri/Calibri/微软雅黑）；无 `w:cs`、无 `w:hint`、无 `w:lang` | 223 个 `rPr` 全是 `rFonts(ascii=Calibri,hAnsi=Calibri,eastAsia=微软雅黑)`；`w:lang`=0、`w:noProof`=0 | 中英混排（如 `Gemini 4 Carbon`、`CEO`）在微软雅黑旁用 Calibri，字面风格不统一；ambiguity range 字符（全角符号/破折号等）的字体归属按 schema 默认走 ascii 字体；拼写/校对语言未标为 zh-CN，中文校对对这部分文本不生效；代码块注释没有 `noProof`，会被拼写检查标红 |

**小结**：BRIEF §3 的 14 条中，8 条完全缺失、4 条半吊子、2 条做错（#2 软换行、#8 列宽均分）。#13 需要修正（`widowControl` 缺省为启用）。

---

## 4. Gap Matrix C：BRIEF §4 的目标态覆盖

### 4.1 A. 包完整性（文档工程）

| 目标项 | 现状 | 判定 | 证据 |
|---|---|---|---|
| 完整 part 集 | 3 个 part | 无 | §2 |
| `docProps/core.xml`（title/creator/created…） | 无 | 无 | zip 清单 |
| `settings.xml` + `w:updateFields` | 无 | 无 | `w:updateFields`=0 |
| 中文标点/字距设置 | 无 | 无 | 无 settings/styles，`w:kinsoku` 应在 pPr（见 §11 C03） |

### 4.2 B. 排版系统（样式驱动）

| 目标项 | 现状 | 判定 | 证据 |
|---|---|---|---|
| 命名样式 Title/Heading1-3/BodyText/Caption/Quote/CodeBlock… | 0 个 | 无 | `w:pStyle`=0 |
| 正文首行缩进 2 字符、1.5 倍行距、`widowControl`、eastAsia 字体 | 无缩进；行距 312/240=**1.3 倍**（L1607）；字体有 | 半 | `w:firstLine`=0；L1607 `w:line="312"`；L1583 |
| 标题 keepNext + keepLines + 段前段后 + 大纲级别 + 编号 | 仅 level≤2 有 keep；无大纲；无编号 | 半 | L1643–1647；`w:outlineLvl`=0 |
| 页面 A4、页边距、页眉页脚距 | A4 + 四边 1440 + header 851/footer 992 | **有** | `sectPr` L1951–1952 |

### 4.3 C. 文档结构（长文档特征）

| 目标项 | 现状 | 判定 | 证据 |
|---|---|---|---|
| 封面节（首页不同，无页眉页脚） | 无 | 无 | 只有 1 个 body 末尾 `sectPr` |
| 正文节（页码从 1 开始） | 无 | 无 | 无 `w:pgNumType` |
| TOC 域 `TOC \o "1-3" \h \z \u` + `updateFields` | 无 | 无 | 无域、无 settings |
| 页脚 `第 X 页 / 共 Y 页` | 无 | 无 | 无 footer part |
| 页眉文档名 + 分隔线 | 无 | 无 | 无 header part |
| 分节符 `w:sectPr`（封面/正文）、`type=nextPage` | 只有一个 body 末尾 sectPr | 无 | L1951 |

### 4.4 D. 内容元素（渲染质量）

| 目标项 | 现状 | 判定 | 证据 |
|---|---|---|---|
| 真编号列表（numbering.xml、多级） | 假列表 | 无 | `w:numPr`=0；字面 `• ` 23 + 序号 9 |
| 表格按内容分配列宽 | 均分 | 无 | L1724–1725 |
| `tcW` / `cantSplit` / 表头重复 / 斑马纹 / 垂直居中 / 单元格边距 | `tblHeader`✓、`vAlign`✓、`tblCellMar`✓、`tblLayout fixed`✓；`tcW`✗、`cantSplit`✗、斑马纹✗ | 半 | L1757–1772 |
| 超链接（`w:hyperlink` + external rel + 颜色下划线） | 无 | 无 | `w:hyperlink`=0 |
| `---` → 段落 `pBdr` 下框线或分页 | 字面文本 | 无 | `paras[121]` |
| 软换行 → `w:br` | 合并成段 | 无 | L1849 |
| 图片 `w:drawing` + media | 无 | 无 | 代码无 `w:drawing` |
| 书签 + REF 交叉引用 | 无 | 无 | 代码无 `w:bookmarkStart` |

---

## 5. 审计新增缺陷（BRIEF §3 未列，全部有代码/产物证据）

| # | 缺陷 | 代码证据 | 产物证据 | Word 可观察后果 |
|---|---|---|---|---|
| E01 | **根元素缺 `xmlns:r` 声明**，任何 `r:id`（页眉页脚引用、超链接、图片）都无法落地 | `markdown_to_document_xml` L1949–1950 只声明 `xmlns:w` | 根元素仅 `xmlns:w` | 直接加 `w:headerReference r:id="rId4"` 会产生未绑定前缀 → XML 非良构 → Word 打开必然弹「发现不可读取的内容」；这是所有 P0 结构改造的前置条件 |
| E02 | **`w:rPr` 子元素顺序违反 `CT_RPr`**（正确顺序要求 `rFonts→b→bCs→i→iCs→…→color→…→sz→szCs`；现有顺序是 `rFonts→sz→szCs→b→bCs`、`…sz→szCs→i→iCs→color`） | `run_xml` L1582–1591（先 sz 后 b/i）；`quote_paragraph` L1685–1686（把 `w:color` 插到 `</w:rPr>` 前，位于所有元素之后） | 223 个 `rPr`：151 个 `(rFonts,sz,szCs)`（顺序合法）；69 个 `(rFonts,sz,szCs,b,bCs)`；2 个 `(…,i,iCs,color)`；1 个 `(…,b,bCs,i,iCs,color)` —— **72/223 违规**（`b`/`color` 被排在 `sz` 之后） | Word 桌面版的实际表现由 task-3 的 COM 打开结果判定；严格校验器（Open XML SDK）会报 `Sch_` 错误并丢弃该 rPr，粗体/斜体/灰色在走 schema 校验的消费方中丢失。本审计对 Word 的实际表现不做预设。 |
| E03 | **表格单元格不解析行内格式**：`table_xml` 把整格文本直接包成一个 run，不调用 `parse_inline_runs` | L1748–1756（`InlineRun { text, bold: is_header, italic: false, mono: false }`） | 本产物数据行单元格恰好没有 `**`；但同一份 Markdown 的表格内容若写 `**重点**`，输出会是字面 `**重点**` | 表格里的人名/数字/结论无法加粗；`[来源](url)` 在表格里也是字面量；与正文的行内渲染行为不一致（同一写法在正文生效、在表格降级） |
| E04 | **`gridCol` 之和 ≠ `tblW`**：`9026/4=2256` 丢 2 twips、`9026/5=1805` 丢 1、`9026/3=3008` 丢 2 | L1724–1725（Rust `u32` 整数除法） | 9024 / 9025 / 9024 vs 9026 | `tblLayout=fixed` 下显示以 grid 为准，但编辑/自动调整时 Word 以 `tblW` 首选宽重算列宽 → 列宽回弹 1–2 twips；末列无法吸收余数 |
| E05 | **表格分隔行判定会误吞数据行**：`is_table_separator` 只要求"字符集 + 含 `-`" | L1782–1784 + L1894–1896（命中即 `continue`，整行丢弃） | 本产物未触发 | `\| - \| 说明 \|` 这种只有短横线的数据行会被当分隔线静默删除；另外表格必须以 `\|` 开头才识别（L1893），无前导竖线的表整张降级为正文；`:---:` 对齐冒号被忽略，不生成单元格对齐 |
| E06 | **标题解析边界**：`#标题`（无空格）不识别；`min(4)` 把 5、6 级压成 4 级；`title` 参数与正文 `#` 标题可重复 | L1786–1798（L1794 `hashes.min(4)`；L1793 要求 `rest` 以空格/制表符开头）；L1837–1839 | 本产物无 `#` 行（用了 `##`），未触发 | 模型写 `#一、报告说明`（中文习惯不加空格）会整行变正文；`#####` 与小标题同字号；`title` 与 `# 标题` 重复时出现两个大标题 |
| E07 | **多行引用被拆成多个独立框**：每行调用一次 `quote_paragraph` | L1931–1934（`parse_quote` 逐行 + `flush_para!`） | 本产物只有 1 行引用（第 40 段） | 连续 `> 行1` `> 行2` 会得到两个各自带左边框+底纹的段落，视觉上像两个引用块；行距/缩进不连续 |
| E08 | **代码块缺 keepLines/语言标记**：整块合并为一个段落 + `w:br` | L1695–1711 | 本产物无代码块（`w:br`=0，无代码块边框） | 代码块可被分页从中间拆开；无 `w:noProof` → 拼写检查标红；无语言标记（无法做语法高亮/可访问性标注） |
| E09 | **正文 1.3 倍行距 + 无两端对齐**：`line=312`、`after=120`，无 `w:jc` | L1607 | 全文 `w:jc`=1（仅标题居中） | 正文行距 1.3 倍（目标 1.5 倍）；左对齐而非两端对齐（`jc=both`）；中文长段落右边缘参差 |
| E10 | **语言/校对标记缺失**：无 `w:lang`、无 `w:noProof`、无 `w:rsid` | `run_xml` L1571–1598 不输出这三者 | 全部为 0 | 中文校对语言未标为 zh-CN；代码/`claude-opus-5-5` 之类字符串被拼写检查标红；Word 不记录修订者 rsid（多人协作时看不出作者） |
| E11 | **表格后固定追加空段落**（`<w:p>`） | L1771 | 空段落 3 个（index 28 / 83 / 119） | 每张表后多出约 6 pt 空白；若用户删掉该空段且下一元素也是表格，Word 会把两张表**合并**成一张 |
| E12 | **`title` 参数不进属性、无封面/副标题/作者/日期字段**：schema 只有 path/title/content | schema L988–1010（L994 `title` 描述"居中显示在正文开头"）；L1837–1839 | 无 docProps；标题仅正文第 1 段 | 用户无法通过工具参数表达作者/日期/副标题/机构；文件属性空白（同 #12） |

---

## 6. 缺陷详解与修复规格

### 6.0 修复总原则

1. **先补包，再谈视觉**：所有排版能力（样式/页码/目录/属性）都要求 part 与关系先存在（§7.1–§7.3）。
2. **样式驱动**：生成器只输出 `w:pStyle` + `w:numPr` + 结构元素，字号/行距/边框/缩进下沉到 `styles.xml`；禁止继续把字号写死在每个 run（现状 `run_xml` 每次重复 `rFonts/sz/szCs`，223 个 rPr 冗余，其中 72 个顺序违规）。
3. **所有 `r:id` 前置 `xmlns:r`**（§8 R01）；所有新 part 前置 Content_Types Override（§8 R02）；所有关系先进 `word/_rels/document.xml.rels`（§8 R03）。
4. **生成器内部先保证不变量**：`sum(gridCol) == tblW`、`sum(tcW) == tblW`、每个 `numId` 有对应 `abstractNum`、`body` 最后一个子元素是 `sectPr`、每节最后一个段落承担该节 `sectPr`。

### 6.1 P0 包结构与根元素（对应 §2、E01）

- 现状：3 part；根元素 `<w:document xmlns:w="…">`（L1950）。
- 修复：① 根元素加 `xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"`（如用 `mc:Ignorable` 扩展再加 `xmlns:mc`/`w14`）；② `build_docx_bytes` L1464–1499 增加 8 个 part 写入；③ `DOCX_CONTENT_TYPES_XML` 与 `DOCX_RELS_XML` 扩写；④ 新增 `word/_rels/document.xml.rels`。
- 落点：`DOCX_CONTENT_TYPES_XML` L1400–1407、`DOCX_RELS_XML` L1409–1414、`build_docx_bytes` L1464–1499。
- 目标 XML：§7.1 / §7.2 / §7.3。

### 6.2 P0 样式系统（对应 #3、#11、#13、#14、E02、E09）

- 现状：0 个 `w:pStyle`；字号/字体/间距全部重复写在 run 与段落 `pPr` 里。
- 修复：新增 `styles.xml`（§7.4），把以下函数改为"只引用样式 + 必要结构"：

| 函数（行） | 现在输出的 pPr | 目标 pPr 头部 | 引用的 styleId |
|---|---|---|---|
| `doc_title_paragraph` L1612–1625 | `spacing` + `jc=center` | `<w:pStyle w:val="Title"/>` | Title（26 pt 居中，黑体） |
| `heading_paragraph` L1628–1651 | `keepNext/keepLines`（≤2 级）+ `spacing` | `<w:pStyle w:val="Heading1"/>`（level 1）… `"Heading4"`（level≥4） | Heading1–4（各带 `outlineLvl` 0–3、keepNext、keepLines） |
| `body_paragraph` L1601–1609 | `spacing` | `<w:pStyle w:val="BodyText"/>` | BodyText（首行缩进 2 字符、1.5 倍行距、`jc=both`、kinsoku/overflowPunct/autoSpaceDE/DN） |
| `list_paragraph` L1654–1675 | `spacing` + 悬挂缩进 + **字面前缀 run** | `<w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>` | ListParagraph |
| `quote_paragraph` L1678–1692 | `pBdr`+`shd`+`spacing`+`ind` | `<w:pStyle w:val="Quote"/>` | Quote（把边框/底纹/缩进下沉到样式） |
| `code_block_paragraph` L1695–1711 | 四边框+`shd`+`spacing`+`ind` | `<w:pStyle w:val="CodeBlock"/><w:keepLines/>` | CodeBlock（含 `noProof`） |
| `table_xml` L1714–1773 | 单元格 `spacing` | 表头 `<w:pStyle w:val="TableHeader"/>`；数据格 `<w:pStyle w:val="TableText"/>` | TableHeader / TableText |

- `doc_title_paragraph` 与 `title` 参数：`title` 只作为封面 Title 段落；正文里不再重复生成标题（E06）。
- 顺带修复 E02：`run_xml` 重排 rPr 为 `rFonts→b→bCs→i/iCs→color→sz→szCs`；`quote_paragraph` 不再用 `replace("</w:rPr>")` 字符串拼接，改为 `run_xml` 的可选 `color: Option<&str>` 参数。

### 6.3 P0 页眉页脚、页码、封面、目录（对应 #4、#5、#6）

- 现状：无 header/footer part、无 `headerReference`、无 `pgNumType`、无 TOC 域、只有一个 body 末尾 `sectPr`。
- 修复（结构级）：
  1. `sectPr`（L1951–1952）扩为两节：封面节（无 header/footer 引用）+ 正文节（引用 header1/footer1、`w:pgNumType w:start="1"`）。
  2. 封面分节符放在**封面最后一段的 `pPr` 内**（段落级 `sectPr`），不能作为 body 中间子元素（§8 R05）。
  3. 正文节后紧跟目录：目录标题段落 + TOC 域（§7.10），依赖样式 `outlineLvl` 与 `settings.xml` 的 `updateFields`。
  4. 页脚放 `PAGE`/`NUMPAGES` 域（§7.7）。
- 落点：`markdown_to_document_xml` L1835–1956（封面/目录要新增 `DocMeta` 入参）、`build_docx_bytes` L1464–1499（新增 header/footer part）、`sectPr` L1951–1952。
- 目标 XML：§7.7 / §7.9 / §7.10。

### 6.4 P1 表格（对应 #8、#9、E03、E04、E05、E11）

- 现状：`table_xml` L1714–1773。已做对：表头重复 `tblHeader`（L1761–1762）、边框、`tblLayout=fixed`、单元格边距 108、垂直居中、表头底纹。缺陷：列宽均分（L1724–1725）、无 `tcW`、无 `cantSplit`、无斑马纹、单元格不解析行内格式、`gridCol` 和 ≠ `tblW`。
- 修复：新增 `compute_col_widths(rows, total=9026)`（§9 伪代码），输出同时写 `w:gridCol` 与每个 `w:tcW`；每行 `w:trPr` 加 `w:cantSplit`；表头行 `tblHeader` 保留；数据行按奇偶加 `w:shd w:fill="F5F7FB"`；单元格文本走 `parse_inline_runs`；`tblW` 数值取 `sum(widths)`。
- 落点：`table_xml` L1714–1773；`split_table_row` L1775–1780（支持 `\|` 转义）；`is_table_separator` L1782–1784（严格化为"每个 cell 都形如 `:?-{1,}:?`"）。
- 目标 XML：§7.3 表格片段 + §7.4 的 TableHeader/TableText 样式。

### 6.5 P1 列表（对应 #7）

- 现状：`list_paragraph` L1654–1675 输出字面 `• ` / `{num}. ` 前缀 + `ind`。
- 修复：删除前缀 run（L1655–1667），改 `w:pStyle=ListParagraph` + `w:numPr`；`numbering.xml` 提供 `numId=1`（bullet）与 `numId=2..n`（每个有序列表块一个独立 numId，起点用 `w:lvlOverride/w:startOverride`）；嵌套用 `w:ilvl`。
- 注意：同一个 `numId` 的段落编号会**连续**，所以每个有序列表必须新分配 `numId`（否则第二段列表从上一段最后一个编号继续）。
- 落点：`list_paragraph` L1654–1675；`markdown_to_document_xml` 中列表分支 L1917–1928（需要 numId 分配器上下文）。
- 目标 XML：§7.5。

### 6.6 P1 分隔线与软换行（对应 #1、#2）

- `---`：新增分支（在 L1910 标题判断前）：整行 `trimmed` 为 `---`/`***`/`___`（≥3 个同类字符）→ 输出 `hr_paragraph()`：

```xml
<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="BFC7D6"/></w:pBdr><w:spacing w:before="120" w:after="120"/></w:pPr></w:p>
```

- 软换行：`flush_para!` L1846–1854 的 `join(" ")` 改为两种策略之一（**推荐 B**）：
  - A. 保留换行：每行一个 run，行间 `<w:r><w:br/></w:r>`（同一段落内软换行，符合 Markdown 语义）。
  - B. 字段行独立成段：若上一行以 `**字段名：**` 开头（中文报告常见字段写法），每行独立 `body_paragraph`，段后距 60；否则保留 A。
  - 无论 A/B，都不允许 `join(" ")`：产物第 3 段 264 字就是该函数的直接结果。
- 落点：L1849（软换行）；L1910 前（`---` 分支）。

### 6.7 P1 文档属性（对应 #12、E12）

- 修复：新增 `docProps/core.xml` / `app.xml`（§7.8），`core.xml` 的 `dc:title` 取 `title` 参数（`escape_xml` 转义），`dc:creator`/`cp:lastModifiedBy` 取默认"桌宠直连 API"或新增参数；`dcterms:created/modified` 用 `chrono::Utc::now()`（`src-tauri/Cargo.toml:47` 已依赖 chrono）。
- 落点：新增常量；`build_docx_bytes` L1464–1499 写盘；`exec_create_docx` L1417–1461 传 title（已传）。
- 目标 XML：§7.8。

### 6.8 P2 超链接（对应 #10）

- 修复：`InlineRun` 增加 `link: Option<String>`；`parse_inline_runs` L1528–1560 增加 `[text](url)` 解析（注意与 `**`/`*` 的优先级：先切链接，再在链接文本内部切粗斜体）；`markdown_to_document_xml` 收集 `Vec<(rId, url)>` 并在 `build_docx_bytes` 拼接 `document.xml.rels`；输出 `<w:hyperlink r:id="rIdN">`，`rIdN` 从 100 起分配，避免与固定 part 关系冲突。
- 落点：`parse_inline_runs` L1528–1560、`markdown_to_document_xml` L1835–1956、`build_docx_bytes` L1464–1499。
- 目标 XML：§7.11。

### 6.9 P2 字体与语言（对应 #14、E10）

- 修复：`docDefaults` 与 `BodyText` 设 `w:eastAsia="宋体"`（正文）/黑体（标题），`w:ascii="Calibri"`；所有中文 run 加 `w:hint="eastAsia"`；`docDefaults` 加 `w:lang w:val="en-US" w:eastAsia="zh-CN"`；代码块/URL 加 `w:noProof`；`rPr` 补 `w:cs`。
- 落点：`run_xml` L1571–1598（rPr 构造参数化）、`styles.xml`（§7.4）。

### 6.10 P3 解析器健壮性（E05、E06、E07、E08）

- `is_table_separator`：改为按 `|` 切分后逐格校验 `^:?-{1,}:?$`，避免把 `| - | 说明 |` 当分隔线（L1782–1784、L1894–1896）。
- 表格识别：允许行内至少 1 个 `|` 且下一行是合法分隔行（L1893），不要强制以 `|` 开头。
- `parse_heading`：允许 `#标题`（去掉"必须空格"约束，L1793），级别上限 6 并映射 Heading1–4（超出 4 级统一 Heading4 或忽略）。
- 引用：把连续 `>` 行合并成一个 `quote_paragraph`（行间 `w:br`），而不是每行一个框（L1931–1934）。
- 代码块：加 `w:keepLines`；空行仍输出 `<w:br/>`（现状可以）；补 `noProof`。

---

## 7. 目标态 XML 规格集（可直接落地）

以下片段是最小可用集合；`[Content_Types].xml` 与关系里的 part 名必须与 zip 条目名完全一致（大小写、相对路径、正斜杠）。

### 7.1 `[Content_Types].xml`（替换 `DOCX_CONTENT_TYPES_XML`，L1400–1407）

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>
<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
<Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/>
<Override PartName="/word/webSettings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>
```

> 约束：`theme/theme1.xml`、`fontTable.xml`、`webSettings.xml` 是 P2 可选项——**只有确实生成了该 part 时才保留对应 Override**，否则删掉；`Default Extension="png"` 同理（无图片时无影响，但声明与写盘保持一一对应最省事）。

### 7.2 `_rels/.rels`（在 `DOCX_RELS_XML` L1409–1414 基础上追加 2 条）

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>
```

### 7.3 `word/_rels/document.xml.rels`（全新 part；超链接关系动态追加到末尾）

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>
<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>
<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>
<Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/>
<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/webSettings" Target="webSettings.xml"/>
<!-- 超链接（动态生成，rId 从 100 起） -->
<Relationship Id="rId100" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/source" TargetMode="External"/>
</Relationships>
```

> **硬约束**：关系只能指向实际写入 zip 的 part。`rId6/rId7/rId8`（theme/fontTable/webSettings）是 P2 可选项——对应 part 不生成时必须删除这三条关系，否则 Word 读到悬空目标会走修复流程（§8 R03）。超链接 rId 从 100 起连续分配，保证与固定关系不冲突。

### 7.4 `word/styles.xml`（最小骨架，19 行，可直接被 Word 接受）

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="宋体" w:cs="Times New Roman"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:widowControl/><w:spacing w:after="120" w:line="360" w:lineRule="auto"/><w:jc w:val="both"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Subtitle"/><w:qFormat/><w:pPr><w:spacing w:before="480" w:after="240"/><w:jc w:val="center"/></w:pPr><w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="52"/><w:szCs w:val="52"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:spacing w:after="240"/><w:jc w:val="center"/></w:pPr><w:rPr><w:rFonts w:eastAsia="楷体"/><w:color w:val="595959"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="480" w:after="240"/><w:jc w:val="left"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="36"/><w:szCs w:val="36"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="360" w:after="200"/><w:jc w:val="left"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="280" w:after="160"/><w:jc w:val="left"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="3"/></w:pPr><w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="26"/><w:szCs w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="BodyText"><w:name w:val="Body Text"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:widowControl/><w:kinsoku/><w:overflowPunct/><w:autoSpaceDE/><w:autoSpaceDN/><w:spacing w:after="120" w:line="360" w:lineRule="auto"/><w:ind w:firstLineChars="200" w:firstLine="480"/><w:jc w:val="both"/></w:pPr><w:rPr><w:rFonts w:eastAsia="宋体"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:spacing w:before="60" w:after="120"/><w:jc w:val="center"/></w:pPr><w:rPr><w:rFonts w:eastAsia="楷体"/><w:i/><w:color w:val="595959"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="BodyText"/><w:next w:val="BodyText"/><w:qFormat/><w:pPr><w:pBdr><w:left w:val="single" w:sz="18" w:space="8" w:color="B7C3D9"/></w:pBdr><w:shd w:val="clear" w:color="auto" w:fill="F3F6FB"/><w:spacing w:before="120" w:after="120"/><w:ind w:left="360"/></w:pPr><w:rPr><w:i/><w:color w:val="595959"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="CodeBlock"><w:name w:val="Code Block"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:keepLines/><w:pBdr><w:top w:val="single" w:sz="4" w:color="E2E2E5"/><w:left w:val="single" w:sz="4" w:color="E2E2E5"/><w:bottom w:val="single" w:sz="4" w:color="E2E2E5"/><w:right w:val="single" w:sz="4" w:color="E2E2E5"/></w:pBdr><w:shd w:val="clear" w:color="auto" w:fill="F5F5F7"/><w:spacing w:before="120" w:after="120"/><w:ind w:left="240" w:right="240"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:eastAsia="微软雅黑"/><w:noProof/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TableHeader"><w:name w:val="Table Header"/><w:basedOn w:val="Normal"/><w:next w:val="TableText"/><w:qFormat/><w:pPr><w:spacing w:before="40" w:after="40"/><w:jc w:val="center"/></w:pPr><w:rPr><w:b/><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TableText"><w:name w:val="Table Text"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:before="20" w:after="20"/><w:jc w:val="left"/></w:pPr><w:rPr><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="60"/></w:pPr></w:style>
<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
</w:styles>
```

> 骨架说明：`w:pPr`/`w:rPr` 内元素顺序均按 `CT_PPr`/`CT_RPr` 的 sequence 排列（顺序错了同样属架构违规）；`w:outlineLvl` 让 TOC `\o "1-3"` 能取到 Heading1–3；`Title` 与正文标题分离，封面用 Title/Subtitle。

### 7.5 `word/numbering.xml`（全新 part）

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>
<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="微软雅黑" w:hAnsi="微软雅黑" w:hint="eastAsia"/></w:rPr></w:lvl>
<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="–"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>
</w:abstractNum>
<w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/>
<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>
<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>
</w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
</w:numbering>
```

> 每个有序列表块分配一个新的 `w:num`（numId 递增，`abstractNumId` 复用 1），若源 Markdown 首个序号不是 1，在该 `w:num` 内加 `<w:lvlOverride w:ilvl="0"><w:startOverride w:val="N"/></w:lvlOverride>`。这样增删条目 Word 会自动重排；同一个 numId 跨列表使用会导致编号连续（第二段列表不从 1 开始）。

### 7.6 `word/settings.xml`（全新 part）

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:zoom w:percent="100"/>
<w:defaultTabStop w:val="420"/>
<w:characterSpacingControl w:val="compressPunctuation"/>
<w:updateFields w:val="true"/>
<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>
</w:settings>
```

> 说明见 §8 R06：`w:updateFields w:val="true"` 只负责"Word 打开时更新域"；TOC/PAGE/NUMPAGES 的域结构必须在正文里先存在。没有域时它不产生任何效果。`w:characterSpacingControl` 是中文标点压缩；`w:compat` 的 `compatibilityMode=15` 声明为 Word 2013+ 格式（不写则按旧格式兼容模式处理）。

### 7.7 `word/header1.xml` / `word/footer1.xml`（全新 part）

`header1.xml`（页眉：文档名 + 下框线，右对齐）：

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="4" w:space="1" w:color="BFC7D6"/></w:pBdr><w:jc w:val="right"/></w:pPr><w:r><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve">AI 领域新闻报告汇总（2026 年 11 月）　|　内部研究稿</w:t></w:r></w:p>
</w:hdr>
```

`footer1.xml`（页脚：`第 X 页 / 共 Y 页`，居中）：

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:p><w:pPr><w:jc w:val="center"/></w:pPr>
<w:r><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve">第 </w:t></w:r>
<w:fldSimple w:instr=" PAGE "><w:r><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t>1</w:t></w:r></w:fldSimple>
<w:r><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve"> 页 / 共 </w:t></w:r>
<w:fldSimple w:instr=" NUMPAGES "><w:r><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t>1</w:t></w:r></w:fldSimple>
<w:r><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve"> 页</w:t></w:r>
</w:p>
</w:ftr>
```

> `w:fldSimple/@w:instr` 的值必须保留两侧空格（`" PAGE "`/`" NUMPAGES "`），否则域指令解析失败。若偏好复杂域，等价写法见 §7.10（TOC 用复杂域）。

### 7.8 `docProps/core.xml` / `docProps/app.xml`（全新 part）

`core.xml`（`dc:title` 用 `escape_xml` 转义；时间为 `chrono::Utc::now()` 的 RFC3339）：

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>AI 领域新闻报告汇总（2026 年 11 月）</dc:title>
<dc:subject>AI 行业周报</dc:subject>
<dc:creator>桌宠直连 API</dc:creator>
<cp:lastModifiedBy>桌宠直连 API</cp:lastModifiedBy>
<cp:revision>1</cp:revision>
<dcterms:created xsi:type="dcterms:W3CDTF">2026-11-13T15:45:00Z</dcterms:created>
<dcterms:modified xsi:type="dcterms:W3CDTF">2026-11-13T15:45:00Z</dcterms:modified>
</cp:coreProperties>
```

`app.xml`：

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">
<Application>AI Desktop Pet</Application><DocSecurity>0</DocSecurity><ScaleCrop>false</ScaleCrop><LinksUpToDate>false</LinksUpToDate><SharedDoc>false</SharedDoc><HyperlinksChanged>false</HyperlinksChanged><AppVersion>16.0000</AppVersion>
</Properties>
```

### 7.9 封面节与 `sectPr`（替换 L1951–1952 的 body 尾部）

```xml
<w:body>
<!-- 封面：Title / Subtitle / 作者 / 日期（由 DocMeta 生成） -->
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>AI 领域新闻报告汇总（2026 年 11 月）</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Subtitle"/></w:pPr><w:r><w:t>2026 年 11 月 1 日 — 11 月 14 日</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="BodyText"/><w:jc w:val="center"/></w:pPr><w:r><w:t>桌宠直连 API　|　2026-11-14</w:t></w:r></w:p>
<!-- 封面节结束：段落级 sectPr（该段落必须是封面最后一段） -->
<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="851" w:footer="992" w:gutter="0"/><w:cols w:space="720"/></w:sectPr></w:pPr></w:p>
<!-- 目录页 -->
<w:p><w:pPr><w:spacing w:before="240" w:after="240"/><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr><w:t>目　录</w:t></w:r></w:p>
<w:p>
<w:r><w:fldChar w:fldCharType="begin"/></w:r>
<w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r>
<w:r><w:fldChar w:fldCharType="separate"/></w:r>
<w:r><w:t>（打开 Word 时自动更新）</w:t></w:r>
<w:r><w:fldChar w:fldCharType="end"/></w:r>
</w:p>
<!-- 正文 … -->
<!-- 正文节：body 的最后一个子元素，带页眉页脚引用与页码起始 -->
<w:sectPr>
<w:headerReference w:type="default" r:id="rId4"/>
<w:footerReference w:type="default" r:id="rId5"/>
<w:pgSz w:w="11906" w:h="16838"/>
<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="851" w:footer="992" w:gutter="0"/>
<w:pgNumType w:start="1"/>
<w:cols w:space="720"/>
<w:docGrid w:linePitch="312"/>
</w:sectPr>
</w:body>
```

> 必须同时满足：① `w:document` 根元素声明 `xmlns:r`（§8 R01）；② 封面节的段落级 `sectPr` 只能放在封面最后一段的 `pPr` 内；③ body 的最后一个子元素是唯一的 body 级 `sectPr`；④ 两个节都省略 `w:type`（默认 nextPage）即"新页起"。

### 7.10 TOC 域（复杂域写法，比 §7.9 的简写更明确）

```xml
<w:p><w:pPr><w:pStyle w:val="BodyText"/></w:pPr>
<w:r><w:fldChar w:fldCharType="begin"/></w:r>
<w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r>
<w:r><w:fldChar w:fldCharType="separate"/></w:r>
<w:r><w:t>目录</w:t></w:r>
<w:r><w:fldChar w:fldCharType="end"/></w:r>
</w:p>
```

- `\o "1-3"` 取 outline level 1–3（对应 Heading1–3 的 `w:outlineLvl` 0–2）；`\h` 生成超链接；`\z` 隐藏域结果中的制表符前导符；`\u` 使用段落大纲级别。
- 配合 `settings.xml` 的 `w:updateFields w:val="true"`：Word 首次打开时自动重建目录，用户无需手动"更新域"。**两者缺一不可**：只有域没有 updateFields → 打开时目录显示占位文本；只有 updateFields 没有域 → 什么也不会发生。

### 7.11 超链接（`w:hyperlink` + 外部关系）

正文片段：

```xml
<w:hyperlink r:id="rId100"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>新浪财经</w:t></w:r></w:hyperlink>
```

`document.xml.rels` 追加（`Target` 必须做 URI 百分号编码，属性值做 XML 转义）：

```xml
<Relationship Id="rId100" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://finance.sina.com.cn/stock/2026-11-12/doc-inxxxx.shtml" TargetMode="External"/>
```

### 7.12 `pStyle` 引用关系总表（函数 → styleId）

| 生成位置（tools.rs） | 目标 XML | styleId（styles.xml） |
|---|---|---|
| `doc_title_paragraph` L1612–1625（仅封面用） | `<w:pStyle w:val="Title"/>` | Title |
| 封面副标题（新增） | `<w:pStyle w:val="Subtitle"/>` | Subtitle |
| `heading_paragraph` L1628–1651，level=1 | `<w:pStyle w:val="Heading1"/>` | Heading1（outlineLvl=0） |
| 同上，level=2 / 3 / ≥4 | `Heading2` / `Heading3` / `Heading4` | outlineLvl=1/2/3 |
| `body_paragraph` L1601–1609 | `<w:pStyle w:val="BodyText"/>` | BodyText |
| `list_paragraph` L1654–1675 | `ListParagraph` + `numPr` | ListParagraph |
| `quote_paragraph` L1678–1692 | `<w:pStyle w:val="Quote"/>` | Quote |
| `code_block_paragraph` L1695–1711 | `<w:pStyle w:val="CodeBlock"/>` | CodeBlock |
| `table_xml` L1714–1773 表头格 | `<w:pStyle w:val="TableHeader"/>` | TableHeader |
| `table_xml` 数据格 | `<w:pStyle w:val="TableText"/>` | TableText |
| 表格说明文字（新增 Caption 支持，P2） | `<w:pStyle w:val="Caption"/>` | Caption |
| 超链接 run（新增 `w:hyperlink`） | `<w:rStyle w:val="Hyperlink"/>` | Hyperlink（字符样式） |
| `---` 分隔线（新增 `hr_paragraph`） | 无 pStyle，仅 `pBdr` | — |

### 7.13 `tools.rs` 改动点清单（按行）

| 行 | 类型 | 改动 |
|---|---|---|
| L1400–1407 | 常量 | `DOCX_CONTENT_TYPES_XML` 扩到 §7.1 全部 Override |
| L1409–1414 | 常量 | `DOCX_RELS_XML` 追加 core/app 两条关系（§7.2） |
| 新增 | 常量 | `DOCX_DOCUMENT_RELS_XML`（§7.3）、`DOCX_STYLES_XML`（§7.4）、`DOCX_NUMBERING_XML`（§7.5）、`DOCX_SETTINGS_XML`（§7.6）、`DOCX_HEADER_XML`/`DOCX_FOOTER_XML`（§7.7）、`DOCX_CORE_XML`/`DOCX_APP_XML`（§7.8） |
| L1464–1499 | `build_docx_bytes` | 增写 8 个 part；动态拼接超链接 rels；`document.xml` 的 `body` 采用 §7.9 结构 |
| L1571–1598 | `run_xml` | 修 rPr 顺序（E02）；增加可选 `color`/`link`/`noProof` 参数；正常中文 run 加 `w:hint="eastAsia"` |
| L1601–1609 | `body_paragraph` | 改 `pStyle=BodyText`；删除内联 spacing（样式接管）；支持软换行 `w:br` |
| L1612–1625 | `doc_title_paragraph` | 改 `pStyle=Title`；仅封面调用 |
| L1628–1651 | `heading_paragraph` | 改 `pStyle=HeadingN`；删除每个 run 的 `w:sz`（样式接管）；不再只对 ≤2 级加 keep |
| L1654–1675 | `list_paragraph` | 删字面前缀 run；改 `pStyle=ListParagraph` + `numPr`（numId 由调用方分配） |
| L1678–1692 | `quote_paragraph` | 改 `pStyle=Quote`；边框/底纹/缩进下沉样式；去掉 `replace("</w:rPr>")` |
| L1695–1711 | `code_block_paragraph` | 改 `pStyle=CodeBlock`；加 `w:keepLines`（在样式里） |
| L1714–1773 | `table_xml` | 新增 `compute_col_widths`；输出 `tcW` + `cantSplit` + 斑马纹；单元格走 `parse_inline_runs`；表头/数据格 pStyle；`tblW=sum(gridCol)` |
| L1775–1784 | `split_table_row` / `is_table_separator` | 支持 `\|` 转义；分隔行严格化（E05） |
| L1786–1798 | `parse_heading` | 允许无空格 `#` 标题；级别映射修正（E06） |
| L1835–1956 | `markdown_to_document_xml` | `flush_para` 去 `join(" ")`（L1849）；加 `---` 分支；列表 numId 分配器；超链接收集；封面 + TOC + 两节 `sectPr`；根元素加 `xmlns:r`（L1949–1950） |
| L1417–1461 | `exec_create_docx` | 传 `title`/可选 author 给 `build_docx_bytes`；时间戳 `chrono::Utc::now()` |
| L5701–5756 | 测试 | 断言升级：part 数 ≥ 11、`w:pStyle`>0、`w:outlineLvl`≥3、`w:numPr`>0、`w:hyperlink`>0、`docProps` 存在、`gridCol` 和 == `tblW` |

---

## 8. Word 兼容性风险清单

> 判定依据：OOXML 架构约束（ECMA-376 Part 1）；"修复提示"= Word 打开时弹「发现不可读取的内容 / 是否修复」。
> 本审计**不预设** Word 对"元素顺序违规"的实际容忍度：顺序违规本身不破坏 XML 良构性（lxml 已解析通过），Word 侧由 task-3 的 COM 打开结果判定；但下面 R02/R03/R04/R10/R11/R12/R14 属于"结构错误/悬空引用/类型不匹配"，只要不满足约束，Word 必然无法正常挂载该 part 或必然弹修复。

| # | 风险 / 触发条件 | 后果 | 规避（对应实现） | 依据 |
|---|---|---|---|---|
| R01 | `w:document` 根元素**未声明 `xmlns:r`** 就使用 `r:id`（页眉页脚引用、超链接、图片） | 未绑定前缀 → XML 命名空间良构性失败 → **必然弹修复提示** | 根元素加 `xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"`（L1949–1950） | XML Namespaces；OOXML 关系引用 |
| R02 | 新增 part 未在 `[Content_Types].xml` 登记 Override（或类型字符串写错） | 该 part 按 `Default Extension="xml"` 归为 `application/xml`，Word 不识别 styles/settings/numbering/header/footer → 样式不生效，或修复 | §7.1 全量 Override；每新增一个 part 同步一条 Override | OPC Content Types |
| R03 | `word/_rels/document.xml.rels` **不存在**，而 `document.xml` 里出现 `r:id`；或关系指向的 part 没有写入 zip（如声明了 theme/fontTable 关系却没写对应文件） | 关系悬空（找不到目标 part）→ 修复提示；页眉页脚/超链接失效 | 新增 §7.3 part；固定关系 rId1–rId8 只在对应 part 存在时写；动态超链接从 rId100 起且不得重复 | OPC Relationships |
| R04 | `w:numPr` 的 `numId` 在 numbering.xml 中不存在，或 `abstractNumId` 悬空；或 `w:num` 排在 `w:abstractNum` 之前 | 编号不显示 / 列表损坏提示 | §7.5：`abstractNum*` 全部先于 `num*`；每个 numId 有对应 abstractNumId | CT_Numbering 顺序定义 |
| R05 | `sectPr` 位置错误：body 中间出现 body 级 `sectPr`；或段落级 `sectPr` 不在该节最后一段；或一个文档出现两个 body 末尾 `sectPr` | 分节异常（封面节没有结束标记，正文节套错页眉页脚），Word 打开时触发结构错误/修复流程 | §7.9：封面 `sectPr` 放在封面最后一段 `pPr` 内；正文 `sectPr` 是 body 最后一个子元素；每个节只有一个 `sectPr` | CT_SectPr / body 结构 |
| R06 | 只有 `w:updateFields` 没有域；或只有 TOC 域没有 `updateFields` | 前者无效果；后者 TOC 打开时显示占位文本/上次结果，页码域不刷新 | §7.6（settings）+ §7.9/§7.10（域结构）同时提供；域必须含 `begin/separate/end` 或 `fldSimple` | FIELD 结构 + settings |
| R07 | `w:rPr` 顺序违规（现状 72/223，违规形态是 `b`/`color` 排在 `sz` 之后） | XML 良构；严格校验器（Open XML SDK）会报错并丢弃该 `rPr`（粗体/斜体/灰色在走 schema 校验的消费方中丢失）；Word 桌面版的实际表现以 task-3 的 COM 打开结果为准 | `run_xml` 重排：`rFonts→b→bCs→i→iCs→color→sz→szCs`；`quote_paragraph` 不再字符串拼接 color | CT_RPr sequence |
| R08 | 新增 `styles.xml` 的 `w:pPr`/`w:rPr` 子元素顺序写错（例如 `w:sz` 排在 `w:b` 前、`w:outlineLvl` 排在 `w:spacing` 前） | 同 R07；样式被 Word 部分忽略 | 按 §7.4：pPr 顺序 `keepNext→keepLines→widowControl→kB→kinsoku→overflowPunct→autoSpaceDE/DN→spacing→ind→jc→outlineLvl`；rPr 顺序 `rFonts→b→i→noProof→color→sz→szCs→u` | CT_PPr / CT_RPr |
| R09 | `tblW` 与 `gridCol`/`tcW` 之和不一致；或 `tblLayout=fixed` 时 `tcW` 缺失 | 编辑/自动调整时列宽回弹；`sum(gridCol)` 必须等于 `tblW` | §9 算法保证 `sum(widths)==9026`；每格写 `tcW` = 该列宽 | CT_TblPr/CT_TblGrid |
| R10 | 超链接关系缺 `TargetMode="External"`；`r:id` 重复；`Target` 未做 URI 编码 | 链接不跳转 / 关系冲突 → 修复提示 | §7.11：外链必带 `TargetMode="External"`；rId 全局唯一；URL 百分号编码 | OPC 外部关系 |
| R11 | `docProps/core.xml` 缺 `xmlns:xsi`/`dcterms`，或 `dcterms:created` 没有 `xsi:type="dcterms:W3CDTF"`，或时间格式不是 `YYYY-MM-DDTHH:MM:SSZ`；core.xml 关系类型写错 | XML 非良构（未绑定前缀）→ 修复提示；或属性不显示 | 用 §7.8 原样常量；`_rels/.rels` 用 `…/package/2006/relationships/metadata/core-properties`；Content_Types 用 `application/vnd.openxmlformats-package.core-properties+xml` | OPC Core Properties |
| R12 | `header1.xml`/`footer1.xml` 未登记专用 Content Type（被 `Default xml` 接管），或关系类型写成普通 xml | 页眉页脚不加载 / 修复提示 | §7.1 的 header/footer Override + §7.3 的 `…/relationships/header`、`…/relationships/footer` | OPC + OOXML |
| R13 | `w:tblHeader` 放在 `w:trPr` 之外，或不在第一行 | 表头不重复（无修复提示，静默失效） | 现状已正确（L1761–1762），保持 | CT_TrPr |
| R14 | `w:fldSimple/@w:instr` 丢掉两侧空格（写成 `"PAGE"`）；`w:instrText` 缺 `xml:space="preserve"`；`w:fldChar` 序列缺 `separate` | 域不执行/显示原始指令文本 | §7.7 保留 `" PAGE "`/`" NUMPAGES "`；§7.10 保留首尾空格与 `xml:space="preserve"` | FIELD 指令语法 |
| R15 | 表格后没有段落（`w:body` 以 `w:tbl` 结尾），或两张表紧邻无段落隔开 | 相邻表格被 Word 合并成一张；body 尾部结构异常 | 保留 `table_xml` L1771 的空段落（现状正确），仅在"表后紧跟另一张表"时必要 | OOXML body 结构 / Word 行为 |
| R16 | zip 条目名大小写不一致（`Word/Document.xml`）、带前导 `/`、或 part 名与关系 Target 不一致 | 关系找不到目标 → 修复提示 | 全部用 §7.1/§7.3 中的小写相对路径；`[Content_Types].xml` 作为第一个条目写入；关系 Target 相对 `word/` 目录 | OPC 打包规则 |

**"必然触发修复提示"的写法清单（实现时必须避免）**：R01、R02（类型字符串错）、R03、R10（rId 重复）、R11（未绑定前缀）、R12（类型错）、R16。

---

## 9. 表格列宽按内容分配算法

### 9.1 常量与单位

- 总宽 `TOTAL = 9026` twips（A4 内容宽：`11906 − 2×1440`，与 L1724 一致）。
- 单元格内边距 `PAD = 216` twips（左右各 108，与 L1771 的 `tblCellMar` 一致）。
- `UNIT = 105` twips = 10.5 pt 下 1 个半角字符宽；1 个全角/宽字符（CJK、全角标点）= **2 units**（210 twips）。
- `MIN_W = 1000`（约 4.8 个汉字，防止单列被压成竖条）；`MAX_W = 3600`（约 17 个汉字，防止单列吃掉半张纸）。
- 列数上限：`n × MIN_W ≤ TOTAL` → `n ≤ 9`；超过 9 列时先拒绝/拆表（本产物最多 5 列，安全）。

### 9.2 伪代码

```
CONST UNIT = 105; PAD = 216; MIN_W = 1000; MAX_W = 3600; TOTAL = 9026

# 1) 长度单位：宽字符 2，半角 1
fn units(s):
    u = 0
    for ch in s:
        u += 2 if east_asian_width(ch) in {W, F} else 1
    return u

# 2) 每列自然宽 = 该列最长单元格（先剥离 Markdown 标记：**、`、[文字](url) 只计文字）+ 左右内边距
fn natural_widths(rows, cols):
    for c in 0..cols-1:
        max_units[c] = max(units(strip_markdown(rows[r][c])) for r in 0..rows-1)
        nat[c] = max_units[c] * UNIT + PAD
    return nat

# 3) 归一到 TOTAL，并钳制 [MIN_W, MAX_W]
fn allocate(nat):
    n = len(nat)
    base[c] = clamp(nat[c], MIN_W, MAX_W)
    if sum(base) <= TOTAL:                      # 放大：余量给未触 MAX 的列
        head[c] = MAX_W - base[c]
        k = (TOTAL - sum(base)) / sum(head)
        w[c] = base[c] + head[c] * k
    else:                                       # 压缩：按"自然宽 - MIN"比例压缩
        exc[c] = max(0, nat[c] - MIN_W)
        k = (TOTAL - n * MIN_W) / sum(exc)
        w[c] = clamp(MIN_W + exc[c] * k, MIN_W, MAX_W)
    # 钳制后调平：差值按 headroom / excess 权重逐 twip 分摊
    while sum(w) != TOTAL:
        d = TOTAL - sum(w)
        pool = (w < MAX_W) if d > 0 else (w > MIN_W)
        for i in pool: w[i] += d * weight[i] / sum(weight)      # weight = 剩余 headroom 或 excess
    # 取整并补偿余数：先 floor，余数按小数部分降序逐列 +1
    wi[c] = floor(w[c])
    rem = TOTAL - sum(wi)
    for c in argsort_desc(w - wi)[0 : rem]: wi[c] += 1
    assert sum(wi) == TOTAL and all(MIN_W <= wi[c] <= MAX_W for c)
    return wi
```

**为什么要"压缩比例用自然宽"，而不是直接用 `clamp` 后的值**：本产物 5 列表的「关键要求」（124 units）和「影响对象」（44 units）在 `MAX_W` 处都被截成 3600，若按钳制后的值压缩，两列会被压成同样宽（3168/3168），丢失"关键要求更长"的差异；按自然宽压缩才能保留内容差异。

### 9.3 对本产物 3 张表的实算结果（脚本可复现）

| 表 | 列 | 最长格 units | 自然宽 twips | 现宽 | 现内宽/最长格行数 | 建议宽 | 建议内宽/最长格行数 |
|---|---|---|---|---|---|---|---|
| 表1（4 列「五条主线」） | `#` | 1 | 321 | 2256 | 2040 / 1 | 1171 | 955 / 1 |
| | 主线 | 26 | 2946 | 2256 | 2040 / 2 | 1958 | 1742 / 2 |
| | 一句话概括 | 73 | 7881 | 2256 | 2040 / 4 | 3600 | 3384 / 3 |
| | 可信度 | 34 | 3786 | 2256 | 2040 / 2 | 2297 | 2081 / 2 |
| **表1 合计** | | | | **9024（≠tblW 9026）** | | | **9026** |
| 表2（5 列「治理合规」） | 地区 | 11 | 1371 | 1805 | 1589 / 1 | 1066 | 850 / 2 |
| | 时间 | 14 | 1686 | 1805 | 1589 / 1 | 1121 | 905 / 2 |
| | 动作 | 61 | 6621 | 1805 | 1589 / 5 | 1995 | 1779 / 4 |
| | 关键要求 | 124 | 13236 | 1805 | 1589 / **9** | 3165 | 2949 / **5** |
| | 影响对象 | 44 | 4836 | 1805 | 1589 / 3 | 1679 | 1463 / 4 |
| **表2 合计** | | | | **9025（≠tblW 9026）** | 行高 = max(1,1,5,9,3) = **9 行** | | **9026**，行高 = max(2,2,4,5,4) = **5 行** |
| 表3（3 列「来源分级」） | 等级 | 10 | 1266 | 3008 | 2792 / 1 | 1826 | 1610 / 1 |
| | 含义 | 52 | 5676 | 3008 | 2792 / 2 | 3600 | 3384 / 2 |
| | 本报告中的例子 | 70 | 7566 | 3008 | 2792 / 3 | 3600 | 3384 / 3 |
| **表3 合计** | | | | **9024（≠tblW 9026）** | | | **9026** |

- 5 列表最直观：现宽下「关键要求」62 字（124 units）折 **9 行**（`1589/105 = 15.1` units/行 → `124/15.1 = 8.2 → 9`），「地区」2 字 1 行；该数据行行高由 9 行格决定。按内容分配后该行最长格折 5 行，行高相对现状 **降 44%**（9→5 行）。
- 代价：「地区」「时间」两列从 1 行变 2 行（自然宽只有 11/14 units，压缩后内宽 850/905 twips）。若希望日期/地区不折行：给这两列设 `hard_min = max_units*UNIT + PAD`（不参与压缩），或把 `MIN_W` 提到 1200。
- 表1 的「一句话概括」仍是全表最宽；表3 的「本报告中的例子」含 61 字长句，建议同时接受 3 行或缩短内容（内容层由 authoring 负责）。

### 9.4 边界情况

- `n × MIN_W > TOTAL`（>9 列）：先降 `MIN_W` 到 `TOTAL/n`，或按列意图拆表；不要静默把某些列压到 0。
- 单元格为空：`units=0`，只计 `PAD`；空列不应参与自然宽竞争（可给 `MIN_W`）。
- 单元格里出现 URL：按半角计长（URL 不可断行，`strip_markdown` 后保留 URL 文字；若 URL 超过列内宽，Word 仍会在 `/` 处断行，属可接受）。
- 合并单元格（如 P2 支持 `colspan`）：`gridSpan` 的列宽 = 被覆盖列宽之和，`tcW` 用和值。

---

## 10. 修复优先级与验收

### 10.1 优先级

| 优先级 | 项目 | 覆盖缺陷 | 工作量 | 风险 |
|---|---|---|---|---|
| **P0** | 补 8 个必需 part + Content_Types + rels + 根元素 `xmlns:r` | §2、E01、#3/#4/#6/#12 | M | 任一处类型/关系写错即触发修复提示（§8 R01–R03） |
| **P0** | 样式系统（styles.xml + pStyle 映射 + outlineLvl） | #3、#11、#13、#14、E02、E09 | M | 样式顺序写错（R07/R08） |
| **P0** | 页眉页脚 + 页码 + 封面分节 + TOC 域 + updateFields | #4、#5、#6、#13 | L | sectPr 位置（R05）、域结构（R06/R14） |
| **P0** | 软换行不再 `join(" ")` | #2 | S | 需同时决定 A/B 策略与字段行判定 |
| **P1** | 表格：内容列宽 + tcW + cantSplit + 斑马纹 + 单元格行内解析 | #8、#9、E03、E04、E05 | M | 列宽算法与取整（R09） |
| **P1** | 真编号列表（numbering.xml + numId 分配器） | #7 | M | numId 悬空/连续编号（R04） |
| **P1** | `---` → pBdr 分隔线 | #1 | S | — |
| **P1** | 文档属性（core/app） | #12、E12 | S | 命名空间/时间格式（R11） |
| **P2** | 超链接（parse_inline_runs + rels） | #10 | M | 外链/URI 编码（R10） |
| **P2** | 字体/语言细化（hint/lang/noProof/cs） | #14、E10 | S | 与样式改造一起做 |
| **P2** | theme/fontTable/webSettings、Caption、书签/REF、图片 | BRIEF §4 的 P2 | L | 非必需 |

### 10.2 自动验收断言（生成后可 grep/XML 检查）

| 断言 | 期望 |
|---|---|
| zip part 数 | ≥ 11（含 `word/_rels/document.xml.rels`、`styles.xml`、`numbering.xml`、`settings.xml`、`header1.xml`、`footer1.xml`、`docProps/core.xml`、`docProps/app.xml`） |
| `[Content_Types].xml` | 每个 part 有对应 Override/Default；`header1.xml` 类型为 `…wordprocessingml.header+xml` |
| `document.xml` 良构 | `lxml.etree.fromstring` 通过；根元素含 `xmlns:r` |
| `w:pStyle` | > 0，且出现 Title/Heading1/Heading2/Heading3/BodyText/Quote/CodeBlock/TableHeader/ListParagraph 的 `w:val` |
| `styles.xml` `w:outlineLvl` | ≥ 3（0/1/2），TOC 才能取到 1–3 级 |
| `w:numPr` | > 0；`numbering.xml` 中每个 `w:numId` 有对应 `w:num` 与 `w:abstractNum` |
| `w:hyperlink` | > 0 时必须有外部关系和 `TargetMode="External"` |
| 页眉页脚 | `w:headerReference` ≥ 1、`w:footerReference` ≥ 1、`w:pgNumType w:start` = 1、`w:updateFields` = 1 |
| 表格 | `sum(gridCol) == tblW`；每个 `w:tc` 有 `w:tcW`；每个数据行 `w:trPr` 有 `w:cantSplit`；`tblHeader` 在首行 |
| `w:rPr` 顺序 | 无 `w:sz` 出现在 `w:b` 之前；无 `w:color` 出现在 `w:sz` 之后 |
| 软换行 | `w:br` 计数 > 0（多行字段场景）；或字段行数 == 段落数 |
| `---` | `w:pBdr/w:bottom` 计数 ≥ 1，且全文无字面 `<w:t>---</w:t>` |
| docProps | `dc:title` == `title` 参数；`dcterms:created` 有 `xsi:type="dcterms:W3CDTF"` |

---

## 11. 与 BRIEF 的不一致（需 Lead 修正）

> 以下为审计中实际核对出的偏差；BRIEF 其余引用（§2.2 指标、§2.3 全部行号）与实际代码/产物一致。

**C01（BRIEF §3.1 最后一段）**：BRIEF 写"产物最后一段正文是字面 `---`（见 document.xml 尾部 `<w:t>---</w:t>`）"，**不准确**。实际 `---` 是全文第 122/123 段（`paras[121]`），其后还有第 123 段「报告边界：本报告为公开信息汇总，不构成投资建议…」，`document.xml` 尾部是「报告边界」段而不是 `---`。证据：

```xml
paras[121] = <w:p><w:pPr><w:spacing w:after="120" w:line="312" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:rFonts …/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr><w:t xml:space="preserve">---</w:t></w:r></w:p>
paras[122] = <w:p>…<w:t xml:space="preserve">报告边界</w:t>…<w:t xml:space="preserve">：本报告为公开信息汇总，不构成投资建议。…</w:t>…</w:p>
```

**C02（BRIEF §3.13 widowControl）**：BRIEF 把"正文无 `w:widowControl`"列为缺陷，**不成立**。`w:widowControl` 是 `CT_PPr` 的元素，**缺省即启用孤行控制**（省略元素 = 默认开启）；产物 `w:widowControl`=0 并不改变 Word 的孤行控制行为。真正成立的是：`heading_paragraph` L1643–1647 **只对 level≤2** 输出 `keepNext/keepLines`，5 个三级标题（3.1–3.5）没有，孤行标题风险只在三级标题上；另外表格缺 `cantSplit`。BRIEF 这条应改为"三级标题缺 keepNext/keepLines + 表格缺 cantSplit"。

**C03（BRIEF §4.A settings.xml 内容）**：BRIEF 把 `w:kinsoku`、`w:overflowPunct` 归入 `settings.xml`，**位置错误**：它们是 `CT_PPr` 的段落属性，正确位置是 `styles.xml` 的 `w:pPr` 或段落 `w:pPr`（本审计已放进 §7.4 的 BodyText）；只有 `w:compat`（`settings.xml`）、`w:characterSpacingControl`（`settings.xml`）与 `w:doNotExpandShiftReturn`（`settings.xml`）属于 settings。若按 BRIEF 写进 settings.xml，属于架构违规。

**C04（BRIEF §3.10 超链接）**：BRIEF 写"新闻报告里的来源全是纯文本，无法点击"，结论方向对，但证据口径需修正：产物 `document.xml` 里 `http` 只出现 1 次且是 `xmlns:w` 命名空间字符串，字面 `](` 出现 0 次——说明**输入 Markdown 根本没有链接语法**，不是"链接被降级成文本"。因此这条应表述为"生成器不具备链接能力（`parse_inline_runs` L1528–1560 无链接分支 + 无 rels part）"，而不是"来源链接被渲染成纯文本"。

**C05（BRIEF §1 的"123 段"）**：BRIEF 说"123 段"，这是 `document.xml` 里 `<w:p>` 的总数（含 59 个表格单元格内段落：4×6 + 5×4 + 3×5）；body 级段落实际 **64 个**（python-docx `len(d.paragraphs)`，lxml 按 body 直接子元素计数同值）。引用段数时建议注明口径，避免与"正文段落"混淆。

**C06（BRIEF §2.2 "`w:sectPr` 1"）**：写法正确但需注明：字符串计数 `w:sectPr`=2 是因为开/闭标签各一次，元素个数是 1，且位于 body 末尾；无 body 中间分节。

---

## 附录 A：取证命令与原始输出

### A.1 part 清单与大小

```
232 ->    429  [Content_Types].xml
172 ->    297  _rels/.rels
8659 -> 65825  word/document.xml
```
（左为压缩后、右为解压后字节数；与 BRIEF §2.1 一致）

### A.2 特征计数（`document.xml` 字符串计数）

```
w:pStyle 0        w:outlineLvl 0      w:numPr 0        w:rStyle 0
w:headerReference 0   w:footerReference 0   w:updateFields 0
w:tblHeader 3     w:tcW 0             w:cantSplit 0    w:tblStyle 0
w:hyperlink 0     w:firstLine 0       w:widowControl 0 w:keepNext 6  w:keepLines 6
w:pgNumType 0     w:pageBreakBefore 0 w:br 0           w:lang 0      w:noProof 0
w:pBdr 2（=1 个元素）  w:shd 13      w:gridCol 12     w:rPr 223
字面 "---" 1     字面 "• " 23        字面 "](“ 0     字面 "http" 1（仅 xmlns）
```

### A.3 表格几何

```
table0: cols=4 rows=6 gridCol=[2256,2256,2256,2256] sum=9024 tblW=9026
table1: cols=5 rows=4 gridCol=[1805,1805,1805,1805,1805] sum=9025 tblW=9026
table2: cols=3 rows=5 gridCol=[3008,3008,3008] sum=9024 tblW=9026
每个表 tblLayout=fixed、tblCellMar 左右 108、首行 trPr=<w:tblHeader/>
```

### A.4 `rPr` 顺序违规统计（223 个中 72 个违规）

```
151  (rFonts, sz, szCs)                          ← 顺序合法
 69  (rFonts, sz, szCs, b, bCs)                  ← b 应在 sz 之前
  2  (rFonts, sz, szCs, i, iCs, color)           ← color 应在 sz 之前
  1  (rFonts, sz, szCs, b, bCs, i, iCs, color)   ← b、color 都位置错误
```

### A.5 XML 良构性与第三方解析

```
lxml.etree.fromstring(word/document.xml) -> OK, root={http://schemas.openxmlformats.org/wordprocessingml/2006/main}document
python-docx Document(path) -> OK, paragraphs=64, tables=3
```

### A.6 参考 docx part 集（python-docx 默认模板，Word 2013 生成）

```
[Content_Types].xml / _rels/.rels / customXml/item1.xml / customXml/itemProps1.xml /
customXml/_rels/item1.xml.rels / docProps/app.xml / docProps/core.xml / docProps/thumbnail.jpeg /
word/_rels/document.xml.rels / word/document.xml / word/fontTable.xml / word/numbering.xml /
word/settings.xml / word/styles.xml / word/stylesWithEffects.xml / word/theme/theme1.xml /
word/webSettings.xml
```

---

## 附录 B：产物 `document.xml` 关键片段

### B.1 软换行被合并的段落（`paras[2]`，264 字）

```xml
<w:p><w:pPr><w:spacing w:after="120" w:line="312" w:lineRule="auto"/></w:pPr>
<w:r><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="微软雅黑"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">覆盖周期</w:t></w:r>
<w:r>…<w:t xml:space="preserve">：2026 年 11 月 1 日 — 11 月 14 日（以 11 月 12—13 日的一周为重点） </w:t></w:r>
<w:r>…<w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">成稿时间</w:t></w:r>
<w:r>…<w:t xml:space="preserve">：2026 年 11 月 14 日 </w:t></w:r>
<w:r>…<w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">信息性质</w:t></w:r>
<w:r>…<w:t xml:space="preserve">：本报告以官方公告、官方博客、权威媒体为主要依据；…明确标为「待核实」。 </w:t></w:r>
<w:r>…<w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">一句话结论</w:t></w:r>
<w:r>…<w:t xml:space="preserve">：这个月的 AI 行业同时踩了两脚油门——…连续出监管动作。</w:t></w:r>
</w:p>
```

### B.2 `---` 段落（`paras[121]`）

```xml
<w:p><w:pPr><w:spacing w:after="120" w:line="312" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="微软雅黑"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr><w:t xml:space="preserve">---</w:t></w:r></w:p>
```

### B.3 二级标题（`paras[1]`：只有字号+keep，无 pStyle/outline）

```xml
<w:p><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="280" w:after="140"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="微软雅黑"/><w:sz w:val="32"/><w:szCs w:val="32"/><w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">一、报告说明</w:t></w:r></w:p>
```

### B.4 假有序列表（`paras[37]`：序号是字面 run）

```xml
<w:p><w:pPr><w:spacing w:after="60"/><w:ind w:left="480" w:hanging="240"/></w:pPr><w:r>…<w:t xml:space="preserve">1. </w:t></w:r><w:r>…<w:t xml:space="preserve">11 月 17 日财报电话会上，临时 CEO 对 2027 年产能与订单可见度的表述；</w:t></w:r></w:p>
```

### B.5 引用块（`paras[40]`：灰色字色插在 rPr 末尾，顺序违规）

```xml
<w:r><w:rPr><w:rFonts …/><w:sz w:val="22"/><w:szCs w:val="22"/><w:i/><w:iCs/><w:color w:val="595959"/></w:rPr><w:t xml:space="preserve">说明：部分自媒体将辞职原因与个人传闻关联，…</w:t></w:r>
```

### B.6 表格属性（`table1`）

```xml
<w:tbl><w:tblPr><w:tblW w:w="9026" w:type="dxa"/><w:tblBorders>…</w:tblBorders><w:tblLayout w:type="fixed"/><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="1805"/>×5</w:tblGrid>…
<w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:tcPr><w:shd w:val="clear" w:fill="E8EEF9"/><w:vAlign w:val="center"/></w:tcPr><w:p><w:pPr><w:spacing w:before="40" w:after="40"/></w:pPr><w:r>…</w:r></w:p></w:tc>…</w:tr>
```

### B.7 `sectPr`（body 末尾，唯一）

```xml
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="851" w:footer="992" w:gutter="0"/></w:sectPr>
```
