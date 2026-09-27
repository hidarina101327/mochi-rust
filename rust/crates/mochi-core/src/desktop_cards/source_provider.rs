//! 以来源为核心的桌面卡片快照。
//!
//! 普通卡片提供方读取的是小而固定的已知工作区索引。来源页不同：
//! 用户显式选定了一个文档、base、画布或知识库目录。这条边界在这里
//! 一直保持显式，卡片刷新才不会一不留神变成全工作区爬取。

use super::{Action, Module, Page, PageSnapshot, Row, MAX_ROWS_PER_PAGE};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::cmp::Reverse;
use std::fs::{self, DirEntry, File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};

const HARD_MAX_READ_BYTES: usize = 8 * 1024 * 1024;
const MAX_KNOWLEDGE_DEPTH: usize = 8;
const MAX_KNOWLEDGE_VISITS: usize = 4096;
const MAX_TEXT_CHARS_IN_ROW: usize = 240;

/// 为「来源是显式选定条目」的页面构建快照。
///
/// 来源路径一律由 [`super::safe_source_path`] 解析——词法路径和
/// canonicalize 目标都会检查。下面的任何分支都不允许在固定访问
/// 预算之外递归扫描任意工作区。
pub fn source_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let empty = || PageSnapshot::empty(page);

    // 没校验过的上限绝不能悄悄变成无上限的读取或目录枚举。
    // 空选项集刻意快速返回、不解析来源；文件正在重命名时，
    // 被禁用的页面也因此无害。
    if page.options.is_empty() {
        return Ok(empty());
    }
    anyhow::ensure!(max_read_bytes > 0, "来源读取上限必须大于 0");
    anyhow::ensure!(max_entries > 0, "来源条目上限必须大于 0");

    match page.module {
        Module::Document => document_snapshot(root, page, max_read_bytes, max_entries),
        Module::Base => base_snapshot(root, page, max_read_bytes, max_entries),
        Module::Canvas => canvas_snapshot(root, page, max_read_bytes, max_entries),
        Module::Knowledge => knowledge_snapshot(root, page, max_read_bytes, max_entries),
        module => bail!("{} 模块不支持来源分页", module.label()),
    }
}

fn document_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let source = resolve_source(root, page, "文档")?;
    let metadata =
        fs::metadata(&source).with_context(|| format!("无法读取文档来源: {}", source.display()))?;
    anyhow::ensure!(metadata.is_file(), "文档来源不是文件: {}", source.display());
    let text = read_text(&source, max_read_bytes)?;
    let relative = relative_path(root, &source)?;
    let parsed = parse_markdown_source(&text);
    let title = parsed
        .title
        .clone()
        .or_else(|| {
            source
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "文档".into());
    let modified = modified_at(&metadata, &source)?;
    let action = Some(Action::OpenFile(relative.clone()));
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let mut rows = Vec::new();

    // 保持当前数据结构中的 title、wordCount 和 modifiedAt 键稳定。
    // 可选的 headings/excerpt 键由本提供方当作前向兼容的扩展接受；
    // 等设置 UI 就绪后，模块描述符可以再把它们暴露出去。
    if page.selected("title") {
        let mut detail = relative.clone();
        if !parsed.headings.is_empty() {
            detail.push_str(&format!(" · {} 个标题", parsed.headings.len()));
        }
        if let Some(excerpt) = parsed.excerpt.as_deref() {
            detail.push_str(" · ");
            detail.push_str(&clip(excerpt, 150));
        }
        rows.push(Row {
            id: format!("document:title:{relative}"),
            title: title.clone(),
            detail,
            action: action.clone(),
            checked: None,
            meta: Default::default(),
        });
    }
    if page.selected("headings") || page.selected("outline") {
        for (index, heading) in parsed.headings.iter().enumerate() {
            if rows.len() >= row_limit {
                break;
            }
            rows.push(Row {
                id: format!("document:heading:{relative}:{index}"),
                title: heading.text.clone(),
                detail: format!("{} 级标题 · {}", heading.level, relative),
                action: action.clone(),
                checked: None,
                meta: Default::default(),
            });
        }
    }
    if page.selected("excerpt") || page.selected("summary") {
        if rows.len() < row_limit {
            if let Some(excerpt) = parsed.excerpt.as_deref() {
                rows.push(Row {
                    id: format!("document:excerpt:{relative}"),
                    title: "摘要".into(),
                    detail: clip(excerpt, MAX_TEXT_CHARS_IN_ROW),
                    action: action.clone(),
                    checked: None,
                    meta: Default::default(),
                });
            }
        }
    }
    if page.selected("wordCount") && rows.len() < row_limit {
        rows.push(Row {
            id: format!("document:word-count:{relative}"),
            title: "字数".into(),
            detail: format!(
                "{} 字 · {}",
                crate::word_count::count_words(&text),
                relative
            ),
            action: action.clone(),
            checked: None,
            meta: Default::default(),
        });
    }
    if page.selected("modifiedAt") && rows.len() < row_limit {
        rows.push(Row {
            id: format!("document:modified:{relative}"),
            title: "修改时间".into(),
            detail: modified,
            action,
            checked: None,
            meta: Default::default(),
        });
    }
    let mut snapshot = PageSnapshot::empty(page);
    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = "当前文档没有启用的展示内容".into();
    }
    Ok(snapshot)
}

