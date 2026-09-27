//! 默认导出路径与可移植文件编码。PDF 页由图形宿主提供，核心不依赖 UI。
pub mod enrichment;
use anyhow::{bail, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
pub fn output_path(root: &Path, source: &Path, format: &str) -> Result<PathBuf> {
    let extension = match format {
        "pdf" => "pdf",
        "html" => "html",
        "markdown" => "md",
        _ => bail!("不支持的导出格式"),
    };
    let root_text = root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_owned();
    let source_text = source.to_string_lossy().replace('\\', "/");
    let relative = if source_text
        .to_lowercase()
        .starts_with(&(root_text.to_lowercase() + "/"))
    {
        source_text[root_text.len() + 1..].to_owned()
    } else {
        source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let mut parts = relative
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if matches!(parts.first(), Some(&"知识库" | &"Docs")) {
        parts.remove(0);
    }
    let name = parts.join("-");
    let name = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&name);
    let mut sanitized = String::new();
    for c in name.chars() {
        let c = if c.is_control() || "<>:\"/\\|?*".contains(c) {
            '-'
        } else {
            c
        };
        if c != '-' || !sanitized.ends_with('-') {
            sanitized.push(c);
        }
    }
    let name = sanitized.trim_matches('-').trim();
    let name = if name.is_empty() { "document" } else { name };
    Ok(root.join("Export").join(format!("{name}.{extension}")))
}
pub fn html(markdown: &str, title: &str) -> String {
    let mut body = String::new();
    let options = pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_TASKLISTS
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH;
    pulldown_cmark::html::push_html(
        &mut body,
        pulldown_cmark::Parser::new_ext(markdown, options),
    );
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<!doctype html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data: file: https: http:\"><title>{title}</title><style>body{{max-width:860px;margin:40px auto;padding:0 32px;font:16px/1.75 'Microsoft YaHei',sans-serif;color:#24292e}}pre{{padding:20px 24px;background:#f8f9fa;border:1px solid #e7e9ee;border-radius:10px;white-space:pre-wrap}}code{{font-family:Consolas,monospace}}table{{border-collapse:collapse}}td,th{{border:1px solid #ddd;padding:10px 14px}}img{{max-width:100%}}blockquote{{border-left:3px solid #ddd;padding-left:16px}}</style></head><body>{body}</body></html>")
}
pub struct PdfPage {
    width: u32,
    height: u32,
    compressed: Vec<u8>,
    text: Vec<PdfText>,
}
/// 以 A4 PDF 点为单位的成组文字簇，从左上角起量。图像仍是视觉上的权威；
/// 这一层只负责提供精确的 Unicode 与选区几何，不重新分发系统字体程序。
#[derive(Debug, Clone, PartialEq)]
pub struct PdfText {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}
impl PdfPage {
    pub fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Result<Self> {
        if width == 0
            || height == 0
            || (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(4))
                != Some(bgra.len())
        {
            bail!("位图大小不匹配")
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        let rgb = bgra
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect::<Vec<_>>();
        encoder.write_all(&rgb)?;
        Ok(Self {
            width,
            height,
            compressed: encoder.finish()?,
            text: Vec::new(),
        })
    }
    pub fn with_text(mut self, text: Vec<PdfText>) -> Result<Self> {
        for cluster in &text {
            if cluster.text.is_empty()
                || cluster.text.encode_utf16().count() > 256
                || ![cluster.left, cluster.top, cluster.width, cluster.height]
                    .iter()
                    .all(|v| v.is_finite())
                || cluster.left < 0.0
                || cluster.top < 0.0
                || cluster.width <= 0.0
                || cluster.height <= 0.0
                || cluster.left + cluster.width > 595.01
                || cluster.top + cluster.height > 842.01
            {
                bail!("PDF 文字层内容或位置无效")
            }
        }
        self.text = text;
        Ok(self)
    }
}
pub fn pdf(pages: &[PdfPage]) -> Result<Vec<u8>> {
    if pages.is_empty() {
        bail!("没有可导出的页面")
    }
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Count {} /Kids [{}] >>",
            pages.len(),
            (0..pages.len())
                .map(|i| format!("{} 0 R", 3 + i * 3))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into_bytes(),
    ];
    // 简单 Type 3 字体用单字节编码。每个子集装 255 个成组文字簇，
    // 不是 Unicode 标量，因此代理对/连字保持完整。
    let mut glyphs = std::collections::BTreeMap::new();
    for cluster in pages.iter().flat_map(|p| &p.text) {
        let next = glyphs.len();
        glyphs.entry(cluster.text.clone()).or_insert(next);
    }
    let font_base = 3 + pages.len() * 3;
    let font_count = glyphs.len().div_ceil(255);
    let empty_glyph = font_base + font_count * 2;
    let resources = (0..font_count)
        .map(|i| format!("/T{i} {} 0 R", font_base + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    for (i, page) in pages.iter().enumerate() {
        let base = 3 + i * 3;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /XObject << /Im0 {} 0 R >> /Font << {resources} >> >> /Contents {} 0 R >>",base+2,base+1).into_bytes());
        let mut content = b"q\n595 0 0 842 0 0 cm\n/Im0 Do\nQ\n".to_vec();
        for cluster in &page.text {
            let id = glyphs[&cluster.text];
            writeln!(
                content,
                "BT /T{} 1 Tf 3 Tr {:.5} 0 0 {:.5} {:.5} {:.5} Tm <{:02X}> Tj ET",
                id / 255,
                cluster.width,
                cluster.height,
                cluster.left,
                842.0 - cluster.top - cluster.height,
                id % 255 + 1
            )?;
        }
        objects.push(pdf_stream(&content));
        let mut image=format!("<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",page.width,page.height,page.compressed.len()).into_bytes();
        image.extend_from_slice(&page.compressed);
        image.extend_from_slice(b"\nendstream");
        objects.push(image);
    }
    let mut ordered = vec![""; glyphs.len()];
    for (text, id) in &glyphs {
        ordered[*id] = text;
    }
    for (font, subset) in ordered.chunks(255).enumerate() {
        let names = (1..=subset.len())
            .map(|i| format!("/g{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let procs = (1..=subset.len())
            .map(|i| format!("/g{i} {empty_glyph} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        let widths = vec!["1000"; subset.len()].join(" ");
        objects.push(format!("<< /Type /Font /Subtype /Type3 /Name /T{font} /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {procs} >> /Encoding << /Type /Encoding /Differences [1 {names}] >> /FirstChar 1 /LastChar {} /Widths [{widths}] /Resources << >> /ToUnicode {} 0 R >>",subset.len(),font_base+font*2+1).into_bytes());
        let mut cmap=b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Mochi) /Ordering (Unicode) /Supplement 0 >> def\n/CMapName /MochiUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n".to_vec();
        // PDF 的 CMap 算子每个块最多接受 100 个映射。
        for (chunk, entries) in subset.chunks(100).enumerate() {
            writeln!(cmap, "{} beginbfchar", entries.len())?;
            for (j, text) in entries.iter().enumerate() {
                let unicode = text
                    .encode_utf16()
                    .map(|c| format!("{c:04X}"))
                    .collect::<String>();
                writeln!(cmap, "<{:02X}> <{unicode}>", chunk * 100 + j + 1)?;
            }
            cmap.extend_from_slice(b"endbfchar\n");
        }
        cmap.extend_from_slice(
            b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        );
        objects.push(pdf_stream(&cmap));
    }
    if font_count > 0 {
        // 一个带度量、不实际绘字的真 Type 3 内嵌字形程序。
        // 空路径也让忽略 Type 3 Tr 的 PDF 1.4 阅读器保持不变。
        objects.push(pdf_stream(b"1000 0 0 0 1000 1000 d1\n0 0 m n\n"));
    }
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = vec![0usize];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj", i + 1)?;
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len())?;
    for offset in offsets.iter().skip(1) {
        writeln!(out, "{offset:010} 00000 n ")?;
    }
    write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len()
    )?;
    Ok(out)
}
fn pdf_stream(content: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    stream
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_path_matches_electron_layout() {
        assert_eq!(
            output_path(
                Path::new("D:/ws"),
                Path::new("D:/ws/知识库/课程/笔记.md"),
                "pdf"
            )
            .unwrap(),
            PathBuf::from("D:/ws/Export/课程-笔记.pdf")
        );
    }
    #[test]
    fn pdf_xref_points_to_real_objects() {
        let page = PdfPage::from_bgra(1, 1, &[255, 255, 255, 255]).unwrap();
        let bytes = pdf(&[page]).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Count 1"));
        let offset = text
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(&bytes[offset..offset + 4], b"xref");
    }
    #[test]
    fn html_preserves_tables_and_blocks_scripts_by_policy() {
        let page = html("| a | b |\n| - | - |\n| c | d |", "标题");
        assert!(page.contains("<table>"));
        assert!(page.contains("default-src 'none'"));
    }
    #[test]
    fn pdf_text_cmaps_preserve_clusters_and_split_font_subsets() {
        let mut text = (0..300)
            .map(|i| PdfText {
                text: char::from_u32(0x4e00 + i).unwrap().to_string(),
                left: 20.0,
                top: 20.0,
                width: 10.0,
                height: 12.0,
            })
            .collect::<Vec<_>>();
        text.push(PdfText {
            text: "😀e\u{301}".into(),
            left: 20.0,
            top: 40.0,
            width: 20.0,
            height: 12.0,
        });
        let page = PdfPage::from_bgra(1, 1, &[255; 4])
            .unwrap()
            .with_text(text)
            .unwrap();
        let bytes = pdf(&[page]).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("/Subtype /Type3").count(), 2);
        assert_eq!(text.matches("100 beginbfchar").count(), 2);
        assert!(text.contains("<D83DDE0000650301>"));
        assert!(text.contains("/LastChar 255") && text.contains("/LastChar 46"));
        assert!(
            !text.contains("/FontFile"),
            "No system font programs are redistributed"
        );
    }
    #[test]
    fn invalid_pdf_bitmap_and_text_geometry_are_rejected() {
        assert!(PdfPage::from_bgra(0, 0, &[]).is_err());
        assert!(PdfPage::from_bgra(u32::MAX, u32::MAX, &[]).is_err());
        for left in [f32::NAN, f32::INFINITY, -1.0, 595.0] {
            assert!(PdfPage::from_bgra(1, 1, &[255; 4])
                .unwrap()
                .with_text(vec![PdfText {
                    text: "中".into(),
                    left,
                    top: 20.0,
                    width: 10.0,
                    height: 12.0
                }])
                .is_err());
        }
    }
}
