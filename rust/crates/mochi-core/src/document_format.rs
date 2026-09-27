//! 普通 HTML 不触发格式升级；检测 Mochi 扩展前先屏蔽代码块和行内代码。

use std::ops::Range;
use std::sync::LazyLock;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::Regex;

static SPAN_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<span\b([^>]*)>").expect("valid span tag regex"));
static MARK_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<mark\b([^>]*)>").expect("valid mark tag regex"));
static DIV_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<div\b([^>]*)>").expect("valid div tag regex"));
static PRE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<pre\b([^>]*)>").expect("valid pre tag regex"));
static IMG_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<img\b([^>]*)>").expect("valid img tag regex"));
static DETAILS_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<details\b[^>]*>").expect("valid details tag regex"));
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(?:^|[\s/])([a-z_:][a-z0-9:._-]*)(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?"#)
        .expect("valid HTML attribute regex")
});
static ATTRIBUTE_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(?:^|[\s/])([a-z_:][a-z0-9:._-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)
        .expect("valid HTML attribute value regex")
});
static STYLE_PROPERTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[;\s])(?:color|background(?:-color)?|font-size)\s*:")
        .expect("valid style property regex")
});
static WIDTH_STYLE_PROPERTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[;\s])width\s*:").expect("valid width style property regex")
});
static MOCHI_CODE_BLOCK_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<!--[\s]*mochi-code-block\b[^>]*-->")
        .expect("valid Mochi code block comment regex")
});
static HTML_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<!--.*?(?:-->|$)").expect("valid HTML comment regex"));
static SCRIPT_OR_STYLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?(?:</(?:script|style)\s*>|$)")
        .expect("valid script/style regex")
});
static HIGHLIGHT_CONTAINER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t]*:::\s*mochi-highlight(?:[ \t]|$)")
        .expect("valid highlight container regex")
});
static AI_LOCATOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t]*mochi://ai-locate\?[^\s<]*[ \t]*\r?$").expect("valid AI locator regex")
});
static OBJECT_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t]*mochi://(?:open|ai-session|block)\?[^\s<]*[ \t]*\r?$")
        .expect("valid Mochi object link regex")
});

/// 判断 `source` 里是否有需要按 `.mc` 保存的内容。
///
/// 只认那些必须依赖 Mochi 扩展文档行为的结构。Markdown 行内代码和
/// 代码块里长得像 HTML 的示例不算数。
pub fn requires_mochi_format(source: &str) -> bool {
    let masked_code = mask_code(source);

    // 原生富文本代码卡就是用这条注释表示的。要在屏蔽普通注释之前检查，
    // 但仍用屏蔽代码后的源码，行内代码/代码块里的示例才不会误触发升级。
    if MOCHI_CODE_BLOCK_COMMENT.is_match(&masked_code) {
        return true;
    }

    // 藏在注释或 script/style 体内的标签不算编辑器 DOM 里的 HTML 节点，
    // 不能因为它们就让纯 Markdown 文件升级成 `.mc`。
    let visible = mask_matches(&mask_matches(&masked_code, &SCRIPT_OR_STYLE), &HTML_COMMENT);

    if DETAILS_TAG.is_match(&visible)
        || HIGHLIGHT_CONTAINER.is_match(&visible)
        || visible
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix('[')?
                    .split_once("](")?
                    .1
                    .strip_suffix(')')
            })
            .any(|target| crate::object_reference::ObjectReference::parse(target).is_some())
        || OBJECT_LINK.find_iter(&visible).any(|line| {
            crate::object_reference::ObjectReference::parse(line.as_str().trim()).is_some()
        })
        || AI_LOCATOR
            .find_iter(&visible)
            .any(|line| crate::ai::locator::Locator::parse(line.as_str()).is_some())
    {
        return true;
    }

    if SPAN_TAG.captures_iter(&visible).any(|capture| {
        capture
            .get(1)
            .and_then(|attrs| attribute_value(attrs.as_str(), "style"))
            .is_some_and(|style| STYLE_PROPERTY.is_match(&style))
    }) {
        return true;
    }

    if MARK_TAG.captures_iter(&visible).any(|capture| {
        capture
            .get(1)
            .and_then(|attrs| attribute_value(attrs.as_str(), "style"))
            .is_some_and(|style| STYLE_PROPERTY.is_match(&style))
    }) {
        return true;
    }

    if DIV_TAG.captures_iter(&visible).any(|capture| {
        capture.get(1).is_some_and(|attrs| {
            attribute_equals(attrs.as_str(), "data-type", "mochi-highlight-block")
        }) || capture
            .get(1)
            .is_some_and(|attrs| attribute_equals(attrs.as_str(), "data-type", "ai-locator"))
    }) {
        return true;
    }

    if PRE_TAG.captures_iter(&visible).any(|capture| {
        capture.get(1).is_some_and(|attrs| {
            has_attribute(attrs.as_str(), "data-title")
                || has_attribute(attrs.as_str(), "data-collapsed")
        })
    }) {
        return true;
    }

    IMG_TAG.captures_iter(&visible).any(|capture| {
        capture.get(1).is_some_and(|attrs| {
            has_attribute(attrs.as_str(), "width")
                || attribute_value(attrs.as_str(), "style")
                    .is_some_and(|style| WIDTH_STYLE_PROPERTY.is_match(&style))
        })
    })
}