fn base_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let source = resolve_source(root, page, "多维表格")?;
    let metadata = fs::metadata(&source)
        .with_context(|| format!("无法读取多维表格来源: {}", source.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "多维表格来源不是文件: {}",
        source.display()
    );
    let text = read_text(&source, max_read_bytes)?;
    let document = crate::base::parse_base_document(&text)
        .with_context(|| format!("无法解析多维表格来源: {}", source.display()))?;
    let relative = relative_path(root, &source)?;
    let action = Some(Action::OpenFile(relative.clone()));
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let mut rows = Vec::new();

    if page.selected("tables") {
        for table in document.tables.iter().take(row_limit) {
            let fields = if table.fields.is_empty() {
                "无字段".into()
            } else {
                table
                    .fields
                    .iter()
                    .take(4)
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            };
            rows.push(Row {
                id: format!("base:table:{}", table.id),
                title: table.name.clone(),
                detail: clip(
                    &format!("{} 条记录 · {} · {}", table.records.len(), fields, relative),
                    MAX_TEXT_CHARS_IN_ROW,
                ),
                action: action.clone(),
                checked: None,
                meta: Default::default(),
            });
        }
    }

    if page.selected("records") && rows.len() < row_limit {
        let active = document
            .active_table_id
            .as_deref()
            .and_then(|id| document.tables.iter().find(|table| table.id == id))
            .or_else(|| document.tables.first());
        if let Some(table) = active {
            for record in table
                .records
                .iter()
                .take(row_limit.saturating_sub(rows.len()))
            {
                let values = table
                    .fields
                    .iter()
                    .filter_map(|field| {
                        let value = record.values.get(&field.id)?;
                        let rendered = crate::base::format_cell_value(field, value);
                        (!rendered.is_empty()).then(|| format!("{}: {}", field.name, rendered))
                    })
                    .take(3)
                    .collect::<Vec<_>>();
                let title = values
                    .first()
                    .and_then(|value| value.split_once(": ").map(|(_, text)| text.to_owned()))
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("记录 {}", record.id));
                rows.push(Row {
                    id: format!("base:record:{}:{}", table.id, record.id),
                    title: clip(&title, MAX_TEXT_CHARS_IN_ROW),
                    detail: clip(
                        &format!("{} · {}", values.join(" · "), table.name),
                        MAX_TEXT_CHARS_IN_ROW,
                    ),
                    action: action.clone(),
                    checked: None,
                    meta: Default::default(),
                });
            }
        }
    }
    let mut snapshot = PageSnapshot::empty(page);
    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = "当前多维表格没有启用的展示内容".into();
    }
    Ok(snapshot)
}

