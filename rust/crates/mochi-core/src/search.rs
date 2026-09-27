//! FTS5 只筛候选，逐行正则决定最终命中。对外 span 使用 UTF-16 码元偏移。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use regex::Regex;

use crate::files::locale_compare;
use crate::metadata_index::{FtsCandidate, MetadataIndexService};
use crate::note_parser;

const PREVIEW_LENGTH: usize = 260;
const PREVIEW_LEAD: usize = 48;
const READ_CONCURRENCY: usize = 12;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
const MAX_SPANS_PER_LINE: usize = 64;

// ---------- 结果形状（与 workspace-search.ts 的接口一一对应）----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatchSpan {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResultLine {
    pub id: String,
    pub path: String,
    pub line: u32,
    pub content: String,
    pub score: f64,
    pub match_start: u32,
    pub match_end: u32,
    pub spans: Vec<SearchMatchSpan>,
    pub match_text: String,
    pub occurrence: u32,
    pub base_location: Option<crate::base::BaseLocation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchFileGroup {
    pub path: String,
    pub title: String,
    pub rel_path: String,
    pub score: f64,
    pub match_count: u32,
    pub title_matched: bool,
    pub mtime_ms: i64,
    pub matches: Vec<SearchResultLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchStats {
    pub total_matches: u32,
    pub total_files: usize,
    pub truncated: bool,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchQueryResult {
    pub groups: Vec<SearchFileGroup>,
    pub stats: SearchStats,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub whole_word: bool,
    /// 多词查询是否要求整句连续出现（否则 AND 关系）。
    pub match_phrase: bool,
    /// 限定扩展名（含点），空表示不限。
    pub extensions: Vec<String>,
    pub max_results: usize,
    pub max_per_file: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            use_regex: false,
            whole_word: false,
            match_phrase: false,
            extensions: Vec::new(),
            max_results: 200,
            max_per_file: 8,
        }
    }
}

// ---------- 查询解析 ----------

static QUERY_TOKENIZER: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r#""([^"]*)"|(\S+)"#).unwrap());

/// 空白拆词；`"..."` 内作为整体。
pub fn tokenize_query(raw: &str) -> Vec<String> {
    QUERY_TOKENIZER
        .captures_iter(raw)
        .filter_map(|c| {
            let term = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str().trim().to_owned())?;
            (!term.is_empty()).then_some(term)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    pub terms: Vec<String>,
    pub fts_terms: Vec<String>,
}

pub fn parse_query(raw: &str, options: &SearchOptions) -> ParsedQuery {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParsedQuery {
            terms: vec![],
            fts_terms: vec![],
        };
    }
    if options.use_regex {
        // 正则无法转成 FTS 表达式 → fts_terms 留空，调用方回退全量扫描
        return ParsedQuery {
            terms: vec![trimmed.into()],
            fts_terms: vec![],
        };
    }
    if options.match_phrase {
        return ParsedQuery {
            terms: vec![trimmed.into()],
            fts_terms: vec![trimmed.into()],
        };
    }
    let terms = tokenize_query(trimmed);
    if terms.is_empty() {
        ParsedQuery {
            terms: vec![trimmed.into()],
            fts_terms: vec![],
        }
    } else {
        ParsedQuery {
            fts_terms: terms.clone(),
            terms,
        }
    }
}

// ---------- 匹配器 ----------

/// `\b` 对中文永远不成立；中文本无词边界概念，直接跳过（与旧版一致）。
///
/// `(?-u:\b)` 关掉 Unicode 词边界，退回 ASCII 语义——这才等价于 JS 不带 `u` 标志的 `\b`，
/// 也是 C# 版用 `RegexOptions.ECMAScript` 想要的效果。
fn apply_whole_word(source: &str, term: &str) -> String {
    if note_parser::contains_cjk(term) {
        source.to_owned()
    } else {
        format!(r"(?-u:\b)(?:{source})(?-u:\b)")
    }
}

fn build_term_pattern(term: &str, options: &SearchOptions) -> String {
    let source = if options.use_regex {
        term.to_owned()
    } else {
        regex::escape(term)
    };
    if options.whole_word {
        apply_whole_word(&source, term)
    } else {
        source
    }
}

pub struct QueryMatcher {
    pub combined: Regex,
    pub per_term: Vec<Regex>,
    pub terms: Vec<String>,
}

