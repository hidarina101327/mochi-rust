use super::*;
use std::time::{Duration, Instant};

struct Fixture {
    app: App,
    root: PathBuf,
    library: PathBuf,
    target: PathBuf,
    source: PathBuf,
}

#[test]
fn package_drop_requires_confirmation_and_cancel_writes_nothing() {
    let mut f = Fixture::new();
    let root = f.app.shell.workspace().unwrap().root.clone();
    let store = mochi_core::workflows::Store::open(&root).unwrap();
    let before = store.list().unwrap().len();
    let package = mochi_core::transfer::Package {
        kind: mochi_core::transfer::Kind::Workflow,
        name: "测试工作流".into(),
        content: serde_json::to_string(&mochi_core::workflows::Workflow::blank()).unwrap(),
    };
    let path = f.root.join("任意名称.zip");
    package.write(&path).unwrap();
    f.drop_files(vec![path.clone()]);
    assert!(f.app.global_import.is_none());
    assert_eq!(f.app.dialog.as_ref().unwrap().title, "导入工作流");
    assert_eq!(store.list().unwrap().len(), before);
    f.app.on_edit_key(0x1b, false, false);
    assert!(f.app.dialog.is_none());
    assert_eq!(store.list().unwrap().len(), before);
    f.drop_files(vec![path.clone(), path.clone(), f.source.clone()]);
    // 确认的是校验过的快照，即便源文件之后被改动也不受影响。
    std::fs::write(&path, b"changed after preview").unwrap();
    f.app.on_edit_key(0x0d, false, false);
    assert_eq!(store.list().unwrap().len(), before + 1);
    assert_eq!(
        f.app.global_import.as_ref().unwrap().paths,
        vec![f.source.clone()]
    );
    assert!(!f.library.join("任意名称.zip").exists());
    f.clean();
}

#[test]
fn package_drop_routes_agent_before_sidebar_copy_and_checks_workspace() {
    let mut f = Fixture::new();
    let package = mochi_core::transfer::Package {
        kind: mochi_core::transfer::Kind::Agent,
        name: "测试 Agent".into(),
        content: "---\nname: 测试 Agent\n---\n帮助用户。".into(),
    };
    let path = f.root.join("agent.zip");
    package.write(&path).unwrap();
    f.app.paint(HWND::default()).unwrap();
    let area = f.app.side.layout.content;
    f.app
        .on_dropped_files(vec![path], area.left + 20.0, area.top + 20.0);
    let action = f
        .app
        .dialog
        .as_ref()
        .unwrap()
        .buttons
        .last()
        .unwrap()
        .action
        .clone();
    assert_eq!(f.app.dialog.as_ref().unwrap().title, "导入Agent");
    if let DialogAction::ImportPackage {
        package, remaining, ..
    } = action
    {
        f.app
            .confirm_package_import(f.root.join("another-workspace"), *package, remaining);
    } else {
        panic!("wrong import route");
    }
    assert!(f
        .app
        .dialog
        .as_ref()
        .unwrap()
        .error
        .contains("工作区已切换"));
    f.app.on_edit_key(0x0d, false, false);
    assert!(f.app.dialog.is_none());
    let service = mochi_core::ai::agent_config::AgentConfigService::new(
        &f.app.shell.workspace().unwrap().root,
    );
    assert!(service.load_agents().iter().any(|a| a.name == "测试 Agent"));
    f.clean();
}

