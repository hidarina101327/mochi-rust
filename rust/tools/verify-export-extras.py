"""Check the real App workflow's export, never writes workspace data."""
import json
import sys
from pathlib import Path
from pypdf import PdfReader
import pdfplumber

root = Path(sys.argv[1]).resolve(strict=True)
pdfs = list((root / "Export").glob("*导出附加验收.pdf"))
assert len(pdfs) == 1
pdf = pdfs[0]
reader = PdfReader(pdf, strict=True)
with pdfplumber.open(pdf) as document:
    alternate = "\n".join(page.extract_text() or "" for page in document.pages)
for text in ["\n".join(page.extract_text() for page in reader.pages), alternate]:
    compact = "".join(text.split())
    for token in ["本轮问题验证", "如何导出", "本轮答案验证", "完整保留", "评论对象验证", "评论正文验证", "评论回复验证", "附件清单验证.pdf", "第1层"]:
        assert token in compact, ("missing", token, compact)
    for token in ["不应导出隐藏消息", "not-read-private-file", "mochi://"]:
        assert token not in compact, ("leaked", token)
markdown = pdf.with_suffix(".md").read_bytes()
original = ("# 导出附加验收\r\n\r\nmochi://ai-locate?session=export-verification&message=answer\r\n\r\n原文结束。\r\n").encode()
assert markdown == original.replace(b"mochi://ai-locate?session=export-verification&message=answer", b"")
source = list((root / "知识库").rglob("导出附加验收.md"))
assert len(source) == 1
assert source[0].read_bytes() == original
print(json.dumps({"pdf": str(pdf), "pages": len(reader.pages), "readers": ["pypdf", "pdfplumber"],
                  "question_answer_comments_attachment_names": True, "no_hidden_messages_or_attachment_paths": True,
                  "markdown_default_and_original_bytes_unchanged": True}, ensure_ascii=False, indent=2))