/// 正则语法非法时返回 `Err`，由调用方转为提示。
pub fn create_matcher(
    parsed: &ParsedQuery,
    options: &SearchOptions,
) -> Result<QueryMatcher, regex::Error> {
    let flags = if options.case_sensitive { "" } else { "(?i)" };
    let patterns: Vec<String> = parsed
        .terms
        .iter()
        .map(|t| build_term_pattern(t, options))
        .collect();

    let combined_src = format!(
        "{flags}{}",
        patterns
            .iter()
            .map(|p| format!("(?:{p})"))
            .collect::<Vec<_>>()
            .join("|")
    );
    let combined = Regex::new(&combined_src)?;
    let per_term = patterns
        .iter()
        .map(|p| Regex::new(&format!("{flags}{p}")))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(QueryMatcher {
        combined,
        per_term,
        terms: parsed.terms.clone(),
    })
}

/// 字节偏移 → UTF-16 码元偏移。
fn to_utf16_offset(line: &str, byte_offset: usize) -> usize {
    line[..byte_offset].encode_utf16().count()
}

/// 收集一行内全部命中区间（UTF-16 偏移）；零宽匹配跳过防死循环。
pub fn collect_line_spans(line: &str, matcher: &Regex, limit: usize) -> Vec<SearchMatchSpan> {
    let mut spans = Vec::new();
    for m in matcher.find_iter(line) {
        if m.start() == m.end() {
            continue;
        }
        spans.push(SearchMatchSpan {
            start: to_utf16_offset(line, m.start()) as u32,
            end: to_utf16_offset(line, m.end()) as u32,
        });
        if spans.len() >= limit {
            break;
        }
    }
    spans
}

// ---------- 预览 ----------

/// 长行裁成以首个命中为中心的预览片段，同步平移高亮区间，两端补省略号。
///
/// 全程在 **UTF-16 码元**空间里算，与 JS 的字符串下标语义一致——
/// 用字节算的话 260 的窗口对中文只有约 86 个字，预览会明显偏短。
pub fn build_preview(line: &str, spans: &[SearchMatchSpan]) -> (String, Vec<SearchMatchSpan>) {
    let units: Vec<u16> = line.encode_utf16().collect();
    let len = units.len();
    let trimmed_start = len - line.trim_start().encode_utf16().count();
    let anchor = spans
        .first()
        .map(|s| s.start as usize)
        .unwrap_or(trimmed_start);

    if len <= PREVIEW_LENGTH && trimmed_start == 0 {
        return (line.to_owned(), spans.to_vec());
    }

    let mut start = trimmed_start.max(anchor.saturating_sub(PREVIEW_LEAD));
    if len - start < PREVIEW_LENGTH {
        start = trimmed_start.max(len.saturating_sub(PREVIEW_LENGTH));
    }
    let end = len.min(start + PREVIEW_LENGTH);

    let prefix = if start > trimmed_start { "…" } else { "" };
    let suffix = if end < len { "…" } else { "" };
    let prefix_len = prefix.encode_utf16().count();

    let shifted = spans
        .iter()
        .filter(|s| (s.end as usize) > start && (s.start as usize) < end)
        .map(|s| SearchMatchSpan {
            start: prefix_len.max((s.start as usize + prefix_len).saturating_sub(start)) as u32,
            end: (prefix_len + (end - start))
                .min((s.end as usize + prefix_len).saturating_sub(start)) as u32,
        })
        .collect();

    let body = String::from_utf16_lossy(&units[start..end]);
    (format!("{prefix}{body}{suffix}"), shifted)
}

// ---------- 打分 ----------

static HEADING_LINE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s+\S").unwrap());
static LIST_MARKER: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d+\.)\s+").unwrap());

fn score_line(line: &str, spans: &[SearchMatchSpan], line_index: usize) -> f64 {
    let mut score = 1.0;
    if HEADING_LINE.is_match(line) {
        score += 3.0;
    } else if LIST_MARKER.is_match(line) {
        score += 0.3;
    }
    score += (spans.len().saturating_sub(1)).min(4) as f64 * 0.8;
    if spans.first().map(|s| s.start).unwrap_or(0) <= 8 {
        score += 0.5;
    }
    score += 1.0 / (1.0 + line_index as f64 * 0.02);
    score
}

fn normalize_for_compare(value: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        value.to_owned()
    } else {
        value.to_lowercase()
    }
}

fn score_title(title: &str, terms: &[String], case_sensitive: bool) -> f64 {
    let haystack = normalize_for_compare(title, case_sensitive);
    let mut score = 0.0;
    for term in terms {
        let needle = normalize_for_compare(term, case_sensitive);
        if needle.is_empty() {
            continue;
        }
        if haystack == needle {
            score += 10.0;
        } else if haystack.starts_with(&needle) {
            score += 5.0;
        } else if haystack.contains(&needle) {
            score += 3.0;
        }
    }
    score
}

