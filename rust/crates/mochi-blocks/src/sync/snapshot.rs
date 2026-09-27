//! 定义同步快照及其中明确列出的文件内容。
use super::*;

/// 快照中一条显式给定的文件。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// 规范的、以斜杠分隔的相对路径。
    pub path: String,
    /// 放到该路径上的确切字节。
    pub bytes: Vec<u8>,
}

impl SnapshotFile {
    /// 先校验路径再构造文件。
    pub fn new(path: impl Into<String>, bytes: impl AsRef<[u8]>) -> Result<Self> {
        let path = validate_snapshot_path(&path.into())?;
        Ok(Self {
            path,
            bytes: bytes.as_ref().to_vec(),
        })
    }
}

/// 一组显式声明的文件变更。
///
/// 快照应用在当前已提交状态之上。files 里列出的文件做 upsert，
/// deleted_paths 里的路径删除，没有提到的路径原样不动。
/// 小块编辑因此可以显式表达，同时保留父提交里的其他全部文件。
/// 它依旧从不扫描工作区。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub files: Vec<SnapshotFile>,
    #[serde(default)]
    pub deleted_paths: Vec<String>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn new<I>(files: I) -> Result<Self>
    where
        I: IntoIterator<Item = SnapshotFile>,
    {
        let snapshot = Self {
            files: files.into_iter().collect(),
            deleted_paths: Vec::new(),
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn from_files<I>(files: I) -> Result<Self>
    where
        I: IntoIterator<Item = SnapshotFile>,
    {
        Self::new(files)
    }

    pub fn with_file(mut self, file: SnapshotFile) -> Result<Self> {
        self.files.push(file);
        self.validate()?;
        Ok(self)
    }

    pub fn delete(mut self, path: impl Into<String>) -> Result<Self> {
        self.deleted_paths
            .push(validate_snapshot_path(&path.into())?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        let mut paths = BTreeSet::new();
        for file in &self.files {
            let path = validate_snapshot_path(&file.path)?;
            ensure!(path == file.path, "snapshot path is not canonical: {path}");
            ensure!(
                paths.insert(path.clone()),
                "duplicate snapshot path: {path}"
            );
        }
        for path in &self.deleted_paths {
            let canonical = validate_snapshot_path(path)?;
            ensure!(
                canonical == *path,
                "deleted snapshot path is not canonical: {path}"
            );
            ensure!(
                paths.insert(canonical.clone()),
                "path both changed and deleted: {canonical}"
            );
        }
        validate_no_path_prefix_collisions(paths.iter().map(String::as_str))
    }

    pub(super) fn changes(&self) -> Result<BTreeMap<String, Option<Vec<u8>>>> {
        self.validate()?;
        let mut changes = BTreeMap::new();
        for file in &self.files {
            changes.insert(file.path.clone(), Some(file.bytes.clone()));
        }
        for path in &self.deleted_paths {
            changes.insert(path.clone(), None);
        }
        Ok(changes)
    }
}

/// 返回用户给出的相对路径的规范形式。
///
/// 反斜杠即使在 Unix 上也按分隔符处理，快照才不会在某个平台
/// 安全、换个平台就变成路径穿越。
pub fn validate_snapshot_path(value: &str) -> Result<String> {
    ensure!(!value.is_empty(), "snapshot path must not be empty");
    ensure!(!value.as_bytes().contains(&0), "snapshot path contains NUL");

    let normalized = value.replace('\\', "/");
    ensure!(
        !normalized.starts_with('/'),
        "absolute snapshot path is not allowed: {value}"
    );
    ensure!(
        !normalized.starts_with("//"),
        "UNC snapshot path is not allowed: {value}"
    );
    ensure!(
        !(normalized.len() >= 2 && normalized.as_bytes()[1] == b':'),
        "drive-qualified snapshot path is not allowed: {value}"
    );

    let components: Vec<&str> = normalized.split('/').collect();
    ensure!(
        components.iter().all(|component| !component.is_empty()),
        "snapshot path contains an empty component: {value}"
    );
    ensure!(
        components
            .iter()
            .all(|component| *component != "." && *component != ".."),
        "snapshot path traversal is not allowed: {value}"
    );

    for (index, component) in components.iter().enumerate() {
        ensure!(
            !component
                .as_bytes()
                .iter()
                .any(|byte| *byte < 0x20 || *byte == 0x7f),
            "snapshot path contains a control character: {value}"
        );
        ensure!(
            !component.contains(':'),
            "snapshot path contains a reserved separator: {value}"
        );
        ensure!(
            !component.ends_with(['.', ' ']),
            "snapshot path has a Windows-ambiguous suffix: {value}"
        );

        let lower = component.to_ascii_lowercase();
        ensure!(
            !is_windows_device_name(&lower),
            "snapshot path contains a reserved device name: {value}"
        );
        ensure!(
            !lower.starts_with(".mochi"),
            "Mochi private paths cannot be uploaded as snapshot files: {value}"
        );

        // .git 在任何深度都禁止：像 nested/.git/config 这样的路径，
        // 只要物化到工作区里就仍是仓库控制目录。其他 Git 顶层保留名
        // 只在根上受限，notes/objects.md 这样的文件才能继续用。
        ensure!(
            lower != ".git",
            "Git control directory is reserved: {value}"
        );
        if index == 0 {
            ensure!(
                !is_reserved_git_root(&lower),
                "Git control path is reserved: {value}"
            );
        }
    }

    Ok(normalized)
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem,
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "clock$"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

fn is_reserved_git_root(component: &str) -> bool {
    matches!(
        component,
        ".git"
            | "objects"
            | "refs"
            | "head"
            | "config"
            | "hooks"
            | "info"
            | "logs"
            | "packed-refs"
            | "index"
            | "branches"
            | "description"
    )
}

fn validate_no_path_prefix_collisions<'a, I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut folded: Vec<(String, &str)> = paths
        .into_iter()
        .map(|path| (path.to_lowercase(), path))
        .collect();
    folded.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    for pair in folded.windows(2) {
        if let [(left_folded, left), (right_folded, right)] = pair {
            ensure!(
                left_folded != right_folded,
                "paths collide on case-insensitive filesystems: {left}, {right}"
            );
            ensure!(
                !right_folded.starts_with(&format!("{left_folded}/")),
                "snapshot paths collide as a file and child on case-insensitive filesystems: {left}, {right}"
            );
        }
    }
    Ok(())
}

pub(super) fn snapshot_from_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Snapshot> {
    let files = files
        .iter()
        .filter(|(path, _)| path.as_str() != BLOCK_ID_COMMENT_PATH)
        .map(|(path, bytes)| SnapshotFile::new(path.clone(), bytes.clone()))
        .collect::<Result<Vec<_>>>()?;
    Snapshot::new(files)
}

pub(super) fn snapshot_to_map(snapshot: &Snapshot) -> Result<BTreeMap<String, Vec<u8>>> {
    snapshot.validate()?;
    let mut files = BTreeMap::new();
    for file in &snapshot.files {
        files.insert(file.path.clone(), file.bytes.clone());
    }
    for path in &snapshot.deleted_paths {
        files.remove(path);
    }
    validate_file_map(&files, false)?;
    Ok(files)
}

pub(super) fn upsert_snapshot_file(snapshot: &mut Snapshot, file: SnapshotFile) {
    snapshot.deleted_paths.retain(|path| path != &file.path);
    if let Some(existing) = snapshot
        .files
        .iter_mut()
        .find(|existing| existing.path == file.path)
    {
        *existing = file;
    } else {
        snapshot.files.push(file);
    }
}

pub(super) fn validate_file_map(
    files: &BTreeMap<String, Vec<u8>>,
    allow_metadata: bool,
) -> Result<()> {
    let mut paths = BTreeSet::new();
    for path in files.keys() {
        let canonical = if allow_metadata && path == BLOCK_ID_COMMENT_PATH {
            path.clone()
        } else {
            validate_snapshot_path(path)
                .with_context(|| format!("invalid committed path: {path}"))?
        };
        ensure!(
            canonical == *path,
            "committed path is not canonical: {path}"
        );
        ensure!(
            paths.insert(path.clone()),
            "duplicate committed path: {path}"
        );
    }
    validate_no_path_prefix_collisions(paths.iter().map(String::as_str))
}
