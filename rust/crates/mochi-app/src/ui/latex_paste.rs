//! 谨慎识别剪贴板内容是否为单独一条公式。
use regex::Regex;
use std::sync::LazyLock;

pub fn formula_block(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 16 * 1024 {
        return None;
    }
    let delimited = text
        .strip_prefix("$$")
        .and_then(|s| s.strip_suffix("$$"))
        .or_else(|| text.strip_prefix(r"\[").and_then(|s| s.strip_suffix(r"\]")));
    let source = delimited.unwrap_or(text).trim();
    if source.is_empty() || source.contains(['$', '`']) {
        return None;
    }
    static COMMAND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\[A-Za-z]+").unwrap());
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z]{3,}").unwrap());
    let (normalized, math_only) = normalize_and_mask_text(source, delimited.is_none());
    if delimited.is_none() {
        // 要求内容中确实包含 TeX 命令；普通文字、路径和常规算术表达式
        // 仍按文本处理。文本标签和环境名称不参与此项检查。
        if !COMMAND.is_match(source) {
            return None;
        }
        let remainder = COMMAND.replace_all(&math_only, "");
        if WORD.is_match(&remainder)
            || remainder
                .chars()
                .any(|ch| !ch.is_ascii() && ch.is_alphabetic())
        {
            return None;
        }
    }
    super::math_layout::formula(&normalized, 0)?;
    Some(format!("$$\n{normalized}\n$$"))
}

fn normalize_and_mask_text(source: &str, repair: bool) -> (String, String) {
    static TEXT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\\(?:text[a-z]*|operatorname|mathrm|mathsf|mathtt|begin|end)\*?\s*\{")
            .unwrap()
    });
    let mut normalized = String::new();
    let mut math_only = String::new();
    let mut offset = 0;
    while offset < source.len() {
        let rest = &source[offset..];
        if let Some(found) = TEXT.find(rest) {
            let mut end = found.end();
            let mut depth = 1;
            let mut escaped = false;
            for ch in rest[end..].chars() {
                end += ch.len_utf8();
                if escaped {
                    escaped = false;
                    continue;
                }
                match ch {
                    '\\' => escaped = true,
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
            }
            normalized.push_str(&rest[..end]);
            math_only.push(' ');
            offset += end;
        } else if repair && (rest.starts_with(r"\_") || rest.starts_with(r"\^")) {
            // 聊天内容或 Markdown 有时会转义下标和上标标记。
            normalized.push(rest.as_bytes()[1] as char);
            math_only.push(rest.as_bytes()[1] as char);
            offset += 2;
        } else if rest.starts_with(r"\\") {
            // 保留矩阵和 aligned 环境中的行分隔符。
            normalized.push_str(r"\\");
            math_only.push_str(r"\\");
            offset += 2;
        } else {
            let ch = rest.chars().next().unwrap();
            normalized.push(ch);
            math_only.push(ch);
            offset += ch.len_utf8();
        }
    }
    (normalized, math_only)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_weighted_sum_and_repairs_markdown_subscripts() {
        let tex = r"S = \sum_{i=1}^{n} w_i s_i, \qquad \sum_{i=1}^{n} w_i = 1";
        assert_eq!(formula_block(tex), Some(format!("$$\n{tex}\n$$")));
        assert_eq!(formula_block(&tex.replace('_', r"\_")), formula_block(tex));
    }
    #[test]
    fn preserves_text_underscores_and_matrix_rows() {
        for tex in [
            r"\text{file\_name} = \frac{a}{b}",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            "\\begin{aligned}\na &= b \\\\\nc &= d\n\\end{aligned}",
        ] {
            assert_eq!(formula_block(tex), Some(format!("$$\n{tex}\n$$")));
        }
        assert_eq!(
            formula_block(r"\[ x^2 + y^2 = 1 \]"),
            Some("$$\nx^2 + y^2 = 1\n$$".into())
        );
        assert_eq!(formula_block("$$\nx^2\n$$"), Some("$$\nx^2\n$$".into()));
    }
    #[test]
    fn leaves_prose_code_paths_inline_math_and_invalid_tex_alone() {
        for text in [
            "普通文字",
            r"Use \sum to add values",
            r"求和公式 \sum_i x_i",
            r"C:\Users\notes",
            r"$\alpha$",
            "```latex\n\\sum_i x_i\n```",
            r"\frac{a}{",
            r"\notARealCommand",
            "x = 1",
            "$$x$$ and $$y$$",
        ] {
            assert_eq!(formula_block(text), None, "{text}");
        }
    }
}
