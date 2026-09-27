//! index.db 可删除重建；单连接串行访问，保持 better-sqlite3 的同步语义。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::note_parser::{self, MAX_INDEXABLE_FILE_SIZE};

/// 与旧版 `IGNORED_DIRS` 一致（`.git`/`.mochi` 等点目录另由前缀规则覆盖）。
const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".mochi",
    "dist",
    "dist-electron",
    "release",
    "build",
    "out",
    ".next",
    ".cache",
    "target",
    "vendor",
    "coverage",
];

// 解析语义变了就 +1。revision 2 特别在于：把选择器产出的独立 Mochi 对象
// URL 也纳入索引。这一次性失效让老库能发现那些链接，同时之后的每次启动
// 依旧走 mtime+size 的增量索引。
const NOTE_PARSER_REVISION: &str = "2";

/// 数据结构是硬性约定——三个版本共用同一个 `index.db`，任何 DDL 差异都会导致其他版本无法读取。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS notes (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  rel_path TEXT NOT NULL,
  title TEXT NOT NULL,
  extension TEXT NOT NULL,
  size INTEGER NOT NULL,
  mtime_ms INTEGER NOT NULL,
  indexed_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notes_rel ON notes(rel_path);
CREATE INDEX IF NOT EXISTS idx_notes_title ON notes(title);

CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
  title,
  body,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS links (
  source_id INTEGER NOT NULL,
  target_raw TEXT NOT NULL,
  target_key TEXT NOT NULL,
  target_id INTEGER,
  kind TEXT NOT NULL,
  heading TEXT,
  block_id TEXT,
  alias TEXT,
  line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_links_source ON links(source_id);
CREATE INDEX IF NOT EXISTS idx_links_target ON links(target_id);
CREATE INDEX IF NOT EXISTS idx_links_key ON links(target_key);

CREATE TABLE IF NOT EXISTS tags (
  note_id INTEGER NOT NULL,
  tag TEXT NOT NULL,
  tag_lower TEXT NOT NULL,
  source TEXT NOT NULL,
  line INTEGER
);
CREATE INDEX IF NOT EXISTS idx_tags_note ON tags(note_id);
CREATE INDEX IF NOT EXISTS idx_tags_lower ON tags(tag_lower);

CREATE TABLE IF NOT EXISTS properties (
  note_id INTEGER NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  value_type TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_props_note ON properties(note_id);
CREATE INDEX IF NOT EXISTS idx_props_key ON properties(key);

CREATE TABLE IF NOT EXISTS headings (
  note_id INTEGER NOT NULL,
  level INTEGER NOT NULL,
  text TEXT NOT NULL,
  line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_headings_note ON headings(note_id);

CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct FtsCandidate {
    pub path: String,
    pub title: String,
    pub rel_path: String,
    pub mtime_ms: i64,
    pub rank: f64,
}

/// 一条已索引的笔记链接。字段与 Electron `NoteLink` 保持一致，供反向链接面板复用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLink {
    pub source_path: PathBuf,
    pub source_title: String,
    pub target_raw: String,
    pub target_path: Option<PathBuf>,
    pub kind: String,
    pub heading: Option<String>,
    pub block_id: Option<String>,
    pub alias: Option<String>,
    pub line: usize,
    pub context: String,
}

/// 正文提到了目标标题、但没有建立链接的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlinkedMention {
    pub source_path: PathBuf,
    pub source_title: String,
    pub line: usize,
    pub context: String,
}

pub struct MetadataIndexService {
    workspace_path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

/// 后台全量索引的进度快照。`processed` 是已扫描的候选文件数，`indexed` 是本轮
/// 实际写入 SQLite 的文件数；未变化的文件同样会计入 `processed`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FullIndexProgress {
    pub processed: usize,
    pub total: usize,
    pub indexed: usize,
}

impl MetadataIndexService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        let workspace_path = workspace_path
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| workspace_path.as_ref().to_path_buf());
        Self {
            workspace_path: strip_unc_prefix(workspace_path),
            conn: Mutex::new(None),
        }
    }

    pub fn workspace_path(&self) -> &Path {
        &self.workspace_path
    }

    // ---------- 生命周期 ----------

    pub fn open(&self) -> Result<()> {
        let mut slot = self.lock();
        if slot.is_some() {
            return Ok(());
        }
        let mochi_dir = self.workspace_path.join(".mochi");
        std::fs::create_dir_all(&mochi_dir)?;

        let conn = Connection::open(mochi_dir.join("index.db"))
            .with_context(|| format!("打开索引失败: {}", mochi_dir.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)
            .context("建表失败（FTS5 可用？）")?;

        *slot = Some(conn);
        Ok(())
    }

    pub fn close(&self) {
        *self.lock() = None;
    }

    fn lock(&self) -> MutexGuard<'_, Option<Connection>> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- 索引写入 ----------

    /// 全量索引（幂等，mtime+size 未变的跳过）。返回实际更新的文件数。
    pub fn run_full_index(&self) -> Result<usize> {
        self.run_full_index_with_progress(|_| {})
    }

    /// 带进度回调的全量索引。回调在调用方线程执行，适合后台任务将进度投递给 UI；
    /// 不在 UI 线程中调用这个方法。
    pub fn run_full_index_with_progress(
        &self,
        mut on_progress: impl FnMut(FullIndexProgress),
    ) -> Result<usize> {
        let mut slot = self.lock();
        let conn = slot.as_mut().context("索引未打开：先调用 open()")?;
        let tx = conn.transaction()?;

        let parser_revision: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = 'note_parser_revision'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let parser_changed = parser_revision.as_deref() != Some(NOTE_PARSER_REVISION);

        let mut known: HashMap<String, (i64, i64, i64)> = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT id, path, mtime_ms, size FROM notes")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    (
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ),
                ))
            })?;
            for row in rows {
                let (path, entry) = row?;
                known.insert(path, entry);
            }
        }

        let files = collect_indexable_files(&self.workspace_path);
        let total = files.len();
        let mut count = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        on_progress(FullIndexProgress {
            processed: 0,
            total,
            indexed: 0,
        });
        for (offset, file_path) in files.into_iter().enumerate() {
            let key = file_path.to_string_lossy().into_owned();
            seen.insert(key.to_lowercase());

            if let Ok(meta) = std::fs::metadata(&file_path) {
                if meta.len() > MAX_INDEXABLE_FILE_SIZE {
                    remove_note_in(&tx, &file_path)?;
                } else {
                    let mtime_ms = mtime_millis(&meta);
                    let unchanged = !parser_changed
                        && known.get(&key).is_some_and(|(_, prev_mtime, prev_size)| {
                            *prev_mtime == mtime_ms && *prev_size == meta.len() as i64
                        });
                    if !unchanged {
                        if let Ok(content) = std::fs::read_to_string(&file_path) {
                            self.write_note(
                                &tx,
                                &file_path,
                                &content,
                                meta.len() as i64,
                                mtime_ms,
                            )?;
                            count += 1;
                        }
                    }
                }
            }

            let processed = offset + 1;
            if processed == total || processed % 20 == 0 {
                on_progress(FullIndexProgress {
                    processed,
                    total,
                    indexed: count,
                });
            }
        }

        for (path, (id, _, _)) in &known {
            if seen.contains(&path.to_lowercase()) {
                continue;
            }
            clear_note_relations(&tx, *id)?;
            tx.execute("DELETE FROM notes WHERE id = ?1", [id])?;
        }

        resolve_links_in(&tx)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('indexed_at', ?1)",
            [crate::jstime::now_millis().to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('note_parser_revision', ?1)",
            [NOTE_PARSER_REVISION],
        )?;
        tx.commit()?;
        on_progress(FullIndexProgress {
            processed: total,
            total,
            indexed: count,
        });
        Ok(count)
    }

    /// 增量索引单文件（watcher 回调）。文件不存在时转为移除。
    pub fn index_single_file(&self, file_path: impl AsRef<Path>) -> Result<bool> {
        Ok(self.index_files(std::iter::once(file_path.as_ref()))? > 0)
    }

    /// watcher 的每批改动共享一个事务、一次链接解析。文件缺失就清掉过期条目；
    /// 偶发的读取失败则保留原样。
    pub fn index_files<I, P>(&self, paths: I) -> Result<usize>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .map(|p| self.resolve_query_path(p.as_ref()))
            .filter(|p| {
                note_parser::indexable_extensions().contains(extension_of(p).as_str())
                    && self.is_inside_workspace(p)
            })
            .collect();
        if paths.is_empty() {
            return Ok(0);
        }
        let mut slot = self.lock();
        let conn = slot.as_mut().context("索引未打开：先调用 open()")?;
        let tx = conn.transaction()?;
        let mut changed = 0;
        let mut seen = HashSet::new();
        for file_path in paths {
            if !seen.insert(file_path.to_string_lossy().to_lowercase()) {
                continue;
            }
            let meta = match std::fs::metadata(&file_path) {
                Ok(meta) => meta,
                Err(error) => {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        changed += usize::from(remove_note_in(&tx, &file_path)?);
                    }
                    continue;
                }
            };
            if meta.len() > MAX_INDEXABLE_FILE_SIZE {
                changed += usize::from(remove_note_in(&tx, &file_path)?);
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&file_path) else {
                continue;
            };
            self.write_note(
                &tx,
                &file_path,
                &content,
                meta.len() as i64,
                mtime_millis(&meta),
            )?;
            changed += 1;
        }
        if changed > 0 {
            resolve_links_in(&tx)?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn remove_from_index(&self, file_path: impl AsRef<Path>) -> Result<bool> {
        let path = self.resolve_query_path(file_path.as_ref());
        let mut slot = self.lock();
        let conn = slot.as_mut().context("索引未打开：先调用 open()")?;
        remove_from_index_in(conn, &path)
    }

    fn write_note(
        &self,
        conn: &Connection,
        file_path: &Path,
        content: &str,
        size: i64,
        mtime_ms: i64,
    ) -> Result<()> {
        let path_str = file_path.to_string_lossy().into_owned();
        let rel_path = crate::paths::to_forward_slashes(
            &file_path
                .strip_prefix(&self.workspace_path)
                .unwrap_or(file_path)
                .to_string_lossy(),
        );
        let extension = extension_of(file_path);
        let parsed = note_parser::parse_note(&path_str, content);
        let now = crate::jstime::now_millis();

        let existing: Option<i64> = conn
            .query_row("SELECT id FROM notes WHERE path = ?1", [&path_str], |r| {
                r.get(0)
            })
            .optional()?;

        let note_id = match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE notes SET rel_path = ?1, title = ?2, extension = ?3,
                                      size = ?4, mtime_ms = ?5, indexed_at = ?6
                     WHERE id = ?7",
                    rusqlite::params![rel_path, parsed.title, extension, size, mtime_ms, now, id],
                )?;
                clear_note_relations(conn, id)?;
                id
            }
            None => {
                conn.execute(
                    "INSERT INTO notes (path, rel_path, title, extension, size, mtime_ms, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        path_str, rel_path, parsed.title, extension, size, mtime_ms, now
                    ],
                )?;
                conn.last_insert_rowid()
            }
        };

        conn.execute(
            "INSERT INTO notes_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                note_id,
                note_parser::segment_cjk(&parsed.title),
                note_parser::segment_cjk(&parsed.body)
            ],
        )?;

        for link in &parsed.links {
            conn.execute(
                "INSERT INTO links (source_id, target_raw, target_key, target_id, kind, heading, block_id, alias, line)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    note_id, link.target_raw, link.target_key, link.kind,
                    link.heading, link.block_id, link.alias, link.line
                ],
            )?;
        }

        let mut seen_tags: HashSet<String> = HashSet::new();
        for tag in &parsed.tags {
            let lower = tag.tag.to_lowercase();
            let dedupe_key = format!(
                "{lower}:{}",
                tag.line
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "fm".into())
            );
            if !seen_tags.insert(dedupe_key) {
                continue;
            }
            conn.execute(
                "INSERT INTO tags (note_id, tag, tag_lower, source, line) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![note_id, tag.tag, lower, tag.source, tag.line],
            )?;
        }

        for property in &parsed.properties {
            conn.execute(
                "INSERT INTO properties (note_id, key, value, value_type) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![note_id, property.key, property.value, property.value_type],
            )?;
        }

        for heading in &parsed.headings {
            conn.execute(
                "INSERT INTO headings (note_id, level, text, line) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![note_id, heading.level, heading.text, heading.line],
            )?;
        }
        Ok(())
    }

    /// `links.target_id` 补齐：完整相对路径 → 去扩展名 → 文件名 → 标题 逐级回退。
    pub fn resolve_links(&self) -> Result<()> {
        let mut slot = self.lock();
        let conn = slot.as_mut().context("索引未打开：先调用 open()")?;
        let tx = conn.transaction()?;
        resolve_links_in(&tx)?;
        tx.commit()?;
        Ok(())
    }

    // ---------- 笔记关系查询 ----------

    /// 其他笔记中指向 `target_path` 的链接；同一篇笔记内的自链接不计入。
    pub fn get_backlinks(&self, target_path: impl AsRef<Path>) -> Result<Vec<NoteLink>> {
        self.query_links_to(target_path.as_ref(), false)
    }

    /// 返回所有指向 `target_path` 的链接，包括目标笔记自身的自引用。
    ///
    /// 普通反向链接面板仍使用 [`Self::get_backlinks`] 排除自链；文件改名时
    /// 则必须同时改写自身正文中的链接，因此提供这个单独的查询契约。
    pub fn get_links_to(&self, target_path: impl AsRef<Path>) -> Result<Vec<NoteLink>> {
        self.query_links_to(target_path.as_ref(), true)
    }

    fn query_links_to(&self, target_path: &Path, include_self: bool) -> Result<Vec<NoteLink>> {
        let resolved = self.resolve_query_path(target_path);
        let resolved_text = resolved.to_string_lossy().into_owned();
        let rows = {
            let slot = self.lock();
            let conn = slot.as_ref().context("索引未打开：先调用 open()")?;
            let Some(target_id) = conn
                .query_row(
                    "SELECT id FROM notes WHERE path = ?1",
                    [&resolved_text],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
            else {
                return Ok(Vec::new());
            };

            let sql = if include_self {
                "SELECT n.path, n.title, l.target_raw, l.kind, l.heading, l.block_id, l.alias, l.line
                 FROM links l
                 JOIN notes n ON n.id = l.source_id
                 WHERE l.target_id = ?1
                 ORDER BY n.rel_path, l.line"
            } else {
                "SELECT n.path, n.title, l.target_raw, l.kind, l.heading, l.block_id, l.alias, l.line
                 FROM links l
                 JOIN notes n ON n.id = l.source_id
                 WHERE l.target_id = ?1 AND l.source_id != ?1
                 ORDER BY n.rel_path, l.line"
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt
                .query_map([target_id], |r| {
                    Ok(NoteLink {
                        source_path: PathBuf::from(r.get::<_, String>(0)?),
                        source_title: r.get(1)?,
                        target_raw: r.get(2)?,
                        target_path: Some(resolved.clone()),
                        kind: r.get(3)?,
                        heading: r.get(4)?,
                        block_id: r.get(5)?,
                        alias: r.get(6)?,
                        line: r.get::<_, i64>(7)?.max(0) as usize,
                        context: String::new(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };

        Ok(rows
            .into_iter()
            .map(|mut link| {
                link.context = read_context(&link.source_path, link.line);
                link
            })
            .collect())
    }

    /// `source_path` 中的全部链接，包括目标尚未创建的悬空链接。
    pub fn get_outgoing_links(&self, source_path: impl AsRef<Path>) -> Result<Vec<NoteLink>> {
        let resolved = self.resolve_query_path(source_path.as_ref());
        let resolved_text = resolved.to_string_lossy().into_owned();
        let slot = self.lock();
        let conn = slot.as_ref().context("索引未打开：先调用 open()")?;
        let Some((source_id, source_title)) = conn
            .query_row(
                "SELECT id, title FROM notes WHERE path = ?1",
                [&resolved_text],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        else {
            return Ok(Vec::new());
        };

        let mut stmt = conn.prepare(
            "SELECT l.target_raw, l.kind, l.heading, l.block_id, l.alias, l.line, t.path
             FROM links l
             LEFT JOIN notes t ON t.id = l.target_id
             WHERE l.source_id = ?1
             ORDER BY l.line",
        )?;
        let rows = stmt
            .query_map([source_id], |r| {
                Ok(NoteLink {
                    source_path: resolved.clone(),
                    source_title: source_title.clone(),
                    target_raw: r.get(0)?,
                    target_path: r.get::<_, Option<String>>(6)?.map(PathBuf::from),
                    kind: r.get(1)?,
                    heading: r.get(2)?,
                    block_id: r.get(3)?,
                    alias: r.get(4)?,
                    line: r.get::<_, i64>(5)?.max(0) as usize,
                    context: String::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 正文里出现了目标笔记标题、但没有建立链接的位置。每篇来源笔记最多返回一条。
    pub fn get_unlinked_mentions(
        &self,
        target_path: impl AsRef<Path>,
    ) -> Result<Vec<UnlinkedMention>> {
        let resolved = self.resolve_query_path(target_path.as_ref());
        let resolved_text = resolved.to_string_lossy().into_owned();
        let (target_id, target_title, linked_sources, candidates) = {
            let slot = self.lock();
            let conn = slot.as_ref().context("索引未打开：先调用 open()")?;
            let Some((target_id, target_title)) = conn
                .query_row(
                    "SELECT id, title FROM notes WHERE path = ?1",
                    [&resolved_text],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?
            else {
                return Ok(Vec::new());
            };
            if target_title.encode_utf16().count() < 2 {
                return Ok(Vec::new());
            }

            let linked_sources = {
                let mut stmt =
                    conn.prepare("SELECT DISTINCT source_id FROM links WHERE target_id = ?1")?;
                let sources = stmt
                    .query_map([target_id], |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<HashSet<_>>>()?;
                sources
            };
            let fts_query = note_parser::build_fts_query_from_terms([target_title.as_str()]);
            if fts_query.is_empty() {
                return Ok(Vec::new());
            }
            let candidates = {
                let mut stmt = conn.prepare(
                    "SELECT n.id, n.path, n.title
                     FROM notes_fts
                     JOIN notes n ON n.id = notes_fts.rowid
                     WHERE notes_fts MATCH ?1
                     LIMIT 200",
                )?;
                let rows = stmt
                    .query_map([fts_query], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            PathBuf::from(r.get::<_, String>(1)?),
                            r.get::<_, String>(2)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            (target_id, target_title, linked_sources, candidates)
        };

        let needle = target_title.to_lowercase();
        let wiki_prefix = format!("[[{target_title}");
        let mut mentions = Vec::new();
        for (candidate_id, source_path, source_title) in candidates {
            if candidate_id == target_id || linked_sources.contains(&candidate_id) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&source_path) else {
                continue;
            };
            if let Some((index, line)) = content.lines().enumerate().find(|(_, line)| {
                line.to_lowercase().contains(&needle)
                    && !line.contains(&wiki_prefix)
                    && !line.contains("](")
            }) {
                mentions.push(UnlinkedMention {
                    source_path,
                    source_title,
                    line: index + 1,
                    context: trim_to_chars(line, 240),
                });
            }
        }
        Ok(mentions)
    }

    // ---------- FTS 候选查询 ----------

    /// 用 FTS 缩小候选集并带出 bm25。返回 `None` 表示索引不可用或查询无法转成
    /// FTS 表达式——调用方回退全量扫描。
    pub fn find_fts_candidates(&self, terms: &[String], limit: usize) -> Option<Vec<FtsCandidate>> {
        let slot = self.lock();
        let conn = slot.as_ref()?;
        let fts_query = note_parser::build_fts_query_from_terms(terms.iter().map(String::as_str));
        if fts_query.is_empty() {
            return None;
        }
        let candidate_limit = (limit * 6).max(200);

        let mut stmt = conn
            .prepare(
                "SELECT n.path, n.title, n.rel_path, n.mtime_ms, bm25(notes_fts, 4.0, 1.0)
                 FROM notes_fts
                 JOIN notes n ON n.id = notes_fts.rowid
                 WHERE notes_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .ok()?;
        let rows = stmt
            .query_map(rusqlite::params![fts_query, candidate_limit as i64], |r| {
                Ok(FtsCandidate {
                    path: r.get(0)?,
                    title: r.get(1)?,
                    rel_path: r.get(2)?,
                    mtime_ms: r.get(3)?,
                    rank: r.get(4)?,
                })
            })
            // 查询语法异常 → 调用方回退全量扫描
            .ok()?;
        rows.collect::<rusqlite::Result<Vec<_>>>().ok()
    }

    /// 状态栏只需要计数，不必克隆整库路径、标题及元数据。
    pub fn indexed_file_count(&self) -> usize {
        let slot = self.lock();
        slot.as_ref()
            .and_then(|conn| {
                conn.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))
                    .ok()
            })
            .unwrap_or(0)
            .max(0) as usize
    }

    /// 全部已索引文件（FTS 不可用时全量扫描的清单）。
    pub fn search_notes_by_title(&self, query: &str, limit: usize) -> Vec<FtsCandidate> {
        let slot = self.lock();
        let Some(conn) = slot.as_ref() else {
            return Vec::new();
        };
        let Ok(mut stmt) = conn.prepare("SELECT path,title,rel_path,mtime_ms FROM notes WHERE extension IN ('.md','.markdown','.mdx') AND instr(lower(title),lower(?1)) > 0 ORDER BY lower(title)=lower(?1) DESC, mtime_ms DESC, rel_path LIMIT ?2") else { return Vec::new() };
        let rows = stmt.query_map(rusqlite::params![query.trim(), limit.min(50) as i64], |r| {
            Ok(FtsCandidate {
                path: r.get(0)?,
                title: r.get(1)?,
                rel_path: r.get(2)?,
                mtime_ms: r.get(3)?,
                rank: 0.0,
            })
        });
        match rows {
            Ok(rows) => rows.filter_map(std::result::Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 全部已索引文件（FTS 不可用时全量扫描的清单）。
    pub fn list_indexed_files(&self) -> Vec<FtsCandidate> {
        let slot = self.lock();
        let Some(conn) = slot.as_ref() else {
            return Vec::new();
        };
        let Ok(mut stmt) = conn.prepare("SELECT path, title, rel_path, mtime_ms FROM notes") else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |r| {
            Ok(FtsCandidate {
                path: r.get(0)?,
                title: r.get(1)?,
                rel_path: r.get(2)?,
                mtime_ms: r.get(3)?,
                rank: 0.0,
            })
        });
        match rows {
            Ok(rows) => rows.filter_map(std::result::Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn is_inside_workspace(&self, file_path: &Path) -> bool {
        let full = file_path
            .canonicalize()
            .map(strip_unc_prefix)
            .unwrap_or_else(|_| file_path.to_path_buf());
        full.starts_with(&self.workspace_path)
    }

    fn resolve_query_path(&self, file_path: &Path) -> PathBuf {
        let full = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            self.workspace_path.join(file_path)
        };
        full.canonicalize()
            .map(strip_unc_prefix)
            .unwrap_or_else(|_| full.components().collect())
    }
}

// ---------- 自由函数 ----------

fn remove_from_index_in(conn: &mut Connection, file_path: &Path) -> Result<bool> {
    let tx = conn.transaction()?;
    let removed = remove_note_in(&tx, file_path)?;
    tx.commit()?;
    Ok(removed)
}

fn remove_note_in(conn: &Connection, file_path: &Path) -> Result<bool> {
    let path_str = file_path.to_string_lossy().into_owned();
    let id: Option<i64> = conn
        .query_row("SELECT id FROM notes WHERE path = ?1", [&path_str], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(id) = id else { return Ok(false) };

    clear_note_relations(conn, id)?;
    conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
    conn.execute(
        "UPDATE links SET target_id = NULL WHERE target_id = ?1",
        [id],
    )?;
    Ok(true)
}

fn clear_note_relations(conn: &Connection, note_id: i64) -> Result<()> {
    for sql in [
        "DELETE FROM links WHERE source_id = ?1",
        "DELETE FROM tags WHERE note_id = ?1",
        "DELETE FROM properties WHERE note_id = ?1",
        "DELETE FROM headings WHERE note_id = ?1",
        "DELETE FROM notes_fts WHERE rowid = ?1",
    ] {
        conn.execute(sql, [note_id])?;
    }
    Ok(())
}

fn resolve_links_in(conn: &Connection) -> Result<()> {
    let mut by_key: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, rel_path, title FROM notes")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, rel, title) = row?;
            let rel_lower = rel.to_lowercase();
            let base_name = file_name_of(&rel_lower);
            // 先到先得：与 C# 的 TryAdd 语义一致，不覆盖已有键
            for key in [
                rel_lower.clone(),
                remove_extension(&rel_lower),
                base_name,
                title.to_lowercase(),
            ] {
                by_key.entry(key).or_insert(id);
            }
        }
    }

    let updates: Vec<(i64, String, Option<i64>)> = {
        let mut stmt =
            conn.prepare("SELECT rowid, target_raw, target_key, target_id FROM links")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (row_id, raw, key, previous) = row?;
            // 更老的索引库把完整 Mochi URL 存进了 target_key。
            // 从 target_raw 推导出可搬移的文件系统键，增量索引/重命名
            // 就能顺手修好那些行，不用全量扫描。
            let effective_key =
                crate::note_parser::object_link_target_key(&raw).unwrap_or_else(|| key.clone());
            let resolved = by_key
                .get(&effective_key)
                .or_else(|| by_key.get(&remove_extension(&effective_key)))
                .or_else(|| by_key.get(&file_name_of(&effective_key)))
                .or_else(|| by_key.get(&remove_extension(&file_name_of(&effective_key))))
                .copied();
            if key != effective_key || previous != resolved {
                out.push((row_id, effective_key, resolved));
            }
        }
        out
    };

    let mut stmt =
        conn.prepare("UPDATE links SET target_key = ?1, target_id = ?2 WHERE rowid = ?3")?;
    for (row_id, key, target) in updates {
        stmt.execute(rusqlite::params![key, target, row_id])?;
    }
    Ok(())
}

fn read_context(file_path: &Path, line: usize) -> String {
    if line == 0 {
        return String::new();
    }
    std::fs::read_to_string(file_path)
        .ok()
        .and_then(|content| {
            content
                .lines()
                .nth(line - 1)
                .map(|line| trim_to_chars(line, 240))
        })
        .unwrap_or_default()
}

fn trim_to_chars(value: &str, max_chars: usize) -> String {
    // Electron slice(0, 240) 的单位为 UTF-16；Rust 字符串不能保留半个代理对。
    let mut units = 0;
    value
        .trim()
        .chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= max_chars
        })
        .collect()
}

/// 去扩展名，但只在最后一个 `/` 之后找 `.`——避免把 `dir.v2/note` 砍成 `dir`。
fn remove_extension(path: &str) -> String {
    let slash = path.rfind('/').map(|i| i as isize).unwrap_or(-1);
    match path.rfind('.') {
        Some(dot) if (dot as isize) > slash + 1 => path[..dot].to_owned(),
        _ => path.to_owned(),
    }
}

fn file_name_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[i + 1..].to_owned(),
        None => path.to_owned(),
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

fn mtime_millis(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn collect_indexable_files(root: &Path) -> Vec<PathBuf> {
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
        if IGNORED_DIRS.contains(&name.as_str()) || name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(t)
                if t.is_file()
                    && note_parser::indexable_extensions()
                        .contains(extension_of(&path).as_str()) =>
            {
                out.push(path);
            }
            _ => {}
        }
    }
}

fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

/// 供外部（如 `SearchService`）复用的忽略清单判定。
pub fn is_ignored_dir_name(name: &str) -> bool {
    IGNORED_DIRS.contains(&name) || name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("mochi-idx-{}-{tag}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d).unwrap();
            }
            fs::write(&p, content).unwrap();
            p
        }
        fn svc(&self) -> MetadataIndexService {
            let s = MetadataIndexService::new(&self.0);
            s.open().unwrap();
            s
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// FTS5 必须可用——bundled SQLite 若未启用该模块，整个搜索都塌了。
    #[test]
    fn fts5_is_available() {
        let ws = TempWs::new("fts5");
        let svc = ws.svc();
        let slot = svc.lock();
        let conn = slot.as_ref().unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM notes_fts", [], |r| r.get(0))
            .expect("notes_fts 不可查询——FTS5 没编进来");
        assert_eq!(n, 0);
    }

    /// 三版共用同一套数据结构；改动 DDL 后，另外两个版本就可能无法读取。
    #[test]
    fn schema_tables_and_tokenizer_match_contract() {
        let ws = TempWs::new("schema");
        let svc = ws.svc();
        let slot = svc.lock();
        let conn = slot.as_ref().unwrap();

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|t| !t.starts_with("notes_fts_") && !t.starts_with("sqlite_"))
            .collect();
        assert_eq!(
            tables,
            [
                "headings",
                "links",
                "meta",
                "notes",
                "notes_fts",
                "properties",
                "tags"
            ]
        );

        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='notes_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            ddl.contains("unicode61 remove_diacritics 2"),
            "分词器配置变了: {ddl}"
        );
    }

    #[test]
    fn full_index_counts_and_skips_unchanged() {
        let ws = TempWs::new("full");
        ws.write("a.md", "# 甲\n内容");
        ws.write("知识库/b.md", "# 乙\n内容");
        ws.write("node_modules/skip.md", "不该被索引");
        ws.write("image.png", "binary");

        let svc = ws.svc();
        assert_eq!(svc.run_full_index().unwrap(), 2, "只应索引两个 .md");
        assert_eq!(svc.run_full_index().unwrap(), 0, "第二次应全部跳过");
        assert_eq!(svc.list_indexed_files().len(), 2);
    }

    #[test]
    fn full_index_reports_progress_from_zero_to_total() {
        let ws = TempWs::new("progress");
        ws.write("a.md", "甲");
        ws.write("nested/b.md", "乙");
        let svc = ws.svc();
        let mut reports = Vec::new();
        assert_eq!(
            svc.run_full_index_with_progress(|progress| reports.push(progress))
                .unwrap(),
            2
        );
        assert_eq!(reports.first().unwrap().processed, 0);
        assert_eq!(reports.first().unwrap().total, 2);
        let last = reports.last().unwrap();
        assert_eq!((last.processed, last.total, last.indexed), (2, 2, 2));
    }

    #[test]
    fn batch_updates_resolve_new_and_deleted_targets_and_deduplicate_paths() {
        let ws = TempWs::new("batch-links");
        let source = ws.write("source.md", "[[target]]");
        let svc = ws.svc();
        svc.index_single_file(&source).unwrap();
        let target = ws.write("target.md", "# target\ncreated");
        assert_eq!(svc.index_files([&source, &target, &source]).unwrap(), 2);
        assert_eq!(svc.get_backlinks(&target).unwrap().len(), 1);
        fs::remove_file(&target).unwrap();
        assert_eq!(svc.index_files([&target]).unwrap(), 1);
        assert_eq!(svc.list_indexed_files().len(), 1);
        let slot = svc.lock();
        let remaining: i64 = slot
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM links WHERE target_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn json_forward_slash_paths_and_relative_paths_share_the_canonical_index_key() {
        let ws = TempWs::new("json-library-path");
        let target = ws.write("库/目标.md", "# 目标");
        let source = ws.write("库/引用.md", "[[目标]]");
        let svc = ws.svc();
        let target_json = target.to_string_lossy().replace('\\', "/");
        let source_json = source.to_string_lossy().replace('\\', "/");
        assert_eq!(
            svc.index_files([target_json.as_str(), source_json.as_str(), "库/引用.md"])
                .unwrap(),
            2
        );
        assert_eq!(svc.get_backlinks(&target).unwrap().len(), 1);
        assert_eq!(svc.get_backlinks(Path::new(&target_json)).unwrap().len(), 1);
        assert_eq!(svc.indexed_file_count(), 2);
        fs::remove_file(&source).unwrap();
        assert!(svc.remove_from_index(Path::new(&source_json)).unwrap());
        assert!(svc.get_backlinks(&target).unwrap().is_empty());
    }

    #[test]
    fn oversized_file_clears_stale_index_in_full_and_incremental_passes() {
        for full in [true, false] {
            let ws = TempWs::new(if full {
                "oversized-full"
            } else {
                "oversized-single"
            });
            let file = ws.write("growing.md", "old indexed text");
            let svc = ws.svc();
            svc.run_full_index().unwrap();
            let handle = fs::OpenOptions::new().write(true).open(&file).unwrap();
            handle.set_len(MAX_INDEXABLE_FILE_SIZE + 1).unwrap();
            drop(handle);
            if full {
                svc.run_full_index().unwrap();
            } else {
                assert!(svc.index_single_file(&file).unwrap());
            }
            assert!(svc.list_indexed_files().is_empty());
            assert!(svc
                .find_fts_candidates(&["indexed".into()], 10)
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn unchanged_link_resolution_does_not_rewrite_rows() {
        let ws = TempWs::new("stable-link-rows");
        ws.write("source.md", "[[target]]");
        ws.write("target.md", "# target");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        {
            let slot = svc.lock();
            slot.as_ref().unwrap().execute_batch("CREATE TEMP TRIGGER deny_link_update BEFORE UPDATE ON links BEGIN SELECT RAISE(ABORT, 'unexpected rewrite'); END;").unwrap();
        }
        svc.resolve_links().unwrap();
        assert_eq!(svc.run_full_index().unwrap(), 0);
    }

    #[test]
    fn failed_batch_rolls_back_all_notes_and_relations() {
        let ws = TempWs::new("batch-rollback");
        let first = ws.write("first.md", "original");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        {
            let slot = svc.lock();
            slot.as_ref().unwrap().execute_batch("CREATE TEMP TRIGGER fail_new_note BEFORE INSERT ON notes BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;").unwrap();
        }
        fs::write(&first, "changed").unwrap();
        let second = ws.write("second.md", "new content");
        assert!(svc.index_files([&first, &second]).is_err());
        assert_eq!(svc.indexed_file_count(), 1);
        assert_eq!(
            svc.find_fts_candidates(&["original".into()], 10)
                .unwrap()
                .len(),
            1
        );
        assert!(svc
            .find_fts_candidates(&["changed".into()], 10)
            .unwrap()
            .is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn temporarily_locked_file_keeps_its_previous_index() {
        use std::os::windows::fs::OpenOptionsExt;
        let ws = TempWs::new("locked-index");
        let file = ws.write("locked.md", "preserved text");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        let guard = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        assert!(!svc.index_single_file(&file).unwrap());
        assert_eq!(svc.indexed_file_count(), 1);
        assert_eq!(
            svc.find_fts_candidates(&["preserved".into()], 10)
                .unwrap()
                .len(),
            1
        );
        drop(guard);
    }

    #[test]
    fn full_index_removes_vanished_files() {
        let ws = TempWs::new("vanish");
        let a = ws.write("a.md", "甲");
        ws.write("b.md", "乙");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        assert_eq!(svc.list_indexed_files().len(), 2);

        fs::remove_file(&a).unwrap();
        svc.run_full_index().unwrap();
        let left: Vec<String> = svc
            .list_indexed_files()
            .into_iter()
            .map(|c| c.rel_path)
            .collect();
        assert_eq!(left, ["b.md"]);
    }

    #[test]
    fn cjk_search_finds_notes_by_single_character() {
        let ws = TempWs::new("cjk");
        ws.write("note.md", "# 计算机通识\n这是关于操作系统的笔记");
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        let hits = svc
            .find_fts_candidates(&["操作系统".to_string()], 10)
            .expect("FTS 应可用");
        assert_eq!(hits.len(), 1, "CJK 逐字分词没生效");
        assert_eq!(hits[0].title, "note");
    }

    #[test]
    fn multi_term_search_requires_all_terms() {
        let ws = TempWs::new("and");
        ws.write("both.md", "包含 甲 也包含 乙");
        ws.write("one.md", "只有 甲");
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        let hits = svc
            .find_fts_candidates(&["甲".to_string(), "乙".to_string()], 10)
            .unwrap();
        let names: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(names, ["both"]);
    }

    #[test]
    fn empty_query_returns_none_so_caller_falls_back() {
        let ws = TempWs::new("empty-q");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        assert!(svc.find_fts_candidates(&["   ".to_string()], 10).is_none());
    }

    #[test]
    fn index_single_file_adds_and_reindexes() {
        let ws = TempWs::new("single");
        let svc = ws.svc();
        let p = ws.write("n.md", "初版 内容");
        assert!(svc.index_single_file(&p).unwrap());
        assert_eq!(svc.list_indexed_files().len(), 1);

        fs::write(&p, "改版 内容").unwrap();
        assert!(svc.index_single_file(&p).unwrap());
        assert_eq!(svc.list_indexed_files().len(), 1, "重复索引不该产生第二条");

        let hits = svc.find_fts_candidates(&["改版".to_string()], 10).unwrap();
        assert_eq!(hits.len(), 1, "旧内容没被替换");
    }

    #[test]
    fn index_single_file_of_missing_file_removes_it() {
        let ws = TempWs::new("single-gone");
        let svc = ws.svc();
        let p = ws.write("n.md", "x");
        svc.index_single_file(&p).unwrap();
        fs::remove_file(&p).unwrap();

        assert!(svc.index_single_file(&p).unwrap(), "应转为移除并返回 true");
        assert!(svc.list_indexed_files().is_empty());
    }

    #[test]
    fn non_indexable_extension_is_rejected() {
        let ws = TempWs::new("ext");
        let svc = ws.svc();
        let p = ws.write("a.png", "x");
        assert!(!svc.index_single_file(&p).unwrap());
    }

    #[test]
    fn links_resolve_by_relative_path_basename_and_title() {
        let ws = TempWs::new("links");
        ws.write("知识库/目标.md", "# 目标\n正文");
        ws.write(
            "源.md",
            "[[目标]] 与 [x](./知识库/目标.md) 与 [[知识库/目标]]",
        );
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        let slot = svc.lock();
        let conn = slot.as_ref().unwrap();
        let unresolved: i64 = conn
            .query_row(
                "SELECT count(*) FROM links WHERE target_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let total: i64 = conn
            .query_row("SELECT count(*) FROM links", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(unresolved, 0, "三种写法都应解析到同一目标");
    }

    #[test]
    fn standalone_mochi_document_links_resolve_and_include_self_links() {
        let ws = TempWs::new("mochi-links");
        let target = ws.write("知识库/旧.md", "# 旧\n");
        let url = format!(
            "{}&label=%E8%87%AA%E5%B7%B1&view=card",
            crate::mochi_url::build_mochi_resource_url(
                &target,
                crate::mochi_url::ResourceKind::File,
                Some(&ws.0),
            )
        );
        fs::write(&target, format!("# 旧\n{url}\n")).unwrap();
        let source = ws.write("引用.md", &format!("{url}\n"));
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        let links = svc.get_links_to(&target).unwrap();
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|link| link.kind == "object"));
        assert!(links.iter().any(|link| link.source_path == target));
        assert!(links.iter().any(|link| link.source_path == source));
        assert!(links
            .iter()
            .all(|link| link.target_path.as_deref() == Some(target.as_path())));
    }

    #[test]
    fn parser_revision_reindexes_unchanged_notes_once() {
        let ws = TempWs::new("mochi-parser-revision");
        let target = ws.write("目标.md", "# 目标\n");
        let url = crate::mochi_url::build_mochi_resource_url(
            &target,
            crate::mochi_url::ResourceKind::File,
            Some(&ws.0),
        );
        ws.write("引用.md", &format!("{url}\n"));
        let svc = ws.svc();
        assert_eq!(svc.run_full_index().unwrap(), 2);
        {
            let slot = svc.lock();
            let conn = slot.as_ref().unwrap();
            conn.execute("DELETE FROM links", []).unwrap();
            conn.execute(
                "UPDATE meta SET value = '1' WHERE key = 'note_parser_revision'",
                [],
            )
            .unwrap();
        }

        assert_eq!(
            svc.run_full_index().unwrap(),
            2,
            "旧 parser 版本应强制重新解析"
        );
        assert_eq!(svc.get_links_to(&target).unwrap().len(), 1);
        assert_eq!(
            svc.run_full_index().unwrap(),
            0,
            "版本迁移后恢复 mtime 增量语义"
        );
    }

    #[test]
    fn backlinks_exclude_self_and_preserve_order_fields_and_disk_context() {
        let ws = TempWs::new("backlinks-query");
        let target = ws.write("目标.md", "[[目标]]");
        ws.write("b.md", "[[目标]]");
        let source = ws.write("a.md", "开头\r\n  [[目标#章节|别名]]  \r\n[[目标#^块]]");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        let links = svc.get_backlinks(&target).unwrap();
        assert_eq!(links.len(), 3);
        let links_for_rename = svc.get_links_to(&target).unwrap();
        assert_eq!(links_for_rename.len(), 4);
        assert!(links_for_rename
            .iter()
            .any(|link| link.source_path == target && link.target_raw == "目标"));
        assert_eq!(
            links
                .iter()
                .map(|l| (l.source_title.as_str(), l.line))
                .collect::<Vec<_>>(),
            [("a", 2), ("a", 3), ("b", 1)]
        );
        assert_eq!(links[0].heading.as_deref(), Some("章节"));
        assert_eq!(links[0].alias.as_deref(), Some("别名"));
        assert_eq!(links[1].block_id.as_deref(), Some("块"));
        assert_eq!(links[0].kind, "wiki");
        assert_eq!(links[0].context, "[[目标#章节|别名]]");
        assert_eq!(links[0].target_path.as_deref(), Some(target.as_path()));
        fs::write(&source, "开头\n磁盘新上下文\n").unwrap();
        assert_eq!(
            svc.get_backlinks(&target).unwrap()[0].context,
            "磁盘新上下文"
        );
        fs::remove_file(&source).unwrap();
        assert!(svc.get_backlinks(&target).unwrap()[0].context.is_empty());
    }

    #[test]
    fn outgoing_links_include_uncreated_targets_and_follow_index_updates() {
        let ws = TempWs::new("outgoing-query");
        let source = ws.write("source.md", "[[稍后创建]]\n[链接](目标.md)\n![[目标]]");
        let target = ws.write("目标.md", "正文");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        let links = svc.get_outgoing_links(&source).unwrap();
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].target_raw, "稍后创建");
        assert!(links[0].target_path.is_none());
        assert_eq!(links[1].target_path.as_deref(), Some(target.as_path()));
        assert_eq!(links[1].kind, "markdown");
        assert_eq!(links[2].kind, "embed");
        assert!(links.iter().all(|l| l.context.is_empty()));
        let created = ws.write("稍后创建.md", "现在存在");
        svc.index_single_file(&created).unwrap();
        assert_eq!(
            svc.get_outgoing_links(&source).unwrap()[0]
                .target_path
                .as_deref(),
            Some(created.as_path())
        );
        svc.remove_from_index(&target).unwrap();
        assert!(svc.get_outgoing_links(&source).unwrap()[1]
            .target_path
            .is_none());
        assert!(svc
            .get_backlinks(ws.0.join("不存在.md"))
            .unwrap()
            .is_empty());
        assert!(svc
            .get_outgoing_links(ws.0.join("不存在.md"))
            .unwrap()
            .is_empty());
        assert!(svc
            .get_unlinked_mentions(ws.0.join("不存在.md"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn mentions_exclude_linked_sources_and_link_syntax_and_use_first_plain_line() {
        let ws = TempWs::new("mentions-query");
        let target = ws.write("墨池笔记.md", "墨池笔记自己的提及");
        ws.write("linked.md", "墨池笔记\n[[墨池笔记]]");
        ws.write("plain.md", "  墨池笔记是正文  \r\n再次墨池笔记");
        ws.write("syntax.md", "墨池笔记 [其他](missing.md)\n正文墨池笔记");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        let mut mentions = svc.get_unlinked_mentions(&target).unwrap();
        mentions.sort_by(|a, b| a.source_title.cmp(&b.source_title));
        assert_eq!(mentions.len(), 2);
        assert_eq!(
            (
                mentions[0].source_title.as_str(),
                mentions[0].line,
                mentions[0].context.as_str()
            ),
            ("plain", 1, "墨池笔记是正文")
        );
        assert_eq!(
            (mentions[1].source_title.as_str(), mentions[1].line),
            ("syntax", 2)
        );
    }

    #[test]
    fn mentions_are_case_insensitive_and_short_titles_are_skipped() {
        let ws = TempWs::new("mentions-case");
        let target = ws.write("Mochi.md", "内容");
        let short = ws.write("墨.md", "内容");
        ws.write("plain.md", "MOCHI 墨 mochi");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        assert_eq!(svc.get_unlinked_mentions(target).unwrap().len(), 1);
        assert!(svc.get_unlinked_mentions(short).unwrap().is_empty());
    }

    #[test]
    fn context_truncation_is_utf16_bounded_and_unicode_safe() {
        assert_eq!(trim_to_chars(&"中".repeat(300), 240), "中".repeat(240));
        assert_eq!(trim_to_chars(&"😀".repeat(130), 240), "😀".repeat(120));
        assert_eq!(trim_to_chars("中😀文", 2), "中");
    }

    #[test]
    fn dangling_link_stays_null_then_resolves_when_target_appears() {
        let ws = TempWs::new("dangling");
        ws.write("源.md", "[[稍后出现]]");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        {
            let slot = svc.lock();
            let n: i64 = slot
                .as_ref()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM links WHERE target_id IS NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1);
        }

        let target = ws.write("稍后出现.md", "来了");
        svc.index_single_file(&target).unwrap();

        let slot = svc.lock();
        let n: i64 = slot
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM links WHERE target_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "目标出现后悬空链接应被补齐");
    }

    #[test]
    fn removing_a_note_nulls_inbound_links() {
        let ws = TempWs::new("unlink");
        let target = ws.write("目标.md", "内容");
        ws.write("源.md", "[[目标]]");
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        assert!(svc.remove_from_index(&target).unwrap());

        let slot = svc.lock();
        let n: i64 = slot
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM links WHERE target_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "指向已删除笔记的链接应回到 NULL");
    }

    #[test]
    fn tags_properties_headings_are_stored() {
        let ws = TempWs::new("relations");
        ws.write(
            "n.md",
            "---\ntitle: 标题\ntags: [甲]\n---\n# 一级\n正文 #乙\n## 二级\n",
        );
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        let slot = svc.lock();
        let conn = slot.as_ref().unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM headings"), 2);
        assert_eq!(
            count("SELECT count(*) FROM tags"),
            2,
            "frontmatter + inline"
        );
        assert!(count("SELECT count(*) FROM properties") >= 2);
    }

    #[test]
    fn duplicate_tags_on_same_line_are_deduped() {
        let ws = TempWs::new("dedupe");
        ws.write("n.md", "#重复 #重复 #重复\n");
        let svc = ws.svc();
        svc.run_full_index().unwrap();

        let slot = svc.lock();
        let n: i64 = slot
            .as_ref()
            .unwrap()
            .query_row("SELECT count(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "同一行同名标签应只存一条");
    }

    #[test]
    fn oversized_files_are_skipped() {
        let ws = TempWs::new("big");
        let big = "x".repeat((MAX_INDEXABLE_FILE_SIZE + 1) as usize);
        ws.write("big.md", &big);
        ws.write("small.md", "ok");
        let svc = ws.svc();
        svc.run_full_index().unwrap();
        let names: Vec<String> = svc
            .list_indexed_files()
            .into_iter()
            .map(|c| c.rel_path)
            .collect();
        assert_eq!(names, ["small.md"]);
    }

    #[test]
    fn remove_extension_only_cuts_after_last_slash() {
        assert_eq!(remove_extension("a/b.md"), "a/b");
        assert_eq!(remove_extension("dir.v2/note"), "dir.v2/note");
        assert_eq!(remove_extension("noext"), "noext");
        assert_eq!(remove_extension(".hidden"), ".hidden");
    }

    #[test]
    fn index_is_rebuildable_after_deleting_db() {
        let ws = TempWs::new("rebuild");
        ws.write("a.md", "内容");
        {
            let svc = ws.svc();
            svc.run_full_index().unwrap();
            svc.close();
        }
        fs::remove_file(ws.0.join(".mochi").join("index.db")).unwrap();

        let svc = ws.svc();
        assert_eq!(svc.run_full_index().unwrap(), 1, "删库后应能重建");
    }
}