fn score_path(rel_path: &str, terms: &[String], case_sensitive: bool) -> f64 {
    let haystack = normalize_for_compare(rel_path, case_sensitive);
    terms
        .iter()
        .map(|t| normalize_for_compare(t, case_sensitive))
        .filter(|n| !n.is_empty() && haystack.contains(n.as_str()))
        .count() as f64
        * 0.6
}

fn score_recency(mtime_ms: i64, now: i64) -> f64 {
    if mtime_ms == 0 {
        return 0.0;
    }
    let age_days = (((now - mtime_ms) as f64) / DAY_MS).max(0.0);
    2.0 / (1.0 + age_days / 7.0)
}

/// bm25 是负数、越负越相关；归一到 0..3 后并入总分。
fn normalize_rank(rank: f64) -> f64 {
    if !rank.is_finite() || rank >= 0.0 {
        0.0
    } else {
        (-rank / 4.0).min(3.0)
    }
}

// ---------- 文件扫描 ----------

struct FileScan {
    lines: Vec<SearchResultLine>,
    total_matches: u32,
    matched_all_terms: bool,
}

fn scan_file_content(
    file_path: &str,
    content: &str,
    matcher: &QueryMatcher,
    max_per_file: usize,
) -> FileScan {
    let mut scored: Vec<SearchResultLine> = Vec::new();
    let mut total_matches = 0u32;

    let needs_term_check = matcher.per_term.len() > 1;
    let mut seen_terms = vec![false; matcher.per_term.len()];
    let mut occurrences: HashMap<String, u32> = HashMap::new();

    for (index, line) in content
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .enumerate()
    {
        let spans = collect_line_spans(line, &matcher.combined, MAX_SPANS_PER_LINE);
        if spans.is_empty() {
            continue;
        }
        total_matches += spans.len() as u32;

        if needs_term_check {
            for (t, re) in matcher.per_term.iter().enumerate() {
                if !seen_terms[t] && re.is_match(line) {
                    seen_terms[t] = true;
                }
            }
        }

        let units: Vec<u16> = line.encode_utf16().collect();
        let slice = |s: &SearchMatchSpan| {
            String::from_utf16_lossy(&units[s.start as usize..s.end as usize])
        };

        let match_text = slice(&spans[0]);
        let mut occurrence = 0u32;
        for (i, span) in spans.iter().enumerate() {
            let key = slice(span).to_lowercase();
            let seen = occurrences.get(&key).copied().unwrap_or(0);
            if i == 0 {
                occurrence = seen;
            }
            occurrences.insert(key, seen + 1);
        }

        let (preview_content, preview_spans) = build_preview(line, &spans);
        let first = preview_spans
            .first()
            .copied()
            .unwrap_or(SearchMatchSpan { start: 0, end: 0 });

        scored.push(SearchResultLine {
            id: format!("{file_path}:{}:{}", index + 1, spans[0].start),
            path: file_path.to_owned(),
            line: (index + 1) as u32,
            content: preview_content,
            score: score_line(line, &spans, index),
            match_start: first.start,
            match_end: first.end,
            spans: preview_spans,
            match_text,
            occurrence,
            base_location: None,
        });
    }

    scored.sort_by(|l, r| {
        r.score
            .partial_cmp(&l.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| l.line.cmp(&r.line))
    });
    scored.truncate(max_per_file);

    FileScan {
        lines: scored,
        total_matches,
        matched_all_terms: !needs_term_check || seen_terms.iter().all(|b| *b),
    }
}

