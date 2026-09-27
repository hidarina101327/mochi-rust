//! 从工作区文档和链接中提取分析数据并构建关系图。
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::paths;
use crate::word_count::count_words;

/// 算作「笔记」的扩展名。图健康度、笔记趋势、总数都只看这些。
pub const NOTE_EXTENSIONS: &[&str] = &[".md", ".mc", ".mdx", ".txt", ".exam"];

/// 会被读进内存做文本分析的扩展名。比笔记集合大——代码/配置也参与字数统计。
pub const TEXT_EXTENSIONS: &[&str] = &[
    ".md", ".mc", ".mdx", ".txt", ".exam", ".json", ".js", ".jsx", ".ts", ".tsx", ".css", ".html",
    ".xml", ".yml", ".yaml",
];

/// 扫描时跳过的目录及其子树；点开头的目录由另一条规则排除。
pub const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".mochi",
    "dist",
    "dist-electron",
    "release",
];

/// 文本分析的文件大小上限为 2 MiB，限制首页统计的读取量和内存占用。
const MAX_TEXT_ANALYSIS_FILE_SIZE: u64 = 2 * 1024 * 1024;

/// 中心节点最多展示几个。
const MAX_CENTRAL_NODES: usize = 5;
/// 文件类型饼图最多展示几种。
const MAX_FILE_TYPES: usize = 10;

static WIKI_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]+\]\(([^)]+)\)").unwrap());

