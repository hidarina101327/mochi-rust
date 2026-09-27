//! 隔离的真实 D2D 离屏验收；不访问用户设置、工作区，也不占用输入桌面。
use super::*;

impl App {
    pub fn snapshot(scenario: &str, output: &Path) -> Result<()> {
        let io_error = |e: anyhow::Error| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
        };
        let root = std::env::temp_dir().join(format!(
            "mochi-native-snapshot-{}-{}",
            std::process::id(),
            Self::now_ms()
        ));
        std::fs::create_dir_all(&root).map_err(|e| io_error(e.into()))?;
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))))?;
        app.shell.open_workspace(&root, || {}).map_err(io_error)?;
        let library = app
            .shell
            .create_library("knowledge-base", "验收库")
            .map_err(io_error)?;
        let folder = PathBuf::from(&app.shell.workspace().unwrap().libraries[library].path);
        let dpi = std::env::var("MOCHI_SNAPSHOT_DPI")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 96.0 && *v <= 288.0)
            .unwrap_or(144.0);
        let snapshot = app.renderer.prepare_snapshot(
            ((if scenario.starts_with("ai-parity") {
                if scenario.contains("narrow") {
                    1000.0
                } else {
                    1600.0
                }
            } else if scenario.starts_with("home-favorites") && scenario.contains("narrow") {
                760.0
            } else if scenario.starts_with("split-right") && scenario.contains("narrow") {
                960.0
            } else if (scenario.starts_with("templates")
                || scenario.starts_with("marketplace")
                || scenario.starts_with("schedule"))
                && scenario.contains("narrow")
            {
                760.0
            } else if scenario.starts_with("schedule") {
                1560.0
            } else {
                1200.0
            }) * dpi
                / 96.0) as u32,
            (800.0 * dpi / 96.0) as u32,
            dpi,
        )?;
        app.load_chrome_settings();
        if scenario.contains("background") {
            let mut pattern = DrawList::new();
            pattern.rect(Rect::new(0.0, 0.0, 1200.0, 800.0), 0xa8c7dc);
            pattern.rect(Rect::new(0.0, 400.0, 1200.0, 800.0), 0xd9c2a4);
            pattern.rounded_rect(Rect::new(750.0, 80.0, 1080.0, 410.0), 165.0, 0xc4d6b0);
            app.renderer.present(HWND::default(), 0xa8c7dc, &pattern)?;
            let path = root.join("background.png");
            app.renderer.save_snapshot(&snapshot, &path)?;
            app.settings
                .set("background.imagePath", &path.to_string_lossy());
            app.settings.set("app.background.enabled", "true");
            app.settings.set("app.editorLayout.cardEnabled", "true");
            app.apply_setting_side_effects("background.enabled");
        }
        app.state.dark = scenario.contains("dark");
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        if scenario.starts_with("marketplace") {
            app.state.view = WorkspaceView::Marketplace;
            app.marketplace.view.loaded = true;
            if !scenario.contains("empty") {
                app.marketplace.view.catalog = crate::ui::marketplace::fixtures::catalog();
            }
            if scenario.contains("detail") {
                app.marketplace.view.selected = Some(0);
            }
            if scenario.contains("page2") {
                app.marketplace.view.page = 1;
            }
            app.paint(HWND::default())?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(&snapshot, output)?;
            return Ok(());
        }
        if scenario.starts_with("templates") {
            app.verify_templates(&folder, output, &snapshot, scenario)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("folder-drop") {
            let target = folder.join("项目资料");
            let source = root.join("外部资料");
            std::fs::create_dir_all(&target).map_err(|e| io_error(e.into()))?;
            std::fs::create_dir_all(&source).map_err(|e| io_error(e.into()))?;
            let note = folder.join("导入说明.md");
            std::fs::write(&note, "# 导入资料\n\n将文件或文件夹拖到左侧目录树，选择保存副本或映射外部文件夹。\n\nMarkdown 引用的本地图片与附件会自动导入资源目录。\n").map_err(|e| io_error(e.into()))?;
            app.shell.open_file(&note);
            app.shell.refresh_tree();
            app.sync_state();
            app.paint(HWND::default())?;
            let index = app
                .shell
                .rows()
                .iter()
                .position(|r| r.path == target)
                .unwrap();
            let row = app.side.layout.rect_of(SidebarHit::Row(index)).unwrap();
            app.on_dropped_files(vec![source], row.left + 80.0, (row.top + row.bottom) / 2.0);
            app.run_dialog_action(DialogAction::RememberFolderDrop);
            if scenario.contains("settings") {
                app.close_dialog();
                app.open_settings("customization");
                app.settings_overlay = Some(("customization".into(), "sidebar".into()));
                app.prefs.search.set_text("拖入文件夹时");
            }
            app.paint(HWND::default())?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(&snapshot, output)?;
            println!("snapshot {}", output.display());
            return Ok(());
        }
        if scenario == "workspace-dialog-design" {
            app.prepare_home_design_preview(&folder);
            app.open_template_picker(folder.clone());
            app.paint(HWND::default())?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(&snapshot, output)?;
            println!("snapshot {}", output.display());
            return Ok(());
        }
        if scenario.starts_with("home-design") {
            app.prepare_home_design_preview(&folder);
            app.paint(HWND::default())?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(&snapshot, output)?;
            println!("snapshot {}", output.display());
            return Ok(());
        }
        if scenario.starts_with("split-right") {
            app.verify_split_right(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("navigation-preferences") {
            app.verify_navigation_preferences(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("quick-note") {
            app.verify_quick_note(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("document-mode") {
            app.verify_document_mode(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("ai-parity") {
            app.verify_ai_parity(&folder, output, &snapshot, scenario.contains("narrow"))
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("home-favorites") {
            app.verify_home_favorites(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("settings-overlay") {
            let tab = if scenario.starts_with("settings-overlay-accent") {
                let color = if scenario.contains("white") {
                    "#ffffff"
                } else {
                    "#a855f7"
                };
                app.settings.set("app.appearance.accentColor", color);
                app.settings.set(
                    "app.appearance.themeMode",
                    if app.state.dark { "dark" } else { "light" },
                );
                app.apply_setting_side_effects("appearance.accentColor");
                "customization"
            } else if scenario == "settings-overlay-notifications" {
                "notifications"
            } else {
                "general"
            };
            app.open_settings(tab);
            app.paint(HWND::default())?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(&snapshot, output)?;
            println!("snapshot {}", output.display());
            return Ok(());
        }
        if let Some(input) = scenario.strip_prefix("memory-benchmark:") {
            app.benchmark_editor_memory(Path::new(input), &folder, output)?;
            app.renderer.save_snapshot(&snapshot, output)?;
            return Ok(());
        }
        if let Some(input) = scenario.strip_prefix("editor-benchmark:") {
            app.benchmark_editor(Path::new(input), &folder, output, &snapshot)?;
            app.renderer.save_snapshot(&snapshot, output)?;
            return Ok(());
        }
        if scenario.starts_with("base-detail") {
            app.verify_base_detail_modal(&folder, output, &snapshot, scenario.contains("long"))
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("base") {
            app.verify_base(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("feature-parity") {
            app.verify_feature_parity(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("blank-lines") {
            app.verify_blank_lines(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if scenario.starts_with("wysiwyg") {
            app.verify_wysiwyg(&folder, output, &snapshot)
                .map_err(io_error)?;
            return Ok(());
        }
        if let Some(path) = scenario.strip_prefix("file:") {
            if !app.shell.open_file(Path::new(path)) {
                return Err(io_error(anyhow::anyhow!("无法打开验收文件")));
            }
            app.sync_state();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
            loop {
                app.take_file_jobs();
                app.on_pdf_event();
                app.paint(HWND(std::ptr::null_mut()))?;
                let ready = match app.viewer_tab() {
                    Some((_, viewer::Content::Pdf(s))) => {
                        if let Some(e) = &s.error {
                            return Err(io_error(anyhow::anyhow!(e.clone())));
                        }
                        !s.loading && s.ready.iter().any(Option::is_some)
                    }
                    Some((_, viewer::Content::Spreadsheet(s))) => {
                        if !s.error.is_empty() {
                            return Err(io_error(anyhow::anyhow!(s.error.clone())));
                        }
                        !s.loading
                    }
                    _ => true,
                };
                if ready {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    return Err(io_error(anyhow::anyhow!("查看器验收超时")));
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        } else if scenario.starts_with("pdf") {
            let path = folder.join("标注验收.pdf");
            std::fs::write(&path, []).map_err(|e| io_error(e.into()))?;
            app.shell.open_file(&path);
            if let Some(viewer::Content::Pdf(s)) = app.viewer_content_mut() {
                s.loaded(vec![(612.0, 792.0), (612.0, 792.0)]);
                s.requested.fill(true);
                s.annotations.items = vec![
                    sidecars::PdfAnnotation {
                        id: "a".into(),
                        kind: "rect".into(),
                        page: 1,
                        x: 0.10,
                        y: 0.18,
                        width: 0.5,
                        height: 0.15,
                        color: "#e11d48".into(),
                        text: None,
                        created_at: 1,
                        updated_at: 1,
                    },
                    sidecars::PdfAnnotation {
                        id: "b".into(),
                        kind: "text".into(),
                        page: 1,
                        x: 0.12,
                        y: 0.42,
                        width: 0.55,
                        height: 0.09,
                        color: "#e11d48".into(),
                        text: Some("这是重点，需要复习".into()),
                        created_at: 2,
                        updated_at: 2,
                    },
                ];
                s.annotations.selected = Some("b".into());
            }
            // 本场景验图元与坐标，不发后台 PDF 请求；页面真实解码由 PDF 烟测覆盖。
            let (h, rx) = crate::pdf::open(path.clone(), path.clone(), 0);
            app.pdf_jobs.insert(path, (h, rx));
            app.state.ai_panel_open = true;
            app.state.right_panel = RightPanel::Annotations;
        } else if scenario.starts_with("sheet") {
            let path = folder.join("课程.xlsx");
            std::fs::write(&path, []).map_err(|e| io_error(e.into()))?;
            app.shell.open_file(&path);
            if let Some(viewer::Content::Spreadsheet(s)) = app.viewer_content_mut() {
                s.requested = true;
                s.loading = false;
                s.data = Some(mochi_core::office::Workbook {
                    names: vec!["课程".into(), "计划".into()],
                    sheet: mochi_core::office::Sheet {
                        name: "课程".into(),
                        rows: vec![
                            vec!["课程".into(), "日期".into(), "完成度".into()],
                            vec!["操作系统".into(), "2026-09-05".into(), "80%".into()],
                            vec!["算法设计".into(), "2026-09-06".into(), "45%".into()],
                        ]
                        .into(),
                        row_count: 3,
                        column_count: 3,
                    },
                });
            }
        } else if scenario.starts_with("exam") {
            let path = folder.join("算法.exam");
            std::fs::write(&path, mochi_core::exam::template()).map_err(|e| io_error(e.into()))?;
            app.shell.open_file(&path);
            app.load_exam_drafts();
            if let Some(viewer::Content::Exam(s)) = app.viewer_content_mut() {
                s.pick(0, 0, 0);
            }
        } else if scenario.starts_with("schedule") {
            app.prepare_schedule_snapshot(scenario).map_err(io_error)?;
        } else if scenario.starts_with("parameters") {
            app.open_settings("ai");
            if let Some(tab) = app.shell.active_mut() {
                if let TabKind::Settings { section, .. } = &mut tab.kind {
                    *section = "parameters".into();
                }
            }
        } else if scenario.starts_with("shortcuts") {
            app.open_settings("shortcuts");
            crate::ui::shortcuts::load(Some(r#"{"Ctrl+P":"Alt+Q"}"#));
        } else if scenario.starts_with("provider") {
            app.open_settings("ai");
            app.prefs.providers.loaded = true;
            app.prefs.providers.inline_enabled = true;
            app.prefs.providers.form =
                Some(providers::Form::new(mochi_core::ai::models::AiProvider {
                    id: "demo".into(),
                    name: "本地模型".into(),
                    base_url: "http://localhost:11434/v1".into(),
                    api_key: "example-secret".into(),
                    model: "local-model".into(),
                    stream: true,
                    protocol: "openai-completions".into(),
                }));
        } else if scenario.starts_with("ai") {
            app.ai
                .panel
                .input
                .set_text("第一行中文消息\n第二行：继续讨论重构\n第三行：检查多行编辑");
            app.focus = Focus::AiInput;
            app.state.view = WorkspaceView::MochiAi;
            app.ai.panel.sessions = vec![AiSessionMeta {
                id: "demo".into(),
                title: "原生重写讨论".into(),
                message_count: 2,
                ..Default::default()
            }];
            if scenario.contains("markdown") {
                app.ai.panel.sessions[0].message_count = 1;
                let mut source = if scenario.contains("wide") {
                    include_str!("../../tests/fixtures/ai-scroll-verification.md")
                } else {
                    include_str!("../../tests/fixtures/ai-markdown-verification.md")
                }
                .to_owned();
                if scenario.contains("resources") {
                    source = (0..300)
                        .map(|i| {
                            format!("公式 {i}\n\n$$\\frac{{x_{{{i}}}^2+1}}{{{}}}$$\n\n", i + 1)
                        })
                        .collect();
                }
                if scenario.contains("wrap") {
                    let tex = include_str!("../../tests/fixtures/ai-math-wrap-verification.md")
                        .split("$$")
                        .nth(1)
                        .unwrap()
                        .trim();
                    source = source.replace(r"\int_0^1 x^2\,dx = \frac{1}{3}", tex);
                }
                app.ai.panel.active = Some(mochi_core::ai::session::AiConversation {
                    id: "demo".into(),
                    messages: vec![mochi_core::ai::session::AiStoredMessage::new(
                        "assistant",
                        &source,
                    )],
                    ..Default::default()
                });
                app.ai.panel.scroll = if scenario.contains("tail") {
                    f32::MAX
                } else {
                    0.0
                };
                app.ai.panel.stick_to_bottom = false;
                if scenario.contains("side") {
                    let path = folder.join("AI 排版验收.md");
                    std::fs::write(&path, "# AI 侧栏排版\n\n右侧仅显示合成验收内容。\n")
                        .map_err(|e| io_error(e.into()))?;
                    app.shell.open_file(&path);
                    app.sync_state();
                    app.state.view = WorkspaceView::Editor;
                    app.state.ai_panel_open = true;
                    app.state.right_panel = RightPanel::Assistant;
                }
            }
        } else {
            let path = folder.join("墨池原生重写.md");
            let raw="# 墨池原生重写\n\n这里是 **实时预览** 与中文排版。\n\n## 公式\n\n$$\n\\alpha + x^2 = \\frac{1}{2}\n$$\n\n## 代码\n\n```rust\nfn main() {\n    println!(\"你好，墨池\");\n}\n```\n\n- [ ] 完成验收\n";
            std::fs::write(&path, raw).map_err(|e| io_error(e.into()))?;
            app.shell.open_file(&path);
            app.sync_state();
            app.links_job.clear();
            app.links.loading = false;
            app.links
                .data
                .mentions
                .push(mochi_core::metadata_index::UnlinkedMention {
                    source_path: folder.join("另一篇.md"),
                    source_title: "另一篇笔记".into(),
                    line: 1,
                    context: "我在这里提到了墨池原生重写。".into(),
                });
            app.links.collapsed[2] = false;
            if scenario.contains("styles") || scenario.contains("table") {
                let raw="# 样式与表格\n\n<span style=\"color:#c2410c\">彩色正文</span> 和 <mark style=\"background-color:#fff2cc\">重点高亮</mark>。\n\n<p style=\"text-align: center\">居中段落</p>\n\n| 课程 | 状态 | 备注 |\n| --- | --- | --- |\n| 操作系统 | 进行中 | 中文内容 |\n| 算法设计 | 已完成 | 复习计划 |\n";
                if let Some(buffer) = app.shell.active_buffer_mut() {
                    let n = buffer.text().len();
                    buffer.replace_range(0..n, raw);
                }
                app.invalidate_main();
            }
            if scenario.starts_with("math") {
                if let Some(buffer) = app.shell.active_buffer_mut() {
                    let length = buffer.text().len();
                    buffer.replace_range(
                        0..length,
                        include_str!("../../tests/fixtures/math-verification.md"),
                    );
                }
                app.invalidate_main();
            }
            if scenario.starts_with("split") {
                let other = folder.join("第二篇.md");
                std::fs::write(&other,"# 第二篇笔记\n\n双栏编辑共享原始 Markdown 文档。\n\n- [x] 独立滚动\n- [ ] 校对内容\n").map_err(|e|io_error(e.into()))?;
                app.shell.open_file(&other);
                app.shell.open_file(&path);
                app.split_to_right(other);
            }
            if scenario.contains("tail") {
                app.paint(HWND::default())?;
                app.shell
                    .set_active_scroll((app.doc.content_height() - 120.0).max(0.0));
            }
        }
        if scenario == "ai-files" {
            app.ai.panel.provider_missing = false;
            app.ai
                .panel
                .input
                .set_text("请根据附件总结，并保留原文格式。");
            let note = root.join("技术说明.md");
            let binary = root.join("示例数据.bin");
            std::fs::write(&note, "# 合成技术说明\r\n\r\n只用于隔离验收。\r\n")
                .map_err(|e| io_error(e.into()))?;
            std::fs::write(&binary, b"synthetic transport fixture")
                .map_err(|e| io_error(e.into()))?;
            app.paint(HWND::default())?;
            let input = app.ai.layout.rect_of(assistant::Hit::Input).unwrap();
            app.on_dropped_files(
                vec![note.clone(), binary.clone()],
                input.left + 2.0,
                input.top + 2.0,
            );
            app.paint(HWND::default())?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(
                &snapshot,
                &output.with_file_name("native-files-pending.png"),
            )?;
            let remove = app
                .ai
                .layout
                .rect_of(assistant::Hit::RemoveFile(1))
                .unwrap();
            app.on_assistant_click(
                HWND::default(),
                (remove.left + remove.right) / 2.0,
                (remove.top + remove.bottom) / 2.0,
            );
            if app.ai.panel.pending_files != vec![note.clone()] {
                return Err(io_error(anyhow::anyhow!("File remove failed")));
            }
            app.add_ai_files(vec![binary.clone()]);
            let prepared = mochi_core::ai::attachments::prepare(
                app.ai.panel.input.text(),
                &app.ai.panel.pending_files,
                0,
            )
            .map_err(io_error)?;
            if prepared.files.len() != 1 || !prepared.content.contains("只用于隔离验收。") {
                return Err(io_error(anyhow::anyhow!("File preparation failed")));
            }
            app.ai.panel.active=Some(AiConversation{id:"file-history".into(),title:"文件附件验收".into(),messages:vec![AiStoredMessage::new("user",&prepared.display),AiStoredMessage::new("assistant","### 附件已就绪\n\n文本按原始内容准备，二进制文件使用文件输入。\n\n> 这是合成回复，不曾调用模型。")],..Default::default()});
            if !app.ai_persist_active() {
                return Err(io_error(anyhow::anyhow!("File history save failed")));
            }
            app.ai.panel.pending_files.clear();
            app.ai.panel.input.clear();
            app.ai.panel.active = None;
            app.ai_open_session("file-history");
            app.ai.panel.provider_missing = false;
            app.paint(HWND::default())?;
            let unchanged = std::fs::read(&note).map_err(|e| io_error(e.into()))?
                == "# 合成技术说明\r\n\r\n只用于隔离验收。\r\n".as_bytes();
            if !unchanged {
                return Err(io_error(anyhow::anyhow!("Attachment source changed")));
            }
            std::fs::write(output.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"workspace":root,"pendingRemovePassed":true,"historyRestored":true,"sourceUnchanged":unchanged,"modelCalled":false})).unwrap()).map_err(|e|io_error(e.into()))?;
        }
        if scenario == "ai-images" || scenario == "ai-images-paste" {
            let paste = scenario == "ai-images-paste";
            let mut art = DrawList::new();
            art.rect(Rect::from_size(0.0, 0.0, 1200.0, 800.0), 0x335a78);
            art.rect(Rect::from_size(600.0, 0.0, 600.0, 800.0), 0xaacbd4);
            art.rounded_rect(Rect::new(200.0, 180.0, 1000.0, 620.0), 32.0, 0xffffff);
            art.math(
                Rect::new(450.0, 280.0, 750.0, 520.0),
                r"\frac{1}{2}+x^2",
                80.0,
                0x335a78,
            );
            app.renderer.present(HWND::default(), 0xffffff, &art)?;
            let source = root.join("user-drop.png");
            app.renderer.save_snapshot(&snapshot, &source)?;
            app.ai.panel.input.clear();
            app.paint(HWND::default())?;
            let input = app.ai.layout.rect_of(assistant::Hit::Input).unwrap();
            if paste {
                let png = std::fs::read(&source).map_err(|e| io_error(e.into()))?;
                app.paste_ai_image_with(|| Ok(Some(png)));
            } else {
                app.on_dropped_files(vec![source.clone()], input.left + 2.0, input.top + 2.0);
            }
            app.paint(HWND::default())?;
            if app.ai.panel.pending_images.len() != 1 {
                return Err(io_error(anyhow::anyhow!("Dropped image missing")));
            }
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            app.renderer.save_snapshot(
                &snapshot,
                &output.with_file_name("native-images-pending.png"),
            )?;
            let close = app
                .ai
                .layout
                .rect_of(assistant::Hit::RemoveImage(0))
                .unwrap();
            app.on_click(
                (close.left + close.right) / 2.0,
                (close.top + close.bottom) / 2.0,
            );
            if !app.ai.panel.pending_images.is_empty() {
                return Err(io_error(anyhow::anyhow!("Pending remove failed")));
            }
            if paste && !app.ai.panel.input.text().is_empty() {
                return Err(io_error(anyhow::anyhow!(
                    "Paste placeholder removal failed"
                )));
            }
            if paste {
                let png = std::fs::read(&source).map_err(|e| io_error(e.into()))?;
                app.paste_ai_image_with(|| Ok(Some(png)));
            } else {
                app.on_dropped_files(vec![source], input.left + 2.0, input.top + 2.0);
            }
            let reference = app.ai.panel.pending_images[0].clone();
            let mut message = AiStoredMessage::new(
                "user",
                if paste {
                    app.ai.panel.input.text()
                } else {
                    "请查看这张图片"
                },
            );
            message.set("images", serde_json::json!([reference]));
            app.ai.panel.active = Some(AiConversation {
                id: "image-history".into(),
                title: "图片会话".into(),
                messages: vec![message],
                ..Default::default()
            });
            app.ai.panel.pending_images.clear();
            if !app.ai_persist_active() {
                return Err(io_error(anyhow::anyhow!("Image session save failed")));
            }
            if paste {
                app.ai.panel.input.clear();
                app.ai.panel.image_placeholders.clear();
                app.ai.panel.next_image_index = 1;
            }
            app.ai.panel.active = None;
            app.ai.panel.image_previews.clear();
            app.ai_open_session("image-history");
            app.paint(HWND::default())?;
            if app.ai.layout.messages[0].images.len() != 1 || app.ai.panel.image_previews.len() != 1
            {
                return Err(io_error(anyhow::anyhow!("Image history restore failed")));
            }
            std::fs::write(output.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"workspace":root,"pendingRemovePassed":true,"historyRestored":true,"reference":reference,"modelCalled":false,"simulatedClipboard":paste})).unwrap()).map_err(|e|io_error(e.into()))?;
        }
        if scenario.starts_with("command-review") {
            app.commands.review = Some(crate::ui::command_review::State::new(
                crate::export_requests::Request {
                    id: "snapshot-only".into(),
                    source: "Write-Output '中文命令审批验收'\nGet-ChildItem -LiteralPath .".into(),
                    output: folder.to_string_lossy().into_owned(),
                    format: "shell".into(),
                    content: None,
                    created_at: 0,
                    state: "pending".into(),
                    session_id: None,
                },
            ));
        } else if scenario.starts_with("file-review") {
            let entry = mochi_core::ai::agent_inbox::InboxEntry::from_value(&serde_json::json!({
                "id":"snapshot-only", "operation":{"kind":"overwrite", "path":folder.join("墨池原生重写.md"),
                "content":"# 修改后的笔记\n\n这是需要批准的新内容。", "status":"pending"}
            })).unwrap();
            app.commands.review = Some(crate::ui::command_review::State::file(
                entry,
                Some("# 原始笔记\n\n原有正文保持可见。".into()),
                None,
            ));
        } else if scenario.starts_with("save-conflict") {
            app.show_save_conflict(folder.join("墨池原生重写.md"));
        }
        app.shell.refresh_tree();
        app.sync_state();
        if scenario == "workflow" {
            app.verify_workflow(output, &snapshot).map_err(io_error)?;
        }
        app.state.active_file_is_pdf = app.shell.active().is_some_and(|t| t.is_pdf());
        if let Some(scroll) = std::env::var("MOCHI_SNAPSHOT_SCROLL")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
        {
            app.paint(HWND::default())?;
            app.shell
                .set_active_scroll(scroll.min(app.doc.content_height()));
        }
        if scenario.starts_with("notice-") {
            app.state.dark = scenario.contains("dark");
            if scenario.contains("progress") {
                app.show_global_progress("正在建立搜索索引", 12, 48);
            } else {
                app.show_global_notice("当前按名称排序\n切换为手动排序后，可拖拽调整顺序");
                app.status_bar.toast.expires = i64::MAX;
            }
        }
        app.paint(HWND(std::ptr::null_mut()))?;
        if scenario.starts_with("schedule") && scenario.contains("dropdown") {
            let pick = crate::ui::agenda::Hit::Panel(crate::ui::agenda::panel::Hit::Pick(
                crate::ui::agenda::panel::PickSlot::Parent,
            ));
            if let Some(rect) = app.sched.layout.rect_of(&pick) {
                app.on_schedule_click(rect.left + 20.0, (rect.top + rect.bottom) / 2.0);
                app.paint(HWND::default())?;
            }
        }
        if scenario == "ai-markdown-resources" {
            let source = app.ai.panel.active.as_ref().unwrap().messages[0]
                .content()
                .to_owned();
            let mut samples = Vec::new();
            for (label, part) in [
                ("top", 0.0),
                ("middle", 0.5),
                ("bottom", 1.0),
                ("top-again", 0.0),
            ] {
                app.ai.panel.scroll = app.ai.layout.max_scroll() * part;
                app.paint(HWND::default())?;
                let (bitmaps, pixels) = app.renderer.math_bitmap_stats();
                let formulas = app
                    .list
                    .cmds()
                    .iter()
                    .filter_map(|c| {
                        if let crate::ui::draw::DrawCmd::Math { tex, .. } = c {
                            Some(tex.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                if bitmaps == 0 || bitmaps >= 20 || formulas.len() != bitmaps {
                    return Err(io_error(anyhow::anyhow!(
                        "Unexpected long-reply math resource count"
                    )));
                }
                if part == 1.0 && !formulas.iter().any(|tex| tex.contains("x_{299}")) {
                    return Err(io_error(anyhow::anyhow!("Last formula not revealed")));
                }
                let file = output.with_file_name(format!("native-resources-{label}.png"));
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
                }
                app.renderer.save_snapshot(&snapshot, &file)?;
                samples.push(serde_json::json!({"position":label,"bitmaps":bitmaps,"decodedPixels":pixels,"formulas":formulas}));
            }
            if source != app.ai.panel.active.as_ref().unwrap().messages[0].content() {
                return Err(io_error(anyhow::anyhow!(
                    "Resource verification changed source"
                )));
            }
            std::fs::write(output.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"formulaCount":300,"samples":samples,"sourceUnchanged":true})).unwrap()).map_err(|e|io_error(e.into()))?;
        }
        if scenario.starts_with("ai-markdown-wrap") && scenario.contains("tail") {
            let before = app.ai.panel.active.as_ref().unwrap().messages[0]
                .content()
                .to_owned();
            let (action, rect, _) = app.ai.layout.messages[0]
                .body
                .copy_targets()
                .filter(|(_, _, kind)| *kind == crate::ui::ai_markdown::CopyKind::Formula)
                .next_back()
                .unwrap();
            let origin = app.ai.layout.body_origin(0, app.ai.panel.scroll).unwrap();
            let point = (
                origin.0 + (rect.left + rect.right) / 2.0,
                origin.1 + rect.bottom - 2.0,
            );
            if app
                .ai
                .layout
                .content_at(app.ai.panel.scroll, point.0, point.1, false)
                != Some((0, action))
            {
                return Err(io_error(anyhow::anyhow!(
                    "Wrapped formula last-row hit failed"
                )));
            }
            let mut copied = String::new();
            if !app.ai_content_copy_with(0, action, |p| {
                copied = p.text.clone();
                true
            }) {
                return Err(io_error(anyhow::anyhow!("Wrapped formula copy failed")));
            }
            let expected = include_str!("../../tests/fixtures/ai-math-wrap-verification.md")
                .split("$$")
                .nth(1)
                .unwrap()
                .trim();
            let unchanged = before == app.ai.panel.active.as_ref().unwrap().messages[0].content();
            let no_horizontal = app.ai.layout.messages[0]
                .body
                .scroll_regions
                .iter()
                .all(|r| r.id.kind != 2);
            if copied != expected || !unchanged || !no_horizontal {
                return Err(io_error(anyhow::anyhow!(
                    "Wrapped formula verification failed"
                )));
            }
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            std::fs::write(output.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"copied":copied,"sourceUnchanged":unchanged,"noHorizontalFormula":no_horizontal,"formulaHeight":rect.height(),"osClipboardUntouched":true})).unwrap()).map_err(|e|io_error(e.into()))?;
        }
        if scenario.starts_with("ai-markdown-wide") && scenario.contains("scroll") {
            let ids = app.ai.layout.messages[0]
                .body
                .scroll_regions
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>();
            if ids.len() != 3 {
                return Err(io_error(anyhow::anyhow!(
                    "Expected table/code/formula horizontal regions"
                )));
            }
            let mut states = Vec::new();
            for id in ids {
                let region = app.ai.layout.messages[0]
                    .body
                    .scroll_regions
                    .iter()
                    .find(|r| r.id == id)
                    .unwrap()
                    .clone();
                let target = app.ai.layout.messages[0].top + 26.0 + region.viewport.top
                    - app.ai.layout.messages_rect.height() / 3.0;
                app.ai.panel.scroll = target.clamp(0.0, app.ai.layout.max_scroll());
                app.paint(HWND::default())?;
                let origin = app.ai.layout.body_origin(0, app.ai.panel.scroll).unwrap();
                let point = (
                    origin.0 + (region.viewport.left + region.viewport.right) / 2.0,
                    origin.1 + (region.viewport.top + region.viewport.bottom) / 2.0,
                );
                if !app.on_horizontal_wheel(point.0, point.1, i16::MAX) {
                    return Err(io_error(anyhow::anyhow!("Horizontal wheel route failed")));
                }
                app.paint(HWND::default())?;
                app.on_mouse_move(point.0, point.1);
                app.paint(HWND::default())?;
                let offset = region.offset(Some(&app.ai.layout.messages[0].horizontal));
                if (offset - region.max_x()).abs() > 0.1 {
                    return Err(io_error(anyhow::anyhow!("Horizontal end position failed")));
                }
                let label = match id.kind {
                    0 => "code",
                    1 => "table",
                    _ => "math",
                };
                let path = output.with_file_name(format!("native-scroll-{label}.png"));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
                }
                app.renderer.save_snapshot(&snapshot, &path)?;
                let mut copied = None;
                if id.kind == 2 {
                    let Some((m, a)) =
                        app.ai
                            .layout
                            .content_at(app.ai.panel.scroll, point.0, point.1, false)
                    else {
                        return Err(io_error(anyhow::anyhow!(
                            "Scrolled formula copy hit failed"
                        )));
                    };
                    if !app.ai_content_copy_with(m, a, |p| {
                        copied = Some(p.text.clone());
                        true
                    }) {
                        return Err(io_error(anyhow::anyhow!("Scrolled formula copy failed")));
                    }
                }
                states.push(serde_json::json!({"kind":label,"offset":offset,"max":region.max_x(),"copied":copied}));
            }
            let unchanged = app.ai.panel.active.as_ref().unwrap().messages[0].content()
                == include_str!("../../tests/fixtures/ai-scroll-verification.md");
            if !unchanged {
                return Err(io_error(anyhow::anyhow!(
                    "Scrolling changed message source"
                )));
            }
            std::fs::write(output.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"regions":states,"sourceUnchanged":unchanged,"osClipboardUntouched":true})).unwrap()).map_err(|e|io_error(e.into()))?;
        }
        if scenario.starts_with("ai-markdown") && scenario.contains("copy") {
            let (action, rect, _) = app.ai.layout.messages[0]
                .body
                .copy_targets()
                .find(|(_, _, kind)| *kind == crate::ui::ai_markdown::CopyKind::Table)
                .unwrap();
            let origin = app.ai.layout.body_origin(0, app.ai.panel.scroll).unwrap();
            let point = (
                origin.0 + (rect.left + rect.right) / 2.0,
                origin.1 + (rect.top + rect.bottom) / 2.0,
            );
            let hit = app
                .ai
                .layout
                .content_at(app.ai.panel.scroll, point.0, point.1, false);
            if hit != Some((0, action)) {
                return Err(io_error(anyhow::anyhow!("Table copy hit test failed")));
            }
            app.on_mouse_move(point.0, point.1);
            let mut payload = None;
            if !app.ai_content_copy_with(0, action, |value| {
                payload = Some(value.clone());
                true
            }) {
                return Err(io_error(anyhow::anyhow!("Table copy controller failed")));
            }
            let payload = payload.unwrap();
            let source_unchanged = app.ai.panel.active.as_ref().unwrap().messages[0].content()
                == include_str!("../../tests/fixtures/ai-markdown-verification.md");
            let report = serde_json::json!({"passed":source_unchanged,"text":payload.text,"html":payload.html,"sourceUnchanged":source_unchanged,"osClipboardUntouched":true,"hover":app.ai.panel.hover_content});
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
            }
            std::fs::write(
                output.with_extension("json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .map_err(|e| io_error(e.into()))?;
            app.paint(HWND::default())?;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(e.into()))?;
        }
        app.renderer.save_snapshot(&snapshot, output)?;
        println!("snapshot {}", output.display());
        Ok(())
    }
}
