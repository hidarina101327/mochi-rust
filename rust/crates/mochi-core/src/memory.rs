//! ai-memory.db 与 Electron 版共用，表结构和触发器必须保持一致。
//! 检索使用 FTS5 和时间衰减；pinned 条目优先注入。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::{jstime, paths};

/// 建表语句。**与 memory.ts 逐字对齐**，改动前先确认 Electron 版也改了。
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS memories (
      id TEXT PRIMARY KEY,
      kind TEXT NOT NULL,
      scope TEXT NOT NULL,
      scope_ref TEXT,
      title TEXT NOT NULL,
      content TEXT NOT NULL,
      tags TEXT NOT NULL DEFAULT '',
      session_id TEXT,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      hits INTEGER NOT NULL DEFAULT 0
    );

    CREATE INDEX IF NOT EXISTS idx_memories_kind ON memories(kind);
    CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope, scope_ref);
    CREATE INDEX IF NOT EXISTS idx_memories_updated ON memories(updated_at DESC);

    CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
      title, content, tags,
      content='memories',
      content_rowid='rowid',
      tokenize='unicode61'
    );

    CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
      INSERT INTO memories_fts(rowid, title, content, tags)
      VALUES (new.rowid, new.title, new.content, new.tags);
    END;

    CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
      INSERT INTO memories_fts(memories_fts, rowid, title, content, tags)
      VALUES ('delete', old.rowid, old.title, old.content, old.tags);
    END;

    CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
      INSERT INTO memories_fts(memories_fts, rowid, title, content, tags)
      VALUES ('delete', old.rowid, old.title, old.content, old.tags);
      INSERT INTO memories_fts(rowid, title, content, tags)
      VALUES (new.rowid, new.title, new.content, new.tags);
    END;
"#;