#[derive(Debug, Clone, PartialEq)]
pub struct ScannedFile {
    pub absolute_path: String,
    pub relative_path: String,
    /// 小写、带点；无扩展名时是 `none`。
    pub extension: String,
    pub size: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ScannedFile {
    pub fn is_note(&self) -> bool {
        NOTE_EXTENSIONS.contains(&self.extension.as_str())
    }
    pub fn is_text(&self) -> bool {
        TEXT_EXTENSIONS.contains(&self.extension.as_str())
    }
    /// 去掉扩展名的文件名，用作展示标题。
    pub fn title(&self) -> String {
        stem(&self.relative_path)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTypeStat {
    pub extension: String,
    pub count: usize,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeStat {
    pub path: String,
    pub title: String,
    pub score: i64,
    pub link_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphHealth {
    pub note_count: usize,
    pub local_link_count: i64,
    pub isolated_note_count: usize,
    pub untagged_note_ratio: f64,
    pub central_nodes: Vec<GraphNodeStat>,
}

impl Default for GraphHealth {
    fn default() -> Self {
        Self {
            note_count: 0,
            local_link_count: 0,
            isolated_note_count: 0,
            untagged_note_ratio: 0.0,
            central_nodes: Vec::new(),
        }
    }
}

/// 去掉最后一段扩展名。`a/b.tar.gz` → `b.tar`（与 `path.basename(p, path.extname(p))` 一致）。
pub fn stem(relative_path: &str) -> String {
    let name = relative_path.rsplit('/').next().unwrap_or(relative_path);
    match name.rfind('.') {
        // 前导点是隐藏文件而非扩展名分隔符
        Some(index) if index > 0 => name[..index].to_owned(),
        _ => name.to_owned(),
    }
}

fn extension_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(index) if index > 0 => format!(".{}", base[index + 1..].to_lowercase()),
        _ => "none".to_owned(),
    }
}

/// 递归扫描工作区。跳过点开头的条目与 `SKIPPED_DIRS`。
///
/// 读不到的目录/文件直接略过——扫描期间文件可能正被删除或重命名，
/// 为此让整张首页失败不划算。
pub fn scan_workspace(root: &Path) -> Vec<ScannedFile> {
    let mut out = Vec::new();
    scan_into(root, root, &mut out);
    out
}

fn scan_into(root: &Path, dir: &Path, out: &mut Vec<ScannedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut items: Vec<_> = entries.flatten().collect();
    // `read_dir` 顺序随文件系统而变；排序让扫描结果（进而是并列时的排名）可复现
    items.sort_by_key(std::fs::DirEntry::file_name);

    for entry in items {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();

        if file_type.is_dir() {
            if SKIPPED_DIRS.contains(&name.as_str()) {
                continue;
            }
            scan_into(root, &path, out);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };

        let modified = system_millis(meta.modified().ok());
        out.push(ScannedFile {
            absolute_path: paths::to_forward_slashes(&path.to_string_lossy()),
            relative_path: relative_to(root, &path),
            extension: extension_of(&name),
            size: meta.len(),
            // 拿不到创建时间的平台/文件系统上退回修改时间，别记 0——
            // 0 会让这篇笔记在「创建趋势」里落到 1970 年那一格
            created_at: system_millis(meta.created().ok())
                .filter(|ms| *ms > 0)
                .unwrap_or(modified.unwrap_or(0)),
            updated_at: modified.unwrap_or(0),
        });
    }
}

fn system_millis(time: Option<std::time::SystemTime>) -> Option<i64> {
    let time = time?;
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn relative_to(root: &Path, path: &Path) -> String {
    let root = paths::to_forward_slashes(&root.to_string_lossy());
    let full = paths::to_forward_slashes(&path.to_string_lossy());
    full.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .unwrap_or(&full)
        .to_owned()
}

/// 按数量降序的文件类型分布，最多 10 种。数量相同时按体积降序。
pub fn file_type_stats(files: &[ScannedFile]) -> Vec<FileTypeStat> {
    let mut by_extension: BTreeMap<&str, FileTypeStat> = BTreeMap::new();
    for file in files {
        let entry = by_extension
            .entry(&file.extension)
            .or_insert_with(|| FileTypeStat {
                extension: file.extension.clone(),
                count: 0,
                size: 0,
            });
        entry.count += 1;
        entry.size += file.size;
    }

    let mut stats: Vec<FileTypeStat> = by_extension.into_values().collect();
    stats.sort_by(|a, b| b.count.cmp(&a.count).then(b.size.cmp(&a.size)));
    stats.truncate(MAX_FILE_TYPES);
    stats
}

/// 从文档头部读取 `tags:`。支持 `[a, b]` 和以空格分隔两种写法。
pub fn parse_frontmatter_tags(content: &str) -> Vec<String> {
    if !content.starts_with("---") {
        return Vec::new();
    }
    let Some(end) = content[3..].find("\n---").map(|i| i + 3) else {
        return Vec::new();
    };
    let frontmatter = &content[3..end];

    let Some(line) = frontmatter.lines().find(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("tags") && trimmed[4..].trim_start().starts_with(':')
    }) else {
        return Vec::new();
    };

    let Some((_, value)) = line.split_once(':') else {
        return Vec::new();
    };
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }

    if value.starts_with('[') && value.ends_with(']') {
        return value[1..value.len() - 1]
            .split(',')
            .map(|tag| tag.trim().trim_matches(['\'', '"']).to_owned())
            .filter(|tag| !tag.is_empty())
            .collect();
    }
    value.split_whitespace().map(str::to_owned).collect()
}

/// 读 `.mochi/file-metadata.json` 里显式打的标签，键归一成工作区相对路径。
pub fn load_metadata_tags(root: &Path) -> HashMap<String, Vec<String>> {
    let Ok(raw) = std::fs::read_to_string(paths::mochi_dir(root).join("file-metadata.json")) else {
        return HashMap::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return HashMap::new();
    };
    let Some(files) = parsed.get("files").and_then(|v| v.as_object()) else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for (path, metadata) in files {
        let Some(relative) = crate::analytics::events::normalize_relative_path(root, Some(path))
        else {
            continue;
        };
        let Some(tags) = metadata.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        out.insert(
            relative,
            tags.iter()
                .filter_map(|t| t.as_str())
                .map(str::to_owned)
                .collect(),
        );
    }
    out
}

pub struct ContentAnalysis {
    pub graph: GraphHealth,
    pub total_words: usize,
}

#[derive(Default, Clone, Copy)]
struct LinkScore {
    inbound: i64,
    outbound: i64,
}

/// 双链图健康度 + 全库字数。
///
/// 两件事合在一次遍历里做，因为都要读文件正文——分开做等于把整个工作区读两遍。
pub fn build_graph_health(root: &Path, files: &[ScannedFile]) -> ContentAnalysis {
    let notes: Vec<&ScannedFile> = files.iter().filter(|f| f.is_note()).collect();
    let metadata_tags = load_metadata_tags(root);

    // 链接目标可以写标题也可以写路径，两种都要能查到。
    // 同名笔记会互相覆盖——这与上游的 `Map.set` 行为一致，链接指向后者。
    let mut by_title: HashMap<String, &ScannedFile> = HashMap::new();
    let mut scores: BTreeMap<&str, LinkScore> = BTreeMap::new();
    for note in &notes {
        by_title.insert(note.title().to_lowercase(), note);
        by_title.insert(note.relative_path.to_lowercase(), note);
        scores.insert(note.relative_path.as_str(), LinkScore::default());
    }

    let mut local_link_count = 0i64;
    let mut tagged_count = 0usize;
    let mut total_words = 0usize;

    for note in &notes {
        let has_metadata_tags = metadata_tags
            .get(&note.relative_path)
            .is_some_and(|t| !t.is_empty());
        if has_metadata_tags {
            tagged_count += 1;
        }

        if note.size > MAX_TEXT_ANALYSIS_FILE_SIZE || !note.is_text() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&note.absolute_path) else {
            continue;
        };

        total_words += count_words(&content);

        // 没有元数据标签时，再检查文档头部；两处内容不会重复计数。
        if !has_metadata_tags && !parse_frontmatter_tags(&content).is_empty() {
            tagged_count += 1;
        }

        let targets = WIKI_LINK
            .captures_iter(&content)
            .chain(MD_LINK.captures_iter(&content))
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_owned()));

