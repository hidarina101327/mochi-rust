//! 为可搬移的多维表格编辑器执行隔离的真实 D2D 验证。
use super::*;
use anyhow::{ensure, Context};

impl App {
    pub(super) fn verify_base_detail_modal(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
        long: bool,
    ) -> anyhow::Result<()> {
        use mochi_core::base::*;
        let mut document = create_base_document();
        let table = &mut document.tables[0];
        table.name = "学习行动".into();
        table.fields[0].name = "学习主题".into();
        table
            .fields
            .push(create_base_field(FieldType::Progress, "完成进度"));
        table
            .fields
            .push(create_base_field(FieldType::Reference, "关联资料"));
        if long {
            for index in 4..=21 {
                table.fields.push(create_base_field(
                    FieldType::Text,
                    &format!("学习记录 {index}"),
                ));
            }
        }
        let note = folder.join("课后笔记.md");
        std::fs::write(&note, "# 课后笔记\n\n记录学习进展。")?;
        let link = mochi_url::build_mochi_resource_url(
            &note,
            ResourceKind::File,
            self.shell
                .workspace()
                .map(|workspace| workspace.root.as_path()),
        );
        let mut record = BaseRecord {
            id: "modal-record".into(),
            ..Default::default()
        };
        for (index, field) in table.fields.iter().enumerate() {
            record.values.insert(
                field.id.clone(),
                match index {
                    0 => serde_json::json!("完成线性代数第一章"),
                    1 => serde_json::json!(65),
                    2 => serde_json::json!([link]),
                    _ => {
                        serde_json::json!(format!("整理第 {} 次练习的收获与下一步行动", index + 1))
                    }
                },
            );
        }
        table.records.push(record);
        let location = BaseLocation {
            table_id: table.id.clone(),
            record_id: Some("modal-record".into()),
            field_id: table.fields.last().map(|field| field.id.clone()),
        };
        let path = folder.join("记录详情.mcb");
        std::fs::write(&path, serialize_base_document(&document)?)?;
        self.state.ai_panel_open = false;
        self.shell.refresh_tree();
        self.shell.open_file(&path);
        self.sync_state();
        self.verify_base_click(base_view::Hit::Record(0))?;
        if long {
            self.reveal_base(location);
            self.on_base_click(base_view::Hit::ToggleEditing);
        }
        self.verify_frame(
            output,
            if long { "long-scrolled" } else { "compact" },
            snapshot,
        )?;
        if !long {
            self.on_click(12.0, 100.0);
            ensure!(
                !self.base_detail_open(),
                "modal backdrop did not close details"
            );
            self.verify_base_click(base_view::Hit::Record(0))?;
            ensure!(self.on_edit_key(0x1b, false, false));
            ensure!(!self.base_detail_open(), "Escape did not close details");
            self.verify_base_click(base_view::Hit::Record(0))?;
            self.verify_base_click(base_view::Hit::ToggleEditing)?;
            self.verify_base_click(base_view::Hit::AddReference(0, 2))?;
            ensure!(self.object_picker.is_some());
            self.on_edit_key(0x1b, false, false);
            ensure!(self.object_picker.is_none());
            ensure!(self.base_detail_open());
            self.verify_base_click(base_view::Hit::Cell(0, 1))?;
            self.verify_base_dialog("72")?;
            self.verify_base_click(base_view::Hit::OpenReference(0, 2, 0))?;
            ensure!(self.active_file_path().as_deref() == Some(note.as_path()));
            println!("base detail modal: backdrop, Escape, edit dialog, reference menu and link navigation passed");
        }
        Ok(())
    }
    fn verify_base_click(&mut self, hit: base_view::Hit) -> anyhow::Result<()> {
        self.paint(HWND::default())?;
        let Some((_, viewer::Content::Base(state))) = self.viewer_tab() else {
            anyhow::bail!("base viewer missing")
        };
        let l = base_view::layout(state, self.base_interaction_area());
        let rect = l
            .entries
            .iter()
            .rev()
            .find(|(_, entry)| *entry == hit)
            .map(|(rect, _)| *rect)
            .context("base hit missing from viewport")?;
        self.on_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        Ok(())
    }
    fn verify_base_dialog(&mut self, text: &str) -> anyhow::Result<()> {
        let dialog = self.dialog.as_mut().context("base edit dialog missing")?;
        dialog
            .field
            .as_mut()
            .context("base text field missing")?
            .set_text(text);
        let action = dialog.buttons.last().unwrap().action.clone();
        self.run_dialog_action(action);
        ensure!(
            self.dialog.is_none(),
            "base dialog failed: {:?}",
            self.dialog.as_ref().map(|d| &d.error)
        );
        Ok(())
    }
    pub(super) fn verify_base(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        use base_view::Hit;
        self.state.ai_panel_open = false;
        self.open_base_dialog(folder.to_path_buf());
        self.verify_base_dialog("多维数据验收")?;
        let path = self
            .active_file_path()
            .context("base file was not created")?;
        ensure!(path.extension().is_some_and(|ext| ext == "mcb"));
        ensure!(self.shell.active().unwrap().buffer().is_none());
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if !state.editing)
        );
        self.verify_base_click(Hit::ToggleEditing)?;
        self.verify_base_click(Hit::NewField)?;
        self.verify_base_click(Hit::Menu(2))?;
        self.verify_base_dialog("分类")?;
        self.verify_base_click(Hit::Field(1))?;
        self.verify_base_click(Hit::Menu(1))?;
        self.verify_base_click(Hit::Menu(0))?;
        self.verify_base_dialog("标签甲")?;
        self.verify_base_click(Hit::Field(1))?;
        self.verify_base_click(Hit::Menu(1))?;
        self.verify_base_click(Hit::Menu(0))?;
        self.verify_base_dialog("标签乙")?;
        self.verify_base_click(Hit::Field(1))?;
        self.verify_base_click(Hit::Menu(1))?;
        self.verify_base_click(Hit::Menu(2))?;
        self.verify_base_click(Hit::Menu(1))?;
        self.verify_base_click(Hit::Menu(6))?;
        self.verify_base_click(Hit::Dismiss)?;
        self.verify_base_click(Hit::NewField)?;
        self.verify_base_click(Hit::Menu(9))?;
        self.verify_base_dialog("完成度")?;
        for (r, (name, option, progress)) in [
            ("记录甲", 1, "32"),
            ("记录乙", 2, "75"),
            ("记录丙", 1, "100"),
        ]
        .into_iter()
        .enumerate()
        {
            self.verify_base_click(Hit::NewRecord)?;
            self.verify_base_click(Hit::Cell(r, 0))?;
            self.verify_base_dialog(name)?;
            self.verify_base_click(Hit::Cell(r, 1))?;
            self.verify_base_click(Hit::Menu(option))?;
            self.verify_base_click(Hit::Cell(r, 2))?;
            self.verify_base_dialog(progress)?;
        }
        self.verify_base_click(Hit::Cell(0, 2))?;
        self.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("101");
        let action = self
            .dialog
            .as_ref()
            .unwrap()
            .buttons
            .last()
            .unwrap()
            .action
            .clone();
        self.run_dialog_action(action);
        ensure!(
            self.dialog.as_ref().is_some_and(|d| !d.error.is_empty()),
            "invalid progress was accepted"
        );
        self.close_dialog();
        ensure!(self.save_active());
        ensure!(!self.shell.active().unwrap().dirty());
        self.verify_frame(output, "grid", snapshot)?;
        self.verify_base_click(Hit::Cell(0, 1))?;
        self.verify_frame(output, "colored-options", snapshot)?;
        ensure!(self.on_edit_key(0x1b, false, false));
        // 可以修改已有数据的列。可转换的内容会保留；
        // 不兼容的值只有在用户通过对话框明确确认后才会清除。
        self.verify_base_click(Hit::Field(0))?;
        self.verify_base_click(Hit::Menu(3))?; // 文本 → 下拉菜单
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.table().fields[0].field_type==mochi_core::base::FieldType::SingleSelect)
        );
        self.verify_base_click(Hit::Field(0))?;
        self.verify_base_click(Hit::Menu(2))?; // 下拉菜单 → 文本
        self.verify_base_click(Hit::Field(0))?;
        self.verify_base_click(Hit::Menu(2))?; // 文本 → 数字，不兼容
        ensure!(self
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.description.contains("清空 3 个")));
        self.verify_frame(output, "type-conversion-confirmation", snapshot)?;
        self.run_dialog_action(DialogAction::Dismiss);
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.table().fields[0].field_type==mochi_core::base::FieldType::Text)
        );
        let original_names = match self.viewer_tab() {
            Some((_, viewer::Content::Base(state))) => state
                .table()
                .records
                .iter()
                .map(|record| {
                    record.values[&state.table().fields[0].id]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>(),
            _ => unreachable!(),
        };
        self.verify_base_click(Hit::Field(0))?;
        self.verify_base_click(Hit::Menu(2))?;
        let confirm = self
            .dialog
            .as_ref()
            .unwrap()
            .buttons
            .last()
            .unwrap()
            .action
            .clone();
        self.run_dialog_action(confirm);
        ensure!(self.dialog.is_none());
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.table().fields[0].field_type==mochi_core::base::FieldType::Number&&state.table().records.iter().all(|record|record.values[&state.table().fields[0].id].is_null()))
        );
        self.verify_base_click(Hit::Field(0))?;
        self.verify_base_click(Hit::Menu(1))?;
        for (row, name) in original_names.iter().enumerate() {
            self.verify_base_click(Hit::Cell(row, 0))?;
            self.verify_base_dialog(name)?;
        }
        self.verify_base_click(Hit::NewField)?;
        self.verify_base_click(Hit::Menu(5))?;
        self.verify_base_dialog("时间点")?;
        self.verify_base_click(Hit::Cell(0, 3))?;
        self.verify_base_dialog("2026-09-12T09:15")?;
        self.verify_base_click(Hit::NewField)?;
        self.verify_base_click(Hit::Menu(6))?;
        self.verify_base_dialog("时间段")?;
        // 真实的水平滚轮操作应能显示新增的范围列。
        let point = (self.editor_area.right - 24.0, self.editor_area.top + 190.0);
        ensure!(self.on_horizontal_wheel(point.0, point.1, 1200));
        self.verify_base_click(Hit::Cell(0, 4))?;
        self.verify_base_dialog("2026-09-12 09:15 至 2026-09-13 17:00")?;
        self.verify_base_click(Hit::Cell(0, 4))?;
        self.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("2026-09-13T17:00 至 2026-09-12T09:15");
        let action = self
            .dialog
            .as_ref()
            .unwrap()
            .buttons
            .last()
            .unwrap()
            .action
            .clone();
        self.run_dialog_action(action);
        ensure!(self
            .dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.error.is_empty()));
        self.close_dialog();
        ensure!(self.save_active());
        self.verify_frame(output, "time-fields", snapshot)?;
        self.verify_base_click(Hit::NewView)?;
        self.verify_base_click(Hit::Menu(1))?;
        self.verify_base_dialog("分类看板")?;
        ensure!(self.save_active());
        self.verify_frame(output, "board", snapshot)?;
        let saved = std::fs::read_to_string(&path)?;
        let document = mochi_core::base::parse_base_document(&saved)?;
        ensure!(document.tables[0].records.len() == 3);
        ensure!(document.tables[0].fields[3].field_type == mochi_core::base::FieldType::DateTime);
        ensure!(document.tables[0].fields[4].field_type == mochi_core::base::FieldType::DateRange);
        ensure!(
            document.tables[0].records[0].values[&document.tables[0].fields[4].id]
                == serde_json::json!(["2026-09-12T09:15", "2026-09-13T17:00"])
        );
        ensure!(
            document.tables[0].fields[1].options[1].color == mochi_core::base::OptionColor::Purple
        );
        self.shell.close_tab(self.shell.active_tab().unwrap());
        ensure!(self.shell.open_file(&path));
        self.sync_state();
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(s)))if s.view().view_type==mochi_core::base::ViewType::Board&&!s.dirty)
        );
        self.verify_frame(output, "reopened", snapshot)?;
        self.verify_base_click(Hit::ToggleEditing)?;
        self.verify_base_click(Hit::Cell(0, 0))?;
        self.verify_base_dialog("本地修订")?;
        let external = format!("{saved}\n");
        std::fs::write(&path, &external)?;
        ensure!(!self.save_active());
        ensure!(self.dialog.is_some());
        ensure!(std::fs::read_to_string(&path)? == external);
        ensure!(self.shell.active().unwrap().dirty());
        self.run_dialog_action(DialogAction::ConflictOverwrite(path.clone()));
        ensure!(self.dialog.is_none());
        ensure!(std::fs::read_to_string(&path)?.contains("本地修订"));
        let invalid = folder.join("无效数据.mcb");
        std::fs::write(&invalid, "{\"format\":\"mochi-base\",\"version\":99}")?;
        ensure!(self.shell.open_file(&invalid));
        ensure!(matches!(
            self.viewer_tab(),
            Some((_, viewer::Content::Unsupported { message: Some(_) }))
        ));
        ensure!(self.shell.active().unwrap().buffer().is_none());
        let markdown = folder.join("普通笔记.md");
        std::fs::write(&markdown, "# 标题\n\n原始正文\n")?;
        ensure!(self.shell.open_file(&markdown));
        ensure!(self.shell.active().unwrap().buffer().is_some());
        self.verify_base_navigation(folder, output, snapshot)?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":true,"gridAndBoard":true,"customColoredOptions":true,"typedCellEdit":true,"existingFieldTypeConversion":true,"destructiveConversionConfirmation":true,"dateTimeAndRangeEdit":true,"invertedRangeRejected":true,"invalidProgressRejected":true,"saveReopen":true,"diskConflictPreserved":true,"invalidFileRejected":true,"markdownUnchanged":true,"defaultViewing":true,"referencePicker":true,"referenceNavigation":true,"columnResize":true,"horizontalScrollbar":true,"stableSearchLocation":true,"recordMochiLink":true,"isolatedWorkspace":true,"file":path}),
            )?,
        )?;
        println!("base verification {}", output.display());
        Ok(())
    }

    fn verify_base_navigation(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        use base_view::Hit;
        use mochi_core::base::*;
        let root = self.shell.workspace().unwrap().root.clone();
        let note = folder.join("课后笔记.md");
        let other = folder.join("新增参考资料.md");
        std::fs::write(&note, "# 课后笔记\n\n保留学习证据。\n")?;
        std::fs::write(&other, "# 新增参考资料\n\n从表格引用这篇笔记。\n")?;
        let note_url = mochi_url::build_mochi_resource_url(&note, ResourceKind::File, Some(&root));
        let mut document = create_base_document();
        let table = &mut document.tables[0];
        table.name = "学习行动".into();
        table.fields[0].name = "学习主题".into();
        table.fields[0].width = Some(220.0);
        table
            .fields
            .push(create_base_field(FieldType::Progress, "进度"));
        table
            .fields
            .push(create_base_field(FieldType::Reference, "关联材料"));
        table.fields[2].width = Some(250.0);
        table
            .fields
            .push(create_base_field(FieldType::Text, "学习产出"));
        table.fields[3].width = Some(260.0);
        table
            .fields
            .push(create_base_field(FieldType::Text, "下一步行动"));
        table.fields[4].width = Some(300.0);
        let mut record = BaseRecord {
            id: "record-learning".into(),
            ..Default::default()
        };
        record.values.insert(
            table.fields[0].id.clone(),
            serde_json::json!("完成线性代数第一章"),
        );
        record
            .values
            .insert(table.fields[1].id.clone(), serde_json::json!(65));
        record
            .values
            .insert(table.fields[2].id.clone(), serde_json::json!([note_url]));
        record.values.insert(
            table.fields[3].id.clone(),
            serde_json::json!("整理矩阵运算例题"),
        );
        record.values.insert(
            table.fields[4].id.clone(),
            serde_json::json!("复测三个薄弱知识点"),
        );
        table.records.push(record);
        let table_id = table.id.clone();
        let reference_id = table.fields[2].id.clone();
        let path = folder.join("引用与记录定位.mcb");
        std::fs::write(&path, serialize_base_document(&document)?)?;
        self.shell.refresh_tree();
        self.shell.refresh_status();
        ensure!(self.shell.open_file(&path));
        self.sync_state();
        self.paint(HWND::default())?;
        self.status_bar.toast.message.clear();
        self.verify_base_click(Hit::Cell(0, 0))?;
        ensure!(self.dialog.is_none());
        ensure!(!self.shell.active().unwrap().dirty());
        self.verify_frame(output, "default-view", snapshot)?;
        self.verify_base_click(Hit::Record(0))?;
        self.verify_frame(output, "reference-detail", snapshot)?;
        self.verify_base_click(Hit::OpenReference(0, 2, 0))?;
        ensure!(self.active_file_path().as_deref() == Some(note.as_path()));
        ensure!(self.shell.open_file(&path));
        self.sync_state();
        self.verify_base_click(Hit::ToggleEditing)?;
        self.verify_base_click(Hit::AddReference(0, 2))?;
        for _ in 0..200 {
            self.poll_object_picker();
            if self.object_picker.as_ref().is_some_and(|d| !d.loading) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let picker = self
            .object_picker
            .as_mut()
            .context("object picker absent")?;
        picker.state.query.set_text("新增参考资料");
        picker.state.query_changed();
        ensure!(picker.state.visible_count() == 1);
        picker.state.toggle_row(0);
        self.verify_frame(output, "reference-picker", snapshot)?;
        self.on_edit_key(0x0d, false, true);
        ensure!(self.object_picker.is_none());
        self.on_base_click(Hit::Dismiss);
        self.on_base_click(Hit::CloseDetail);
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.table().records[0].values[&reference_id].as_array().is_some_and(|urls|urls.len()==2))
        );
        self.paint(HWND::default())?;
        let l = match self.viewer_tab() {
            Some((_, viewer::Content::Base(state))) => base_view::layout(state, self.editor_area),
            _ => unreachable!(),
        };
        let handle = l
            .entries
            .iter()
            .find(|(_, hit)| *hit == Hit::Resize(0))
            .context("resize handle absent")?
            .0;
        let point = (handle.left + 4.0, handle.top + 16.0);
        ensure!(self.cursor_for(point.0, point.1) == Some(platform::CURSOR_RESIZE_HORIZONTAL));
        self.on_click(point.0, point.1);
        ensure!(self.is_dragging());
        self.on_mouse_move(point.0 + 64.0, point.1);
        self.end_drag();
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.table().fields[0].width==Some(284.0))
        );
        self.paint(HWND::default())?;
        let l = match self.viewer_tab() {
            Some((_, viewer::Content::Base(state))) => base_view::layout(state, self.editor_area),
            _ => unreachable!(),
        };
        let thumb = l.scroll_thumb.context("horizontal scrollbar absent")?;
        let track = l.scroll_track.unwrap();
        self.on_click(thumb.left + 3.0, thumb.top + 3.0);
        ensure!(self.is_dragging());
        self.on_mouse_move(track.right - 3.0, thumb.top + 3.0);
        self.end_drag();
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.scroll_x>100.0&&state.scroll_y==0.0)
        );
        self.verify_frame(output, "resized-and-scrolled", snapshot)?;
        ensure!(self.save_active());
        ensure!(
            parse_base_document(&std::fs::read_to_string(&path)?)?.tables[0].fields[0].width
                == Some(284.0)
        );
        self.verify_base_click(Hit::ToggleEditing)?;
        let result = {
            let workspace = self.shell.workspace().unwrap();
            workspace.index.run_full_index()?;
            SearchService::new(&root, Arc::clone(&workspace.index)).search(
                "课后笔记",
                &mochi_core::search::SearchOptions {
                    extensions: vec![".mcb".into()],
                    ..Default::default()
                },
            )
        };
        ensure!(!result.groups.is_empty());
        ensure!(result.groups[0].matches[0]
            .base_location
            .as_ref()
            .is_some_and(|location| location.table_id == table_id
                && location.record_id.as_deref() == Some("record-learning")
                && location.field_id.as_deref() == Some(&reference_id)));
        self.open_search();
        {
            let search = self.search.as_mut().unwrap();
            search.query.set_text("课后笔记");
            search.apply_result(result);
            search.selected = 1;
        }
        self.paint(HWND::default())?;
        self.verify_frame(output, "display-text-search", snapshot)?;
        self.open_selected_search_row();
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.detail==Some(0)&&state.selected==Some((0,2))&&!state.editing)
        );
        self.verify_frame(output, "search-record-location", snapshot)?;
        let url = mochi_url::build_mochi_record_url(
            &path,
            &table_id,
            "record-learning",
            Some(&reference_id),
            Some(&root),
        );
        ensure!(self.shell.open_file(&note));
        self.open_link(&url);
        ensure!(self.active_file_path().as_deref() == Some(path.as_path()));
        ensure!(
            matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.detail==Some(0)&&state.selected==Some((0,2)))
        );
        self.verify_frame(output, "record-link-location", snapshot)?;
        let (task, _) =
            self.agenda_store()
                .unwrap()
                .mutate(mochi_core::agenda::Source::User, |ed| {
                    ed.create_task(mochi_core::agenda::TaskDraft {
                        title: "引用日程回跳验收".into(),
                        ..Default::default()
                    })
                })?;
        self.open_link(&format!(
            "mochi://open?path=agenda&kind=task&item={}",
            mochi_url::form_encode(&task)
        ));
        ensure!(self.state.view == WorkspaceView::Schedule);
        ensure!(
            self.sched
                .view
                .panel
                .as_ref()
                .and_then(|panel| panel.record_id())
                == Some(task.as_str())
        );
        self.open_link(&url);
        ensure!(self.state.view == WorkspaceView::Editor);
        ensure!(self.active_file_path().as_deref() == Some(path.as_path()));
        Ok(())
    }
}