fn scan_base_content(
    file_path: &str,
    content: &str,
    matcher: &QueryMatcher,
    max_per_file: usize,
) -> Option<FileScan> {
    let document = crate::base::parse_base_document(content).ok()?;
    let mut lines = Vec::new();
    let mut locations = Vec::new();
    // 文件名命中也照样打开 base，但不会假装文件名是个单元格。
    lines.push(
        Path::new(file_path)
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    locations.push(None);
    for entry in crate::base::base_search_entries(&document) {
        for line in entry.text.lines() {
            lines.push(line.to_owned());
            locations.push(Some(entry.location.clone()));
        }
    }
    let mut scan = scan_file_content(file_path, &lines.join("\n"), matcher, max_per_file);
    for matched in &mut scan.lines {
        matched.base_location = locations
            .get(matched.line.saturating_sub(1) as usize)
            .cloned()
            .flatten();
    }
    Some(scan)
}

// ---------- 主流程 ----------

pub struct SearchService {
    workspace_path: PathBuf,
    index: Arc<MetadataIndexService>,
}

impl SearchService {
    pub fn new(workspace_path: impl AsRef<Path>, index: Arc<MetadataIndexService>) -> Self {
        Self {
            workspace_path: workspace_path.as_ref().to_path_buf(),
            index,
        }
    }

    pub fn search(&self, raw_query: &str, options: &SearchOptions) -> SearchQueryResult {
        self.search_cancellable(raw_query, options, &AtomicBool::new(false))
    }

    /// 在打开更多文件之前，先停掉已被取代的 UI 请求。现有调用方
    /// 保持同步 API 与一致的结果语义不变。
    pub fn search_cancellable(
        &self,
        raw_query: &str,
        options: &SearchOptions,
        cancel: &AtomicBool,
    ) -> SearchQueryResult {
        let started_at = crate::jstime::now_millis();
        let empty = |error: Option<String>| SearchQueryResult {
            groups: vec![],
            stats: SearchStats {
                total_matches: 0,
                total_files: 0,
                truncated: false,
                duration_ms: crate::jstime::now_millis() - started_at,
            },
            error,
        };

        if cancel.load(AtomicOrdering::Relaxed) {
            return empty(Some("搜索已取消".into()));
        }
        let parsed = parse_query(raw_query, options);
        if parsed.terms.is_empty() {
            return empty(None);
        }

        let matcher = match create_matcher(&parsed, options) {
            Ok(m) => m,
            Err(_) => {
                return empty(Some(
                    if options.use_regex {
                        "正则表达式语法有误"
                    } else {
                        "搜索条件无法解析"
                    }
                    .into(),
                ))
            }
        };

        let candidates = self.collect_candidates(&parsed, options);
        if cancel.load(AtomicOrdering::Relaxed) {
            return empty(Some("搜索已取消".into()));
        }
        let now = crate::jstime::now_millis();
        let name_terms: &[String] = if options.use_regex {
            &[]
        } else {
            &matcher.terms
        };

        // 并发读文件。用作用域线程 + 原子游标做工作窃取，避免引入 rayon/tokio 依赖。
        let cursor = AtomicUsize::new(0);
        let collected: Mutex<Vec<SearchFileGroup>> = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..READ_CONCURRENCY.min(candidates.len().max(1)) {
                scope.spawn(|| loop {
                    if cancel.load(AtomicOrdering::Relaxed) {
                        break;
                    }
                    let i = cursor.fetch_add(1, AtomicOrdering::Relaxed);
                    let Some(candidate) = candidates.get(i) else {
                        break;
                    };
                    if let Some(group) =
                        scan_candidate(candidate, &matcher, options, name_terms, now)
                    {
                        collected
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(group);
                    }
                });
            }
        });

        if cancel.load(AtomicOrdering::Relaxed) {
            return empty(Some("搜索已取消".into()));
        }
        let mut hits = collected.into_inner().unwrap_or_else(|e| e.into_inner());
        hits.sort_by(|l, r| {
            r.score
                .partial_cmp(&l.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| locale_compare(&l.rel_path, &r.rel_path))
        });

        let mut limited = Vec::new();
        let mut shown = 0usize;
        for group in &hits {
            if shown >= options.max_results {
                break;
            }
            shown += group.matches.len();
            limited.push(group.clone());
        }

        SearchQueryResult {
            stats: SearchStats {
                total_matches: hits.iter().map(|g| g.match_count).sum(),
                total_files: hits.len(),
                truncated: hits.len() > limited.len(),
                duration_ms: crate::jstime::now_millis() - started_at,
            },
            groups: limited,
            error: None,
        }
    }

    /// 候选集：FTS 优先；索引帮不上忙时才扫全库
    /// （正则无法转 FTS / 无一条候选——后者覆盖 FTS 词中匹配的短板）。
    fn collect_candidates(
        &self,
        parsed: &ParsedQuery,
        options: &SearchOptions,
    ) -> Vec<FtsCandidate> {
        // 写成自由函数而不是闭包：闭包会持有 map 的可变借用，后面就没法读 map.is_empty()
        fn add(
            map: &mut HashMap<String, FtsCandidate>,
            options: &SearchOptions,
            candidate: FtsCandidate,
        ) {
            if !options.extensions.is_empty() {
                let ext = Path::new(&candidate.path)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                    .unwrap_or_default();
                if !options.extensions.iter().any(|e| e.to_lowercase() == ext) {
                    return;
                }
            }
            let key = candidate.path.to_lowercase();
            // 同一文件保留带 bm25 的那份
            match map.get(&key) {
                Some(existing) if !(existing.rank == 0.0 && candidate.rank != 0.0) => {}
                _ => {
                    map.insert(key, candidate);
                }
            }
        }

        let mut map: HashMap<String, FtsCandidate> = HashMap::new();

        let fts = if parsed.fts_terms.is_empty() {
            None
        } else {
            self.index
                .find_fts_candidates(&parsed.fts_terms, options.max_results)
        };
        if let Some(list) = &fts {
            for c in list {
                add(&mut map, options, c.clone());
            }
        }

        if fts.is_none() || map.is_empty() {
            let indexed = self.index.list_indexed_files();
            if !indexed.is_empty() {
                for c in indexed {
                    add(&mut map, options, c);
                }
            } else {
                for file in collect_disk_files(&self.workspace_path) {
                    let rel = crate::paths::to_forward_slashes(
                        &file
                            .strip_prefix(&self.workspace_path)
                            .unwrap_or(&file)
                            .to_string_lossy(),
                    );
                    add(
                        &mut map,
                        options,
                        FtsCandidate {
                            path: file.to_string_lossy().into_owned(),
                            title: file
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            rel_path: rel,
                            mtime_ms: 0,
                            rank: 0.0,
                        },
                    );
                }
            }
        }

        // 旧索引可能把引用 URL 本身而不是显示标签建进了索引。
        // 在 base 重建索引之前，先让它留在候选集里。
        for candidate in self
            .index
            .list_indexed_files()
            .into_iter()
            .filter(|candidate| candidate.path.to_ascii_lowercase().ends_with(".mcb"))
        {
            add(&mut map, options, candidate);
        }
        map.into_values().collect()
    }
}

