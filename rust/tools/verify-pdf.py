"""Read-only extraction/geometry check of a native :verification: export.

Run with the user's conda y interpreter. Rendering is a separate Poppler check.
No real workspaces, model accounts, or foreground windows are accessed.
"""
import json
import re
import sys
import io
import subprocess
from pathlib import Path
from pypdf import PdfReader, PdfWriter
from pypdf.generic import DecodedStreamObject, NameObject
import pdfplumber
from PIL import Image, ImageChops


def compact(text):
    return re.sub(r"\s+", "", text)


path = Path(sys.argv[1]).resolve(strict=True)
printing = len(sys.argv) > 2 and sys.argv[2] == "print"
reader = PdfReader(path, strict=True)
texts = [page.extract_text() for page in reader.pages]
with pdfplumber.open(path) as pdf:
    other_texts = [page.extract_text(x_tolerance=2) or "" for page in pdf.pages]
    chars = [char for page in pdf.pages for char in page.chars]
    for char in chars:
        assert 0 <= char["x0"] <= char["x1"] <= 595.02, char
        assert -0.02 <= char["top"] <= char["bottom"] <= 842.02, char

required = ["墨池导出验证", "Englishsearchabletext0123456789.", "😀", "𠀀", "e\u0301",
            "保持原文", "中文表格", "搜索与复制", "中文代码42", "全量保真检查",
            r"\frac{a+b}{c}=\sqrt{x}", "x^2+y^2=z^2"]
if printing:
    required = ["QUOTEBEGIN", "QUOTEEND", "PRINTEND"]
for engine, pages in [("pypdf", texts), ("pdfplumber", other_texts)]:
    joined = compact("\n".join(pages))
    for token in required:
        assert token in joined, (engine, "missing", token, joined[:1300])
    assert len(pages) >= 3, (engine, "expected multi-page fixture")
    assert "代码块名称" not in joined, "editor placeholder leaked into print"
    groups = [("FILL", 39), ("CODE", 6), ("TABLE", 18), ("LONG", 80)] if printing else [("ROW", 100)]
    for prefix, count in groups:
        locations = []
        for row in range(count):
            marker = f"{prefix}{row:03}"
            assert joined.count(marker) == 1, (engine, "row count", marker, joined.count(marker))
            found = [i for i, page in enumerate(pages) if marker in compact(page)]
            assert len(found) == 1, (engine, "split row", marker)
            locations.extend(found)
        if prefix in ["CODE", "TABLE"]:
            assert len(set(locations)) == 1, (engine, "split atomic block", prefix, locations)
        if prefix == "LONG":
            assert len(set(locations)) >= 2, "long code must exercise continuation"
    if printing:
        assert any("QUOTEBEGIN" in page and "QUOTEEND" in page for page in pages), "split quote"
    else:
        alphabet = "".join(chr(0x4e00 + i) for i in range(300))
        assert alphabet in joined, (engine, "font subset mapping lost or reordered")

font_count = len(reader.pages[0]["/Resources"]["/Font"])
assert font_count >= (1 if printing else 2), "expected font subsets"
for font in reader.pages[0]["/Resources"]["/Font"].values():
    font = font.get_object()
    assert font["/Subtype"] == "/Type3" and font["/ToUnicode"].get_data()
    assert font["/CharProcs"], "font program must be embedded"

# 使用相同的 PDF 图像变换，比较有无语义层时的渲染结果。
# 如果直接与源像素比较，会把 Poppler 的重采样误判为差异。
for index, page in enumerate(reader.pages, 1):
    rendered = subprocess.run(["pdftoppm", "-r", "144", "-f", str(index), "-l", str(index),
                               "-png", str(path)], check=True, capture_output=True)
    assert not rendered.stderr.strip(), rendered.stderr.decode(errors="replace")
    raster = Image.open(io.BytesIO(rendered.stdout)).convert("RGB")
    writer = PdfWriter()
    baseline_page = writer.add_page(page)
    image_content = page.get_contents().get_data().partition(b"\nBT ")[0] + b"\n"
    assert b"/Im0 Do" in image_content and b" Tj" not in image_content
    content = DecodedStreamObject()
    content.set_data(image_content)
    baseline_page[NameObject("/Contents")] = writer._add_object(content)
    buffer = io.BytesIO()
    writer.write(buffer)
    baseline = subprocess.run(["pdftoppm", "-r", "144", "-png", "-"], input=buffer.getvalue(),
                              check=True, capture_output=True)
    assert not baseline.stderr.strip(), baseline.stderr.decode(errors="replace")
    original = Image.open(io.BytesIO(baseline.stdout)).convert("RGB")
    assert raster.size == original.size, (raster.size, original.size)
    assert ImageChops.difference(raster, original).getbbox() is None, (index, "text layer altered visual pixels")

print(json.dumps({"path": str(path), "pages": len(reader.pages), "font_subsets": font_count,
                  "characters": len(chars), "engines": ["pypdf", "pdfplumber"],
                  "fixture": "print" if printing else "unicode",
                  "all_numbered_rows_exactly_once": True, "atomic_blocks_passed": printing,
                  "unicode_and_geometry_passed": True, "text_layer_changes_zero_rendered_pixels": True}, ensure_ascii=False, indent=2))
