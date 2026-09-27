//! 子文件必须位于父文件旁的 .<父文件全名>.sub/；索引读写都校验这一点。
//! 关系使用绝对路径，sub-documents.json 不加尾换行。

use std::path::Path;

use anyhow::{Context, Result};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{json2, paths};

/// 父 → 子的有序映射。**按插入序**序列化，不按字典序——
/// 与 Electron 版写出的键顺序一致，否则两版交替保存会反复重排整个文件。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Relations(Vec<(String, Vec<String>)>);

impl Relations {
    pub fn get(&self, parent: &str) -> Option<&[String]> {
        self.0
            .iter()
            .find(|(p, _)| p == parent)
            .map(|(_, c)| c.as_slice())
    }

    pub fn parents(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(p, _)| p.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.0.iter().map(|(p, c)| (p.as_str(), c.as_slice()))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 追加一个子文档；父键不存在时新建（追加到末尾，保持插入序）。
    pub fn push(&mut self, parent: &str, child: String) {
        match self.0.iter_mut().find(|(p, _)| p == parent) {
            Some((_, children)) => children.push(child),
            None => self.0.push((parent.to_owned(), vec![child])),
        }
    }

    /// 从**所有**父文档下摘掉某个子路径；空掉的父键随之删除
    /// （TS 的 `detach` 同样会删空键——留着空数组会让文件持续增长）。
    pub fn detach(&mut self, child: &str) -> bool {
        let mut changed = false;
        for (_, children) in &mut self.0 {
            let before = children.len();
            children.retain(|c| c != child);
            changed |= children.len() != before;
        }
        self.0.retain(|(_, children)| !children.is_empty());
        changed
    }

    pub fn remove_parent(&mut self, parent: &str) -> Option<Vec<String>> {
        let index = self.0.iter().position(|(p, _)| p == parent)?;
        Some(self.0.remove(index).1)
    }

    pub fn set(&mut self, parent: &str, children: Vec<String>) {
        if children.is_empty() {
            self.remove_parent(parent);
            return;
        }
        match self.0.iter_mut().find(|(p, _)| p == parent) {
            Some((_, existing)) => *existing = children,
            None => self.0.push((parent.to_owned(), children)),
        }
    }
}

impl Serialize for Relations {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (parent, children) in &self.0 {
            map.serialize_entry(parent, children)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Relations {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OrderedMap;
        impl<'de> Visitor<'de> for OrderedMap {
            type Value = Relations;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("父文档路径 → 子文档路径列表")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Relations, M::Error> {
                let mut out = Vec::with_capacity(access.size_hint().unwrap_or(0));
                while let Some((parent, children)) = access.next_entry::<String, Vec<String>>()? {
                    out.push((parent, children));
                }
                Ok(Relations(out))
            }
        }
        deserializer.deserialize_map(OrderedMap)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubDocumentStore {
    pub version: String,
    #[serde(default)]
    pub relations: Relations,
}

impl Default for SubDocumentStore {
    fn default() -> Self {
        Self {
            version: "1.0".into(),
            relations: Relations::default(),
        }
    }
}

pub fn store_path(workspace: &Path) -> String {
    format!(
        "{}/{}/sub-documents.json",
        paths::to_forward_slashes(&workspace.to_string_lossy()).trim_end_matches('/'),
        paths::MOCHI_DIR_NAME
    )
}

/// 读索引。文件不存在或损坏都返回空索引——一个坏掉的索引不该让整个工作区打不开
/// （代价只是子文档暂时挂不上，文件本身还在伴生夹里）。
pub fn load(workspace: &Path) -> SubDocumentStore {
    let path = store_path(workspace);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return SubDocumentStore::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// 写索引。**结尾不加换行**——TS 那边就是 `writeFile(JSON.stringify(store, null, 2))`。
pub fn save(workspace: &Path, store: &SubDocumentStore) -> Result<()> {
    json2::write_to_file(store_path(workspace), &json2::serialize(store)?)
        .context("写入子文档索引失败")
}

/// 伴生夹：`<父目录>/.<父文件全名>.sub`。
pub fn sidecar_path(parent_path: &str) -> String {
    let normalized = paths::to_forward_slashes(parent_path);
    match normalized.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/.{name}.sub"),
        None => format!(".{normalized}.sub"),
    }
}

/// 子路径是否确实落在该父文档的伴生夹内。
pub fn is_inside_sidecar(parent_path: &str, child_path: &str) -> bool {
    let sidecar = sidecar_path(parent_path);
    let child = paths::to_forward_slashes(child_path);
    child.starts_with(&format!("{sidecar}/"))
}

fn basename(path: &str) -> String {
    paths::to_forward_slashes(path)
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        // 下标 0 的点是「.gitignore」这种隐藏文件，整体算词干
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// 目录内不冲突的文件名：`笔记.md` → `笔记 1.md` → `笔记 2.md`。
/// 对齐 TS 的 `uniqueName`（**空格分隔**，不是 `-` 或 `(1)`）。
pub fn unique_name(dir: &str, name: &str) -> String {
    let stem = stem_of(name);
    let ext = &name[stem.len()..];
    let mut candidate = name.to_owned();
    let mut i = 1;
    while Path::new(&format!("{dir}/{candidate}")).exists() {
        candidate = format!("{stem} {i}{ext}");
        i += 1;
    }
    candidate
}

/// 在父文档下新建子文档，返回子文档绝对路径。
pub fn create(workspace: &Path, parent_path: &str, name: &str, content: &str) -> Result<String> {
    let parent = paths::to_forward_slashes(parent_path);
    let sidecar = sidecar_path(&parent);
    std::fs::create_dir_all(&sidecar).with_context(|| format!("创建伴生夹失败: {sidecar}"))?;

    let final_name = unique_name(&sidecar, name);
    let child_path = format!("{sidecar}/{final_name}");
    let body = if content.is_empty() {
        format!("# {}\n\n", stem_of(&final_name))
    } else {
        content.to_owned()
    };
    std::fs::write(&child_path, body).with_context(|| format!("写入子文档失败: {child_path}"))?;

    let mut store = load(workspace);
    store.relations.push(&parent, child_path.clone());
    save(workspace, &store)?;
    Ok(child_path)
}

/// 把已有文件移入伴生夹并挂载。
pub fn mount_existing(workspace: &Path, parent_path: &str, file_path: &str) -> Result<String> {
    let parent = paths::to_forward_slashes(parent_path);
    let source = paths::to_forward_slashes(file_path);
    let sidecar = sidecar_path(&parent);
    std::fs::create_dir_all(&sidecar).with_context(|| format!("创建伴生夹失败: {sidecar}"))?;

    let final_name = unique_name(&sidecar, &basename(&source));
    let dest = format!("{sidecar}/{final_name}");
    std::fs::rename(&source, &dest).with_context(|| format!("移动文件失败: {source}"))?;

    let mut store = load(workspace);
    // 原本挂在别处时先解除旧关系，否则一个文件会挂在两个父文档下
    store.relations.detach(&source);
    store.relations.push(&parent, dest.clone());
    save(workspace, &store)?;
    Ok(dest)
}

/// 取消挂载：移回父文档所在目录。没挂载过则返回 `None`。
pub fn unmount(workspace: &Path, child_path: &str) -> Result<Option<String>> {
    let child = paths::to_forward_slashes(child_path);
    let mut store = load(workspace);

    let Some(parent) = store
        .relations
        .iter()
        .find(|(_, children)| children.contains(&child))
        .map(|(p, _)| p.to_owned())
    else {
        return Ok(None);
    };

    let target_dir = parent
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("")
        .to_owned();
    let final_name = unique_name(&target_dir, &basename(&child));
    let dest = format!("{target_dir}/{final_name}");
    std::fs::rename(&child, &dest).with_context(|| format!("移出子文档失败: {child}"))?;

    store.relations.detach(&child);
    remove_sidecar_if_empty(&store, &parent);
    save(workspace, &store)?;
    Ok(Some(dest))
}

fn remove_sidecar_if_empty(store: &SubDocumentStore, parent: &str) {
    if store.relations.get(parent).is_some_and(|c| !c.is_empty()) {
        return;
    }
    let sidecar = sidecar_path(parent);
    if let Ok(mut entries) = std::fs::read_dir(&sidecar) {
        if entries.next().is_none() {
            let _ = std::fs::remove_dir(&sidecar);
        }
    }
}

/// 改名/移动时级联。父文档改名 → 迁伴生夹并改写键与子路径前缀；
/// 子文档改名 → 仍在伴生夹内就更新条目，移出去了就解除挂载。
pub fn update_path(workspace: &Path, old_path: &str, new_path: &str) -> Result<()> {
    let old = paths::to_forward_slashes(old_path);
    let new = paths::to_forward_slashes(new_path);
    let mut store = load(workspace);
    let mut changed = false;

    if let Some(children) = store.relations.remove_parent(&old) {
        let old_sidecar = sidecar_path(&old);
        let new_sidecar = sidecar_path(&new);
        if Path::new(&old_sidecar).exists() {
            let _ = std::fs::rename(&old_sidecar, &new_sidecar);
        }
        let moved = children
            .into_iter()
            .map(|c| match c.strip_prefix(&old_sidecar) {
                Some(rest) => format!("{new_sidecar}{rest}"),
                None => c,
            })
            .collect();
        store.relations.set(&new, moved);
        changed = true;
    }

    let parents: Vec<String> = store.relations.parents().map(str::to_owned).collect();
    for parent in parents {
        let Some(children) = store.relations.get(&parent) else {
            continue;
        };
        let Some(index) = children.iter().position(|c| *c == old) else {
            continue;
        };

        let mut updated = children.to_vec();
        if is_inside_sidecar(&parent, &new) {
            updated[index] = new.clone();
        } else {
            // 移出伴生夹 → 解除挂载，否则文件会同时出现在文件夹下和父文档下
            updated.remove(index);
        }
        store.relations.set(&parent, updated);
        changed = true;
    }

    if changed {
        save(workspace, &store)?;
    }
    Ok(())
}

/// 删除前的级联清理。父文档被删 → 连伴生夹一起删；子文档被删 → 从父列表移除。
///
/// **只删索引与伴生夹，不碰被删对象本身**——调用方负责删它，
/// 顺序上必须先调这里（父文档没了就找不到伴生夹了）。
pub fn handle_delete(workspace: &Path, target_path: &str) -> Result<()> {
    let target = paths::to_forward_slashes(target_path);
    let mut store = load(workspace);
    let mut changed = false;

    if store.relations.remove_parent(&target).is_some() {
        let sidecar = sidecar_path(&target);
        if Path::new(&sidecar).exists() {
            let _ = std::fs::remove_dir_all(&sidecar);
        }
        changed = true;
    }
    changed |= store.relations.detach(&target);

    if changed {
        save(workspace, &store)?;
    }
    Ok(())
}

/// 某父文档下**当前有效**的子文档：既在索引里、又确实在伴生夹内、且文件还在。
pub fn children_of(workspace: &Path, parent_path: &str) -> Vec<String> {
    let parent = paths::to_forward_slashes(parent_path);
    load(workspace)
        .relations
        .get(&parent)
        .map(|children| {
            children
                .iter()
                .filter(|c| is_inside_sidecar(&parent, c) && Path::new(c.as_str()).exists())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Workspace(PathBuf);
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Workspace {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("mochi-subdoc-{}-{tag}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("知识库")).unwrap();
            std::fs::write(root.join("知识库").join("笔记.md"), "# 父文档").unwrap();
            Self(root)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn abs(&self, rel: &str) -> String {
            format!(
                "{}/{rel}",
                paths::to_forward_slashes(&self.0.to_string_lossy())
            )
        }
        fn parent(&self) -> String {
            self.abs("知识库/笔记.md")
        }
    }

    #[test]
    fn sidecar_path_keeps_the_full_parent_name() {
        assert_eq!(
            sidecar_path("D:/ws/知识库/笔记.md"),
            "D:/ws/知识库/.笔记.md.sub",
            "要带扩展名，否则 笔记.md 和 笔记.mc 会共用一个伴生夹"
        );
        assert_eq!(sidecar_path("D:\\ws\\笔记.md"), "D:/ws/.笔记.md.sub");
    }

    #[test]
    fn creating_a_sub_document_writes_the_file_and_the_index() {
        let ws = Workspace::new("create");
        let child = create(ws.path(), &ws.parent(), "试题.mc", "# 试题\n\n1. 略").unwrap();

        assert!(child.ends_with(".笔记.md.sub/试题.mc"), "{child}");
        assert_eq!(std::fs::read_to_string(&child).unwrap(), "# 试题\n\n1. 略");
        assert_eq!(children_of(ws.path(), &ws.parent()), [child]);
    }

    /// 不给内容时补一个以文件名为标题的空文档，而不是留下 0 字节文件。
    #[test]
    fn empty_content_gets_a_title_heading() {
        let ws = Workspace::new("empty");
        let child = create(ws.path(), &ws.parent(), "摘要.mc", "").unwrap();
        assert_eq!(std::fs::read_to_string(&child).unwrap(), "# 摘要\n\n");
    }

    /// 重名要自动让路，不能覆盖已有的子文档。
    #[test]
    fn duplicate_names_get_a_numeric_suffix() {
        let ws = Workspace::new("dup");
        let first = create(ws.path(), &ws.parent(), "摘要.mc", "一").unwrap();
        let second = create(ws.path(), &ws.parent(), "摘要.mc", "二").unwrap();
        let third = create(ws.path(), &ws.parent(), "摘要.mc", "三").unwrap();

        assert!(first.ends_with("/摘要.mc"));
        assert!(second.ends_with("/摘要 1.mc"), "{second}");
        assert!(third.ends_with("/摘要 2.mc"), "{third}");
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            "一",
            "已有文件不该被覆盖"
        );
        assert_eq!(children_of(ws.path(), &ws.parent()).len(), 3);
    }

    /// 索引文件形状是硬契约：三版共读。
    #[test]
    fn the_index_file_matches_the_electron_format() {
        let ws = Workspace::new("format");
        create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();

        let raw = std::fs::read_to_string(store_path(ws.path())).unwrap();
        assert!(raw.starts_with("{\n  \"version\": \"1.0\","), "{raw}");
        assert!(raw.contains("\"relations\""));
        assert!(
            !raw.ends_with('\n'),
            "这个文件结尾没有换行（与 schedule 那批不同）"
        );
        assert!(!raw.contains("\r\n"), "必须是 LF");
        assert!(raw.contains("知识库"), "中文路径不该被转义");
    }

    #[test]
    fn a_missing_or_corrupt_index_reads_as_empty() {
        let ws = Workspace::new("corrupt");
        assert!(load(ws.path()).relations.is_empty());

        json2::write_to_file(store_path(ws.path()), "{ 这不是 JSON").unwrap();
        let store = load(ws.path());
        assert!(store.relations.is_empty());
        assert_eq!(store.version, "1.0");
    }

    /// 键顺序按插入序而非字典序——否则两版交替保存会反复重排整个文件。
    #[test]
    fn parent_keys_keep_their_insertion_order() {
        let ws = Workspace::new("order");
        for name in ["乙.md", "甲.md", "丙.md"] {
            std::fs::write(ws.0.join("知识库").join(name), "x").unwrap();
            create(ws.path(), &ws.abs(&format!("知识库/{name}")), "子.mc", "x").unwrap();
        }

        let store = load(ws.path());
        let order: Vec<String> = store.relations.parents().map(basename).collect();
        assert_eq!(order, ["乙.md", "甲.md", "丙.md"]);

        // 往返一次也不能重排
        save(ws.path(), &store).unwrap();
        let reloaded: Vec<String> = load(ws.path()).relations.parents().map(basename).collect();
        assert_eq!(reloaded, order);
    }

    #[test]
    fn mounting_an_existing_file_moves_it_into_the_sidecar() {
        let ws = Workspace::new("mount");
        let loose = ws.abs("知识库/散落的笔记.md");
        std::fs::write(&loose, "内容").unwrap();

        let mounted = mount_existing(ws.path(), &ws.parent(), &loose).unwrap();
        assert!(!Path::new(&loose).exists(), "原位置应已移走");
        assert_eq!(std::fs::read_to_string(&mounted).unwrap(), "内容");
        assert_eq!(children_of(ws.path(), &ws.parent()), [mounted]);
    }

    /// 一个文件不能同时挂在两个父文档下。
    #[test]
    fn remounting_detaches_the_previous_parent() {
        let ws = Workspace::new("remount");
        let other_parent = ws.abs("知识库/另一篇.md");
        std::fs::write(&other_parent, "# 另一篇").unwrap();

        let child = create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();
        let moved = mount_existing(ws.path(), &other_parent, &child).unwrap();

        assert!(
            children_of(ws.path(), &ws.parent()).is_empty(),
            "旧父文档下应已摘掉"
        );
        assert_eq!(children_of(ws.path(), &other_parent), [moved]);
    }

    #[test]
    fn unmounting_moves_the_file_next_to_its_parent() {
        let ws = Workspace::new("unmount");
        let child = create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();

        let dest = unmount(ws.path(), &child).unwrap().unwrap();
        assert_eq!(dest, ws.abs("知识库/试题.mc"));
        assert!(Path::new(&dest).exists());
        assert!(children_of(ws.path(), &ws.parent()).is_empty());
        assert!(
            !Path::new(&sidecar_path(&ws.parent())).exists(),
            "空掉的伴生夹应被清掉，否则文件树里留一堆隐藏空目录"
        );
    }

    #[test]
    fn unmounting_something_that_was_never_mounted_is_a_no_op() {
        let ws = Workspace::new("unmount-none");
        assert!(unmount(ws.path(), &ws.abs("知识库/笔记.md"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn renaming_the_parent_moves_the_sidecar_and_rewrites_paths() {
        let ws = Workspace::new("rename-parent");
        let child = create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();

        let new_parent = ws.abs("知识库/新名字.md");
        std::fs::rename(ws.parent(), &new_parent).unwrap();
        update_path(ws.path(), &ws.parent(), &new_parent).unwrap();

        assert!(children_of(ws.path(), &ws.parent()).is_empty());
        let moved = children_of(ws.path(), &new_parent);
        assert_eq!(moved.len(), 1, "子文档应跟着父文档走");
        assert!(moved[0].contains(".新名字.md.sub/"), "{}", moved[0]);
        assert!(Path::new(&moved[0]).exists(), "伴生夹应已整体迁移");
        assert!(!Path::new(&child).exists());
    }

    #[test]
    fn renaming_a_child_inside_the_sidecar_updates_its_entry() {
        let ws = Workspace::new("rename-child");
        let child = create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();
        let renamed = format!("{}/新试题.mc", sidecar_path(&ws.parent()));
        std::fs::rename(&child, &renamed).unwrap();

        update_path(ws.path(), &child, &renamed).unwrap();
        assert_eq!(children_of(ws.path(), &ws.parent()), [renamed]);
    }

    /// 移出伴生夹就该解除挂载，否则同一个文件会在文件夹下和父文档下各出现一次。
    #[test]
    fn moving_a_child_out_of_the_sidecar_unmounts_it() {
        let ws = Workspace::new("move-out");
        let child = create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();
        let outside = ws.abs("知识库/试题.mc");
        std::fs::rename(&child, &outside).unwrap();

        update_path(ws.path(), &child, &outside).unwrap();
        assert!(children_of(ws.path(), &ws.parent()).is_empty());
    }

    #[test]
    fn deleting_the_parent_removes_the_sidecar_and_the_relation() {
        let ws = Workspace::new("delete-parent");
        create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();

        handle_delete(ws.path(), &ws.parent()).unwrap();
        assert!(!Path::new(&sidecar_path(&ws.parent())).exists());
        assert!(load(ws.path()).relations.is_empty());
        assert!(
            Path::new(&ws.parent()).exists(),
            "级联只清索引与伴生夹，父文档本身由调用方删"
        );
    }

    #[test]
    fn deleting_a_child_drops_it_from_the_parent_list() {
        let ws = Workspace::new("delete-child");
        let first = create(ws.path(), &ws.parent(), "甲.mc", "x").unwrap();
        let second = create(ws.path(), &ws.parent(), "乙.mc", "x").unwrap();

        handle_delete(ws.path(), &first).unwrap();
        std::fs::remove_file(&first).unwrap();
        assert_eq!(children_of(ws.path(), &ws.parent()), [second]);
    }

    /// 索引里指到伴生夹外的脏条目要被过滤掉：那种文件会在文件树里重复出现。
    #[test]
    fn entries_outside_the_sidecar_are_ignored() {
        let ws = Workspace::new("stray");
        let stray = ws.abs("知识库/别处.md");
        std::fs::write(&stray, "x").unwrap();

        let mut store = load(ws.path());
        store.relations.push(&ws.parent(), stray.clone());
        save(ws.path(), &store).unwrap();

        assert!(children_of(ws.path(), &ws.parent()).is_empty());
        assert!(!is_inside_sidecar(&ws.parent(), &stray));
    }

    /// 索引里的文件已被外部删掉时静默跳过，而不是让整棵树报错。
    #[test]
    fn missing_child_files_are_skipped() {
        let ws = Workspace::new("ghost");
        let child = create(ws.path(), &ws.parent(), "试题.mc", "x").unwrap();
        std::fs::remove_file(&child).unwrap();
        assert!(children_of(ws.path(), &ws.parent()).is_empty());
    }

    #[test]
    fn unique_name_handles_dotfiles_and_extensionless_names() {
        let dir = std::env::temp_dir().join(format!("mochi-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = paths::to_forward_slashes(&dir.to_string_lossy());

        assert_eq!(unique_name(&dir, "全新.md"), "全新.md");
        assert_eq!(
            stem_of(".gitignore"),
            ".gitignore",
            "开头的点不是扩展名分隔符"
        );
        assert_eq!(stem_of("无扩展名"), "无扩展名");
        assert_eq!(stem_of("a.b.c"), "a.b");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
