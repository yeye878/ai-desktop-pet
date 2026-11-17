#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
gold_docx.py — Markdown → 「专业报告级」.docx 的参考实现（手写 OOXML 字符串 + zipfile）

设计原则（对齐 Rust src-tauri/src/direct_api/tools.rs 的 build_docx_bytes / markdown_to_document_xml）：
  1. 生成侧只用标准库（zipfile / re / unicodedata / datetime），**不依赖 python-docx**；
     python-docx 仅用于外部验收（见 verify_docx.py），保证结论可 1:1 移植回 Rust 字符串拼接。
  2. 所有 part 都是可读的 XML 字符串常量/函数，便于逐段替换成 Rust 的 `format!`。
  3. 样式驱动排版：document.xml 只写 pStyle / numPr / 关系 id，外观集中在 styles.xml。
  4. part 集合、关系 id、节结构显式编号，移植时可按 README 的映射表照抄。

用法：
  python gold_docx.py --src input.md --out out.docx --title "标题" --author "作者"
可选参数：
  --subtitle / --date / --org / --subject / --no-toc

front matter（可选，文件最前面的 --- 块）支持键：
  title / subtitle / author / date / subject / keywords / org
"""

from __future__ import annotations

import argparse
import datetime as _dt
import html
import os
import re
import sys
import unicodedata
import zipfile
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

# ---------------------------------------------------------------------------
# 0. 常量：命名空间 / 页面尺寸 / 关系 id 分配约定
# ---------------------------------------------------------------------------

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PKG_REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
CT_NS = "http://schemas.openxmlformats.org/package/2006/content-types"
CP_NS = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
DC_NS = "http://purl.org/dc/elements/1.1/"
DCTERMS_NS = "http://purl.org/dc/terms/"
XSI_NS = "http://www.w3.org/2001/XMLSchema-instance"
EP_NS = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"
VT_NS = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"
A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"

XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'

# A4：11906 × 16838 twips；页边距上下 2.54cm(1440)、左右 2.54cm(1440)
PAGE_W = 11906
PAGE_H = 16838
MARGIN_TB = 1440
MARGIN_LR = 1440
CONTENT_W = PAGE_W - 2 * MARGIN_LR  # = 9026 twips，表格总宽基准
HEADER_DIST = 851
FOOTER_DIST = 992
assert CONTENT_W == 9026, "A4 + 1440 左右边距的内容宽度应为 9026 twips"

# document.xml.rels 的固定关系 id：rId1..rId6 = 基础设施 part，rId7/rId8 = 页眉/页脚，
# rId9 起动态分配给超链接（同一 URL 复用同一 rId）。
RID_STYLES = "rId1"
RID_NUMBERING = "rId2"
RID_SETTINGS = "rId3"
RID_THEME = "rId4"
RID_FONT_TABLE = "rId5"
RID_WEB_SETTINGS = "rId6"
RID_HEADER = "rId7"
RID_FOOTER = "rId8"
FIRST_DYNAMIC_RID = 9

REL_TYPE_BASE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"


# ---------------------------------------------------------------------------
# 1. 转义 / 纯文本工具
# ---------------------------------------------------------------------------

_CTRL_RE = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f]")


def escape_xml(text: object) -> str:
    """XML 文本/属性转义，并剔除 XML 1.0 不允许的控制字符。"""
    s = _CTRL_RE.sub("", str(text))
    return (
        s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&apos;")
    )


def plain_text(md_text: str) -> str:
    """把行内 markdown 还原成"可见文本"，用于封面/统计/表格列宽估算。"""
    s = re.sub(r"!\[([^\]]*)\]\(([^)]+)\)", r"\1", md_text)
    s = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r"\1", s)
    s = re.sub(r"<((?:https?|mailto):[^>\s]+)>", r"\1", s)
    s = s.replace("**", "").replace("`", "").replace("*", "")
    s = s.replace("~~", "").replace("__", "")
    s = re.sub(r"\\([\\`*_{}\[\]()#+\-.!|>~])", r"\1", s)
    return s.strip()


def display_units(text: str) -> int:
    """表格列宽估算：CJK 宽字符（East Asian Width = W/F）记 2 个长度单位，其余记 1。"""
    n = 0
    for ch in text:
        if ch in "\r\n\t":
            continue
        if unicodedata.east_asian_width(ch) in ("W", "F"):
            n += 2
        else:
            n += 1
    return n


# ---------------------------------------------------------------------------
# 2. 包级 part：索引 / 关系 / 文档属性
# ---------------------------------------------------------------------------


def package_content_types() -> str:
    """[Content_Types].xml：19 个 part 的默认类型 + 全部 Override。"""
    parts = [
        ("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
        ("/word/styles.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"),
        ("/word/numbering.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"),
        ("/word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
        ("/word/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml"),
        ("/word/fontTable.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"),
        ("/word/webSettings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"),
        ("/word/header1.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"),
        ("/word/footer1.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"),
        ("/docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
        ("/docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
    ]
    overrides = "".join(
        f'<Override PartName="{name}" ContentType="{ctype}"/>' for name, ctype in parts
    )
    return (
        XML_DECL
        + f'<Types xmlns="{CT_NS}">'
        + '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        + '<Default Extension="xml" ContentType="application/xml"/>'
        + overrides
        + "</Types>"
    )


def package_rels() -> str:
    """_rels/.rels：officeDocument + core-properties + extended-properties。"""
    return (
        XML_DECL
        + f'<Relationships xmlns="{PKG_REL_NS}">'
        + f'<Relationship Id="rId1" Type="{REL_TYPE_BASE}officeDocument" Target="word/document.xml"/>'
        + f'<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>'
        + f'<Relationship Id="rId3" Type="{REL_TYPE_BASE}extended-properties" Target="docProps/app.xml"/>'
        + "</Relationships>"
    )


def document_rels(hyperlink_rels: Sequence[Tuple[str, str]]) -> str:
    """word/_rels/document.xml.rels：基础设施 part + 页眉页脚 + 动态超链接关系。"""
    base = [
        (RID_STYLES, f"{REL_TYPE_BASE}styles", "styles.xml", None),
        (RID_NUMBERING, f"{REL_TYPE_BASE}numbering", "numbering.xml", None),
        (RID_SETTINGS, f"{REL_TYPE_BASE}settings", "settings.xml", None),
        (RID_THEME, f"{REL_TYPE_BASE}theme", "theme/theme1.xml", None),
        (RID_FONT_TABLE, f"{REL_TYPE_BASE}fontTable", "fontTable.xml", None),
        (RID_WEB_SETTINGS, f"{REL_TYPE_BASE}webSettings", "webSettings.xml", None),
        (RID_HEADER, f"{REL_TYPE_BASE}header", "header1.xml", None),
        (RID_FOOTER, f"{REL_TYPE_BASE}footer", "footer1.xml", None),
    ]
    rels = "".join(
        f'<Relationship Id="{rid}" Type="{rtype}" Target="{escape_xml(target)}"/>'
        for rid, rtype, target, _ in base
    )
    for rid, target in hyperlink_rels:
        rels += (
            f'<Relationship Id="{rid}" Type="{REL_TYPE_BASE}hyperlink" '
            f'Target="{escape_xml(target)}" TargetMode="External"/>'
        )
    return XML_DECL + f'<Relationships xmlns="{PKG_REL_NS}">' + rels + "</Relationships>"


def _iso_now() -> str:
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def core_properties(meta: Dict[str, str], created: Optional[str] = None,
                    modified: Optional[str] = None) -> str:
    """docProps/core.xml：title / subject / creator / lastModifiedBy / created / modified。"""
    created = created or _iso_now()
    modified = modified or created
    return (
        XML_DECL
        + f'<cp:coreProperties xmlns:cp="{CP_NS}" xmlns:dc="{DC_NS}" '
        f'xmlns:dcterms="{DCTERMS_NS}" xmlns:dcmitype="http://purl.org/dc/dcmitype/" '
        f'xmlns:xsi="{XSI_NS}">'
        + f"<dc:title>{escape_xml(meta.get('title', ''))}</dc:title>"
        + f"<dc:subject>{escape_xml(meta.get('subject', ''))}</dc:subject>"
        + f"<dc:creator>{escape_xml(meta.get('author', ''))}</dc:creator>"
        + f"<cp:lastModifiedBy>{escape_xml(meta.get('author', ''))}</cp:lastModifiedBy>"
        + f"<cp:keywords>{escape_xml(meta.get('keywords', ''))}</cp:keywords>"
        + f"<cp:category>{escape_xml(meta.get('category', '研究报告'))}</cp:category>"
        + f'<dcterms:created xsi:type="dcterms:W3CDTF">{created}</dcterms:created>'
        + f'<dcterms:modified xsi:type="dcterms:W3CDTF">{modified}</dcterms:modified>'
        + "</cp:coreProperties>"
    )


def extended_properties(meta: Dict[str, str], stats: Dict[str, int]) -> str:
    """docProps/app.xml：Application / Pages / Words / Characters / Lines / Paragraphs 等。"""
    words = stats.get("words", 0)
    chars = stats.get("characters", 0)
    chars_ws = stats.get("characters_with_spaces", chars)
    lines = stats.get("lines", 0)
    paras = stats.get("paragraphs", 0)
    pages = stats.get("pages", 1)
    return (
        XML_DECL
        + f'<Properties xmlns="{EP_NS}" xmlns:vt="{VT_NS}">'
        + "<Template>Normal.dotm</Template>"
        + "<TotalTime>0</TotalTime>"
        + f"<Pages>{pages}</Pages>"
        + f"<Words>{words}</Words>"
        + f"<Characters>{chars}</Characters>"
        + "<Application>AI Desktop Pet · raw OOXML writer</Application>"
        + "<DocSecurity>0</DocSecurity>"
        + f"<Lines>{lines}</Lines>"
        + f"<Paragraphs>{paras}</Paragraphs>"
        + "<ScaleCrop>false</ScaleCrop>"
        + f"<Company>{escape_xml(meta.get('org', ''))}</Company>"
        + "<LinksUpToDate>false</LinksUpToDate>"
        + f"<CharactersWithSpaces>{chars_ws}</CharactersWithSpaces>"
        + "<SharedDoc>false</SharedDoc>"
        + "<HyperlinksChanged>false</HyperlinksChanged>"
        + "<AppVersion>16.0000</AppVersion>"
        + "</Properties>"
    )


def document_settings() -> str:
    """word/settings.xml：updateFields（让 Word 打开时更新 TOC/PAGE/NUMPAGES 域）+ 中文排版兼容项。"""
    return (
        XML_DECL
        + f'<w:settings xmlns:w="{W_NS}">'
        + '<w:zoom w:percent="100"/>'
        + '<w:defaultTabStop w:val="420"/>'
        + '<w:characterSpacingControl w:val="compressPunctuation"/>'
        + '<w:compat>'
        + '<w:compatSetting w:name="compatibilityMode" '
        + 'w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>'
        + "</w:compat>"
        + '<w:themeFontLang w:val="en-US" w:eastAsia="zh-CN"/>'
        + '<w:updateFields w:val="true"/>'
        + "</w:settings>"
    )


def theme_part() -> str:
    """word/theme/theme1.xml：最小但结构完整的 Office 主题（clrScheme/fontScheme/fmtScheme）。"""
    accents = "".join(
        f'<a:accent{i}><a:srgbClr val="{c}"/></a:accent{i}>'
        for i, c in enumerate(("4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"), start=1)
    )
    lines = "".join(
        f'<a:ln w="{w}" cap="flat" cmpd="sng" algn="ctr">'
        '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        '<a:prstDash val="solid"/></a:ln>'
        for w in (6350, 12700, 19050)
    )
    return (
        XML_DECL
        + f'<a:theme xmlns:a="{A_NS}" name="Office 主题">'
        + "<a:themeElements>"
        + '<a:clrScheme name="Office">'
        + '<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>'
        + '<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>'
        + '<a:dk2><a:srgbClr val="44546A"/></a:dk2>'
        + '<a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>'
        + accents
        + '<a:hlink><a:srgbClr val="0563C1"/></a:hlink>'
        + '<a:folHlink><a:srgbClr val="954F72"/></a:folHlink>'
        + "</a:clrScheme>"
        + '<a:fontScheme name="Office">'
        + '<a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>'
        + '<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>'
        + "</a:fontScheme>"
        + '<a:fmtScheme name="Office">'
        + "<a:fillStyleLst>"
        + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        + "</a:fillStyleLst>"
        + f"<a:lnStyleLst>{lines}</a:lnStyleLst>"
        + "<a:effectStyleLst>"
        + "<a:effectStyle><a:effectLst/></a:effectStyle>"
        + "<a:effectStyle><a:effectLst/></a:effectStyle>"
        + "<a:effectStyle><a:effectLst/></a:effectStyle>"
        + "</a:effectStyleLst>"
        + "<a:bgFillStyleLst>"
        + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
        + "</a:bgFillStyleLst>"
        + "</a:fmtScheme>"
        + "</a:themeElements>"
        + "</a:theme>"
    )


def font_table_part() -> str:
    """word/fontTable.xml：本实现实际用到的字体（含中文字体 altName）。"""
    fonts = [
        ('Times New Roman', None, "00", "roman", "variable"),
        ("Arial", None, "00", "swiss", "variable"),
        ("Calibri", None, "00", "swiss", "variable"),
        ("Consolas", None, "00", "modern", "fixed"),
        ("宋体", "SimSun", "86", "auto", "variable"),
        ("黑体", "SimHei", "86", "auto", "variable"),
        ("微软雅黑", "Microsoft YaHei", "86", "auto", "variable"),
    ]
    entries = ""
    for name, alt, charset, family, pitch in fonts:
        alt_xml = f'<w:altName w:val="{escape_xml(alt)}"/>' if alt else ""
        entries += (
            f'<w:font w:name="{escape_xml(name)}">'
            + alt_xml
            + f'<w:charset w:val="{charset}"/>'
            + f'<w:family w:val="{family}"/>'
            + f'<w:pitch w:val="{pitch}"/>'
            + "</w:font>"
        )
    return XML_DECL + f'<w:fonts xmlns:w="{W_NS}">' + entries + "</w:fonts>"


def web_settings_part() -> str:
    return (
        XML_DECL
        + f'<w:webSettings xmlns:w="{W_NS}">'
        + "<w:optimizeForBrowser/>"
        + "<w:allowPNG/>"
        + "</w:webSettings>"
    )


def header_part(title: str) -> str:
    """word/header1.xml：页眉文档名（右对齐 + 下框线，外观由 Header 样式控制）。"""
    return (
        XML_DECL
        + f'<w:hdr xmlns:w="{W_NS}" xmlns:r="{R_NS}">'
        + '<w:p><w:pPr><w:pStyle w:val="Header"/></w:pPr>'
        + f'<w:r><w:t xml:space="preserve">{escape_xml(title)}</w:t></w:r>'
        + "</w:p></w:hdr>"
    )


def footer_part() -> str:
    """word/footer1.xml：页脚「第 X 页 / 共 Y 页」，X=PAGE 域，Y=NUMPAGES 域。"""
    def field(instr: str) -> str:
        return (
            '<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
            f'<w:r><w:instrText xml:space="preserve"> {instr} </w:instrText></w:r>'
            '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
            '<w:r><w:t>1</w:t></w:r>'
            '<w:r><w:fldChar w:fldCharType="end"/></w:r>'
        )

    return (
        XML_DECL
        + f'<w:ftr xmlns:w="{W_NS}" xmlns:r="{R_NS}">'
        + '<w:p><w:pPr><w:pStyle w:val="Footer"/></w:pPr>'
        + '<w:r><w:t xml:space="preserve">第 </w:t></w:r>'
        + field("PAGE  \\* MERGEFORMAT")
        + '<w:r><w:t xml:space="preserve"> 页 / 共 </w:t></w:r>'
        + field("NUMPAGES  \\* MERGEFORMAT")
        + '<w:r><w:t xml:space="preserve"> 页</w:t></w:r>'
        + "</w:p></w:ftr>"
    )


def write_package(out_path: str, parts: Dict[str, str]) -> int:
    """手写 zip：固定时间戳 + Deflate，返回写入字节数。"""
    os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for name, text in parts.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o600 << 16
            zf.writestr(info, text.encode("utf-8"))
    return os.path.getsize(out_path)


# ---------------------------------------------------------------------------
# 3. word/numbering.xml：真编号列表（多级 ilvl + 每个列表独立 numId）
# ---------------------------------------------------------------------------

_BULLET_CHARS = ("•", "◦", "▪", "–")


def _bullet_level(ilvl: int) -> str:
    ind_left = 720 + 360 * ilvl
    return (
        f'<w:lvl w:ilvl="{ilvl}">'
        '<w:start w:val="1"/>'
        '<w:numFmt w:val="bullet"/>'
        f'<w:lvlText w:val="{_BULLET_CHARS[ilvl]}"/>'
        '<w:lvlJc w:val="left"/>'
        f'<w:pPr><w:ind w:left="{ind_left}" w:hanging="360"/></w:pPr>'
        '<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" '
        'w:eastAsia="微软雅黑" w:cs="Arial"/></w:rPr>'
        "</w:lvl>"
    )


def _ordered_level(ilvl: int) -> str:
    ind_left = 720 + 360 * ilvl
    return (
        f'<w:lvl w:ilvl="{ilvl}">'
        '<w:start w:val="1"/>'
        '<w:numFmt w:val="decimal"/>'
        '<w:suff w:val="space"/>'
        f'<w:lvlText w:val="%{ilvl + 1}."/>'
        '<w:lvlJc w:val="left"/>'
        f'<w:pPr><w:ind w:left="{ind_left}" w:hanging="360"/></w:pPr>'
        '<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" '
        'w:eastAsia="宋体" w:cs="Times New Roman"/></w:rPr>'
        "</w:lvl>"
    )


def numbering_part(instances: Sequence[Tuple[str, int]]) -> str:
    """instances: [(kind, start)]，kind 为 'bullet' / 'ordered'，每个列表一个 w:num 实例。

    ordered 列表额外写 w:lvlOverride + w:startOverride，保证多个有序列表各自从 1 重新编号。
    """
    abstract_xml = (
        '<w:abstractNum w:abstractNumId="0">'
        '<w:nsid w:val="2A4B6C8D"/>'
        '<w:multiLevelType w:val="hybridMultilevel"/>'
        + "".join(_bullet_level(i) for i in range(4))
        + "</w:abstractNum>"
        '<w:abstractNum w:abstractNumId="1">'
        '<w:nsid w:val="2A4B6C8E"/>'
        '<w:multiLevelType w:val="hybridMultilevel"/>'
        + "".join(_ordered_level(i) for i in range(4))
        + "</w:abstractNum>"
    )
    nums = ""
    for num_id, (kind, start) in enumerate(instances, start=1):
        if kind == "ordered":
            abstract_id, override = 1, (
                f'<w:lvlOverride w:ilvl="0">'
                f'<w:startOverride w:val="{max(1, int(start))}"/>'
                f"</w:lvlOverride>"
            )
        else:
            abstract_id, override = 0, ""
        nums += (
            f'<w:num w:numId="{num_id}">'
            f'<w:abstractNumId w:val="{abstract_id}"/>{override}</w:num>'
        )
    return XML_DECL + f'<w:numbering xmlns:w="{W_NS}">' + abstract_xml + nums + "</w:numbering>"


# ---------------------------------------------------------------------------
# 4. Markdown 解析：front matter → block 级节点；行内 → RunToken
# ---------------------------------------------------------------------------


@dataclass
class RunToken:
    """一个行内 run：文本 + 粗体/斜体/等宽 + 可选外链。"""

    text: str
    bold: bool = False
    italic: bool = False
    mono: bool = False
    href: Optional[str] = None


@dataclass
class Node:
    """block 级节点。kind ∈ heading/body/quote/code/list/table/hr/caption。"""

    kind: str
    level: int = 0
    text: str = ""
    lines: List[str] = field(default_factory=list)
    rows: List[List[str]] = field(default_factory=list)
    items: List[Tuple[int, List[str]]] = field(default_factory=list)
    ordered: bool = False
    start: int = 1


_FM_KEY_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*[:：]\s*(.*)$")
_HEADING_RE = re.compile(r"^(#{1,6})\s*(.*)$")
_HR_RE = re.compile(r"^([*_-])\1{2,}$")
_CAPTION_RE = re.compile(
    r"^(图|表|Table|Figure)\s*[0-9０-９一二三四五六七八九十百]*\s*[：:.]", re.IGNORECASE
)
_BULLET_RE = re.compile(r"^(\s*)([-*+])\s+(.*)$")
_ORDERED_RE = re.compile(r"^(\s*)(\d{1,2})[.)]\s+(.*)$")
_QUOTE_RE = re.compile(r"^(\s*)>\s?(.*)$")
_LINK_RE = re.compile(r'\[([^\]\n]+)\]\(([^)\s]+)\)|<((?:https?|mailto):[^>\s]+)>')
_MD_ESCAPE_RE = re.compile(r"\\([\\`*_{}\[\]()#+\-.!|>~])")


def _unescape_md(text: str) -> str:
    return _MD_ESCAPE_RE.sub(r"\1", text)


def split_paired_marker(text: str, marker: str) -> List[Tuple[str, bool]]:
    """按成对出现的标记切分文本，返回 [(片段, 是否在标记内)]。

    与 Rust `split_paired_marker`（tools.rs L1510-1525）语义一致：奇数个标记结尾时，
    最后一段视为在标记内，容忍不闭合的 Markdown。
    """
    if marker not in text:
        return [(text, False)]
    parts: List[Tuple[str, bool]] = []
    inside = False
    rest = text
    while True:
        pos = rest.find(marker)
        if pos < 0:
            break
        parts.append((rest[:pos], inside))
        inside = not inside
        rest = rest[pos + len(marker):]
    parts.append((rest, inside))
    return [(seg, flag) for seg, flag in parts if seg]


def _styled_tokens(text: str, href: Optional[str]) -> List[RunToken]:
    """`代码` → **粗体** → *斜体*（嵌套顺序与 Rust parse_inline_runs 一致）。"""
    tokens: List[RunToken] = []
    for seg, mono in split_paired_marker(text, "`"):
        if mono:
            if seg:
                tokens.append(RunToken(seg, mono=True, href=href))
            continue
        for seg2, bold in split_paired_marker(seg, "**"):
            for seg3, italic in split_paired_marker(seg2, "*"):
                if seg3:
                    tokens.append(RunToken(seg3, bold=bold, italic=italic, href=href))
    return tokens


def inline_tokens(text: str) -> List[RunToken]:
    """先切 [文字](url) / <url>，再对剩余文本切代码/粗体/斜体。"""
    text = re.sub(r"!\[([^\]]*)\]\(([^)]+)\)", r"[\1](\2)", text)  # 图片降级为链接文本
    tokens: List[RunToken] = []
    pos = 0
    for match in _LINK_RE.finditer(text):
        if match.start() > pos:
            plain = _unescape_md(html.unescape(text[pos:match.start()]))
            tokens.extend(_styled_tokens(plain, None))
        if match.group(1) is not None:
            label, href = match.group(1), match.group(2)
        else:
            label, href = match.group(3), match.group(3)
        href = href.strip()
        if re.match(r"^(https?|mailto):", href):
            tokens.extend(_styled_tokens(_unescape_md(html.unescape(label)), href))
        else:  # 非法 URL：保留原文本，不建关系
            tokens.extend(_styled_tokens(_unescape_md(html.unescape(match.group(0))), None))
        pos = match.end()
    if pos < len(text):
        plain = _unescape_md(html.unescape(text[pos:]))
        tokens.extend(_styled_tokens(plain, None))
    if not tokens:
        tokens.append(RunToken(""))
    return tokens


def extract_front_matter(lines: List[str]) -> Tuple[Dict[str, str], List[str]]:
    """识别文件最前面的 --- YAML 块；只接受 key: value，否则整段当正文。"""
    idx = 0
    while idx < len(lines) and not lines[idx].strip():
        idx += 1
    if idx >= len(lines) or lines[idx].strip() != "---":
        return {}, lines
    for end in range(idx + 1, min(len(lines), idx + 60)):
        if lines[end].strip() not in ("---", "..."):
            continue
        meta: Dict[str, str] = {}
        for raw in lines[idx + 1:end]:
            if not raw.strip():
                continue
            match = _FM_KEY_RE.match(raw.strip())
            if not match:
                return {}, lines
            value = match.group(2).strip()
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            meta[match.group(1).lower()] = value
        return meta, lines[end + 1:]
    return {}, lines


def is_table_separator(line: str) -> bool:
    chars = set(line.strip())
    return bool(chars) and chars <= set("|-: \t") and "-" in chars


def split_table_row(line: str) -> List[str]:
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|") and not s.endswith("\\|"):
        s = s[:-1]
    cells = re.split(r"(?<!\\)\|", s)
    return [cell.strip().replace("\\|", "|") for cell in cells]


def _strip_trailing_hashes(text: str) -> str:
    return re.sub(r"\s+#+\s*$", "", text).strip()


def parse_markdown(markdown: str) -> Tuple[Dict[str, str], List[Node]]:
    """把 Markdown 解析成 block 节点列表；软换行在 Node.lines 中逐行保留。"""
    text = markdown.replace("\r\n", "\n").replace("\r", "\n")
    if text.startswith("\ufeff"):
        text = text[1:]
    lines = text.split("\n")
    meta, lines = extract_front_matter(lines)

    nodes: List[Node] = []
    state: Dict[str, object] = {"para": [], "list": None, "table": None, "quote": None, "code": None}

    def flush_para() -> None:
        para = state["para"]
        if para:
            nodes.append(Node(kind="body", lines=list(para)))
            state["para"] = []

    def flush_list() -> None:
        cur = state["list"]
        if cur:
            nodes.append(
                Node(
                    kind="list",
                    ordered=bool(cur["ordered"]),
                    start=int(cur["start"]),
                    items=[(lv, list(ls)) for lv, ls in cur["items"]],
                )
            )
            state["list"] = None

    def flush_table() -> None:
        cur = state["table"]
        if cur and cur["rows"]:
            nodes.append(Node(kind="table", rows=[list(r) for r in cur["rows"]]))
        state["table"] = None

    def flush_quote() -> None:
        cur = state["quote"]
        if cur:
            nodes.append(Node(kind="quote", lines=list(cur["lines"])))
            state["quote"] = None

    def flush_code() -> None:
        cur = state["code"]
        if cur is not None:
            if cur:
                nodes.append(Node(kind="code", lines=list(cur)))
            state["code"] = None

    def flush_all() -> None:
        flush_para()
        flush_list()
        flush_table()
        flush_quote()

    i = 0
    total = len(lines)
    while i < total:
        raw = lines[i]
        line = raw.rstrip()
        stripped = line.strip()

        # ---- 代码块（围栏内不解析任何 Markdown）----
        if state["code"] is not None:
            if stripped.startswith("```"):
                flush_code()
            else:
                state["code"].append(raw)
            i += 1
            continue
        if stripped.startswith("```"):
            flush_all()
            state["code"] = []
            i += 1
            continue

        # ---- 空行：结束所有块 ----
        if not stripped:
            flush_all()
            i += 1
            continue

        # ---- 表格行 ----
        if stripped.startswith("|") and "|" in stripped[1:]:
            if is_table_separator(stripped):
                i += 1
                continue
            flush_para()
            flush_list()
            flush_quote()
            if state["table"] is None:
                state["table"] = {"rows": []}
            state["table"]["rows"].append(split_table_row(stripped))
            i += 1
            continue
        flush_table()

        # ---- Setext H1（=====），`---` 一律按水平分隔线处理，避免误判新闻正文 ----）
        if i + 1 < total and not stripped.startswith(("#", "|", ">")):
            nxt = lines[i + 1].strip()
            if nxt and len(nxt) >= 2 and set(nxt) == {"="}:
                flush_all()
                nodes.append(Node(kind="heading", level=1, text=stripped))
                i += 2
                continue

        # ---- ATX 标题 ----
        match = _HEADING_RE.match(stripped)
        if match:
            level = min(3, len(match.group(1)))
            heading_text = _strip_trailing_hashes(match.group(2))
            flush_all()
            if heading_text:
                nodes.append(Node(kind="heading", level=level, text=heading_text))
            i += 1
            continue

        # ---- 水平分隔线：--- / *** / ___ → 段落下框线（Divider 样式）----
        if _HR_RE.match(stripped):
            flush_all()
            nodes.append(Node(kind="hr"))
            i += 1
            continue

        # ---- 题注：图 1 / 表 2：… ----
        if _CAPTION_RE.match(stripped):
            flush_all()
            nodes.append(Node(kind="caption", text=stripped))
            i += 1
            continue

        # ---- 列表项 ----
        bullet = _BULLET_RE.match(raw)
        ordered = _ORDERED_RE.match(raw)
        if bullet or ordered:
            flush_para()
            flush_table()
            flush_quote()
            indent = len((bullet or ordered).group(1).expandtabs(4))
            if bullet:
                kind, item_text, number = "bullet", bullet.group(3), 1
            else:
                kind, item_text, number = "ordered", ordered.group(3), int(ordered.group(2))
            cur = state["list"]
            if cur is None or bool(cur["ordered"]) != (kind == "ordered"):
                flush_list()
                cur = {
                    "ordered": kind == "ordered",
                    "start": number,
                    "indents": [indent],
                    "items": [],
                    "last_indent": indent,
                }
                state["list"] = cur
                ilvl = 0
            else:
                indents = cur["indents"]
                while len(indents) > 1 and indent < indents[-1]:
                    indents.pop()
                if indent > indents[-1]:
                    indents.append(indent)
                cur["last_indent"] = indent
                ilvl = min(3, len(indents) - 1)
            cur["items"].append((ilvl, [item_text]))
            i += 1
            continue

        # ---- 列表项续行（比当前项缩进更深）→ 并入上一项，保留 w:br ----
        if state["list"] is not None and raw[:1] in (" ", "\t"):
            indent = len(raw) - len(raw.lstrip())
            if indent >= int(state["list"]["last_indent"]) + 2:
                state["list"]["items"][-1][1].append(stripped)
                i += 1
                continue

        # ---- 引用块 ----
        quote = _QUOTE_RE.match(line)
        if quote:
            flush_para()
            flush_list()
            flush_table()
            if state["quote"] is None:
                state["quote"] = {"lines": []}
            state["quote"]["lines"].append(quote.group(2).strip())
            i += 1
            continue

        # ---- 普通段落：同一段的多行全部保留（渲染时用 w:br 软换行）----
        flush_list()
        flush_quote()
        state["para"].append(stripped)
        i += 1

    flush_all()
    flush_code()

    rebase_heading_levels(nodes)
    return meta, nodes


def rebase_heading_levels(nodes: List[Node]) -> None:
    """标题层级归一：正文最小标题级别上移为 1。

    例如 front matter 已提供文档标题、正文用 `##`/`###` 表示一/二级章节时，
    渲染后分别成为 Heading1 / Heading2；文档标题不会作为正文 heading 重复出现。
    """
    levels = [node.level for node in nodes if node.kind == "heading"]
    if not levels:
        return
    minimum = min(levels)
    if minimum <= 1:
        return
    shift = minimum - 1
    for node in nodes:
        if node.kind == "heading":
            node.level = max(1, min(3, node.level - shift))
# ---------------------------------------------------------------------------
# 5. word/styles.xml：命名样式（样式驱动排版的核心）
# ---------------------------------------------------------------------------

# 约定：Heading1/2/3 的 outlineLvl = 0/1/2（Word 导航窗格与 TOC 依据）；
# 其余命名样式显式写 outlineLvl=9（body text level），确保它们不会被 TOC 收录。
STYLES_XML_BODY = """
<w:docDefaults>
<w:rPrDefault><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:kern w:val="2"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="zh-CN" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr><w:widowControl/><w:kinsoku/><w:overflowPunct/><w:autoSpaceDE/><w:autoSpaceDN/><w:snapToGrid/><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault>
</w:docDefaults>
<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/><w:pPr><w:widowControl/><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Subtitle"/><w:uiPriority w:val="10"/><w:qFormat/><w:pPr><w:spacing w:before="0" w:after="360"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="微软雅黑" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="1F3864"/><w:sz w:val="52"/><w:szCs w:val="52"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="11"/><w:qFormat/><w:pPr><w:spacing w:before="0" w:after="240"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="微软雅黑" w:cs="Arial"/><w:color w:val="595959"/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="360" w:after="180"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="1F3864"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="280" w:after="140"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="2E5496"/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="333333"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="BodyText"><w:name w:val="Body Text"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="20"/><w:qFormat/><w:pPr><w:widowControl/><w:spacing w:line="360" w:lineRule="auto" w:after="120"/><w:ind w:firstLineChars="200" w:firstLine="480"/><w:jc w:val="both"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="Caption"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="35"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="120" w:after="60"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="333333"/><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="29"/><w:qFormat/><w:pPr><w:pBdr><w:left w:val="single" w:sz="12" w:space="8" w:color="B7C3D9"/></w:pBdr><w:shd w:val="clear" w:color="auto" w:fill="F5F8FC"/><w:spacing w:before="120" w:after="180" w:line="320" w:lineRule="auto"/><w:ind w:left="480" w:right="360"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:color w:val="404040"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="CodeBlock"><w:name w:val="Code Block"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:pBdr><w:top w:val="single" w:sz="4" w:space="4" w:color="E2E2E5"/><w:left w:val="single" w:sz="4" w:space="4" w:color="E2E2E5"/><w:bottom w:val="single" w:sz="4" w:space="4" w:color="E2E2E5"/><w:right w:val="single" w:sz="4" w:space="4" w:color="E2E2E5"/></w:pBdr><w:shd w:val="clear" w:color="auto" w:fill="F5F5F7"/><w:spacing w:before="120" w:after="180" w:line="260" w:lineRule="auto"/><w:ind w:left="240" w:right="240"/><w:contextualSpacing/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:eastAsia="宋体" w:cs="Consolas"/><w:color w:val="1A1A1A"/><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="34"/><w:qFormat/><w:pPr><w:spacing w:after="60" w:line="320" w:lineRule="auto"/><w:contextualSpacing/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TableHeader"><w:name w:val="Table Header"/><w:basedOn w:val="Normal"/><w:next w:val="TableText"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:spacing w:before="40" w:after="40" w:line="240" w:lineRule="auto"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="1F3864"/><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TableText"><w:name w:val="Table Text"/><w:basedOn w:val="Normal"/><w:next w:val="TableText"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:spacing w:before="40" w:after="40" w:line="260" w:lineRule="auto"/><w:jc w:val="left"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TOCHeading"><w:name w:val="TOC Heading"/><w:basedOn w:val="Normal"/><w:next w:val="TOC1"/><w:uiPriority w:val="39"/><w:qFormat/><w:pPr><w:spacing w:before="240" w:after="180"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="1F3864"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TOC1"><w:name w:val="toc 1"/><w:basedOn w:val="Normal"/><w:next w:val="TOC1"/><w:uiPriority w:val="39"/><w:qFormat/><w:pPr><w:spacing w:after="60" w:line="300" w:lineRule="auto"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体" w:cs="Arial"/><w:b/><w:bCs/><w:color w:val="1F3864"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TOC2"><w:name w:val="toc 2"/><w:basedOn w:val="Normal"/><w:next w:val="TOC2"/><w:uiPriority w:val="39"/><w:qFormat/><w:pPr><w:spacing w:after="40" w:line="280" w:lineRule="auto"/><w:ind w:left="240"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:color w:val="333333"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TOC3"><w:name w:val="toc 3"/><w:basedOn w:val="Normal"/><w:next w:val="TOC3"/><w:uiPriority w:val="39"/><w:qFormat/><w:pPr><w:spacing w:after="40" w:line="280" w:lineRule="auto"/><w:ind w:left="480"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:color w:val="595959"/><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Divider"><w:name w:val="Divider"/><w:basedOn w:val="Normal"/><w:next w:val="BodyText"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="6" w:color="BFBFBF"/></w:pBdr><w:spacing w:before="120" w:after="180"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:sz w:val="12"/><w:szCs w:val="12"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Header"><w:name w:val="header"/><w:basedOn w:val="Normal"/><w:next w:val="Header"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="4" w:color="BFBFBF"/></w:pBdr><w:spacing w:after="0"/><w:jc w:val="right"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:color w:val="666666"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Footer"><w:name w:val="footer"/><w:basedOn w:val="Normal"/><w:next w:val="Footer"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:spacing w:before="0" w:after="0"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:color w:val="666666"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="CoverMeta"><w:name w:val="Cover Meta"/><w:basedOn w:val="Normal"/><w:next w:val="CoverMeta"/><w:uiPriority w:val="99"/><w:qFormat/><w:pPr><w:spacing w:before="0" w:after="120"/><w:jc w:val="center"/><w:outlineLvl w:val="9"/></w:pPr><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/><w:color w:val="595959"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="99"/><w:unhideWhenUsed/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
"""


def styles_part() -> str:
    return XML_DECL + f'<w:styles xmlns:w="{W_NS}">' + STYLES_XML_BODY + "</w:styles>"


# ---------------------------------------------------------------------------
# 6. document.xml 渲染
# ---------------------------------------------------------------------------

TOTAL_TABLE_WIDTH = CONTENT_W  # 9026 twips
MIN_COL_TWIPS = 620


def allocate_table_widths(rows: Sequence[Sequence[str]]) -> List[int]:
    """按内容分配表格列宽（总宽 9026 twips，CJK 宽字符按 2 个长度单位）。

    算法：每列先按最长内容估算权重（中文=2、西文=1，再 +1 余量），
    以 MIN_COL_TWIPS 保底，其余宽度按权重等比分配；最后用最大余数法保证
    各列 tcW / gridCol 之和精确等于 9026。
    """
    cols = max((len(row) for row in rows), default=1) or 1
    if cols == 1:
        return [TOTAL_TABLE_WIDTH]
    units: List[int] = []
    for ci in range(cols):
        longest = 0
        for row in rows:
            text = plain_text(row[ci]) if ci < len(row) else ""
            longest = max(longest, display_units(text))
        units.append(max(4, longest + 1))
    min_width = min(MIN_COL_TWIPS, TOTAL_TABLE_WIDTH // cols)
    remaining = TOTAL_TABLE_WIDTH - min_width * cols
    total_units = sum(units)
    raw_widths = [min_width + remaining * u / total_units for u in units]
    widths = [int(w) for w in raw_widths]
    remainder = TOTAL_TABLE_WIDTH - sum(widths)
    order = sorted(range(cols), key=lambda idx: raw_widths[idx] - widths[idx], reverse=True)
    for k in range(remainder):
        widths[order[k % cols]] += 1
    assert sum(widths) == TOTAL_TABLE_WIDTH, widths
    return widths


def _metrics_text(nodes: Sequence[Node]) -> str:
    chunks: List[str] = []
    for node in nodes:
        if node.text:
            chunks.append(node.text)
        chunks.extend(node.lines)
        for row in node.rows:
            chunks.extend(row)
        for _, item_lines in node.items:
            chunks.extend(item_lines)
    return "\n".join(chunks)


def content_stats(nodes: Sequence[Node], document_xml: str) -> Dict[str, int]:
    text = _metrics_text(nodes)
    cjk = len(re.findall(r"[\u3400-\u9fff]", text))
    latin_words = len(re.findall(r"[A-Za-z0-9_]+", text))
    chars = len(re.sub(r"\s", "", text))
    paragraphs = document_xml.count("<w:p>")
    # 粗略页数：封面 + 目录各 1 页，正文按 1300 非空白字符/页估算
    pages = 2 + max(1, chars // 1300 + 1)
    return {
        "words": cjk + latin_words,
        "characters": chars,
        "characters_with_spaces": len(text),
        "lines": paragraphs,
        "paragraphs": paragraphs,
        "pages": pages,
    }


class DocxRenderer:
    """把 block 节点渲染为 document.xml；同时登记编号实例与超链接关系。"""

    def __init__(self, meta: Dict[str, str], nodes: Sequence[Node], include_toc: bool = True):
        self.meta = meta
        self.nodes = list(nodes)
        self.include_toc = include_toc
        self.link_rels: List[Tuple[str, str]] = []
        self._href_rid: Dict[str, str] = {}
        self._next_rid = FIRST_DYNAMIC_RID
        self.number_instances: List[Tuple[str, int]] = []

    # ---- 关系 id ----
    def _rid_for_href(self, target: str) -> str:
        if target in self._href_rid:
            return self._href_rid[target]
        rid = f"rId{self._next_rid}"
        self._next_rid += 1
        self._href_rid[target] = rid
        self.link_rels.append((rid, target))
        return rid

    # ---- 原子构件 ----
    def _plain_run(self, text: str, rpr: str = "") -> str:
        if not text:
            return ""
        rpr_xml = f"<w:rPr>{rpr}</w:rPr>" if rpr else ""
        return f'<w:r>{rpr_xml}<w:t xml:space="preserve">{escape_xml(text)}</w:t></w:r>'

    def _run_xml(self, token: RunToken) -> str:
        if not token.text:
            return ""
        rpr = ""
        if token.href:
            rpr += '<w:rStyle w:val="Hyperlink"/>'
        if token.mono:
            rpr += (
                '<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" '
                'w:eastAsia="宋体" w:cs="Consolas"/>'
            )
        if token.bold:
            rpr += "<w:b/><w:bCs/>"
        if token.italic:
            rpr += "<w:i/><w:iCs/>"
        if token.href or token.mono:
            color = "0563C1" if token.href else "1F3864"
            rpr += f'<w:color w:val="{color}"/>'
        if token.mono:
            rpr += '<w:sz w:val="22"/><w:szCs w:val="22"/>'
        if token.href:
            rpr += '<w:u w:val="single"/>'
        rpr_xml = f"<w:rPr>{rpr}</w:rPr>" if rpr else ""
        text_xml = f'<w:t xml:space="preserve">{escape_xml(token.text)}</w:t>'
        if token.href:
            rid = self._rid_for_href(token.href)
            return (
                f'<w:hyperlink r:id="{rid}" w:history="1">'
                f"<w:r>{rpr_xml}{text_xml}</w:r></w:hyperlink>"
            )
        return f"<w:r>{rpr_xml}{text_xml}</w:r>"

    def _runs(self, text: str) -> str:
        return "".join(self._run_xml(token) for token in inline_tokens(text))

    def _line_runs(self, lines: Sequence[str], render_inline: bool = True) -> str:
        """多行合并进同一段落：行间用 w:br（保留 Markdown 软换行）。"""
        runs = ""
        for idx, line in enumerate(lines):
            if idx:
                runs += "<w:r><w:br/></w:r>"
            runs += self._runs(line) if render_inline else self._plain_run(line)
        return runs

    def _p(self, style_id: str, runs: str, extra_ppr: str = "") -> str:
        return (
            f'<w:p><w:pPr><w:pStyle w:val="{style_id}"/>{extra_ppr}</w:pPr>'
            f"{runs}</w:p>"
        )

    # ---- 封面 / 目录 / 分节 ----
    def _cover(self) -> str:
        parts = [
            '<w:p><w:pPr><w:spacing w:before="2100" w:line="240" w:lineRule="auto"/></w:pPr></w:p>',
            self._p("Title", self._plain_run(self.meta["title"])),
        ]
        if self.meta.get("subtitle"):
            parts.append(self._p("Subtitle", self._plain_run(self.meta["subtitle"])))
        parts.append(self._p("Divider", ""))
        parts.append(self._p("CoverMeta", self._plain_run(f"作　者：{self.meta['author']}")))
        parts.append(self._p("CoverMeta", self._plain_run(f"日　期：{self.meta['date']}")))
        if self.meta.get("org"):
            parts.append(self._p("CoverMeta", self._plain_run(f"出　品：{self.meta['org']}")))
        return "".join(parts)

    def _cover_section_break(self) -> str:
        """封面节：nextPage 分节；titlePg（首页不同）；不引用 header/footer → 封面无页眉页脚。"""
        return (
            "<w:p><w:pPr><w:sectPr>"
            '<w:type w:val="nextPage"/>'
            f'<w:pgSz w:w="{PAGE_W}" w:h="{PAGE_H}"/>'
            f'<w:pgMar w:top="{MARGIN_TB}" w:right="{MARGIN_LR}" w:bottom="{MARGIN_TB}" '
            f'w:left="{MARGIN_LR}" w:header="{HEADER_DIST}" w:footer="{FOOTER_DIST}" w:gutter="0"/>'
            "<w:titlePg/>"
            '<w:docGrid w:type="lines" w:linePitch="312"/>'
            "</w:sectPr></w:pPr></w:p>"
        )

    def _toc(self) -> str:
        """目录：TOC 域 + TOCHeading 标题；之后的 page break 让正文另起一页。"""
        heading = self._p("TOCHeading", self._plain_run("目录"))
        field = (
            '<w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr>'
            '<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
            '<w:r><w:instrText xml:space="preserve"> TOC \\o "1-3" \\h \\z \\u </w:instrText></w:r>'
            '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
            '<w:r><w:rPr><w:color w:val="808080"/></w:rPr>'
            "<w:t>（目录将在 Word 打开时自动更新；若未更新，请右键选择“更新域”。）</w:t></w:r>"
            '<w:r><w:fldChar w:fldCharType="end"/></w:r>'
            "</w:p>"
        )
        page_break = '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
        return heading + field + page_break

    def _final_section_props(self) -> str:
        """正文节：引用页眉/页脚，页码从 1 开始；A4 + 页边距 + 中文文档网格。"""
        return (
            "<w:sectPr>"
            f'<w:headerReference w:type="default" r:id="{RID_HEADER}"/>'
            f'<w:footerReference w:type="default" r:id="{RID_FOOTER}"/>'
            f'<w:pgSz w:w="{PAGE_W}" w:h="{PAGE_H}"/>'
            f'<w:pgMar w:top="{MARGIN_TB}" w:right="{MARGIN_LR}" w:bottom="{MARGIN_TB}" '
            f'w:left="{MARGIN_LR}" w:header="{HEADER_DIST}" w:footer="{FOOTER_DIST}" w:gutter="0"/>'
            '<w:pgNumType w:start="1"/>'
            '<w:docGrid w:type="lines" w:linePitch="312"/>'
            "</w:sectPr>"
        )

    # ---- 内容元素 ----
    def _heading(self, level: int, text: str) -> str:
        style = {1: "Heading1", 2: "Heading2", 3: "Heading3"}.get(level, "Heading3")
        return self._p(style, self._runs(text))

    def _body(self, lines: Sequence[str]) -> str:
        return self._p("BodyText", self._line_runs(lines))

    def _quote(self, lines: Sequence[str]) -> str:
        return self._p("Quote", self._line_runs(lines))

    def _code(self, lines: Sequence[str]) -> str:
        runs = ""
        for idx, line in enumerate(lines):
            if idx:
                runs += "<w:r><w:br/></w:r>"
            if line:
                runs += self._plain_run(line)
        return self._p("CodeBlock", runs)

    def _divider(self) -> str:
        # 直接用 pBdr 下框线（同时保留 Divider 样式，Word 中表现为一条水平线）
        return (
            '<w:p><w:pPr><w:pStyle w:val="Divider"/>'
            '<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="6" w:color="BFBFBF"/></w:pBdr>'
            "</w:pPr></w:p>"
        )

    def _caption(self, text: str) -> str:
        return self._p("Caption", self._runs(text))

    def _list(self, node: Node) -> str:
        kind = "ordered" if node.ordered else "bullet"
        self.number_instances.append((kind, max(1, int(node.start))))
        num_id = len(self.number_instances)
        parts: List[str] = []
        for ilvl, item_lines in node.items:
            num_pr = (
                f'<w:numPr><w:ilvl w:val="{ilvl}"/>'
                f'<w:numId w:val="{num_id}"/></w:numPr>'
            )
            parts.append(self._p("ListParagraph", self._line_runs(item_lines), num_pr))
        return "".join(parts)

    def _table(self, rows: Sequence[Sequence[str]]) -> str:
        if not rows:
            return ""
        cols = max(len(row) for row in rows) or 1
        normalized = [list(row) + [""] * (cols - len(row)) for row in rows]
        widths = allocate_table_widths(normalized)
        border = 'w:val="single" w:sz="4" w:space="0" w:color="BFC7D6"'
        table_props = (
            "<w:tblPr>"
            f'<w:tblW w:w="{TOTAL_TABLE_WIDTH}" w:type="dxa"/>'
            "<w:tblBorders>"
            f"<w:top {border}/><w:left {border}/><w:bottom {border}/><w:right {border}/>"
            f"<w:insideH {border}/><w:insideV {border}/>"
            "</w:tblBorders>"
            '<w:tblLayout w:type="fixed"/>'
            "<w:tblCellMar>"
            '<w:top w:w="60" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
            '<w:bottom w:w="60" w:type="dxa"/><w:right w:w="108" w:type="dxa"/>'
            "</w:tblCellMar>"
            "</w:tblPr>"
        )
        grid = "".join(f'<w:gridCol w:w="{width}"/>' for width in widths)
        rows_xml: List[str] = []
        for ri, row in enumerate(normalized):
            is_header = ri == 0
            tr_pr = "<w:trPr><w:cantSplit/>"
            if is_header:
                tr_pr += "<w:tblHeader/>"
            tr_pr += "</w:trPr>"
            cells_xml = ""
            for ci, cell in enumerate(row):
                shading = ""
                if is_header:
                    shading = (
                        '<w:shd w:val="clear" w:color="auto" w:fill="DCE6F2"/>'
                    )
                elif ri % 2 == 0:
                    shading = (
                        '<w:shd w:val="clear" w:color="auto" w:fill="F4F6FA"/>'
                    )
                cell_style = "TableHeader" if is_header else "TableText"
                cells_xml += (
                    f'<w:tc><w:tcPr><w:tcW w:w="{widths[ci]}" w:type="dxa"/>{shading}'
                    '<w:vAlign w:val="center"/></w:tcPr>'
                    f'<w:p><w:pPr><w:pStyle w:val="{cell_style}"/></w:pPr>'
                    f"{self._runs(cell)}</w:p></w:tc>"
                )
            rows_xml.append(f"<w:tr>{tr_pr}{cells_xml}</w:tr>")
        table_xml = (
            f"<w:tbl>{table_props}<w:tblGrid>{grid}</w:tblGrid>"
            + "".join(rows_xml)
            + "</w:tbl>"
        )
        # 表后补一个 6pt 空段落，保证表格与后续内容之间有呼吸空间
        spacer = (
            '<w:p><w:pPr><w:spacing w:before="0" w:after="120" w:line="240" w:lineRule="auto"/>'
            '<w:rPr><w:sz w:val="12"/><w:szCs w:val="12"/></w:rPr></w:pPr></w:p>'
        )
        return table_xml + spacer

    def _render_node(self, node: Node) -> str:
        if node.kind == "heading":
            return self._heading(node.level, node.text)
        if node.kind == "body":
            return self._body(node.lines)
        if node.kind == "quote":
            return self._quote(node.lines)
        if node.kind == "code":
            return self._code(node.lines)
        if node.kind == "list":
            return self._list(node)
        if node.kind == "table":
            return self._table(node.rows)
        if node.kind == "hr":
            return self._divider()
        if node.kind == "caption":
            return self._caption(node.text)
        return ""

    # ---- 组装 ----
    def render(self, document_stats: Optional[Dict[str, int]] = None) -> str:
        body = self._cover()
        body += self._cover_section_break()
        if self.include_toc:
            body += self._toc()
        rendered = "".join(self._render_node(node) for node in self.nodes)
        if not rendered:
            rendered = self._p("BodyText", self._plain_run("（源文档没有可渲染的正文内容。）"))
        body += rendered
        body += self._final_section_props()
        return (
            XML_DECL
            + f'<w:document xmlns:w="{W_NS}" xmlns:r="{R_NS}">'
            + f"<w:body>{body}</w:body></w:document>"
        )


# ---------------------------------------------------------------------------
# 7. 元数据解析 / 构建入口 / CLI
# ---------------------------------------------------------------------------


def _read_source(path: str) -> str:
    for encoding in ("utf-8-sig", "gb18030"):
        try:
            with open(path, "r", encoding=encoding) as handle:
                return handle.read()
        except UnicodeDecodeError:
            continue
        except OSError as exc:
            raise SystemExit(f"读取源文件失败：{path}（{exc}）")
    raise SystemExit(f"无法解码源文件（已尝试 UTF-8 / GB18030）：{path}")


def _norm_title(text: str) -> str:
    return re.sub(r"\s+", "", plain_text(text))


def _format_cover_date(value: Optional[str]) -> str:
    if not value:
        value = _dt.date.today().isoformat()
    match = re.match(r"^(\d{4})-(\d{1,2})-(\d{1,2})$", value.strip())
    if match:
        return (
            f"{match.group(1)} 年 {int(match.group(2))} 月 {int(match.group(3))} 日"
        )
    return value.strip()


def _drop_duplicate_title(nodes: List[Node], title: str) -> List[Node]:
    """若正文首个 Heading1 与封面标题同名，删掉它以保持"标题只在封面出现一次"。"""
    if not nodes:
        return nodes
    first = nodes[0]
    if first.kind == "heading" and first.level == 1 and _norm_title(first.text) == _norm_title(title):
        nodes = nodes[1:]
        rebase_heading_levels(nodes)
    return nodes


def build_docx(
    src_path: str,
    out_path: str,
    title: Optional[str] = None,
    author: Optional[str] = None,
    subtitle: Optional[str] = None,
    date: Optional[str] = None,
    org: Optional[str] = None,
    subject: Optional[str] = None,
    include_toc: bool = True,
    created: Optional[str] = None,
) -> Dict[str, object]:
    markdown = _read_source(src_path)
    front_matter, nodes = parse_markdown(markdown)

    resolved_title = title or front_matter.get("title")
    if not resolved_title and nodes and nodes[0].kind == "heading" and nodes[0].level == 1:
        resolved_title = nodes[0].text
    if not resolved_title:
        resolved_title = os.path.splitext(os.path.basename(out_path))[0]
    nodes = _drop_duplicate_title(nodes, resolved_title)

    author_value = author or front_matter.get("author") or "AI 桌面宠物"
    meta: Dict[str, str] = {
        "title": plain_text(resolved_title),
        "subtitle": plain_text(subtitle or front_matter.get("subtitle", "")),
        "author": plain_text(author_value),
        "date": _format_cover_date(date or front_matter.get("date")),
        "org": plain_text(org or front_matter.get("org", "")),
        "subject": plain_text(subject or front_matter.get("subject", "") or front_matter.get("subtitle", "")),
        "keywords": plain_text(front_matter.get("keywords", "AI, 研究报告, OOXML")),
        "category": "研究报告",
    }

    renderer = DocxRenderer(meta, nodes, include_toc=include_toc)
    document_xml = renderer.render()
    stats = content_stats(nodes, document_xml)

    parts = {
        "[Content_Types].xml": package_content_types(),
        "_rels/.rels": package_rels(),
        "word/document.xml": document_xml,
        "word/styles.xml": styles_part(),
        "word/numbering.xml": numbering_part(renderer.number_instances),
        "word/settings.xml": document_settings(),
        "word/theme/theme1.xml": theme_part(),
        "word/fontTable.xml": font_table_part(),
        "word/webSettings.xml": web_settings_part(),
        "word/header1.xml": header_part(meta["title"]),
        "word/footer1.xml": footer_part(),
        "word/_rels/document.xml.rels": document_rels(renderer.link_rels),
        "docProps/core.xml": core_properties(meta, created=created),
        "docProps/app.xml": extended_properties(meta, stats),
    }
    size = write_package(out_path, parts)

    counts = {
        "paragraphs": document_xml.count("<w:p>"),
        "pStyle": document_xml.count("<w:pStyle"),
        "numPr": document_xml.count("<w:numPr>"),
        "hyperlink": document_xml.count("<w:hyperlink "),
        "table": document_xml.count("<w:tbl>"),
        "list": len(renderer.number_instances),
        "br": document_xml.count("<w:br/>"),
        "divider": document_xml.count('w:pStyle w:val="Divider"'),
        "external_rel": len(renderer.link_rels),
        "heading": sum(1 for node in nodes if node.kind == "heading"),
    }
    return {
        "out": out_path,
        "size": size,
        "part_count": len(parts),
        "meta": meta,
        "stats": stats,
        "counts": counts,
        "parts": parts,
    }


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Markdown → 专业报告级 .docx（手写 OOXML + zipfile 参考实现）"
    )
    parser.add_argument("--src", required=True, help="输入 Markdown 文件路径")
    parser.add_argument("--out", required=True, help="输出 .docx 文件路径")
    parser.add_argument("--title", help="文档标题（默认取 front matter title 或首个 H1）")
    parser.add_argument("--author", help="作者（默认取 front matter author）")
    parser.add_argument("--subtitle", help="封面副标题")
    parser.add_argument("--date", help="封面日期，如 2026-11-14")
    parser.add_argument("--org", help="出品方")
    parser.add_argument("--subject", help="文档主题（写入 core.xml）")
    parser.add_argument("--no-toc", action="store_true", help="不插入目录页")
    args = parser.parse_args(argv)

    result = build_docx(
        src_path=args.src,
        out_path=args.out,
        title=args.title,
        author=args.author,
        subtitle=args.subtitle,
        date=args.date,
        org=args.org,
        subject=args.subject,
        include_toc=not args.no_toc,
    )
    meta = result["meta"]
    counts = result["counts"]
    stats = result["stats"]
    print(f"已生成：{result['out']}  ({result['size'] / 1024:.1f} KB, {result['part_count']} 个 part)")
    print(f"标题：{meta['title']}")
    print(f"副标题：{meta['subtitle'] or '（无）'}")
    print(f"作者：{meta['author']}    日期：{meta['date']}")
    print(
        "内容统计："
        f"段落 {counts['paragraphs']} / 标题 {counts['heading']} / 表格 {counts['table']} / "
        f"列表 {counts['list']} / 超链接 {counts['hyperlink']} / "
        f"pStyle {counts['pStyle']} / numPr {counts['numPr']} / 软换行 {counts['br']} / "
        f"分隔线 {counts['divider']}"
    )
    print(
        "估算："
        f"约 {stats['pages']} 页 / {stats['words']} 词 / {stats['characters']} 字符"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
