use super::*;

fn network(shell: &Shell) -> usize {
    shell.rows().iter().position(|r| r.name == "网络").unwrap()
}

#[test]
fn expand_all_loads_nested_folders_with_bounded_workers_and_collapse_cancels_expansion() {
    let (ws, mut shell) = opened("tree-expand-all");
    for i in 0..8 {
        ws.write(
            &format!("知识库/计算机通识/branch-{i}/nested/deep.md"),
            "deep",
        );
    }
    shell.refresh_tree();
    shell.expand_all();
    assert_eq!(shell.tree_loads.running.len(), 2);
    assert!(!shell.tree_loads.queued.is_empty());
    shell.collapse_all();
    assert!(shell.tree_loads.queued.is_empty());
    shell.wait_for_tree_loads();
    assert!(shell
        .rows()
        .iter()
        .all(|row| row.depth == 0 && !row.expanded));
    shell.expand_all();
    shell.wait_for_tree_loads();
    assert!(shell.all_expanded());
    assert_eq!(
        shell
            .rows()
            .iter()
            .filter(|row| row.name == "deep.md")
            .count(),
        8
    );
}

#[test]
fn cached_toggles_do_not_read_disk_until_refresh() {
    let (ws, mut shell) = opened("tree-cache");
    let dir = network(&shell);
    shell.toggle_loaded(dir);
    ws.write("知识库/计算机通识/网络/new.md", "new");
    for _ in 0..20 {
        shell.toggle(dir);
        assert!(!shell.tree_loading_pending(), "collapse must not read disk");
        shell.toggle(dir);
        assert!(
            !shell.tree_loading_pending(),
            "reopen must use loaded children"
        );
        assert!(shell.rows().iter().any(|r| r.name == "TCP.md"));
        assert!(!shell.rows().iter().any(|r| r.name == "new.md"));
    }
    shell.refresh_tree();
    assert!(shell.rows().iter().any(|r| r.name == "new.md"));
}

#[test]
fn rapid_toggles_deduplicate_reads_and_late_results_keep_the_folder_collapsed() {
    let (_ws, mut shell) = opened("tree-rapid");
    let dir = network(&shell);
    let before = shell.rows().to_vec();
    shell.toggle(dir);
    assert!(
        shell.rows()[dir].expanded,
        "the click is reflected before receiving disk results"
    );
    assert_eq!(shell.rows().len(), before.len());
    for _ in 0..20 {
        shell.toggle(dir);
        shell.toggle(dir);
    }
    assert_eq!(shell.tree_loads.running.len(), 1);
    assert!(shell.tree_loads.queued.is_empty());
    shell.toggle(dir);
    shell.wait_for_tree_loads();
    assert_eq!(shell.rows(), before);
    shell.toggle(dir);
    assert!(!shell.tree_loading_pending());
    assert!(shell.rows().iter().any(|r| r.name == "TCP.md"));
}

#[test]
fn pending_results_cannot_replace_a_new_scope_or_refreshed_tree() {
    let (_ws, mut shell) = opened("tree-stale");
    let dir = network(&shell);
    shell.toggle(dir);
    shell.select_type("knowledge-base");
    assert!(!shell.tree_loading_pending());
    let rows = shell.rows().to_vec();
    assert!(!shell.poll_tree_loads());
    assert_eq!(shell.rows(), rows);
    shell.select_library(0);
    let dir = network(&shell);
    shell.toggle(dir);
    shell.refresh_tree();
    assert!(!shell.tree_loading_pending());
    assert!(shell.rows().iter().any(|r| r.name == "TCP.md"));
}

#[test]
fn failed_reads_can_be_retried_and_async_insertion_preserves_selection() {
    let (ws, mut shell) = opened("tree-retry");
    let dir = network(&shell);
    // 把测试夹具挪走，制造一个确定性的读目录错误。
    let path = shell.rows()[dir].path.clone();
    let moved = path.with_file_name("network-moved");
    fs::rename(&path, &moved).unwrap();
    shell.toggle_loaded(dir);
    assert!(!shell.rows()[dir].expanded);
    assert!(
        shell.rows()[dir].has_children,
        "failed read must remain retryable"
    );
    fs::rename(&moved, &path).unwrap();
    let selected = shell
        .rows()
        .iter()
        .position(|r| r.name == "索引.md")
        .unwrap();
    shell.select(selected);
    let selected_path = shell.rows()[selected].path.clone();
    shell.toggle_loaded(dir);
    assert_eq!(shell.rows()[shell.selected().unwrap()].path, selected_path);
    assert!(shell.rows().iter().any(|r| r.name == "TCP.md"));
    drop(shell);
    drop(ws);
}
