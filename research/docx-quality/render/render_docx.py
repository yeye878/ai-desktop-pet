#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""render_docx.py —— docx 渲染 + 兼容性/结构校验（research/docx-quality/render 交付脚本）

两种能力：
  1) 结构校验（纯 Python / zipfile + ElementTree，任何环境都能跑）：
     部件清单、命名样式、大纲级别、真编号、分节/封面、页眉页脚与 PAGE/NUMPAGES 域、
     TOC 域、settings updateFields、docProps、表格列宽、rPr 子元素顺序、字面量 "---"。
  2) 渲染出图（本机 Word COM → PDF → PyMuPDF → PNG + contact_sheet）：
     只有在 Microsoft Word COM 可用时才会产出 pdf/png；失败会记录原因并自动回退结构校验。

用法（在本目录下）：
  python render_docx.py --preset before                 # 桌面原文件（只读，不改）
  python render_docx.py --preset after                  # ../out/gold.docx
  python render_docx.py --preset report                 # ../out/研究报告_....docx
  python render_docx.py --source path/to/x.docx --label x
  python render_docx.py --preset before --mode structure   # 强制只做结构校验
  python render_docx.py --preset after --mode auto        # 先试 Word，失败自动回退
  python render_docx.py --compare                       # 用已有 before/after PNG 拼并排图

模式：
  structure  纯结构校验（默认回退，秒级）
  word       仅 Word COM 渲染（≤90s 超时；失败退出码 2）
  auto       先 word，失败回退 structure（默认）

