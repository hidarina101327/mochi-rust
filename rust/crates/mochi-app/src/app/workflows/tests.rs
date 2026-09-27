use super::*;
use serde_json::json;
use ui::{Editor, Hit};

fn app() -> (App, PathBuf) {
    let root = std::env::temp_dir().join(core::new_id("mochi-workflow-app-test"));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.workflows.root = Some(root.clone());
    app.workflows.store = Some(Store::open(&root).unwrap());
    app.state.view = WorkspaceView::Automations;
    (app, root)
}

#[test]
fn workflow_completed_results_documents_history_and_settings_render_and_open() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    for (width, height, dark) in [(1440, 900, false), (1100, 800, true), (800, 740, false)] {
        let (mut app, root) = app();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.state.dark = dark;
        let snapshot = app.renderer.prepare_snapshot(width, height, 96.).unwrap();
        app.workflows_action(Hit::New).unwrap();
        {
            let g = app.workflows.view.draft.as_mut().unwrap();
            g.name = "每日互联网与 AI 前沿快讯".into();
            let mut condition = core::Node::new("check", "condition", 420., 100.);
            condition.label = "是否有新增内容".into();
            condition.inputs = json!({"left":3,"right":0});
            condition.config = json!({"operator":"greater"});
            let mut writer = core::Node::new("write", "file_write", 780., 100.);
            writer.label = "保存每日简报".into();
            writer.inputs = json!({"path":"今日简报.md", "content":"# 今日简报\n\n工作流已完成资料整理，并生成了可打开的文档。\n\n- 汇总新增条目：3 条\n- 报告状态：已保存\n- 下一步：点击文件卡片查看完整内容"});
            g.nodes[1].position.x = 1140.;
            g.nodes[1].inputs = json!({"report":"$nodes.write.output.path", "summary":"已整理 3 条新内容，报告已保存。", "item_count":3});
            g.nodes.insert(1, condition);
            g.nodes.insert(2, writer);
            g.edges = [
                ("start", "check", None),
                ("check", "write", Some("true")),
                ("write", "end", None),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (a, b, branch))| core::Edge {
                id: format!("e{i}"),
                source: a.into(),
                target: b.into(),
                source_handle: branch.map(str::to_owned),
            })
            .collect();
        }
        app.workflows.view.dirty = true;
        app.workflows.view.palette = true;
        app.workflows_action(Hit::Run).unwrap();
        assert!(app.dialog.is_none());
        assert!(!app.workflows.view.palette);
        let store = app.workflows.store.clone().unwrap();
        let host = core::native_host::NativeHost::new(&root, app.settings.clone()).unwrap();
        let run = core::execute(
            &store,
            store.claim().unwrap().unwrap(),
            &host,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(run.status, "succeeded", "{:?}", run.error);
        assert!(root.join("今日简报.md").is_file());
        app.workflows_timer();
        assert_eq!(app.workflows.view.run.as_ref().unwrap().status, "succeeded");
        assert!(app.workflows.view.selected_node.is_none());
        assert!(app.workflows.view.result_text.contains("已整理 3 条新内容"));
        app.workflows.view.status.clear();
        app.status_bar.toast = Default::default();
        app.paint(HWND::default()).unwrap();
        let save = |app: &mut App, name: &str| {
            if let Some(dir) = std::env::var_os("MOCHI_WORKFLOW_TEST_ARTIFACTS") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                app.renderer
                    .save_snapshot(&snapshot, &dir.join(format!("workflow-{width}-{name}.png")))
                    .unwrap();
            }
        };
        assert!(!app.workflows.view.inspector.is_empty());
        assert!(!app
            .workflows
            .view
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::CancelRun));
        let open = app
            .workflows
            .view
            .hits
            .iter()
            .find_map(|(_, h)| {
                if let Hit::OpenArtifact(p) = h {
                    Some(Hit::OpenArtifact(p.clone()))
                } else {
                    None
                }
            })
            .unwrap();
        save(&mut app, "completed-result");
        app.workflows_action(Hit::ResultTab(2)).unwrap();
        assert!(app.workflows.view.field.text().contains("summary"));
        app.workflows.view.select_node(2);
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.workflows.view.result_tab, 0);
        assert!(app
            .workflows
            .view
            .hits
            .iter()
            .any(|(_, h)| matches!(h, Hit::OpenArtifact(_))));
        save(&mut app, "node-result");
        app.workflows_action(Hit::ClosePanel).unwrap();
        app.paint(HWND::default()).unwrap();
        assert!(app.workflows.view.inspector.is_empty());
        app.workflows_action(Hit::ResultOverview).unwrap();
        app.workflows_action(Hit::History).unwrap();
        app.workflows.view.history = (0..25)
            .map(|i| {
                let mut r = app.workflows.view.history[0].clone();
                r.started_at -= i * 60000;
                r.finished_at = r.finished_at.map(|t| t - i * 60000);
                r
            })
            .collect();
        app.workflows.view.scroll = 29.;
        app.paint(HWND::default()).unwrap();
        let panel = app.workflows.view.panel_body;
        app.workflows_wheel(panel.left + 50., panel.top + 30., -120);
        assert_eq!(app.workflows.view.scroll, 29.);
        assert!(app.workflows.view.panel_scroll > 0.);
        app.paint(HWND::default()).unwrap();
        save(&mut app, "history-list");
        app.workflows_action(Hit::RunHistory(0)).unwrap();
        assert!(!app.workflows.view.history_open);
        assert!(app.workflows.view.selected_node.is_none());
        app.workflows_action(open).unwrap();
        assert_eq!(app.state.view, WorkspaceView::Editor);
        assert!(app
            .workflows_action(Hit::OpenArtifact("missing.md".into()))
            .is_err());
        app.state.view = WorkspaceView::Automations;
        app.workflows_action(Hit::CloseRun).unwrap();
        assert!(app.workflows.view.palette);
        app.workflows_action(Hit::Settings).unwrap();
        app.paint(HWND::default()).unwrap();
        save(&mut app, "settings-list");
        app.workflows_action(Hit::ClosePanel).unwrap();
        app.workflows.view.select_node(2);
        app.workflows_action(Hit::ConfigTab(true)).unwrap();
        assert!(app.workflows.view.history_open);
    }
}

