//! 筛选并生成控制台最近内容的展示数据。
use super::*;
impl App {
    pub(in crate::app) fn console_filter_recent(&mut self) {
        let Ok(root) = self.console_root() else {
            return;
        };
        let hidden: std::collections::BTreeMap<String, i64> =
            std::fs::read_to_string(root.join(".mochi/recent-hidden.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        self.views.recent.docs.retain(|d| {
            hidden
                .get(&mochi_core::document_unread::key(&d.path))
                .is_none_or(|t| d.mtime_ms > *t)
        });
    }
    pub(super) fn console_content(
        &mut self,
        name: &str,
        action: &str,
        a: &Value,
    ) -> CResult<Value> {
        let root = self.console_root()?;
        let d = data(a);
        let writing = !mochi_core::ai::tools::console_tools::read_only(name, action);
        let scope = match name {
            "inbox_manage" => "收件箱",
            "templates_manage" => ".mochi/templates",
            "favorites_manage" => ".mochi/favorites.json",
            "unread_manage" => ".mochi/document-unread.sqlite3",
            "recent_manage" => ".mochi/recent-hidden.json",
            _ => ".",
        };
        self.console_scope(scope, writing)?;
        if matches!(action, "delete" | "clear") {
            self.ai
                .permissions
                .as_ref()
                .context("权限不可用")?
                .assert_tool_action_allowed(
                    mochi_core::ai::permission::AiToolAction::DeleteFile,
                    Some(&root.join(scope).to_string_lossy()),
                )
                .map_err(|e| anyhow::anyhow!(e.message))?;
        }
        match name {
            "inbox_manage" => {
                if action == "batch" {
                    let ops = operations(a)?;
                    ensure!(
                        ops.iter().all(|o| matches!(
                            o["action"].as_str(),
                            Some("create" | "update" | "delete" | "archive" | "to_task")
                        )),
                        "批次包含未知操作"
                    );
                    let results = ops
                        .iter()
                        .enumerate()
                        .map(|(i, o)| {
                            match self.console_content(name, o["action"].as_str().unwrap(), o) {
                                Ok(value) => json!({"index":i,"ok":true,"value":value}),
                                Err(e) => json!({"index":i,"ok":false,"error":format!("{e:#}")}),
                            }
                        })
                        .collect::<Vec<_>>();
                    return Ok(json!({"atomic":false,"results":results}));
                }
                let svc = mochi_core::capture::CaptureService::new(&root);
                let value = match action {
                    "list" => json!(svc.list_items(d["status"].as_str().unwrap_or("all"))),
                    "create" => json!(svc
                        .add(text(d, "content")?, d["source"].as_str().unwrap_or("agent"))?
                        .context("内容为空")?),
                    "update" => {
                        if let Some(s) = d["status"].as_str() {
                            ensure!(
                                matches!(s, "inbox" | "archived"),
                                "status 只支持 inbox/archived"
                            );
                        }
                        json!(svc
                            .update(
                                text(a, "id")?,
                                d["content"].as_str(),
                                d["status"].as_str(),
                                None
                            )?
                            .context("条目不存在")?)
                    }
                    "delete" => json!({"removed":svc.remove(text(a,"id")?)?}),
                    "archive" => {
                        let dest = self.console_target(a, true)?;
                        json!({"path":svc.archive_to_note(text(a,"id")?,&dest)?})
                    }
                    "to_task" => {
                        self.console_path("agenda", true)?;
                        json!(svc.convert_to_task(
                            text(a, "id")?,
                            &mochi_core::agenda::AgendaStore::new(&root)
                        )?)
                    }
                    _ => bail!("未知收集箱操作"),
                };
                self.reload_home_dashboard();
                Ok(value)
            }
            "favorites_manage" => {
                let mut svc = mochi_core::favorites::Favorites::load(&root)?;
                match action {
                    "list" => {}
                    "add" | "remove" => {
                        let p = self.console_target(a, false)?;
                        if svc.contains(&p) != (action == "add") {
                            svc.toggle(&p)?;
                        }
                    }
                    "reorder" => {
                        check_revision(a, &json!(svc.paths()))?;
                        let paths = d["paths"]
                            .as_array()
                            .context("缺少 paths")?
                            .iter()
                            .map(|v| {
                                self.console_path(v.as_str().context("路径必须是字符串")?, false)
                            })
                            .collect::<CResult<Vec<_>>>()?;
                        svc.reorder(&paths)?;
                    }
                    _ => bail!("未知收藏操作"),
                }
                self.shell.reload_favorites();
                self.reload_home_dashboard();
                Ok(versioned(json!(svc.paths())))
            }
            "recent_manage" => {
                self.reload_recent();
                if writing {
                    let path = root.join(".mochi/recent-hidden.json");
                    let mut hidden: std::collections::BTreeMap<String, i64> = if path.exists() {
                        serde_json::from_str(&std::fs::read_to_string(&path)?)?
                    } else {
                        Default::default()
                    };
                    let target = if action == "remove" {
                        Some(self.console_target(a, false)?)
                    } else {
                        None
                    };
                    ensure!(action == "clear" || target.is_some(), "未知最近记录操作");
                    for doc in &self.views.recent.docs {
                        if target.as_ref().is_none_or(|p| p == &doc.path) {
                            hidden
                                .insert(mochi_core::document_unread::key(&doc.path), doc.mtime_ms);
                        }
                    }
                    mochi_core::files::FileService::new()
                        .write_file_safe(&path, &serde_json::to_string(&hidden)?)?;
                    self.console_filter_recent();
                }
                Ok(
                    json!({"documents":self.views.recent.docs.iter().map(|d|json!({"path":d.path,"title":d.title,"modifiedAt":d.mtime_ms})).collect::<Vec<_>>()}),
                )
            }
            "unread_manage" => {
                let svc = mochi_core::document_unread::Store::new(&root);
                if action == "mark" {
                    let paths = d["paths"]
                        .as_array()
                        .context("缺少 paths")?
                        .iter()
                        .map(|v| self.console_path(v.as_str().context("路径必须是字符串")?, false))
                        .collect::<CResult<Vec<_>>>()?;
                    svc.mark(&paths)?;
                }
                if action == "read" {
                    let p = self.console_target(a, false)?;
                    ensure!(
                        svc.read(&p, text(d, "token")?)?,
                        "已读版本已变化，请重新读取"
                    );
                }
                self.refresh_document_unread();
                Ok(json!(svc.snapshot()?.documents))
            }
            "templates_manage" => {
                let svc = mochi_core::templates::TemplateService::new(&root);
                let templates = svc.list()?;
                let target = || {
                    templates
                        .iter()
                        .find(|t| {
                            a["path"]
                                .as_str()
                                .is_some_and(|p| t.path == root.join(p) || t.path == Path::new(p))
                        })
                        .context("模板不存在；请用 list 返回的 path")
                };
                match action {
                    "list" => {}
                    "get" => {
                        let t = target()?;
                        return Ok(
                            json!({"path":t.path,"content":svc.read(t)?,"revision":revision(&json!(svc.read(t)?))}),
                        );
                    }
                    "create" => {
                        svc.create_in_group(
                            text(d, "name")?,
                            text(d, "content")?,
                            d["group"]
                                .as_str()
                                .unwrap_or(mochi_core::templates::UNGROUPED),
                        )?;
                    }
                    "update" => {
                        let t = target()?;
                        check_revision(a, &json!(svc.read(t)?))?;
                        self.console_path(&t.path.to_string_lossy(), true)?;
                        mochi_core::files::FileService::new()
                            .write_file_safe(&t.path, text(d, "content")?)?;
                    }
                    "delete" => svc.delete(target()?)?,
                    "instantiate" => {
                        let parent = self.console_path(text(d, "parent")?, true)?;
                        let path = svc.instantiate(target()?, &parent)?;
                        self.shell.refresh_tree();
                        return Ok(json!({"path":path}));
                    }
                    "group_create" => svc.create_group(text(d, "name")?)?,
                    "group_rename" => svc.rename_group(text(d, "name")?, text(d, "newName")?)?,
                    "group_delete" => svc.delete_group(text(d, "name")?)?,
                    "move" => {
                        svc.move_to_group(target()?, text(d, "group")?)?;
                    }
                    _ => bail!("未知模板操作"),
                }
                self.reload_templates();
                Ok(
                    json!({"groups":svc.groups()?,"templates":svc.list()?.iter().map(|t|json!({"name":t.name,"group":t.group,"path":t.path,"revision":revision(&json!(svc.read(t).unwrap_or_default()))})).collect::<Vec<_>>()}),
                )
            }
            "notifications_manage" => {
                let h = &mut self.notifications.view.history;
                match action {
                    "list" => {}
                    "read" => {
                        if let Some(id) = a["id"].as_str() {
                            h.mark_read(id.parse()?);
                        } else {
                            h.mark_all_read();
                        }
                    }
                    "delete" => {
                        let id = text(a, "id")?.parse::<u64>()?;
                        h.entries.retain(|n| n.id != id);
                    }
                    "clear" => h.entries.clear(),
                    _ => bail!("未知通知操作"),
                }
                if writing {
                    self.save_notifications();
                    ensure!(!self.notifications.view.save_failed, "通知保存失败");
                }
                Ok(
                    json!({"entries":self.notifications.view.history.entries,"unread":self.notifications.view.history.unread()}),
                )
            }
            "home_stats" => Ok(serde_json::to_value(mochi_core::analytics::build(
                &root,
                d["days"].as_f64(),
                chrono::Local::now(),
            ))?),
            _ => bail!("未知内容工具"),
        }
    }
}