fn canvas_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let source = resolve_source(root, page, "画布")?;
    let metadata =
        fs::metadata(&source).with_context(|| format!("无法读取画布来源: {}", source.display()))?;
    anyhow::ensure!(metadata.is_file(), "画布来源不是文件: {}", source.display());
    let text = read_text(&source, max_read_bytes)?;
    let canvas = crate::canvas::parse(&text)
        .with_context(|| format!("无法解析画布来源: {}", source.display()))?;
    let relative = relative_path(root, &source)?;
    let action = Some(Action::OpenFile(relative.clone()));
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let mut rows = Vec::new();
    let title = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("画布");

    if page.selected("canvases") {
        let target_preview = canvas
            .cards
            .iter()
            .take(2)
            .map(canvas_target_label)
            .chain(canvas.texts.iter().take(2).map(|text| clip(&text.text, 60)))
            .collect::<Vec<_>>();
        let count = canvas.cards.len() + canvas.texts.len() + canvas.strokes.len();
        let detail = if target_preview.is_empty() {
            format!("{count} 个画布对象 · {relative}")
        } else {
            format!(
                "{} 个画布对象 · {} · {relative}",
                count,
                target_preview.join("、")
            )
        };
        rows.push(Row {
            id: format!("canvas:{relative}"),
            title: title.to_owned(),
            detail: clip(&detail, MAX_TEXT_CHARS_IN_ROW),
            action: action.clone(),
            checked: None,
            meta: Default::default(),
        });
    }
    // `cards`/`nodes`/`content` 是给未来模块描述符的前向兼容别名。
    // 当前的描述符暴露 `canvases`，其中已包含真实对象数和一小段内容预览。
    if page.selected("cards") || page.selected("nodes") || page.selected("content") {
        for card in canvas
            .cards
            .iter()
            .take(row_limit.saturating_sub(rows.len()))
        {
            rows.push(Row {
                id: format!("canvas:card:{}", card.id),
                title: canvas_target_label(card),
                detail: format!("画布对象 · {relative}"),
                action: action.clone(),
                checked: None,
                meta: Default::default(),
            });
        }
    }
    if page.selected("modifiedAt") && rows.len() < row_limit {
        rows.push(Row {
            id: format!("canvas:modified:{relative}"),
            title: "修改时间".into(),
            detail: modified_at(&metadata, &source)?,
            action,
            checked: None,
            meta: Default::default(),
        });
    }
    let mut snapshot = PageSnapshot::empty(page);
    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = "当前画布没有启用的展示内容".into();
    }
    Ok(snapshot)
}

fn knowledge_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let source = resolve_source(root, page, "知识库")?;
    let metadata = fs::metadata(&source)
        .with_context(|| format!("无法读取知识库来源: {}", source.display()))?;
    anyhow::ensure!(
        metadata.is_dir(),
        "知识库来源不是目录: {}",
        source.display()
    );
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let mut rows = Vec::new();

    if page.selected("libraries") {
        let mut directories = immediate_entries(&source, root, max_entries)?
            .into_iter()
            .filter(|entry| entry.metadata().map(|m| m.is_dir()).unwrap_or(false))
            .collect::<Vec<_>>();
        directories.sort_by_key(|entry| entry.file_name());
        if directories.is_empty() {
            let relative = relative_path(root, &source)?;
            let name = source
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("知识库");
            rows.push(Row {
                id: format!("knowledge:library:{relative}"),
                title: name.to_owned(),
                detail: "知识库目录".into(),
                action: Some(Action::OpenFile(relative)),
                checked: None,
                meta: Default::default(),
            });
        } else {
            for entry in directories.into_iter().take(row_limit) {
                let path = canonical_entry_path(&entry, root)?;
                let relative = relative_path(root, &path)?;
                rows.push(Row {
                    id: format!("knowledge:library:{relative}"),
                    title: entry.file_name().to_string_lossy().into_owned(),
                    detail: "知识库目录".into(),
                    action: Some(Action::OpenFile(relative)),
                    checked: None,
                    meta: Default::default(),
                });
            }
        }
    }

    if page.selected("recent") && rows.len() < row_limit {
        let mut budget =
            ScanBudget::new(MAX_KNOWLEDGE_VISITS.min(max_entries.saturating_mul(32).max(1)));
        let mut files = Vec::new();
        collect_recent_files(&source, root, 0, &mut budget, &mut files)?;
        files.sort_by_key(|(mtime, _)| Reverse(*mtime));
        for (mtime, path) in files.into_iter().take(row_limit.saturating_sub(rows.len())) {
            let relative = relative_path(root, &path)?;
            let title = path
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("文档");
            let detail = match mtime {
                Some(time) => format!("{} · {}", format_system_time(time), relative),
                None => relative.clone(),
            };
            rows.push(Row {
                id: format!("knowledge:recent:{relative}"),
                title: title.to_owned(),
                detail: clip(&detail, MAX_TEXT_CHARS_IN_ROW),
                action: Some(Action::OpenFile(relative)),
                checked: None,
                meta: Default::default(),
            });
        }
    }
    let mut snapshot = PageSnapshot::empty(page);
    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = "当前知识库没有启用的展示内容".into();
    }
    // `max_read_bytes` 刻意留在这个 API 里，尽管目录列表只需要元数据。
    // 这里避免读取文件正文，刷新大型知识库时才能保持轻快。
    let _ = max_read_bytes;
    Ok(snapshot)
}

