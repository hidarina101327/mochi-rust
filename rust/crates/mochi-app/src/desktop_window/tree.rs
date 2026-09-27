//! 侧栏树适配器：后台浅层加载、缓存展开状态、共享行数据和渲染器。
use super::*;
use crate::ui::{
    sidebar::{self, SidebarHit, SidebarModel},
    widgets::TextField,
};
use mochi_core::{desktop_cards::Module, domain::FileNode};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};
pub(super) const TIMER_ID: usize = 0x4d44_5452;
struct Job {
    path: PathBuf,
    receiver: Receiver<std::result::Result<Vec<FileNode>, String>>,
}
#[derive(Default)]
pub(super) struct Tree {
    identity: String,
    nodes: Vec<FileNode>,
    expanded: HashSet<PathBuf>,
    jobs: Vec<Job>,
    queue: VecDeque<PathBuf>,
}
pub(super) fn enabled(view: &View) -> bool {
    view.module == Some(Module::Knowledge) && view.presentation.expand_libraries
}
pub(super) fn style_key(view: &View, path: &str) -> String {
    let p = Path::new(path);
    let relative = p
        .strip_prefix(&view.workspace)
        .unwrap_or(p)
        .to_string_lossy();
    mochi_core::desktop_cards::item_key("", &relative)
}
fn find<'a>(nodes: &'a mut [FileNode], path: &Path) -> Option<&'a mut FileNode> {
    for n in nodes {
        if Path::new(&n.path) == path {
            return Some(n);
        }
        if let Some(children) = n.children.as_mut() {
            if let Some(found) = find(children, path) {
                return Some(found);
            }
        }
    }
    None
}
pub(super) fn sync(s: &mut State) {
    if !enabled(&s.view) {
        return;
    }
    let roots: Vec<_> = s
        .view
        .rows
        .iter()
        .filter(|r| r.meta.depth == 0 && r.meta.directory)
        .collect();
    let identity = format!(
        "{}:{}:{:?}",
        s.view.workspace.display(),
        s.view.page_id,
        roots
            .iter()
            .map(|r| (&r.meta.path, &r.title))
            .collect::<Vec<_>>()
    );
    if identity != s.tree.identity {
        s.tree = Tree {
            identity,
            nodes: roots
                .into_iter()
                .map(|r| {
                    let path = s
                        .view
                        .workspace
                        .join(&r.meta.path)
                        .to_string_lossy()
                        .into_owned();
                    FileNode {
                        id: path.clone(),
                        path,
                        name: r.title.clone(),
                        kind: "directory".into(),
                        icon: Some(r.meta.icon.clone()),
                        lazy: true,
                        ..Default::default()
                    }
                })
                .collect(),
            ..Default::default()
        };
    }
    project(s);
}
fn project(s: &mut State) {
    s.view.tree_rows = crate::shell::desktop_tree_rows(&s.tree.nodes, &s.tree.expanded);
    s.view.tree_loading = s
        .tree
        .jobs
        .iter()
        .map(|j| j.path.clone())
        .chain(s.tree.queue.iter().cloned())
        .collect();
}
pub(super) fn toggle(s: &mut State, path: &str) {
    let path = PathBuf::from(path);
    if !s.tree.expanded.remove(&path) {
        s.tree.expanded.insert(path.clone());
        if find(&mut s.tree.nodes, &path).is_some_and(|n| n.lazy)
            && !s.tree.jobs.iter().any(|j| j.path == path)
            && !s.tree.queue.contains(&path)
        {
            s.tree.queue.push_back(path);
        }
    }
    launch(s);
    project(s);
}
fn launch(s: &mut State) {
    while s.tree.jobs.len() < 2 {
        let Some(path) = s.tree.queue.pop_front() else {
            break;
        };
        if !s.tree.expanded.contains(&path) {
            continue;
        }
        let root = s.view.workspace.clone();
        let target = path.clone();
        let mode = crate::ui::settings_values::text("sidebar.sortOrder", "manual");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::shell::desktop_tree_children(&root, &target, &mode)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        s.tree.jobs.push(Job { path, receiver: rx });
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    if !enabled(&s.view) {
        return false;
    }
    let mut done = vec![];
    for (index, job) in s.tree.jobs.iter().enumerate() {
        match job.receiver.try_recv() {
            Ok(result) => done.push((index, result)),
            Err(mpsc::TryRecvError::Disconnected) => {
                done.push((index, Err("目录读取已中断".into())))
            }
            _ => {}
        }
    }
    if done.is_empty() {
        return false;
    }
    for (index, result) in done.into_iter().rev() {
        let job = s.tree.jobs.remove(index);
        match result {
            Ok(children) => {
                if let Some(n) = find(&mut s.tree.nodes, &job.path) {
                    n.children = Some(children);
                    n.lazy = false;
                }
            }
            Err(error) => {
                s.tree.expanded.remove(&job.path);
                s.view.empty = error;
                s.view.error = true;
            }
        }
    }
    launch(s);
    project(s);
    true
}
fn model<'a>(view: &'a View, search: &'a TextField, hover: Option<&Hit>) -> SidebarModel<'a> {
    let path = match hover {
        Some(Hit::TreeOpen(path) | Hit::TreeToggle(path)) => Some(Path::new(path)),
        _ => None,
    };
    SidebarModel {
        icons: None,
        title: "",
        rows: &view.tree_rows,
        loading_paths: view.tree_loading.iter().map(PathBuf::as_path).collect(),
        active_path: None,
        selected_row: None,
        hover_row: path.and_then(|p| view.tree_rows.iter().position(|r| r.path == p)),
        search,
        search_focused: false,
        all_expanded: false,
        editing: None,
        drop_indicator: None,
    }
}
fn geometry(spec: &Spec, view: &View, a: Rect) -> Rect {
    let body = painting::content_rect(spec, view, a);
    Rect::new(
        body.left,
        body.top - sidebar::HEADER_HEIGHT,
        body.right,
        body.bottom,
    )
}
pub(super) fn scroll_max(spec: &Spec, view: &View, a: Rect) -> usize {
    let search = TextField::new("");
    sidebar::layout(&model(view, &search, None), geometry(spec, view, a), 0.0)
        .max_scroll()
        .ceil() as usize
}
pub(super) fn controls(spec: &Spec, view: &View, a: Rect, offset: usize) -> Vec<(Rect, Hit)> {
    let search = TextField::new("");
    let layout = sidebar::layout(
        &model(view, &search, None),
        geometry(spec, view, a),
        offset as f32,
    );
    let body = painting::content_rect(spec, view, a);
    layout
        .entries
        .into_iter()
        .rev()
        .filter_map(|(r, h)| {
            let (index, toggle) = match h {
                SidebarHit::Row(i) => (i, view.tree_rows[i].is_dir),
                SidebarHit::RowChevron(i) => (i, true),
                _ => return None,
            };
            let path = view.tree_rows[index].path.to_string_lossy().into_owned();
            let r = r.intersect(&body);
            (!r.is_empty()).then(|| {
                (
                    r,
                    if toggle {
                        Hit::TreeToggle(path)
                    } else {
                        Hit::TreeOpen(path)
                    },
                )
            })
        })
        .collect()
}
pub(super) fn paint(
    list: &mut DrawList,
    spec: &Spec,
    view: &View,
    a: Rect,
    offset: usize,
    hover: Option<&Hit>,
) {
    let search = TextField::new("");
    let mut model = model(view, &search, hover);
    let icons: HashMap<_, _> = view
        .rows
        .iter()
        .filter(|r| !r.meta.icon.is_empty())
        .map(|r| {
            (
                view.workspace.join(&r.meta.path),
                match r.meta.icon.as_str() {
                    "library" => "BookOpen".into(),
                    "folder" => "Folder".into(),
                    "file" => "File".into(),
                    _ => r.meta.icon.clone(),
                },
            )
        })
        .collect();
    model.icons = Some(&icons);
    let area = geometry(spec, view, a);
    let layout = sidebar::layout(&model, area, offset as f32);
    let p = painting::palette(spec);
    let styler = |row: &crate::shell::Row| {
        painting::item_palette(&p, view, &style_key(view, &row.path.to_string_lossy()))
    };
    let style = sidebar::Appearance {
        row_palette: &styler,
        show_icons: view.presentation.show_icons,
        show_extensions: view.presentation.show_extensions,
    };
    let body = painting::content_rect(spec, view, a);
    list.push_clip(body);
    sidebar::paint_custom(list, area, &model, &layout, &p, Some(&style));
    list.pop_clip();
    let max = layout.max_scroll();
    if max > 0.0 {
        let h = (body.height() * body.height() / layout.content_height).max(18.0);
        list.rounded_rect(
            Rect::from_size(
                a.right - 5.0,
                body.top + (body.height() - h) * offset as f32 / max,
                3.0,
                h,
            ),
            1.5,
            p.border,
        );
    }
}
