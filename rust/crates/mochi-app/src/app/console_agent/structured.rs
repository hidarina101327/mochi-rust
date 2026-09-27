//! 整理控制台结构化数据和操作的响应内容。
use super::*;
impl App {
    pub(super) fn console_structured(
        &mut self,
        name: &str,
        action: &str,
        a: &Value,
    ) -> CResult<Value> {
        let d = data(a);
        if name == "canvas_manage" {
            let p = self.console_target(a, action != "get")?;
            let mut a = a.clone();
            a["path"] = json!(p);
            return self
                .canvas_agent_action(
                    if action == "get" {
                        "canvas_get"
                    } else {
                        "canvas_manage"
                    },
                    &a,
                )
                .map_err(anyhow::Error::msg);
        }
        let path = self.console_target(a, !matches!(action, "get" | "list" | "locate"))?;
        let path_str = path.to_string_lossy().into_owned();
        if name == "base_manage" {
            ensure!(
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("mcb")),
                "需要 .mcb 文件"
            );
            let original = std::fs::read_to_string(&path)?;
            let doc = mochi_core::base::parse_base_document(&original)?;
            if action == "get" {
                return Ok(versioned(serde_json::to_value(doc)?));
            }
            ensure!(!self.shell.tabs().iter().any(|t|t.path()==Some(path.as_path())&&(t.dirty()||matches!(&t.kind,TabKind::Viewer{content:viewer::Content::Base(s),..} if s.editing))),"表格有未保存编辑");
            check_revision(a, &serde_json::to_value(&doc)?)?;
            let next = mochi_core::ai::tools::structured_manage::base_batch(&doc, operations(a)?)?;
            let content = mochi_core::base::serialize_base_document(&next)?;
            ensure!(
                std::fs::read_to_string(&path)? == original,
                "文件已被其他进程修改"
            );
            mochi_core::files::FileService::new().write_file_safe(&path, &content)?;
            if let Some(viewer::Content::Base(s)) = self.viewer_content_for(&path) {
                *s = crate::ui::base_view::State::parse(content)?;
            }
            return Ok(versioned(serde_json::to_value(next)?));
        }
        if name == "comments_manage" {
            self.console_path(&sidecars::comment_sidecar_path(&path_str), action != "list")?;
            let sidecar = PathBuf::from(sidecars::comment_sidecar_path(&path_str));
            let original = if sidecar.exists() {
                Some(std::fs::read_to_string(&sidecar)?)
            } else {
                None
            };
            let mut comments = if let Some(raw) = &original {
                let file: sidecars::CommentFile =
                    serde_json::from_str(raw).context("评论侧文件损坏，已保留原文件")?;
                ensure!(file.version == 1, "评论版本不受支持");
                file.comments
            } else {
                vec![]
            };
            if action == "list" {
                return Ok(versioned(json!(comments)));
            }
            check_revision(a, &json!(comments))?;
            match action {
                "create" | "reply" => {
                    let parent = if action == "reply" {
                        let id = text(a, "id")?;
                        ensure!(comments.iter().any(|c| c.id == id), "父评论不存在");
                        Some(id.into())
                    } else {
                        None
                    };
                    comments.push(sidecars::DocumentComment {
                        resolved: false,
                        id: mochi_core::paths::random_base36(16),
                        parent_id: parent,
                        target_type: "document".into(),
                        author: d["author"].as_str().unwrap_or("Agent").into(),
                        content: text(d, "content")?.into(),
                        created_at: mochi_core::jstime::now(),
                        updated_at: None,
                        anchor: d
                            .get("anchor")
                            .cloned()
                            .map(serde_json::from_value)
                            .transpose()?,
                        attachments: vec![],
                    });
                }
                "delete" => {
                    let id = text(a, "id")?;
                    ensure!(comments.iter().any(|c| c.id == id), "评论不存在");
                    let mut ids = std::collections::HashSet::from([id.to_owned()]);
                    loop {
                        let n = ids.len();
                        for c in &comments {
                            if c.parent_id.as_ref().is_some_and(|p| ids.contains(p)) {
                                ids.insert(c.id.clone());
                            }
                        }
                        if ids.len() == n {
                            break;
                        }
                    }
                    comments.retain(|c| !ids.contains(&c.id));
                }
                "update" | "resolve" | "reopen" => {
                    let c = comments
                        .iter_mut()
                        .find(|c| Some(c.id.as_str()) == a["id"].as_str())
                        .context("评论不存在")?;
                    if action == "update" {
                        c.content = text(d, "content")?.into();
                    } else {
                        c.resolved = action == "resolve";
                    }
                    c.updated_at = Some(mochi_core::jstime::now());
                }
                _ => bail!("未知评论操作"),
            }
            ensure!(
                std::fs::read_to_string(&sidecar).ok() == original,
                "评论文件已变化，请重读"
            );
            sidecars::save_comments(&path_str, comments.clone())?;
            self.refresh_right_panel();
            return Ok(versioned(json!(comments)));
        }
        if name == "annotations_manage" {
            self.console_path(
                &sidecars::annotation_sidecar_path(&path_str),
                !matches!(action, "list" | "locate"),
            )?;
            ensure!(
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("pdf")),
                "需要 PDF 文件"
            );
            let sidecar = PathBuf::from(sidecars::annotation_sidecar_path(&path_str));
            let source = if sidecar.exists() {
                sidecar.clone()
            } else {
                PathBuf::from(format!("{path_str}.annotations.json"))
            };
            self.console_path(&source.to_string_lossy(), false)?;
            let original = if source.exists() {
                Some(std::fs::read_to_string(&source)?)
            } else {
                None
            };
            let mut items = sidecars::load_pdf_annotations(&path_str).annotations;
            if let Some(raw) = &original {
                let parsed: Value =
                    serde_json::from_str(raw).context("批注侧文件损坏，已保留原文件")?;
                ensure!(
                    parsed["annotations"]
                        .as_array()
                        .is_some_and(|a| a.len() == items.len()),
                    "批注含无效条目，已保留原文件"
                );
            }
            if action == "list" {
                return Ok(versioned(json!(items)));
            }
            if action == "locate" {
                ensure!(self.shell.open_file_with_mode(&path, true), "无法打开 PDF");
                self.state.view = WorkspaceView::Editor;
                self.sync_state();
                ensure!(
                    matches!(self.viewer_content_mut(),Some(viewer::Content::Pdf(s)) if s.page_count()>0),
                    "PDF 正在加载，请稍后重试"
                );
                if let Some(id) = a["id"].as_str() {
                    ensure!(items.iter().any(|i| i.id == id), "批注不存在");
                    self.pdf_locate_annotation(id);
                } else {
                    let page = d["page"].as_u64().context("缺少 page（从 1 开始）")?;
                    let body = self.viewer_layout.body;
                    let s = match self.viewer_content_mut() {
                        Some(viewer::Content::Pdf(s)) => s,
                        _ => bail!("PDF 尚未加载"),
                    };
                    ensure!(page > 0 && (page as usize) <= s.page_count(), "页码越界");
                    let rects = viewer::pdf_page_rects(body, s);
                    s.scroll =
                        (rects[page as usize - 1].top - viewer::PDF_LABEL_H - body.top).max(0.0);
                }
                return Ok(json!({"located":true,"path":path}));
            }
            check_revision(a, &json!(items))?;
            match action {
                "create" => {
                    let mut v = d.clone();
                    v["id"] = json!(mochi_core::paths::random_base36(16));
                    v["createdAt"] = json!(mochi_core::jstime::now_millis());
                    v["updatedAt"] = v["createdAt"].clone();
                    items.push(serde_json::from_value(v)?);
                }
                "update" => {
                    let item = items
                        .iter_mut()
                        .find(|i| Some(i.id.as_str()) == a["id"].as_str())
                        .context("批注不存在")?;
                    let mut value = serde_json::to_value(&*item)?;
                    for (k, v) in d.as_object().context("data 必须是对象")? {
                        ensure!(
                            !matches!(k.as_str(), "id" | "createdAt"),
                            "不可修改 ID 或创建时间"
                        );
                        ensure!(value.get(k).is_some() || k == "text", "未知字段");
                        value[k] = v.clone();
                    }
                    value["updatedAt"] = json!(mochi_core::jstime::now_millis());
                    *item = serde_json::from_value(value)?;
                }
                "delete" => {
                    let id = text(a, "id")?;
                    ensure!(items.iter().any(|i| i.id == id), "批注不存在");
                    items.retain(|i| i.id != id);
                }
                _ => bail!("未知批注操作"),
            }
            for i in &items {
                ensure!(
                    i.page >= 1
                        && [i.x, i.y, i.width, i.height]
                            .iter()
                            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                        && i.width >= 0.0
                        && i.height >= 0.0
                        && matches!(i.kind.as_str(), "rect" | "circle" | "text"),
                    "批注位置或类型无效"
                );
            }
            ensure!(
                std::fs::read_to_string(&source).ok() == original,
                "批注文件已变化，请重读"
            );
            ensure!(
                self.pdf_save_annotations(&path, items.clone()),
                "批注保存失败"
            );
            return Ok(versioned(json!(items)));
        }
        bail!("未知结构化操作")
    }
}