#[test]
fn package_drop_imports_desktop_cards_after_confirmation() {
    use mochi_core::desktop_cards::{Card, DesktopConfig, Module};
    let mut f = Fixture::new();
    let root = f.app.shell.workspace().unwrap().root.clone();
    let mut config = DesktopConfig::new();
    let mut card = Card::new("导入卡片", Module::Home);
    card.enabled = true;
    config.cards.push(card);
    let package = mochi_core::transfer::Package {
        kind: mochi_core::transfer::Kind::DesktopCards,
        name: "测试布局".into(),
        content: config.export_json().unwrap(),
    };
    let path = f.root.join("cards.zip");
    package.write(&path).unwrap();
    let before = DesktopConfig::load(&root).unwrap().cards.len();
    f.drop_files(vec![path]);
    assert_eq!(f.app.dialog.as_ref().unwrap().title, "导入桌面卡片");
    assert_eq!(DesktopConfig::load(&root).unwrap().cards.len(), before);
    let start = Instant::now();
    // 工作区启动时桌面数据是异步加载的，仅在仍忙时重试。
    while f.app.dialog.is_some() {
        f.app.desktop_take_results();
        f.app.on_edit_key(0x0d, false, false);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            f.app.dialog.as_ref().map(|d| &d.error)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let saved = DesktopConfig::load(&root).unwrap();
    assert_eq!(saved.cards.len(), before + 1);
    assert!(!saved.cards.last().unwrap().enabled);
    f.clean();
}

impl Fixture {
    fn new() -> Self {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-global-import-{}",
            mochi_core::paths::random_base36(12)
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.open_workspace(HWND::default(), workspace, false)
            .unwrap();
        let library = PathBuf::from(&app.shell.workspace().unwrap().libraries[0].path);
        let target = library.join("项目资料");
        std::fs::create_dir_all(target.join("子目录")).unwrap();
        let source = root.join("报告.md");
        std::fs::write(&source, "# 报告\n\n![image](image.png)").unwrap();
        std::fs::write(root.join("image.png"), b"fixture image").unwrap();
        app.shell.refresh_tree();
        app.state.view = WorkspaceView::Home;
        app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        Self {
            app,
            root,
            library,
            target,
            source,
        }
    }

    fn drop_files(&mut self, paths: Vec<PathBuf>) {
        self.app.paint(HWND::default()).unwrap();
        self.app.on_dropped_files(paths, 600.0, 650.0);
    }

    fn select(&mut self, target: &Path) {
        let index = self
            .app
            .global_import
            .as_ref()
            .unwrap()
            .view
            .folders
            .iter()
            .position(|f| f.path == target)
            .unwrap();
        self.app.import_picker_action(Hit::Select(index));
    }

    fn finish(&mut self) {
        let start = Instant::now();
        loop {
            self.app.take_file_jobs();
            if self.app.state.status_text.starts_with("已导入") {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "{}",
                self.app.state.status_text
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn clean(self) {
        let root = self.root.clone();
        drop(self);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn global_import_waits_for_destination_and_copies_resources_without_overwriting() {
    let mut f = Fixture::new();
    std::fs::write(f.target.join("报告.md"), "keep existing").unwrap();
    f.drop_files(vec![f.source.clone(), f.source.clone()]);
    let pending = f.app.global_import.as_ref().unwrap();
    assert_eq!(pending.paths.len(), 1);
    assert_eq!(pending.view.selected.as_ref(), Some(&f.library));
    assert!(!f.library.join("报告.md").exists());
    let target = f.target.clone();
    f.select(&target);
    f.app.import_picker_action(Hit::Confirm);
    assert!(f.app.global_import.is_none());
    f.finish();
    assert_eq!(
        std::fs::read_to_string(target.join("报告.md")).unwrap(),
        "keep existing"
    );
    assert!(std::fs::read_to_string(target.join("报告 (1).md"))
        .unwrap()
        .contains("./assets/"));
    assert!(target.join("assets").is_dir());
    assert!(f.source.is_file());
    f.clean();
}

#[test]
fn global_import_routes_home_editor_hidden_sidebar_and_ai_panel_blank_space() {
    let mut f = Fixture::new();
    for view in [
        WorkspaceView::Home,
        WorkspaceView::Recent,
        WorkspaceView::Schedule,
        WorkspaceView::Editor,
    ] {
        f.app.state.view = view;
        f.app.state.sidebar_visible = false;
        f.app.state.ai_panel_open = true;
        f.drop_files(vec![f.source.clone()]);
        assert!(f.app.global_import.is_some(), "{view:?}");
        assert!(f
            .app
            .on_accelerator(HWND::default(), 0x1b, false, false, false));
        assert!(f.app.global_import.is_none());
    }
    assert!(!f.library.join("报告.md").exists());
    f.clean();
}

#[test]
fn global_import_modal_keeps_drop_batch_and_blocks_background_input() {
    let mut f = Fixture::new();
    f.app.focus = Focus::SidebarSearch;
    f.app.side.search.set_text("unchanged");
    f.drop_files(vec![f.source.clone()]);
    f.app
        .on_dropped_files(vec![f.root.join("image.png")], 900.0, 700.0);
    assert_eq!(
        f.app.global_import.as_ref().unwrap().paths,
        vec![f.source.clone()]
    );
    f.app.on_char('x');
    f.app.on_ime_commit("输入");
    f.app.on_right_click(900.0, 700.0);
    f.app.on_double_click(900.0, 700.0);
    assert_eq!(f.app.side.search.text(), "unchanged");
    assert!(f.app.menu.is_none());
    let scroll = f.app.shell.active_scroll();
    f.app.on_wheel(900.0, 700.0, -120);
    f.app.on_horizontal_wheel(900.0, 700.0, -120);
    assert_eq!(f.app.shell.active_scroll(), scroll);
    f.app.import_picker_action(Hit::Cancel);
    assert_eq!(f.app.focus, Focus::SidebarSearch);
    assert!(!f.library.join("报告.md").exists());
    f.clean();
}

#[test]
fn global_import_directory_in_ai_composer_uses_destination_picker() {
    let mut f = Fixture::new();
    let source = f.root.join("外部文件夹");
    std::fs::create_dir_all(&source).unwrap();
    f.app.state.view = WorkspaceView::MochiAi;
    f.app.paint(HWND::default()).unwrap();
    let input = f.app.ai.layout.rect_of(assistant::Hit::Input).unwrap();
    f.app
        .on_dropped_files(vec![source], input.left + 10.0, input.top + 10.0);
    assert!(f.app.global_import.is_some());
    assert!(f.app.ai.panel.pending_files.is_empty());
    f.app.import_picker_action(Hit::Cancel);
    f.clean();
}

#[test]
fn global_import_lists_all_libraries_and_loads_nested_folders_on_demand() {
    let mut f = Fixture::new();
    let other = f
        .app
        .shell
        .create_library("knowledge-base", "第二个知识库")
        .unwrap();
    let other = PathBuf::from(&f.app.shell.workspace().unwrap().libraries[other].path);
    std::fs::create_dir_all(other.join("资料/附件")).unwrap();
    std::fs::create_dir_all(other.join(".mochi/internal")).unwrap();
    f.drop_files(vec![f.source.clone()]);
    let view = &f.app.global_import.as_ref().unwrap().view;
    assert!(view.folders.iter().any(|r| r.path == f.library));
    assert!(view.folders.iter().any(|r| r.path == other));
    assert!(!view.folders.iter().any(|r| r.path.ends_with("附件")));
    let index = view
        .folders
        .iter()
        .position(|r| r.path == other.join("资料"))
        .unwrap();
    f.app.import_picker_action(Hit::Toggle(index));
    let target = other.join("资料/附件");
    f.select(&target);
    assert!(!f
        .app
        .global_import
        .as_ref()
        .unwrap()
        .view
        .folders
        .iter()
        .any(|r| r.name == ".mochi"));
    f.app.import_picker_action(Hit::Confirm);
    f.finish();
    assert!(target.join("报告.md").is_file());
    f.clean();
}

#[test]
fn global_import_revalidates_deleted_destination_before_starting() {
    let mut f = Fixture::new();
    f.drop_files(vec![f.source.clone()]);
    let target = f.target.clone();
    f.select(&target);
    std::fs::remove_dir(target.join("子目录")).unwrap();
    std::fs::remove_dir(&target).unwrap();
    f.app.import_picker_action(Hit::Confirm);
    assert!(f
        .app
        .global_import
        .as_ref()
        .unwrap()
        .view
        .error
        .contains("已不存在"));
    assert!(!target.exists());
    f.app.import_picker_action(Hit::Cancel);
    f.clean();
}

#[test]
fn global_import_does_not_carry_pending_files_into_another_workspace() {
    let mut f = Fixture::new();
    f.drop_files(vec![f.source.clone()]);
    let other = f.root.join("other-workspace");
    std::fs::create_dir_all(&other).unwrap();
    f.app.open_workspace(HWND::default(), other, false).unwrap();
    assert!(f.app.global_import.is_none());
    assert!(!f.library.join("报告.md").exists());
    assert!(f.source.is_file());
    f.clean();
}

#[test]
fn global_import_folder_selection_reuses_existing_copy_or_mapping_dialog() {
    let mut f = Fixture::new();
    let source = f.root.join("外部资料");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("file.txt"), "source").unwrap();
    f.drop_files(vec![source.clone()]);
    let target = f.target.clone();
    f.select(&target);
    f.app.import_picker_action(Hit::Confirm);
    assert!(f.app.global_import.is_none());
    let action = f
        .app
        .dialog
        .as_ref()
        .unwrap()
        .buttons
        .iter()
        .find(|b| matches!(b.action, DialogAction::ImportDropped { mapping: false, .. }))
        .unwrap()
        .action
        .clone();
    assert!(!target.join("外部资料").exists());
    f.app.run_dialog_action(action);
    f.finish();
    assert_eq!(
        std::fs::read_to_string(target.join("外部资料/file.txt")).unwrap(),
        "source"
    );
    f.clean();
}

#[test]
fn global_import_keeps_large_batches_and_mixed_editor_drops_intact() {
    let mut f = Fixture::new();
    let note = f.library.join("正在编辑.md");
    std::fs::write(&note, "# Keep this document").unwrap();
    f.app.shell.open_file(&note);
    f.app.state.view = WorkspaceView::Editor;
    f.app.sync_state();
    f.app.paint(HWND::default()).unwrap();
    let area = f.app.editor_area;
    f.app.on_dropped_files(
        vec![f.root.join("image.png"), f.source.clone()],
        area.left + 20.0,
        area.top + 40.0,
    );
    assert_eq!(f.app.global_import.as_ref().unwrap().paths.len(), 2);
    assert_eq!(
        std::fs::read_to_string(note).unwrap(),
        "# Keep this document"
    );
    f.app.import_picker_action(Hit::Cancel);
    let sources = (0..30)
        .map(|i| {
            let path = f.root.join(format!("批量-{i}.txt"));
            std::fs::write(&path, "batch").unwrap();
            path
        })
        .collect::<Vec<_>>();
    f.drop_files(sources.clone());
    assert_eq!(f.app.global_import.as_ref().unwrap().paths.len(), 30);
    let target = f.target.clone();
    f.select(&target);
    f.app.import_picker_action(Hit::Confirm);
    f.finish();
    assert!(sources
        .iter()
        .all(|p| target.join(p.file_name().unwrap()).is_file()));
    f.clean();
}

#[test]
fn global_import_visual_snapshots() {
    let mut f = Fixture::new();
    for folder in [
        "课程笔记",
        "项目资料/需求文档",
        "项目资料/会议记录",
        "收集与参考",
        "归档",
    ] {
        std::fs::create_dir_all(f.library.join(folder)).unwrap();
    }
    f.app
        .shell
        .create_library("knowledge-base", "工作资料")
        .unwrap();
    f.app.shell.select_library(0);
    f.drop_files(vec![f.source.clone(), f.root.join("image.png")]);
    let index = f
        .app
        .global_import
        .as_ref()
        .unwrap()
        .view
        .folders
        .iter()
        .position(|r| r.path == f.target)
        .unwrap();
    f.app.import_picker_action(Hit::Toggle(index));
    f.select(&f.library.join("项目资料/需求文档"));
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/global-import");
    std::fs::create_dir_all(&output).unwrap();
    for (name, width, height, dpi, dark) in [
        ("light", 1200, 800, 96.0, false),
        ("dark", 1800, 1200, 144.0, true),
        ("narrow", 680, 520, 96.0, false),
    ] {
        f.app.state.dark = dark;
        let snapshot = f.app.renderer.prepare_snapshot(width, height, dpi).unwrap();
        f.app.paint(HWND::default()).unwrap();
        f.app
            .renderer
            .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
            .unwrap();
    }
    f.app.import_picker_action(Hit::Cancel);
    f.clean();
}
