use super::*;
#[path = "tree_performance_tests.rs"]
mod tree_performance_tests;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct TempWs(PathBuf);
impl TempWs {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("mochi-shell-{}-{tag}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn write(&self, rel: &str, content: &str) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }
}
impl Drop for TempWs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn opened(tag: &str) -> (TempWs, Shell) {
    let ws = TempWs::new(tag);
    // 此测试数据模拟已有工作区，不包含新建工作区时附带的指南资料库。
    ws.write(".mochi/libraries.json", "[]");
    ws.write("知识库/计算机通识/操作系统.md", "# 操作系统\n内容");
    ws.write("知识库/计算机通识/网络/TCP.md", "# TCP");
    ws.write("知识库/计算机通识/索引.md", "# 索引");
    let mut shell = Shell::new();
    shell.open_workspace(&ws.0, || {}).unwrap();
    (ws, shell)
}

#[test]
fn tree_drop_reorders_siblings_and_moves_documents_into_folders() {
    let (ws, mut shell) = opened("tree-drop");
    let root = ws.0.join("知识库/计算机通识");
    let operating_system = root.join("操作系统.md");
    let index = root.join("索引.md");
    let network = root.join("网络");

    shell
        .move_for_tree_drop(&index, &operating_system, TreeDropPosition::Before)
        .unwrap();
    let order = &shell.manual_sort_orders[&path_key(&root)];
    assert!(
        order.iter().position(|path| path == &path_key(&index))
            < order
                .iter()
                .position(|path| path == &path_key(&operating_system))
    );
    assert!(ws.0.join(".mochi/drag-sort.json").is_file());

    let moved = shell
        .move_for_tree_drop(&index, &network, TreeDropPosition::Into)
        .unwrap();
    assert_eq!(moved, network.join("索引.md"));
    assert!(moved.is_file());
    assert!(!index.exists());
}

#[test]
fn tree_drop_changes_only_the_requested_sibling_position() {
    let (_ws, mut shell) = opened("tree-drop-stable-order");
    let root = PathBuf::from(&shell.workspace().unwrap().libraries[0].path);
    fs::write(root.join("a.md"), "a").unwrap();
    fs::create_dir_all(root.join("b-folder")).unwrap();
    fs::write(root.join("z.md"), "z").unwrap();
    shell.refresh_tree();
    let mut expected = shell
        .rows()
        .iter()
        .filter(|r| r.depth == 0)
        .map(|r| r.path.clone())
        .collect::<Vec<_>>();
    let source = root.join("z.md");
    let target = root.join("a.md");
    expected.retain(|p| p != &source);
    let at = expected.iter().position(|p| p == &target).unwrap();
    expected.insert(at, source.clone());
    shell
        .move_for_tree_drop(&source, &target, TreeDropPosition::Before)
        .unwrap();
    let actual = shell
        .rows()
        .iter()
        .filter(|r| r.depth == 0)
        .map(|r| r.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "拖一个项目不能重排其他兄弟节点");
}

#[test]
fn tree_drop_library_order_applies_in_all_libraries_view_and_survives_reload() {
    let (ws, mut shell) = opened("tree-drop-libraries");
    let kind = shell.workspace().unwrap().libraries[0].kind.clone();
    shell.create_library(&kind, "第二库").unwrap();
    shell.create_library(&kind, "第三库").unwrap();
    shell.select_type(&kind);
    let mut expected = shell
        .rows()
        .iter()
        .filter(|r| r.depth == 0)
        .map(|r| r.path.clone())
        .collect::<Vec<_>>();
    let source = expected.pop().unwrap();
    let target = expected[0].clone();
    expected.insert(0, source.clone());
    shell
        .move_for_tree_drop(&source, &target, TreeDropPosition::Before)
        .unwrap();
    assert_eq!(
        shell
            .rows()
            .iter()
            .map(|r| r.path.clone())
            .collect::<Vec<_>>(),
        expected
    );
    let mut reopened = Shell::new();
    reopened.open_workspace(&ws.0, || {}).unwrap();
    reopened.select_type(&kind);
    assert_eq!(
        reopened
            .rows()
            .iter()
            .map(|r| r.path.clone())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn tree_drop_cannot_turn_a_registered_library_into_a_subfolder() {
    let (_ws, mut shell) = opened("tree-drop-library-protection");
    let source = PathBuf::from(&shell.workspace().unwrap().libraries[0].path);
    let kind = shell.workspace().unwrap().libraries[0].kind.clone();
    let index = shell.create_library(&kind, "目标库").unwrap();
    let target = PathBuf::from(&shell.workspace().unwrap().libraries[index].path);
    assert!(shell
        .move_for_tree_drop(&source, &target, TreeDropPosition::Into)
        .is_err());
    assert!(source.is_dir());
    assert!(!target.join(source.file_name().unwrap()).exists());
}

#[test]
fn tree_drop_keeps_source_folder_open_and_allows_moving_a_folder() {
    let (_ws, mut shell) = opened("tree-drop-folder");
    let root = PathBuf::from(&shell.workspace().unwrap().libraries[0].path);
    let network = root.join("网络");
    let tcp = network.join("TCP.md");
    let archive = root.join("归档");
    fs::create_dir_all(&network).unwrap();
    fs::write(&tcp, "TCP").unwrap();
    fs::write(network.join("UDP.md"), "UDP").unwrap();
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("说明.md"), "归档").unwrap();
    shell.refresh_tree();

    let network_row = shell
        .rows()
        .iter()
        .position(|row| row.path == network)
        .unwrap();
    shell.toggle_loaded(network_row);
    assert!(shell
        .rows()
        .iter()
        .any(|row| row.path == network && row.expanded));

    // 监听器/拖放都会走刷新：已经展开的懒加载目录应重载子项，不能只留下向下箭头。
    shell.refresh_tree();
    assert!(shell
        .rows()
        .iter()
        .any(|row| row.path == network && row.expanded));
    assert!(shell.rows().iter().any(|row| row.path == tcp));

    shell
        .move_for_tree_drop(&tcp, &archive, TreeDropPosition::Into)
        .unwrap();
    assert!(shell
        .rows()
        .iter()
        .any(|row| row.path == network && row.expanded));

    let moved_folder = shell
        .move_for_tree_drop(&network, &archive, TreeDropPosition::Into)
        .unwrap();
    assert_eq!(moved_folder, archive.join("网络"));
    assert!(moved_folder.is_dir());
    assert!(!network.exists());
}

#[test]
fn mapped_folder_roots_cannot_be_moved_into_a_library_subfolder() {
    let (ws, mut shell) = opened("mapped-folder-drop");
    let library = ws.0.join("知识库/计算机通识");
    let source = ws.0.join("external-mapped-folder");
    std::fs::create_dir_all(&source).unwrap();
    mochi_core::mapped_folders::Service::new(&ws.0)
        .add(
            &library,
            &source,
            "外部资料",
            mochi_core::mapped_folders::MappingRules::default(),
        )
        .unwrap();

    let error = shell
        .move_for_tree_drop(&source, &library.join("网络"), TreeDropPosition::Into)
        .unwrap_err();
    assert!(error.to_string().contains("虚拟入口"));
    assert!(source.is_dir());
}

#[cfg(windows)]
#[test]
fn accepting_an_approved_write_matches_windows_path_case() {
    let (ws, mut shell) = opened("approved-path-case");
    let path = ws.0.join("知识库/计算机通识/操作系统.md");
    assert!(shell.open_file(&path));
    let differently_cased = PathBuf::from(path.to_string_lossy().to_uppercase());
    shell.accept_written_text(&differently_cased, "# 已批准\n新内容");
    let buffer = shell.active_buffer_mut().unwrap();
    assert_eq!(buffer.text(), "# 已批准\n新内容");
    assert!(!buffer.dirty());
}

#[test]
fn favorite_documents_follow_folder_moves_deletion_and_workspace_switches() {
    let (ws, mut shell) = opened("favorites-lifecycle");
    let folder = ws.0.join("知识库/计算机通识/网络");
    let file = folder.join("TCP.md");
    assert!(shell.toggle_favorite(&file).unwrap());
    assert!(shell.is_favorite(&file));
    let moved = shell.rename(&folder, "协议").unwrap().join("TCP.md");
    assert!(shell.is_favorite(&moved));
    assert!(!shell.is_favorite(&file));
    shell.open_workspace(&ws.0, || {}).unwrap();
    assert!(shell.is_favorite(&moved), "重启后保留收藏");
    let other = TempWs::new("favorites-other");
    shell.open_workspace(&other.0, || {}).unwrap();
    assert!(shell.favorite_paths().is_empty());
    shell.open_workspace(&ws.0, || {}).unwrap();
    shell.delete(&moved).unwrap();
    shell.reload_favorites();
    assert!(shell.favorite_paths().is_empty());
}

#[test]
fn favorites_follow_confirmed_external_changes_and_preserve_corrupt_storage() {
    let (ws, mut shell) = opened("favorites-external");
    let from = ws.0.join("知识库/计算机通识/操作系统.md");
    let to = from.with_file_name("系统.md");
    shell.toggle_favorite(&from).unwrap();
    fs::rename(&from, &to).unwrap();
    shell.accept_external_rename(&from, &to);
    assert!(shell.is_favorite(&to));
    fs::remove_file(&to).unwrap();
    shell.accept_external_delete(&to);
    assert!(shell.favorite_paths().is_empty());
    let store = ws.0.join(".mochi/favorites.json");
    fs::write(&store, "{broken").unwrap();
    let other = ws.0.join("知识库/计算机通识/索引.md");
    assert!(shell.toggle_favorite(&other).is_err());
    assert_eq!(fs::read_to_string(store).unwrap(), "{broken");
}

#[test]
fn favorites_tree_contains_only_files_with_their_ancestors_and_defaults_open() {
    let (ws, mut shell) = opened("favorites-tree");
    let operating_system = ws.0.join("知识库/计算机通识/操作系统.md");
    let tcp = ws.0.join("知识库/计算机通识/网络/TCP.md");
    shell.toggle_favorite(&operating_system).unwrap();
    shell.toggle_favorite(&tcp).unwrap();

    // 本测试专门覆盖需要用户选择启用的旧版树形结构。默认显示方式
    // 是平铺的收藏视图（见下方测试）。
    shell.set_favorite_show_parents(true);
    shell.select_favorites();

    assert!(shell.favorites_selected());
    assert_eq!(shell.selected_library(), None);
    assert_eq!(shell.scope_type_id(), None);
    assert_eq!(
        shell.tree_root(),
        Some(shell.workspace().unwrap().root.clone())
    );
    assert!(shell.rows().iter().any(|row| row.name == "操作系统.md"));
    assert!(shell.rows().iter().any(|row| row.name == "TCP.md"));
    assert!(!shell.rows().iter().any(|row| row.name == "索引.md"));
    assert!(shell
        .rows()
        .iter()
        .filter(|row| row.is_dir)
        .all(|row| row.expanded));
}

#[test]
fn favorites_tree_spans_libraries_and_workspace_root_files() {
    let (ws, mut shell) = opened("favorites-multi-library");
    let root_file = ws.0.join("根收藏.md");
    fs::write(&root_file, "根").unwrap();
    let second_index = shell.create_library("knowledge-base", "第二库").unwrap();
    let second_root = PathBuf::from(&shell.workspace().unwrap().libraries[second_index].path);
    let second_file = second_root.join("第二收藏.md");
    fs::write(&second_file, "第二").unwrap();

    shell.toggle_favorite(&root_file).unwrap();
    shell.toggle_favorite(&second_file).unwrap();
    shell.set_favorite_show_parents(true);
    shell.select_favorites();

    let names = shell
        .rows()
        .iter()
        .map(|row| row.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"根收藏.md"));
    assert!(names.contains(&"第二库"));
    assert!(names.contains(&"第二收藏.md"));
    assert!(!names.contains(&"操作系统.md"));
}

#[test]
fn favorites_default_to_flat_files_and_keep_same_names_from_different_paths() {
    let (ws, mut shell) = opened("favorites-flat-default");
    let first = ws.0.join("知识库/计算机通识/同名.md");
    let second = ws.0.join("知识库/计算机通识/网络/同名.md");
    fs::write(&first, "第一份").unwrap();
    fs::write(&second, "第二份").unwrap();

    shell.toggle_favorite(&first).unwrap();
    shell.toggle_favorite(&second).unwrap();
    shell.select_favorites();

    assert!(!shell.show_favorite_parents());
    assert!(!shell.rows().iter().any(|row| row.is_dir));
    assert!(shell.rows().iter().all(|row| row.depth == 0));
    let same_name = shell
        .rows()
        .iter()
        .filter(|row| row.name == "同名.md")
        .collect::<Vec<_>>();
    assert_eq!(same_name.len(), 2);
    assert_ne!(same_name[0].path, same_name[1].path);
}

#[test]
fn toggling_favorite_parent_display_rebuilds_rows_without_changing_tabs_or_expansion() {
    let (ws, mut shell) = opened("favorites-flat-toggle");
    let operating_system = ws.0.join("知识库/计算机通识/操作系统.md");
    let tcp = ws.0.join("知识库/计算机通识/网络/TCP.md");
    shell.toggle_favorite(&operating_system).unwrap();
    shell.toggle_favorite(&tcp).unwrap();
    assert!(shell.open_file(&operating_system));
    let active_tab = shell.active_tab();
    let tab_count = shell.tabs().len();

    shell.select_favorites();
    assert!(shell.rows().iter().all(|row| !row.is_dir && row.depth == 0));

    shell.set_favorite_show_parents(true);
    assert!(shell.show_favorite_parents());
    assert!(shell
        .rows()
        .iter()
        .any(|row| row.name == "网络" && row.is_dir));
    assert!(shell.rows().iter().any(|row| row.name == "TCP.md"));
    assert!(shell
        .rows()
        .iter()
        .filter(|row| row.is_dir)
        .all(|row| row.expanded));
    assert_eq!(shell.active_tab(), active_tab);
    assert_eq!(shell.tabs().len(), tab_count);

    let network = shell
        .rows()
        .iter()
        .position(|row| row.name == "网络")
        .unwrap();
    shell.toggle_loaded(network);
    assert!(!shell.rows().iter().any(|row| row.name == "TCP.md"));
    shell.set_favorite_show_parents(false);
    assert!(!shell.show_favorite_parents());
    assert!(shell.rows().iter().all(|row| !row.is_dir && row.depth == 0));

    shell.set_favorite_show_parents(true);
    let network = shell.rows().iter().find(|row| row.name == "网络").unwrap();
    assert!(!network.expanded, "切换显示方式不应重置收藏目录的展开状态");
    assert!(!shell.rows().iter().any(|row| row.name == "TCP.md"));
    assert_eq!(shell.active_tab(), active_tab);
    assert_eq!(shell.tabs().len(), tab_count);
}

#[test]
fn favorites_expansion_refresh_cancellation_and_scope_switch_round_trip() {
    let (ws, mut shell) = opened("favorites-expansion");
    let tcp = ws.0.join("知识库/计算机通识/网络/TCP.md");
    shell.toggle_favorite(&tcp).unwrap();
    shell.set_favorite_show_parents(true);
    shell.select_favorites();
    let network = shell
        .rows()
        .iter()
        .position(|row| row.name == "网络")
        .unwrap();
    shell.toggle_loaded(network);
    assert!(!shell.rows().iter().any(|row| row.name == "TCP.md"));

    shell.refresh_tree();
    assert!(!shell.rows().iter().any(|row| row.name == "TCP.md"));
    shell.toggle_favorite(&tcp).unwrap();
    assert!(shell.rows().is_empty());

    shell.leave_favorites();
    assert!(!shell.favorites_selected());
    assert_eq!(shell.selected_library(), Some(0));
    assert!(shell.rows().iter().any(|row| row.name == "操作系统.md"));
}

#[test]
fn renaming_a_favorite_updates_the_visible_favorites_tree() {
    let (ws, mut shell) = opened("favorites-rename-tree");
    let old = ws.0.join("知识库/计算机通识/网络/TCP.md");
    shell.toggle_favorite(&old).unwrap();
    shell.select_favorites();
    let new = shell.rename(&old, "TCP-v2.md").unwrap();

    assert!(shell.is_favorite(&new));
    assert!(!shell.is_favorite(&old));
    assert!(shell.rows().iter().any(|row| row.name == "TCP-v2.md"));
    assert!(!shell.rows().iter().any(|row| row.name == "TCP.md"));
}

#[test]
fn adding_to_an_empty_favorites_tree_reveals_the_new_document() {
    let (ws, mut shell) = opened("favorites-empty-add");
    shell.select_favorites();
    assert!(shell.rows().is_empty());
    let tcp = ws.0.join("知识库/计算机通识/网络/TCP.md");
    assert!(shell.toggle_favorite(&tcp).unwrap());
    assert!(shell.rows().iter().any(|row| row.path == tcp));
    shell.reload_favorites();
    assert!(shell.rows().iter().any(|row| row.path == tcp));
}

#[test]
fn closing_the_last_file_keeps_favorites_available_beside_the_empty_file_area() {
    let (ws, mut shell) = opened("favorites-close-file");
    shell.open_special(
        TabKind::Settings {
            tab: "general".into(),
            section: "appearance".into(),
        },
        "设置",
    );
    let path = ws.0.join("知识库/计算机通识/操作系统.md");
    assert!(shell.open_file(&path));
    shell.toggle_favorite(&path).unwrap();
    shell.select_favorites();
    shell.close_tab(shell.active_tab().unwrap());
    assert!(shell.favorites_selected());
    assert!(shell.active().is_none());
    assert!(shell.rows().iter().any(|row| row.path == path));
    shell.select_tab(0);
    assert!(!shell.favorites_selected());
    assert!(matches!(
        shell.active().unwrap().kind,
        TabKind::Settings { .. }
    ));
}

#[test]
fn external_updates_do_not_get_overwritten_by_old_buffers() {
    let (ws, mut shell) = opened("external-conflict");
    let path = ws.0.join("知识库/计算机通识/操作系统.md");
    shell.open_file(&path);
    fs::write(&path, "磁盘新版本").unwrap();
    assert!(shell.save_active());
    assert_eq!(shell.active_buffer_mut().unwrap().text(), "磁盘新版本");
    shell.active_buffer_mut().unwrap().insert("本地修改");
    let local = shell.active_buffer_mut().unwrap().text().to_owned();
    fs::write(&path, "另一程序的更新").unwrap();
    assert!(!shell.save_active());
    assert_eq!(shell.active_buffer_mut().unwrap().text(), local);
    assert_eq!(fs::read_to_string(&path).unwrap(), "另一程序的更新");
    assert!(shell.force_save_file(&path));
    assert_eq!(fs::read_to_string(&path).unwrap(), local);
}

#[test]
fn large_markdown_keeps_the_full_editable_document_experience() {
    let (ws, mut shell) = opened("large-markdown");
    let path = ws.0.join("知识库/计算机通识/六兆文档.md");
    let source = "## 大文档\n正文内容🙂\n".repeat(300_000);
    fs::write(&path, &source).unwrap();

    assert!(shell.open_file(&path));
    let tab = shell.active().unwrap();
    assert!(matches!(tab.kind, TabKind::File { .. }));
    assert_eq!(tab.buffer().unwrap().text(), source);
    assert!(!tab.buffer().unwrap().dirty());
}

#[test]
fn rich_markdown_save_upgrades_to_mc_and_reports_the_path_change() {
    let (ws, mut shell) = opened("document-format-upgrade");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let target = source.with_extension("mc");
    assert!(shell.open_file(&source));
    let source_len = shell.active_buffer_mut().unwrap().text().len();
    let source_sub = PathBuf::from(mochi_core::sub_documents::sidecar_path(
        &source.to_string_lossy(),
    ));
    fs::create_dir_all(&source_sub).unwrap();
    fs::write(source_sub.join("子文档.md"), "子文档").unwrap();
    shell
        .active_buffer_mut()
        .unwrap()
        .replace_range(0..source_len, r#"<span style="color: red">内容</span>"#);

    assert!(shell.save_active());
    assert!(!source.exists(), "升级后旧的 Markdown 路径应被移走");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        r#"<span style="color: red">内容</span>"#
    );
    let target_sub = PathBuf::from(mochi_core::sub_documents::sidecar_path(
        &target.to_string_lossy(),
    ));
    assert!(
        target_sub.join("子文档.md").is_file(),
        "子文档伴生夹应随升级迁移"
    );
    assert!(!source_sub.exists());
    assert_eq!(shell.active().unwrap().path(), Some(target.as_path()));
    assert!(!shell.active().unwrap().dirty());
    assert_eq!(shell.take_document_format_changes(), vec![(source, target)]);
    assert!(shell.take_document_format_changes().is_empty());
}

#[test]
fn existing_mc_target_rejects_upgrade_without_overwriting_or_cleaning_buffer() {
    let (ws, mut shell) = opened("document-format-upgrade-collision");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let target = source.with_extension("mc");
    let original = fs::read_to_string(&source).unwrap();
    fs::write(&target, "目标文件原内容").unwrap();
    assert!(shell.open_file(&source));
    shell.active_buffer_mut().unwrap().replace_range(
        0..original.len(),
        r#"<details><summary>内容</summary>正文</details>"#,
    );

    assert!(!shell.save_active());
    assert_eq!(fs::read_to_string(&source).unwrap(), original);
    assert_eq!(fs::read_to_string(&target).unwrap(), "目标文件原内容");
    assert_eq!(shell.active().unwrap().path(), Some(source.as_path()));
    assert!(shell.active().unwrap().dirty());
    assert!(shell.take_document_format_changes().is_empty());
}

#[test]
fn failed_format_migration_can_retry_after_more_edits_and_still_detect_external_changes() {
    let (ws, mut shell) = opened("document-format-upgrade-retry");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let comment = mochi_core::sidecars::comment_sidecar_path(&source.to_string_lossy());
    fs::create_dir_all(Path::new(&comment).parent().unwrap()).unwrap();
    fs::write(&comment, "broken comment JSON").unwrap();
    assert!(shell.open_file(&source));
    shell
        .active_buffer_mut()
        .unwrap()
        .insert("<span style=\"color: red\">颜色</span>\n");
    assert!(!shell.save_active());
    assert!(shell.active().unwrap().dirty());
    assert!(
        !shell.has_disk_conflict(&source),
        "our own completed write is not an external conflict"
    );
    let written = fs::read_to_string(&source).unwrap();
    shell.active_buffer_mut().unwrap().insert("继续编辑\n");
    fs::write(&source, "另一个程序的修改").unwrap();
    assert!(!shell.save_active());
    assert_eq!(fs::read_to_string(&source).unwrap(), "另一个程序的修改");
    fs::write(&source, written).unwrap();
    fs::write(&comment, r#"{"comments":[]}"#).unwrap();
    let expected = shell.active_buffer_mut().unwrap().text().to_owned();
    assert!(shell.save_active(), "{}", shell.status());
    assert_eq!(
        fs::read_to_string(source.with_extension("mc")).unwrap(),
        expected
    );
    assert!(!source.exists());
    assert!(!shell.active().unwrap().dirty());
}

#[test]
fn format_upgrade_persists_self_links_and_other_document_references() {
    let (ws, mut shell) = opened("document-format-upgrade-links");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let reference = ws.0.join("知识库/计算机通识/索引.md");
    fs::write(&source, "# 操作系统\n\n[自己](操作系统.md)\n").unwrap();
    fs::write(&reference, "[正文](操作系统.md)\n").unwrap();
    shell
        .workspace()
        .unwrap()
        .index
        .index_single_file(&source)
        .unwrap();
    shell
        .workspace()
        .unwrap()
        .index
        .index_single_file(&reference)
        .unwrap();
    shell.toggle_favorite(&source).unwrap();
    assert!(shell.open_file(&source));
    shell
        .active_buffer_mut()
        .unwrap()
        .insert("<span style=\"color: red\">颜色</span>\n\n");
    assert!(shell.save_active(), "{}", shell.status());
    let target = source.with_extension("mc");
    let content = fs::read_to_string(&target).unwrap();
    assert!(content.contains("[自己](操作系统.mc)"), "{content}");
    assert_eq!(content, shell.active_buffer_mut().unwrap().text());
    assert_eq!(
        fs::read_to_string(&reference).unwrap(),
        "[正文](操作系统.mc)\n"
    );
    assert!(shell.is_favorite(&target));
    assert!(!shell.is_favorite(&source));
    assert!(!shell.active().unwrap().dirty());
}

#[test]
fn format_upgrade_follows_mochi_links_in_self_other_and_dirty_buffers() {
    let (ws, mut shell) = opened("document-format-upgrade-mochi-links");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let reference = ws.0.join("知识库/计算机通识/索引.md");
    let dirty_path = ws.0.join("知识库/计算机通识/脏缓冲区.md");
    let old_url = format!(
        "{}&label=%E8%87%AA%E5%B7%B1&view=card",
        mochi_core::mochi_url::build_mochi_resource_url(
            &source,
            mochi_core::mochi_url::ResourceKind::File,
            Some(&ws.0),
        )
    );
    fs::write(&source, format!("# 操作系统\n\n{old_url}\n")).unwrap();
    fs::write(&reference, format!("{old_url}\n")).unwrap();
    fs::write(&dirty_path, "# 脏缓冲区\n").unwrap();
    let index = &shell.workspace().unwrap().index;
    index.index_single_file(&source).unwrap();
    index.index_single_file(&reference).unwrap();
    index.index_single_file(&dirty_path).unwrap();

    assert!(shell.open_file(&source));
    assert!(shell.open_file_with_mode(&dirty_path, true));
    shell
        .active_buffer_mut()
        .unwrap()
        .insert(&format!("\n{old_url}\n"));
    let source_tab = shell
        .tabs()
        .iter()
        .position(|tab| {
            tab.path()
                .is_some_and(|open| same_native_path(open, source.as_path()))
        })
        .unwrap();
    shell.select_tab(source_tab);
    shell
        .active_buffer_mut()
        .unwrap()
        .insert("<span style=\"color: red\">颜色</span>\n");

    assert!(shell.save_active(), "{}", shell.status());
    let target = source.with_extension("mc");
    let parse_path = |raw: &str| {
        mochi_core::object_reference::ObjectReference::parse(raw.trim())
            .and_then(|reference| reference.path)
    };
    let target_content = fs::read_to_string(&target).unwrap();
    assert_eq!(
        parse_path(
            target_content
                .lines()
                .find(|line| line.starts_with("mochi://"))
                .unwrap()
        ),
        Some("知识库/计算机通识/操作系统.mc".into())
    );
    assert_eq!(
        parse_path(&fs::read_to_string(&reference).unwrap()),
        Some("知识库/计算机通识/操作系统.mc".into())
    );

    let dirty_tab = shell
        .tabs()
        .iter()
        .find(|tab| {
            tab.path()
                .is_some_and(|open| same_native_path(open, dirty_path.as_path()))
        })
        .unwrap();
    assert!(dirty_tab.dirty(), "脏缓冲区的其它编辑应保留");
    assert_eq!(
        parse_path(
            dirty_tab
                .buffer()
                .unwrap()
                .text()
                .lines()
                .find(|line| line.starts_with("mochi://"))
                .unwrap()
        ),
        Some("知识库/计算机通识/操作系统.mc".into())
    );
    assert!(!source.exists());
}

#[test]
fn rename_link_maintenance_preserves_the_existing_document_format() {
    let (ws, mut shell) = opened("document-format-rename-links");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    fs::write(
        &source,
        "<span style=\"color: red\">颜色</span>\n\n[自己](操作系统.md)\n",
    )
    .unwrap();
    assert!(shell.open_file(&source));
    let target = shell.rename(&source, "新名称.md").unwrap();
    assert!(target.exists());
    assert!(!target.with_extension("mc").exists());
    assert!(fs::read_to_string(&target)
        .unwrap()
        .contains("[自己](新名称.md)"));
    assert_eq!(shell.active().unwrap().path(), Some(target.as_path()));
    assert!(shell.take_document_format_changes().is_empty());
}

#[test]
fn companion_target_collision_rejects_upgrade_before_writing_markdown() {
    let (ws, mut shell) = opened("document-format-upgrade-companion-collision");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let target = source.with_extension("mc");
    let original = fs::read_to_string(&source).unwrap();
    let source_sub = PathBuf::from(mochi_core::sub_documents::sidecar_path(
        &source.to_string_lossy(),
    ));
    let target_sub = PathBuf::from(mochi_core::sub_documents::sidecar_path(
        &target.to_string_lossy(),
    ));
    fs::create_dir_all(&source_sub).unwrap();
    fs::write(source_sub.join("子文档.md"), "源子文档").unwrap();
    fs::create_dir_all(&target_sub).unwrap();
    fs::write(target_sub.join("existing.md"), "目标子文档").unwrap();
    assert!(shell.open_file(&source));
    let source_len = shell.active_buffer_mut().unwrap().text().len();
    shell.active_buffer_mut().unwrap().replace_range(
        0..source_len,
        r#"<details><summary>内容</summary>正文</details>"#,
    );

    assert!(!shell.save_active());
    assert_eq!(fs::read_to_string(&source).unwrap(), original);
    assert!(source.exists());
    assert!(!target.exists());
    assert_eq!(
        fs::read_to_string(source_sub.join("子文档.md")).unwrap(),
        "源子文档"
    );
    assert_eq!(
        fs::read_to_string(target_sub.join("existing.md")).unwrap(),
        "目标子文档"
    );
    assert!(shell.active().unwrap().dirty());
    assert!(shell.take_document_format_changes().is_empty());
}

#[test]
fn workspace_switch_discards_unconsumed_document_format_changes() {
    let (ws, mut shell) = opened("document-format-change-clear");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    assert!(shell.open_file(&source));
    let source_len = shell.active_buffer_mut().unwrap().text().len();
    shell
        .active_buffer_mut()
        .unwrap()
        .replace_range(0..source_len, r#"<img src="x.png" width="320">"#);
    assert!(shell.save_active());
    let other = TempWs::new("document-format-change-other");
    shell.open_workspace(&other.0, || {}).unwrap();
    assert!(shell.take_document_format_changes().is_empty());
}

#[test]
fn removing_deleted_tabs_never_recreates_dirty_files() {
    let (ws, mut shell) = opened("deleted-dirty");
    let path = ws.0.join("知识库/计算机通识/操作系统.md");
    shell.open_file(&path);
    shell.active_buffer_mut().unwrap().insert("未保存");
    fs::remove_file(&path).unwrap();
    shell.forget_tabs_under(&path);
    assert!(!path.exists());
    assert!(shell
        .tabs()
        .iter()
        .all(|t| t.path() != Some(path.as_path())));
}

#[test]
fn rename_updates_dirty_reference_buffer_without_saving_unrelated_edits() {
    let (ws, mut shell) = opened("rename-dirty-link");
    let target = ws.0.join("知识库/计算机通识/操作系统.md");
    let reference = ws.0.join("知识库/计算机通识/索引.md");
    fs::write(&reference, "[[操作系统]]\r\n").unwrap();
    shell
        .workspace()
        .unwrap()
        .index
        .index_single_file(&reference)
        .unwrap();
    shell.open_file(&reference);
    shell.active_buffer_mut().unwrap().insert("尚未保存\r\n");
    shell.rename(&target, "系统原理.md").unwrap();
    assert_eq!(
        shell.active_buffer_mut().unwrap().text(),
        "尚未保存\r\n[[系统原理]]\r\n"
    );
    assert!(shell.active_buffer_mut().unwrap().dirty());
    assert_eq!(fs::read_to_string(reference).unwrap(), "[[操作系统]]\r\n");
}

#[test]
fn opening_a_workspace_populates_libraries_and_tree() {
    let (_ws, shell) = opened("open");
    let state = shell.workspace().expect("应已打开");
    assert_eq!(state.libraries.len(), 1, "知识库/ 下的目录应被自动登记");
    assert_eq!(state.libraries[0].name, "计算机通识");
    assert!(state.indexed_files >= 3, "三个 .md 应进索引");
    assert!(shell.status().contains("个库"), "{}", shell.status());
}

/// 折叠状态下只看得到顶层，展开后才多出子项。
#[test]
fn tree_starts_collapsed_and_expands_on_toggle() {
    let (_ws, mut shell) = opened("expand");
    let top: Vec<&str> = shell.rows().iter().map(|r| r.name.as_str()).collect();
    // 目录优先，其余按系统区域排序（拼音：操 cāo < 索 suǒ）
    assert_eq!(top, ["网络", "操作系统.md", "索引.md"]);

    let dir_row = shell.rows().iter().position(|r| r.name == "网络").unwrap();
    shell.toggle_loaded(dir_row);

    let names: Vec<&str> = shell.rows().iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"TCP.md"), "展开后应看到子项: {names:?}");
    assert!(shell.rows()[dir_row].expanded);
    assert_eq!(shell.rows()[dir_row + 1].depth, 1, "子项缩进层级应 +1");
}

#[test]
fn refresh_attaches_new_subdocuments_to_their_parent_document() {
    let (ws, mut shell) = opened("subdocument-tree");
    let parent = ws.0.join("知识库/计算机通识/操作系统.md");
    let child =
        mochi_core::sub_documents::create(&ws.0, &parent.to_string_lossy(), "摘要.md", "").unwrap();

    shell.refresh_tree();
    let parent_row = shell
        .rows()
        .iter()
        .position(|row| row.path == parent)
        .expect("父文档应留在文件树中");
    assert!(
        shell.rows()[parent_row].has_children,
        "父文档应显示展开箭头"
    );
    shell.toggle_loaded(parent_row);

    let child_row = shell
        .rows()
        .iter()
        .find(|row| row.path == PathBuf::from(&child))
        .expect("创建后刷新文件树应显示子文档");
    assert_eq!(child_row.depth, 1, "子文档应缩进显示在父文档下");
}

#[test]
fn toggling_twice_returns_to_the_original_rows() {
    let (_ws, mut shell) = opened("toggle-twice");
    let before = shell.rows().len();
    let dir = shell.rows().iter().position(|r| r.is_dir).unwrap();
    shell.toggle_loaded(dir);
    assert!(shell.rows().len() > before);
    shell.toggle_loaded(dir);
    assert_eq!(shell.rows().len(), before);
}

#[test]
fn toggling_a_file_row_does_nothing() {
    let (_ws, mut shell) = opened("toggle-file");
    let file = shell.rows().iter().position(|r| !r.is_dir).unwrap();
    let before = shell.rows().len();
    shell.toggle_loaded(file);
    assert_eq!(shell.rows().len(), before);
}

/// 展开状态按路径记录；刷新后不应错位。这是按下标记录时经常出现的问题。
#[test]
fn refresh_preserves_expansion_and_selection() {
    let (ws, mut shell) = opened("refresh");
    let dir = shell.rows().iter().position(|r| r.name == "网络").unwrap();
    shell.toggle_loaded(dir);
    let target = shell
        .rows()
        .iter()
        .position(|r| r.name == "TCP.md")
        .unwrap();
    shell.select(target);
    let selected_path = shell.rows()[target].path.clone();

    // 外部新增一个文件后刷新
    ws.write("知识库/计算机通识/新增.md", "x");
    shell.refresh_tree();

    assert!(
        shell.rows().iter().any(|r| r.name == "新增.md"),
        "新文件没进树"
    );
    assert!(
        shell.rows().iter().any(|r| r.name == "TCP.md"),
        "展开状态丢了"
    );
    assert_eq!(
        shell.rows()[shell.selected().unwrap()].path,
        selected_path,
        "刷新后选中项跑到别的文件上了"
    );
}

#[test]
fn expand_all_and_collapse_all_round_trip() {
    let (_ws, mut shell) = opened("expand-all");
    assert!(!shell.all_expanded());
    shell.expand_all();
    shell.wait_for_tree_loads();
    assert!(shell.all_expanded());
    assert!(shell.rows().iter().any(|r| r.name == "TCP.md"));
    shell.collapse_all();
    assert!(!shell.all_expanded());
    assert!(!shell.rows().iter().any(|r| r.name == "TCP.md"));
}

#[test]
fn creating_a_file_writes_the_heading_template_and_expands_the_parent() {
    let (ws, mut shell) = opened("create-file");
    let parent = ws.0.join("知识库/计算机通识/网络");
    let path = shell.create_file(&parent, "HTTP.mc").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "# HTTP\n\n");
    assert!(
        shell.rows().iter().any(|r| r.name == "HTTP.mc"),
        "新文件要立刻出现在树里（父目录被展开）"
    );
    assert!(
        shell.create_file(&parent, "HTTP.mc").is_err(),
        "同名要拒绝，不能覆盖"
    );
}

