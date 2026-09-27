//! 工作区文档的已读状态，与通知历史、文档内容相互独立。
use anyhow::Result;
use rusqlite::{params, Connection, OpenFlags};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

pub fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text
        .strip_prefix("//?/")
        .unwrap_or(&text)
        .trim_end_matches('/');
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.to_owned()
    }
}

fn within(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|tail| tail.starts_with('/'))
}

#[derive(Default, Clone, Debug)]
pub struct Snapshot {
    pub documents: HashMap<String, String>,
    ancestors: HashSet<String>,
}

impl Snapshot {
    pub fn has(&self, path: &Path) -> bool {
        self.ancestors.contains(&key(path))
    }
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
    fn insert(&mut self, path: String, token: String) {
        for ancestor in Path::new(&path).ancestors() {
            self.ancestors.insert(key(ancestor));
        }
        self.documents.insert(path, token);
    }
}

pub struct Store {
    root: PathBuf,
    path: PathBuf,
}
impl Store {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            path: root.join(".mochi/document-unread.sqlite3"),
        }
    }
    fn connect(&self) -> Result<Connection> {
        std::fs::create_dir_all(self.path.parent().unwrap())?;
        let db = Connection::open(&self.path)?;
        db.busy_timeout(Duration::from_secs(3))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS unread_documents(path TEXT PRIMARY KEY, token TEXT NOT NULL)")?;
        Ok(db)
    }
    /// 只有显式存在、真实的文档产物才有资格计数；缓存文件永远不配挂角标。
    pub fn mark(&self, paths: &[PathBuf]) -> Result<usize> {
        let root = self.root.canonicalize()?;
        let internal = key(&root.join(".mochi"));
        let eligible: HashSet<_> = paths
            .iter()
            .filter_map(|path| {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    self.root.join(path)
                };
                let path = path.canonicalize().ok()?;
                let identity = key(&path);
                (path.is_file() && within(&identity, &key(&root)) && !within(&identity, &internal))
                    .then_some(identity)
            })
            .collect();
        if eligible.is_empty() {
            return Ok(0);
        }
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        for path in &eligible {
            tx.execute("INSERT INTO unread_documents(path,token) VALUES (?1,lower(hex(randomblob(16)))) ON CONFLICT(path) DO UPDATE SET token=excluded.token", [path])?;
        }
        tx.commit()?;
        Ok(eligible.len())
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        if !self.path.exists() {
            return Ok(Snapshot::default());
        }
        let db = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.busy_timeout(Duration::from_millis(100))?;
        let mut query = db.prepare("SELECT path,token FROM unread_documents")?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result = Snapshot::default();
        for row in rows {
            let (path, token) = row?;
            if Path::new(&path).is_file() {
                result.insert(path, token);
            }
        }
        Ok(result)
    }
    /// 对旧版本的确认操作不能弄丢并发的 workflow 更新。
    pub fn read(&self, path: &Path, token: &str) -> Result<bool> {
        if !self.path.exists() {
            return Ok(false);
        }
        Ok(self.connect()?.execute(
            "DELETE FROM unread_documents WHERE path=?1 AND token=?2",
            params![key(path), token],
        )? > 0)
    }
    pub fn relocate(&self, from: &Path, to: &Path) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let rows = tx
            .prepare("SELECT path,token FROM unread_documents")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let from = key(from);
        let to = key(to);
        for (path, token) in rows.into_iter().filter(|(path, _)| within(path, &from)) {
            tx.execute("DELETE FROM unread_documents WHERE path=?1", [&path])?;
            tx.execute(
                "INSERT OR IGNORE INTO unread_documents(path,token) VALUES (?1,?2)",
                params![format!("{to}{}", &path[from.len()..]), token],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn remove_tree(&self, path: &Path) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let rows = tx
            .prepare("SELECT path FROM unread_documents")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let parent = key(path);
        for row in rows.into_iter().filter(|p| within(p, &parent)) {
            tx.execute("DELETE FROM unread_documents WHERE path=?1", [row])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(crate::paths::new_library_id("mochi-unread"));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn persistent_read_state_propagates_and_preserves_concurrent_updates() {
        let tmp = Temp::new();
        let root = tmp.path();
        let folder = root.join("知识库/行业");
        std::fs::create_dir_all(&folder).unwrap();
        let a = folder.join("a.md");
        let b = folder.join("b.md");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let store = Store::new(root);
        assert_eq!(store.mark(&[a.clone(), b.clone()]).unwrap(), 2);
        let state = Store::new(root).snapshot().unwrap();
        assert!(state.has(&folder));
        assert!(state.has(&root.join("知识库")));
        assert!(!state.has(&root.join("知识")));
        store.mark(&[a.clone()]).unwrap();
        assert!(!store.read(&a, &state.documents[&key(&a)]).unwrap());
        assert!(store.read(&b, &state.documents[&key(&b)]).unwrap());
        let state = store.snapshot().unwrap();
        assert!(state.has(&folder));
        assert!(store.read(&a, &state.documents[&key(&a)]).unwrap());
        assert!(store.snapshot().unwrap().is_empty());
    }
    #[test]
    fn caches_missing_files_and_other_workspaces_do_not_create_badges() {
        let tmp = Temp::new();
        let outside = Temp::new();
        std::fs::create_dir(tmp.path().join(".mochi")).unwrap();
        let cache = tmp.path().join(".mochi/cache.md");
        let foreign = outside.path().join("a.md");
        std::fs::write(&cache, "c").unwrap();
        std::fs::write(&foreign, "c").unwrap();
        assert_eq!(
            Store::new(tmp.path())
                .mark(&[
                    cache,
                    foreign,
                    tmp.path().join("missing.md"),
                    tmp.path().to_owned()
                ])
                .unwrap(),
            0
        );
    }
    #[test]
    fn folder_rename_and_delete_preserve_correct_ancestors() {
        let tmp = Temp::new();
        let root = tmp.path();
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir(&old).unwrap();
        let file = old.join("a.md");
        std::fs::write(&file, "a").unwrap();
        let store = Store::new(root);
        store.mark(&[file]).unwrap();
        std::fs::rename(&old, &new).unwrap();
        store.relocate(&old, &new).unwrap();
        let state = store.snapshot().unwrap();
        assert!(!state.has(&old));
        assert!(state.has(&new));
        store.remove_tree(&new).unwrap();
        assert!(store.snapshot().unwrap().is_empty());
    }
}