#[test]
fn workflow_library_folder_lifecycle_and_move_actions() {
    let (mut app, root) = app();
    app.workflows_action(Hit::NewFolder).unwrap();
    app.workflows.view.field.set_text("研究资料");
    app.workflows_action(Hit::Apply).unwrap();
    let folder = app.workflows.view.folders[0].id.clone();
    app.workflows_action(Hit::Folder(Some(folder.clone())))
        .unwrap();
    app.workflows_action(Hit::New).unwrap();
    let id = app
        .workflows
        .view
        .selected
        .as_ref()
        .unwrap()
        .definition
        .id
        .clone();
    assert_eq!(
        Store::open(&root).unwrap().summaries().unwrap()[0]
            .folder_id
            .as_deref(),
        Some(folder.as_str())
    );
    app.workflows_action(Hit::Back).unwrap();
    assert_eq!(app.workflows.view.folder.as_deref(), Some(folder.as_str()));
    app.workflows_action(Hit::MoveWorkflow(id.clone(), None))
        .unwrap();
    assert_eq!(app.workflows.view.workflows[0].folder_id, None);
    app.workflows_action(Hit::MoveWorkflow(id, Some(folder.clone())))
        .unwrap();
    app.workflows_action(Hit::DeleteFolder(folder)).unwrap();
    assert_eq!(app.workflows.view.workflows.len(), 1);
    assert_eq!(app.workflows.view.workflows[0].folder_id, None);
}