#[test]
fn duplicating_a_document_uses_the_source_name_and_never_overwrites() {
    let (ws, mut shell) = opened("duplicate-document");
    let source = ws.0.join("知识库/计算机通识/操作系统.md");
    let first = shell.duplicate_file(&source).unwrap();
    assert_eq!(first.file_name().unwrap(), "操作系统 1.md");
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        fs::read_to_string(&source).unwrap()
    );

    let second = shell.duplicate_file(&source).unwrap();
    assert_eq!(second.file_name().unwrap(), "操作系统 2.md");
    assert!(shell.rows().iter().any(|row| row.path == first));
    assert!(shell.rows().iter().any(|row| row.path == second));
    assert!(
        !PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &first.to_string_lossy()
        ))
        .exists(),
        "普通副本不应创建或复用原文档的子文档伴生夹"
    );
}

#[test]
fn reload_file_refreshes_an_open_canvas_after_an_external_write() {
    use crate::ui::viewer;

    let (ws, mut shell) = opened("reload-canvas");
    let parent = ws.0.join("知识库/计算机通识");
    let path = shell.create_canvas(&parent, "漏洞关系.mcanvas").unwrap();
    assert!(shell.open_file(&path));
    assert!(matches!(
        &shell.active().unwrap().kind,
        TabKind::Viewer {
            content: viewer::Content::Canvas(state),
            ..
        } if state.document.cards.is_empty()
    ));

    let mut updated = mochi_core::canvas::CanvasDocument::empty();
    updated
        .add_mochi_reference(
            "mochi://open?path=knowledge%2Fweb%2Fsql.md&kind=file&label=SQL%20注入",
        )
        .unwrap();
    fs::write(&path, mochi_core::canvas::serialize(&updated).unwrap()).unwrap();
    shell.reload_file(&path);

    assert!(matches!(
        &shell.active().unwrap().kind,
        TabKind::Viewer {
            content: viewer::Content::Canvas(state),
            ..
        } if state.document.cards.len() == 1
    ));
}

