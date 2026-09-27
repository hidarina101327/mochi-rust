//! 把 Obsidian 的图片嵌入导入为 Mochi 的图片块语法。
//! 靠解析器 span 保证代码示例、注释和非图片 wikilink 原样保留。
use super::*;

struct Embed {
    range: Range<usize>,
    image: String,
    close_marks: String,
    open_marks: String,
}

pub(super) fn normalize_images(source: &str) -> String {
    if !source.contains("![[") {
        return source.to_owned();
    }
    let mut images = Vec::new();
    let mut marks = Vec::new();
    let mut protected = Vec::new();
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image {
                link_type: LinkType::WikiLink { .. },
                dest_url,
                ..
            }) => {
                let url = unescape_path(dest_url.trim()).replace('\\', "/");
                if is_image(&url) {
                    let alias = source[range.clone()]
                        .strip_suffix("]]")
                        .and_then(|raw| raw.split_once('|'))
                        .map(|(_, alias)| alias.trim())
                        .unwrap_or("");
                    images.push((range, image_markup(&url, alias)));
                }
            }
            Event::Start(Tag::Strong) => {
                marks.push((range.clone(), &source[range.start..range.start + 2]))
            }
            Event::Start(Tag::Emphasis) => {
                marks.push((range.clone(), &source[range.start..range.start + 1]))
            }
            Event::Start(Tag::Strikethrough) => {
                let count = if source[range.clone()].starts_with("~~") {
                    2
                } else {
                    1
                };
                marks.push((range.clone(), &source[range.start..range.start + count]))
            }
            // 把链接标签或表格单元格拆成段落会破坏周边结构；保持原格式。
            Event::Start(Tag::Link { .. } | Tag::Table(_)) => protected.push(range),
            _ => {}
        }
    }
    let mut edits: Vec<Embed> = Vec::new();
    for (original, image) in images {
        if protected
            .iter()
            .any(|r| r.start <= original.start && r.end >= original.end)
        {
            continue;
        }
        let mut range = original.clone();
        let mut close_marks = String::new();
        let mut open_marks = String::new();
        let mut parents: Vec<_> = marks
            .iter()
            .filter(|(r, _)| r.start < original.start && r.end > original.end)
            .collect();
        parents.sort_by_key(|(r, _)| r.len());
        for (parent, marker) in parents {
            // 只有图片时要剥掉周围的强调标记。旁边还有文字的话，
            // 在新插入的图片块两侧闭合/重新打开同一组标记。
            if source[parent.start + marker.len()..range.start]
                .trim()
                .is_empty()
            {
                range.start = parent.start;
            } else {
                close_marks.push_str(marker);
            }
            if source[range.end..parent.end - marker.len()]
                .trim()
                .is_empty()
            {
                range.end = parent.end;
            } else {
                open_marks.insert_str(0, marker);
            }
        }
        // 强调标记要紧贴文字，不能带出尾随/前导空格。
        while range.start > 0 && matches!(source.as_bytes()[range.start - 1], b' ' | b'\t') {
            range.start -= 1;
        }
        while range.end < source.len() && matches!(source.as_bytes()[range.end], b' ' | b'\t') {
            range.end += 1;
        }
        if let Some(previous) = edits.last_mut() {
            if range.start <= previous.range.end {
                // 相邻的两个嵌入之间不需要空的强调段落。
                previous.open_marks.clear();
                close_marks.clear();
                range.start = previous.range.end;
            }
        }
        edits.push(Embed {
            range,
            image,
            close_marks,
            open_marks,
        });
    }
    let newline = if source.contains("\r\n") {
        "\r\n\r\n"
    } else {
        "\n\n"
    };
    let mut result = source.to_owned();
    for edit in edits.into_iter().rev() {
        let before = source[..edit.range.start]
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .trim();
        let after = source[edit.range.end..]
            .split('\n')
            .next()
            .unwrap_or("")
            .trim();
        let replacement = format!(
            "{}{}{}{}{}",
            edit.close_marks,
            if before.is_empty() { "" } else { newline },
            edit.image,
            if after.is_empty() { "" } else { newline },
            edit.open_marks
        );
        result.replace_range(edit.range, &replacement);
    }
    result
}