#[test]
fn workflow_context_delete_variable_wire_and_execute_real_value() {
    let (mut app, root) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.draft.as_mut().unwrap().defaults = json!({"value": 42});
    app.workflows
        .view
        .connect_variable(0, 1, "result", "output")
        .unwrap();
    app.workflows.view.hits.push((
        Rect::from_size(200., 100., 40., 30.),
        Hit::Binding(1, "result".into()),
    ));
    app.workflows_context_menu(210., 110.);
    let action = app.menu.as_ref().unwrap().items[0].action.clone();
    app.run_menu_action(action);
    assert!(app.workflows.view.bindings().is_empty());
    app.workflows_action(Hit::Undo).unwrap();
    app.workflows_action(Hit::Run).unwrap();
    assert!(app.dialog.is_none());
    let store = app.workflows.store.as_ref().unwrap();
    let host = core::native_host::NativeHost::new(&root, app.settings.clone()).unwrap();
    let run = core::execute(
        store,
        store.claim().unwrap().unwrap(),
        &host,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(run.output["result"]["value"], 42);
    app.workflows_action(Hit::CloseRun).unwrap();
    app.workflows.view.hits.clear();
    app.workflows
        .view
        .hits
        .push((Rect::from_size(200., 100., 40., 30.), Hit::Edge(0)));
    app.workflows_context_menu(210., 110.);
    let action = app.menu.as_ref().unwrap().items[0].action.clone();
    app.run_menu_action(action);
    assert!(app.workflows.view.draft.as_ref().unwrap().edges.is_empty());
    app.workflows_action(Hit::Undo).unwrap();
    assert_eq!(app.workflows.view.draft.as_ref().unwrap().edges.len(), 1);
}

#[test]
fn workflow_library_and_variable_selector_render_native_screenshots() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    for (width, height, dark) in [(1440, 900, false), (1100, 800, true), (800, 740, false)] {
        let (mut app, root) = app();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.state.dark = dark;
        let snapshot = app.renderer.prepare_snapshot(width, height, 96.).unwrap();
        let store = app.workflows.store.as_ref().unwrap();
        store.save_folder(None, "每日简报").unwrap();
        store.save_folder(None, "资料处理").unwrap();
        for name in [
            "每日互联网与 AI 前沿快讯",
            "每日漏洞情报",
            "整理网页资料",
            "每周工作总结",
            "推送自动化通知",
        ] {
            let mut flow = core::Workflow::blank();
            flow.name = name.into();
            flow.description = "收集公开资料，整理信息并保存到知识库。".into();
            store.save(&flow, None).unwrap();
        }
        app.workflows_refresh();
        app.paint(HWND::default()).unwrap();
        let save = |app: &mut App, name: &str| {
            if let Some(dir) = std::env::var_os("MOCHI_WORKFLOW_TEST_ARTIFACTS") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                app.renderer
                    .save_snapshot(&snapshot, &dir.join(format!("workflow-{width}-{name}.png")))
                    .unwrap();
            }
        };
        save(&mut app, "library");
        assert!(!app
            .workflows
            .view
            .hits
            .iter()
            .any(|(_, h)| matches!(h, Hit::Sample(_))));
        app.workflows_action(Hit::Open(0)).unwrap();
        app.workflows
            .view
            .connect_variable(0, 1, "result", "output")
            .unwrap();
        app.workflows.view.palette = false;
        app.workflows.view.needs_fit = true;
        app.paint(HWND::default()).unwrap();
        save(&mut app, "variables");
        app.workflows_action(Hit::VariablePicker("/inputs/result".into()))
            .unwrap();
        app.paint(HWND::default()).unwrap();
        save(&mut app, "dropdown");
    }
}
#[test]
fn native_workflow_ui_creates_edits_saves_and_runs_trusted_flow() {
    let (mut app, root) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.select_node(1);
    app.workflows
        .view
        .field
        .set_text(r#"{"inputs":{"message":"中文流程"}}"#);
    app.workflows_action(Hit::Save).unwrap();
    let saved = app.workflows.view.selected.clone().unwrap();
    assert!(saved.approved);
    assert_eq!(saved.definition.nodes[1].inputs["message"], "中文流程");
    app.workflows.pending_input = Some(json!({}));
    app.workflows_action(Hit::Run).unwrap();
    assert_eq!(app.workflows.view.run.as_ref().unwrap().status, "queued");
    let store = app.workflows.store.as_ref().unwrap();
    let run = store.claim().unwrap().unwrap();
    let host = core::native_host::NativeHost::new(&root, app.settings.clone()).unwrap();
    let run = core::execute(
        store,
        run,
        &host,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(run.output["message"], "中文流程");
}
#[test]
fn native_workflow_editor_uses_field_ime_and_arrow_routing() {
    let (mut app, _) = app();
    app.workflows
        .view
        .set_editor(Editor::Import, "第一行\n第二行".into());
    app.workflows.view.field_rect = Rect::from_size(0., 0., 320., 200.);
    app.focus = Focus::Workflow;
    app.workflows.view.field.buffer.set_cursor(0, false);
    app.on_ime_composition("中文", 2);
    assert!(app.workflows.view.field.buffer.composition().is_some());
    app.on_ime_commit("中文");
    assert!(app.workflows.view.field.text().starts_with("中文"));
    let cursor = app.workflows.view.field.buffer.cursor();
    app.workflows_key(0x28, false, false);
    assert!(app.workflows.view.field.buffer.cursor() > cursor);
    assert!(!app.main_accepts_input());
}
#[test]
fn unsaved_invalid_json_draft_survives_reopening_without_becoming_executable() {
    let (mut app, root) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.select_node(1);
    app.workflows.view.field.set_text("{ broken");
    app.workflows_persist_draft();
    let saved = app.workflows.view.selected.clone().unwrap();
    assert!(
        app.workflows
            .store
            .as_ref()
            .unwrap()
            .get(&saved.definition.id)
            .unwrap()
            .approved
    );
    assert!(app.workflows_action(Hit::Run).is_err());
    assert!(app.workflows.view.run.is_none());
    let mut other = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    other.workflows.store = Some(Store::open(&root).unwrap());
    other.workflows_restore_draft();
    assert_eq!(other.workflows.view.field.text(), "{ broken");
    assert!(other.workflows.view.dirty);
}
#[test]
fn running_is_an_explicit_action_without_authorization_dialog() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows_action(Hit::Run).unwrap();
    assert!(app.dialog.is_none());
    assert_eq!(app.workflows.view.run.as_ref().unwrap().status, "queued");
    assert!(app.workflows.view.selected.as_ref().unwrap().approved);
}

#[test]
fn workflow_run_uses_pending_definition_and_explicit_run_input() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows_action(Hit::Definition).unwrap();
    let mut flow = app.workflows.view.draft.clone().unwrap();
    flow.defaults = json!({"a": 17});
    app.workflows
        .view
        .field
        .set_text(&serde_json::to_string(&flow).unwrap());
    app.workflows_action(Hit::RunOptions).unwrap();
    app.workflows.view.field.set_text("{\"a\": 23}");
    app.workflows_action(Hit::Run).unwrap();
    assert!(app.dialog.is_none());
    let run = app.workflows.view.run.as_ref().unwrap();
    assert_eq!(run.definition.defaults["a"], 17);
    assert_eq!(run.input["a"], 23);
}