#[test]
fn base_files_save_reopen_and_preserve_dirty_edits_on_disk_conflict_and_rename() {
    use crate::ui::{base_view, viewer};
    let (ws, mut shell) = opened("base-files");
    let parent = ws.0.join("知识库/计算机通识");
    let path = shell.create_file(&parent, "数据.mcb").unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(mochi_core::base::parse_base_document(&raw).is_ok());
    assert!(shell.open_file(&path));
    assert!(shell.active().unwrap().buffer().is_none());
    if let TabKind::Viewer {
        content: viewer::Content::Base(state),
        ..
    } = &mut shell.active_mut().unwrap().kind
    {
        state.activate(base_view::Hit::ToggleEditing);
        state.activate(base_view::Hit::NewRecord);
        state
            .submit(base_view::Edit::Cell(0, 0), "保留内容")
            .unwrap();
    } else {
        panic!("expected base viewer")
    }
    assert!(shell.active().unwrap().dirty());
    fs::write(&path, format!("{raw}\n")).unwrap();
    assert!(!shell.save_active());
    assert!(shell.has_disk_conflict(&path));
    assert!(shell.active().unwrap().dirty());
    assert!(shell.force_save_file(&path));
    assert!(!shell.active().unwrap().dirty());
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("保留内容"));
    if let TabKind::Viewer {
        content: viewer::Content::Base(state),
        ..
    } = &mut shell.active_mut().unwrap().kind
    {
        state
            .submit(base_view::Edit::Cell(0, 0), "改名后保留")
            .unwrap();
    }
    let target = shell.rename(&path, "新名称.mcb").unwrap();
    assert!(shell.active().unwrap().dirty());
    shell.save_dirty_tabs().unwrap();
    shell.close_tab(shell.active_tab().unwrap());
    assert!(shell.open_file(&target));
    if let TabKind::Viewer {
        content: viewer::Content::Base(state),
        ..
    } = &shell.active().unwrap().kind
    {
        assert!(state.saved_raw.contains("改名后保留"));
        assert!(!state.dirty);
    } else {
        panic!("expected base viewer")
    }
    fs::write(&target, "invalid").unwrap();
    assert!(!shell.refresh_visible_files(None));
    assert!(
        matches!(&shell.active().unwrap().kind,TabKind::Viewer{content:viewer::Content::Base(state),..}if state.saved_raw.contains("改名后保留")&&!state.error.is_empty())
    );
}

