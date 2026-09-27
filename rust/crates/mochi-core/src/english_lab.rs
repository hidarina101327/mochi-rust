//! 沿用 Electron 的 .mochi/extensions-data/english/default/data.db 及其迁移规则。
use anyhow::{Context, Result};
use chrono::{Local, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub struct Service {
    path: PathBuf,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Dashboard {
    pub words: u64,
    pub due: u64,
    pub learned_today: u64,
    pub answers_today: u64,
    pub accuracy: f64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Word {
    pub id: i64,
    pub word: String,
    pub phonetic: String,
    pub meaning: String,
    pub state: String,
    pub due: Option<String>,
    pub mastery: f64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct DictionarySource {
    pub book_id: String,
    pub name: String,
    pub category: String,
    pub level: i64,
    pub word_count: u64,
    pub imported_at: Option<String>,
    pub group_id: Option<i64>,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct DictionaryRemoval {
    pub name: String,
    pub removable_words: u64,
    pub retained_words: u64,
}
/// Electron 的「自定义词表」存为普通训练分组，而非 dictionary_sources。
/// 保留独立类型，让原生词典中心能同时列出两类导入内容。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CustomWordList {
    pub id: i64,
    pub name: String,
    pub word_count: u64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReviewRecord {
    pub id: i64,
    pub word: String,
    pub meaning: String,
    pub question_type: String,
    pub correct: f64,
    pub rating: Option<i64>,
    pub created_at: String,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct LearningStats {
    pub total_answers: u64,
    pub correct_answers: u64,
    pub mastered_words: u64,
    pub learning_words: u64,
    pub active_days: u64,
    pub total_seconds: u64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub translation: String,
    pub difficulty: i64,
    pub source: String,
}
/// 听力页使用的句子。沿用 Electron 数据库已有的 `sentences` 表，因此两个版本可
/// 立即互相看到用户添加的素材。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Sentence {
    pub id: i64,
    pub content: String,
    pub translation: String,
    pub source: String,
    pub difficulty: i64,
}
/// Electron 的「句子精听」不是非黑即白：漏一个词也要保留部分得分。原生版
/// 返回同样可被 UI 使用的得分与标准句，而不把这层判断塞进界面代码。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DictationResult {
    pub expected: String,
    pub score: f64,
    pub exact: bool,
}

impl Service {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            path: workspace
                .as_ref()
                .join(".mochi/extensions-data/english/default/data.db"),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&self.path)
            .with_context(|| format!("打开 English Lab 数据库失败：{}", self.path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        self.migrate(&conn)?;
        Ok(conn)
    }
    pub fn dashboard(&self) -> Result<Dashboard> {
        let conn = self.open()?;
        let day = Local::now().format("%Y-%m-%d").to_string();
        Ok(Dashboard { words: count(&conn,"SELECT COUNT(*) FROM words",[])? , due: count(&conn,"SELECT COUNT(*) FROM word_states WHERE suspended=0 AND due IS NOT NULL AND due<=?1",[&Utc::now().to_rfc3339()])?, learned_today: count(&conn,"SELECT COUNT(*) FROM word_states WHERE date(first_seen)=?1",[&day])?, answers_today: count(&conn,"SELECT COUNT(*) FROM test_records WHERE date(create_time)=?1",[&day])?, accuracy: conn.query_row("SELECT COALESCE(AVG(is_correct),0) FROM test_records WHERE date(create_time)=?1",params![day],|r|r.get(0))? })
    }
    pub fn due_words(&self, limit: usize) -> Result<Vec<Word>> {
        let conn = self.open()?;
        let now = Utc::now().to_rfc3339();
        let mut st=conn.prepare("SELECT w.id,w.word,COALESCE(w.us_phonetic,w.phonetic,''),COALESCE((SELECT chinese FROM meanings m WHERE m.word_id=w.id ORDER BY m.id LIMIT 1),''),COALESCE(s.state,'new'),s.due,COALESCE(s.mastery,0) FROM words w LEFT JOIN word_states s ON s.word_id=w.id WHERE COALESCE(s.suspended,0)=0 AND (s.due IS NULL OR s.due<=?1) ORDER BY CASE WHEN s.due IS NULL THEN 0 ELSE 1 END,s.due LIMIT ?2")?;
        let rows = st.query_map(params![now, limit as i64], row_word)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub fn recent_words(&self, limit: usize) -> Result<Vec<Word>> {
        let conn = self.open()?;
        let mut st=conn.prepare("SELECT w.id,w.word,COALESCE(w.us_phonetic,w.phonetic,''),COALESCE((SELECT chinese FROM meanings m WHERE m.word_id=w.id ORDER BY m.id LIMIT 1),''),COALESCE(s.state,'new'),s.due,COALESCE(s.mastery,0) FROM words w LEFT JOIN word_states s ON s.word_id=w.id ORDER BY w.id DESC LIMIT ?1")?;
        let rows = st.query_map(params![limit as i64], row_word)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    /// 已安装的 Electron 词典来源。数据直接读取兼容数据库，导入后立刻可见。
    pub fn dictionary_sources(&self) -> Result<Vec<DictionarySource>> {
        let conn = self.open()?;
        let mut statement = conn.prepare("SELECT book_id,name,COALESCE(category,''),COALESCE(level,3),COALESCE(word_count,0),imported_at,group_id FROM dictionary_sources ORDER BY level,name")?;
        let rows = statement.query_map([], |row| {
            Ok(DictionarySource {
                book_id: row.get(0)?,
                name: row.get(1)?,
                category: row.get(2)?,
                level: row.get(3)?,
                word_count: row.get::<_, i64>(4)?.max(0) as u64,
                imported_at: row.get(5)?,
                group_id: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    /// Electron 词库卡片的「练这本」把训练队列限制在来源分组内；原生端仍以
    /// 相同的到期优先规则排序，只收窄候选词集合。
    pub fn due_words_in_group(&self, group_id: i64, limit: usize) -> Result<Vec<Word>> {
        let conn = self.open()?;
        let now = Utc::now().to_rfc3339();
        let mut st = conn.prepare("SELECT w.id,w.word,COALESCE(w.us_phonetic,w.phonetic,''),COALESCE((SELECT chinese FROM meanings m WHERE m.word_id=w.id ORDER BY m.id LIMIT 1),''),COALESCE(s.state,'new'),s.due,COALESCE(s.mastery,0) FROM group_items gi JOIN words w ON w.id=gi.item_id LEFT JOIN word_states s ON s.word_id=w.id WHERE gi.group_id=?1 AND gi.item_type='word' AND COALESCE(s.suspended,0)=0 AND (s.due IS NULL OR s.due<=?2) ORDER BY CASE WHEN s.due IS NULL THEN 0 ELSE 1 END,s.due LIMIT ?3")?;
        let rows = st.query_map(params![group_id, now, limit as i64], row_word)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    /// Electron「导入自定义词表」的同一行格式：`word, 释义`、`word\t释义`、
    /// `word; 释义`、`word|释义` 或 `word 释义`。不合格式的行会被忽略，
    /// 若整份内容没有有效词条则返回可显示的错误。
    pub fn import_word_list(&self, name: &str, contents: &str) -> Result<usize> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("词表名称不能为空");
        }
        let entries = contents
            .lines()
            .filter_map(parse_word_list_line)
            .collect::<Vec<_>>();
        if entries.is_empty() {
            anyhow::bail!("没有解析出词条；每行应为“英文单词, 中文释义”");
        }
        let mut conn = self.open()?;
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        // 不合并同名分组：Electron 每次导入都会新建一份用户分组，避免后一次
        // 导入意外改写已有训练范围。
        tx.execute(
            "INSERT INTO groups_data(name,type,created_time) VALUES(?1,'user',?2)",
            params![name, now],
        )?;
        let group_id = tx.last_insert_rowid();
        for (word, meaning) in &entries {
            tx.execute(
                "INSERT INTO words(word,part_of_speech,difficulty_level,created_time) VALUES(?1,'',3,?2) ON CONFLICT(word) DO NOTHING",
                params![word, Utc::now().to_rfc3339()],
            )?;
            let word_id: i64 =
                tx.query_row("SELECT id FROM words WHERE word=?1", params![word], |row| {
                    row.get(0)
                })?;
            tx.execute(
                "INSERT INTO meanings(word_id,chinese,frequency_level,source) SELECT ?1,?2,1,'custom' WHERE NOT EXISTS(SELECT 1 FROM meanings WHERE word_id=?1 AND chinese=?2)",
                params![word_id, meaning],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO group_items(group_id,item_type,item_id) VALUES(?1,'word',?2)",
                params![group_id, word_id],
            )?;
            tx.execute(
                "INSERT INTO word_states(word_id,state) VALUES(?1,'new') ON CONFLICT(word_id) DO NOTHING",
                params![word_id],
            )?;
        }
        tx.commit()?;
        Ok(entries.len())
    }
    pub fn custom_word_lists(&self) -> Result<Vec<CustomWordList>> {
        let conn = self.open()?;
        let mut statement = conn.prepare(
            "SELECT g.id,g.name,COUNT(gi.item_id) FROM groups_data g LEFT JOIN group_items gi ON gi.group_id=g.id AND gi.item_type='word' WHERE g.type='user' GROUP BY g.id,g.name ORDER BY g.id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CustomWordList {
                id: row.get(0)?,
                name: row.get(1)?,
                word_count: row.get::<_, i64>(2)?.max(0) as u64,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    /// 与 Electron「移除词库」保持相同的保留规则：已练习过或同时属于其他词书
    /// 的词永远不删，只清理由此词典独占且未产生训练记录的数据。
    pub fn dictionary_removal_plan(&self, book_id: &str) -> Result<DictionaryRemoval> {
        let conn = self.open()?;
        let (id, name, total): (i64, String, i64) = conn
            .query_row(
                "SELECT id,name,word_count FROM dictionary_sources WHERE book_id=?1",
                params![book_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .context("词典不存在")?;
        let removable = count(&conn, "SELECT COUNT(*) FROM word_sources ws WHERE ws.source_id=?1 AND NOT EXISTS(SELECT 1 FROM word_sources other WHERE other.word_id=ws.word_id AND other.source_id<>ws.source_id) AND NOT EXISTS(SELECT 1 FROM test_records r WHERE r.word_id=ws.word_id)", [id])?;
        Ok(DictionaryRemoval {
            name,
            removable_words: removable,
            retained_words: (total.max(0) as u64).saturating_sub(removable),
        })
    }
    pub fn remove_dictionary(&self, book_id: &str) -> Result<DictionaryRemoval> {
        let plan = self.dictionary_removal_plan(book_id)?;
        let mut conn = self.open()?;
        let tx = conn.transaction()?;
        let (source_id, group_id): (i64, Option<i64>) = tx.query_row(
            "SELECT id,group_id FROM dictionary_sources WHERE book_id=?1",
            params![book_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut statement = tx.prepare("SELECT ws.word_id FROM word_sources ws WHERE ws.source_id=?1 AND NOT EXISTS(SELECT 1 FROM word_sources other WHERE other.word_id=ws.word_id AND other.source_id<>ws.source_id) AND NOT EXISTS(SELECT 1 FROM test_records r WHERE r.word_id=ws.word_id)")?;
        let removable = statement
            .query_map(params![source_id], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for word_id in removable {
            for table in [
                "meanings",
                "phrases",
                "word_relations",
                "sentence_words",
                "word_states",
                "word_notes",
            ] {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE word_id=?1"),
                    params![word_id],
                )?;
            }
            tx.execute(
                "DELETE FROM group_items WHERE item_type='word' AND item_id=?1",
                params![word_id],
            )?;
            tx.execute("DELETE FROM words WHERE id=?1", params![word_id])?;
        }
        tx.execute(
            "DELETE FROM word_sources WHERE source_id=?1",
            params![source_id],
        )?;
        tx.execute("DELETE FROM sentences WHERE source=?1 AND NOT EXISTS(SELECT 1 FROM sentence_words sw WHERE sw.sentence_id=sentences.id)", params![book_id])?;
        tx.execute(
            "DELETE FROM dictionary_sources WHERE id=?1",
            params![source_id],
        )?;
        if let Some(group_id) = group_id {
            tx.execute("DELETE FROM groups_data WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM group_items WHERE group_id=?1)", params![group_id])?;
        }
        tx.commit()?;
        Ok(plan)
    }
    pub fn recent_reviews(&self, limit: usize) -> Result<Vec<ReviewRecord>> {
        let conn = self.open()?;
        let mut statement = conn.prepare("SELECT r.id,COALESCE(NULLIF(w.word,''),NULLIF(r.question_content,''),'听写'),COALESCE(NULLIF((SELECT chinese FROM meanings m WHERE m.word_id=r.word_id ORDER BY m.id LIMIT 1),''),r.correct_answer,''),COALESCE(r.question_type,'recognize'),COALESCE(r.is_correct,0),r.rating,r.create_time FROM test_records r LEFT JOIN words w ON w.id=r.word_id ORDER BY r.id DESC LIMIT ?1")?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(ReviewRecord {
                id: row.get(0)?,
                word: row.get(1)?,
                meaning: row.get(2)?,
                question_type: row.get(3)?,
                correct: row.get(4)?,
                rating: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    /// 统计页所需的可核验聚合，避免 UI 自己猜测学习状态。
    pub fn learning_stats(&self) -> Result<LearningStats> {
        let conn = self.open()?;
        let (total_answers, correct_answers, total_seconds):(i64,i64,i64) = conn.query_row("SELECT COUNT(*),COALESCE(SUM(CASE WHEN is_correct>=1 THEN 1 ELSE 0 END),0),COALESCE(SUM(COALESCE(elapsed_ms,cost_time,0))/1000,0) FROM test_records", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        let mastered_words = count(
            &conn,
            "SELECT COUNT(*) FROM word_states WHERE mastery>=80",
            [],
        )?;
        let learning_words = count(
            &conn,
            "SELECT COUNT(*) FROM word_states WHERE state IN ('new','learning','relearning')",
            [],
        )?;
        let active_days = count(
            &conn,
            "SELECT COUNT(DISTINCT date(create_time)) FROM test_records",
            [],
        )?;
        Ok(LearningStats {
            total_answers: total_answers.max(0) as u64,
            correct_answers: correct_answers.max(0) as u64,
            mastered_words,
            learning_words,
            active_days,
            total_seconds: total_seconds.max(0) as u64,
        })
    }
    /// 阅读训练和 Electron 共用 `articles` 表；不迁移、不复制旧用户文章。
    pub fn articles(&self, limit: usize) -> Result<Vec<Article>> {
        let conn = self.open()?;
        let mut statement = conn.prepare("SELECT id,title,content,COALESCE(translation,''),COALESCE(difficulty,3),COALESCE(source,'') FROM articles ORDER BY id DESC LIMIT ?1")?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(Article {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                translation: row.get(3)?,
                difficulty: row.get(4)?,
                source: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub fn article(&self, id: i64) -> Result<Article> {
        let conn = self.open()?;
        conn.query_row("SELECT id,title,content,COALESCE(translation,''),COALESCE(difficulty,3),COALESCE(source,'') FROM articles WHERE id=?1", params![id], |row| Ok(Article { id: row.get(0)?, title: row.get(1)?, content: row.get(2)?, translation: row.get(3)?, difficulty: row.get(4)?, source: row.get(5)? }))
            .context("阅读文章不存在")
    }
    pub fn add_article(
        &self,
        title: &str,
        content: &str,
        translation: &str,
        difficulty: i64,
        source: &str,
    ) -> Result<i64> {
        let title = title.trim();
        let content = content.trim();
        anyhow::ensure!(!title.is_empty(), "文章标题不能为空");
        anyhow::ensure!(!content.is_empty(), "文章正文不能为空");
        let conn = self.open()?;
        conn.execute("INSERT INTO articles(title,content,translation,difficulty,source,created_time) VALUES(?1,?2,?3,?4,?5,?6)", params![title,content,translation.trim(),difficulty.clamp(1,5),source.trim(),Utc::now().to_rfc3339()])?;
        Ok(conn.last_insert_rowid())
    }
    pub fn sentences(&self, limit: usize) -> Result<Vec<Sentence>> {
        let conn = self.open()?;
        let mut statement = conn.prepare("SELECT id,content,COALESCE(translation,''),COALESCE(source,''),COALESCE(difficulty,3) FROM sentences ORDER BY id DESC LIMIT ?1")?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(Sentence {
                id: row.get(0)?,
                content: row.get(1)?,
                translation: row.get(2)?,
                source: row.get(3)?,
                difficulty: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub fn add_sentence(
        &self,
        content: &str,
        translation: &str,
        difficulty: i64,
        source: &str,
    ) -> Result<i64> {
        let content = content.trim();
        anyhow::ensure!(!content.is_empty(), "听力句子不能为空");
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO sentences(content,translation,source,difficulty) VALUES(?1,?2,?3,?4)",
            params![
                content,
                translation.trim(),
                source.trim(),
                difficulty.clamp(1, 5)
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }
    /// 保存一次听写，记录格式和旧版 `test_records` 保持一致；句子没有 word_id，
    /// 所以把题目和标准答案写入其原本就提供的通用字段。
    pub fn grade_dictation(
        &self,
        sentence_id: i64,
        answer: &str,
        elapsed_ms: u64,
    ) -> Result<DictationResult> {
        let conn = self.open()?;
        let expected: String = conn
            .query_row(
                "SELECT content FROM sentences WHERE id=?1",
                params![sentence_id],
                |row| row.get(0),
            )
            .context("听力句子不存在")?;
        let expected_tokens = dictation_tokens(&expected);
        let answer_tokens = dictation_tokens(answer);
        let matched = lcs_len(&expected_tokens, &answer_tokens);
        let score = if expected_tokens.is_empty() {
            0.0
        } else {
            matched as f64 / expected_tokens.len() as f64
        };
        let exact = score >= 0.999_999 && expected_tokens.len() == answer_tokens.len();
        conn.execute("INSERT INTO test_records(question_type,question_content,correct_answer,user_answer,is_correct,cost_time,elapsed_ms,mode,create_time) VALUES('sentence_dictation',?1,?2,?3,?4,?5,?6,'listening',?7)", params!["句子听写", expected, answer.trim(), score, elapsed_ms as i64, elapsed_ms as i64, Utc::now().to_rfc3339()])?;
        Ok(DictationResult {
            expected,
            score,
            exact,
        })
    }
    pub fn add_word(&self, word: &str, meaning: &str) -> Result<i64> {
        let word = word.trim().to_lowercase();
        let conn = self.open()?;
        let stamp = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO words(word,created_time) VALUES(?1,?2) ON CONFLICT(word) DO NOTHING",
            params![word, stamp],
        )?;
        let id: i64 = conn.query_row("SELECT id FROM words WHERE word=?1", params![word], |r| {
            r.get(0)
        })?;
        if !meaning.trim().is_empty() {
            conn.execute("INSERT INTO meanings(word_id,chinese) SELECT ?1,?2 WHERE NOT EXISTS(SELECT 1 FROM meanings WHERE word_id=?1 AND chinese=?2)",params![id,meaning.trim()])?;
        }
        conn.execute("INSERT INTO word_states(word_id,state,due,first_seen) VALUES(?1,'new',NULL,NULL) ON CONFLICT(word_id) DO NOTHING",params![id])?;
        Ok(id)
    }
    /// 导入 Electron English Lab 使用的 schemaVersion 2 词典包（`*_2.json`）。
    /// 同一 bookId 重复导入时更新来源信息，并按词条去重，不会清空学习进度。
    pub fn import_dictionary_file(&self, path: impl AsRef<Path>) -> Result<usize> {
        let raw = std::fs::read(path.as_ref())
            .with_context(|| format!("读取词典包失败：{}", path.as_ref().display()))?;
        let pack: serde_json::Value =
            serde_json::from_slice(&raw).context("词典包不是有效 JSON")?;
        let book_id = pack
            .get("bookId")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .context("词典包缺少 bookId")?;
        let name = pack.get("name").and_then(|v| v.as_str()).unwrap_or(book_id);
        let category = pack.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let level = pack.get("level").and_then(|v| v.as_i64()).unwrap_or(3);
        let mut conn = self.open()?;
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO dictionary_sources(book_id,name,imported_at,word_count,category,level) VALUES(?1,?2,?3,0,?4,?5) ON CONFLICT(book_id) DO UPDATE SET name=excluded.name,imported_at=excluded.imported_at,category=excluded.category,level=excluded.level",params![book_id,name,Utc::now().to_rfc3339(),category,level])?;
        let source_id: i64 = tx.query_row(
            "SELECT id FROM dictionary_sources WHERE book_id=?1",
            params![book_id],
            |row| row.get(0),
        )?;
        // Electron 为每本词典建立一个可训练分组；保留这一层关系，才能让导入后
        // 的词、例句、短语和关联词继续被同一词书筛选。
        tx.execute(
            // Electron 建的已安装词包都是 `system` 分组。这里只按名称匹配，
            // 早期原生导入（`dictionary`）建立的分组才能被复用，
            // 而不是悄悄多出第二个训练范围。
            "INSERT INTO groups_data(name,type,created_time) SELECT ?1,'system',?2 WHERE NOT EXISTS(SELECT 1 FROM groups_data WHERE name=?1)",
            params![name, Utc::now().to_rfc3339()],
        )?;
        let group_id: i64 = tx.query_row(
            "SELECT id FROM groups_data WHERE name=?1 ORDER BY id LIMIT 1",
            params![name],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE dictionary_sources SET group_id=?1 WHERE id=?2",
            params![group_id, source_id],
        )?;
        let mut imported = 0usize;
        for row in pack
            .get("words")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let word = row
                .get("w")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            if word.is_empty() {
                continue;
            }
            let phonetic = row.get("ph").and_then(|v| v.as_str());
            let uk = row.get("uk").and_then(|v| v.as_str());
            let us = row.get("us").and_then(|v| v.as_str());
            let memory = row.get("mem").and_then(|v| v.as_str());
            let rank = row.get("rank").and_then(|v| v.as_i64()).unwrap_or(0);
            tx.execute("INSERT INTO words(word,phonetic,uk_phonetic,us_phonetic,memory_method,freq_rank,created_time) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(word) DO UPDATE SET phonetic=COALESCE(excluded.phonetic,words.phonetic),uk_phonetic=COALESCE(excluded.uk_phonetic,words.uk_phonetic),us_phonetic=COALESCE(excluded.us_phonetic,words.us_phonetic),memory_method=COALESCE(excluded.memory_method,words.memory_method),freq_rank=CASE WHEN words.freq_rank IS NULL OR words.freq_rank=0 THEN excluded.freq_rank ELSE MIN(words.freq_rank,excluded.freq_rank) END",params![word,phonetic,uk,us,memory,rank,Utc::now().to_rfc3339()])?;
            let word_id: i64 =
                tx.query_row("SELECT id FROM words WHERE word=?1", params![word], |row| {
                    row.get(0)
                })?;
            tx.execute("INSERT INTO word_sources(word_id,source_id,source_word_id,source_rank) VALUES(?1,?2,?3,?4) ON CONFLICT(word_id,source_id) DO UPDATE SET source_word_id=excluded.source_word_id,source_rank=excluded.source_rank",params![word_id,source_id,row.get("id").and_then(|v|v.as_str()),rank])?;
            tx.execute(
                "INSERT OR IGNORE INTO group_items(group_id,item_type,item_id) VALUES(?1,'word',?2)",
                params![group_id, word_id],
            )?;
            for meaning in row
                .get("m")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let chinese = meaning
                    .get("cn")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !chinese.is_empty() {
                    tx.execute("INSERT INTO meanings(word_id,chinese,english,part_of_speech,frequency_level,source) SELECT ?1,?2,?3,?4,?5,?6 WHERE NOT EXISTS(SELECT 1 FROM meanings WHERE word_id=?1 AND chinese=?2)",params![word_id,chinese,meaning.get("en").and_then(|v|v.as_str()),meaning.get("pos").and_then(|v|v.as_str()),meaning.get("f").and_then(|v|v.as_i64()).unwrap_or(1),book_id])?;
                }
            }
            for sentence in row
                .get("s")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let content = sentence
                    .get("en")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if content.is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT INTO sentences(content,translation,source,difficulty) SELECT ?1,?2,?3,?4 WHERE NOT EXISTS(SELECT 1 FROM sentences WHERE content=?1)",
                    params![content, sentence.get("cn").and_then(|v| v.as_str()).unwrap_or(""), book_id, level],
                )?;
                let sentence_id: i64 = tx.query_row(
                    "SELECT id FROM sentences WHERE content=?1 ORDER BY id LIMIT 1",
                    params![content],
                    |row| row.get(0),
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO sentence_words(sentence_id,word_id) VALUES(?1,?2)",
                    params![sentence_id, word_id],
                )?;
            }
            for phrase in row
                .get("p")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let content = phrase
                    .get("en")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !content.is_empty() {
                    tx.execute(
                        "INSERT OR IGNORE INTO phrases(word_id,content,translation,source) VALUES(?1,?2,?3,?4)",
                        params![word_id, content, phrase.get("cn").and_then(|v| v.as_str()).unwrap_or(""), book_id],
                    )?;
                }
            }
            for relation in row
                .get("r")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let related = relation
                    .get("w")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !related.is_empty() {
                    tx.execute(
                        "INSERT OR IGNORE INTO word_relations(word_id,related_word,relation_type,part_of_speech,translation,source) VALUES(?1,?2,?3,?4,?5,?6)",
                        params![word_id, related, relation.get("t").and_then(|v| v.as_str()).unwrap_or("related"), relation.get("pos").and_then(|v| v.as_str()).unwrap_or(""), relation.get("cn").and_then(|v| v.as_str()).unwrap_or(""), book_id],
                    )?;
                }
            }
            tx.execute("INSERT INTO word_states(word_id,state) VALUES(?1,'new') ON CONFLICT(word_id) DO NOTHING",params![word_id])?;
            imported += 1;
        }
        tx.execute(
            "UPDATE dictionary_sources SET word_count=?1 WHERE id=?2",
            params![imported as i64, source_id],
        )?;
        tx.commit()?;
        Ok(imported)
    }
    /// 简化评分路径；完整 FSRS 调度使用 grade_fsrs。
    pub fn grade(&self, word_id: i64, grade: u8, elapsed_ms: u64) -> Result<()> {
        let conn = self.open()?;
        let now = Utc::now();
        let score = match grade {
            1 => 0.0,
            2 => 0.6,
            _ => 1.0,
        };
        let (reps, stability, difficulty, lapses): (i64, f64, f64, i64) = conn
            .query_row(
                "SELECT reps,stability,difficulty,lapses FROM word_states WHERE word_id=?1",
                params![word_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .unwrap_or((0, 0.0, 5.0, 0));
        let next_reps = reps + 1;
        let next_stability = if reps == 0 {
            [0.4872, 1.4003, 3.7145, 13.8206][grade.saturating_sub(1).min(3) as usize]
        } else {
            (stability.max(0.1)
                * (if grade == 1 {
                    0.55
                } else if grade == 2 {
                    1.3
                } else if grade == 3 {
                    2.0
                } else {
                    2.8
                }))
            .max(0.1)
        };
        let days = ((next_stability * 1.9).round() as i64).clamp(1, 365);
        let due = (now + chrono::Duration::days(days)).to_rfc3339();
        let mastery = ((next_stability / (next_stability + 20.0)) * 100.0).min(100.0);
        conn.execute("INSERT INTO word_states(word_id,state,due,stability,difficulty,reps,lapses,last_review,last_interval,mastery,total_answers,total_score,avg_ms,first_seen) VALUES(?1,'review',?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?11,?7) ON CONFLICT(word_id) DO UPDATE SET state='review',due=excluded.due,stability=excluded.stability,difficulty=excluded.difficulty,reps=excluded.reps,lapses=excluded.lapses,last_review=excluded.last_review,last_interval=excluded.last_interval,mastery=excluded.mastery,total_answers=word_states.total_answers+1,total_score=word_states.total_score+excluded.total_score,avg_ms=CASE WHEN word_states.avg_ms=0 THEN excluded.avg_ms ELSE (word_states.avg_ms*0.8+excluded.avg_ms*0.2) END,first_seen=COALESCE(word_states.first_seen,excluded.first_seen)",params![word_id,due,next_stability,difficulty,next_reps,lapses+if grade==1{1}else{0},now.to_rfc3339(),days,mastery,score,elapsed_ms as f64])?;
        conn.execute("INSERT INTO test_records(word_id,question_type,is_correct,cost_time,elapsed_ms,rating,create_time) VALUES(?1,'recognize',?2,?3,?4,?5,?6)",params![word_id,score,elapsed_ms as i64,elapsed_ms as i64,grade,now.to_rfc3339()])?;
        Ok(())
    }
    /// 与 Electron `core/srs.js` 相同的 FSRS 4.5 调度：四级评分、学习/重学
    /// 阶梯、稳定度、难度、间隔、漏答与熟练度均按同一公式更新。
    pub fn grade_fsrs(&self, word_id: i64, grade: u8, elapsed_ms: u64) -> Result<()> {
        let grade = grade.clamp(1, 4);
        let conn = self.open()?;
        let now = Utc::now();
        let mut state = conn.query_row(
            "SELECT state,due,stability,difficulty,reps,lapses,step,last_review,last_interval,mastery,total_answers,total_score,avg_ms,leech,suspended,first_seen FROM word_states WHERE word_id=?1",
            params![word_id], row_state,
        ).optional()?.unwrap_or_else(FsrsState::blank);
        let before = state.clone();
        schedule_fsrs(&mut state, grade, now);
        let score = match grade {
            1 => 0.0,
            2 => 0.6,
            _ => 1.0,
        };
        state.total_answers += 1;
        state.total_score += score;
        if elapsed_ms > 0 {
            state.avg_ms = if state.avg_ms <= 0.0 {
                elapsed_ms as f64
            } else {
                ((state.avg_ms * (state.total_answers - 1) as f64) + elapsed_ms as f64)
                    / state.total_answers as f64
            };
        }
        state.mastery = fsrs_mastery(&state, now);
        conn.execute("INSERT INTO word_states(word_id,state,due,stability,difficulty,reps,lapses,step,last_review,last_interval,mastery,total_answers,total_score,avg_ms,leech,suspended,first_seen) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) ON CONFLICT(word_id) DO UPDATE SET state=excluded.state,due=excluded.due,stability=excluded.stability,difficulty=excluded.difficulty,reps=excluded.reps,lapses=excluded.lapses,step=excluded.step,last_review=excluded.last_review,last_interval=excluded.last_interval,mastery=excluded.mastery,total_answers=excluded.total_answers,total_score=excluded.total_score,avg_ms=excluded.avg_ms,leech=excluded.leech,first_seen=COALESCE(word_states.first_seen,excluded.first_seen)", params![word_id,state.state,state.due,state.stability,state.difficulty,state.reps,state.lapses,state.step,state.last_review,state.last_interval,state.mastery,state.total_answers,state.total_score,state.avg_ms,state.leech,state.suspended,state.first_seen])?;
        conn.execute("INSERT INTO test_records(word_id,question_type,is_correct,cost_time,elapsed_ms,rating,interval_before,interval_after,state_before,create_time) VALUES(?1,'recognize',?2,?3,?4,?5,?6,?7,?8,?9)", params![word_id,score,elapsed_ms as i64,elapsed_ms as i64,grade,before.last_interval,state.last_interval,before.state,now.to_rfc3339()])?;
        Ok(())
    }
    fn migrate(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS words(id INTEGER PRIMARY KEY AUTOINCREMENT,word TEXT NOT NULL UNIQUE,phonetic TEXT,part_of_speech TEXT,difficulty_level INTEGER DEFAULT 3,created_time TEXT NOT NULL,uk_phonetic TEXT,us_phonetic TEXT,memory_method TEXT,freq_rank INTEGER); CREATE TABLE IF NOT EXISTS meanings(id INTEGER PRIMARY KEY AUTOINCREMENT,word_id INTEGER NOT NULL,chinese TEXT NOT NULL,english TEXT,frequency_level INTEGER DEFAULT 1,part_of_speech TEXT,source TEXT); CREATE TABLE IF NOT EXISTS sentences(id INTEGER PRIMARY KEY AUTOINCREMENT,content TEXT NOT NULL,translation TEXT,source TEXT,difficulty INTEGER DEFAULT 3); CREATE TABLE IF NOT EXISTS articles(id INTEGER PRIMARY KEY AUTOINCREMENT,title TEXT NOT NULL,content TEXT NOT NULL,translation TEXT,difficulty INTEGER DEFAULT 3,source TEXT,created_time TEXT NOT NULL); CREATE TABLE IF NOT EXISTS dictionary_sources(id INTEGER PRIMARY KEY AUTOINCREMENT,book_id TEXT NOT NULL UNIQUE,name TEXT NOT NULL,repository TEXT,upstream_commit TEXT,license_note TEXT,imported_at TEXT,word_count INTEGER DEFAULT 0,category TEXT,level INTEGER DEFAULT 3,group_id INTEGER); CREATE TABLE IF NOT EXISTS word_sources(word_id INTEGER NOT NULL,source_id INTEGER NOT NULL,source_word_id TEXT,source_rank INTEGER DEFAULT 0,PRIMARY KEY(word_id,source_id)); CREATE TABLE IF NOT EXISTS groups_data(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,type TEXT DEFAULT 'user',created_time TEXT NOT NULL); CREATE TABLE IF NOT EXISTS group_items(group_id INTEGER,item_type TEXT,item_id INTEGER,PRIMARY KEY(group_id,item_type,item_id)); CREATE TABLE IF NOT EXISTS training_sessions(id INTEGER PRIMARY KEY AUTOINCREMENT,config_id INTEGER,start_time TEXT,end_time TEXT,status TEXT,total_question INTEGER DEFAULT 0,correct_count REAL DEFAULT 0,content_type TEXT DEFAULT 'word'); CREATE TABLE IF NOT EXISTS test_records(id INTEGER PRIMARY KEY AUTOINCREMENT,session_id INTEGER,word_id INTEGER,meaning_id INTEGER,question_type TEXT,question_content TEXT,correct_answer TEXT,user_answer TEXT,cost_time INTEGER,is_correct REAL,ai_hard_label TEXT,ai_soft_label TEXT,user_label TEXT,user_note TEXT,create_time TEXT NOT NULL,rating INTEGER,elapsed_ms INTEGER DEFAULT 0,mode TEXT DEFAULT 'word',interval_before REAL DEFAULT 0,interval_after REAL DEFAULT 0,state_before TEXT); CREATE TABLE IF NOT EXISTS app_settings(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE IF NOT EXISTS word_states(word_id INTEGER PRIMARY KEY,state TEXT NOT NULL DEFAULT 'new',due TEXT,stability REAL DEFAULT 0,difficulty REAL DEFAULT 5,reps INTEGER DEFAULT 0,lapses INTEGER DEFAULT 0,step INTEGER DEFAULT 0,last_review TEXT,last_interval REAL DEFAULT 0,mastery REAL DEFAULT 0,total_answers INTEGER DEFAULT 0,total_score REAL DEFAULT 0,avg_ms INTEGER DEFAULT 0,leech INTEGER DEFAULT 0,suspended INTEGER DEFAULT 0,first_seen TEXT,confidence REAL DEFAULT 0,coverage REAL DEFAULT 0); CREATE TABLE IF NOT EXISTS daily_stats(day TEXT PRIMARY KEY,answers INTEGER DEFAULT 0,score REAL DEFAULT 0,new_words INTEGER DEFAULT 0,reviews INTEGER DEFAULT 0,seconds INTEGER DEFAULT 0,sessions INTEGER DEFAULT 0); CREATE TABLE IF NOT EXISTS word_notes(word_id INTEGER PRIMARY KEY,note TEXT,starred INTEGER DEFAULT 0,updated_time TEXT); CREATE TABLE IF NOT EXISTS reading_sessions(id INTEGER PRIMARY KEY AUTOINCREMENT,article_id INTEGER,start_time TEXT,end_time TEXT,seconds INTEGER DEFAULT 0,words_read INTEGER DEFAULT 0,wpm INTEGER DEFAULT 0,sentences_done INTEGER DEFAULT 0,lookups INTEGER DEFAULT 0); CREATE TABLE IF NOT EXISTS article_questions(id INTEGER PRIMARY KEY AUTOINCREMENT,article_id INTEGER,question TEXT NOT NULL,answer TEXT,kind TEXT DEFAULT 'comprehension',created_time TEXT); CREATE TABLE IF NOT EXISTS article_words(article_id INTEGER NOT NULL,word_id INTEGER NOT NULL,surface TEXT,created_time TEXT,PRIMARY KEY(article_id,word_id)); CREATE TABLE IF NOT EXISTS word_skill(word_id INTEGER NOT NULL,facet TEXT NOT NULL,alpha REAL DEFAULT 0,beta REAL DEFAULT 0,samples INTEGER DEFAULT 0,ema_ms INTEGER DEFAULT 0,last_seen TEXT,PRIMARY KEY(word_id,facet)); CREATE TABLE IF NOT EXISTS word_trajectory(word_id INTEGER NOT NULL,day TEXT NOT NULL,mastery REAL DEFAULT 0,confidence REAL DEFAULT 0,coverage REAL DEFAULT 0,breadth REAL DEFAULT 0,facets TEXT,PRIMARY KEY(word_id,day)); CREATE TABLE IF NOT EXISTS learner_skill(scope TEXT NOT NULL,key TEXT NOT NULL,alpha REAL DEFAULT 0,beta REAL DEFAULT 0,samples INTEGER DEFAULT 0,ema_ms INTEGER DEFAULT 0,last_seen TEXT,PRIMARY KEY(scope,key)); CREATE TABLE IF NOT EXISTS facet_trajectory(day TEXT NOT NULL,facet TEXT NOT NULL,estimate REAL DEFAULT 0,confidence REAL DEFAULT 0,samples INTEGER DEFAULT 0,PRIMARY KEY(day,facet)); CREATE INDEX IF NOT EXISTS idx_states_due ON word_states(due); CREATE INDEX IF NOT EXISTS idx_words_word ON words(word);")?;
        // 这三张表原本就在 Electron 的数据结构中，但早期 Rust 迁移时漏掉了。
        // `IF NOT EXISTS` 让这条语句对全新工作区和任一宿主生成的库都安全。
        conn.execute_batch("CREATE TABLE IF NOT EXISTS sentence_words(sentence_id INTEGER,word_id INTEGER,PRIMARY KEY(sentence_id,word_id)); CREATE TABLE IF NOT EXISTS phrases(id INTEGER PRIMARY KEY AUTOINCREMENT,word_id INTEGER NOT NULL,content TEXT NOT NULL,translation TEXT,source TEXT,UNIQUE(word_id,content)); CREATE TABLE IF NOT EXISTS word_relations(id INTEGER PRIMARY KEY AUTOINCREMENT,word_id INTEGER NOT NULL,related_word TEXT NOT NULL,relation_type TEXT NOT NULL,part_of_speech TEXT,translation TEXT,source TEXT,UNIQUE(word_id,related_word,relation_type)); CREATE INDEX IF NOT EXISTS idx_sentence_words_word ON sentence_words(word_id);")?;
        for (table, column, definition) in [
            ("words", "uk_phonetic", "TEXT"),
            ("words", "us_phonetic", "TEXT"),
            ("words", "memory_method", "TEXT"),
            ("words", "freq_rank", "INTEGER"),
            ("meanings", "part_of_speech", "TEXT"),
            ("meanings", "source", "TEXT"),
            ("test_records", "rating", "INTEGER"),
            ("test_records", "elapsed_ms", "INTEGER DEFAULT 0"),
            ("test_records", "mode", "TEXT DEFAULT 'word'"),
            ("test_records", "interval_before", "REAL DEFAULT 0"),
            ("test_records", "interval_after", "REAL DEFAULT 0"),
            ("test_records", "state_before", "TEXT"),
            ("word_states", "confidence", "REAL DEFAULT 0"),
            ("word_states", "coverage", "REAL DEFAULT 0"),
        ] {
            ensure_column(conn, table, column, definition)?;
        }
        Ok(())
    }
}

fn dictation_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '\'' | '’' | '–' | '-') {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 最长公共子序列具备与旧 JavaScript 对齐器相同的有用性质：
/// 漏掉一个词，不会让它后面的词全部错位。
fn parse_word_list_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let split = line
        .char_indices()
        // 释义里的中文标点要保留。Electron 只把下面这些 ASCII 分隔符
        // 当显式切分点；`实施；引导` 不能再按它解释性的分号拆开。
        .find(|(_, ch)| matches!(ch, ',' | '，' | '\t' | ';' | '|'))
        .map(|(index, ch)| (index, ch.len_utf8()))
        .or_else(|| {
            line.char_indices()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(index, ch)| (index, ch.len_utf8()))
        })?;
    let word = line[..split.0].trim().to_lowercase();
    let meaning = line[split.0 + split.1..].trim();
    // 与 Electron 的页面一致：词条以 ASCII 英文字母开头，
    // 词的部分可以包含空格、撇号和连字符。
    if word.is_empty()
        || meaning.is_empty()
        || !word.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || !word
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '\'' | '’' | '-' | ' '))
    {
        return None;
    }
    Some((word, meaning.to_owned()))
}

fn lcs_len(expected: &[String], actual: &[String]) -> usize {
    let mut row = vec![0usize; actual.len() + 1];
    for left in expected.iter().rev() {
        let mut next = row.clone();
        for (index, right) in actual.iter().enumerate().rev() {
            next[index] = if left == right {
                1 + row[index + 1]
            } else {
                row[index].max(next[index + 1])
            };
        }
        row = next;
    }
    row[0]
}
fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let found = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !found {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}
fn count<P: rusqlite::Params>(conn: &Connection, sql: &str, params: P) -> Result<u64> {
    let value: i64 = conn.query_row(sql, params, |row| row.get(0))?;
    Ok(value.max(0) as u64)
}
fn row_word(row: &rusqlite::Row<'_>) -> rusqlite::Result<Word> {
    Ok(Word {
        id: row.get(0)?,
        word: row.get(1)?,
        phonetic: row.get(2)?,
        meaning: row.get(3)?,
        state: row.get(4)?,
        due: row.get(5)?,
        mastery: row.get(6)?,
    })
}

#[derive(Clone)]
struct FsrsState {
    state: String,
    due: Option<String>,
    stability: f64,
    difficulty: f64,
    reps: i64,
    lapses: i64,
    step: i64,
    last_review: Option<String>,
    last_interval: f64,
    mastery: f64,
    total_answers: i64,
    total_score: f64,
    avg_ms: f64,
    leech: i64,
    suspended: i64,
    first_seen: Option<String>,
}
impl FsrsState {
    fn blank() -> Self {
        Self {
            state: "new".into(),
            due: None,
            stability: 0.0,
            difficulty: 0.0,
            reps: 0,
            lapses: 0,
            step: 0,
            last_review: None,
            last_interval: 0.0,
            mastery: 0.0,
            total_answers: 0,
            total_score: 0.0,
            avg_ms: 0.0,
            leech: 0,
            suspended: 0,
            first_seen: None,
        }
    }
}
fn row_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<FsrsState> {
    Ok(FsrsState {
        state: row.get(0)?,
        due: row.get(1)?,
        stability: row.get(2)?,
        difficulty: row.get(3)?,
        reps: row.get(4)?,
        lapses: row.get(5)?,
        step: row.get(6)?,
        last_review: row.get(7)?,
        last_interval: row.get(8)?,
        mastery: row.get(9)?,
        total_answers: row.get(10)?,
        total_score: row.get(11)?,
        avg_ms: row.get(12)?,
        leech: row.get(13)?,
        suspended: row.get(14)?,
        first_seen: row.get(15)?,
    })
}

const FSRS_W: [f64; 17] = [
    0.4872, 1.4003, 3.7145, 13.8206, 5.1618, 1.2298, 0.8975, 0.031, 1.6474, 0.1367, 1.0461, 2.1072,
    0.0793, 0.3246, 1.587, 0.2272, 2.8755,
];
const FSRS_DECAY: f64 = -0.5;
const FSRS_FACTOR: f64 = 19.0 / 81.0;
fn clamp(v: f64, min: f64, max: f64) -> f64 {
    v.max(min).min(max)
}
fn initial_stability(grade: u8) -> f64 {
    FSRS_W[(grade - 1) as usize].max(0.1)
}
fn initial_difficulty(grade: u8) -> f64 {
    clamp(FSRS_W[4] - (grade as f64 - 3.0) * FSRS_W[5], 1.0, 10.0)
}
fn next_difficulty(difficulty: f64, grade: u8) -> f64 {
    clamp(
        FSRS_W[7] * FSRS_W[4] + (1.0 - FSRS_W[7]) * (difficulty - FSRS_W[6] * (grade as f64 - 3.0)),
        1.0,
        10.0,
    )
}
fn retrievability(stability: f64, elapsed_days: f64) -> f64 {
    if stability <= 0.0 {
        0.0
    } else {
        (1.0 + FSRS_FACTOR * elapsed_days.max(0.0) / stability).powf(FSRS_DECAY)
    }
}
fn elapsed_days(state: &FsrsState, at: chrono::DateTime<Utc>) -> f64 {
    state
        .last_review
        .as_deref()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|then| {
            ((at.timestamp_millis() - then.timestamp_millis()).max(0) as f64) / 86_400_000.0
        })
        .unwrap_or(0.0)
}
fn interval_for(stability: f64) -> f64 {
    clamp(
        ((stability / FSRS_FACTOR) * (0.9f64.powf(1.0 / FSRS_DECAY) - 1.0)).round(),
        1.0,
        365.0,
    )
}
fn schedule_fsrs(next: &mut FsrsState, grade: u8, at: chrono::DateTime<Utc>) {
    let previous = next.clone();
    next.reps += 1;
    next.last_review = Some(at.to_rfc3339());
    let graduate = |state: &mut FsrsState, stability: f64, difficulty: f64| {
        let interval = interval_for(stability);
        state.state = "review".into();
        state.step = 0;
        state.stability = stability;
        state.difficulty = difficulty;
        state.last_interval = interval;
        state.due =
            Some((at + chrono::Duration::seconds((interval * 86400.0) as i64)).to_rfc3339());
    };
    let park = |state: &mut FsrsState, days: f64, phase: &str, step: i64| {
        state.state = phase.into();
        state.step = step;
        state.last_interval = days;
        state.due = Some((at + chrono::Duration::seconds((days * 86400.0) as i64)).to_rfc3339());
    };
    if previous.state == "new" || previous.reps == 0 {
        next.first_seen = previous.first_seen.or_else(|| Some(at.to_rfc3339()));
        next.stability = initial_stability(grade);
        next.difficulty = initial_difficulty(grade);
        if grade == 4 {
            graduate(next, next.stability, next.difficulty)
        } else if grade == 1 {
            park(next, 1.0 / 1440.0, "learning", 0)
        } else if grade == 2 {
            park(next, 1.5 / 1440.0, "learning", 0)
        } else {
            park(next, 10.0 / 1440.0, "learning", 1)
        }
        return;
    }
    if previous.state == "learning" || previous.state == "relearning" {
        let ladder = if previous.state == "learning" {
            [1.0 / 1440.0, 10.0 / 1440.0].as_slice()
        } else {
            [10.0 / 1440.0].as_slice()
        };
        next.difficulty = next_difficulty(
            if previous.difficulty > 0.0 {
                previous.difficulty
            } else {
                initial_difficulty(3)
            },
            grade,
        );
        if grade == 1 {
            park(next, ladder[0], &previous.state, 0)
        } else if grade == 2 {
            park(
                next,
                ladder
                    .get(previous.step as usize)
                    .copied()
                    .unwrap_or(ladder[0])
                    * 1.5,
                &previous.state,
                previous.step,
            )
        } else if grade == 4 {
            graduate(
                next,
                next.stability.max(initial_stability(4)),
                next.difficulty,
            )
        } else {
            let step = previous.step + 1;
            if (step as usize) < ladder.len() {
                park(next, ladder[step as usize], &previous.state, step)
            } else {
                graduate(
                    next,
                    previous.stability.max(initial_stability(3)),
                    next.difficulty,
                )
            }
        }
        return;
    }
    let r = retrievability(previous.stability, elapsed_days(&previous, at));
    next.difficulty = next_difficulty(
        if previous.difficulty > 0.0 {
            previous.difficulty
        } else {
            initial_difficulty(3)
        },
        grade,
    );
    if grade == 1 {
        next.lapses += 1;
        next.stability = (FSRS_W[11]
            * next.difficulty.powf(-FSRS_W[12])
            * ((previous.stability + 1.0).powf(FSRS_W[13]) - 1.0)
            * (FSRS_W[14] * (1.0 - r)).exp())
        .min(previous.stability)
        .max(0.1);
        if next.lapses >= 6 {
            next.leech = 1;
        }
        park(next, 10.0 / 1440.0, "relearning", 0)
    } else {
        let hard = if grade == 2 { FSRS_W[15] } else { 1.0 };
        let easy = if grade == 4 { FSRS_W[16] } else { 1.0 };
        next.stability = (previous.stability.max(initial_stability(grade))
            * (1.0
                + (FSRS_W[8].exp()
                    * (11.0 - next.difficulty)
                    * previous
                        .stability
                        .max(initial_stability(grade))
                        .powf(-FSRS_W[9])
                    * ((FSRS_W[10] * (1.0 - r)).exp() - 1.0)
                    * hard
                    * easy)))
            .max(0.1);
        graduate(next, next.stability, next.difficulty)
    }
}
fn fsrs_mastery(state: &FsrsState, at: chrono::DateTime<Utc>) -> f64 {
    let accuracy = if state.total_answers > 0 {
        state.total_score / state.total_answers as f64
    } else {
        0.0
    };
    let settled = 1.0 - (-state.stability / 25.0).exp();
    let lapse = (state.lapses as f64 * 0.03).min(0.18);
    clamp(
        ((0.55 * settled
            + 0.32 * accuracy
            + 0.13 * retrievability(state.stability, elapsed_days(state, at))
            - lapse)
            * 100.0)
            .round(),
        0.0,
        100.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opens_the_electron_database_location_and_preserves_words() {
        let root =
            std::env::temp_dir().join(format!("mochi-english-{}", crate::paths::random_base36(8)));
        let service = Service::new(&root);
        assert!(service
            .path()
            .ends_with(".mochi/extensions-data/english/default/data.db"));
        let id = service.add_word("abandon", "放弃").unwrap();
        service.grade_fsrs(id, 3, 800).unwrap();
        assert_eq!(service.dashboard().unwrap().words, 1);
        assert_eq!(service.recent_words(10).unwrap()[0].word, "abandon");
        let conn = service.open().unwrap();
        let (state, reps, interval): (String, i64, f64) = conn
            .query_row(
                "SELECT state,reps,last_interval FROM word_states WHERE word_id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "learning");
        assert_eq!(reps, 1);
        assert!((interval - 10.0 / 1440.0).abs() < 0.000_001);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn imports_electron_style_custom_word_lists_from_csv_tsv_and_plain_text() {
        let root = std::env::temp_dir().join(format!(
            "mochi-english-custom-{}",
            crate::paths::random_base36(8)
        ));
        let service = Service::new(&root);
        let count = service
            .import_word_list(
                "我的词表",
                "resilient, 有韧性的\nabandon\t放弃\nconduct 实施；引导\nnot-a-word\n",
            )
            .unwrap();
        assert_eq!(count, 3);
        let lists = service.custom_word_lists().unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].name, "我的词表");
        assert_eq!(lists[0].word_count, 3);
        let words = service.recent_words(10).unwrap();
        assert_eq!(words.len(), 3);
        assert!(words
            .iter()
            .any(|entry| entry.word == "resilient" && entry.meaning == "有韧性的"));
        assert!(words
            .iter()
            .any(|entry| entry.word == "conduct" && entry.meaning == "实施；引导"));
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn imports_electron_dictionary_pack_without_resetting_existing_progress() {
        let root = std::env::temp_dir().join(format!(
            "mochi-english-pack-{}",
            crate::paths::random_base36(8)
        ));
        let pack = root.join("pack.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&pack,r#"{"schemaVersion":2,"bookId":"TEST_2","name":"测试词库","category":"测试","level":2,"words":[{"id":"TEST_2_1","rank":1,"w":"apple","uk":"ˈæpəl","m":[{"cn":"苹果","en":"apple","pos":"n","f":1}],"s":[{"en":"An apple a day keeps the doctor away.","cn":"一天一苹果，医生远离我。"}],"p":[{"en":"apple pie","cn":"苹果派"}],"r":[{"w":"pear","t":"related","pos":"n","cn":"梨"}]}]}"#).unwrap();
        let service = Service::new(&root);
        assert_eq!(service.import_dictionary_file(&pack).unwrap(), 1);
        let id = service.recent_words(1).unwrap()[0].id;
        service.grade_fsrs(id, 3, 1).unwrap();
        assert_eq!(service.import_dictionary_file(&pack).unwrap(), 1);
        let sources = service.dictionary_sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].book_id, "TEST_2");
        let group_type: String = service
            .open()
            .unwrap()
            .query_row(
                "SELECT type FROM groups_data WHERE name='测试词库'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(group_type, "system");
        let group_id = sources[0]
            .group_id
            .expect("imported dictionary has a group");
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE word_states SET due=NULL WHERE word_id=?1",
                params![id],
            )
            .unwrap();
        assert_eq!(service.due_words_in_group(group_id, 10).unwrap().len(), 1);
        let reviews = service.recent_reviews(10).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].word, "apple");
        let stats = service.learning_stats().unwrap();
        assert_eq!(stats.total_answers, 1);
        assert_eq!(stats.learning_words, 1);
        let article_id = service
            .add_article(
                "A tiny article",
                "Mochi helps people learn.",
                "墨池帮助人们学习。",
                2,
                "test",
            )
            .unwrap();
        assert_eq!(service.articles(10).unwrap()[0].id, article_id);
        assert_eq!(service.article(article_id).unwrap().title, "A tiny article");
        let sentence_id = service
            .add_sentence("Practice makes progress.", "练习带来进步。", 2, "test")
            .unwrap();
        assert_eq!(service.sentences(10).unwrap()[0].id, sentence_id);
        let perfect = service
            .grade_dictation(sentence_id, "practice makes progress", 450)
            .unwrap();
        assert!(perfect.exact);
        assert_eq!(perfect.score, 1.0);
        let partial = service
            .grade_dictation(sentence_id, "practice creates progress", 450)
            .unwrap();
        assert!(!partial.exact);
        assert!((partial.score - 2.0 / 3.0).abs() < 0.000_001);
        let dictations: i64 = service
            .open()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM test_records WHERE mode='listening'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dictations, 2);
        let conn = service.open().unwrap();
        let reps: i64 = conn
            .query_row(
                "SELECT reps FROM word_states WHERE word_id=?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reps, 1);
        let (links, phrases, relations, grouped): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM sentence_words),(SELECT COUNT(*) FROM phrases),(SELECT COUNT(*) FROM word_relations),(SELECT COUNT(*) FROM group_items WHERE item_type='word')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!((links, phrases, relations, grouped), (1, 1, 1, 1));
        let plan = service.dictionary_removal_plan("TEST_2").unwrap();
        assert_eq!(plan.removable_words, 0, "已练习词必须保留");
        assert_eq!(plan.retained_words, 1);
        service.remove_dictionary("TEST_2").unwrap();
        assert!(service.dictionary_sources().unwrap().is_empty());
        assert_eq!(service.recent_words(10).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