fn scan_candidate(
    candidate: &FtsCandidate,
    matcher: &QueryMatcher,
    options: &SearchOptions,
    name_terms: &[String],
    now: i64,
) -> Option<SearchFileGroup> {
    let meta = std::fs::metadata(&candidate.path).ok()?;
    if meta.len() > MAX_FILE_SIZE {
        return None;
    }
    let content = std::fs::read_to_string(&candidate.path).ok()?;

    let scan = if candidate.path.to_ascii_lowercase().ends_with(".mcb") {
        scan_base_content(&candidate.path, &content, matcher, options.max_per_file)?
    } else {
        scan_file_content(&candidate.path, &content, matcher, options.max_per_file)
    };
    if scan.lines.is_empty() || !scan.matched_all_terms {
        return None;
    }

    let mtime_ms = if candidate.mtime_ms != 0 {
        candidate.mtime_ms
    } else {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    };

    let title_score = score_title(&candidate.title, name_terms, options.case_sensitive);
    let score = normalize_rank(candidate.rank)
        + title_score
        + score_path(&candidate.rel_path, name_terms, options.case_sensitive)
        + (1.0 + scan.total_matches as f64).log2()
        + score_recency(mtime_ms, now)
        + scan.lines[0].score * 0.4;

    Some(SearchFileGroup {
        path: candidate.path.clone(),
        title: candidate.title.clone(),
        rel_path: if candidate.rel_path.is_empty() {
            Path::new(&candidate.path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            candidate.rel_path.clone()
        },
        score,
        match_count: scan.total_matches,
        title_matched: title_score > 0.0,
        mtime_ms,
        matches: scan.lines,
    })
}

fn collect_disk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(root, &mut files);
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(t) if t.is_file() => {
                let ext = path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                    .unwrap_or_default();
                if note_parser::indexable_extensions().contains(ext.as_str()) {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicUsize;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf, Arc<MetadataIndexService>);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("mochi-search-{}-{tag}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            let idx = Arc::new(MetadataIndexService::new(&p));
            idx.open().unwrap();
            Self(p, idx)
        }
        fn write(&self, rel: &str, content: &str) {
            let p = self.0.join(rel);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d).unwrap();
            }
            fs::write(&p, content).unwrap();
        }
        fn svc(&self) -> SearchService {
            self.1.run_full_index().unwrap();
            SearchService::new(self.1.workspace_path(), Arc::clone(&self.1))
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn opts() -> SearchOptions {
        SearchOptions::default()
    }
    #[test]
    fn base_matches_display_labels_and_returns_stable_locations_without_json_noise() {
        use crate::base::*;
        use serde_json::json;
        let ws = TempWs::new("base-display");
        let mut document = create_base_document();
        let table = &mut document.tables[0];
        table.name = "成长计划".into();
        table.fields[0].name = "学习主题".into();
        let mut select = create_base_field(FieldType::SingleSelect, "进展");
        select.options.push(BaseOption {
            id: "secret_option_id".into(),
            label: "已掌握".into(),
            color: OptionColor::Green,
            ..Default::default()
        });
        let reference = create_base_field(FieldType::Reference, "材料");
        table.fields.push(select);
        table.fields.push(reference);
        let mut record = BaseRecord {
            id: "row_stable".into(),
            ..Default::default()
        };
        record
            .values
            .insert(table.fields[0].id.clone(), json!("中文学习\n深入复习"));
        record
            .values
            .insert(table.fields[1].id.clone(), json!("secret_option_id"));
        record.values.insert(
            table.fields[2].id.clone(),
            json!(["mochi://open?path=source.md&label=%E5%BC%95%E7%94%A8%E6%95%99%E6%9D%90"]),
        );
        table.records.push(record);
        let table_id = table.id.clone();
        let field_id = table.fields[1].id.clone();
        ws.write("工作台.mcb", &serialize_base_document(&document).unwrap());
        let result = ws.svc().search("已掌握", &opts());
        let hit = &result.groups[0].matches[0];
        assert_eq!(hit.content, "已掌握");
        assert_eq!(
            hit.base_location,
            Some(BaseLocation {
                table_id,
                record_id: Some("row_stable".into()),
                field_id: Some(field_id)
            })
        );
        assert_eq!(ws.svc().search("引用教材", &opts()).stats.total_files, 1);
        assert_eq!(
            ws.svc().search("深入复习", &opts()).groups[0].matches[0]
                .base_location
                .as_ref()
                .unwrap()
                .record_id
                .as_deref(),
            Some("row_stable")
        );
        for noise in ["secret_option_id", "mochi-base", "row_stable", "source.md"] {
            assert_eq!(
                ws.svc().search(noise, &opts()).stats.total_files,
                0,
                "{noise}"
            );
        }
        assert_eq!(ws.svc().search("工作台", &opts()).stats.total_files, 1);
        assert!(ws.svc().search("学习主题", &opts()).groups[0].matches[0]
            .base_location
            .as_ref()
            .unwrap()
            .record_id
            .is_none());
    }

    #[test]
    fn cancelled_search_returns_no_partial_results_and_next_search_still_works() {
        let ws = TempWs::new("cancel");
        ws.write("note.md", "中文目标");
        let svc = ws.svc();
        let result = svc.search_cancellable("中文", &opts(), &AtomicBool::new(true));
        assert_eq!(result.error.as_deref(), Some("搜索已取消"));
        assert!(result.groups.is_empty());
        assert_eq!(svc.search("中文", &opts()).stats.total_files, 1);
    }

    #[test]
    fn tokenize_respects_quotes() {
        assert_eq!(tokenize_query("a b"), ["a", "b"]);
        assert_eq!(tokenize_query("\"hello world\" x"), ["hello world", "x"]);
        assert_eq!(tokenize_query("   "), Vec::<String>::new());
    }

    #[test]
    fn regex_mode_keeps_query_whole_and_skips_fts() {
        let p = parse_query(
            "a.*b",
            &SearchOptions {
                use_regex: true,
                ..opts()
            },
        );
        assert_eq!(p.terms, ["a.*b"]);
        assert!(p.fts_terms.is_empty(), "正则不能转 FTS，必须回退全量扫描");
    }

    #[test]
    fn phrase_mode_keeps_query_whole() {
        let p = parse_query(
            "hello world",
            &SearchOptions {
                match_phrase: true,
                ..opts()
            },
        );
        assert_eq!(p.terms, ["hello world"]);
        assert_eq!(p.fts_terms, ["hello world"]);
    }

    /// span 必须是 UTF-16 偏移，否则中文行的高亮会错位。
    #[test]
    fn spans_are_utf16_offsets_not_bytes() {
        let re = Regex::new("世界").unwrap();
        let spans = collect_line_spans("你好世界", &re, 64);
        assert_eq!(
            spans,
            [SearchMatchSpan { start: 2, end: 4 }],
            "看起来是字节偏移"
        );
    }

    #[test]
    fn preview_keeps_short_lines_intact() {
        let spans = [SearchMatchSpan { start: 0, end: 2 }];
        let (content, out) = build_preview("短行", &spans);
        assert_eq!(content, "短行");
        assert_eq!(out, spans);
    }

    /// 预览窗口按字符算而不是字节——用字节的话中文只能显示三分之一。
    #[test]
    fn preview_window_is_measured_in_utf16_units() {
        let line = "中".repeat(400);
        // 命中在行首附近：窗口两端都要有省略号
        let spans = [SearchMatchSpan {
            start: 100,
            end: 101,
        }];
        let (content, out) = build_preview(&line, &spans);

        let visible = content.chars().filter(|c| *c == '中').count();
        assert_eq!(visible, PREVIEW_LENGTH, "预览长度不是 260 个字符");
        assert!(
            content.starts_with('…') && content.ends_with('…'),
            "{content:.20}"
        );
        assert!(!out.is_empty(), "高亮区间被裁没了");
        let content_len = content.encode_utf16().count();
        assert!((out[0].end as usize) <= content_len, "平移后的区间越界");
    }

    /// 命中靠近行尾时窗口回退到"显示末尾 260 字"，此时不该再有尾部省略号。
    #[test]
    fn preview_near_end_slides_window_back_without_trailing_ellipsis() {
        let line = "中".repeat(400);
        let spans = [SearchMatchSpan {
            start: 300,
            end: 301,
        }];
        let (content, out) = build_preview(&line, &spans);

        assert!(content.starts_with('…'));
        assert!(!content.ends_with('…'), "窗口已到行尾，不该有尾部省略号");
        assert_eq!(
            content.chars().filter(|c| *c == '中').count(),
            PREVIEW_LENGTH
        );
        assert!(!out.is_empty());
    }

    #[test]
    fn whole_word_is_skipped_for_cjk() {
        // 中文没有词边界，加 \b 会导致永远匹配不到
        let o = SearchOptions {
            whole_word: true,
            ..opts()
        };
        let p = parse_query("笔记", &o);
        let m = create_matcher(&p, &o).unwrap();
        assert!(m.combined.is_match("我的笔记本"), "中文全字匹配不该加 \\b");
    }

    #[test]
    fn whole_word_applies_to_ascii() {
        let o = SearchOptions {
            whole_word: true,
            ..opts()
        };
        let p = parse_query("cat", &o);
        let m = create_matcher(&p, &o).unwrap();
        assert!(m.combined.is_match("a cat here"));
        assert!(!m.combined.is_match("concatenate"), "\\b 没生效");
    }

    #[test]
    fn invalid_regex_reports_error_instead_of_panicking() {
        let ws = TempWs::new("badre");
        let svc = ws.svc();
        let r = svc.search(
            "(unclosed",
            &SearchOptions {
                use_regex: true,
                ..opts()
            },
        );
        assert_eq!(r.error.as_deref(), Some("正则表达式语法有误"));
        assert!(r.groups.is_empty());
    }

    #[test]
    fn empty_query_returns_nothing() {
        let ws = TempWs::new("emptyq");
        assert!(ws.svc().search("   ", &opts()).groups.is_empty());
    }

    #[test]
    fn finds_matches_and_reports_line_numbers() {
        let ws = TempWs::new("basic");
        ws.write("n.md", "第一行\n包含关键词的一行\n第三行\n");
        let r = ws.svc().search("关键词", &opts());

        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].matches[0].line, 2);
        assert_eq!(r.groups[0].match_count, 1);
        assert!(r.error.is_none());
    }

    /// 核心排序诉求：搜「快捷键」时文件名叫《快捷键》的必须排第一。
    #[test]
    fn filename_match_ranks_first() {
        let ws = TempWs::new("rank");
        ws.write("快捷键.md", "正文提到一次快捷键");
        ws.write("其他.md", "快捷键 快捷键 快捷键 快捷键");
        let r = ws.svc().search("快捷键", &opts());

        assert_eq!(r.groups.len(), 2);
        assert_eq!(r.groups[0].title, "快捷键", "文件名命中没有排到第一");
        assert!(r.groups[0].title_matched);
    }

    #[test]
    fn heading_lines_outrank_body_lines() {
        let ws = TempWs::new("heading");
        ws.write("n.md", "正文里的目标\n\n# 目标\n");
        let r = ws.svc().search("目标", &opts());
        assert_eq!(r.groups[0].matches[0].line, 3, "标题行应排在正文行前");
    }

    #[test]
    fn multi_term_requires_all_terms_in_one_file() {
        let ws = TempWs::new("andfile");
        ws.write("both.md", "甲在这里\n乙在那里\n");
        ws.write("one.md", "只有甲\n");
        let r = ws.svc().search("甲 乙", &opts());

        let titles: Vec<&str> = r.groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["both"], "缺一个词的文件不该入选");
    }

    #[test]
    fn case_sensitivity_is_honored() {
        let ws = TempWs::new("case");
        ws.write("n.md", "Hello hello\n");

        let insensitive = ws.svc().search("hello", &opts());
        assert_eq!(insensitive.groups[0].match_count, 2);

        let sensitive = ws.svc().search(
            "Hello",
            &SearchOptions {
                case_sensitive: true,
                ..opts()
            },
        );
        assert_eq!(sensitive.groups[0].match_count, 1);
    }

    #[test]
    fn regex_mode_matches() {
        let ws = TempWs::new("regex");
        ws.write("n.md", "abc123\nxyz\n");
        let r = ws.svc().search(
            r"[a-c]+\d+",
            &SearchOptions {
                use_regex: true,
                ..opts()
            },
        );
        assert_eq!(r.groups[0].matches[0].line, 1);
    }

    #[test]
    fn extension_filter_limits_candidates() {
        let ws = TempWs::new("extfilter");
        ws.write("a.md", "目标");
        ws.write("b.txt", "目标");
        let r = ws.svc().search(
            "目标",
            &SearchOptions {
                extensions: vec![".md".into()],
                ..opts()
            },
        );
        let titles: Vec<&str> = r.groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["a"]);
    }

    #[test]
    fn max_per_file_caps_lines_but_not_match_count() {
        let ws = TempWs::new("perfile");
        let body = (0..20)
            .map(|i| format!("第{i}行 目标"))
            .collect::<Vec<_>>()
            .join("\n");
        ws.write("n.md", &body);

        let r = ws.svc().search(
            "目标",
            &SearchOptions {
                max_per_file: 3,
                ..opts()
            },
        );
        assert_eq!(r.groups[0].matches.len(), 3, "行数应被截断");
        assert_eq!(r.groups[0].match_count, 20, "统计仍应是全量");
    }

    #[test]
    fn occurrence_counts_prior_hits_of_same_text() {
        let ws = TempWs::new("occ");
        ws.write("n.md", "目标\n目标\n目标\n");
        let r = ws.svc().search(
            "目标",
            &SearchOptions {
                max_per_file: 8,
                ..opts()
            },
        );

        let mut occ: Vec<u32> = r.groups[0].matches.iter().map(|m| m.occurrence).collect();
        occ.sort_unstable();
        assert_eq!(occ, [0, 1, 2], "同一文本的第 n 次出现应递增编号");
    }

    #[test]
    fn stats_report_duration_and_totals() {
        let ws = TempWs::new("stats");
        ws.write("a.md", "目标");
        ws.write("b.md", "目标 目标");
        let r = ws.svc().search("目标", &opts());
        assert_eq!(r.stats.total_files, 2);
        assert_eq!(r.stats.total_matches, 3);
        assert!(!r.stats.truncated);
        assert!(r.stats.duration_ms >= 0);
    }

    #[test]
    fn falls_back_to_disk_scan_when_index_is_empty() {
        // 不跑 run_full_index，索引里一条都没有——正则模式也拿不到 FTS 候选
        let n = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-search-fallback-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("n.md"), "磁盘兜底目标").unwrap();

        let idx = Arc::new(MetadataIndexService::new(&root));
        idx.open().unwrap();
        let svc = SearchService::new(idx.workspace_path(), Arc::clone(&idx));

        let r = svc.search("兜底", &opts());
        assert_eq!(r.groups.len(), 1, "索引为空时应回退磁盘扫描");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_rank_maps_bm25_into_zero_to_three() {
        assert_eq!(normalize_rank(0.0), 0.0);
        assert_eq!(normalize_rank(5.0), 0.0, "正的 bm25 无意义");
        assert_eq!(normalize_rank(-4.0), 1.0);
        assert_eq!(normalize_rank(-100.0), 3.0, "应被夹到 3");
        assert_eq!(normalize_rank(f64::NAN), 0.0);
    }

    #[test]
    fn recency_decays_over_time() {
        let now = 1_700_000_000_000;
        let fresh = score_recency(now, now);
        let week_old = score_recency(now - 7 * DAY_MS as i64, now);
        let year_old = score_recency(now - 365 * DAY_MS as i64, now);
        assert!(fresh > week_old && week_old > year_old);
        assert_eq!(score_recency(0, now), 0.0, "无 mtime 不参与打分");
    }
}
