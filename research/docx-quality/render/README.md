# render/ —— 结构校验与渲染尝试（结论：本机无法出图）

## 状态

**视觉渲染不可用（已放弃）**，改用**纯结构校验**兜底。所有 before/after 数字仍可复现。

## 为什么没有图

本机没有可用的 docx 渲染器：

| 方案 | 结果 |
| --- | --- |
| LibreOffice / `soffice` | `C:\Program Files\LibreOffice` 是残缺安装（只有 `program/logs`，无 `soffice.exe`），`no libreoffice` |
| DSH office sidecar（`dsoffice` / `@deepseek-ai/libreoffice-kit`） | **未安装**；本任务不申请下载授权 |
| Microsoft Word（`WINWORD.EXE`，COM 自动化 → PDF → PyMuPDF） | **半可用**：`Documents.Open` 被模态对话框阻塞，报 `'Word 未正确安装'`；`Application.Quit` 报 `'“Quit”不是一种属性'`；每次尝试需要 terminate 残留 WINWORD 进程。相关证据见 `logs/probe2_stdout.txt`、`logs/probe3.jsonl`、`logs/cleanup_word.json` |

结论：**分页观感、孤行/寡行、表格跨页断裂、目录页码实际渲染效果**这几项在本机无法验证，均标注为「需渲染」。

## 仍然可用的产出

`render_docx.py` 是结构校验脚本（Word COM 导出段为可选路径，失败即跳过），对每个文档产出：

- `out/before/structure.json` —— 桌面原文件的结构画像
- `out/after/structure.json` —— `out/gold.docx` 的结构画像
- `out/report/structure.json` —— 最终研究报告本体的结构画像

字段覆盖：部件清单与缺失项、`pStyle` 与命名样式使用分布、`styles.xml` 的 `outlineLvl`、`numPr`/`numbering.xml`、`hyperlink` 与外链关系、`sectPr`/`titlePg`/`pgNumType`/页眉页脚引用、`TOC` 与 `updateFields`、`docProps`、段落统计（超长段/字面量 `---`/`w:br`）、表格几何（`gridCol` 之和 vs `tblW`、`tcW`、`cantSplit`、`tblHeader`）、`rPr` 顺序违规计数。

## 复现

```bash
python research/docx-quality/render/render_docx.py \
  --input <docx> --output-dir research/docx-quality/render/out/<name>
```

## 关键结论（详见 ../REPORT.md 第七节）

| 指标 | before（桌面原文件） | after（gold.docx） |
| --- | --- | --- |
| part 数 | 3 | 14 |
| 缺失的目标 part | 9 个（styles/settings/numbering/header/footer/rels/docProps…） | 0 |
| `pStyle` | 0 | 129 |
| `styles.xml` 的 `outlineLvl` | 0 | 20（Heading1/2/3 = 0/1/2） |
| `numPr`（编号定义） | 0（无 numbering.xml） | 32（10 个编号定义） |
| `hyperlink` / 外链关系 | 0 / 0 | 4 / 4（`TargetMode="External"`） |
| `sectPr` / `titlePg` / `pgNumType start` | 1 / 无 / 无 | 2 / 有 / 1 |
| 页脚域 | 无 | `PAGE` + `NUMPAGES` |
| `TOC` 域 / `updateFields` | 无 / 无 | 有 / 有 |
| `docProps` | 无 | core + app |
| 表格 `gridCol` 之和 | 9024/9025/9024（≠9026） | 三张均为 9026 |
| `tcW` / `cantSplit` | 0 / 0 | 59 / 15 |
| 字面量 `---` | 1 处 | 0 |
| `rPr` 顺序违规 | 72/223 | 0/64 |
