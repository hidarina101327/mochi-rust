//! 处理从应用外拖入文件后的导入请求和导入流程。
use super::*;

impl App {
    pub(super) fn on_tree_files_drop(&mut self, paths: &[PathBuf], x: f32, y: f32) -> bool {
        if !self.side.layout.content.contains(x, y) {
            return false;
        }
        if paths.is_empty() {
            return true;
        }
        // 用渲染时的路径，而不是可能已被文件监视器改动的行号。
        let row = self
            .side
            .layout
            .drop_row(x, y)
            .and_then(|(index, _)| self.side.layout.row_path(index));
        let target = row
            .map(|path| {
                if path.is_dir() {
                    path.to_path_buf()
                } else {
                    path.parent().unwrap_or(path).to_path_buf()
                }
            })
            .or_else(|| {
                (!self.shell.favorites_selected())
                    .then(|| self.shell.tree_root())
                    .flatten()
            });
        let Some(target) = target else {
            self.show_global_notice("请拖到一个具体的文件夹中");
            return true;
        };
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return true;
        };
        if !target.is_dir()
            || mochi_core::mapped_folders::Service::new(root)
                .for_path(&target)
                .ok()
                .flatten()
                .is_some()
        {
            self.show_global_notice("请拖到工作区中的实际文件夹，映射目录不接收导入");
            return true;
        }
        // 类型视图/收藏视图的空白区域没有明确的根文件夹可落。
        if self.shell.library_root_for_path(&target).is_none() {
            self.show_global_notice("请拖到一个知识库或其中的文件夹");
            return true;
        }
        self.request_external_import(paths, target);
        true
    }

    pub(super) fn request_external_import(&mut self, paths: &[PathBuf], target: PathBuf) {
        self.menu = None;
        self.cancel_sidebar_tree_drag();
        if paths.iter().any(|path| path.is_dir()) {
            let preference = app_settings::descriptor("sidebar.folderDropAction")
                .map(|descriptor| self.app_settings.read(descriptor).to_storage())
                .unwrap_or_default();
            match preference.as_str() {
                "粘贴文件夹" => self.start_external_import(paths.to_vec(), target, false),
                "创建映射文件夹" => self.start_external_import(paths.to_vec(), target, true),
                _ => {
                    self.dialog = Some(Dialog {
                        title: "导入文件夹".into(),
                        description: format!(
                            "添加到「{}」",
                            target
                                .file_name()
                                .unwrap_or(target.as_os_str())
                                .to_string_lossy()
                        ),
                        field: None,
                        error: String::new(),
                        note: None,
                        buttons: vec![
                            DialogButton {
                                label: "记住我的选择（可在设置中重新设置）".into(),
                                kind: ButtonKind::Checkbox(false),
                                action: DialogAction::RememberFolderDrop,
                            },
                            DialogButton {
                                label: "粘贴文件夹".into(),
                                kind: ButtonKind::Primary,
                                action: DialogAction::ImportDropped {
                                    paths: paths.to_vec(),
                                    target: target.clone(),
                                    mapping: false,
                                },
                            },
                            DialogButton {
                                label: "创建映射文件夹".into(),
                                kind: ButtonKind::Ghost,
                                action: DialogAction::ImportDropped {
                                    paths: paths.to_vec(),
                                    target,
                                    mapping: true,
                                },
                            },
                        ],
                        dismiss: DialogAction::Dismiss,
                        hover: None,
                    });
                    self.focus = Focus::Dialog;
                }
            }
        } else {
            self.start_external_import(paths.to_vec(), target, false);
        }
    }

    pub(super) fn start_external_import(
        &mut self,
        paths: Vec<PathBuf>,
        target: PathBuf,
        mapping: bool,
    ) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let Some(library) = self.shell.library_root_for_path(&target) else {
            return;
        };
        let rules = self.default_mapping_rules();
        self.state.status_text = "正在导入文件并扫描 Markdown 引用资源…".into();
        self.file_jobs
            .submit(target.clone(), self.hwnd_raw, move || {
                let mut copies = Vec::new();
                let mut mapped = Vec::new();
                let mut warnings = Vec::new();
                let service = mochi_core::mapped_folders::Service::new(&root);
                let mut seen = HashSet::new();
                for path in paths {
                    let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
                    if !seen.insert(identity) {
                        continue;
                    }
                    if mapping && path.is_dir() {
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        match service.add_at(&library, &target, &path, name, rules.clone()) {
                            Ok(_) => mapped.push(path),
                            Err(error) => warnings.push(format!("{}：{error:#}", path.display())),
                        }
                    } else {
                        copies.push(path);
                    }
                }
                let mut report = mochi_core::external_import::copy_paths(&copies, &target);
                report.imported.extend(mapped);
                report.warnings.extend(warnings);
                Ok(crate::file_runtime::Payload::Imported {
                    root,
                    target,
                    report,
                })
            });
    }

    pub(super) fn default_mapping_rules(&self) -> mochi_core::mapped_folders::MappingRules {
        let defaults = self
            .shell
            .workspace()
            .and_then(|ws| {
                mochi_core::mapped_folders::Service::new(&ws.root)
                    .load()
                    .ok()
            })
            .map(|config| config.defaults)
            .unwrap_or_default();
        let read = |key, fallback: Vec<String>| {
            app_settings::descriptor(key)
                .map(|descriptor| {
                    self.app_settings
                        .read(descriptor)
                        .to_storage()
                        .split(';')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or(fallback)
        };
        mochi_core::mapped_folders::MappingRules {
            include: read("mappedFolders.defaultInclude", defaults.include),
            exclude: read("mappedFolders.defaultExclude", defaults.exclude),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Fixture {
        app: App,
        root: PathBuf,
        target: PathBuf,
        external: PathBuf,
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
                "mochi-external-drop-{}",
                mochi_core::paths::random_base36(12)
            ));
            std::fs::create_dir_all(root.join("workspace")).unwrap();
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.open_workspace(HWND::default(), root.join("workspace"), false)
                .unwrap();
            let library = PathBuf::from(&app.shell.workspace().unwrap().libraries[0].path);
            let target = library.join("target");
            let external = root.join("external");
            std::fs::create_dir_all(&target).unwrap();
            std::fs::create_dir_all(&external).unwrap();
            std::fs::write(external.join("image.png"), b"fixture image").unwrap();
            std::fs::write(external.join("note.md"), "![image](image.png)").unwrap();
            app.shell.refresh_tree();
            app.state.view = WorkspaceView::Editor;
            app.renderer.prepare_snapshot(1400, 900, 96.0).unwrap();
            app.paint(HWND::default()).unwrap();
            Self {
                app,
                root,
                target,
                external,
            }
        }
        fn drop_paths(&mut self, paths: Vec<PathBuf>) {
            self.app.paint(HWND::default()).unwrap();
            let index = self
                .app
                .shell
                .rows()
                .iter()
                .position(|r| r.path == self.target)
                .unwrap();
            let rect = self
                .app
                .side
                .layout
                .rect_of(SidebarHit::Row(index))
                .unwrap();
            self.app
                .on_dropped_files(paths, rect.left + 80.0, (rect.top + rect.bottom) / 2.0);
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
        fn choose(&mut self, mapping: bool) {
            let action = self.app.dialog.as_ref().unwrap().buttons.iter().find(|b| matches!(b.action, DialogAction::ImportDropped { mapping: m, .. } if m == mapping)).unwrap().action.clone();
            self.app.run_dialog_action(action);
            self.finish();
        }
        fn clean(self) {
            let root = self.root.clone();
            drop(self);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn external_drop_copies_markdown_to_selected_folder_with_resources() {
        let mut f = Fixture::new();
        f.drop_paths(vec![f.external.join("note.md")]);
        assert!(f.app.dialog.is_none());
        f.finish();
        assert!(f.target.join("note.md").is_file());
        assert!(f.target.join("assets").is_dir());
        assert!(f
            .app
            .shell
            .rows()
            .iter()
            .any(|r| r.path == f.target.join("note.md")));
        assert!(std::fs::read_to_string(f.target.join("note.md"))
            .unwrap()
            .contains("./assets/"));
        assert!(f.app.ai.panel.pending_files.is_empty());
        f.clean();
    }

    #[test]
    fn external_drop_obsidian_report_paints_all_five_imported_images() {
        use crate::ui::{document, draw::DrawCmd, imginfo};
        let mut f = Fixture::new();
        let content = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mochi-core/src/external_import/fixtures/obsidian-images.md"
        ));
        let png = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mochi-core/assets/user-guide/1789482073801-31720-0.png"
        ));
        std::fs::create_dir_all(f.external.join("assets")).unwrap();
        for name in [
            "ulsjvejlocb5aqql",
            "y9vz0z95tl41zoaw",
            "w2uliiza9mnu1rb7",
            "objps18zdg8i7ncv",
            "g697pgxoz4qirtzn",
        ] {
            std::fs::write(f.external.join(format!("assets/import-{name}.png")), png).unwrap();
        }
        let source = f.external.join("测试报告.md");
        std::fs::write(&source, content).unwrap();
        f.drop_paths(vec![source.clone()]);
        f.finish();
        let imported = std::fs::read_to_string(f.target.join("测试报告.md")).unwrap();
        let parsed = document::parse_ranged(&imported);
        let image_sources: Vec<_> = parsed
            .blocks
            .iter()
            .filter_map(|block| match &block.block {
                document::Block::Image { src, .. } => Some(src.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(image_sources.len(), 5, "{imported}");
        let probe = |src: &str| {
            let path = document::resolve_image_src(src, Some(&f.target));
            let dimensions = imginfo::dimensions(Path::new(&path));
            assert!(dimensions.is_some(), "image cannot be decoded: {path}");
            dimensions
        };
        let layout = document::layout_live(&parsed.blocks, &imported, None, 640.0, &probe);
        let mut painted = HashSet::new();
        for line in layout
            .lines
            .iter()
            .filter(|line| line.decoration == document::Decoration::Image)
        {
            let mut list = DrawList::new();
            document::paint_in(
                &mut list,
                Rect::from_size(0.0, 0.0, 700.0, 900.0),
                &layout,
                line.y,
                Some(&f.target),
                theme::tokens().palette(false),
            );
            for cmd in list.cmds() {
                if let DrawCmd::Image { src, .. } = cmd {
                    painted.insert(src.clone());
                }
            }
            assert!(list.finish().is_ok());
        }
        assert_eq!(painted.len(), 5);
        assert_eq!(std::fs::read_to_string(source).unwrap(), content);
        f.clean();
    }

    #[test]
    fn external_drop_cancel_remember_restart_and_reset_preference() {
        let mut f = Fixture::new();
        f.drop_paths(vec![f.external.clone()]);
        assert!(f.app.dialog.is_some());
        assert!(!f.target.join("external").exists());
        f.app.on_edit_key(0x1b, false, false);
        assert!(f.app.dialog.is_none());
        f.drop_paths(vec![f.external.clone()]);
        f.app.on_edit_key(0x20, false, false);
        assert_eq!(
            f.app.dialog.as_ref().unwrap().buttons[0].kind,
            ButtonKind::Checkbox(true)
        );
        // 回车走粘贴，"映射文件夹"仍作为显式可选项保留。
        f.app.on_edit_key(0x0d, false, false);
        f.finish();
        assert!(f.target.join("external/note.md").is_file());
        let descriptor = app_settings::descriptor("sidebar.folderDropAction").unwrap();
        f.app.settings.flush().unwrap();
        let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
            f.root.join("settings.json"),
        ))));
        assert_eq!(
            reopened.read(descriptor),
            SettingValue::Text("粘贴文件夹".into())
        );
        f.drop_paths(vec![f.external.clone()]);
        assert!(f.app.dialog.is_none());
        f.finish();
        assert!(f.target.join("external (1)/note.md").is_file());
        f.app
            .app_settings
            .write(descriptor, &SettingValue::Text("每次询问".into()));
        f.drop_paths(vec![f.external.clone()]);
        assert!(f.app.dialog.is_some());
        f.clean();
    }

    #[test]
    fn external_drop_mapping_is_nested_and_mixed_files_still_copy() {
        let mut f = Fixture::new();
        let attachment = f.root.join("attachment.pdf");
        std::fs::write(&attachment, b"pdf").unwrap();
        f.drop_paths(vec![f.external.clone(), attachment]);
        f.choose(true);
        assert!(!f.target.join("external").exists());
        assert!(f.target.join("attachment.pdf").is_file());
        let source = f.external.canonicalize().unwrap();
        let index = f
            .app
            .shell
            .rows()
            .iter()
            .position(|r| r.path == source)
            .unwrap();
        assert!(f.app.shell.rows()[index].is_mapped_folder);
        assert_eq!(f.app.shell.rows()[index].depth, 1);
        f.app.shell.toggle_loaded(index);
        assert!(f
            .app
            .shell
            .rows()
            .iter()
            .any(|r| r.path == source.join("note.md")));
        f.app.shell.refresh_tree();
        assert!(f
            .app
            .shell
            .rows()
            .iter()
            .any(|r| r.path == source && r.depth == 1));
        let ws = f.app.shell.workspace().unwrap();
        let saved = mochi_core::mapped_folders::Service::new(&ws.root)
            .load()
            .unwrap();
        assert_eq!(saved.folders[0].parent(), f.target);
        let renamed = f.app.shell.rename(&f.target, "renamed").unwrap();
        let saved =
            mochi_core::mapped_folders::Service::new(&f.app.shell.workspace().unwrap().root)
                .load()
                .unwrap();
        assert_eq!(saved.folders[0].parent(), renamed);
        assert!(f
            .app
            .shell
            .rows()
            .iter()
            .any(|r| r.path == source && r.depth == 1));
        assert_eq!(
            f.app
                .app_settings
                .read(app_settings::descriptor("sidebar.folderDropAction").unwrap()),
            SettingValue::Text("每次询问".into())
        );
        f.clean();
    }
}