fn is_image(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let decoded = decode_url(path).unwrap_or_else(|| path.to_owned());
    Path::new(&decoded).extension().is_some_and(|extension| {
        matches!(
            extension.to_string_lossy().to_ascii_lowercase().as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "svg"
                | "ico"
                | "avif"
                | "tif"
                | "tiff"
                | "heic"
                | "heif"
        )
    })
}

fn unescape_path(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            result.push(chars.next().unwrap());
        } else {
            result.push(ch);
        }
    }
    result
}

fn image_markup(url: &str, alias: &str) -> String {
    // 转义目标路径，但不改动已百分号编码的文件名、URL、查询串和锚点；
    // 资源导入器接下来会解析它们。
    let url = url
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('"', "%22");
    let dimension = |value: &str| value.parse::<u32>().ok().filter(|value| *value > 0);
    let size = alias
        .split_once('x')
        .and_then(|(w, h)| Some((dimension(w)?, Some(dimension(h)?))))
        .or_else(|| dimension(alias).map(|width| (width, None)));
    if let Some((width, height)) = size {
        let height = height
            .map(|height| format!(" height=\"{height}\""))
            .unwrap_or_default();
        format!(
            "<img src=\"{}\" alt=\"\" width=\"{width}\"{height} />",
            url.replace('&', "&amp;")
        )
    } else {
        let alt = alias
            .replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]");
        format!("![{alt}](<{url}>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_inline_and_emphasized_images_become_blocks_without_dangling_marks() {
        for source in [
            "![[p.png]]",
            "**![[p.png]]**",
            "__![[p.png]]__",
            "***![[p.png]]***",
            "~~![[p.png]]~~",
        ] {
            assert_eq!(normalize_images(source), "![](<p.png>)", "{source}");
        }
        assert_eq!(
            normalize_images("前文![[p.png]]后文"),
            "前文\n\n![](<p.png>)\n\n后文"
        );
        assert_eq!(
            normalize_images("**前文 ![[p.png]] 后文**"),
            "**前文**\n\n![](<p.png>)\n\n**后文**"
        );
        assert_eq!(
            normalize_images("**![[p.png]] 后文**"),
            "![](<p.png>)\n\n**后文**"
        );
        assert_eq!(
            normalize_images("**前文 ![[p.png]]**"),
            "**前文**\n\n![](<p.png>)"
        );
        let adjacent = normalize_images("**![[a.png]] ![[b.png]]**");
        assert!(!adjacent.contains("**"), "{adjacent}");
        assert_eq!(
            adjacent
                .lines()
                .filter(|line| line.starts_with("![]("))
                .count(),
            2
        );
    }

    #[test]
    fn obsidian_conversion_preserves_examples_non_image_embeds_and_crlf() {
        let examples = "`![[inline.png]]`\r\n\r\n```md\r\n![[fenced.png]]\r\n```\r\n\r\n    ![[indented.png]]\r\n\r\n<!-- ![[comment.png]] -->\r\n![[笔记.md]]\r\n[[图片.png]]\r\n[![[linked.png]]](https://example.com)\r\n";
        assert_eq!(normalize_images(examples), examples);
        assert_eq!(
            normalize_images("前文![[p.png]]\r\n后文\r\n"),
            "前文\r\n\r\n![](<p.png>)\r\n后文\r\n"
        );
    }

    #[test]
    fn obsidian_paths_aliases_and_sizes_use_supported_image_syntax() {
        assert_eq!(
            normalize_images(r"![[assets\图 \(1\)\.png|截图说明]]"),
            "![截图说明](<assets/图 (1).png>)"
        );
        assert_eq!(
            normalize_images("![[assets/p.png|200]]"),
            "<img src=\"assets/p.png\" alt=\"\" width=\"200\" />"
        );
        assert_eq!(
            normalize_images("![[assets/p.png|200x100]]"),
            "<img src=\"assets/p.png\" alt=\"\" width=\"200\" height=\"100\" />"
        );
        assert_eq!(
            normalize_images("![[https://example.com/p.PNG?q=1|示例]]"),
            "![示例](<https://example.com/p.PNG?q=1>)"
        );
    }
}