        for raw in targets {
            // `[[笔记#小节]]` 指向的仍是那篇笔记
            let target = raw.split('#').next().unwrap_or("").trim();
            if target.is_empty() || is_external(target) {
                continue;
            }
            let normalized = paths::to_forward_slashes(target);
            let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);

            let hit = by_title
                .get(&normalized.to_lowercase())
                .or_else(|| by_title.get(&stem(normalized).to_lowercase()))
                .map(|f| f.relative_path.clone());
            let Some(target_path) = hit else { continue };

            local_link_count += 1;
            if let Some(score) = scores.get_mut(note.relative_path.as_str()) {
                score.outbound += 1;
            }
            if let Some(score) = scores.get_mut(target_path.as_str()) {
                score.inbound += 1;
            }
        }
    }

    // 被引用比引用别人更能说明「这是一篇枢纽笔记」，所以入链权重加倍
    let mut central: Vec<GraphNodeStat> = scores
        .iter()
        .map(|(path, score)| GraphNodeStat {
            path: (*path).to_owned(),
            title: stem(path),
            score: score.inbound * 2 + score.outbound,
            link_count: score.inbound + score.outbound,
        })
        .filter(|node| node.link_count > 0)
        .collect();
    central.sort_by_key(|b| std::cmp::Reverse(b.score));
    central.truncate(MAX_CENTRAL_NODES);

    let isolated = scores
        .values()
        .filter(|s| s.inbound + s.outbound == 0)
        .count();
    let untagged_ratio = if notes.is_empty() {
        0.0
    } else {
        (((notes.len() - tagged_count.min(notes.len())) as f64) / notes.len() as f64)
            .clamp(0.0, 1.0)
    };

    ContentAnalysis {
        graph: GraphHealth {
            note_count: notes.len(),
            local_link_count,
            isolated_note_count: isolated,
            untagged_note_ratio: untagged_ratio,
            central_nodes: central,
        },
        total_words,
    }
}

