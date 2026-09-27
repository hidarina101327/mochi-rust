//! 外部目录映射：只保存配置并按需列出名称，绝不读取文件正文。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{domain::FileNode, json2, paths};

static EDIT_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MappingRules {
    /// 空白名单表示允许全部；默认值显式落盘，方便用户直接在设置中修改。
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for MappingRules {
    fn default() -> Self {
        Self {
            include: vec!["**".into()],
            exclude: vec![
                ".git/**".into(),
                "**/.git/**".into(),
                ".mochi/**".into(),
                "**/.mochi/**".into(),
                "node_modules/**".into(),
                "**/node_modules/**".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MappedFolder {
    pub id: String,
    pub name: String,
    /// 外部目录绝对路径。
    pub source: String,
    /// 所属知识库目录的绝对路径。
    pub library_path: String,
    /// 挂载位置；旧配置省略时仍挂在知识库根目录。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_path: Option<String>,
    pub rules: MappingRules,
}

impl Default for MappedFolder {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            source: String::new(),
            library_path: String::new(),
            parent_path: None,
            rules: MappingRules::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// 新建映射时预先填入表单的全局默认规则；用户可在每个映射中覆盖。
    pub defaults: MappingRules,
    pub folders: Vec<MappedFolder>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            defaults: MappingRules::default(),
            folders: Vec::new(),
        }
    }
}

pub struct Service {
    workspace: PathBuf,
}

impl Service {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }

    pub fn load(&self) -> Result<Config> {
        let path = paths::mapped_folders_file(&self.workspace);
        if !path.exists() {
            return Ok(Config::default());
        }
        json2::read_from_file(&path)
            .with_context(|| format!("读取映射文件夹配置失败: {}", path.display()))
            .map(Option::unwrap_or_default)
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        json2::write(paths::mapped_folders_file(&self.workspace), config)
    }

    pub fn add(
        &self,
        library_path: &Path,
        source: &Path,
        name: impl Into<String>,
        rules: MappingRules,
    ) -> Result<MappedFolder> {
        self.add_at(library_path, library_path, source, name, rules)
    }

    pub fn add_at(
        &self,
        library_path: &Path,
        parent: &Path,
        source: &Path,
        name: impl Into<String>,
        rules: MappingRules,
    ) -> Result<MappedFolder> {
        let _guard = EDIT_GATE.lock().unwrap_or_else(|error| error.into_inner());
        let source = source
            .canonicalize()
            .unwrap_or_else(|_| source.to_path_buf());
        if !source.is_dir() {
            bail!("映射来源必须是一个存在的文件夹");
        }
        if !paths::paths_equal(parent, library_path) {
            let library = library_path.canonicalize()?;
            let parent = parent.canonicalize()?;
            if !parent.is_dir() || !paths::path_is_within(&library, &parent) {
                bail!("映射位置必须在所属知识库内");
            }
        }
        if paths::path_is_within(
            &source,
            &parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf()),
        ) {
            bail!("不能将文件夹映射到自身或其子文件夹");
        }
        let name = name.into().trim().to_owned();
        if name.is_empty() || name.chars().any(|c| matches!(c, '/' | '\\' | '\0')) {
            bail!("映射文件夹名称无效");
        }
        let mut config = self.load()?;
        if config.folders.iter().any(|folder| {
            paths::paths_equal(Path::new(&folder.library_path), library_path)
                && ((paths::paths_equal(folder.parent(), parent)
                    && folder.name.eq_ignore_ascii_case(&name))
                    || paths::paths_equal(Path::new(&folder.source), &source))
        }) {
            bail!("该知识库中已存在同名或同来源的映射文件夹");
        }
        let folder = MappedFolder {
            id: format!("mapped-{}", paths::random_base36(12)),
            name,
            source: source.to_string_lossy().into_owned(),
            library_path: library_path.to_string_lossy().into_owned(),
            parent_path: (!paths::paths_equal(parent, library_path))
                .then(|| parent.to_string_lossy().into_owned()),
            rules,
        };
        config.folders.push(folder.clone());
        self.save(&config)?;
        Ok(folder)
    }

    pub fn for_library(&self, library: &Path) -> Result<Vec<MappedFolder>> {
        Ok(self
            .load()?
            .folders
            .into_iter()
            .filter(|folder| paths::paths_equal(Path::new(&folder.library_path), library))
            .collect())
    }

    pub fn for_path(&self, path: &Path) -> Result<Option<MappedFolder>> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Ok(self
            .load()?
            .folders
            .into_iter()
            .filter(|folder| paths::path_is_within(Path::new(&folder.source), &path))
            .max_by_key(|folder| folder.source.len()))
    }

    pub fn for_parent(&self, parent: &Path) -> Result<Vec<MappedFolder>> {
        Ok(self
            .load()?
            .folders
            .into_iter()
            .filter(|folder| paths::paths_equal(folder.parent(), parent))
            .collect())
    }

    /// 物理父目录改名/移动后，虚拟子目录仍要挂得住。
    pub fn move_mounts(&self, from: &Path, to: &Path, library: Option<&Path>) -> Result<()> {
        let _guard = EDIT_GATE.lock().unwrap_or_else(|error| error.into_inner());
        let mut config = self.load()?;
        let mut changed = false;
        for folder in &mut config.folders {
            if folder.parent_path.is_none() || !paths::path_is_within(from, folder.parent()) {
                continue;
            }
            let relative = folder
                .parent()
                .components()
                .skip(from.components().count())
                .collect::<PathBuf>();
            folder.parent_path = Some(to.join(relative).to_string_lossy().into_owned());
            if let Some(library) = library {
                folder.library_path = library.to_string_lossy().into_owned();
            }
            changed = true;
        }
        if changed {
            self.save(&config)?;
        }
        Ok(())
    }

    /// 只读取这一层的目录项和名称，不递归，也不打开文件内容。
    pub fn list_children(&self, folder: &MappedFolder, directory: &Path) -> Result<Vec<FileNode>> {
        let root = Path::new(&folder.source);
        let directory = directory
            .canonicalize()
            .unwrap_or_else(|_| directory.to_path_buf());
        if !paths::path_is_within(root, &directory) || !directory.is_dir() {
            bail!("映射目录已不可访问");
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let ty = entry.file_type()?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if !visible(&relative, ty.is_dir(), &folder.rules) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            out.push(FileNode {
                id: path.to_string_lossy().into_owned(),
                name,
                path: path.to_string_lossy().into_owned(),
                kind: if ty.is_dir() { "directory" } else { "file" }.into(),
                children: None,
                lazy: ty.is_dir(),
                ..Default::default()
            });
        }
        out.sort_by(|a, b| {
            b.is_directory()
                .cmp(&a.is_directory())
                .then_with(|| crate::files::compare_names(&a.name, &b.name))
        });
        Ok(out)
    }
}

