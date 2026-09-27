//! 与 TipTap 输出兼容的 HTML 行内样式；只解释样式，不执行 HTML。
use super::text::Emphasis;
pub fn aligned(line: &str) -> Option<(super::draw::Align, u8, String)> {
    let line = line.trim();
    if !line.starts_with('<') {
        return None;
    }
    // 每条非空 Markdown 行都会调用此逻辑。如果每行都重新编译表达式，
    // 即使很短的纯文本笔记，在 Debug 模式下解析也要花上几秒。
    static ALIGNED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = ALIGNED.get_or_init(|| regex::Regex::new(r#"(?is)^<(p|h[1-6])\s+style=["'][^"']*text-align\s*:\s*(left|center|right)[^"']*["'][^>]*>(.*)</(?:p|h[1-6])>$"#).expect("static alignment regex"));
    let c = pattern.captures(line)?;
    let level = c[1]
        .strip_prefix('h')
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let align = match &c[2] {
        "center" => super::draw::Align::Center,
        "right" => super::draw::Align::Trailing,
        _ => super::draw::Align::Leading,
    };
    Some((align, level, c[3].into()))
}
pub fn align_line(line: &str, alignment: super::draw::Align) -> String {
    let cr = if line.ends_with('\r') { "\r" } else { "" };
    let raw = line.trim_end_matches('\r');
    if raw.trim().is_empty() {
        return line.into();
    }
    let (level, body) = if let Some((_, level, body)) = aligned(raw) {
        (level, body)
    } else {
        let level = raw.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && raw.as_bytes().get(level) == Some(&b' ') {
            (level as u8, raw[level + 1..].into())
        } else {
            (0, raw.into())
        }
    };
    let tag = if level > 0 {
        format!("h{level}")
    } else {
        "p".into()
    };
    let alignment = match alignment {
        super::draw::Align::Center => "center",
        super::draw::Align::Trailing => "right",
        _ => "left",
    };
    format!("<{tag} style=\"text-align: {alignment}\">{body}</{tag}>{cr}")
}
pub fn color(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            3 => {
                u32::from_str_radix(&hex.chars().flat_map(|c| [c, c]).collect::<String>(), 16).ok()
            }
            6 => u32::from_str_radix(hex, 16).ok(),
            8 => u32::from_str_radix(&hex[..6], 16).ok(),
            _ => None,
        };
    }
    if value.starts_with("rgb(") || value.starts_with("rgba(") {
        let values = value
            .split_once('(')?
            .1
            .trim_end_matches(')')
            .split(',')
            .take(3)
            .map(|s| s.trim().parse::<u32>().ok().filter(|n| *n <= 255))
            .collect::<Option<Vec<_>>>()?;
        if values.len() == 3 {
            return Some(values[0] << 16 | values[1] << 8 | values[2]);
        }
    }
    None
}
pub struct Span {
    pub body_start: usize,
    pub body_end: usize,
    pub end: usize,
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub flags: u8,
}
pub fn html_span(source: &str) -> Option<Span> {
    let header_end = source.find('>')?;
    let header = &source[..header_end + 1];
    let name = header
        .strip_prefix('<')?
        .split(|c: char| c.is_ascii_whitespace() || c == '>')
        .next()?
        .to_ascii_lowercase();
    let mut flags = match name.as_str() {
        "strong" | "b" => 1,
        "em" | "i" => 2,
        "u" => 4,
        "s" | "del" => 8,
        "code" => 16,
        "span" | "mark" => 0,
        _ => return None,
    };
    let lower = source.to_ascii_lowercase();
    let close = format!("</{name}>");
    let open = format!("<{name}");
    let mut cursor = header_end + 1;
    let mut depth = 1;
    let end = loop {
        let next_close = cursor + lower[cursor..].find(&close)?;
        let next_open = lower[cursor..next_close]
            .match_indices(&open)
            .find(|(i, _)| {
                lower
                    .as_bytes()
                    .get(cursor + i + open.len())
                    .is_some_and(|c| c.is_ascii_whitespace() || *c == b'>')
            })
            .map(|(i, _)| cursor + i);
        if let Some(i) = next_open {
            depth += 1;
            if depth > 16 {
                return None;
            }
            cursor = i + open.len();
        } else {
            depth -= 1;
            if depth == 0 {
                break next_close;
            }
            cursor = next_close + close.len();
        }
    };
    let property = |name: &str| -> Option<u32> {
        let lower = header.to_ascii_lowercase();
        let token = format!("{name}:");
        static COLON_SPACING: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let spaced = COLON_SPACING
            .get_or_init(|| regex::Regex::new(r"\s*:\s*").expect("static CSS spacing regex"))
            .replace_all(&lower, ":")
            .into_owned();
        for (i, _) in spaced.match_indices(&token) {
            if i > 0 && !b"; \"'".contains(&spaced.as_bytes()[i - 1]) {
                continue;
            }
            let value = spaced[i + token.len()..].split([';', '\"', '\'']).next()?;
            if let Some(value) = color(value) {
                return Some(value);
            }
        }
        None
    };
    if header.contains("underline") {
        flags |= 4;
    }
    if header.contains("line-through") {
        flags |= 8;
    }
    Some(Span {
        body_start: header_end + 1,
        body_end: end,
        end: end + close.len(),
        fg: property("color"),
        bg: property("background-color").or((name == "mark").then_some(0xfff2cc)),
        flags,
    })
}
pub fn combine(inner: Emphasis, outer: &Span) -> Emphasis {
    let (fg, bg, flags) = match inner {
        Emphasis::Styled { fg, bg, flags } => (fg, bg, flags),
        _ => (
            None,
            None,
            match inner {
                Emphasis::Bold => 1,
                Emphasis::BoldItalic => 3,
                Emphasis::Italic => 2,
                Emphasis::Code => 16,
                Emphasis::Link => 32,
                Emphasis::Math => 64,
                _ => 0,
            },
        ),
    };
    Emphasis::Styled {
        fg: fg.or(outer.fg),
        bg: bg.or(outer.bg),
        flags: flags | outer.flags,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_span_matches_its_own_close() {
        let s = "<span style=\"color:#abc\">甲<span>乙</span>丙</span>后";
        let span = html_span(s).unwrap();
        assert_eq!(&s[span.body_start..span.body_end], "甲<span>乙</span>丙");
        assert_eq!(span.fg, Some(0xaabbcc));
    }
    #[test]
    fn color_does_not_confuse_background() {
        let s = html_span("<mark style=\"background-color: rgb(255, 0, 0)\">重点</mark>").unwrap();
        assert_eq!(s.bg, Some(0xff0000));
        assert_eq!(s.fg, None);
    }
    #[test]
    fn cached_style_parsing_preserves_alignment_and_spaced_colors() {
        let (align, level, body) =
            aligned("  <h2 style='color: red; text-align : center'><strong>标题</strong></h2>\r\n")
                .unwrap();
        assert!(matches!(align, super::super::draw::Align::Center));
        assert_eq!(level, 2);
        assert_eq!(body, "<strong>标题</strong>");
        assert!(aligned("正文 text-align: center").is_none());
        let span =
            html_span("<span style='color : #abc; background-color : #123'>中文</span>").unwrap();
        assert_eq!(span.fg, Some(0xaabbcc));
        assert_eq!(span.bg, Some(0x112233));
    }
}