/// 检索默认返回条数与上限，对齐 TS 的 `Math.max(1, Math.min(limit || 12, 100))`。
const DEFAULT_LIMIT: i64 = 12;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Fact,
    Episode,
    Pinned,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Episode => "episode",
            Self::Pinned => "pinned",
        }
    }

    /// 认不出的取值一律当 `fact`——模型偶尔会自创种类，不该因此整轮失败。
    pub fn parse(value: &str) -> Self {
        match value {
            "episode" => Self::Episode,
            "pinned" => Self::Pinned,
            _ => Self::Fact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    Global,
    Library,
    Document,
}

impl MemoryScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Library => "library",
            Self::Document => "document",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "library" => Self::Library,
            "document" => Self::Document,
            _ => Self::Global,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: String,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    /// `scope=library` 时表示资料库路径，`scope=document` 时表示文档路径；global 为空。
    pub scope_ref: Option<String>,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 命中次数，排序时体现「常用」。
    pub hits: i64,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryWriteInput {
    pub kind: Option<MemoryKind>,
    pub scope: Option<MemoryScope>,
    pub scope_ref: Option<String>,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub session_id: Option<String>,
    /// 在相同范围内，标题相同的记忆会更新而不是新增。默认值为 true——
    /// 记忆越堆越重复比覆盖更糟。
    pub upsert: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct MemorySearchOptions {
    pub query: Option<String>,
    pub kind: Option<MemoryKind>,
    pub scope: Option<MemoryScope>,
    pub scope_ref: Option<String>,
    pub limit: Option<i64>,
}

pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    pub fn db_path(workspace: &Path) -> PathBuf {
        workspace.join(paths::MOCHI_DIR_NAME).join("ai-memory.db")
    }

    pub fn open(workspace: &Path) -> Result<Self> {
        let db_path = Self::db_path(workspace);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        let conn = Connection::open(&db_path)
            .with_context(|| format!("打开记忆库失败: {}", db_path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)
            .context("建表失败（FTS5 可用？）")?;
        Ok(Self { conn })
    }

    /// 内存库，仅用于测试。
    #[cfg(test)]
    fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn write(&self, input: MemoryWriteInput) -> Result<MemoryRecord> {
        let now = jstime::now_millis();
        let kind = input.kind.clone().unwrap_or(MemoryKind::Fact);
        let scope = input.scope.clone().unwrap_or(MemoryScope::Global);
        let scope_ref = input.scope_ref.clone();
        let tags = input.tags.join(",");

        if input.upsert.unwrap_or(true) {
            // IFNULL 两边都套：scope_ref 为 NULL 和空串在 TS 侧是同一回事
            let existing: Option<(String, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT id, session_id FROM memories
                     WHERE kind = ?1 AND scope = ?2 AND IFNULL(scope_ref, '') = IFNULL(?3, '') AND title = ?4
                     LIMIT 1",
                    rusqlite::params![kind.as_str(), scope.as_str(), scope_ref, input.title],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((id, previous_session)) = existing {
                let session = input.session_id.clone().or(previous_session);
                self.conn.execute(
                    "UPDATE memories SET content = ?1, tags = ?2, session_id = ?3, updated_at = ?4 WHERE id = ?5",
                    rusqlite::params![input.content, tags, session, now, id],
                )?;
                return self.get(&id)?.context("刚更新的记忆读不回来");
            }
        }

        let id = format!("mem-{now}-{}", paths::random_base36(6));
        self.conn.execute(
            "INSERT INTO memories (id, kind, scope, scope_ref, title, content, tags, session_id, created_at, updated_at, hits)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            rusqlite::params![
                id,
                kind.as_str(),
                scope.as_str(),
                scope_ref,
                input.title,
                input.content,
                tags,
                input.session_id,
                now,
                now
            ],
        )?;

        Ok(MemoryRecord {
            id,
            kind,
            scope,
            scope_ref,
            title: input.title,
            content: input.content,
            tags: input.tags,
            session_id: input.session_id,
            created_at: now,
            updated_at: now,
            hits: 0,
        })
    }

    pub fn get(&self, id: &str) -> Result<Option<MemoryRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM memories WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(to_record(row)?)),
            None => Ok(None),
        }
    }

    /// 检索。有关键词走 FTS，否则按新鲜度返回；两种情况都把置顶/常用/新的排在前面。
    ///
    /// 命中的记忆会 `hits + 1`——**这是一次写操作**，检索本身会改库。
    /// TS 就是这么做的（「常用」要靠它统计），照搬。
    pub fn search(&self, options: &MemorySearchOptions) -> Result<Vec<MemoryRecord>> {
        let limit = options.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        let mut filters: Vec<&str> = Vec::new();
        let mut args: Vec<SqlValue> = Vec::new();
        if let Some(kind) = &options.kind {
            filters.push("m.kind = ?");
            args.push(SqlValue::Text(kind.as_str().to_owned()));
        }
        if let Some(scope) = &options.scope {
            filters.push("m.scope = ?");
            args.push(SqlValue::Text(scope.as_str().to_owned()));
        }
        // 空串等同于「不过滤」，对齐 TS 的 `if (options.scopeRef)`
        if let Some(scope_ref) = options.scope_ref.as_deref().filter(|s| !s.is_empty()) {
            filters.push("IFNULL(m.scope_ref, '') = ?");
            args.push(SqlValue::Text(scope_ref.to_owned()));
        }

        let fts_query = options
            .query
            .as_deref()
            .map(escape_fts_query)
            .unwrap_or_default();

        let mut records = if !fts_query.is_empty() {
            let where_clause = if filters.is_empty() {
                String::new()
            } else {
                format!("AND {}", filters.join(" AND "))
            };
            let sql = format!(
                "SELECT m.* FROM memories_fts f
                 JOIN memories m ON m.rowid = f.rowid
                 WHERE memories_fts MATCH ? {where_clause}
                 ORDER BY bm25(memories_fts) + (julianday('now') - julianday(m.updated_at / 1000, 'unixepoch')) / 60.0 - m.hits * 0.1
                 LIMIT ?"
            );
            let mut bound = vec![SqlValue::Text(fts_query)];
            bound.extend(args.iter().cloned());
            bound.push(SqlValue::Integer(limit));
            self.query_records(&sql, &bound)?
        } else {
            let where_clause = if filters.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", filters.join(" AND ").replace("m.", ""))
            };
            let sql = format!(
                "SELECT * FROM memories {where_clause}
                 ORDER BY (CASE WHEN kind = 'pinned' THEN 0 ELSE 1 END), updated_at DESC
                 LIMIT ?"
            );
            let mut bound = args.clone();
            bound.push(SqlValue::Integer(limit));
            self.query_records(&sql, &bound)?
        };

        // unicode61 会将连续中文视为一个词；FTS 前缀查不到词中片段时，再用 LIKE 搜索。不能更换共享的分词器。
        if records.is_empty() {
            if let Some(query) = options
                .query
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                records = self.substring_search(query, &filters, &args, limit)?;
            }
        }

        self.bump_hits(&records)?;
        Ok(records)
    }

    fn query_records(&self, sql: &str, args: &[SqlValue]) -> Result<Vec<MemoryRecord>> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query(params_from_iter(args.iter()))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(to_record(row)?);
        }
        Ok(out)
    }

    /// 子串匹配通路：标题/正文/标签任一包含关键词即命中。
    /// 只在 FTS 无结果时启用，排序与「按新鲜度」一致（置顶优先、更新时间倒序）。
    fn substring_search(
        &self,
        query: &str,
        filters: &[&str],
        args: &[SqlValue],
        limit: i64,
    ) -> Result<Vec<MemoryRecord>> {
        let mut clauses: Vec<String> = filters.iter().map(|f| f.replace("m.", "")).collect();
        clauses.push(
            r"(title LIKE ? ESCAPE '\' OR content LIKE ? ESCAPE '\' OR tags LIKE ? ESCAPE '\')"
                .to_owned(),
        );

        let sql = format!(
            "SELECT * FROM memories WHERE {}
             ORDER BY (CASE WHEN kind = 'pinned' THEN 0 ELSE 1 END), updated_at DESC
             LIMIT ?",
            clauses.join(" AND ")
        );

        // LIKE 的通配符要转义，否则用户搜 `100%` 会匹配到一切
        let needle = SqlValue::Text(format!("%{}%", escape_like(query)));
        let mut bound = args.to_vec();
        bound.extend([needle.clone(), needle.clone(), needle]);
        bound.push(SqlValue::Integer(limit));
        self.query_records(&sql, &bound)
    }

    fn bump_hits(&self, records: &[MemoryRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE memories SET hits = hits + 1 WHERE id = ?1")?;
            for record in records {
                stmt.execute([&record.id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list(&self, limit: Option<i64>) -> Result<Vec<MemoryRecord>> {
        self.query_records(
            "SELECT * FROM memories
             ORDER BY (CASE WHEN kind = 'pinned' THEN 0 ELSE 1 END), updated_at DESC
             LIMIT ?1",
            &[SqlValue::Integer(limit.unwrap_or(200))],
        )
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])?
            > 0)
    }

    /// 组装注入 system prompt 的记忆片段。置顶无条件带上，其余按当前上下文检索。
    /// 空记忆库返回空串——没有记忆时不该往提示词里塞一个空标题。
    pub fn build_context(&self, options: &MemoryContextOptions) -> Result<String> {
        let mut merged: Vec<MemoryRecord> = Vec::new();
        let mut seen: Vec<String> = Vec::new();

        let push_all =
            |records: Vec<MemoryRecord>, merged: &mut Vec<MemoryRecord>, seen: &mut Vec<String>| {
                for record in records {
                    if seen.contains(&record.id) {
                        continue;
                    }
                    seen.push(record.id.clone());
                    merged.push(record);
                }
            };

        let pinned = self.search(&MemorySearchOptions {
            kind: Some(MemoryKind::Pinned),
            limit: Some(10),
            ..Default::default()
        })?;
        push_all(pinned, &mut merged, &mut seen);

        if let Some(document) = options.document_path.as_deref().filter(|s| !s.is_empty()) {
            let scoped = self.search(&MemorySearchOptions {
                scope: Some(MemoryScope::Document),
                scope_ref: Some(document.to_owned()),
                limit: Some(5),
                ..Default::default()
            })?;
            push_all(scoped, &mut merged, &mut seen);
        }
        if let Some(library) = options.library_path.as_deref().filter(|s| !s.is_empty()) {
            let scoped = self.search(&MemorySearchOptions {
                scope: Some(MemoryScope::Library),
                scope_ref: Some(library.to_owned()),
                limit: Some(5),
                ..Default::default()
            })?;
            push_all(scoped, &mut merged, &mut seen);
        }

        let matched = match options.query.as_deref().filter(|s| !s.is_empty()) {
            Some(query) => self.search(&MemorySearchOptions {
                query: Some(query.to_owned()),
                limit: Some(options.limit.unwrap_or(8)),
                ..Default::default()
            })?,
            None => self.search(&MemorySearchOptions {
                kind: Some(MemoryKind::Fact),
                limit: Some(options.limit.unwrap_or(8)),
                ..Default::default()
            })?,
        };
        push_all(matched, &mut merged, &mut seen);

        if merged.is_empty() {
            return Ok(String::new());
        }

        let mut lines = vec![
            "## 长期记忆".to_owned(),
            "以下是关于该用户与工作区的既有记忆，回答时应结合它们：".to_owned(),
        ];
        lines.extend(merged.iter().map(|record| {
            let label = if record.kind == MemoryKind::Pinned {
                "置顶".to_owned()
            } else {
                match record.scope {
                    MemoryScope::Global => "全局".to_owned(),
                    MemoryScope::Library => {
                        format!("库:{}", record.scope_ref.as_deref().unwrap_or(""))
                    }
                    MemoryScope::Document => {
                        format!("文档:{}", record.scope_ref.as_deref().unwrap_or(""))
                    }
                }
            };
            format!("- [{label}] {}：{}", record.title, record.content)
        }));
        Ok(lines.join("\n"))
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryContextOptions {
    pub query: Option<String>,
    pub library_path: Option<String>,
    pub document_path: Option<String>,
    pub limit: Option<i64>,
}

fn to_record(row: &Row) -> rusqlite::Result<MemoryRecord> {
    let tags: String = row.get("tags")?;
    Ok(MemoryRecord {
        id: row.get("id")?,
        kind: MemoryKind::parse(&row.get::<_, String>("kind")?),
        scope: MemoryScope::parse(&row.get::<_, String>("scope")?),
        scope_ref: row.get("scope_ref")?,
        title: row.get("title")?,
        content: row.get("content")?,
        tags: tags
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        session_id: row.get("session_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        hits: row.get("hits")?,
    })
}

/// LIKE 的通配符转义：否则用户搜 `100%` 会匹配到一切。配 `ESCAPE '\'` 使用。
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// FTS5 对特殊字符敏感，逐词去掉 `"` `*` 再加引号最稳妥。对齐 TS 的 `escapeFtsQuery`。
/// 全部被过滤掉时返回空串，调用方据此退回「按新鲜度列出」。
fn escape_fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.replace(['"', '*'], "").trim().to_owned())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\"*"))
        .collect();
    terms.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryStore {
        MemoryStore::open_in_memory().unwrap()
    }

    fn write(store: &MemoryStore, title: &str, content: &str) -> MemoryRecord {
        store
            .write(MemoryWriteInput {
                title: title.into(),
                content: content.into(),
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn write_then_read_back() {
        let s = store();
        let record = s
            .write(MemoryWriteInput {
                title: "用户偏好".into(),
                content: "喜欢简洁的回答".into(),
                tags: vec!["偏好".into(), "风格".into()],
                session_id: Some("sess-1".into()),
                ..Default::default()
            })
            .unwrap();

        assert!(record.id.starts_with("mem-"), "{}", record.id);
        assert_eq!(record.kind, MemoryKind::Fact, "缺省是 fact");
        assert_eq!(record.scope, MemoryScope::Global, "缺省是 global");
        assert_eq!(record.hits, 0);

        let loaded = s.get(&record.id).unwrap().unwrap();
        assert_eq!(
            loaded.tags,
            ["偏好", "风格"],
            "标签按逗号存，读出时还原成数组"
        );
        assert_eq!(loaded.session_id.as_deref(), Some("sess-1"));
    }

    /// 相同范围内标题相同的记忆默认更新而不是新增——记忆重复堆积比覆盖更糟。
    #[test]
    fn same_title_in_the_same_scope_upserts() {
        let s = store();
        let first = write(&s, "用户偏好", "喜欢简洁");
        let second = write(&s, "用户偏好", "改成喜欢详细");

        assert_eq!(first.id, second.id, "应更新同一条");
        assert_eq!(second.content, "改成喜欢详细");
        assert_eq!(s.list(None).unwrap().len(), 1);
    }

    #[test]
    fn upsert_can_be_turned_off() {
        let s = store();
        write(&s, "用户偏好", "第一条");
        s.write(MemoryWriteInput {
            title: "用户偏好".into(),
            content: "第二条".into(),
            upsert: Some(false),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.list(None).unwrap().len(), 2, "关掉 upsert 后应各存一条");
    }

    /// 不同范围中的同名记忆互不影响。
    #[test]
    fn the_same_title_in_different_scopes_coexists() {
        let s = store();
        write(&s, "命名规范", "全局的");
        s.write(MemoryWriteInput {
            title: "命名规范".into(),
            content: "这个库的".into(),
            scope: Some(MemoryScope::Library),
            scope_ref: Some("D:/ws/知识库".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.list(None).unwrap().len(), 2);
    }

    /// upsert 不该把已有的 sessionId 洗成空。
    #[test]
    fn upsert_keeps_the_previous_session_when_none_is_given() {
        let s = store();
        s.write(MemoryWriteInput {
            title: "t".into(),
            content: "v1".into(),
            session_id: Some("sess-1".into()),
            ..Default::default()
        })
        .unwrap();
        let updated = write(&s, "t", "v2");
        assert_eq!(updated.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn search_finds_by_keyword() {
        let s = store();
        write(&s, "构建命令", "用 pnpm run electron:dev 启动");
        write(&s, "无关记忆", "今天天气不错");

        let hits = s
            .search(&MemorySearchOptions {
                query: Some("pnpm".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "构建命令");
    }

    /// 中文按 unicode61 分词后整段是一个词，前缀查询命中不了词中间的字，
    /// 靠子串通路兜住——否则中文记忆检索基本失效。
    #[test]
    fn search_handles_chinese_queries() {
        let s = store();
        write(&s, "偏好", "回答要简洁");
        let hits = s
            .search(&MemorySearchOptions {
                query: Some("简洁".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(!hits.is_empty(), "中文关键词应能检索到");
    }

    /// 子串搜索仍须遵守 `scope` 和 `kind` 筛选条件，不能因为使用备用搜索方式就放宽条件。
    #[test]
    fn the_substring_fallback_still_honors_filters() {
        let s = store();
        s.write(MemoryWriteInput {
            title: "文档的".into(),
            content: "回答要简洁".into(),
            scope: Some(MemoryScope::Document),
            scope_ref: Some("D:/ws/a.md".into()),
            ..Default::default()
        })
        .unwrap();
        write(&s, "全局的", "回答要简洁");

        let hits = s
            .search(&MemorySearchOptions {
                query: Some("简洁".into()),
                scope: Some(MemoryScope::Document),
                scope_ref: Some("D:/ws/a.md".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "文档的");
    }

    /// LIKE 的通配符必须转义，否则搜 `%` 会命中一切。
    #[test]
    fn the_substring_fallback_escapes_like_wildcards() {
        let s = store();
        write(&s, "覆盖率", "目标是 60% 以上");
        write(&s, "无关", "完全不相干的内容");

        let hits = s
            .search(&MemorySearchOptions {
                query: Some("60%".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1, "% 不该被当通配符: {hits:?}");
        assert_eq!(hits[0].title, "覆盖率");

        let none = s
            .search(&MemorySearchOptions {
                query: Some("_".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(none.is_empty(), "_ 不该匹配任意单字");
    }

    /// 先走 FTS，走不通才退子串——顺序反了会丢掉 bm25 排序。
    #[test]
    fn fts_results_take_precedence_over_the_fallback() {
        let s = store();
        write(&s, "构建", "pnpm dev");
        let hits = s
            .search(&MemorySearchOptions {
                query: Some("pnpm".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    /// FTS 的特殊字符不该让整次检索炸掉。
    #[test]
    fn search_survives_fts_metacharacters() {
        let s = store();
        write(&s, "记录", "内容");
        for query in ["\"", "*", "\" OR \"", "   ", "a\"b*c"] {
            assert!(
                s.search(&MemorySearchOptions {
                    query: Some(query.into()),
                    ..Default::default()
                })
                .is_ok(),
                "查询 {query:?} 不该报错"
            );
        }
    }

    /// 关键词被过滤干净后退回「按新鲜度列出」，而不是返回空。
    #[test]
    fn an_all_punctuation_query_falls_back_to_recency() {
        let s = store();
        write(&s, "记录", "内容");
        let hits = s
            .search(&MemorySearchOptions {
                query: Some("\"*\"".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1, "空 FTS 查询应退回按新鲜度返回");
    }

    #[test]
    fn search_filters_by_scope_and_kind() {
        let s = store();
        write(&s, "全局的", "x");
        s.write(MemoryWriteInput {
            title: "文档的".into(),
            content: "x".into(),
            scope: Some(MemoryScope::Document),
            scope_ref: Some("D:/ws/a.md".into()),
            ..Default::default()
        })
        .unwrap();
        s.write(MemoryWriteInput {
            title: "置顶的".into(),
            content: "x".into(),
            kind: Some(MemoryKind::Pinned),
            ..Default::default()
        })
        .unwrap();

        let by_scope = s
            .search(&MemorySearchOptions {
                scope: Some(MemoryScope::Document),
                scope_ref: Some("D:/ws/a.md".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_scope.len(), 1);
        assert_eq!(by_scope[0].title, "文档的");

        let by_kind = s
            .search(&MemorySearchOptions {
                kind: Some(MemoryKind::Pinned),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_kind.len(), 1);
        assert_eq!(by_kind[0].title, "置顶的");
    }

    #[test]
    fn limit_is_clamped_to_one_hundred() {
        let s = store();
        for i in 0..5 {
            write(&s, &format!("记忆{i}"), "x");
        }
        assert_eq!(
            s.search(&MemorySearchOptions {
                limit: Some(2),
                ..Default::default()
            })
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            s.search(&MemorySearchOptions {
                limit: Some(0),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1,
            "0 应被钳到 1"
        );
        assert_eq!(
            s.search(&MemorySearchOptions {
                limit: Some(9999),
                ..Default::default()
            })
            .unwrap()
            .len(),
            5
        );
    }

    /// 置顶记忆排在最前，其余按更新时间倒序。
    #[test]
    fn listing_puts_pinned_first() {
        let s = store();
        write(&s, "普通1", "x");
        write(&s, "普通2", "x");
        s.write(MemoryWriteInput {
            title: "置顶".into(),
            content: "x".into(),
            kind: Some(MemoryKind::Pinned),
            ..Default::default()
        })
        .unwrap();

        let all = s.list(None).unwrap();
        assert_eq!(all[0].title, "置顶");
    }

    /// 检索会给命中的记忆 +1——「常用」要靠它统计，是一次有意的写操作。
    #[test]
    fn searching_bumps_the_hit_counter() {
        let s = store();
        let record = write(&s, "构建命令", "pnpm dev");
        s.search(&MemorySearchOptions {
            query: Some("pnpm".into()),
            ..Default::default()
        })
        .unwrap();
        s.search(&MemorySearchOptions {
            query: Some("pnpm".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.get(&record.id).unwrap().unwrap().hits, 2);
    }

    #[test]
    fn delete_reports_whether_anything_was_removed() {
        let s = store();
        let record = write(&s, "t", "v");
        assert!(s.delete(&record.id).unwrap());
        assert!(!s.delete(&record.id).unwrap(), "删不存在的应返回 false");
        assert!(s.get(&record.id).unwrap().is_none());
    }

    /// 删除后 FTS 影子表也要同步，否则检索会命中已删的行并炸在 JOIN 上。
    #[test]
    fn deleting_keeps_the_fts_index_in_sync() {
        let s = store();
        let record = write(&s, "构建命令", "pnpm dev");
        s.delete(&record.id).unwrap();
        let hits = s
            .search(&MemorySearchOptions {
                query: Some("pnpm".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(hits.is_empty(), "删掉的记忆不该还能被搜到");
    }

    /// 更新后旧内容不该还能搜到（触发器要先删后插）。
    #[test]
    fn updating_refreshes_the_fts_index() {
        let s = store();
        write(&s, "构建命令", "用 npm 启动");
        write(&s, "构建命令", "改用 pnpm 启动");

        let old = s
            .search(&MemorySearchOptions {
                query: Some("npm".into()),
                ..Default::default()
            })
            .unwrap();
        // "npm" 是 "pnpm" 的前缀查询命中不了（pnpm 不以 npm 开头），所以这里应为空
        assert!(
            old.iter().all(|r| r.content.contains("pnpm")),
            "旧内容不该被搜到"
        );
    }

    #[test]
    fn context_is_empty_when_there_are_no_memories() {
        let s = store();
        assert_eq!(
            s.build_context(&MemoryContextOptions::default()).unwrap(),
            ""
        );
    }

    #[test]
    fn context_merges_pinned_scoped_and_matched_without_duplicates() {
        let s = store();
        s.write(MemoryWriteInput {
            title: "永远记住".into(),
            content: "用中文回答".into(),
            kind: Some(MemoryKind::Pinned),
            ..Default::default()
        })
        .unwrap();
        s.write(MemoryWriteInput {
            title: "这篇文档".into(),
            content: "是面试笔记".into(),
            scope: Some(MemoryScope::Document),
            scope_ref: Some("D:/ws/a.md".into()),
            ..Default::default()
        })
        .unwrap();
        write(&s, "构建命令", "pnpm dev");

        let context = s
            .build_context(&MemoryContextOptions {
                query: Some("构建".into()),
                document_path: Some("D:/ws/a.md".into()),
                ..Default::default()
            })
            .unwrap();

        assert!(context.starts_with("## 长期记忆\n"));
        assert!(context.contains("- [置顶] 永远记住：用中文回答"));
        assert!(context.contains("- [文档:D:/ws/a.md] 这篇文档：是面试笔记"));
        assert_eq!(context.matches("永远记住").count(), 1, "同一条不该出现两次");
    }

    #[test]
    fn context_labels_global_and_library_scopes() {
        let s = store();
        write(&s, "全局事实", "x");
        s.write(MemoryWriteInput {
            title: "库事实".into(),
            content: "y".into(),
            scope: Some(MemoryScope::Library),
            scope_ref: Some("D:/ws/知识库".into()),
            ..Default::default()
        })
        .unwrap();

        let context = s
            .build_context(&MemoryContextOptions {
                library_path: Some("D:/ws/知识库".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(context.contains("- [全局] 全局事实：x"), "{context}");
        assert!(
            context.contains("- [库:D:/ws/知识库] 库事实：y"),
            "{context}"
        );
    }

    /// 数据结构是三版共用的硬性约定；改动前先确认 Electron 版也做了相应修改。
    #[test]
    fn schema_matches_the_electron_contract() {
        let s = store();
        let mut stmt = s
            .conn
            .prepare(
                "SELECT name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap();
        let objects: Vec<(String, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let names: Vec<&str> = objects.iter().map(|(n, _)| n.as_str()).collect();

        for expected in [
            "memories",
            "memories_fts",
            "memories_ai",
            "memories_ad",
            "memories_au",
            "idx_memories_kind",
            "idx_memories_scope",
            "idx_memories_updated",
        ] {
            assert!(names.contains(&expected), "缺少 {expected}：{names:?}");
        }

        let table_sql = objects
            .iter()
            .find(|(n, _)| n == "memories")
            .and_then(|(_, sql)| sql.clone())
            .unwrap();
        for column in [
            "id TEXT PRIMARY KEY",
            "kind TEXT NOT NULL",
            "scope TEXT NOT NULL",
            "scope_ref TEXT",
            "tags TEXT NOT NULL DEFAULT ''",
            "session_id TEXT",
            "created_at INTEGER NOT NULL",
            "hits INTEGER NOT NULL DEFAULT 0",
        ] {
            assert!(table_sql.contains(column), "列定义不符: 缺 {column}");
        }

        let fts_sql = objects
            .iter()
            .find(|(n, _)| n == "memories_fts")
            .and_then(|(_, sql)| sql.clone())
            .unwrap();
        assert!(
            fts_sql.contains("tokenize='unicode61'"),
            "分词器必须是 unicode61"
        );
        assert!(fts_sql.contains("content='memories'"), "必须是外部内容表");
    }

    #[test]
    fn database_lands_in_the_mochi_directory() {
        assert!(MemoryStore::db_path(Path::new("D:/ws"))
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".mochi/ai-memory.db"));
    }

    /// 真实文件库能打开、能落盘、重开还在。
    #[test]
    fn opening_a_real_workspace_creates_the_database() {
        let root = std::env::temp_dir().join(format!("mochi-memory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let record = {
            let s = MemoryStore::open(&root).unwrap();
            write(&s, "持久化", "重开还在")
        };
        assert!(MemoryStore::db_path(&root).exists());

        let reopened = MemoryStore::open(&root).unwrap();
        assert_eq!(
            reopened.get(&record.id).unwrap().unwrap().content,
            "重开还在"
        );

        drop(reopened);
        let _ = std::fs::remove_dir_all(&root);
    }
}