impl MappedFolder {
    pub fn parent(&self) -> &Path {
        Path::new(self.parent_path.as_deref().unwrap_or(&self.library_path))
    }
}

/// Gitignore 风格的轻量通配：`*` 不跨目录、`**` 可跨目录、`?` 匹配一个字符；
/// 规则前的 `!` 可把前一条排除规则重新包含。
pub fn matches(patterns: &[String], path: &str, default: bool) -> bool {
    let mut result = default;
    for raw in patterns {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let (include, pattern) = raw.strip_prefix('!').map_or((true, raw), |p| (false, p));
        let pattern = pattern.trim_start_matches('/');
        // `foo/**` 在 gitignore 中也会命中目录 `foo` 自身；否则默认黑名单
        // 会漏出可展开的 `.git` / `node_modules` 顶层入口。
        let matches_directory_root = pattern
            .strip_suffix("/**")
            .is_some_and(|root| glob_matches(root, path));
        if matches_directory_root || glob_matches(pattern, path) {
            result = include;
        }
    }
    result
}

fn visible(path: &str, is_dir: bool, rules: &MappingRules) -> bool {
    let include = rules.include.is_empty() || matches(&rules.include, path, false);
    // 目录即使尚未命中白名单，也要保留入口，让 `**/*.md` 之类的规则可向下遍历。
    include && !matches(&rules.exclude, path, false)
        || (is_dir && !matches(&rules.exclude, path, false))
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    fn inner(p: &[u8], t: &[u8]) -> bool {
        match p {
            [] => t.is_empty(),
            // gitignore 中 `**/name` 的 `**/` 也能匹配零层目录。
            [b'*', b'*', b'/', rest @ ..] => {
                inner(rest, t)
                    || t.iter()
                        .enumerate()
                        .filter(|(_, byte)| **byte == b'/')
                        .any(|(i, _)| inner(p, &t[i + 1..]))
            }
            [b'*', b'*', rest @ ..] => (0..=t.len()).any(|i| inner(rest, &t[i..])),
            [b'*', rest @ ..] => {
                let mut i = 0;
                while i <= t.len() {
                    if inner(rest, &t[i..]) {
                        return true;
                    }
                    if i == t.len() || t[i] == b'/' {
                        break;
                    }
                    i += 1;
                }
                false
            }
            [b'?', rest @ ..] => !t.is_empty() && t[0] != b'/' && inner(rest, &t[1..]),
            [head, rest @ ..] => {
                !t.is_empty() && head.eq_ignore_ascii_case(&t[0]) && inner(rest, &t[1..])
            }
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gitignore_style_rules_support_globs_and_reinclude() {
        assert!(matches(&["**/*.md".into()], "notes/a.md", false));
        assert!(!matches(&["**/*.md".into()], "notes/a.pdf", false));
        assert!(!matches(
            &["**".into(), "!keep.md".into()],
            "keep.md",
            false
        ));
        assert!(matches(&["**".into(), "!keep.md".into()], "drop.md", false));
        assert!(matches(&["**/.git/**".into()], ".git", false));
    }

    #[test]
    fn mapping_persists_and_lists_only_the_current_level() {
        let root = std::env::temp_dir().join(format!(
            "mochi-mapped-folders-{}",
            crate::paths::random_base36(12)
        ));
        let source = root.join("external");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(source.join("visible.md"), "not read while listing").unwrap();
        std::fs::write(source.join("hidden.pdf"), "not read while listing").unwrap();
        let service = Service::new(&root);
        let created = service
            .add(
                &root.join("library"),
                &source,
                "外部资料",
                MappingRules {
                    include: vec!["**/*.md".into()],
                    exclude: vec![],
                },
            )
            .unwrap();
        assert_eq!(service.for_library(&root.join("library")).unwrap().len(), 1);
        let listed = service.list_children(&created, &source).unwrap();
        assert!(listed.iter().any(|node| node.name == "visible.md"));
        assert!(listed.iter().any(|node| node.name == "nested" && node.lazy));
        assert!(!listed.iter().any(|node| node.name == "hidden.pdf"));
        assert!(listed.iter().all(|node| node.children.is_none()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
