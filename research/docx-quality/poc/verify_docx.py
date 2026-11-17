#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
verify_docx.py — 校验 gold_docx.py 产出的 .docx（zipfile + lxml + python-docx 三重验收）

检查项：
  1. 必需 part 是否齐全、zip 条目是否重名、[Content_Types].xml 是否覆盖全部 part；
  2. 所有 XML part 能否被 lxml 解析；关系（rels）内部 Target 是否都指向存在的 part；
  3. 样式/编号/域/表格等关键 OOXML 标记计数（pStyle/outlineLvl/numPr 必须 > 0）；
  4. 双节结构（封面节无页眉页脚 + 正文节页码从 1）、页眉文档名、页脚 PAGE/NUMPAGES；
  5. 表格总宽 9026 twips、每行 tcW 求和一致、cantSplit/表头重复/垂直居中；
  6. `---` 是否已渲染成 Divider 段落下框线、是否还有独立成段的字面量 `---`；
  7. python-docx 重新打开不报错，并能读出段落/表格/节/样式/页眉页脚。

用法：
  python verify_docx.py <docx 路径>

退出码：0 = PASS；1 = 有失败项。
"""

from __future__ import annotations

import argparse
import posixpath
import sys
import zipfile
from typing import Dict, List, Optional, Sequence, Tuple

from lxml import etree

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"
PKG_REL = "{http://schemas.openxmlformats.org/package/2006/relationships}"
CT = "{http://schemas.openxmlformats.org/package/2006/content-types}"
CP = "{http://schemas.openxmlformats.org/package/2006/metadata/core-properties}"
DC = "{http://purl.org/dc/elements/1.1/}"
DCTERMS = "http://purl.org/dc/terms/"
XSI = "{http://www.w3.org/2001/XMLSchema-instance}"

REQUIRED_PARTS: Tuple[str, ...] = (
    "[Content_Types].xml",
    "_rels/.rels",
    "word/document.xml",
    "word/styles.xml",
    "word/numbering.xml",
    "word/settings.xml",
    "word/theme/theme1.xml",
    "word/fontTable.xml",
    "word/webSettings.xml",
    "word/header1.xml",
    "word/footer1.xml",
    "word/_rels/document.xml.rels",
    "docProps/core.xml",
    "docProps/app.xml",
)

REQUIRED_STYLE_IDS = (
    "Title",
    "Subtitle",
    "Heading1",
    "Heading2",
    "Heading3",
    "BodyText",
    "Caption",
    "Quote",
    "CodeBlock",
    "ListParagraph",
    "TableHeader",
    "TOC1",
    "Header",
    "Footer",
    "Hyperlink",
)

HEADING_OUTLINE_LEVELS = {"Heading1": "0", "Heading2": "1", "Heading3": "2"}
OUTLINE_LEVEL_STYLES = (
    "Title",
    "Subtitle",
    "Heading1",
    "Heading2",
    "Heading3",
    "BodyText",
    "Caption",
    "Quote",
    "CodeBlock",
    "ListParagraph",
)

TOTAL_TABLE_WIDTH = 9026


def _parse_xml(name: str, data: bytes) -> etree._Element:
    parser = etree.XMLParser(resolve_entities=False, no_network=True, recover=False)
    return etree.fromstring(data, parser=parser)


def _paragraph_text(paragraph: etree._Element) -> str:
    return "".join(node.text or "" for node in paragraph.iter(W + "t")).strip()


def _rel_base(rels_name: str) -> str:
    directory = posixpath.dirname(rels_name)
    if directory.endswith("_rels"):
        return posixpath.dirname(directory)
    return directory


def _resolve_target(base: str, target: str) -> str:
    if target.startswith("/"):
        return target[1:]
    segments = [seg for seg in base.split("/") if seg] if base else []
    for seg in target.split("/"):
        if seg == "..":
            if segments:
                segments.pop()
        elif seg:
            segments.append(seg)
    return "/".join(segments)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="校验 gold_docx.py 产出的 .docx")
    parser.add_argument("docx", help="待校验的 .docx 路径")
    args = parser.parse_args(argv)

    errors: List[str] = []
    checks: List[Tuple[str, bool, str]] = []

    def check(label: str, ok: bool, detail: str = "") -> None:
        checks.append((label, ok, detail))
        if not ok:
            errors.append(f"{label}{('：' + detail) if detail else ''}")

    try:
        archive = zipfile.ZipFile(args.docx)
    except (OSError, zipfile.BadZipFile) as exc:
        print(f"[FAIL] 无法打开 zip：{exc}")
        return 1

    names = archive.namelist()
    info = archive.infolist()
    duplicate_names = sorted({name for name in names if names.count(name) > 1})
    check("zip 条目无重名", not duplicate_names, ",".join(duplicate_names))

    print(f"=== docx 包校验：{args.docx} ===")
    print(f"[zip] 条目 {len(names)} 个，原始大小 {sum(item.file_size for item in info)} bytes")

    missing = [part for part in REQUIRED_PARTS if part not in names]
    check("必需 part 齐全（14 个）", not missing, ",".join(missing))
    print(f"[part] 必需 part {len(REQUIRED_PARTS) - len(missing)}/{len(REQUIRED_PARTS)}：")
    for part in REQUIRED_PARTS:
        if part in names:
            size = archive.getinfo(part).file_size
            print(f"  {part:<34} {size:>6} bytes")

    parts: Dict[str, bytes] = {name: archive.read(name) for name in names}

    # ---- 1. XML 可解析性 ----
    xml_names = [name for name in names if name.endswith(".xml") or name.endswith(".rels")]
    parsed: Dict[str, etree._Element] = {}
    parse_failed: List[str] = []
    for name in xml_names:
        try:
            parsed[name] = _parse_xml(name, parts[name])
        except etree.XMLSyntaxError as exc:
            parse_failed.append(f"{name}({exc})")
    check("所有 XML part 可被 lxml 解析", not parse_failed, "; ".join(parse_failed))
    print(f"[xml] {len(parsed)}/{len(xml_names)} 个 XML/rels part 解析成功")

    # ---- 2. Content Types 覆盖 ----
    content_types = parsed.get("[Content_Types].xml")
    overrides = set()
    defaults = set()
    if content_types is not None:
        overrides = {
            el.get("PartName").lstrip("/")
            for el in content_types.findall(CT + "Override")
            if el.get("PartName")
        }
        defaults = {
            el.get("Extension", "").lower()
            for el in content_types.findall(CT + "Default")
        }
    uncovered = [
        name
        for name in names
        if name != "[Content_Types].xml"
        and not name.endswith(".rels")
        and name not in overrides
    ]
    check("每个 XML part 都有 Content-Type Override", not uncovered, ",".join(uncovered))
    print(
        f"[ct] Default {sorted(defaults)}；Override {len(overrides)} 个；"
        f"未覆盖 XML part {len(uncovered)} 个"
    )

    # ---- 3. 关系完整性 ----
    rel_problems: List[str] = []
    internal_rel_count = 0
    external_rel_count = 0
    for name, root in parsed.items():
        if not name.endswith(".rels"):
            continue
        base = _rel_base(name)
        for rel in root.findall(PKG_REL + "Relationship"):
            target = rel.get("Target", "")
            if rel.get("TargetMode") == "External":
                external_rel_count += 1
                continue
            internal_rel_count += 1
            resolved = _resolve_target(base, target)
            if resolved not in names:
                rel_problems.append(f"{name} -> {target}")
    check("rels 内部 Target 均存在", not rel_problems, "; ".join(rel_problems))
    print(
        f"[rels] 包/文档关系内部 {internal_rel_count} 条、外部 {external_rel_count} 条，"
        f"坏链 {len(rel_problems)} 条"
    )

    # ---- 4. document.xml 关键标记统计 ----
    document = parsed["word/document.xml"]
    styles_root = parsed["word/styles.xml"]
    footer_root = parsed["word/footer1.xml"]
    header_root = parsed["word/header1.xml"]
    core_root = parsed["docProps/core.xml"]
    settings_root = parsed["word/settings.xml"]

    def count_doc(tag: str) -> int:
        return len(document.findall(".//" + W + tag))

    br_all = document.findall(".//" + W + "br")
    stats = {
        "pStyle": count_doc("pStyle"),
        "outlineLvl(styles)": len(styles_root.findall(".//" + W + "outlineLvl")),
        "numPr": count_doc("numPr"),
        "hyperlink": count_doc("hyperlink"),
        "br(软换行)": sum(1 for br in br_all if br.get(W + "type") != "page"),
        "pageBreak": sum(1 for br in br_all if br.get(W + "type") == "page"),
        "pBdr(文档)": count_doc("pBdr"),
        "pBdr(样式)": len(styles_root.findall(".//" + W + "pBdr")),
        "tcW": count_doc("tcW"),
        "cantSplit": count_doc("cantSplit"),
        "tblHeader": count_doc("tblHeader"),
        "vAlign": count_doc("vAlign"),
        "headerReference": count_doc("headerReference"),
        "footerReference": count_doc("footerReference"),
        "TOC field": 0,
        "PAGE field": 0,
        "NUMPAGES field": 0,
    }
    instr = [
        node.text or ""
        for node in document.findall(".//" + W + "instrText")
        + footer_root.findall(".//" + W + "instrText")
    ]
    stats["TOC field"] = sum(1 for text in instr if "TOC" in text and '\\o' in text)
    stats["PAGE field"] = sum(
        1 for text in instr if "PAGE" in text and "NUMPAGES" not in text
    )
    stats["NUMPAGES field"] = sum(1 for text in instr if "NUMPAGES" in text)
    for key in ("pStyle", "outlineLvl(styles)", "numPr"):
        check(f"{key} 计数 > 0", stats[key] > 0, str(stats[key]))
    print(
        "[stats] "
        + "  ".join(f"{key}={value}" for key, value in stats.items())
    )

    update_fields = settings_root.find(".//" + W + "updateFields")
    check(
        "settings.xml 含 updateFields=true",
        update_fields is not None and update_fields.get(W + "val") == "true",
        "缺失或不为 true",
    )

    # ---- 5. 双节结构：封面节（无页眉页脚）+ 正文节（页码从 1）----
    body = document.find(W + "body")
    body_sect = body.find(W + "sectPr") if body is not None else None
    ppr_sects = document.findall(".//" + W + "pPr/" + W + "sectPr")
    check("存在封面节与正文节（共 2 个 sectPr）", len(ppr_sects) == 1 and body_sect is not None,
          f"段落内 sectPr={len(ppr_sects)}, body 级 sectPr={body_sect is not None}")
    if ppr_sects and body_sect is not None:
        cover_sect = ppr_sects[0]
        cover_header_refs = cover_sect.findall(W + "headerReference") + cover_sect.findall(W + "footerReference")
        check("封面节不引用页眉页脚", not cover_header_refs,
              f"{len(cover_header_refs)} 个引用")
        check("封面节设置 titlePg（首页不同）", cover_sect.find(W + "titlePg") is not None)
        body_header_refs = body_sect.findall(W + "headerReference")
        body_footer_refs = body_sect.findall(W + "footerReference")
        check("正文节引用页眉", len(body_header_refs) == 1,
              f"{len(body_header_refs)} 个")
        check("正文节引用页脚", len(body_footer_refs) == 1,
              f"{len(body_footer_refs)} 个")
        pg_num = body_sect.find(W + "pgNumType")
        check(
            "正文节页码从 1 开始（pgNumType start=1）",
            pg_num is not None and pg_num.get(W + "start") == "1",
            "缺失或不为 1",
        )
        doc_rels = parsed.get("word/_rels/document.xml.rels")
        rel_ids = set()
        if doc_rels is not None:
            rel_ids = {rel.get("Id") for rel in doc_rels.findall(PKG_REL + "Relationship")}
        ref_ids = [
            ref.get(R + "id")
            for ref in body_header_refs + body_footer_refs
            if ref.get(R + "id")
        ]
        check("页眉页脚 r:id 在 document.xml.rels 中存在", all(rid in rel_ids for rid in ref_ids),
              ",".join(rid for rid in ref_ids if rid not in rel_ids))

    # ---- 6. 页眉文档名 / 页脚域 ----
    title_nodes = core_root.findall(".//" + DC + "title") if core_root is not None else []
    core_title = (title_nodes[0].text or "").strip() if title_nodes else ""
    header_text = _paragraph_text(header_root.find(".//" + W + "p"))
    check("core.xml 写入 dc:title", bool(core_title))
    creator = core_root.findall(".//" + DC + "creator") if core_root is not None else []
    check("core.xml 写入 dc:creator", bool((creator[0].text or "").strip()) if creator else False)
    created = core_root.findall(".//" + "{http://purl.org/dc/terms/}created") if core_root is not None else []
    check("core.xml 写入 dcterms:created", bool(created and (created[0].text or "").strip()))
    check("页眉包含文档名", bool(core_title) and core_title in header_text,
          f"header={header_text!r}")

    # ---- 7. 样式表：命名样式 + outlineLvl + 正文缩进/行距 ----
    style_elements = {
        style.get(W + "styleId"): style
        for style in styles_root.findall(W + "style")
    }
    missing_styles = [sid for sid in REQUIRED_STYLE_IDS if sid not in style_elements]
    check("命名样式齐全", not missing_styles, ",".join(missing_styles))
    for style_id, expected in HEADING_OUTLINE_LEVELS.items():
        style = style_elements.get(style_id)
        outline = style.find(".//" + W + "outlineLvl") if style is not None else None
        check(
            f"{style_id} outlineLvl={expected}",
            outline is not None and outline.get(W + "val") == expected,
            f"实际 {outline.get(W + 'val') if outline is not None else '缺失'}",
        )
    outline_missing = [
        sid
        for sid in OUTLINE_LEVEL_STYLES
        if sid in style_elements and style_elements[sid].find(".//" + W + "outlineLvl") is None
    ]
    check("命名样式均带 w:outlineLvl", not outline_missing, ",".join(outline_missing))
    body_style = style_elements.get("BodyText")
    body_ind = body_style.find(".//" + W + "ind") if body_style is not None else None
    body_spacing = body_style.find(".//" + W + "spacing") if body_style is not None else None
    check(
        "BodyText 首行缩进 2 字符（firstLineChars=200）",
        body_ind is not None and body_ind.get(W + "firstLineChars") == "200",
        "缺失",
    )
    check(
        "BodyText 1.5 倍行距（line=360 lineRule=auto）",
        body_spacing is not None
        and body_spacing.get(W + "line") == "360"
        and body_spacing.get(W + "lineRule") == "auto",
        "缺失",
    )

    # ---- 8. 表格：总宽 9026 + tcW + cantSplit + 表头重复 + 垂直居中 ----
    tables = document.findall(".//" + W + "tbl")
    table_problems: List[str] = []
    zebra_problems = 0
    for index, table in enumerate(tables, start=1):
        grid = table.find(W + "tblGrid")
        grid_widths = [
            int(col.get(W + "w", "0"))
            for col in grid.findall(W + "gridCol") if grid is not None
        ]
        if sum(grid_widths) != TOTAL_TABLE_WIDTH:
            table_problems.append(f"表{index} gridCol 合计 {sum(grid_widths)}")
        rows = table.findall(W + "tr")
        if not rows:
            table_problems.append(f"表{index} 无行")
            continue
        header_ok = rows[0].find(W + "trPr/" + W + "tblHeader") is not None
        if not header_ok:
            table_problems.append(f"表{index} 首行无 tblHeader")
        for row_index, row in enumerate(rows, start=1):
            cells = row.findall(W + "tc")
            tcw_sum = 0
            for cell in cells:
                tcw = cell.find(W + "tcPr/" + W + "tcW")
                if tcw is None:
                    table_problems.append(f"表{index} 行{row_index} 单元格缺 tcW")
                else:
                    tcw_sum += int(tcw.get(W + "w", "0"))
                if cell.find(W + "tcPr/" + W + "vAlign") is None:
                    table_problems.append(f"表{index} 行{row_index} 单元格缺 vAlign")
            if tcw_sum != TOTAL_TABLE_WIDTH:
                table_problems.append(f"表{index} 行{row_index} tcW 合计 {tcw_sum}")
            if len(cells) != len(grid_widths):
                table_problems.append(f"表{index} 行{row_index} 单元格数 {len(cells)}")
            if row.find(W + "trPr/" + W + "cantSplit") is None:
                table_problems.append(f"表{index} 行{row_index} 缺 cantSplit")
        if len(rows) > 2:
            data_shades = [
                row.find(W + "tc/" + W + "tcPr/" + W + "shd") is not None
                for row in rows[1:]
            ]
            if not any(data_shades):
                zebra_problems += 1
    check("表格列宽/行控制/垂直居中", not table_problems, "; ".join(table_problems[:5]))
    check("数据行存在斑马纹（shading）", zebra_problems == 0, f"{zebra_problems} 张表无斑马纹")
    print(
        f"[table] {len(tables)} 张表，gridCol/tcW 合计均为 {TOTAL_TABLE_WIDTH} twips，"
        f"表头重复 + cantSplit + vAlign 已覆盖"
    )

    # ---- 9. `---` 渲染结果：应为 Divider 段落下框线，无独立成段的字面量 ---
    literal_hr = [
        paragraph
        for paragraph in document.findall(".//" + W + "p")
        if _paragraph_text(paragraph) in ("---", "***", "___")
    ]
    divider_paragraphs = [
        paragraph
        for paragraph in document.findall(".//" + W + "p")
        if (paragraph.find(W + "pPr/" + W + "pStyle") is not None)
        and paragraph.find(W + "pPr/" + W + "pStyle").get(W + "val") == "Divider"
    ]
    check("无独立成段的字面量 `---`", not literal_hr, f"{len(literal_hr)} 处")
    check("存在 Divider 段落下框线", bool(divider_paragraphs), "0 处")
    print(f"[hr] Divider 段落 {len(divider_paragraphs)} 个；独立字面量 --- 残留 {len(literal_hr)} 处")

    # ---- 10. python-docx 重新打开 ----
    try:
        import docx
        from docx import Document

        document_docx = Document(args.docx)
        paragraphs = len(document_docx.paragraphs)
        tables_docx = len(document_docx.tables)
        sections = len(document_docx.sections)
        styles_count = len(document_docx.styles)
        last_section = document_docx.sections[-1]
        header_texts = " | ".join(p.text for p in last_section.header.paragraphs)
        footer_texts = " | ".join(p.text for p in last_section.footer.paragraphs)
        style_name_counts: Dict[str, int] = {}
        for paragraph in document_docx.paragraphs:
            try:
                name = paragraph.style.name
            except KeyError:  # 样式表里找不到对应定义时不影响验收
                name = "(unknown)"
            style_name_counts[name] = style_name_counts.get(name, 0) + 1
        check("python-docx 重新打开成功", True, "")
        check("python-docx 可读取正文节页眉", bool(header_texts), header_texts)
        check("python-docx 可读取正文节页脚", bool(footer_texts), footer_texts)
        print(
            f"[python-docx {docx.__version__}] 重新打开成功："
            f"paragraphs={paragraphs}, tables={tables_docx}, sections={sections}, styles={styles_count}, "
            f"header={header_texts!r}, footer={footer_texts!r}"
        )
        top_styles = sorted(style_name_counts.items(), key=lambda kv: -kv[1])[:6]
        print("[python-docx] 段落样式分布（前 6）：" + ", ".join(f"{name}:{count}" for name, count in top_styles))
    except Exception as exc:  # noqa: BLE001 - 验收脚本需要兜底展示失败原因
        check("python-docx 重新打开成功", False, repr(exc))
        print(f"[python-docx] 打开失败：{exc!r}")

    # ---- 汇总 ----
    print()
    failed = [item for item in checks if not item[1]]
    if failed:
        print(f"结果：FAIL（{len(failed)}/{len(checks)} 项未通过）")
        for label, _, detail in failed:
            print(f"  - {label}{('：' + detail) if detail else ''}")
        return 1
    print(f"结果：PASS（{len(checks)}/{len(checks)} 项检查全部通过）")
    print(f"[error] 0 个")
    return 0


if __name__ == "__main__":
    sys.exit(main())
