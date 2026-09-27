//! 用户显式发起的导入。原始文件和目标位置的既有文件绝不覆盖。
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};

mod obsidian;

#[derive(Debug, Default)]
pub struct Report {
    pub imported: Vec<PathBuf>,
    pub resources: usize,
    pub warnings: Vec<String>,
}

/// 每个顶层条目独立导入，避免一个读不了的条目掩盖其余成功的导入。
/// Markdown 按「原始目录」为基准改写。
pub fn copy_paths(sources: &[PathBuf], target: &Path) -> Report {
    let mut report = Report::default();
    let mut seen = HashSet::new();
    let mut assets = HashMap::new();
    for source in sources {
        let result = (|| -> Result<Option<PathBuf>> {
            let source = source
                .canonicalize()
                .with_context(|| format!("无法读取 {}", source.display()))?;
            if !seen.insert(source.clone()) {
                return Ok(None);
            }
            let target = target.canonicalize().context("目标文件夹已不可访问")?;
            if !target.is_dir() {
                bail!("目标必须是文件夹");
            }
            if source.is_dir() && crate::paths::path_is_within(&source, &target) {
                bail!("不能将文件夹复制到自身或其子文件夹");
            }
            let destination = available_path(&target, &source)?;
            copy_entry(&source, &destination, &mut report, &mut assets)?;
            Ok(Some(destination))
        })();
        match result {
            Ok(Some(path)) => report.imported.push(path),
            Ok(None) => {}
            Err(error) => report
                .warnings
                .push(format!("{}：{error:#}", source.display())),
        }
    }
    report
}

pub fn available_path(target: &Path, source: &Path) -> Result<PathBuf> {
    let name = source.file_name().context("来源没有文件名")?;
    let first = target.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return Ok(first);
    }
    let (stem, extension) = if source.is_dir() {
        (name.to_string_lossy().into_owned(), String::new())
    } else {
        (
            source
                .file_stem()
                .unwrap_or(name)
                .to_string_lossy()
                .into_owned(),
            source
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default(),
        )
    };
    for i in 1..10000 {
        let candidate = target.join(format!("{stem} ({i}){extension}"));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    bail!("同名文件过多，请更换目标目录")
}

type Assets = HashMap<(PathBuf, PathBuf), PathBuf>;

fn copy_entry(
    source: &Path,
    destination: &Path,
    report: &mut Report,
    assets: &mut Assets,
) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    let linked = metadata.file_type().is_symlink();
    if linked {
        bail!("跳过符号链接或目录联接，请直接拖入实际来源");
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        // 先复制子资源再改写笔记，这样源目录自带的 `assets`
        // 就不会和导入笔记生成的资源目录撞名。
        let entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        let (notes, rest): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .partition(|e| is_markdown(&e.path()) && e.path().is_file());
        for entry in rest.into_iter().chain(notes) {
            if let Err(error) = copy_entry(
                &entry.path(),
                &destination.join(entry.file_name()),
                report,
                assets,
            ) {
                report
                    .warnings
                    .push(format!("{}：{error:#}", entry.path().display()));
            }
        }
    } else if metadata.is_file() {
        if is_markdown(source) {
            let content = fs::read_to_string(source).context("Markdown 不是有效的 UTF-8 文档")?;
            let content = obsidian::normalize_images(&content);
            let content = import_references(&content, source, destination, report, assets);
            write_new(destination, content.as_bytes())?;
        } else {
            copy_new(source, destination)?;
        }
    } else {
        bail!("只能导入普通文件或文件夹");
    }
    Ok(())
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

fn write_new(path: &Path, content: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if let Err(error) = file.write_all(content) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

fn copy_new(source: &Path, destination: &Path) -> Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    if let Err(error) = std::io::copy(&mut input, &mut output) {
        drop(output);
        let _ = fs::remove_file(destination);
        return Err(error.into());
    }
    Ok(())
}