#[test]
fn renaming_a_directory_moves_the_open_tabs_under_it() {
    let (ws, mut shell) = opened("rename-dir");
    let dir = ws.0.join("知识库/计算机通识/网络");
    let file = dir.join("TCP.md");
    assert!(shell.open_file(&file));
    let to = shell.rename(&dir, "网络协议").unwrap();
    assert!(to.ends_with("网络协议"));
    assert_eq!(
        shell.active().unwrap().path(),
        Some(to.join("TCP.md").as_path())
    );
    // 树里也换了名
    assert!(shell.rows().iter().any(|r| r.name == "网络协议"));
}

#[test]
fn renaming_a_file_updates_its_tab_title() {
    let (ws, mut shell) = opened("rename-file");
    let file = ws.0.join("知识库/计算机通识/操作系统.md");
    assert!(shell.open_file(&file));
    shell.rename(&file, "OS.md").unwrap();
    assert_eq!(shell.active().unwrap().title, "OS.md");
    assert!(!file.exists());
}

#[test]
fn deleting_closes_the_tabs_of_the_deleted_files() {
    let (ws, mut shell) = opened("delete");
    let dir = ws.0.join("知识库/计算机通识/网络");
    assert!(shell.open_file(&dir.join("TCP.md")));
    assert!(shell.open_file(&ws.0.join("知识库/计算机通识/操作系统.md")));
    assert_eq!(shell.tabs().len(), 2);
    shell.delete(&dir).unwrap();
    assert_eq!(
        shell.tabs().len(),
        1,
        "目录下的标签要关掉，留着会在保存时把文件写回来"
    );
    assert!(!dir.exists());
}

