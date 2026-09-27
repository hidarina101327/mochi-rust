use super::*;
use mochi_core::object_reference::ObjectKind;

fn candidate(name: &str) -> ObjectCandidate {
    ObjectCandidate::new(
        format!("mochi://open?path={name}.md"),
        name,
        "",
        ObjectKind::Document,
    )
}

#[test]
fn pending_scan_is_shared_and_completed_snapshot_is_available_before_refresh() {
    let root = Path::new("workspace-a");
    let mut cache = Cache::default();
    let scan = cache.begin_refresh(root).unwrap();
    assert!(cache.get(root).is_none());
    assert!(cache.begin_refresh(root).is_none());
    scan.send(Ok(vec![candidate("b")])).unwrap();
    assert_eq!(cache.poll(), vec![(root.to_path_buf(), true)]);
    let refresh = cache.begin_refresh(root).unwrap();
    assert_eq!(cache.get(root).unwrap(), &[candidate("b")]);
    refresh
        .send(Ok(vec![candidate("a"), candidate("b")]))
        .unwrap();
    cache.poll();
    assert_eq!(cache.get(root).unwrap(), &[candidate("b"), candidate("a")]);
}

#[test]
fn failures_keep_snapshot_and_allow_retry_but_successful_empty_scan_clears_it() {
    let root = Path::new("workspace-a");
    let mut cache = Cache::default();
    cache
        .begin_refresh(root)
        .unwrap()
        .send(Ok(vec![candidate("a")]))
        .unwrap();
    cache.poll();
    cache
        .begin_refresh(root)
        .unwrap()
        .send(Err(std::io::Error::other("unavailable")))
        .unwrap();
    assert_eq!(cache.poll(), vec![(root.to_path_buf(), false)]);
    assert_eq!(cache.get(root).unwrap(), &[candidate("a")]);
    drop(cache.begin_refresh(root).unwrap());
    assert_eq!(cache.poll(), vec![(root.to_path_buf(), false)]);
    assert_eq!(cache.get(root).unwrap(), &[candidate("a")]);
    cache.begin_refresh(root).unwrap().send(Ok(vec![])).unwrap();
    cache.poll();
    assert_eq!(cache.get(root), Some(&vec![]));
}

#[test]
fn out_of_order_workspace_results_never_mix() {
    let a = Path::new("workspace-a");
    let b = Path::new("workspace-b");
    let mut cache = Cache::default();
    let scan_a = cache.begin_refresh(a).unwrap();
    let scan_b = cache.begin_refresh(b).unwrap();
    scan_b.send(Ok(vec![candidate("b")])).unwrap();
    cache.poll();
    assert!(cache.get(a).is_none());
    scan_a.send(Ok(vec![candidate("a")])).unwrap();
    cache.poll();
    assert_eq!(cache.get(a).unwrap(), &[candidate("a")]);
    assert_eq!(cache.get(b).unwrap(), &[candidate("b")]);
}

#[test]
fn recently_used_workspaces_are_retained_with_a_bounded_cache() {
    let mut cache = Cache::default();
    for name in ["a", "b", "c", "d", "a", "e"] {
        cache
            .begin_refresh(Path::new(name))
            .unwrap()
            .send(Ok(vec![candidate(name)]))
            .unwrap();
        cache.poll();
    }
    assert_eq!(cache.entries.len(), 4);
    assert!(cache.get(Path::new("a")).is_some());
    assert!(cache.get(Path::new("b")).is_none());
}