#[test]
fn invalid_definition_can_be_discarded_without_trapping_focus() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows_action(Hit::Definition).unwrap();
    app.workflows.view.field.set_text("broken JSON");
    app.workflows
        .view
        .hits
        .push((Rect::from_size(10., 10., 80., 30.), Hit::Back));
    app.workflows_click(20., 20.);
    assert!(app.dialog.is_some());
    app.run_dialog_action(DialogAction::WorkflowDiscard);
    assert!(app.workflows.view.draft.is_none());
}

#[test]
fn closing_invalid_node_parameters_only_discards_that_editor() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.select_node(1);
    app.workflows.view.field.set_text("bad JSON");
    app.workflows_action(Hit::CloseEditor).unwrap();
    assert!(app.dialog.is_some());
    app.run_dialog_action(DialogAction::WorkflowDiscardEditor);
    assert!(app.workflows.view.editor.is_none());
    assert!(app.workflows.view.draft.is_some());
    assert!(!app.workflows.view.dirty);
}

#[test]
fn ctrl_s_applies_definition_before_persisting() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows_action(Hit::Definition).unwrap();
    let mut definition = app.workflows.view.draft.clone().unwrap();
    definition.name = "快捷键保存".into();
    app.workflows
        .view
        .field
        .set_text(&serde_json::to_string(&definition).unwrap());
    assert!(app.workflows_key(0x53, false, true));
    assert_eq!(
        app.workflows
            .view
            .selected
            .as_ref()
            .unwrap()
            .definition
            .name,
        "快捷键保存"
    );
    assert!(!app.workflows.view.dirty);
}