#[test]
fn empty_shell_is_safe_to_poke() {
    let mut shell = Shell::new();
    assert!(!shell.has_workspace());
    assert!(shell.rows().is_empty());
    shell.toggle_loaded(0);
    shell.select(0);
    shell.expand_all();
    shell.collapse_all();
    shell.refresh_tree();
    assert!(shell.tree_root().is_none());
}

#[test]
fn opening_a_missing_directory_fails_without_corrupting_state() {
    let mut shell = Shell::new();
    let missing = std::env::temp_dir().join("mochi-shell-does-not-exist-4f1a");
    assert!(shell.open_workspace(&missing, || {}).is_err());
    assert!(!shell.has_workspace(), "失败后不该留下半开的状态");
}

#[test]
fn desktop_library_entry_opens_a_blank_tab_without_following_an_existing_document() {
    let (ws, mut shell) = opened("desktop-library-landing");
    let a = shell.create_library("knowledge-base", "甲").unwrap();
    let b = shell.create_library("knowledge-base", "乙").unwrap();
    let path =
        PathBuf::from(&shell.workspace.as_ref().unwrap().libraries[a].path).join("已有文档.md");
    fs::write(&path, "# 文档").unwrap();
    assert!(shell.open_file(&path));
    let file_tab = shell.active_tab().unwrap();
    shell.open_library_landing(b);
    let landing = shell.active_tab().unwrap();
    assert_ne!(landing, file_tab);
    assert!(matches!(
        shell.active().unwrap().kind,
        TabKind::Library { .. }
    ));
    assert_eq!(shell.selected_library(), Some(b));
    assert!(shell.selected.is_none());
    shell.select_tab(file_tab);
    assert_eq!(shell.active().unwrap().path(), Some(path.as_path()));
    shell.select_tab(landing);
    assert_eq!(shell.selected_library(), Some(b));
    assert!(shell.active().unwrap().path().is_none());
    drop(ws);
}