fn import_references(
    content: &str,
    source: &Path,
    destination: &Path,
    report: &mut Report,
    assets: &mut Assets,
) -> String {
    let mut edits = Vec::new();
    for (range, url) in references(content) {
        let Some((path, suffix)) = local_reference(&url, source.parent().unwrap()) else {
            continue;
        };
        let result = (|| -> Result<String> {
            let path = path
                .canonicalize()
                .with_context(|| format!("引用资源不存在：{url}"))?;
            if !path.is_file() {
                return Ok(url.clone());
            }
            let directory = destination
                .parent()
                .context("目标文档目录不可用")?
                .join("assets");
            let key = (path.clone(), directory.clone());
            let saved = if let Some(saved) = assets.get(&key) {
                saved.clone()
            } else {
                fs::create_dir_all(&directory)?;
                anyhow::ensure!(
                    crate::paths::path_is_within(
                        &destination.parent().unwrap().canonicalize()?,
                        &directory.canonicalize()?
                    ),
                    "资源目录指向文档目录之外，未写入"
                );
                let extension = path
                    .extension()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = format!(
                    "import-{}{}",
                    crate::paths::random_base36(16),
                    if extension.is_empty() {
                        String::new()
                    } else {
                        format!(".{extension}")
                    }
                );
                let saved = directory.join(name);
                copy_new(&path, &saved)?;
                assets.insert(key, saved.clone());
                report.resources += 1;
                saved
            };
            Ok(format!(
                "./assets/{}{suffix}",
                encode_filename(&saved.file_name().unwrap().to_string_lossy())
            ))
        })();
        match result {
            Ok(replacement) if replacement != url => edits.push((range, replacement)),
            Ok(_) => {}
            Err(error) => report
                .warnings
                .push(format!("{}：{error:#}", source.display())),
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    edits.dedup_by(|a, b| a.0 == b.0);
    let mut result = content.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    result
}

fn encode_filename(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn decode_url(value: &str) -> Option<String> {
    let mut decoded = Vec::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                decoded.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).ok()
}

fn local_reference(value: &str, base: &Path) -> Option<(PathBuf, String)> {
    if value.is_empty() || value.starts_with('#') || value.starts_with("//") {
        return None;
    }
    let split = value.find(['?', '#']).unwrap_or(value.len());
    let (value, suffix) = value.split_at(split);
    if value.to_ascii_lowercase().starts_with("file:") {
        return url::Url::parse(value)
            .ok()?
            .to_file_path()
            .ok()
            .map(|path| (path, suffix.into()));
    }
    // Windows 盘符前缀算本地路径；其他 scheme（http、data、mailto、
    // mochi…）原样保留，不发任何网络请求。
    let drive = value.as_bytes().get(1) == Some(&b':') && value.as_bytes()[0].is_ascii_alphabetic();
    if value.contains(':') && !drive {
        return None;
    }
    let path = PathBuf::from(decode_url(value)?.replace('\\', "/"));
    Some((
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        },
        suffix.into(),
    ))
}

/// 用解析器给出的 span，行内/围栏代码里的示例永远不会被改写。
/// 原始 Markdown 的其余部分逐字节保留，CRLF 也不例外。
fn references(content: &str) -> Vec<(Range<usize>, String)> {
    let parser = Parser::new_ext(content, Options::all());
    let mut refs = Vec::new();
    for (_, definition) in parser.reference_definitions().iter() {
        let raw = &content[definition.span.clone()];
        if let Some(start) = raw.find("]:") {
            if let Some(range) = destination_range(raw, start + 2) {
                refs.push((
                    range.start + definition.span.start..range.end + definition.span.start,
                    definition.dest.to_string(),
                ));
            }
        }
    }
    for (event, span) in parser.into_offset_iter() {
        match event {
            Event::Start(
                Tag::Image {
                    link_type,
                    dest_url,
                    ..
                }
                | Tag::Link {
                    link_type,
                    dest_url,
                    ..
                },
            ) => {
                let raw = &content[span.clone()];
                let range = match link_type {
                    LinkType::Inline => inline_destination(raw),
                    LinkType::WikiLink { .. } => raw.find("[[").map(|start| {
                        let start = start + 2;
                        let end = raw[start..]
                            .find(['|', ']'])
                            .map_or(raw.len(), |n| start + n);
                        start..end
                    }),
                    _ => None,
                };
                if let Some(range) = range {
                    refs.push((
                        range.start + span.start..range.end + span.start,
                        dest_url.to_string(),
                    ));
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                // 处理 HTML 的 image/audio/video 与附件属性；只扫真正的标签，
                // 不碰周围文本和注释。
                static TAGS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                    regex::Regex::new(r#"(?is)<(?:img|audio|video|source|a|object)\b[^>]*>"#)
                        .unwrap()
                });
                static ATTRS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                    regex::Regex::new(
                        r#"(?i)\s(?:src|href|poster|data)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#,
                    )
                    .unwrap()
                });
                if content[span.clone()].trim_start().starts_with("<!--") {
                    continue;
                }
                for tag in TAGS.find_iter(&content[span.clone()]) {
                    for captures in ATTRS.captures_iter(tag.as_str()) {
                        if let Some(value) = captures
                            .get(1)
                            .or_else(|| captures.get(2))
                            .or_else(|| captures.get(3))
                        {
                            let start = span.start + tag.start() + value.start();
                            refs.push((
                                start..start + value.len(),
                                value.as_str().replace("&amp;", "&").replace("&quot;", "\""),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    refs
}

fn inline_destination(raw: &str) -> Option<Range<usize>> {
    // 找到外层标签的收括号（标签里可以再嵌一张图）。
    let bytes = raw.as_bytes();
    let mut depth = 0;
    let mut i = usize::from(bytes.first() == Some(&b'!'));
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 && bytes.get(i + 1) == Some(&b'(') {
                    return destination_range(raw, i + 2);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn destination_range(raw: &str, mut start: usize) -> Option<Range<usize>> {
    let bytes = raw.as_bytes();
    while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
        start += 1;
    }
    let angle = bytes.get(start) == Some(&b'<');
    if angle {
        start += 1;
    }
    let mut end = start;
    let mut depth = 0;
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => {
                end += 2;
                continue;
            }
            b'>' if angle => break,
            b'(' if !angle => depth += 1,
            b')' if !angle && depth == 0 => break,
            b')' if !angle => depth -= 1,
            b if !angle && b.is_ascii_whitespace() => break,
            _ => {}
        }
        end += 1;
    }
    (end > start && end <= raw.len()).then_some(start..end)
}

#[cfg(test)]
mod tests;
