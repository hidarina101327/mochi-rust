//! 读取目录并构建文件树，合并映射文件夹和子文档节点。
use super::*;

pub(super) fn read_children(root: &Path, path: &Path) -> Result<Vec<FileNode>> {
    let mapped = mochi_core::mapped_folders::Service::new(root);
    let mut children = match mapped.for_path(path)? {
        Some(folder) => mapped.list_children(&folder, path)?,
        None => {
            let mut children = FileService::new().build_file_tree_shallow(path)?;
            append_mapped_nodes(&mut children, &mapped.for_parent(path).unwrap_or_default());
            children
        }
    };
    attach_sub_documents(root, &mut children);
    Ok(children)
}

/// 当前库的磁盘根目录。库路径在 `libraries.json` 里是正斜杠的绝对路径；
/// 没有库时退回工作区根，保证空工作区也能显示点东西。
pub(super) fn library_root(workspace_root: &Path, libraries: &[Library], active: usize) -> PathBuf {
    libraries
        .get(active)
        .map(|l| PathBuf::from(&l.path))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| workspace_root.to_path_buf())
}

/// 按范围建文件树。按类型显示时，每个库是一个顶层文件夹（Electron 的
/// `Sidebar` 在 `selectedLibraryId` 为空时就是这么列的）。
pub(super) fn build_tree(
    workspace_root: &Path,
    libraries: &[Library],
    scope: &TreeScope,
) -> Vec<FileNode> {
    let fs = FileService::new();
    let mut tree = match scope {
        TreeScope::Library(i) => fs
            .build_file_tree_shallow(library_root(workspace_root, libraries, *i))
            .unwrap_or_default(),
        TreeScope::Type(type_id) => libraries
            .iter()
            .filter(|l| &l.kind == type_id)
            .filter_map(|l| {
                let path = PathBuf::from(&l.path);
                if !path.is_dir() {
                    return None;
                }
                let children = fs.build_file_tree_shallow(&path).unwrap_or_default();
                Some(FileNode {
                    id: l.path.clone(),
                    name: l.name.clone(),
                    path: l.path.clone(),
                    kind: "directory".to_owned(),
                    children: Some(children),
                    ..Default::default()
                })
            })
            .collect(),
    };
    let mapped = mochi_core::mapped_folders::Service::new(workspace_root);
    match scope {
        TreeScope::Library(i) => {
            let root = library_root(workspace_root, libraries, *i);
            append_mapped_nodes(&mut tree, &mapped.for_parent(&root).unwrap_or_default());
        }
        TreeScope::Type(type_id) => {
            for node in &mut tree {
                let library = libraries.iter().find(|library| {
                    &library.kind == type_id && Path::new(&library.path) == Path::new(&node.path)
                });
                if let Some(library) = library {
                    let mapped = mapped
                        .for_parent(Path::new(&library.path))
                        .unwrap_or_default();
                    append_mapped_nodes(node.children.get_or_insert_with(Vec::new), &mapped);
                }
            }
        }
    }
    attach_sub_documents(workspace_root, &mut tree);
    tree
}

pub(super) fn append_mapped_nodes(
    nodes: &mut Vec<FileNode>,
    mapped: &[mochi_core::mapped_folders::MappedFolder],
) {
    for folder in mapped {
        let source = Path::new(&folder.source);
        nodes.push(FileNode {
            id: format!("mapped:{}", folder.id),
            name: folder.name.clone(),
            path: folder.source.clone(),
            kind: "mapped-directory".into(),
            children: None,
            lazy: source.is_dir(),
            ..Default::default()
        });
    }
}

/// 子文档保存在父文档旁的隐藏伴生夹中，常规磁盘扫描会跳过它。文件树建好后
/// 再依据索引把这些子项挂回父文档，保留索引中的插入顺序。
fn attach_sub_documents(workspace_root: &Path, nodes: &mut [FileNode]) {
    // 每次读目录只取一份索引快照，而不是每个文件都读一次盘。
    let store = mochi_core::sub_documents::load(workspace_root);
    if store.relations.is_empty() {
        return;
    }
    let relations = store.relations.iter().collect::<HashMap<_, _>>();
    attach_sub_document_nodes(nodes, &relations);
}

fn attach_sub_document_nodes(nodes: &mut [FileNode], relations: &HashMap<&str, &[String]>) {
    for node in nodes {
        if let Some(children) = &mut node.children {
            attach_sub_document_nodes(children, relations);
            continue;
        }
        if node.is_directory() {
            continue;
        }

        let parent_path = node.path.clone();
        let parent_key = mochi_core::paths::to_forward_slashes(&parent_path);
        let children = relations
            .get(parent_key.as_str())
            .into_iter()
            .flat_map(|children| children.iter())
            .filter(|path| mochi_core::sub_documents::is_inside_sidecar(&parent_key, path))
            .filter_map(|path| {
                let path = PathBuf::from(path);
                if !path.is_file() {
                    return None;
                }
                let name = path.file_name()?.to_string_lossy().into_owned();
                Some(FileNode {
                    id: path.to_string_lossy().into_owned(),
                    name,
                    path: path.to_string_lossy().into_owned(),
                    kind: "file".into(),
                    sub_doc_parent: Some(parent_path.clone()),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();
        if !children.is_empty() {
            node.children = Some(children);
            if let Some(children) = &mut node.children {
                attach_sub_document_nodes(children, relations);
            }
        }
    }
}

pub(super) fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 与 Electron 版的 `.mochi/drag-sort.json` 兼容；状态损坏或缺失都无妨，
/// 直接沿用文件系统顺序即可。
pub(super) fn read_manual_sort_orders(root: &Path) -> HashMap<String, Vec<String>> {
    std::fs::read_to_string(root.join(".mochi").join("drag-sort.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<HashMap<String, Vec<String>>>(&json).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(folder, paths)| {
            (
                folder.replace('\\', "/"),
                paths
                    .into_iter()
                    .map(|path| path.replace('\\', "/"))
                    .collect(),
            )
        })
        .collect()
}

pub(super) fn remap_expanded_paths(expanded: &mut HashSet<PathBuf>, from: &Path, to: &Path) {
    let affected = expanded
        .iter()
        .filter(|path| (*path).as_path() == from || (*path).starts_with(from))
        .cloned()
        .collect::<Vec<_>>();
    for path in affected {
        expanded.remove(&path);
        let suffix = path.strip_prefix(from).unwrap_or_else(|_| Path::new(""));
        expanded.insert(to.join(suffix));
    }
}

pub(super) fn remove_expanded_under(expanded: &mut HashSet<PathBuf>, path: &Path) {
    expanded.retain(|entry| entry.as_path() != path && !entry.starts_with(path));
}
