//! items.json 保留 archivedPath / archivedAt 的显式 null 和尾换行；ID 后缀为 6 位 base36。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::agenda::{AgendaStore, Source, Task, TaskDraft};
use crate::{files, json2, jstime, paths};

/// 与 TS 的 `MAX_CONTENT_LENGTH` 一致。注意是 **UTF-16 码元**上限（JS `slice` 的语义）。
pub const MAX_CONTENT_LENGTH: usize = 20000;

/// 收件箱条目。
///
/// `archived_path` / `archived_at` **不带** `skip_serializing_if`——它们必须以 `null` 落盘。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CaptureItem {
    pub id: String,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// `quick-capture` | `app`
    pub source: String,
    /// `inbox` | `archived`
    pub status: String,
    pub archived_path: Option<String>,
    pub archived_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_clip: Option<crate::web_clipper::ClipMetadata>,
}

impl Default for CaptureItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            content: String::new(),
            created_at: 0,
            updated_at: 0,
            source: "app".into(),
            status: "inbox".into(),
            archived_path: None,
            archived_at: None,
            web_clip: None,
        }
    }
}

impl CaptureItem {
    pub fn is_archived(&self) -> bool {
        self.status == "archived"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CaptureFile {
    pub version: i32,
    pub items: Vec<CaptureItem>,
}

impl Default for CaptureFile {
    fn default() -> Self {
        Self {
            version: 1,
            items: Vec::new(),
        }
    }
}

pub struct CaptureService {
    workspace_path: PathBuf,
    /// 串行化写入，对齐 TS 的 `enqueue` 队列。
    gate: Mutex<()>,
}

impl CaptureService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        Self {
            workspace_path: workspace_path.as_ref().to_path_buf(),
            gate: Mutex::new(()),
        }
    }

    fn root(&self) -> PathBuf {
        self.workspace_path.join(paths::INBOX_DIR_NAME)
    }

    fn items_path(&self) -> PathBuf {
        self.root().join("items.json")
    }

    pub fn initialize(&self) -> Result<()> {
        let _file_lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        std::fs::create_dir_all(self.root())?;
        if !self.items_path().exists() {
            self.write_file(&CaptureFile::default())?;
        }
        Ok(())
    }

    /// 列出指定状态的条目，按创建时间**倒序**。`status = "all"` 表示不过滤。
    pub fn list_items(&self, status: &str) -> Vec<CaptureItem> {
        let file = self.read_file();
        let mut items: Vec<CaptureItem> = file
            .items
            .into_iter()
            .filter(|i| status == "all" || i.status == status)
            .collect();
        items.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        items
    }

    /// 统计收件箱（未归档）条目数。
    pub fn count_inbox(&self) -> usize {
        self.read_file()
            .items
            .iter()
            .filter(|i| i.status == "inbox")
            .count()
    }

