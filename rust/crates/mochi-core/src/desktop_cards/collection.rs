//! 只读的文档集合。多个来源合并成一个视图，而不是各自一份拷贝。
use super::{safe_source_path, Action, Module, Page, Row, RowMeta};
use crate::{domain::Library, files::FileService};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub fn accepts(module: Module, path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match module {
        Module::Base => ext == "mcb",
        Module::Canvas => ext == "mcanvas",
        Module::Exam => ext == "exam",
        Module::Document => !matches!(ext.as_str(), "mcb" | "mcanvas" | "exam"),
        _ => false,
    }
}

pub fn rows(
    root: &Path,
    page: &Page,
    max_entries: usize,
    max_rows: usize,
) -> anyhow::Result<Vec<Row>> {
    let canonical = root.canonicalize()?;
    let sources: Vec<_> = page.sources.iter().chain(page.source.iter()).collect();
    let mut pending = Vec::<PathBuf>::new();
    if sources.is_empty() {
        let libraries: Vec<Library> = std::fs::read(root.join(".mochi/libraries.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        pending.extend(libraries.into_iter().map(|l| {
            let p = PathBuf::from(l.path);
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
        }));
        // 兼容老工作区：把根目录文档和未注册的资料库也算进来。
        pending.push(root.to_owned());
    } else {
        for source in sources {
            pending.push(safe_source_path(root, source)?);
        }
    }
    let fs = FileService::new();
    let mut visited = HashSet::new();
    let mut rows = Vec::new();
    let mut count = 0;
    while let Some(path) = pending.pop() {
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !crate::paths::path_is_within(&canonical, &path) || !visited.insert(path.clone()) {
            continue;
        }
        count += 1;
        anyhow::ensure!(
            count <= max_entries,
            "来源项目超过本次读取预算，请缩小来源范围"
        );
        if path.is_dir() {
            let children = fs.build_file_tree_shallow(&path)?;
            pending.extend(children.into_iter().rev().map(|n| PathBuf::from(n.path)));
        } else if accepts(page.module, &path) {
            let relative = path
                .strip_prefix(&canonical)?
                .to_string_lossy()
                .replace('\\', "/");
            rows.push(Row {
                id: format!("file:{relative}"),
                title: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                action: Some(Action::OpenFile(relative.clone())),
                meta: RowMeta {
                    path: relative,
                    icon: "file".into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            if rows.len() >= max_rows {
                break;
            }
        }
    }
    rows.sort_by(|a, b| a.meta.path.to_lowercase().cmp(&b.meta.path.to_lowercase()));
    Ok(rows)
}

/// 桌面来源只索引文件系统条目，AI/日程对象和文档正文一概不碰。
pub fn source_candidates(root: &Path) -> Vec<crate::object_reference::ObjectCandidate> {
    use crate::object_reference::{build_document_reference, ObjectCandidate, ObjectKind};
    let mut out = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = pending.pop() {
        if depth > 32 || out.len() >= 100_000 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if !source_relative_allowed(relative) {
                continue;
            }
            let Ok(ty) = entry.file_type() else {
                continue;
            };
            // 不跟进 junction/符号链接，防止跳到外部卷或成环。
            if ty.is_symlink() || (!ty.is_dir() && !ty.is_file()) {
                continue;
            }
            let kind = if ty.is_dir() {
                ObjectKind::Directory
            } else {
                ObjectKind::Document
            };
            let title = entry.file_name().to_string_lossy().into_owned();
            out.push(ObjectCandidate::new(
                build_document_reference(&path, kind, Some(root), None),
                title,
                relative.to_string_lossy().replace('\\', "/"),
                kind,
            ));
            if ty.is_dir() {
                pending.push((path, depth + 1));
            }
            if out.len() >= 100_000 {
                break;
            }
        }
    }
    out.sort_by(|a, b| {
        (a.kind != crate::object_reference::ObjectKind::Directory)
            .cmp(&(b.kind != crate::object_reference::ObjectKind::Directory))
            .then_with(|| a.title.cmp(&b.title))
    });
    out
}

pub fn source_relative_allowed(relative: &Path) -> bool {
    use std::path::Component;
    let mut parts = relative.components();
    let Some(Component::Normal(first)) = parts.next() else {
        return false;
    };
    if matches!(first.to_str(), Some("Agent配置" | "AI提示词" | "schedule")) {
        return false;
    }
    relative.components().all(|part| match part {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            !name.starts_with('.')
                && ![
                    "node_modules",
                    "target",
                    "dist",
                    "build",
                    "coverage",
                    "vendor",
                ]
                .contains(&name.as_ref())
        }
        _ => false,
    })
}