fn attribute_value(attrs: &str, name: &str) -> Option<String> {
    ATTRIBUTE_VALUE.captures_iter(attrs).find_map(|capture| {
        let attr_name = capture.get(1)?.as_str();
        if !attr_name.eq_ignore_ascii_case(name) {
            return None;
        }
        capture
            .get(2)
            .or_else(|| capture.get(3))
            .or_else(|| capture.get(4))
            .map(|value| value.as_str().to_owned())
    })
}

fn attribute_equals(attrs: &str, name: &str, expected: &str) -> bool {
    attribute_value(attrs, name).is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn has_attribute(attrs: &str, name: &str) -> bool {
    ATTRIBUTE.captures_iter(attrs).any(|capture| {
        capture
            .get(1)
            .is_some_and(|value| value.as_str().eq_ignore_ascii_case(name))
    })
}

fn mask_code(source: &str) -> String {
    let mut ranges = Vec::new();
    let mut code_block_start = None;

    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                code_block_start = Some(range.start);
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_block_start.take() {
                    ranges.push(start..range.end);
                }
            }
            Event::Code(_) => ranges.push(range),
            _ => {}
        }
    }

    if let Some(start) = code_block_start {
        ranges.push(start..source.len());
    }

    mask_ranges(source, ranges)
}

fn mask_matches(source: &str, pattern: &Regex) -> String {
    mask_ranges(
        source,
        pattern
            .find_iter(source)
            .map(|matched| matched.range())
            .collect(),
    )
}

fn mask_ranges(source: &str, mut ranges: Vec<Range<usize>>) -> String {
    if ranges.is_empty() {
        return source.to_owned();
    }

    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.start >= range.end {
            continue;
        }
        if let Some(previous) = merged.last_mut() {
            if range.start <= previous.end {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        } else {
            merged.push(range);
        }
    }

    let mut masked = source.to_owned().into_bytes();
    for range in merged {
        for byte in &mut masked[range] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(masked).expect("masking ASCII bytes preserves UTF-8")
}

#[cfg(test)]
mod tests {
    use super::requires_mochi_format;

    #[test]
    fn detects_editor_mochi_features() {
        let samples = [
            r#"<span style="color: red">text</span>"#,
            r#"<span style='background-color: yellow'>text</span>"#,
            r#"<span style="font-size: 18px">text</span>"#,
            r#"<mark style="background-color: #ffeb3b">text</mark>"#,
            "<details><summary>More</summary>text</details>",
            r#"<div data-type="mochi-highlight-block">text</div>"#,
            r#"<div data-type='ai-locator'>text</div>"#,
            r#"<pre data-title="shell">code</pre>"#,
            r#"<pre data-collapsed="true">code</pre>"#,
            r#"<img src="x.png" width="320">"#,
            r#"<img src="x.png" style="width: 320px">"#,
            "::: mochi-highlight color=yellow\ntext\n:::",
            "mochi://ai-locate?session=s&message=m",
            "mochi://open?path=notes%2Fa.mc&view=card",
            "mochi://ai-session?session=s&label=conversation",
            "mochi://block?path=note.md&id=block_00000000-0000-4000-8000-000000000001",
            "[参考文档](mochi://open?path=notes%2Fa.md)",
            "<!-- mochi-code-block title=\"shell\" collapsed=\"false\" -->\n```sh\necho hi\n```",
        ];

        for sample in samples {
            assert!(
                requires_mochi_format(sample),
                "expected Mochi feature: {sample}"
            );
        }
    }

    #[test]
    fn ignores_features_inside_code() {
        let source = r#"
```html
<span style="color: red">text</span>
<details>text</details>
<div data-type="mochi-highlight-block">text</div>
<pre data-title="shell">code</pre>
<img width="320">
<!-- mochi-code-block title="shell" collapsed="false" -->
:::
mochi://ai-locate?session=s&message=m
```

`<details>example</details>` and `<img width="320">`
"#;

        assert!(!requires_mochi_format(source));
    }

    #[test]
    fn ignores_html_examples_in_comments_and_script() {
        let source = r#"
<!-- <details>example</details><span style="color: red">example</span> -->
<script>const html = '<img width="320">';</script>
<style>.x { color: red; }</style>
"#;

        assert!(!requires_mochi_format(source));
    }

    #[test]
    fn leaves_plain_markdown_plain() {
        assert!(!requires_mochi_format(
            "# Heading\n\nA [link](https://example.com) and **bold** text."
        ));
    }
}