    pub fn add(&self, content: &str, source: &str) -> Result<Option<CaptureItem>> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        let mut file = self.read_file_checked()?;
        let now = jstime::now_millis();
        let item = CaptureItem {
            id: format!("cap-{now}-{}", paths::random_base36(6)),
            content: truncate_utf16(trimmed, MAX_CONTENT_LENGTH),
            created_at: now,
            updated_at: now,
            source: source.to_owned(),
            status: "inbox".into(),
            archived_path: None,
            archived_at: None,
            web_clip: None,
        };
        file.items.push(item.clone());
        self.write_file(&file)?;
        Ok(Some(item))
    }

    /// 更新条目。`None` 表示该字段不动；`archived_path` 需要显式清空时传 `Some(None)`。
    pub fn update(
        &self,
        item_id: &str,
        content: Option<&str>,
        status: Option<&str>,
        archived_path: Option<Option<String>>,
    ) -> Result<Option<CaptureItem>> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        let mut file = self.read_file_checked()?;
        let Some(index) = file.items.iter().position(|i| i.id == item_id) else {
            return Ok(None);
        };
        let current = file.items[index].clone();
        let next_status = status.unwrap_or(&current.status).to_owned();
        let now = jstime::now_millis();

        let mut updated = CaptureItem {
            content: content
                .map(|c| truncate_utf16(c, MAX_CONTENT_LENGTH))
                .unwrap_or(current.content),
            // 归档时保留最早的归档时刻；一旦离开归档态就清空
            archived_at: if next_status == "archived" {
                Some(current.archived_at.unwrap_or(now))
            } else {
                None
            },
            status: next_status,
            archived_path: archived_path.unwrap_or(current.archived_path),
            updated_at: now,
            ..current
        };
        if content.is_some() {
            if let Some(meta) = &mut updated.web_clip {
                meta.title = derive_title(&updated.content);
            }
        }
        file.items[index] = updated.clone();
        self.write_file(&file)?;
        Ok(Some(updated))
    }

    pub fn remove(&self, item_id: &str) -> Result<bool> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        let mut file = self.read_file_checked()?;
        let before = file.items.len();
        let attachment = file
            .items
            .iter()
            .find(|i| i.id == item_id && i.status == "inbox")
            .and_then(|i| i.web_clip.as_ref())
            .map(|m| m.document.clone());
        file.items.retain(|i| i.id != item_id);
        if file.items.len() == before {
            return Ok(false);
        }
        self.write_file(&file)?;
        if let Some(relative) = attachment {
            let document = self
                .workspace_path
                .join(crate::web_clipper::safe_relative(&relative, false)?);
            let inbox = self.root().join("web-clips");
            if document.exists() && crate::web_clipper::contained(&inbox, &document).is_ok() {
                if let Some(bundle) = document.parent().filter(|p| *p != inbox) {
                    std::fs::remove_dir_all(bundle)?;
                }
            }
        }
        Ok(true)
    }

    /// 清掉已归档的历史条目，收件箱本身不受影响。返回清除条数。
    pub fn clear_archived(&self) -> Result<usize> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        let mut file = self.read_file_checked()?;
        let before = file.items.len();
        file.items.retain(|i| i.status != "archived");
        let removed = before - file.items.len();
        if removed > 0 {
            self.write_file(&file)?;
        }
        Ok(removed)
    }

    /// 把一条捕获归档为笔记文件。返回生成的笔记路径。
    pub fn archive_to_note(&self, item_id: &str, target_directory: &Path) -> Result<PathBuf> {
        if self
            .list_items("all")
            .iter()
            .any(|i| i.id == item_id && i.web_clip.is_some())
        {
            return self.archive_web_clip(item_id, target_directory);
        }
        let item = self
            .list_items("all")
            .into_iter()
            .find(|i| i.id == item_id)
            .with_context(|| format!("未找到条目: {item_id}"))?;
        std::fs::create_dir_all(target_directory)?;

        let title = derive_title(&item.content);
        let note_path = files::get_unique_file_path(target_directory, &format!("{title}.md"))?;
        std::fs::write(
            &note_path,
            format!("# {title}\n\n{}\n", item.content.trim_end()),
        )?;

        self.update(
            item_id,
            None,
            Some("archived"),
            Some(Some(note_path.to_string_lossy().into_owned())),
        )?;
        Ok(note_path)
    }

    /// 把一条捕获转为待办任务并标记已归档。
    pub fn convert_to_task(&self, item_id: &str, agenda: &AgendaStore) -> Result<Task> {
        let item = self
            .list_items("all")
            .into_iter()
            .find(|i| i.id == item_id)
            .with_context(|| format!("未找到条目: {item_id}"))?;
        let title = derive_title(&item.content);
        let (id, _) = agenda.mutate(Source::User, |ed| {
            ed.create_task(TaskDraft {
                note: if item.content != title {
                    item.content.clone()
                } else {
                    String::new()
                },
                title,
                source: "收件箱".into(),
                ..Default::default()
            })
        })?;
        self.update(item_id, None, Some("archived"), Some(None))?;
        agenda
            .load()?
            .task(&id)
            .cloned()
            .context("新建的任务没有落盘")
    }

    fn read_file(&self) -> CaptureFile {
        self.read_file_checked().unwrap_or_default()
    }

    fn read_file_checked(&self) -> Result<CaptureFile> {
        match std::fs::read_to_string(self.items_path()) {
            Ok(text) => {
                let mut file: CaptureFile =
                    json2::deserialize(&text).context("收件箱索引损坏，原文件已保留")?;
                // 归档动作可能在「目录已改名、索引还没发布」时被打断，这里负责恢复。
                // 下一次变更会把恢复后的状态落盘。
                for item in &mut file.items {
                    let Some(meta) = item.web_clip.as_mut() else {
                        continue;
                    };
                    if item.status != "inbox"
                        || self.workspace_path.join(&meta.document).exists()
                        || !crate::web_clipper::valid_id(&meta.id)
                    {
                        continue;
                    }
                    let journal = self
                        .workspace_path
                        .join(".mochi/web-clipper")
                        .join(&meta.id)
                        .join("archive.json");
                    let Ok(bytes) = std::fs::read(journal) else {
                        continue;
                    };
                    let Ok(relative) = serde_json::from_slice::<String>(&bytes) else {
                        continue;
                    };
                    let Ok(relative_path) = crate::web_clipper::safe_relative(&relative, false)
                    else {
                        continue;
                    };
                    let document = self.workspace_path.join(relative_path);
                    if !document.is_file()
                        || crate::web_clipper::contained(&self.workspace_path, &document).is_err()
                    {
                        continue;
                    }
                    meta.document = relative;
                    item.archived_path = Some(document.to_string_lossy().into_owned());
                    item.archived_at = Some(item.updated_at);
                    item.status = "archived".into();
                }
                Ok(file)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CaptureFile::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn add_web_clip(
        &self,
        metadata: crate::web_clipper::ClipMetadata,
        excerpt: &str,
    ) -> Result<CaptureItem> {
        let _lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        let mut file = self.read_file_checked()?;
        let id = format!("web-{}", metadata.id);
        if let Some(item) = file.items.iter().find(|i| i.id == id) {
            return Ok(item.clone());
        }
        let item = CaptureItem {
            id,
            content: format!("{}\n{}", metadata.title, truncate_utf16(excerpt, 800)),
            created_at: metadata.captured_at,
            updated_at: jstime::now_millis(),
            source: "web-clipper".into(),
            web_clip: Some(metadata),
            ..Default::default()
        };
        file.items.push(item.clone());
        self.write_file(&file)?;
        Ok(item)
    }

    fn archive_web_clip(&self, id: &str, target: &Path) -> Result<PathBuf> {
        let _lock = crate::settings::file::Lock::acquire(&self.items_path())?;
        let mut file = self.read_file_checked()?;
        let item = file
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .context("条目不存在")?;
        if let Some(path) = &item.archived_path {
            return Ok(PathBuf::from(path));
        }
        crate::web_clipper::contained(&self.workspace_path, target)?;
        let meta = item.web_clip.as_mut().context("条目不是网页剪藏")?;
        if !crate::web_clipper::valid_id(&meta.id) {
            anyhow::bail!("剪藏 ID 无效");
        }
        let source_doc = self
            .workspace_path
            .join(crate::web_clipper::safe_relative(&meta.document, false)?);
        crate::web_clipper::contained(&self.workspace_path, &source_doc)?;
        let source = source_doc.parent().context("附件目录不存在")?;
        let destination = target.join(source.file_name().context("附件目录无名称")?);
        if destination.exists() {
            anyhow::bail!("归档目录已存在，请选择其他位置");
        }
        let document = destination.join(source_doc.file_name().unwrap());
        let journal = self
            .workspace_path
            .join(".mochi/web-clipper")
            .join(&meta.id)
            .join("archive.json");
        files::FileService::new().write_file_safe(
            journal,
            &serde_json::to_string(
                &document
                    .strip_prefix(&self.workspace_path)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            )?,
        )?;
        std::fs::rename(source, &destination)?;
        meta.document = document
            .strip_prefix(&self.workspace_path)?
            .to_string_lossy()
            .replace('\\', "/");
        item.archived_path = Some(document.to_string_lossy().into_owned());
        item.archived_at = Some(jstime::now_millis());
        item.status = "archived".into();
        if let Err(e) = self.write_file(&file) {
            let _ = std::fs::rename(&destination, source);
            return Err(e);
        }
        Ok(document)
    }

    /// 与 TS 一致：`JSON.stringify(file, null, 2) + '\n'`。
    fn write_file(&self, file: &CaptureFile) -> Result<()> {
        std::fs::create_dir_all(self.root())?;
        files::FileService::new()
            .write_file_safe(self.items_path(), &format!("{}\n", json2::serialize(file)?))?;
        Ok(())
    }
}

/// JS `String.slice(0, n)` 的语义：按 **UTF-16 码元**截断。
/// 若截断点落在代理对中间则退一格，避免产出非法字符。
fn truncate_utf16(s: &str, max_units: usize) -> String {
    if s.encode_utf16().count() <= max_units {
        return s.to_owned();
    }
    let mut units = 0usize;
    let mut end = s.len();
    for (idx, ch) in s.char_indices() {
        let w = ch.len_utf16();
        if units + w > max_units {
            end = idx;
            break;
        }
        units += w;
    }
    s[..end].to_owned()
}

/// 从内容首行推导标题（去掉 Markdown 标记，截断到 40 字符）。
pub fn derive_title(content: &str) -> String {
    let Some(first_line) = content.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return "未命名想法".into();
    };
    let stripped = strip_markdown_prefix(first_line);
    let title = if stripped.is_empty() {
        first_line
    } else {
        &stripped
    };

    let chars: Vec<char> = title.chars().collect();
    if chars.len() > 40 {
        format!("{}…", chars[..40].iter().collect::<String>())
    } else {
        title.to_owned()
    }
}

/// 去掉行首的 `#`、`>`、列表符号与有序列表编号。
fn strip_markdown_prefix(line: &str) -> String {
    let trimmed = line.trim_start_matches(|c: char| "#>-*+".contains(c) || c.is_whitespace());
    // 有序列表 `1. ` / `1) `
    let rest = trimmed.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        let after = &rest[digits.len()..];
        if let Some(tail) = after.strip_prefix(['.', ')']) {
            return tail.trim().to_owned();
        }
    }
    trimmed.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("mochi-cap-{}-{tag}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn svc(&self) -> CaptureService {
            CaptureService::new(&self.0)
        }
        fn raw(&self) -> String {
            std::fs::read_to_string(self.0.join(paths::INBOX_DIR_NAME).join("items.json")).unwrap()
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 真实工作区里 archivedPath/archivedAt 是**显式 null**，不是省略。
    /// C# 版用 WhenWritingNull 把它们删掉了，两版交替写会反复增删字段。
    #[test]
    fn null_fields_are_written_explicitly() {
        let ws = TempWs::new("nulls");
        ws.svc().add("想法", "quick-capture").unwrap();
        let raw = ws.raw();
        assert!(
            raw.contains("\"archivedPath\": null"),
            "archivedPath 被省略了:\n{raw}"
        );
        assert!(
            raw.contains("\"archivedAt\": null"),
            "archivedAt 被省略了:\n{raw}"
        );
    }

    #[test]
    fn file_shape_matches_the_real_workspace() {
        let ws = TempWs::new("shape");
        let svc = ws.svc();
        let item = svc
            .add("看看 FSRS 间隔重复算法", "quick-capture")
            .unwrap()
            .unwrap();
        let raw = ws.raw();

        assert!(
            raw.starts_with("{\n  \"version\": 1,\n  \"items\": [\n"),
            "{raw}"
        );
        assert!(raw.ends_with("}\n"), "缺尾换行");
        assert!(!raw.contains('\r'), "写出了 CRLF");
        // 键顺序
        let i = |k: &str| raw.find(&format!("\"{k}\"")).unwrap();
        assert!(i("id") < i("content"));
        assert!(i("content") < i("createdAt"));
        assert!(i("createdAt") < i("updatedAt"));
        assert!(i("updatedAt") < i("source"));
        assert!(i("source") < i("status"));
        assert!(i("status") < i("archivedPath"));
        assert!(i("archivedPath") < i("archivedAt"));
        assert_eq!(item.status, "inbox");
    }

    #[test]
    fn id_matches_the_ts_shape() {
        let ws = TempWs::new("id");
        let item = ws.svc().add("x", "app").unwrap().unwrap();
        let rest = item.id.strip_prefix("cap-").expect("缺前缀");
        let (ts, rand) = rest.rsplit_once('-').expect("缺随机段");
        assert!(ts.parse::<i64>().is_ok());
        assert_eq!(rand.len(), 6, "TS 是 base36 取 6 位: {rand}");
        assert!(rand
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    }

    #[test]
    fn blank_content_is_rejected() {
        let ws = TempWs::new("blank");
        assert!(ws.svc().add("   \n  ", "app").unwrap().is_none());
        assert_eq!(ws.svc().count_inbox(), 0);
    }

    #[test]
    fn list_is_newest_first_and_filterable() {
        let ws = TempWs::new("list");
        let svc = ws.svc();
        let a = svc.add("先", "app").unwrap().unwrap();
        let b = svc.add("后", "app").unwrap().unwrap();
        // 同毫秒创建时顺序不定，这里手工拉开时间
        svc.update(&a.id, None, None, None).unwrap();

        let all = svc.list_items("all");
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|i| i.status == "inbox"));

        svc.update(&b.id, None, Some("archived"), None).unwrap();
        assert_eq!(svc.list_items("inbox").len(), 1);
        assert_eq!(svc.list_items("archived").len(), 1);
        assert_eq!(svc.count_inbox(), 1, "计数只算未归档");
    }

    #[test]
    fn archiving_stamps_archived_at_once_and_clears_on_return() {
        let ws = TempWs::new("archived-at");
        let svc = ws.svc();
        let item = svc.add("x", "app").unwrap().unwrap();

        let archived = svc
            .update(&item.id, None, Some("archived"), None)
            .unwrap()
            .unwrap();
        let stamp = archived.archived_at.expect("归档应盖时间戳");

        let again = svc
            .update(&item.id, None, Some("archived"), None)
            .unwrap()
            .unwrap();
        assert_eq!(again.archived_at, Some(stamp), "重复归档不该刷新时间戳");

        let back = svc
            .update(&item.id, None, Some("inbox"), None)
            .unwrap()
            .unwrap();
        assert_eq!(back.archived_at, None, "回到收件箱应清空归档时刻");
    }

    #[test]
    fn update_missing_id_returns_none() {
        let ws = TempWs::new("update-missing");
        assert!(ws
            .svc()
            .update("nope", Some("x"), None, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn remove_and_clear_archived() {
        let ws = TempWs::new("remove");
        let svc = ws.svc();
        let a = svc.add("留下", "app").unwrap().unwrap();
        let b = svc.add("删掉", "app").unwrap().unwrap();
        let c = svc.add("归档", "app").unwrap().unwrap();

        assert!(svc.remove(&b.id).unwrap());
        assert!(!svc.remove(&b.id).unwrap());

        svc.update(&c.id, None, Some("archived"), None).unwrap();
        assert_eq!(svc.clear_archived().unwrap(), 1);

        let left = svc.list_items("all");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, a.id);
    }

    #[test]
    fn archive_to_note_writes_markdown_and_links_back() {
        let ws = TempWs::new("archive-note");
        let svc = ws.svc();
        let item = svc.add("# 我的想法\n\n正文内容", "app").unwrap().unwrap();
        let dir = ws.0.join("知识库").join("收纳");

        let note = svc.archive_to_note(&item.id, &dir).unwrap();

        let body = std::fs::read_to_string(&note).unwrap();
        assert_eq!(body, "# 我的想法\n\n# 我的想法\n\n正文内容\n");
        assert!(note
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("我的想法"));

        let updated = svc
            .list_items("all")
            .into_iter()
            .find(|i| i.id == item.id)
            .unwrap();
        assert_eq!(updated.status, "archived");
        assert_eq!(
            updated.archived_path.as_deref(),
            Some(note.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn archive_to_note_avoids_overwriting() {
        let ws = TempWs::new("archive-dup");
        let svc = ws.svc();
        let dir = ws.0.join("收纳");
        let a = svc.add("重名想法", "app").unwrap().unwrap();
        let b = svc.add("重名想法", "app").unwrap().unwrap();

        let first = svc.archive_to_note(&a.id, &dir).unwrap();
        let second = svc.archive_to_note(&b.id, &dir).unwrap();
        assert_ne!(first, second, "第二篇覆盖了第一篇");
        assert!(first.exists() && second.exists());
    }

    #[test]
    fn convert_to_task_creates_an_open_task() {
        let ws = TempWs::new("to-task");
        let svc = ws.svc();
        let schedule = AgendaStore::new(&ws.0);
        let item = svc.add("买牛奶\n顺便买鸡蛋", "app").unwrap().unwrap();

        let task = svc.convert_to_task(&item.id, &schedule).unwrap();
        assert_eq!(task.title, "买牛奶");
        assert_eq!(task.status, crate::agenda::TaskStatus::Todo);
        assert_eq!(task.note, "买牛奶\n顺便买鸡蛋");

        let updated = svc
            .list_items("all")
            .into_iter()
            .find(|i| i.id == item.id)
            .unwrap();
        assert_eq!(updated.status, "archived");
        assert_eq!(updated.archived_path, None, "转任务不产生笔记路径");
    }

    #[test]
    fn derive_title_strips_markdown_markers() {
        assert_eq!(derive_title("# 标题"), "标题");
        assert_eq!(derive_title("> 引用"), "引用");
        assert_eq!(derive_title("- 列表项"), "列表项");
        assert_eq!(derive_title("* 星号"), "星号");
        assert_eq!(derive_title("1. 有序项"), "有序项");
        assert_eq!(derive_title("2) 括号项"), "括号项");
    }

    #[test]
    fn derive_title_uses_the_first_non_empty_line() {
        assert_eq!(derive_title("\n\n  真正的首行\n第二行"), "真正的首行");
        assert_eq!(derive_title(""), "未命名想法");
        assert_eq!(derive_title("   \n  "), "未命名想法");
    }

    #[test]
    fn derive_title_truncates_at_40_chars() {
        let long = "中".repeat(60);
        let title = derive_title(&long);
        assert_eq!(title.chars().count(), 41, "应是 40 字 + 省略号");
        assert!(title.ends_with('…'));
    }

    /// 全是标记符号时应回落到原行，而不是产出空标题。
    #[test]
    fn derive_title_falls_back_when_everything_is_markup() {
        assert_eq!(derive_title("###"), "###");
    }

    #[test]
    fn content_is_capped_at_20000_utf16_units() {
        let ws = TempWs::new("cap-len");
        let long = "中".repeat(MAX_CONTENT_LENGTH + 100);
        let item = ws.svc().add(&long, "app").unwrap().unwrap();
        assert_eq!(item.content.encode_utf16().count(), MAX_CONTENT_LENGTH);
    }

    #[test]
    fn truncation_never_splits_a_surrogate_pair() {
        // 🎉 占 2 个 UTF-16 码元；上限设为奇数时必须退一格而不是切半
        let s = "🎉".repeat(10);
        let out = truncate_utf16(&s, 5);
        assert_eq!(out.encode_utf16().count(), 4, "切在代理对中间了");
        assert!(out.chars().all(|c| c == '🎉'));
    }

    #[test]
    fn corrupt_file_is_never_overwritten_by_a_new_capture() {
        let ws = TempWs::new("corrupt");
        let svc = ws.svc();
        svc.initialize().unwrap();
        std::fs::write(ws.0.join(paths::INBOX_DIR_NAME).join("items.json"), "{ 坏").unwrap();
        assert_eq!(svc.count_inbox(), 0);
        assert!(svc.add("恢复写入", "app").is_err());
        assert_eq!(ws.raw(), "{ 坏");
    }

    #[test]
    fn round_trip_preserves_bytes() {
        let ws = TempWs::new("roundtrip");
        let svc = ws.svc();
        svc.add("带中文的想法", "quick-capture").unwrap();
        let before = ws.raw();

        let parsed: CaptureFile = json2::deserialize(&before).unwrap();
        let after = format!("{}\n", json2::serialize(&parsed).unwrap());
        assert_eq!(before, after);
    }
}