输出（out/<label>/）：
  structure.json       结构校验明细
  manifest.json        本次运行的输入/步骤/产物清单
  word_attempt.json    Word COM 尝试的阶段性证据（模态框文本、COM 报错等）
  before.pdf / pages/*.png / contact_sheet.png   ← 仅 Word 可用时
以及 out/compare/contact_sheet_before_after.png（--compare 且两侧 PNG 都存在时）
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
import zipfile

import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, ".."))          # research/docx-quality
OUT = os.path.join(HERE, "out")
LOGDIR = os.path.join(HERE, "logs")

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
REL = "http://schemas.openxmlformats.org/package/2006/relationships"
CP = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"

PRESETS = {
    # label: (source_path, output_label)
    "before": (r"C:\Users\15188\Desktop\AI领域新闻报告汇总_2026年11月.docx", "before"),
    "after": (os.path.join(ROOT, "out", "gold.docx"), "after"),
    "report": (os.path.join(ROOT, "out", "研究报告_直连API生成高质量Word能力提升.docx"), "report"),
}

# 目标态（专业报告）必需的可选 part；before 只写了 3 个 part
TARGET_PARTS = [
    "word/_rels/document.xml.rels",
    "word/styles.xml",
    "word/settings.xml",
    "word/theme/theme1.xml",
    "word/fontTable.xml",
    "word/webSettings.xml",
    "word/numbering.xml",
    "docProps/core.xml",
    "docProps/app.xml",
]
CORE_PARTS = ["[Content_Types].xml", "_rels/.rels", "word/document.xml"]

# ECMA-376 CT_RPr 子元素顺序（用于 rPr 顺序违规检查）
RPR_ORDER = [
    "rStyle", "rFonts", "b", "bCs", "i", "iCs", "caps", "smallCaps", "strike",
    "dstrike", "outline", "shadow", "emboss", "imprint", "noProof", "snapToGrid",
    "vanish", "webHidden", "color", "spacing", "w", "kern", "position", "sz",
    "szCs", "highlight", "u", "effect", "bdr", "shd", "fitText", "vertAlign",
    "rtl", "cs", "em", "lang", "eastAsianLayout", "specVanish", "oMath",
]


# ---------------------------------------------------------------- 小工具
def localname(tag):
    return tag.rsplit("}", 1)[-1]


def wattr(el, name):
    return el.get("{%s}%s" % (W, name))


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def read_part(z, name):
    try:
        return z.read(name)
    except KeyError:
        return None


def parse_xml(z, name):
    data = read_part(z, name)
    if data is None:
        return None
    try:
        return ET.fromstring(data)
    except Exception as exc:  # noqa: BLE001
        return {"_parse_error": repr(exc)}


# ---------------------------------------------------------------- 结构校验
def rpr_order_violations(root, part):
    """返回 rPr 子元素顺序违规明细（ECMA-376 CT_RPr 顺序），全量统计。"""
    bad = []
    seen = 0
    if not isinstance(root, ET.Element):
        return bad, 0
    idx = {name: i for i, name in enumerate(RPR_ORDER)}
    for rpr in root.iter("{%s}rPr" % W):
        seen += 1
        kids = [localname(c.tag) for c in list(rpr)]
        known = [(idx[k], k) for k in kids if k in idx]
        for a in range(len(known) - 1):
            if known[a][0] > known[a + 1][0]:
                bad.append({"part": part, "sequence": kids,
                            "expect_after": known[a][1],
                            "found_before": known[a + 1][1]})
                break
    return bad, seen


def analyze_structure(source, outdir):
    """静态结构校验。返回 (report_dict, checks_list)。"""
    rep = {
        "source": source,
        "file_name": os.path.basename(source),
        "size": os.path.getsize(source),
        "sha256": sha256(source),
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    checks = []

    def add(name, status, detail, category):
        checks.append({"check": name, "status": status, "detail": detail,
                       "category": category})

    with zipfile.ZipFile(source) as z:
        parts = sorted(z.namelist())
        rep["parts"] = {
            "count": len(parts),
            "list": parts,
            "core_present": all(p in parts for p in CORE_PARTS),
            "core_missing": [p for p in CORE_PARTS if p not in parts],
            "target_missing": [p for p in TARGET_PARTS if p not in parts],
            "header_parts": [p for p in parts if p.startswith("word/header")],
            "footer_parts": [p for p in parts if p.startswith("word/footer")],
            "media_parts": [p for p in parts if p.startswith("word/media/")],
        }

        doc = parse_xml(z, "word/document.xml")
        styles = parse_xml(z, "word/styles.xml")
        settings = parse_xml(z, "word/settings.xml")
        numbering = parse_xml(z, "word/numbering.xml")
        doc_rels = parse_xml(z, "word/_rels/document.xml.rels")
        core = parse_xml(z, "docProps/core.xml")
        app = parse_xml(z, "docProps/app.xml")
        footers = {p: parse_xml(z, p) for p in rep["parts"]["footer_parts"]}
        headers = {p: parse_xml(z, p) for p in rep["parts"]["header_parts"]}
        content_types = parse_xml(z, "[Content_Types].xml")

    doc_ok = isinstance(doc, ET.Element)
    if not doc_ok:
        rep["document_xml_error"] = doc.get("_parse_error") if isinstance(doc, dict) else "missing"
        add("document.xml 可解析", "FAIL", str(rep["document_xml_error"]), "包结构")
        ET.Element("body")

    paragraphs = [el for el in doc.iter("{%s}p" % W)]
    body = None
    for el in doc:
        if localname(el.tag) == "body":
            body = el
            break
    body_paras = [c for c in (list(body) if body is not None else []) if localname(c.tag) == "p"]
    tables = [el for el in doc.iter("{%s}tbl" % W)]

    # ---- 段落/样式/大纲 ----
    pstyle_ids = []
    for p in paragraphs:
        ppr = p.find("{%s}pPr" % W)
        if ppr is not None:
            ps = ppr.find("{%s}pStyle" % W)
            if ps is not None:
                pstyle_ids.append(wattr(ps, "val") or "")
    named_style_use = {}
    for sid in pstyle_ids:
        named_style_use[sid] = named_style_use.get(sid, 0) + 1

    style_defs = {}
    style_outline = {}
    style_body_text = {}
    if isinstance(styles, ET.Element):
        for st in styles.iter("{%s}style" % W):
            sid = st.get("{%s}styleId" % W)
            stype = st.get("{%s}type" % W)
            style_defs[sid] = stype
            # outlineLvl 挂在 style 的 w:pPr 下（不是 w:style 的直接子元素）
            for ch in st.iter("{%s}outlineLvl" % W):
                v = wattr(ch, "val")
                try:
                    style_outline[sid] = int(v)
                except Exception:
                    style_outline[sid] = v
            if sid in ("BodyText", "Normal"):
                info = {}
                ppr = st.find("{%s}pPr" % W)
                if ppr is not None:
                    wi = ppr.find("{%s}ind" % W)
                    if wi is not None:
                        info["ind"] = {localname(k): v for k, v in wi.attrib.items()}
                    wsp = ppr.find("{%s}spacing" % W)
                    if wsp is not None:
                        info["spacing"] = {localname(k): v for k, v in wsp.attrib.items()}
                style_body_text[sid] = info
    # 只保留与标题/目录相关的样式大纲级别，避免 latent 样式噪音
    outline_defined_styles = {sid: lvl for sid, lvl in style_outline.items()
                              if str(sid).lower().startswith(("heading", "toc"))}
    heading_styles = {sid: lvl for sid, lvl in style_outline.items()
                      if str(sid).lower().startswith("heading") or sid in ("1", "2", "3")}
    outline_doc = sum(1 for el in doc.iter("{%s}outlineLvl" % W)) if doc_ok else 0
    doc_heading_by_outline = {sid: 0 for sid in heading_styles}
    for p in paragraphs:
        ppr = p.find("{%s}pPr" % W)
        if ppr is None:
            continue
        ps = ppr.find("{%s}pStyle" % W)
        if ps is not None:
            sid = wattr(ps, "val")
            if sid in doc_heading_by_outline:
                doc_heading_by_outline[sid] += 1

    numpr = sum(1 for el in doc.iter("{%s}numPr" % W)) if doc_ok else 0
    numbering_nums = sum(1 for el in numbering.iter("{%s}num" % W)) if (numbering is not None and isinstance(numbering, ET.Element)) else 0

    hyperlinks = sum(1 for el in doc.iter("{%s}hyperlink" % W)) if doc_ok else 0
    external_rels = 0
    all_rels = []
    if isinstance(doc_rels, ET.Element):
        for rel in doc_rels.iter("{%s}Relationship" % REL):
            item = {"Id": rel.get("Id"), "Type": rel.get("Type"),
                    "Target": rel.get("Target"), "TargetMode": rel.get("TargetMode")}
            all_rels.append(item)
            if item["TargetMode"] == "External":
                external_rels += 1

    br_count = sum(1 for el in doc.iter("{%s}br" % W)) if doc_ok else 0

    # ---- 分节 / 封面 / 页码 ----
    sectprs = [el for el in doc.iter("{%s}sectPr" % W)] if doc_ok else []
    sect_detail = []
    titlepg = 0
    pgstart = []
    header_refs = 0
    footer_refs = 0
    content_width_twips = None  # 版心宽度 = pgSz.w - 左右页边距，用于表格列宽合计校验
    for sp in sectprs:
        kids = [localname(c.tag) for c in list(sp)]
        d = {"children": kids}
        pgsz = sp.find("{%s}pgSz" % W)
        pgmar = sp.find("{%s}pgMar" % W)
        if pgsz is not None and pgmar is not None:
            try:
                content_width_twips = (int(wattr(pgsz, "w")) - int(wattr(pgmar, "left"))
                                       - int(wattr(pgmar, "right")))
                d["content_width_twips"] = content_width_twips
            except Exception:
                pass
        tp = sp.find("{%s}titlePg" % W)
        if tp is not None:
            val = (wattr(tp, "val") or "").lower()
            if val in ("", "true", "1", "on"):
                titlepg += 1
                d["titlePg"] = True
        pg = sp.find("{%s}pgNumType" % W)
        if pg is not None:
            start = wattr(pg, "start")
            d["pgNumType"] = {"start": start, "fmt": wattr(pg, "fmt")}
            pgstart.append(start)
        for ch in sp:
            ln = localname(ch.tag)
            if ln == "headerReference":
                header_refs += 1
            elif ln == "footerReference":
                footer_refs += 1
        sect_detail.append(d)

    # ---- 域：页眉页脚 PAGE / NUMPAGES、正文 TOC ----
    def instr_texts(root):
        out = []
        if not isinstance(root, ET.Element):
            return out
        for el in root.iter():
            ln = localname(el.tag)
            if ln == "instrText" and el.text:
                out.append(el.text)
            elif ln == "fldSimple":
                instr = wattr(el, "instr")
                if instr:
                    out.append(instr)
        return out

    doc_instr = instr_texts(doc)
    toc_instr = [s.strip() for s in doc_instr if "TOC" in s.upper()]
    footer_instr = {}
    page_fields = 0
    numpages_fields = 0
    for name, root in list(footers.items()) + list(headers.items()):
        texts = instr_texts(root)
        footer_instr[name] = [t.strip() for t in texts]
        for t in texts:
            if re.search(r"\bPAGE\b", t.upper()):
                page_fields += 1
            if re.search(r"\bNUMPAGES\b", t.upper()):
                numpages_fields += 1
    footer_text_sample = {}
    for name, root in footers.items():
        if isinstance(root, ET.Element):
            txt = "".join(t for t in root.itertext())
            footer_text_sample[name] = txt.strip()[:80]

    # ---- settings ----
    update_fields = None
    compat = None
    kinsoku = overflow_punct = None
    if isinstance(settings, ET.Element):
        for el in settings.iter("{%s}updateFields" % W):
            update_fields = wattr(el, "val")
        compat = any(localname(c.tag) == "compat" for c in list(settings))
        kinsoku = any(localname(c.tag) == "kinsoku" for c in list(settings))
        overflow_punct = any(localname(c.tag) == "overflowPunct" for c in list(settings))

    # ---- docProps ----
    core_vals = {}
    app_vals = {}
    if isinstance(core, ET.Element):
        for el in core.iter():
            ln = localname(el.tag)
            if el.text and ln in ("title", "subject", "creator", "lastModifiedBy",
                                  "created", "modified", "description", "keywords"):
                core_vals.setdefault(ln, el.text.strip())
    if isinstance(app, ET.Element):
        for el in app.iter():
            ln = localname(el.tag)
            if el.text and ln in ("Application", "Company", "Pages", "Words", "Paragraphs"):
                app_vals.setdefault(ln, el.text.strip())

    # ---- 表格 ----
    tables_info = []
    for i, tb in enumerate(tables, 1):
        grid = tb.find("{%s}tblGrid" % W)
        cols = []
        if grid is not None:
            cols = [wattr(c, "w") for c in list(grid) if localname(c.tag) == "gridCol"]
        try:
            grid_sum = sum(int(c) for c in cols if c)
        except Exception:
            grid_sum = None
        tw = tb.find("{%s}tblW" % W)
        tbl_w = wattr(tw, "w") if tw is not None else None
        tbl_type = wattr(tw, "type") if tw is not None else None
        try:
            tbl_w_i = int(tbl_w) if tbl_w is not None else None
        except Exception:
            tbl_w_i = None
        rows = sum(1 for r in list(tb) if localname(r.tag) == "tr")
        tcw = sum(1 for el in tb.iter("{%s}tcW" % W))
        cant = sum(1 for el in tb.iter("{%s}cantSplit" % W))
        thdr = sum(1 for el in tb.iter("{%s}tblHeader" % W))
        width_match = None
        if grid_sum is not None and tbl_w_i is not None and tbl_type in (None, "dxa"):
            width_match = (grid_sum == tbl_w_i)
        elif grid_sum is not None and tbl_w_i is None and content_width_twips:
            # 没有 w:tblW（Word 视为 auto/100%）时，用版心宽度作参照
            width_match = (grid_sum == content_width_twips)
        tables_info.append({"index": i, "gridCols": len(cols), "gridCol_sum": grid_sum,
                            "tblW": tbl_w_i, "tblW_type": tbl_type,
                            "reference_width": tbl_w_i if tbl_w_i is not None else content_width_twips,
                            "gridCol_equals_tblW": width_match, "rows": rows,
                            "tcW_count": tcw, "cantSplit_count": cant,
                            "tblHeader_count": thdr})
    width_bad = [t for t in tables_info if t["gridCol_equals_tblW"] is False]
    width_good = [t for t in tables_info if t["gridCol_equals_tblW"] is True]

    # ---- 段落文本度量（软换行合并 / 长段） ----
    def para_text(p):
        return "".join(t.text or "" for t in p.iter("{%s}t" % W))

    body_texts = [para_text(p) for p in body_paras]
    all_texts = [para_text(p) for p in paragraphs]
    longest = max(all_texts, key=len) if all_texts else ""
    long_paras = [t for t in all_texts if len(t) >= 200]
    # 字面量 "---"：区分「独立成段的分隔线垃圾」与「正文/表格里的正常引用」
    dash_nodes = 0
    standalone_dash_paras = []
    if doc_ok:
        for el in doc.iter("{%s}t" % W):
            if (el.text or "").strip() == "---":
                dash_nodes += 1
        for p in paragraphs:
            if para_text(p).strip() == "---":
                standalone_dash_paras.append(para_text(p)[:40])
    # 正文段落的首行缩进 / 行距（直接格式）
    body_indent_direct = 0
    body_spacing_line = {}
    for p in body_paras:
        ppr = p.find("{%s}pPr" % W)
        if ppr is None:
            continue
        wi = ppr.find("{%s}ind" % W)
        if wi is not None and (wattr(wi, "firstLine") or wattr(wi, "firstLineChars")):
            body_indent_direct += 1
        wsp = ppr.find("{%s}spacing" % W)
        if wsp is not None:
            key = "%s/%s" % (wattr(wsp, "line"), wattr(wsp, "lineRule"))
            body_spacing_line[key] = body_spacing_line.get(key, 0) + 1

    rpr_bad_doc, rpr_seen_doc = (rpr_order_violations(doc, "word/document.xml") if doc_ok else ([], 0))
    rpr_bad_st, rpr_seen_st = (rpr_order_violations(styles, "word/styles.xml")
                               if isinstance(styles, ET.Element) else ([], 0))
    rpr_bad = rpr_bad_doc + rpr_bad_st

    # ---- python-docx 可打开性 ----
    py_docx = {"available": False}
    try:
        import docx  # noqa: F401
        d = docx.Document(source)
        py_docx = {"available": True, "ok": True,
                   "paragraphs": len(d.paragraphs), "tables": len(d.tables),
                   "sections": len(d.sections)}
    except Exception as exc:  # noqa: BLE001
        py_docx = {"available": True, "ok": False, "error": repr(exc)[:300]}

    rep.update({
        "package": {
            "parts_count": rep["parts"]["count"],
        },
        "styles": {
            "pstyle_count": len(pstyle_ids),
            "named_style_use": named_style_use,
            "styles_defined_count": len(style_defs),
            "styles_xml_outlineLvl_count": len(style_outline),
            "styles_outline_levels": dict(sorted(style_outline.items())),
            "outline_defined_styles": outline_defined_styles,
            "heading_styles": heading_styles,
            "document_outlineLvl_count": outline_doc,
            "heading_paragraphs_by_style": doc_heading_by_outline,
            "style_body_text": style_body_text,
        },
        "numbering": {"numPr_count": numpr, "numbering_part": "word/numbering.xml" in rep["parts"]["list"],
                      "numbering_defs": numbering_nums},
        "hyperlinks": {"hyperlink_elements": hyperlinks,
                       "external_rels": external_rels,
                       "document_rels": [r for r in all_rels if r["Type"] and "hyperlink" in r["Type"]]},
        "layout": {
            "sectPr_count": len(sectprs),
            "sections": sect_detail,
            "content_width_twips": content_width_twips,
            "titlePg_count": titlepg,
            "pgNumType_starts": pgstart,
            "header_reference_count": header_refs,
            "footer_reference_count": footer_refs,
            "footer_parts": list(footers.keys()),
            "header_parts": list(headers.keys()),
            "footer_field_instr": footer_instr,
            "footer_text_sample": footer_text_sample,
            "page_fields": page_fields,
            "numpages_fields": numpages_fields,
        },
        "fields": {
            "toc_instr": toc_instr,
            "toc_field_present": bool(toc_instr),
            "updateFields": update_fields,
            "updateFields_value": update_fields,
            "compat_block": compat,
            "kinsoku": kinsoku,
            "overflowPunct": overflow_punct,
        },
        "docprops": {"core": core_vals, "app": app_vals,
                     "core_part": "docProps/core.xml" in rep["parts"]["list"],
                     "app_part": "docProps/app.xml" in rep["parts"]["list"]},
        "paragraphs": {
            "count_all_w_p": len(paragraphs),
            "count_body_level": len(body_paras),
            "count_in_tables": len(paragraphs) - len(body_paras),
            "longest_text_chars": len(longest),
            "longest_text_head": longest[:80],
            "paragraphs_ge_200_chars": len(long_paras),
            "long_paragraph_heads": [t[:60] for t in long_paras[:5]],
            "literal_dash_paragraph_count": len(standalone_dash_paras),
            "literal_dash_sample": standalone_dash_paras[0] if standalone_dash_paras else None,
            "literal_dash_wt_nodes": dash_nodes,
            "body_indent_direct_count": body_indent_direct,
            "body_spacing_line": body_spacing_line,
            "br_count": br_count,
        },
        "tables": {"count": len(tables_info),
                   "grid_equals_tblW_pass": len(width_good),
                   "grid_equals_tblW_fail": len(width_bad),
                   "detail": tables_info},
        "rpr_order": {"elements_checked": rpr_seen_doc + rpr_seen_st,
                      "violations": len(rpr_bad),
                      "samples": rpr_bad[:10]},
        "python_docx": py_docx,
    })

    # ---- 目标态检查（静态可判定的部分） ----
    pkg = rep["parts"]
    add("包结构：3 个核心 part 齐全", "PASS" if pkg["core_present"] else "FAIL",
        "缺: %s" % (pkg["core_missing"] or "无"), "包结构")
    add("包结构：目标态可选 part 齐全（styles/settings/theme/fontTable/webSettings/numbering/rels/docProps/header-footer）",
        "PASS" if not pkg["target_missing"] and pkg["header_parts"] and pkg["footer_parts"] else "FAIL",
        "缺: %s；header=%d footer=%d" % (pkg["target_missing"] or "无",
                                        len(pkg["header_parts"]), len(pkg["footer_parts"])),
        "包结构")
    add("命名样式（w:pStyle 引用）", "PASS" if len(pstyle_ids) > 0 else "FAIL",
        "pStyle=%d, 使用样式=%d 种, styles.xml=%s" % (
            len(pstyle_ids), len(named_style_use), "有" if isinstance(styles, ET.Element) else "无"),
        "排版系统")
    add("大纲级别（styles.xml 中 Heading outlineLvl）",
        "PASS" if len(style_outline) > 0 else "FAIL",
        "styles.xml outlineLvl=%d, 段落级 outlineLvl=%d, 标题样式=%s" % (
            len(style_outline), outline_doc, heading_styles or "无"), "排版系统")
    add("真编号列表（w:numPr + numbering.xml）",
        "PASS" if numpr > 0 and "word/numbering.xml" in pkg["list"] else
        ("WARN" if numpr == 0 else "FAIL"),
        "numPr=%d, numbering.xml=%s（=0 表示列表是手写文本前缀）" % (
            numpr, "有" if "word/numbering.xml" in pkg["list"] else "无"), "内容元素")
    add("页眉页脚 + PAGE/NUMPAGES 域",
        "PASS" if (footer_instr and page_fields and numpages_fields) else "FAIL",
        "footer=%s, PAGE 域=%d, NUMPAGES 域=%d" % (
            list(footers.keys()) or "无", page_fields, numpages_fields), "文档结构")
    add("目录 TOC 域", "PASS" if toc_instr else "FAIL",
        "instrText=%s" % (toc_instr[:2] or "无"), "文档结构")
    add("settings 打开时更新域（w:updateFields）",
        "PASS" if update_fields in ("true", "1", "on") else "FAIL",
        "updateFields=%r" % update_fields, "文档结构")
    add("文档属性 docProps/core.xml + app.xml",
        "PASS" if rep["docprops"]["core_part"] and rep["docprops"]["app_part"] else "FAIL",
        "core=%s app=%s, title=%r creator=%r" % (
            rep["docprops"]["core_part"], rep["docprops"]["app_part"],
            core_vals.get("title"), core_vals.get("creator")), "包结构")
    add("封面分节 + 首页不同 + 页码起始 1",
        "PASS" if (len(sectprs) >= 2 and titlepg > 0 and pgstart and pgstart[0] == "1") else "FAIL",
        "sectPr=%d, titlePg=%d, pgNumType.start=%s" % (
            len(sectprs), titlepg, pgstart), "文档结构")
    add("表格列宽 gridCol 合计 == tblW（无 tblW 时以版心宽 %s twips 为参照）" % content_width_twips,
        "PASS" if tables_info and not width_bad else ("WARN" if not tables_info else "FAIL"),
        "table=%d, 一致=%d, 不一致=%d, 明细=%s" % (
            len(tables_info), len(width_good), len(width_bad),
            [(t["gridCol_sum"], t["reference_width"]) for t in tables_info]),
        "内容元素")
    add("表格行控制/单元格宽度（tcW / cantSplit）",
        "PASS" if tables_info and all(t["tcW_count"] > 0 for t in tables_info) else "WARN",
        "tcW=%s, cantSplit=%s" % ([t["tcW_count"] for t in tables_info],
                                  [t["cantSplit_count"] for t in tables_info]), "内容元素")
    add("rPr 子元素顺序无违规", "PASS" if not rpr_bad else "FAIL",
        "检查 rPr=%d, 违规=%d" % (rpr_seen_doc + rpr_seen_st, len(rpr_bad)), "内容元素")
    add("字面量 '---' 分隔线未残留在正文（整段 == '---'）",
        "PASS" if len(standalone_dash_paras) == 0 else "FAIL",
        "独立成段=%d, w:t 命中=%d, 样本=%r" % (
            len(standalone_dash_paras), dash_nodes,
            standalone_dash_paras[0] if standalone_dash_paras else None), "内容元素")
    body_style = style_body_text.get("BodyText", {})
    add("正文首行缩进（BodyText firstLineChars/firstLine 或段落 w:ind firstLine）",
        "PASS" if (body_indent_direct > 0 or body_style.get("ind")) else "FAIL",
        "段落直接缩进=%d 段；BodyText 样式 ind=%s；Normal=%s" % (
            body_indent_direct, body_style.get("ind"),
            style_body_text.get("Normal", {}).get("ind")), "排版系统")
    add("行距（BodyText spacing line 或正文段落 spacing 一致）",
        "PASS" if (body_style.get("spacing") and body_style["spacing"].get("line")) else
        ("WARN" if not body_paras else "FAIL"),
        "BodyText spacing=%s；正文段落行距覆盖=%s" % (
            body_style.get("spacing"), body_spacing_line), "排版系统")
    add("超链接（w:hyperlink + external relationship）",
        "PASS" if hyperlinks > 0 and external_rels > 0 else "WARN",
        "hyperlink=%d, external rels=%d" % (hyperlinks, external_rels), "内容元素")
    add("python-docx 可打开", "PASS" if py_docx.get("ok") else "FAIL",
        py_docx.get("error", "ok: paragraphs=%s tables=%s sections=%s" % (
            py_docx.get("paragraphs"), py_docx.get("tables"), py_docx.get("sections"))),
        "兼容性")
    add("软换行合并（长段落 >=200 字）——静态观察项",
        "WARN" if len(long_paras) else "PASS",
        "%d 段 >=200 字，最长 %d 字：%r" % (len(long_paras), len(longest),
                                        longest[:60]), "内容元素")

    # ---- 静态判定 ----
    core_fail = [c for c in checks if c["status"] == "FAIL" and c["category"] in
                 ("包结构", "排版系统", "文档结构", "内容元素", "兼容性")]
    if core_fail:
        verdict = "FAIL"
    elif [c for c in checks if c["status"] == "WARN"]:
        verdict = "WARN"
    else:
        verdict = "PASS"
    rep["checks"] = checks
    rep["static_verdict"] = verdict
    rep["fail_count"] = len([c for c in checks if c["status"] == "FAIL"])
    rep["warn_count"] = len([c for c in checks if c["status"] == "WARN"])
    rep["pass_count"] = len([c for c in checks if c["status"] == "PASS"])
    return rep


# ---------------------------------------------------------------- Word COM 渲染
WINWORD = r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE"


def _win32_available():
    try:
        import win32com.client  # noqa: F401
        import win32gui  # noqa: F401
        return True
    except Exception:  # noqa: BLE001
        return False


def word_attempt_worker(source, outdir, result_path):
    """子进程 worker：直接启动本机 Word 并 COM 驱动，导出 PDF。

    - Word 以本脚本进程的直接子进程启动（TEMP/TMP 指向 workspace 的 tmp/），
      不做任何沙箱外写入；
    - 打开前后各拍一次窗口快照：若已存在“可见 Word 窗口”，判定用户正在用 Word，
      直接放弃（绝不碰用户的 Word）；
    - 守护线程只自动点掉已知的 "Word 无法创建工作文件…" 模态框并记录文本；
      未知模态框只记录、不点击（避免误操作）；
    - 每步写入 result json（flush），父进程超时后仍可读到最后阶段。
    """
    import json as _json
    import os as _os
    import subprocess as _sp
    import threading as _th
    import time as _t

    here = _os.path.dirname(_os.path.abspath(__file__))
    t0 = _t.time()
    log = []

    def rec(stage, **kw):
        log.append({"t": round(_t.time() - t0, 2), "stage": stage, **kw})
        tmp = result_path + ".partial"
        with open(tmp, "w", encoding="utf-8") as f:
            _json.dump({"ok": None, "log": log, "source": source}, f, ensure_ascii=False, indent=1)
        _os.replace(tmp, result_path)

    rec("begin", source=source, outdir=outdir)
    try:
        import win32con
        import win32gui
        import win32process
        import win32com.client as wc
        import pythoncom
    except Exception as exc:  # noqa: BLE001
        rec("import_error", error=repr(exc)[:300])
        _json.dump({"ok": False, "error": "pywin32 不可用: %r" % (exc,), "log": log},
                   open(result_path, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        return 1

    def opus_windows():
        rows = []

        def cb(hwnd, _):
            try:
                if win32gui.GetClassName(hwnd) == "OpusApp":
                    _, pid = win32process.GetWindowThreadProcessId(hwnd)
                    rows.append({"hwnd": int(hwnd), "pid": int(pid),
                                 "title": win32gui.GetWindowText(hwnd),
                                 "visible": bool(win32gui.IsWindowVisible(hwnd))})
            except Exception:
                pass

        try:
            win32gui.EnumWindows(cb, None)
        except Exception:
            pass
        return rows

    def work_file_dialogs():
        found = []

        def cb(hwnd, _):
            try:
                if win32gui.GetClassName(hwnd) != "#32770":
                    return
            except Exception:
                return
            texts, buttons = [], []

            def cb2(h, _):
                try:
                    cls = win32gui.GetClassName(h)
                    txt = win32gui.GetWindowText(h)
                    if cls == "Button":
                        buttons.append((int(h), txt))
                    elif txt:
                        texts.append(txt)
                except Exception:
                    pass

            try:
                win32gui.EnumChildWindows(hwnd, cb2, None)
            except Exception:
                pass
            if "工作文件" in " ".join(texts) or "work file" in " ".join(texts).lower():
                found.append({"hwnd": int(hwnd), "texts": texts, "buttons": buttons})

        try:
            win32gui.EnumWindows(cb, None)
        except Exception:
            pass
        return found

    # 安全闸门：用户已有可见 Word 窗口 -> 放弃，不做任何操作
    before = opus_windows()
    visible = [w for w in before if w["visible"]]
    if visible:
        rec("abort_user_word_visible", windows=visible[:5])
        _json.dump({"ok": False, "error": "检测到用户正在使用的可见 Word 窗口，放弃自动化",
                    "log": log}, open(result_path, "w", encoding="utf-8"),
                   ensure_ascii=False, indent=1)
        return 2

    tmp = _os.path.join(here, "tmp")
    _os.makedirs(tmp, exist_ok=True)
    env = dict(_os.environ)
    env["TEMP"] = tmp
    env["TMP"] = tmp
    proc = _sp.Popen([WINWORD], env=env, stdout=_sp.DEVNULL, stderr=_sp.DEVNULL)
    rec("word_spawned", child_pid=proc.pid)
    pidfile = _os.path.join(outdir, "word_worker.pid")
    with open(pidfile, "w") as f:
        f.write(str(proc.pid))

    pythoncom.CoInitialize()
    word = None
    tries = 0
    for tries in range(1, 161):
        try:
            word = wc.GetActiveObject("Word.Application")
            break
        except Exception:
            _t.sleep(0.25)
    rec("attach", tries=tries, ok=word is not None, opus=opus_windows()[:5])
    if word is None:
        _json.dump({"ok": False, "error": "无法附着到 Word COM 实例", "log": log},
                   open(result_path, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        try:
            proc.terminate()
        except Exception:
            pass
        return 2

    dialogs = []
    STOP = _th.Event()

    def watchdog():
        while not STOP.is_set():
            try:
                for d in work_file_dialogs():
                    if d["texts"] not in [x["texts"] for x in dialogs]:
                        dialogs.append(d)
                        rec("dialog_detected", hwnd=d["hwnd"], texts=d["texts"])
                    for h, txt in d["buttons"]:
                        if txt.strip() in ("确定", "OK", "&OK"):
                            try:
                                win32gui.SendMessage(h, win32con.BM_CLICK, 0, 0)
                                rec("dialog_clicked", hwnd=d["hwnd"], text=txt)
                            except Exception as e:  # noqa: BLE001
                                rec("dialog_click_error", error=repr(e)[:150])
                            break
            except Exception as e:  # noqa: BLE001
                rec("watchdog_error", error=repr(e)[:150])
            _t.sleep(0.2)

    th = _th.Thread(target=watchdog, daemon=True)
    th.start()
    doc = None
    ok = False
    pdf_path = _os.path.join(outdir, "before.pdf" if "before" in outdir else "after.pdf")
    try:
        word.Visible = False
        word.DisplayAlerts = 0
        try:
            word.Options.UpdateLinksAtOpen = False
        except Exception:
            pass
        rec("app_configured", version=str(word.Version))
        doc = word.Documents.Open(FileName=source, ConfirmConversions=False, ReadOnly=True,
                                  AddToRecentFiles=False, Visible=False,
                                  NoEncodingDialog=True, OpenAndRepair=False)
        rec("opened", paragraphs=int(doc.Paragraphs.Count), tables=int(doc.Tables.Count),
            sections=int(doc.Sections.Count), fields=int(doc.Fields.Count),
            tocs=int(doc.TablesOfContents.Count), readonly=bool(doc.ReadOnly))
        doc.Fields.Update()
        rec("fields_updated")
        n_toc = int(doc.TablesOfContents.Count)
        toc_entries = 0
        for i in range(1, n_toc + 1):
            doc.TablesOfContents(i).Update()
            toc_entries += int(doc.TablesOfContents(i).Range.Paragraphs.Count)
        rec("toc_updated", tocs=n_toc, entries=toc_entries)
        doc.Repaginate()
        pages = int(doc.ComputeStatistics(2))
        rec("repaginated", pages=pages)
        doc.ExportAsFixedFormat(OutputFileName=pdf_path, ExportFormat=17)
        rec("exported", pdf=_os.path.exists(pdf_path),
            size=_os.path.getsize(pdf_path) if _os.path.exists(pdf_path) else 0)
        ok = _os.path.exists(pdf_path)
    except Exception as exc:  # noqa: BLE001
        rec("error", error=repr(exc)[:600], dialogs=len(dialogs))
    finally:
        STOP.set()
        try:
            if doc is not None:
                doc.Close(SaveChanges=0)
                rec("doc_closed")
        except Exception as e:  # noqa: BLE001
            rec("doc_close_error", error=repr(e)[:200])
        quit_ok = False
        try:
            word.Quit()
            quit_ok = True
            rec("word_quit")
        except Exception as e:  # noqa: BLE001
            rec("word_quit_error", error=repr(e)[:300])
        for _i in range(24):
            if proc.poll() is not None:
                break
            _t.sleep(0.25)
        rec("word_exited", exited=proc.poll() is not None, code=proc.poll())
        if proc.poll() is None:
            try:
                proc.terminate()
                rec("word_terminate", sent=True)
                proc.wait(timeout=10)
                rec("word_terminated", code=proc.poll())
            except Exception as e:  # noqa: BLE001
                rec("word_terminate_failed", error=repr(e)[:200])
        try:
            pythoncom.CoUninitialize()
        except Exception:
            pass
        try:
            _os.remove(pidfile)
        except Exception:
            pass
        result = {"ok": ok, "pdf": pdf_path if ok else None, "log": log,
                  "dialogs": [{"texts": d["texts"]} for d in dialogs],
                  "quit_ok": quit_ok, "elapsed_s": round(_t.time() - t0, 2)}
        _json.dump(result, open(result_path, "w", encoding="utf-8"),
                   ensure_ascii=False, indent=1)
    return 0 if ok else 1


def cleanup_leftover_word(outdir, timeout_s=20):
    """清理本次失败尝试留下的隐藏 Word 实例（只处理“不可见 OpusApp + 已知 work-file 对话框”）。

    先看 pidfile：只关闭该 pid 的 Word；pidfile 丢失时退化为“隐藏 OpusApp 窗口”；
    可见 Word 窗口一律不碰，并记录在案。
    """
    try:
        import win32con
        import win32gui
        import win32process
    except Exception:  # noqa: BLE001
        return {"available": False}
    actions = []

    def snapshot():
        rows = []

        def cb(hwnd, _):
            try:
                cls = win32gui.GetClassName(hwnd)
                if cls not in ("OpusApp", "#32770"):
                    return
                _, pid = win32process.GetWindowThreadProcessId(hwnd)
                rows.append({"hwnd": int(hwnd), "pid": int(pid), "class": cls,
                             "title": win32gui.GetWindowText(hwnd),
                             "visible": bool(win32gui.IsWindowVisible(hwnd))})
            except Exception:
                pass

        try:
            win32gui.EnumWindows(cb, None)
        except Exception:
            pass
        return rows

    pid = None
    pidfile = os.path.join(outdir, "word_worker.pid")
    try:
        with open(pidfile) as f:
            pid = int(f.read().strip())
    except Exception:
        pid = None

    for r in snapshot():
        user_visible = r["class"] == "OpusApp" and r["visible"]
        if user_visible:
            continue
        ours = False
        if pid is not None:
            ours = (r["pid"] == pid)
        elif r["class"] == "OpusApp" and (r["title"].strip() == "Word" or "before_desktop" in r["title"]):
            ours = True
        if not ours:
            continue
        if r["class"] == "#32770":
            # 已知 work-file 对话框：点确定
            texts = []

            def cb2(h, _):
                try:
                    texts.append(win32gui.GetWindowText(h))
                except Exception:
                    pass

            try:
                win32gui.EnumChildWindows(r["hwnd"], cb2, None)
            except Exception:
                pass
            if "工作文件" in " ".join(texts):
                try:
                    def cb3(h, _):
                        try:
                            if win32gui.GetClassName(h) == "Button":
                                win32gui.SendMessage(h, win32con.BM_CLICK, 0, 0)
                        except Exception:
                            pass

                    win32gui.EnumChildWindows(r["hwnd"], cb3, None)
                    actions.append({"action": "click_ok", "hwnd": r["hwnd"]})
                except Exception:
                    pass
        else:
            try:
                win32gui.PostMessage(r["hwnd"], win32con.WM_CLOSE, 0, 0)
                actions.append({"action": "wm_close", "hwnd": r["hwnd"], "pid": r["pid"]})
            except Exception:
                pass
    try:
        os.remove(pidfile)
    except Exception:
        pass
    return {"available": True, "actions": actions,
            "still": [r for r in snapshot() if r["class"] == "OpusApp" and not r["visible"]]}


def try_word_export(source, outdir, timeout_s=90):
    """以子进程方式跑 Word COM，强制超时；失败/超时都清理残留实例。"""
    os.makedirs(outdir, exist_ok=True)
    result_path = os.path.join(outdir, "word_attempt.json")
    t0 = time.time()
    if not os.path.exists(WINWORD):
        return {"ok": False, "available": False,
                "error": "找不到 WINWORD.EXE: %s" % WINWORD, "elapsed_s": 0}
    if not _win32_available():
        return {"ok": False, "available": False,
                "error": "pywin32 不可用（win32com/win32gui import 失败）", "elapsed_s": 0}
    cmd = [sys.executable, os.path.abspath(__file__), "--_word-worker", source, outdir, result_path]
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = proc.communicate(timeout=timeout_s)
        timed_out = False
    except subprocess.TimeoutExpired:
        timed_out = True
        proc.kill()
        try:
            out, err = proc.communicate(timeout=15)
        except Exception:  # noqa: BLE001
            out, err = b"", b""
    data = {}
    try:
        with open(result_path, encoding="utf-8") as f:
            data = json.load(f)
    except Exception:
        pass
    result = {
        "available": True,
        "timed_out": timed_out,
        "returncode": proc.returncode,
        "elapsed_s": round(time.time() - t0, 1),
        "ok": bool(data.get("ok")),
        "pdf": data.get("pdf"),
        "dialogs": data.get("dialogs", []),
        "stage_log": data.get("log", []),
        "error": data.get("error"),
        "stderr_tail": (err or b"").decode("utf-8", "replace")[-500:],
    }
    if timed_out:
        result["error"] = "Word COM 渲染超过 %ss 未返回，已终止并清理" % timeout_s
    result["cleanup"] = cleanup_leftover_word(outdir)
    return result


# ---------------------------------------------------------------- PDF -> PNG
def render_pdf(pdf_path, outdir, label, dpi=150, max_pages=3):
    """PDF 前 max_pages 页 + 目录页 → PNG；返回 manifest 片段。"""
    try:
        import fitz
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": "PyMuPDF(fitz) 不可用: %r" % (exc,)}
    try:
        import PIL  # noqa: F401
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": "Pillow 不可用: %r" % (exc,)}
    pages_dir = os.path.join(outdir, "pages")
    os.makedirs(pages_dir, exist_ok=True)
    try:
        doc = fitz.open(pdf_path)
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": "打开 PDF 失败: %r" % (exc,)}
    n = doc.page_count
    toc_page = None
    for i in range(min(n, 12)):
        text = doc[i].get_text() or ""
        head = text[:1200]
        if "目录" in head or "CONTENTS" in head.upper() or "Table of Contents" in head:
            toc_page = i
            break
        if re.search(r"\.{3,}\s*\d+", text):
            toc_page = i
            break
    wanted = list(range(min(max_pages, n)))
    if toc_page is not None and toc_page not in wanted:
        wanted.append(toc_page)
    files = []
    for i in wanted:
        pix = doc[i].get_pixmap(dpi=dpi, alpha=False)
        name = "%s_page_%02d%s.png" % (label, i + 1, "_toc" if i == toc_page else "")
        path = os.path.join(pages_dir, name)
        pix.save(path)
        # 像素有效性检查：灰度标准差（避免把透明/纯白误判为空白）
        try:
            from PIL import Image
            im = Image.open(path).convert("L")
            px = list(im.getdata())
            mean = sum(px) / len(px)
            std = (sum((v - mean) ** 2 for v in px) / len(px)) ** 0.5
            blank = std < 2.0
        except Exception:
            blank = None
        files.append({"page": i + 1, "file": path, "bytes": os.path.getsize(path),
                      "grayscale_std": None if blank is None else round(std, 3),
                      "blank": blank, "is_toc_page": i == toc_page})
    doc.close()
    sheet = None
    if files:
        try:
            sheet = make_contact_sheet([f["file"] for f in files], label,
                                       os.path.join(outdir, "contact_sheet.png"))
        except Exception as exc:  # noqa: BLE001
            sheet = None
    return {"ok": True, "pdf": pdf_path, "pages_total": n, "toc_page": None if toc_page is None else toc_page + 1,
            "pngs": files, "contact_sheet": sheet}


def make_contact_sheet(pngs, label, out_path, cols=3, thumb_w=700):
    from PIL import Image, ImageDraw, ImageFont

    imgs = []
    for p in pngs:
        try:
            im = Image.open(p).convert("RGB")
            w, h = im.size
            im = im.resize((thumb_w, int(h * thumb_w / w)))
            imgs.append(im)
        except Exception:
            continue
    if not imgs:
        return None
    pad, header = 18, 46
    rows = (len(imgs) + cols - 1) // cols
    cell_h = max(im.height for im in imgs)
    sheet_w = cols * thumb_w + (cols + 1) * pad
    sheet_h = header + rows * (cell_h + pad) + pad
    sheet = Image.new("RGB", (sheet_w, sheet_h), "white")
    draw = ImageDraw.Draw(sheet)
    try:
        font = ImageFont.truetype("msyh.ttc", 22)
    except Exception:
        font = ImageFont.load_default()
    draw.text((pad, 12), "%s（docx 渲染，Word COM → PDF → PNG）" % label, fill="black", font=font)
    for i, im in enumerate(imgs):
        r, c = divmod(i, cols)
        x = pad + c * (thumb_w + pad)
        y = header + r * (cell_h + pad)
        sheet.paste(im, (x, y + header - header))
    sheet.save(out_path)
    return out_path


def make_compare_sheet(before_dir, after_dir, out_path):
    from PIL import Image, ImageDraw, ImageFont

    def pages(d):
        pd = os.path.join(d, "pages")
        if not os.path.isdir(pd):
            return []
        return sorted(os.path.join(pd, f) for f in os.listdir(pd) if f.lower().endswith(".png"))

    bp, ap = pages(before_dir), pages(after_dir)
    n = max(len(bp), len(ap))
    if n == 0:
        return None
    thumb_w = 640
    imgs = []
    for i in range(n):
        pair = []
        for src in (bp, ap):
            if i < len(src):
                im = Image.open(src[i]).convert("RGB")
                w, h = im.size
                pair.append(im.resize((thumb_w, int(h * thumb_w / w))))
            else:
                pair.append(Image.new("RGB", (thumb_w, 860), "white"))
        imgs.append(pair)
    cell_h = max(im.height for pair in imgs for im in pair) + 34
    sheet_w = thumb_w * 2 + 60
    sheet_h = 60 + n * (cell_h + 14)
    sheet = Image.new("RGB", (sheet_w, sheet_h), "white")
    draw = ImageDraw.Draw(sheet)
    try:
        font = ImageFont.truetype("msyh.ttc", 20)
    except Exception:
        font = ImageFont.load_default()

    def colname(d):
        return "before（桌面原文件）" if "before" in os.path.basename(d) else "after（gold.docx）"

    y = 0
    for i, pair in enumerate(imgs):
        draw.text((12, y + 12), "%s  第 %d 页" % (colname(before_dir), i + 1), fill="black", font=font)
        draw.text((thumb_w + 48, y + 12), "%s  第 %d 页" % (colname(after_dir), i + 1),
                  fill="black", font=font)
        y += 34
        sheet.paste(pair[0], (10, y))
        sheet.paste(pair[1], (thumb_w + 50, y))
        y += cell_h + 14 - 34
    sheet.save(out_path)
    return out_path


# ---------------------------------------------------------------- 主流程
def copy_source_if_needed(source, outdir):
    """把源文件复制到 out/<label>/source/，保证 Word 只碰副本（原桌面文件只读不改）。"""
    src_dir = os.path.join(outdir, "source")
    os.makedirs(src_dir, exist_ok=True)
    ext = os.path.splitext(source)[1] or ".docx"
    dst = os.path.join(src_dir, "source_copy" + ext)
    if os.path.abspath(source) != os.path.abspath(dst):
        shutil.copy2(source, dst)
    return dst, sha256(source), sha256(dst)


def run_one(source, label, outdir, mode="auto", word_timeout=90):
    os.makedirs(outdir, exist_ok=True)
    os.makedirs(LOGDIR, exist_ok=True)
    t0 = time.time()
    manifest = {
        "label": label,
        "source": source,
        "outdir": outdir,
        "mode": mode,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "environment": {
            "python": sys.version.split()[0],
            "word_exe": WINWORD if os.path.exists(WINWORD) else None,
            "free_offers": None,
        },
    }
    copy_path, src_sha, copy_sha = copy_source_if_needed(source, outdir)
    manifest["source_copy"] = copy_path
    manifest["sha256"] = {"original": src_sha, "copy": copy_sha,
                          "identical": src_sha == copy_sha}
    manifest["size"] = os.path.getsize(source)

    # 1) 渲染尝试
    render = {"attempted": False, "ok": False, "reason": "mode=structure，跳过 Word 渲染"}
    if mode in ("auto", "word"):
        render = {"attempted": True}
        word_res = try_word_export(copy_path, outdir, timeout_s=word_timeout)
        manifest["word_attempt"] = word_res
        if word_res.get("ok"):
            pdf = word_res["pdf"]
            png = render_pdf(pdf, outdir, label)
            render = {"attempted": True, "ok": png.get("ok", False),
                      "pdf": pdf, "png": png}
            if not png.get("ok"):
                render["reason"] = png.get("error")
        else:
            render["ok"] = False
            render["reason"] = word_res.get("error") or "Word COM 渲染失败（无 PDF 产物）"
            render["dialogs"] = word_res.get("dialogs", [])
            render["cleanup"] = word_res.get("cleanup")
            render["evidence"] = os.path.join(outdir, "word_attempt.json")
        if mode == "word" and not render["ok"]:
            manifest["render"] = render
            manifest["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
            manifest["elapsed_s"] = round(time.time() - t0, 2)
            with open(os.path.join(outdir, "manifest.json"), "w", encoding="utf-8") as f:
                json.dump(manifest, f, ensure_ascii=False, indent=1)
            return manifest
    manifest["render"] = render

    # 2) 结构校验（永远做）
    structure = analyze_structure(source, outdir)
    with open(os.path.join(outdir, "structure.json"), "w", encoding="utf-8") as f:
        json.dump(structure, f, ensure_ascii=False, indent=1)
    manifest["structure"] = {
        "file": os.path.join(outdir, "structure.json"),
        "verdict": structure["static_verdict"],
        "parts_count": structure["parts"]["count"],
        "checks": {"pass": structure["pass_count"], "fail": structure["fail_count"],
                   "warn": structure["warn_count"]},
        "failed_checks": [c["check"] for c in structure["checks"] if c["status"] == "FAIL"],
    }
    manifest["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    manifest["elapsed_s"] = round(time.time() - t0, 2)
    with open(os.path.join(outdir, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)
    return manifest


def main(argv=None):
    ap = argparse.ArgumentParser(description="docx 渲染 + 兼容性/结构校验")
    ap.add_argument("--preset", choices=sorted(PRESETS), help="before|after|report")
    ap.add_argument("--source", help="任意 docx 路径")
    ap.add_argument("--label", help="输出标签（默认 before/after/report 或文件名）")
    ap.add_argument("--outdir", help="输出目录（默认 out/<label>）")
    ap.add_argument("--mode", choices=("structure", "word", "auto"), default="auto")
    ap.add_argument("--word-timeout", type=int, default=90, help="Word COM 单份文档超时秒数（默认 90）")
    ap.add_argument("--compare", action="store_true", help="用已有 before/after PNG 生成并排对比图")
    ap.add_argument("--_word-worker", nargs=3, metavar=("SOURCE", "OUTDIR", "RESULT"),
                    help=argparse.SUPPRESS)
    args = ap.parse_args(argv)

    if args._word_worker:
        return word_attempt_worker(args._word_worker[0], args._word_worker[1],
                                   args._word_worker[2])

    if args.compare:
        cmp_dir = os.path.join(OUT, "compare")
        os.makedirs(cmp_dir, exist_ok=True)
        out = os.path.join(cmp_dir, "contact_sheet_before_after.png")
        res = make_compare_sheet(os.path.join(OUT, "before"), os.path.join(OUT, "after"), out)
        print(json.dumps({"compare_sheet": res}, ensure_ascii=False))
        return 0 if res else 2

    if args.preset:
        source, label = PRESETS[args.preset]
    elif args.source:
        source, label = args.source, (args.label or os.path.splitext(os.path.basename(args.source))[0])
    else:
        ap.error("需要 --preset 或 --source")
    if not os.path.exists(source):
        print(json.dumps({"error": "源文件不存在: %s" % source}, ensure_ascii=False))
        return 2
    label = args.label or label
    outdir = args.outdir or os.path.join(OUT, label)
    manifest = run_one(os.path.abspath(source), label, outdir, args.mode, args.word_timeout)
    summary = {
        "label": label,
        "outdir": os.path.abspath(outdir),
        "render": {"attempted": manifest["render"].get("attempted"),
                   "ok": manifest["render"].get("ok"),
                   "reason": manifest["render"].get("reason")},
        "structure": manifest.get("structure"),
        "sha256_identical": manifest["sha256"]["identical"],
    }
    print(json.dumps(summary, ensure_ascii=False, indent=1))
    return 0 if (manifest["render"].get("ok") or manifest["render"].get("attempted") is False) else 2


if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:  # noqa: BLE001
        pass
    sys.exit(main())