#[test]
fn workflow_modern_canvas_renders_native_forms_palette_and_minimap() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    for (width, height, dark) in [(1440, 900, false), (1100, 760, true), (800, 700, false)] {
        let (mut app, root) = app();
        app.shell.open_workspace(&root, || {}).unwrap();
        let snapshot = app.renderer.prepare_snapshot(width, height, 96.).unwrap();
        app.state.dark = dark;
        app.workflows_action(Hit::New).unwrap();
        app.workflows.view.add_node("ai");
        app.workflows.view.add_node("condition");
        app.workflows.view.add_node("end");
        {
            let g = app.workflows.view.draft.as_mut().unwrap();
            g.edges = vec![
                core::Edge {
                    id: "a".into(),
                    source: g.nodes[0].id.clone(),
                    target: g.nodes[2].id.clone(),
                    source_handle: None,
                },
                core::Edge {
                    id: "b".into(),
                    source: g.nodes[2].id.clone(),
                    target: g.nodes[3].id.clone(),
                    source_handle: None,
                },
                core::Edge {
                    id: "c".into(),
                    source: g.nodes[3].id.clone(),
                    target: g.nodes[1].id.clone(),
                    source_handle: Some("true".into()),
                },
                core::Edge {
                    id: "d".into(),
                    source: g.nodes[3].id.clone(),
                    target: g.nodes[4].id.clone(),
                    source_handle: Some("false".into()),
                },
            ];
        }
        app.workflows.view.editor = None;
        app.workflows.view.selected_node = None;
        app.paint(HWND::default()).unwrap();
        app.workflows.view.auto_layout();
        app.workflows.view.minimap = true;
        app.paint(HWND::default()).unwrap();
        let save = |app: &mut App, name: &str| {
            if let Some(dir) = std::env::var_os("MOCHI_WORKFLOW_TEST_ARTIFACTS") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                app.renderer
                    .save_snapshot(&snapshot, &dir.join(format!("workflow-{width}-{name}.png")))
                    .unwrap();
            }
        };
        save(&mut app, "overview");
        app.workflows.view.select_node(2);
        app.paint(HWND::default()).unwrap();
        assert!(app
            .workflows
            .view
            .hits
            .iter()
            .any(|(_, h)| matches!(h,Hit::Parameter(path) if path=="/label")));
        app.workflows_action(Hit::Parameter("/label".into()))
            .unwrap();
        app.workflows.view.field.set_text("资料整理 AI");
        app.workflows_action(Hit::Save).unwrap();
        assert_eq!(
            app.workflows
                .view
                .selected
                .as_ref()
                .unwrap()
                .definition
                .nodes[2]
                .label,
            "资料整理 AI"
        );
        app.paint(HWND::default()).unwrap();
        save(&mut app, "form");
        app.workflows_action(Hit::CloseEditor).unwrap();
        app.workflows_action(Hit::Add).unwrap();
        app.workflows.view.palette = true;
        app.workflows.view.search.set_text("脚本");
        app.paint(HWND::default()).unwrap();
        let kinds: Vec<_> = app
            .workflows
            .view
            .hits
            .iter()
            .filter_map(|(_, h)| if let Hit::Kind(i) = h { Some(*i) } else { None })
            .collect();
        assert!(!kinds.is_empty());
        assert!(kinds
            .iter()
            .all(|i| matches!(core::catalog::KINDS[*i], "script" | "tool")));
        save(&mut app, "palette");
    }
}

#[test]
fn workflow_parameter_draft_restores_its_field_identity() {
    let (mut app, root) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.select_node(1);
    app.workflows_action(Hit::Parameter("/label".into()))
        .unwrap();
    app.workflows.view.field.set_text("尚未保存的名称");
    app.workflows_persist_draft();
    let mut other = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    other.workflows.store = Some(Store::open(&root).unwrap());
    other.workflows_restore_draft();
    assert_eq!(other.workflows.view.parameter, "/label");
    other.workflows_action(Hit::Save).unwrap();
    assert_eq!(
        other.workflows.view.selected.unwrap().definition.nodes[1].label,
        "尚未保存的名称"
    );
}

#[test]
fn workflow_provider_and_variable_menus_keep_node_identity() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.add_node("ai");
    let id = app.workflows.view.draft.as_ref().unwrap().nodes[2]
        .id
        .clone();
    app.workflows_action(Hit::Parameter("/config/provider_id".into()))
        .unwrap();
    assert!(app.menu.as_ref().unwrap().search.is_some());
    app.run_menu_action(MenuAction::WorkflowProvider(
        id.clone(),
        Some("provider-test".into()),
    ));
    assert_eq!(
        app.workflows.view.draft.as_ref().unwrap().nodes[2].config["provider_id"],
        "provider-test"
    );
    app.workflows_action(Hit::Parameter("/inputs/prompt".into()))
        .unwrap();
    app.workflows.view.field.set_text("日期：");
    app.workflows_action(Hit::Variables).unwrap();
    let action = app.menu.as_ref().unwrap().items.iter().find(|item| matches!(&item.action, MenuAction::WorkflowAction(Hit::SetVariable(_, _, value)) if value == "$run.date")).unwrap().action.clone();
    app.run_menu_action(action);
    assert_eq!(app.workflows.view.field.text(), "$run.date");
    app.run_menu_action(MenuAction::WorkflowReference(
        "stale-node".into(),
        "/inputs/prompt".into(),
        "$input".into(),
    ));
    assert_eq!(app.workflows.view.field.text(), "$run.date");
}