fn resolve_source(root: &Path, page: &Page, label: &str) -> Result<PathBuf> {
    let source = page
        .source
        .as_deref()
        .filter(|source| !source.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{label}来源未配置"))?;
    super::safe_source_path(root, source).with_context(|| format!("{label}分页来源无效: {source}"))
}

fn read_text(path: &Path, max_read_bytes: usize) -> Result<String> {
    let limit = max_read_bytes.min(HARD_MAX_READ_BYTES);
    anyhow::ensure!(limit > 0, "来源读取上限必须大于 0");
    let metadata =
        fs::metadata(path).with_context(|| format!("来源不存在或无法读取: {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "来源不是文件: {}", path.display());
    anyhow::ensure!(
        metadata.len() <= limit as u64,
        "来源文件超过读取上限（{} 字节）: {}",
        limit,
        path.display()
    );
    let file = File::open(path).with_context(|| format!("无法打开来源文件: {}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len().min(limit as u64) as usize);
    file.take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("无法读取来源文件: {}", path.display()))?;
    anyhow::ensure!(
        bytes.len() <= limit,
        "来源文件在读取期间超过上限（{} 字节）: {}",
        limit,
        path.display()
    );
    let text = String::from_utf8(bytes)
        .with_context(|| format!("来源文件不是 UTF-8 文本: {}", path.display()))?;
    Ok(text.trim_start_matches('\u{feff}').to_owned())
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let root = root
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", root.display()))?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("来源不存在或无法读取: {}", path.display()))?;
    anyhow::ensure!(
        crate::paths::path_is_within(&root, &canonical),
        "来源路径越出工作区: {}",
        path.display()
    );
    let relative = canonical
        .strip_prefix(&root)
        .context("来源路径无法转换为工作区相对路径")?
        .to_string_lossy()
        .replace('\\', "/");
    anyhow::ensure!(!relative.is_empty(), "来源不能是工作区根目录");
    Ok(relative)
}

fn modified_at(metadata: &Metadata, path: &Path) -> Result<String> {
    let modified = metadata
        .modified()
        .with_context(|| format!("无法读取来源修改时间: {}", path.display()))?;
    Ok(format_system_time(modified))
}

fn format_system_time(time: std::time::SystemTime) -> String {
    crate::jstime::from(DateTime::<Utc>::from(time))
}

fn clip(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let value = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{value}…")
    } else {
        value
    }
}

#[derive(Debug, Clone)]
struct Heading {
    level: usize,
    text: String,
}

#[derive(Debug, Default)]
struct MarkdownSource {
    title: Option<String>,
    headings: Vec<Heading>,
    excerpt: Option<String>,
}

fn parse_markdown_source(source: &str) -> MarkdownSource {
    let mut result = MarkdownSource::default();
    let mut in_frontmatter = false;
    let mut excerpt = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "---" && result.headings.is_empty() && excerpt.is_none() {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter || trimmed.is_empty() || trimmed == "---" {
            continue;
        }
        if let Some((level, text)) = parse_heading(trimmed) {
            if result.title.is_none() {
                result.title = Some(text.clone());
            }
            result.headings.push(Heading { level, text });
            continue;
        }
        if excerpt.is_none() {
            excerpt = Some(clip(trimmed, MAX_TEXT_CHARS_IN_ROW));
        }
    }
    result.excerpt = excerpt;
    result
}

fn parse_heading(line: &str) -> Option<(usize, String)> {
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let text = line[level..].trim();
    (!text.is_empty()).then(|| (level, clip(text, MAX_TEXT_CHARS_IN_ROW)))
}

fn canvas_target_label(card: &crate::canvas::Card) -> String {
    match &card.target {
        crate::canvas::Target::Mochi { url } => clip(url, 100),
        crate::canvas::Target::PdfAnnotation {
            path,
            annotation_id,
        } => clip(&format!("{}#{}", path, annotation_id), 100),
    }
}

fn immediate_entries(source: &Path, root: &Path, max_entries: usize) -> Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(source)
        .with_context(|| format!("无法读取目录来源: {}", source.display()))?
        .take(max_entries)
    {
        let entry = entry.with_context(|| format!("读取目录条目失败: {}", source.display()))?;
        // 用条目构造动作之前先 canonicalize。坏链接和工作区外的链接
        // 一律忽略，不让它们变成不安全的打开目标。
        if canonical_entry_path(&entry, root).is_ok() {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn canonical_entry_path(entry: &DirEntry, root: &Path) -> Result<PathBuf> {
    let path = entry.path();
    let canonical = path
        .canonicalize()
        .with_context(|| format!("目录条目不存在或无法读取: {}", path.display()))?;
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", root.display()))?;
    anyhow::ensure!(
        crate::paths::path_is_within(&canonical_root, &canonical),
        "目录条目越出工作区: {}",
        path.display()
    );
    Ok(canonical)
}

#[derive(Debug)]
struct ScanBudget {
    remaining: usize,
}

impl ScanBudget {
    fn new(remaining: usize) -> Self {
        Self { remaining }
    }

    fn spend(&mut self) -> bool {
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }
}

fn collect_recent_files(
    directory: &Path,
    root: &Path,
    depth: usize,
    budget: &mut ScanBudget,
    output: &mut Vec<(Option<std::time::SystemTime>, PathBuf)>,
) -> Result<()> {
    if depth > MAX_KNOWLEDGE_DEPTH || !budget.spend() {
        return Ok(());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("无法读取知识库目录: {}", directory.display()))?
        .take(budget.remaining.min(256))
    {
        let entry = entry.with_context(|| format!("读取目录条目失败: {}", directory.display()))?;
        if canonical_entry_path(&entry, root).is_ok() {
            entries.push(entry);
        }
        if !budget.spend() {
            break;
        }
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(canonical) = canonical_entry_path(&entry, root) else {
            continue;
        };
        let metadata = match fs::metadata(&canonical) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            collect_recent_files(&canonical, root, depth + 1, budget, output)?;
        } else if metadata.is_file() && is_document_extension(&path) {
            output.push((metadata.modified().ok(), canonical));
        }
        if !budget.spend() {
            break;
        }
    }
    Ok(())
}

fn is_document_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "mc" | "mdx" | "markdown" | "txt" | "exam")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_cards::Module;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn workspace(tag: &str) -> PathBuf {
        let number = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mochi-source-provider-{tag}-{number}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn page(module: Module, source: &str, options: &[&str]) -> Page {
        let mut page = Page::with_id(module, "test-page");
        page.source = Some(source.into());
        page.options = options.iter().map(|value| (*value).into()).collect();
        page
    }

    #[test]
    fn document_snapshot_reads_real_markdown_content_with_bounds() {
        let root = workspace("document");
        fs::write(
            root.join("note.mc"),
            "\u{feff}# 项目计划\n\n这是一个包含 alpha beta 的摘要。\n## 下一步\n",
        )
        .unwrap();
        let snapshot = source_snapshot(
            &root,
            &page(
                Module::Document,
                "note.mc",
                &["title", "headings", "excerpt", "wordCount", "modifiedAt"],
            ),
            4096,
            12,
        )
        .unwrap();
        assert!(snapshot.rows.iter().any(|row| row.title == "项目计划"));
        assert!(snapshot.rows.iter().any(|row| row.title == "下一步"));
        assert!(snapshot.rows.iter().any(|row| row.title == "摘要"));
        assert!(snapshot.rows.iter().any(|row| row.title == "字数"));
        assert!(snapshot.rows.iter().any(|row| row.title == "修改时间"));
        assert!(snapshot
            .rows
            .iter()
            .all(|row| row.action == Some(Action::OpenFile("note.mc".into()))));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn base_snapshot_parses_the_checked_in_mcb_fixture() {
        let root = workspace("base");
        fs::write(
            root.join("table.mcb"),
            include_str!("../../../../../tests/fixtures/base-v1.mcb"),
        )
        .unwrap();
        let snapshot = source_snapshot(
            &root,
            &page(Module::Base, "table.mcb", &["tables", "records"]),
            256 * 1024,
            12,
        )
        .unwrap();
        assert!(snapshot.rows.iter().any(|row| row.title == "数据表"));
        assert!(snapshot
            .rows
            .iter()
            .any(|row| row.detail.contains("条记录")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canvas_snapshot_counts_persisted_cards() {
        let root = workspace("canvas");
        let mut canvas = crate::canvas::CanvasDocument::empty();
        canvas
            .add_mochi_reference("mochi://open?path=notes%2Fa.md&kind=file&label=A")
            .unwrap();
        canvas
            .add_pdf_annotation("papers/a.pdf", "annotation_1")
            .unwrap();
        fs::write(
            root.join("board.mcanvas"),
            crate::canvas::serialize(&canvas).unwrap(),
        )
        .unwrap();
        let snapshot = source_snapshot(
            &root,
            &page(Module::Canvas, "board.mcanvas", &["canvases", "modifiedAt"]),
            256 * 1024,
            12,
        )
        .unwrap();
        assert!(snapshot.rows[0].detail.contains("2 个画布对象"));
        assert!(snapshot.rows.iter().any(|row| row.title == "修改时间"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canvas_snapshot_counts_free_notes_without_reference_cards() {
        let root = workspace("canvas-free-notes");
        let mut canvas = crate::canvas::CanvasDocument::empty();
        canvas.texts.push(crate::canvas::TextNote {
            id: "text".into(),
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 64.0,
            text: "自由笔记".into(),
            color: 0,
            font_size: 24.0,
        });
        canvas.strokes.push(crate::canvas::Stroke {
            id: "stroke".into(),
            points: vec![crate::canvas::Point { x: 1.0, y: 2.0 }],
            color: 0,
            width: 4.0,
        });
        fs::write(
            root.join("board.mcanvas"),
            crate::canvas::serialize(&canvas).unwrap(),
        )
        .unwrap();
        let snapshot = source_snapshot(
            &root,
            &page(Module::Canvas, "board.mcanvas", &["canvases"]),
            256 * 1024,
            12,
        )
        .unwrap();
        assert!(snapshot.rows[0].detail.contains("2 个画布对象"));
        assert!(snapshot.rows[0].detail.contains("自由笔记"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn knowledge_snapshot_is_bounded_and_rejects_escape() {
        let root = workspace("knowledge");
        fs::create_dir_all(root.join("知识库").join("库一")).unwrap();
        for index in 0..20 {
            fs::write(
                root.join("知识库")
                    .join("库一")
                    .join(format!("note-{index}.md")),
                format!("# Note {index}\nbody"),
            )
            .unwrap();
        }
        let snapshot = source_snapshot(
            &root,
            &page(Module::Knowledge, "知识库", &["libraries", "recent"]),
            4096,
            3,
        )
        .unwrap();
        assert!(snapshot.rows.len() <= 3);
        assert!(source_snapshot(
            &root,
            &page(Module::Knowledge, "../outside", &["recent"]),
            4096,
            3,
        )
        .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_source_is_an_error_and_empty_options_are_cheap() {
        let root = workspace("limits");
        fs::write(root.join("large.md"), "0123456789").unwrap();
        assert!(source_snapshot(
            &root,
            &page(Module::Document, "large.md", &["title"]),
            4,
            12,
        )
        .is_err());
        let disabled = page(Module::Document, "missing.md", &[]);
        assert!(source_snapshot(&root, &disabled, 0, 0)
            .unwrap()
            .rows
            .is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
