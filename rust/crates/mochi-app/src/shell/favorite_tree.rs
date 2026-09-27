//! 根据收藏路径构建文件树节点及其父目录。
use super::*;

/// 按收藏路径建树。这里故意不调用 `build_file_tree`：收藏视图只需要读取每个
/// 收藏文件和它的父目录，不能因为切到收藏就扫描整个工作区。关闭父级目录时，
/// 每个收藏文件直接作为顶层节点，允许不同目录下的同名文件同时显示。
pub(super) fn build_favorites_tree(
    workspace_root: &Path,
    favorites: &[PathBuf],
    show_parents: bool,
) -> Vec<FileNode> {
    let mut tree = Vec::new();
    for path in favorites {
        if !path.is_file() {
            // 收藏索引保留暂时缺失的路径，等文件恢复；树当前只展示真实文件。
            continue;
        }
        let Ok(relative) = path.strip_prefix(workspace_root) else {
            continue;
        };
        if !show_parents {
            // 上面的 `strip_prefix` 会将平铺视图限制在当前工作区内，
            // 而保留原始路径可避免同名文件被混淆。
            if relative.components().next().is_some() {
                tree.push(favorite_file_node(path));
            }
            continue;
        }
        let components = relative
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(name) => Some(name.to_os_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if components.is_empty() {
            continue;
        }
        insert_favorite_node(&mut tree, workspace_root, &components, path);
    }
    tree
}

fn insert_favorite_node(
    nodes: &mut Vec<FileNode>,
    parent: &Path,
    components: &[std::ffi::OsString],
    favorite_path: &Path,
) {
    let Some(name) = components.first() else {
        return;
    };
    let path = parent.join(name);
    let is_leaf = components.len() == 1;
    let path_text = path.to_string_lossy().into_owned();
    let index = nodes.iter().position(|node| node.path == path_text);

    if is_leaf {
        let node = favorite_file_node(favorite_path);
        if let Some(index) = index {
            nodes[index] = node;
        } else {
            nodes.push(node);
        }
        return;
    }

    let index = index.unwrap_or_else(|| {
        nodes.push(FileNode {
            id: path.to_string_lossy().into_owned(),
            name: name.to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            kind: "directory".to_owned(),
            children: Some(Vec::new()),
            ..Default::default()
        });
        nodes.len() - 1
    });
    let children = nodes[index].children.get_or_insert_with(Vec::new);
    insert_favorite_node(children, &path, &components[1..], favorite_path);
}

fn favorite_file_node(path: &Path) -> FileNode {
    let raw_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let is_link = raw_name.to_ascii_lowercase().ends_with(".link.json");
    let name = if is_link {
        raw_name[..raw_name.len() - ".link.json".len()].to_owned()
    } else {
        raw_name
    };
    let metadata = std::fs::metadata(path).ok();
    let size = metadata.as_ref().map(|metadata| metadata.len() as i64);
    let mtime = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .map(|time| mochi_core::jstime::from(chrono::DateTime::<chrono::Utc>::from(time)));
    let created_at = metadata
        .as_ref()
        .and_then(|metadata| metadata.created().ok())
        .map(|time| mochi_core::jstime::from(chrono::DateTime::<chrono::Utc>::from(time)));
    FileNode {
        id: path.to_string_lossy().into_owned(),
        name,
        path: path.to_string_lossy().into_owned(),
        kind: if is_link { "link" } else { "file" }.to_owned(),
        size,
        mtime,
        created_at,
        ..Default::default()
    }
}

pub(super) fn favorite_ancestor_dirs(
    workspace_root: &Path,
    favorites: &[PathBuf],
) -> HashSet<PathBuf> {
    let mut expanded = HashSet::new();
    for path in favorites {
        if !path.is_file() {
            continue;
        }
        let Ok(relative) = path.strip_prefix(workspace_root) else {
            continue;
        };
        let components = relative.components().collect::<Vec<_>>();
        if components.len() < 2 {
            continue;
        }
        let mut directory = workspace_root.to_path_buf();
        for component in &components[..components.len() - 1] {
            let std::path::Component::Normal(name) = component else {
                continue;
            };
            directory.push(name);
            expanded.insert(directory.clone());
        }
    }
    expanded
}