#[test]
fn workflow_eleven_nodes_at_54_percent_has_library_and_typed_trigger() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let (mut app, root) = app();
    app.shell.open_workspace(&root, || {}).unwrap();
    let snapshot = app.renderer.prepare_snapshot(2000, 1100, 96.).unwrap();
    app.workflows_action(Hit::New).unwrap();
    let mut saved = app.workflows.view.selected.clone().unwrap();
    let g = &mut saved.definition;
    g.name = "每日互联网与 AI 前沿快讯推送".into();
    g.nodes.clear();
    g.edges.clear();
    let specs = [
        ("start", "每日 09:00 · 北京时间"),
        ("script", "并行采集 · 去重 · 来源健康"),
        ("condition", "是否有窗口内条目"),
        ("ai", "AI 精选与中文整理"),
        ("script", "校验引用 · AI 失败保留原文"),
        ("file_write", "保存每日简报"),
        ("notify", "推送墨池通知"),
        ("end", "完成推送"),
        ("script", "无新增 / 来源不可用说明"),
        ("file_write", "保存采集状态报告"),
        ("notify", "推送采集状态"),
    ];
    for (i, (kind, label)) in specs.iter().enumerate() {
        let mut n = core::Node::new(&format!("n{i}"), kind, i as f32 * 240., 100.);
        n.label = (*label).into();
        g.nodes.push(n);
    }
    for (i, (a, b, branch)) in [
        (0, 1, None),
        (1, 2, None),
        (2, 3, Some("true")),
        (3, 4, None),
        (4, 5, None),
        (5, 6, None),
        (6, 7, None),
        (2, 8, Some("false")),
        (8, 9, None),
        (9, 10, None),
        (10, 7, None),
    ]
    .into_iter()
    .enumerate()
    {
        g.edges.push(core::Edge {
            id: format!("e{i}"),
            source: format!("n{a}"),
            target: format!("n{b}"),
            source_handle: branch.map(str::to_owned),
        });
    }
    app.workflows.view.open(saved);
    app.workflows.view.select_node(0);
    app.workflows_action(Hit::Choice("/workflow/trigger/type".into(), "daily".into()))
        .unwrap();
    app.paint(HWND::default()).unwrap();
    app.workflows.view.zoom = 0.54;
    app.workflows.view.pan = (32., 220.);
    app.workflows.view.needs_fit = false;
    app.workflows.view.minimap = true;
    app.paint(HWND::default()).unwrap();
    assert_eq!(app.workflows.view.draft.as_ref().unwrap().nodes.len(), 11);
    assert!(app.workflows.view.palette_rect.width() > 200.);
    assert!(app
        .workflows
        .view
        .hits
        .iter()
        .any(|(_, h)| matches!(h,Hit::Parameter(p) if p=="/workflow/trigger/type")));
    if let Some(dir) = std::env::var_os("MOCHI_WORKFLOW_TEST_ARTIFACTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.renderer
            .save_snapshot(&snapshot, &dir.join("workflow-eleven-54.png"))
            .unwrap();
    }
}

#[test]
fn workflow_boolean_controls_and_select_all_preserve_types() {
    let (mut app, _) = app();
    app.workflows_action(Hit::New).unwrap();
    app.workflows.view.add_node("condition");
    let i = app.workflows.view.selected_node.unwrap();
    app.workflows.view.draft.as_mut().unwrap().nodes[i].inputs["right"] = serde_json::json!(true);
    app.workflows_action(Hit::Parameter("/inputs/right".into()))
        .unwrap();
    app.workflows.view.field.set_text("false");
    app.workflows_action(Hit::Apply).unwrap();
    assert_eq!(
        app.workflows.view.draft.as_ref().unwrap().nodes[i].inputs["right"],
        false
    );
    app.workflows_action(Hit::Choice("/on_error".into(), "continue".into()))
        .unwrap();
    assert_eq!(
        app.workflows.view.draft.as_ref().unwrap().nodes[i].on_error,
        "continue"
    );
    assert!(app.workflows_key(0x41, false, true));
    assert_eq!(app.workflows.view.group.len(), 3);
    assert!(app.workflows_key(0x2e, false, false));
    assert!(app.workflows.view.draft.as_ref().unwrap().nodes.is_empty());
    app.workflows_action(Hit::Undo).unwrap();
    assert_eq!(app.workflows.view.draft.as_ref().unwrap().nodes.len(), 3);
}