fn is_external(target: &str) -> bool {
    let lower = target.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Ws(PathBuf);
    impl Drop for Ws {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Ws {
        fn write(&self, relative: &str, body: &str) -> &Self {
            let target = self.0.join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, body).unwrap();
            self
        }
        fn scan(&self) -> Vec<ScannedFile> {
            scan_workspace(&self.0)
        }
    }

    fn ws(tag: &str) -> Ws {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-graph-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Ws(root)
    }

    // ---------- 扫描 ----------

    #[test]
    fn scanning_walks_subdirectories_and_records_metadata() {
        let w = ws("scan");
        w.write("知识库/数学/a.mc", "内容")
            .write("知识库/b.md", "内容更长一些");

        let files = w.scan();
        assert_eq!(files.len(), 2);
        let a = files
            .iter()
            .find(|f| f.relative_path == "知识库/数学/a.mc")
            .unwrap();
        assert_eq!(a.extension, ".mc");
        assert!(a.size > 0);
        assert!(
            a.updated_at > 0 && a.created_at > 0,
            "时间戳不能是 0，否则会落到 1970 那一格"
        );
        assert_eq!(a.title(), "a");
    }

    #[test]
    fn dot_entries_and_build_dirs_are_skipped() {
        let w = ws("skip");
        w.write("正常.mc", "x")
            .write(".mochi/index.db", "x")
            .write(".hidden/a.mc", "x")
            .write("node_modules/pkg/index.js", "x")
            .write("dist/bundle.js", "x")
            .write("release/app.exe", "x");

        let paths: Vec<String> = w.scan().into_iter().map(|f| f.relative_path).collect();
        assert_eq!(paths, ["正常.mc"], "只该扫到这一个: {paths:?}");
    }

    #[test]
    fn scan_order_is_stable() {
        let w = ws("order");
        for name in ["c.mc", "a.mc", "b.mc"] {
            w.write(name, "x");
        }
        let first: Vec<String> = w.scan().into_iter().map(|f| f.relative_path).collect();
        let second: Vec<String> = w.scan().into_iter().map(|f| f.relative_path).collect();
        assert_eq!(first, second);
        assert_eq!(first, ["a.mc", "b.mc", "c.mc"]);
    }

    #[test]
    fn extensions_are_lowercased_and_default_to_none() {
        assert_eq!(extension_of("a.MC"), ".mc");
        assert_eq!(extension_of("a.tar.gz"), ".gz");
        assert_eq!(extension_of("README"), "none");
        assert_eq!(
            extension_of(".gitignore"),
            "none",
            "前导点是隐藏文件，不是扩展名"
        );
    }

    #[test]
    fn stems_drop_only_the_last_extension() {
        assert_eq!(stem("知识库/数学/概率.mc"), "概率");
        assert_eq!(stem("a.tar.gz"), "a.tar");
        assert_eq!(stem("README"), "README");
        assert_eq!(stem(".gitignore"), ".gitignore");
    }

    // ---------- 文件类型 ----------

    #[test]
    fn file_types_rank_by_count_then_size() {
        let w = ws("types");
        w.write("a.mc", "1")
            .write("b.mc", "22")
            .write("c.png", "3333")
            .write("d.md", "4");

        let stats = file_type_stats(&w.scan());
        assert_eq!(stats[0].extension, ".mc");
        assert_eq!(stats[0].count, 2);
        assert_eq!(stats[0].size, 3);
        // .png 比 .md 大，数量相同时排前面
        assert_eq!(stats[1].extension, ".png");
        assert_eq!(stats[2].extension, ".md");
    }

    #[test]
    fn file_types_are_capped_at_ten() {
        let w = ws("types-cap");
        for i in 0..15 {
            w.write(&format!("f{i}.x{i}"), "x");
        }
        assert_eq!(file_type_stats(&w.scan()).len(), MAX_FILE_TYPES);
    }

    // ---------- 文档头部标签 ----------

    #[test]
    fn frontmatter_tags_support_both_syntaxes() {
        assert_eq!(
            parse_frontmatter_tags("---\ntags: [数学, 概率]\n---\n正文"),
            ["数学", "概率"]
        );
        assert_eq!(
            parse_frontmatter_tags("---\ntags: 数学 概率\n---\n正文"),
            ["数学", "概率"]
        );
        assert_eq!(
            parse_frontmatter_tags("---\ntitle: x\ntags: [\"a\", 'b']\n---\n"),
            ["a", "b"],
            "引号要剥掉"
        );
    }

    #[test]
    fn missing_or_malformed_frontmatter_yields_no_tags() {
        for raw in [
            "没有 frontmatter",
            "---\ntitle: x\n---\n",
            "---\ntags:\n---\n",
            "---\n没有结尾",
        ] {
            assert!(parse_frontmatter_tags(raw).is_empty(), "{raw:?}");
        }
    }

    // ---------- 图健康度 ----------

    #[test]
    fn wiki_and_markdown_links_both_build_the_graph() {
        let w = ws("links");
        w.write("枢纽.mc", "见 [[分支一]] 和 [分支二](分支二.mc)")
            .write("分支一.mc", "回看 [[枢纽]]")
            .write("分支二.mc", "无出链");

        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(analysis.graph.note_count, 3);
        assert_eq!(analysis.graph.local_link_count, 3, "两条出链 + 一条回链");
        assert_eq!(analysis.graph.isolated_note_count, 0);

        // 枢纽：入 1 出 2 = 1*2+2 = 4；分支一：入 1 出 1 = 3；分支二：入 1 = 2
        let hub = analysis
            .graph
            .central_nodes
            .iter()
            .find(|n| n.path == "枢纽.mc")
            .unwrap();
        assert_eq!(hub.score, 4);
        assert_eq!(hub.link_count, 3);
        assert_eq!(
            analysis.graph.central_nodes[0].path, "枢纽.mc",
            "按分数降序"
        );
    }

    /// 入链权重加倍——被引用比引用别人更能说明是枢纽。
    #[test]
    fn inbound_links_weigh_double() {
        let w = ws("weights");
        w.write("被引.mc", "无出链")
            .write("引用者.mc", "[[被引]] [[被引]]");

        let analysis = build_graph_health(&w.0, &w.scan());
        let cited = analysis
            .graph
            .central_nodes
            .iter()
            .find(|n| n.path == "被引.mc")
            .unwrap();
        let citing = analysis
            .graph
            .central_nodes
            .iter()
            .find(|n| n.path == "引用者.mc")
            .unwrap();
        assert_eq!(cited.score, 4, "入 2 → 4");
        assert_eq!(citing.score, 2, "出 2 → 2");
        assert_eq!(analysis.graph.central_nodes[0].path, "被引.mc");
    }

    #[test]
    fn external_links_and_anchors_are_handled() {
        let w = ws("external");
        w.write(
            "源.mc",
            "[外链](https://example.com) [[目标#某小节]] [也是外链](HTTP://EXAMPLE.COM)",
        )
        .write("目标.mc", "内容");

        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(
            analysis.graph.local_link_count, 1,
            "只有指向本地笔记的那条算"
        );
        let target = analysis
            .graph
            .central_nodes
            .iter()
            .find(|n| n.path == "目标.mc")
            .unwrap();
        assert_eq!(target.link_count, 1, "锚点后缀不影响目标解析");
    }

    #[test]
    fn links_resolve_by_title_or_by_relative_path() {
        let w = ws("resolve");
        w.write("知识库/数学/概率.mc", "内容").write(
            "源.mc",
            "[[概率]] 和 [[知识库/数学/概率.mc]] 和 [[./知识库/数学/概率.mc]]",
        );

        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(analysis.graph.local_link_count, 3, "三种写法都要能解析");
    }

    #[test]
    fn notes_with_no_links_are_counted_as_isolated() {
        let w = ws("isolated");
        w.write("孤儿一.mc", "没有链接")
            .write("孤儿二.mc", "也没有");

        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(analysis.graph.isolated_note_count, 2);
        assert!(
            analysis.graph.central_nodes.is_empty(),
            "没有链接就不该进中心节点榜"
        );
    }

    #[test]
    fn untagged_ratio_counts_both_metadata_and_frontmatter_tags() {
        let w = ws("tags");
        w.write("有标签.mc", "---\ntags: [数学]\n---\n正文")
            .write("没标签.mc", "正文")
            .write("也没标签.mc", "正文");

        let analysis = build_graph_health(&w.0, &w.scan());
        assert!(
            (analysis.graph.untagged_note_ratio - 2.0 / 3.0).abs() < 1e-9,
            "三篇里两篇没标签: {}",
            analysis.graph.untagged_note_ratio
        );
    }

    #[test]
    fn metadata_tags_are_read_from_the_mochi_directory() {
        let w = ws("meta-tags");
        w.write("甲.mc", "正文没有 frontmatter");
        let meta = serde_json::json!({ "files": { "甲.mc": { "tags": ["手动标签"] } } });
        w.write(".mochi/file-metadata.json", &meta.to_string());

        assert_eq!(
            load_metadata_tags(&w.0).get("甲.mc").unwrap(),
            &["手动标签"]
        );
        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(
            analysis.graph.untagged_note_ratio, 0.0,
            "元数据标签也算打过标签"
        );
    }

    #[test]
    fn total_words_sums_the_whole_workspace() {
        let w = ws("words");
        w.write("a.mc", "hello world").write("b.md", "你好世界");
        assert_eq!(build_graph_health(&w.0, &w.scan()).total_words, 6, "2 + 4");
    }

    /// 非笔记扩展名不进图，也不计入笔记数。
    #[test]
    fn non_note_files_stay_out_of_the_graph() {
        let w = ws("non-note");
        w.write("笔记.mc", "[[图片]]").write("图片.png", "二进制");

        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(analysis.graph.note_count, 1);
        assert_eq!(
            analysis.graph.local_link_count, 0,
            "指向非笔记的链接不算本地链接"
        );
    }

    #[test]
    fn an_empty_workspace_yields_a_zeroed_graph() {
        let w = ws("empty");
        let analysis = build_graph_health(&w.0, &w.scan());
        assert_eq!(analysis.graph, GraphHealth::default());
        assert_eq!(analysis.total_words, 0);
    }
}
