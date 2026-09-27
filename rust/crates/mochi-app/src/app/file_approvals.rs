//! 文件提案必须与当前缓冲区和磁盘基准相符；先展示完整变更，再按用户批准执行。
use super::*;
use mochi_core::ai::agent_inbox::InboxEntry;

fn block_proposal(
    entry: &InboxEntry,
) -> std::result::Result<mochi_core::ai::tools::host::BlockEditProposal, String> {
    let value = entry.to_value();
    let edit: mochi_core::ai::tools::host::BlockEditProposal =
        serde_json::from_value(value["operation"]["blockEdit"].clone())
            .map_err(|e| format!("块提案无效：{e}"))?;
    if edit.path != entry.path() {
        return Err("块提案路径不一致".into());
    }
    if mochi_blocks::model::hash_content(&edit.old_text) != edit.original_hash {
        return Err("块提案原文哈希无效".into());
    }
    Ok(edit)
}

fn block_writeback_source(
    entry: &InboxEntry,
    current: &str,
) -> std::result::Result<String, String> {
    let edit = block_proposal(entry)?;
    let document =
        mochi_core::document_blocks::document_from_source(Path::new(entry.path()), current)
            .map_err(|e| e.to_string())?;
    if !document.has_persisted_ids() {
        return Err("文档块标识已变化，请重新生成提案".into());
    }
    mochi_core::document_blocks::replace_block(
        &document,
        &edit.block_id,
        &edit.original_hash,
        &edit.new_text,
    )
    .map(|next| next.source().to_owned())
    .map_err(|e| format!("块内容或结构已变化，未覆盖任何内容：{e}"))
}

fn approval_path_identity(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized = resolved.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn same_approval_paths(left: &Path, right: &Path) -> bool {
    approval_path_identity(left) == approval_path_identity(right)
}

fn same_approval_path(left: &str, right: &str) -> bool {
    same_approval_paths(Path::new(left), Path::new(right))
}

/// 只有目标文件不存在时，`file_write` 才会用 `kind: write` 且不带
/// `previousContent`。与 `overwrite` 区分开，让缺少基准的旧版覆盖提案
/// 保持原有行为。
fn is_new_file_proposal(entry: &InboxEntry) -> bool {
    entry.kind() == "write" && entry.previous_content().is_none()
}

fn new_file_target_is_present(
    entries: &[InboxEntry],
    current: Option<&str>,
    disk: Option<&[u8]>,
) -> bool {
    entries.iter().any(is_new_file_proposal) && (current.is_some() || disk.is_some())
}

fn new_file_target_error() -> String {
    "新建文件提案目标已出现或未保存内容发生变化，请重新生成提案。".into()
}

fn approval_payload(entry: &InboxEntry) -> serde_json::Value {
    let mut value = entry.to_value();
    if let Some(operation) = value
        .get_mut("operation")
        .and_then(serde_json::Value::as_object_mut)
    {
        operation.remove("status");
        operation.remove("error");
        operation.remove("approvalClaimToken");
        operation.remove("approvalClaimedAt");
    }
    value
}

fn claimed_proposals_match(expected: &[InboxEntry], claimed: &[InboxEntry]) -> bool {
    expected.len() == claimed.len()
        && expected.iter().all(|entry| {
            claimed.iter().any(|candidate| {
                candidate.id() == entry.id()
                    && approval_payload(candidate) == approval_payload(entry)
            })
        })
}

fn approval_disk_snapshot(path: &Path) -> std::result::Result<Option<Vec<u8>>, String> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_dir() {
        return std::fs::read(path)
            .map(Some)
            .map_err(|error| format!("无法读取审批目标：{error}"));
    }

    fn visit(
        root: &Path,
        directory: &Path,
        hasher: &mut std::collections::hash_map::DefaultHasher,
    ) -> std::result::Result<(), String> {
        use std::hash::Hash;

        let mut entries = std::fs::read_dir(directory)
            .map_err(|error| format!("无法读取待操作目录 {}：{error}", directory.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| format!("无法枚举待操作目录 {}：{error}", directory.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap_or(path.as_path());
            relative.to_string_lossy().replace('\\', "/").hash(hasher);
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("无法读取目录项 {}：{error}", path.display()))?;
            metadata.len().hash(hasher);
            metadata.permissions().readonly().hash(hasher);
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .hash(hasher);
            let kind = metadata.file_type();
            if kind.is_symlink() {
                2_u8.hash(hasher);
                std::fs::read_link(&path)
                    .map_err(|error| format!("无法读取目录链接 {}：{error}", path.display()))?
                    .hash(hasher);
            } else if kind.is_dir() {
                1_u8.hash(hasher);
                visit(root, &path, hasher)?;
            } else if kind.is_file() {
                0_u8.hash(hasher);
                std::fs::read(&path)
                    .map_err(|error| format!("无法读取目录文件 {}：{error}", path.display()))?
                    .hash(hasher);
            } else {
                3_u8.hash(hasher);
            }
        }
        Ok(())
    }

    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    visit(path, path, &mut hasher)?;
    Ok(Some(hasher.finish().to_le_bytes().to_vec()))
}

fn base_schema_without_records(
    mut document: mochi_core::base::BaseDocument,
) -> mochi_core::base::BaseDocument {
    for table in &mut document.tables {
        table.records.clear();
    }
    document
}

/// 合并同一份已保存基准生成的多个独立 `.mcb` 提案。基础工具目前提交的是
/// 整文件覆盖，按顺序逐个应用会让第一个之后的所有条目全部过期。合并基于
/// 稳定的表/记录/字段 id 进行，遇到改表结构、删除或值重叠时直接报错，
/// 而不是挑一个赢家。
fn merge_base_proposals(
    entries: &[InboxEntry],
    current: &str,
) -> std::result::Result<String, String> {
    let baseline = entries
        .first()
        .and_then(InboxEntry::previous_content)
        .ok_or("多维表格提案缺少原始版本")?;
    if current != baseline {
        return Err("多维表格在提案生成后发生了变化，请重新生成提案".into());
    }
    if entries
        .iter()
        .any(|entry| entry.previous_content() != Some(baseline))
    {
        return Err("同一文档的审批基线不一致，不能合并".into());
    }
    let base = mochi_core::base::parse_base_document(baseline)
        .map_err(|error| format!("多维表格原始版本无效：{error}"))?;
    let schema = base_schema_without_records(base.clone());
    let mut merged = base.clone();

    for entry in entries {
        let proposed = mochi_core::base::parse_base_document(
            entry.content().ok_or("多维表格提案缺少修改内容")?,
        )
        .map_err(|error| format!("多维表格提案无效：{error}"))?;
        if base_schema_without_records(proposed.clone()) != schema {
            return Err("多维表格结构发生变化，不能把记录审批自动合并".into());
        }
        for proposed_table in &proposed.tables {
            let base_table = base
                .tables
                .iter()
                .find(|table| table.id == proposed_table.id)
                .ok_or("多维表格提案引用了未知数据表")?;
            let merged_table = merged
                .tables
                .iter_mut()
                .find(|table| table.id == proposed_table.id)
                .ok_or("多维表格合并状态缺少数据表")?;

            if base_table.records.iter().any(|record| {
                !proposed_table
                    .records
                    .iter()
                    .any(|item| item.id == record.id)
            }) {
                return Err("批量审批不自动合并删除记录的提案".into());
            }
            for proposed_record in &proposed_table.records {
                let original = base_table
                    .records
                    .iter()
                    .find(|record| record.id == proposed_record.id);
                let Some(original) = original else {
                    match merged_table
                        .records
                        .iter()
                        .find(|record| record.id == proposed_record.id)
                    {
                        None => merged_table.records.push(proposed_record.clone()),
                        Some(existing) if existing == proposed_record => {}
                        Some(_) => {
                            return Err(format!(
                                "记录 {} 被多个审批以不同内容新增，整组未写入",
                                proposed_record.id
                            ));
                        }
                    }
                    continue;
                };
                if proposed_record == original {
                    continue;
                }
                if proposed_record.extra != original.extra {
                    return Err(format!(
                        "记录 {} 的扩展结构发生变化，不能自动合并",
                        proposed_record.id
                    ));
                }
                let merged_record = merged_table
                    .records
                    .iter_mut()
                    .find(|record| record.id == proposed_record.id)
                    .ok_or("多维表格合并状态缺少原记录")?;
                let mut field_ids = original.values.keys().cloned().collect::<Vec<_>>();
                for id in proposed_record.values.keys() {
                    if !field_ids.iter().any(|existing| existing == id) {
                        field_ids.push(id.clone());
                    }
                }
                for field_id in field_ids {
                    let old = original.values.get(&field_id);
                    let next = proposed_record.values.get(&field_id);
                    if old == next {
                        continue;
                    }
                    let accumulated = merged_record.values.get(&field_id);
                    if accumulated != old && accumulated != next {
                        return Err(format!(
                            "记录 {} 的字段 {} 存在互相冲突的修改，整组未写入",
                            proposed_record.id, field_id
                        ));
                    }
                    match next {
                        Some(value) => {
                            merged_record.values.insert(field_id, value.clone());
                        }
                        None => {
                            merged_record.values.remove(&field_id);
                        }
                    }
                }
            }
        }
    }
    mochi_core::base::serialize_base_document(&merged)
        .map_err(|error| format!("多维表格合并结果无法保存：{error}"))
}

fn merge_block_proposals(
    entries: &[InboxEntry],
    current: &str,
) -> std::result::Result<String, String> {
    let mut merged = current.to_owned();
    for entry in entries {
        merged = block_writeback_source(entry, &merged)?;
    }
    Ok(merged)
}
impl App {
    pub(super) fn prepare_file_deletion(&mut self, path: &Path) -> bool {
        let valid = self
            .shell
            .workspace()
            .and_then(|w| w.root.canonicalize().ok())
            .zip(path.canonicalize().ok())
            .is_some_and(|(root, target)| {
                !mochi_core::paths::paths_equal(&root, &target)
                    && mochi_core::paths::path_is_within(&root, &target)
            });
        if !valid {
            self.state.status_text = "不能删除工作区外路径或工作区根目录".into();
            return false;
        }
        let indices = self
            .shell
            .tabs()
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.dirty() && t.path().is_some_and(|p| p == path || p.starts_with(path))
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        for i in indices {
            if !self.shell.save_tab(i) {
                self.state.status_text = "保存未完成，未删除文件".into();
                return false;
            }
        }
        self.ai_cancel();
        self.editor_ai.invalidate();
        true
    }
    fn validate_file_proposal(&self, entry: &InboxEntry) -> std::result::Result<(), String> {
        let host = self.ai.host.as_ref().ok_or("操作宿主不可用")?;
        let root = self
            .shell
            .workspace()
            .ok_or("工作区不可用")?
            .root
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let action = if matches!(entry.kind(), "delete-file" | "delete-folder") {
            mochi_core::ai::permission::AiToolAction::DeleteFile
        } else {
            mochi_core::ai::permission::AiToolAction::WriteFile
        };
        for raw in std::iter::once(entry.path()).chain(entry.new_path()) {
            let path = Path::new(raw);
            if !path.is_absolute() {
                return Err("提案路径必须为绝对路径".into());
            }
            mochi_core::ai::tools::host::resolve_workspace_path(
                host.as_ref(),
                Some(raw),
                Default::default(),
            )?;
            let mut ancestor = path;
            while !ancestor.exists() {
                ancestor = ancestor.parent().ok_or("路径无效")?;
            }
            let resolved = ancestor.canonicalize().map_err(|e| e.to_string())?;
            if !mochi_core::paths::path_is_within(&root, &resolved)
                || (path.exists() && mochi_core::paths::paths_equal(&resolved, &root))
            {
                return Err("不能操作工作区外路径或工作区根目录".into());
            }
            self.ai
                .permissions
                .as_ref()
                .ok_or("权限服务不可用")?
                .assert_tool_action_allowed(action, Some(raw))
                .map_err(|e| e.to_string())?;
        }
        if !matches!(
            entry.kind(),
            "write"
                | "overwrite"
                | "create-folder"
                | "block-edit"
                | "delete-file"
                | "delete-folder"
                | "rename"
        ) {
            return Err("不支持的提案类型".into());
        }
        if matches!(entry.kind(), "write" | "overwrite") && entry.content().is_none() {
            return Err("提案缺少修改后的内容".into());
        }
        if entry.kind() == "block-edit" {
            block_proposal(entry)?;
        }
        if matches!(entry.kind(), "write" | "overwrite")
            && Path::new(entry.path())
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mcb"))
        {
            mochi_core::base::parse_base_document(entry.content().unwrap())
                .map_err(|error| format!("多维表格校验失败：{error}"))?;
        }
        Ok(())
    }
    fn current_proposal_text(&self, path: &Path) -> Option<String> {
        self.shell
            .tabs()
            .iter()
            .find(|t| t.path().is_some_and(|open| same_approval_paths(open, path)))
            .and_then(|t| match &t.kind {
                TabKind::Viewer {
                    content: viewer::Content::Base(state),
                    ..
                } => {
                    if state.dirty {
                        mochi_core::base::serialize_base_document(&state.document).ok()
                    } else {
                        Some(state.saved_raw.clone())
                    }
                }
                _ => t.buffer().map(|buffer| buffer.text().to_owned()),
            })
            .or_else(|| std::fs::read_to_string(path).ok())
    }
    pub(super) fn review_file_proposal(&mut self, entry: InboxEntry) {
        self.review_file_proposals(vec![entry]);
    }

    pub(super) fn review_file_proposals(&mut self, entries: Vec<InboxEntry>) {
        let Some(first) = entries.first() else {
            return;
        };
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        let same_path = entries
            .iter()
            .all(|entry| same_approval_path(entry.path(), first.path()));
        let mut error = if same_path {
            entries
                .iter()
                .find_map(|entry| self.validate_file_proposal(entry).err())
        } else {
            Some("一次只能审批同一文档的修改".into())
        };
        let current = if error.is_none() {
            self.current_proposal_text(Path::new(first.path()))
        } else {
            None
        };
        let disk = if error.is_none() {
            match approval_disk_snapshot(Path::new(first.path())) {
                Ok(snapshot) => snapshot,
                Err(snapshot_error) => {
                    error = Some(snapshot_error);
                    None
                }
            }
        } else {
            None
        };
        let block_error = if entries.len() == 1 && first.kind() == "block-edit" && error.is_none() {
            current
                .as_deref()
                .ok_or_else(|| "文档不存在或不可读取".to_owned())
                .and_then(|source| block_writeback_source(first, source))
                .err()
                .or_else(|| {
                    self.shell
                        .tabs()
                        .iter()
                        .find(|t| {
                            t.path().is_some_and(|open| {
                                same_approval_paths(open, Path::new(first.path()))
                            })
                        })
                        .and_then(|t| t.buffer())
                        .and_then(|buffer| {
                            let disk_text = disk
                                .as_ref()
                                .and_then(|bytes| std::str::from_utf8(bytes).ok());
                            (!disk_text.is_some_and(|text| buffer.matches_saved(text)))
                                .then(|| "磁盘文档已有变化，请先处理保存冲突".to_owned())
                        })
                })
        } else {
            None
        };
        let conflict = entries.len() == 1
            && first.kind() != "block-edit"
            && first
                .previous_content()
                .is_some_and(|previous| Some(previous) != current.as_deref());
        let create_target_present =
            new_file_target_is_present(&entries, current.as_deref(), disk.as_deref());
        let binary = entries.len() == 1
            && matches!(first.kind(), "write" | "overwrite")
            && disk.is_some()
            && current.is_none();
        let grouped = if entries.len() > 1 && error.is_none() {
            let source = current
                .as_deref()
                .ok_or_else(|| "文档不存在或不可读取".to_owned());
            source.and_then(|source| {
                let kinds_are_blocks = entries.iter().all(|entry| entry.kind() == "block-edit");
                let kinds_are_writes = entries
                    .iter()
                    .all(|entry| matches!(entry.kind(), "write" | "overwrite"));
                if kinds_are_blocks {
                    let dirty = self
                        .shell
                        .tabs()
                        .iter()
                        .find(|tab| {
                            tab.path().is_some_and(|open| {
                                same_approval_paths(open, Path::new(first.path()))
                            })
                        })
                        .and_then(|tab| tab.buffer())
                        .is_some_and(|buffer| {
                            let disk_text = disk
                                .as_ref()
                                .and_then(|bytes| std::str::from_utf8(bytes).ok());
                            !disk_text.is_some_and(|text| buffer.matches_saved(text))
                        });
                    if dirty {
                        Err("磁盘文档已有变化，请先处理保存冲突".into())
                    } else {
                        merge_block_proposals(&entries, source)
                    }
                } else if kinds_are_writes && first.path().to_ascii_lowercase().ends_with(".mcb") {
                    merge_base_proposals(&entries, source)
                } else {
                    Err("多条整篇文本修改无法安全自动合并；请改用块修改或逐条处理".into())
                }
            })
        } else {
            Ok(String::new())
        };
        let grouped_error = grouped.as_ref().err().cloned();
        let mut review = crate::ui::command_review::State::files(entries, current, disk);
        if review.files.len() > 1 {
            review.proposed_text = grouped.ok();
        }
        review.error = error.or(block_error).or(grouped_error).unwrap_or_else(|| {
            if create_target_present {
                new_file_target_error()
            } else if conflict {
                "文档在提案生成后发生了变化，请重新生成提案。".into()
            } else if binary {
                "不能用文本提案覆盖非 UTF-8 文件。".into()
            } else {
                String::new()
            }
        });
        self.commands.review = Some(review);
        self.commands.pressed = None;
        self.drag = None;
    }

    /// 收件箱紧凑行上的对勾本身就是用户的批准。先对整个文档组做预检，
    /// 安全则立即执行；有冲突时保持详情对话框打开，并给出具体原因。
    pub(super) fn approve_file_proposals(&mut self, entries: Vec<InboxEntry>) -> bool {
        self.review_file_proposals(entries);
        let Some(review) = self.commands.review.as_ref() else {
            return false;
        };
        if !review.error.is_empty() {
            return false;
        }
        self.apply_reviewed_file();
        self.commands.review.is_none()
    }

    pub(super) fn inbox_approval_group(&self, group_index: usize) -> Vec<InboxEntry> {
        let groups = inbox::groups(&self.panels.inbox);
        groups
            .get(group_index)
            .map(|group| {
                group
                    .indices
                    .iter()
                    .filter_map(|index| self.panels.inbox.entries.get(*index).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn open_inbox_approval_group(&mut self, group_index: usize) {
        let entries = self.inbox_approval_group(group_index);
        let Some(first) = entries.first() else {
            return;
        };
        match first.kind() {
            "native-shell-command" => self.review_command(first.id()),
            "native-document-export" => self.review_document_export(first.id()),
            _ => self.review_file_proposals(entries),
        }
    }

    pub(super) fn approve_inbox_approval_group(&mut self, group_index: usize) {
        let entries = self.inbox_approval_group(group_index);
        let Some(first) = entries.first() else {
            return;
        };
        if entries
            .iter()
            .all(|entry| matches!(entry.kind(), "write" | "overwrite" | "block-edit"))
        {
            if self.approve_file_proposals(entries) {
                let message = self.state.status_text.clone();
                self.show_global_notice(&message);
            }
            return;
        }
        match first.kind() {
            "native-shell-command" => self.review_command(first.id()),
            "native-document-export" => {
                self.approve_document_export(first.id());
            }
            _ => self.review_file_proposals(entries),
        }
    }

    pub(super) fn reject_inbox_approval_entries(
        &mut self,
        entries: &[InboxEntry],
    ) -> std::result::Result<usize, String> {
        let durable = entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.kind(),
                    "native-document-export" | "native-shell-command"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut rejected = 0;
        if !durable.is_empty() {
            let root = self
                .shell
                .workspace()
                .map(|workspace| workspace.root.clone())
                .ok_or("工作区不可用")?;
            let ids = durable
                .iter()
                .map(|entry| entry.id().to_owned())
                .collect::<Vec<_>>();
            let service = AgentInboxService::new(root);
            let claimed = service
                .claim_many_pending(&ids)
                .map_err(|error| error.to_string())?;
            let claim_token = claimed
                .first()
                .and_then(InboxEntry::claim_token)
                .map(str::to_owned);
            if !claimed_proposals_match(&durable, &claimed) {
                if let Some(claim_token) = claim_token.as_deref() {
                    let _ = service.release_many_claimed(&ids, claim_token, "pending", None);
                }
                return Err("审批已被处理或内容发生变化；未拒绝任何文档修改".into());
            }
            let Some(claim_token) = claim_token else {
                return Err("审批领取缺少所有权标识；未拒绝任何文档修改".into());
            };
            let resolved = service
                .release_many_claimed(&ids, &claim_token, "rejected", None)
                .map_err(|error| error.to_string())?;
            if resolved != ids.len() {
                return Err(format!(
                    "只拒绝了 {resolved}/{} 条文档审批，请刷新后重试",
                    ids.len()
                ));
            }
            rejected += resolved;
            for entry in &durable {
                self.ai_set_pending_edit_status(entry.id(), "rejected", None);
            }
        }

        let queue = self.ai.export_requests.clone();
        for entry in entries.iter().filter(|entry| {
            matches!(
                entry.kind(),
                "native-document-export" | "native-shell-command"
            )
        }) {
            let request = queue.as_ref().and_then(|queue| {
                queue
                    .pending()
                    .into_iter()
                    .find(|request| request.id == entry.id())
            });
            let Some(queue) = queue.as_ref() else {
                return Err("审批队列不可用".into());
            };
            queue
                .reject_pending(entry.id())
                .map_err(|error| error.to_string())?;
            if entry.kind() == "native-shell-command" {
                self.ai_reject_shell_card(
                    entry.id(),
                    request
                        .as_ref()
                        .and_then(|request| request.session_id.as_deref()),
                );
            }
            rejected += 1;
        }
        Ok(rejected)
    }

    pub(super) fn reject_inbox_approval_group(&mut self, group_index: usize) {
        let entries = self.inbox_approval_group(group_index);
        match self.reject_inbox_approval_entries(&entries) {
            Ok(count) => {
                let message = format!("已拒绝 {count} 项审批");
                self.state.status_text = message.clone();
                self.show_global_notice(&message);
            }
            Err(error) => self.state.status_text = error,
        }
    }

    pub(super) fn approve_all_document_approvals(&mut self) {
        let groups = inbox::groups(&self.panels.inbox)
            .into_iter()
            .filter_map(|group| {
                let entries = group
                    .indices
                    .iter()
                    .filter_map(|index| self.panels.inbox.entries.get(*index).cloned())
                    .collect::<Vec<_>>();
                (!entries.is_empty()
                    && entries
                        .iter()
                        .all(|entry| matches!(entry.kind(), "write" | "overwrite" | "block-edit")))
                .then_some(entries)
            })
            .collect::<Vec<_>>();
        let skipped = self.panels.inbox.entries.len() - groups.iter().map(Vec::len).sum::<usize>();
        if groups.is_empty() {
            let message = if skipped > 0 {
                "没有可批量通过的文档修改；命令、导出和路径操作需逐项确认"
            } else {
                "没有待审批的文档修改"
            };
            self.state.status_text = message.into();
            self.show_global_notice(message);
            return;
        }
        let mut applied = 0;
        for entries in groups {
            let count = entries.len();
            if !self.approve_file_proposals(entries) {
                if applied > 0 {
                    let error = self.state.status_text.clone();
                    self.state.status_text =
                        format!("已通过 {applied} 项；后续文档未写入：{error}");
                }
                return;
            }
            applied += count;
        }
        self.state.status_text = if skipped > 0 {
            format!("已通过 {applied} 项文档修改；{skipped} 项命令、导出或路径操作需逐项确认")
        } else {
            format!("已通过全部 {applied} 项文档修改")
        };
        let message = self.state.status_text.clone();
        self.show_global_notice(&message);
    }

    pub(super) fn reject_all_inbox_approvals(&mut self) {
        let entries = self.panels.inbox.entries.clone();
        if entries.is_empty() {
            self.state.status_text = "没有待审批操作".into();
            self.show_global_notice("没有待审批操作");
            return;
        }
        match self.reject_inbox_approval_entries(&entries) {
            Ok(count) => {
                let message = format!("已拒绝全部 {count} 项审批");
                self.state.status_text = message.clone();
                self.show_global_notice(&message);
            }
            Err(error) => self.state.status_text = error,
        }
    }

    pub(super) fn apply_reviewed_file(&mut self) {
        let Some(review) = self.commands.review.as_ref() else {
            return;
        };
        if !review.error.is_empty() {
            return;
        }
        let Some(entry) = review.file.clone() else {
            return;
        };
        let entries = if review.files.is_empty() {
            vec![entry.clone()]
        } else {
            review.files.clone()
        };
        let base = review.base_text.clone();
        let disk = review.base_disk.clone();
        let proposed_text = review.proposed_text.clone();
        let grouped = entries.len() > 1;
        let root = match self.shell.workspace() {
            Some(workspace) => workspace.root.clone(),
            None => return,
        };
        let service = AgentInboxService::new(&root);
        let ids = entries
            .iter()
            .map(|entry| entry.id().to_owned())
            .collect::<Vec<_>>();
        let mut claimed = false;
        let mut claim_token = None::<String>;
        let apply = (|| -> std::result::Result<(), String> {
            let unique_ids = ids.iter().collect::<std::collections::HashSet<_>>();
            if unique_ids.len() != ids.len() {
                return Err("审批组包含重复标识，未写入任何内容".into());
            }
            for entry in &entries {
                self.validate_file_proposal(entry)?;
            }
            let path = Path::new(entry.path());
            let current_disk = approval_disk_snapshot(path)?;
            let current_text = self.current_proposal_text(path);
            if new_file_target_is_present(
                &entries,
                current_text.as_deref(),
                current_disk.as_deref(),
            ) {
                return Err(new_file_target_error());
            }
            if ((grouped || entry.kind() != "block-edit") && current_text != base)
                || current_disk != disk
            {
                return Err("批准前文档又有变化，未覆盖任何内容；请重新生成提案。".into());
            }

            let claimed_entries = service
                .claim_many_pending(&ids)
                .map_err(|error| format!("无法领取审批：{error}"))?;
            if claimed_entries.len() != ids.len() {
                return Err("审批已被其他窗口处理；未写入任何内容，请刷新列表。".into());
            }
            claimed = true;
            claim_token = claimed_entries
                .first()
                .and_then(InboxEntry::claim_token)
                .map(str::to_owned);
            if claim_token.is_none() {
                return Err("审批领取缺少所有权标识；未写入任何内容。".into());
            }
            if !claimed_proposals_match(&entries, &claimed_entries) {
                return Err("审批内容在打开详情后发生变化；未写入任何内容，请刷新列表。".into());
            }

            let current_disk = approval_disk_snapshot(path)?;
            let current_text = self.current_proposal_text(path);
            if new_file_target_is_present(
                &entries,
                current_text.as_deref(),
                current_disk.as_deref(),
            ) {
                return Err(new_file_target_error());
            }
            if ((grouped || entry.kind() != "block-edit") && current_text != base)
                || current_disk != disk
            {
                return Err("领取审批后文档又有变化，未覆盖任何内容；请重新生成提案。".into());
            }
            if grouped {
                let content = proposed_text.as_deref().ok_or("整组修改尚未通过预检")?;
                mochi_core::files::FileService::new()
                    .write_file_safe(path, content)
                    .map_err(|e| e.to_string())?;
                self.shell.accept_written_text(path, content);
                return Ok(());
            }
            match entry.kind() {
                "block-edit" => {
                    let current = self.current_proposal_text(path).ok_or("无法读取当前文档")?;
                    let content = block_writeback_source(&entry, &current)?;
                    mochi_core::files::FileService::new()
                        .write_file_safe(path, &content)
                        .map_err(|e| e.to_string())?;
                    self.shell.accept_written_text(path, &content);
                }
                "write" | "overwrite" => {
                    let content = entry.content().ok_or("提案内容缺失")?;
                    mochi_core::files::FileService::new()
                        .write_file_safe(path, content)
                        .map_err(|e| e.to_string())?;
                    self.shell.accept_written_text(path, content);
                }
                "create-folder" => {
                    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
                }
                "rename" => {
                    let to = Path::new(entry.new_path().ok_or("目标路径缺失")?);
                    self.shell.move_path(path, to).map_err(|e| e.to_string())?;
                }
                "delete-file" | "delete-folder" => {
                    let root = &self.shell.workspace().unwrap().root;
                    AgentInboxService::new(root).apply(&entry)?;
                    self.shell.forget_tabs_under(path);
                }
                _ => return Err("提案类型无效".into()),
            }
            Ok(())
        })();
        match apply {
            Ok(()) => {
                let result = claim_token
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("审批领取缺少所有权标识"))
                    .and_then(|token| service.release_many_claimed(&ids, token, "applied", None))
                    .and_then(|count| {
                        if count == ids.len() {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("只更新了 {count}/{} 条审批记录", ids.len()))
                        }
                    });
                match result {
                    Ok(()) => {
                        for entry in &entries {
                            self.ai_set_pending_edit_status(entry.id(), "applied", None);
                        }
                        self.commands.review = None;
                        self.state.status_text = if grouped {
                            format!("已合并并应用 {} 项文档修改", entries.len())
                        } else {
                            "已应用文件修改".into()
                        };
                    }
                    Err(error) => {
                        let message =
                            format!("文档修改已写入，但审批记录收尾失败：{error}；已禁止重复执行");
                        for entry in &entries {
                            self.ai_set_pending_edit_status(entry.id(), "error", Some(&message));
                        }
                        self.state.status_text = message.clone();
                        if let Some(review) = self.commands.review.as_mut() {
                            review.error = message;
                        }
                    }
                }
                self.shell.refresh_tree();
                self.invalidate_main();
                self.sync_state();
                self.refresh_right_panel();
            }
            Err(mut error) => {
                if claimed {
                    let release = claim_token
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("审批领取缺少所有权标识"))
                        .and_then(|token| {
                            service.release_many_claimed(&ids, token, "pending", None)
                        });
                    match release {
                        Ok(count) if count == ids.len() => {}
                        Ok(count) => error.push_str(&format!(
                            "；只恢复了 {count}/{} 条审批，请刷新列表",
                            ids.len()
                        )),
                        Err(release_error) => {
                            error.push_str(&format!("；恢复审批状态失败：{release_error}"))
                        }
                    }
                }
                let status = if error.contains("变化") {
                    "conflict"
                } else {
                    "error"
                };
                for entry in &entries {
                    self.ai_set_pending_edit_status(entry.id(), status, Some(&error));
                }
                self.state.status_text = error.clone();
                if let Some(review) = self.commands.review.as_mut() {
                    review.error = error;
                }
            }
        }
    }
}

#[cfg(test)]
mod base_tests {
    use super::*;
    #[test]
    fn real_block_host_persists_proposals_and_native_review_applies_or_rejects() {
        use crate::ui::command_review::Hit;
        use mochi_core::ai::tools::host::{BlockEditProposal, ToolHost};
        let root = std::env::temp_dir().join(format!(
            "mochi-block-approval-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            let path = root.join("note.md");
            std::fs::write(&path, "第一块\n\n第二块\n").unwrap();
            assert!(app.shell.open_file(&path));
            let document = app.ensure_active_document_blocks().unwrap();
            let source = document.source().to_owned();
            {
                let mut snapshot = app.ai.snapshot.lock().unwrap();
                snapshot.edit_apply_mode = "auto".into();
                snapshot.active_document = Some(ActiveDocument {
                    id: "note".into(),
                    title: "note".into(),
                    path: path.to_string_lossy().into_owned(),
                    is_dirty: false,
                });
                snapshot.active_text = Some(source.clone());
            }
            let block = &document.blocks[0];
            let result = app
                .ai
                .host
                .as_ref()
                .unwrap()
                .propose_block_edit(BlockEditProposal {
                    path: path.to_string_lossy().into_owned(),
                    block_id: block.id.to_string(),
                    original_hash: block.content_hash(),
                    old_text: block.content.clone(),
                    new_text: "第一块改进\n".into(),
                    summary: "改进第一块".into(),
                })
                .unwrap();
            let id = result["inboxId"].as_str().unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                source,
                "automatic mode must not apply block proposals"
            );
            let entry = AgentInboxService::new(&root)
                .list_pending()
                .into_iter()
                .find(|entry| entry.id() == id)
                .unwrap();
            let buffer = app.shell.active_buffer_mut().unwrap();
            let at = buffer.text().find("第二块").unwrap();
            buffer.replace_range(at..at + "第二块".len(), "第二块本地编辑😀");
            app.review_file_proposal(entry.clone());
            assert!(app.commands.review.as_ref().unwrap().error.is_empty());
            app.apply_reviewed_file();
            assert!(app.commands.review.is_none(), "{}", app.state.status_text);
            let applied = std::fs::read_to_string(&path).unwrap();
            assert!(applied.contains("第一块改进"));
            assert!(applied.contains("第二块本地编辑😀"));
            assert!(!AgentInboxService::new(&root)
                .list_pending()
                .iter()
                .any(|entry| entry.id() == id));
            assert!(mochi_core::document_blocks::conversion_backup_path(&path).exists());
            // 再排入一个持久化提案，然后用与原生审查按钮相同的操作拒绝它。
            // 期间不允许发生任何文档写入。
            let mut operation = entry.to_value()["operation"].clone();
            operation["id"] = "reject-test".into();
            operation["status"] = "pending".into();
            let queued = AgentInboxService::new(&root)
                .add(operation, None, Some("test"))
                .unwrap();
            app.review_file_proposal(queued.clone());
            app.command_review_action(Hit::Reject);
            assert!(app.commands.review.is_none());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), applied);
            assert!(!AgentInboxService::new(&root)
                .list_pending()
                .iter()
                .any(|entry| entry.id() == queued.id()));
        }
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mochi-block-approval-"));
        std::fs::remove_dir_all(resolved).unwrap();
    }
    fn block_entry(source: &str) -> InboxEntry {
        let document =
            mochi_core::document_blocks::document_from_source(Path::new("note.md"), source)
                .unwrap();
        let block = &document.blocks[0];
        InboxEntry::from_value(&serde_json::json!({"id":"test-approval","operation":{"kind":"block-edit","path":"note.md","status":"pending","blockEdit":{
            "path":"note.md","blockId":block.id,"originalHash":block.content_hash(),"oldText":block.content,"newText":"修改后的第一块。\n","summary":"调整说明"
        }}})).unwrap()
    }

    fn block_entry_at(source: &str, index: usize, id: &str, new_text: &str) -> InboxEntry {
        let document =
            mochi_core::document_blocks::document_from_source(Path::new("note.md"), source)
                .unwrap();
        let block = &document.blocks[index];
        InboxEntry::from_value(&serde_json::json!({
            "id": id,
            "operation": {
                "kind": "block-edit",
                "path": "note.md",
                "status": "pending",
                "blockEdit": {
                    "path": "note.md",
                    "blockId": block.id.to_string(),
                    "originalHash": block.content_hash(),
                    "oldText": block.content,
                    "newText": new_text,
                    "summary": "调整说明"
                }
            }
        }))
        .unwrap()
    }

    fn base_record(id: &str, field_id: &str, value: &str) -> mochi_core::base::BaseRecord {
        let mut values = serde_json::Map::new();
        values.insert(
            field_id.to_owned(),
            serde_json::Value::String(value.to_owned()),
        );
        mochi_core::base::BaseRecord {
            id: id.into(),
            values,
            ..Default::default()
        }
    }

    fn base_entry(id: &str, baseline: &str, content: &str) -> InboxEntry {
        InboxEntry::from_value(&serde_json::json!({
            "id": id,
            "operation": {
                "kind": "overwrite",
                "path": "data.mcb",
                "status": "pending",
                "previousContent": baseline,
                "content": content
            }
        }))
        .unwrap()
    }

    #[test]
    fn directory_approval_snapshot_detects_content_and_tree_changes() {
        let root = std::env::temp_dir().join(format!(
            "mochi-directory-approval-snapshot-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("same-size.txt");
        std::fs::write(&file, b"before").unwrap();
        let before = approval_disk_snapshot(&root).unwrap();
        std::fs::write(&file, b"after!").unwrap();
        let content_changed = approval_disk_snapshot(&root).unwrap();
        assert_ne!(before, content_changed);
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/new.txt"), b"new").unwrap();
        assert_ne!(content_changed, approval_disk_snapshot(&root).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn approval_path_identity_matches_windows_case_and_slashes() {
        assert!(same_approval_path(
            r"D:\Mochi\Knowledge\Note.mcb",
            "d:/mochi/knowledge/note.mcb"
        ));
    }

    #[test]
    fn merge_base_proposals_keeps_independent_new_records() {
        let baseline_document = mochi_core::base::create_base_document();
        let field_id = baseline_document.tables[0].fields[0].id.clone();
        let baseline = mochi_core::base::serialize_base_document(&baseline_document).unwrap();

        let mut first_document = baseline_document.clone();
        first_document.tables[0]
            .records
            .push(base_record("record-a", &field_id, "甲"));
        let first = mochi_core::base::serialize_base_document(&first_document).unwrap();

        let mut second_document = baseline_document;
        second_document.tables[0]
            .records
            .push(base_record("record-b", &field_id, "乙"));
        let second = mochi_core::base::serialize_base_document(&second_document).unwrap();

        let merged = merge_base_proposals(
            &[
                base_entry("approval-a", &baseline, &first),
                base_entry("approval-b", &baseline, &second),
            ],
            &baseline,
        )
        .unwrap();
        let merged_document = mochi_core::base::parse_base_document(&merged).unwrap();
        let records = &merged_document.tables[0].records;
        assert_eq!(records.len(), 2);
        assert_eq!(
            records
                .iter()
                .find(|record| record.id == "record-a")
                .unwrap()
                .values[&field_id],
            serde_json::json!("甲")
        );
        assert_eq!(
            records
                .iter()
                .find(|record| record.id == "record-b")
                .unwrap()
                .values[&field_id],
            serde_json::json!("乙")
        );
    }

    #[test]
    fn merge_base_proposals_rejects_conflicting_record_updates() {
        let mut baseline_document = mochi_core::base::create_base_document();
        let field_id = baseline_document.tables[0].fields[0].id.clone();
        baseline_document.tables[0]
            .records
            .push(base_record("record-shared", &field_id, "原值"));
        let baseline = mochi_core::base::serialize_base_document(&baseline_document).unwrap();

        let mut first_document = baseline_document.clone();
        first_document.tables[0].records[0]
            .values
            .insert(field_id.clone(), serde_json::json!("甲"));
        let first = mochi_core::base::serialize_base_document(&first_document).unwrap();

        let mut second_document = baseline_document;
        second_document.tables[0].records[0]
            .values
            .insert(field_id, serde_json::json!("乙"));
        let second = mochi_core::base::serialize_base_document(&second_document).unwrap();

        let error = merge_base_proposals(
            &[
                base_entry("approval-a", &baseline, &first),
                base_entry("approval-b", &baseline, &second),
            ],
            &baseline,
        )
        .unwrap_err();
        assert!(error.contains("record-shared"), "{error}");
    }

    #[test]
    fn merge_base_proposals_rejects_mismatched_baselines_and_changed_current() {
        let baseline_document = mochi_core::base::create_base_document();
        let baseline = mochi_core::base::serialize_base_document(&baseline_document).unwrap();
        let mut changed_document = baseline_document.clone();
        changed_document.tables[0].name = "当前版本".into();
        let changed = mochi_core::base::serialize_base_document(&changed_document).unwrap();
        let mut proposed_document = baseline_document;
        let field_id = proposed_document.tables[0].fields[0].id.clone();
        proposed_document.tables[0]
            .records
            .push(base_record("record-a", &field_id, "甲"));
        let proposed = mochi_core::base::serialize_base_document(&proposed_document).unwrap();

        let entry = base_entry("approval-a", &baseline, &proposed);
        assert!(merge_base_proposals(std::slice::from_ref(&entry), &changed).is_err());

        let mismatched = base_entry("approval-b", &changed, &proposed);
        assert!(merge_base_proposals(&[entry, mismatched], &baseline).is_err());
    }

    #[test]
    fn merge_block_proposals_combines_different_blocks_and_rejects_repeated_target() {
        let initial = mochi_core::document_blocks::prepare_conversion(
            Path::new("note.md"),
            "第一块。\n\n第二块。\n",
        )
        .unwrap()
        .converted_source;
        let initial_document =
            mochi_core::document_blocks::document_from_source(Path::new("note.md"), &initial)
                .unwrap();
        let first_id = initial_document.blocks[0].id.to_string();
        let second_id = initial_document.blocks[1].id.to_string();
        let first = block_entry_at(&initial, 0, "approval-a", "第一块的新版本。\n");
        let second = block_entry_at(&initial, 1, "approval-b", "第二块的新版本。\n");

        let merged = merge_block_proposals(&[first, second], &initial).unwrap();
        let merged_document =
            mochi_core::document_blocks::document_from_source(Path::new("note.md"), &merged)
                .unwrap();
        assert_eq!(
            merged_document.find_block(&first_id).unwrap().content,
            "第一块的新版本。"
        );
        assert_eq!(
            merged_document.find_block(&second_id).unwrap().content,
            "第二块的新版本。"
        );

        let first = block_entry_at(&initial, 0, "approval-c", "第一块的另一个版本。\n");
        let second = block_entry_at(&initial, 0, "approval-d", "第一块的冲突版本。\n");
        assert!(merge_block_proposals(&[first, second], &initial).is_err());
    }

    #[test]
    fn block_approval_preserves_unrelated_edits_and_rejects_changed_target() {
        let initial = mochi_core::document_blocks::prepare_conversion(
            Path::new("note.md"),
            "第一块。\n\n第二块。\n",
        )
        .unwrap()
        .converted_source;
        let entry = block_entry(&initial);
        let current = initial.replace("第二块。", "第二块的本地修改😀。");
        let applied = block_writeback_source(&entry, &current).unwrap();
        assert!(applied.contains("修改后的第一块。"));
        assert!(applied.contains("第二块的本地修改😀。"));
        let before =
            mochi_core::document_blocks::document_from_source(Path::new("note.md"), &initial)
                .unwrap();
        let after =
            mochi_core::document_blocks::document_from_source(Path::new("note.md"), &applied)
                .unwrap();
        assert_eq!(
            before.blocks.iter().map(|b| &b.id).collect::<Vec<_>>(),
            after.blocks.iter().map(|b| &b.id).collect::<Vec<_>>()
        );
        assert!(
            block_writeback_source(&entry, &initial.replace("第一块。", "用户已编辑第一块。"))
                .is_err()
        );
        assert!(block_writeback_source(&entry, &applied).is_err());
        let review = crate::ui::command_review::State::file(entry, Some(current), None);
        assert!(review.field.text().contains("- 第一块。"));
        assert!(review.field.text().contains("+ 修改后的第一块。"));
        assert!(!review.field.text().contains("第二块"));
    }

    #[test]
    fn malformed_block_proposal_cannot_redirect_path_or_forge_original_hash() {
        let initial =
            mochi_core::document_blocks::prepare_conversion(Path::new("note.md"), "内容\n")
                .unwrap()
                .converted_source;
        let mut value = block_entry(&initial).to_value();
        value["operation"]["blockEdit"]["path"] = "other.md".into();
        assert!(
            block_writeback_source(&InboxEntry::from_value(&value).unwrap(), &initial).is_err()
        );
        value["operation"]["blockEdit"]["path"] = "note.md".into();
        value["operation"]["blockEdit"]["oldText"] = "伪造原文".into();
        assert!(
            block_writeback_source(&InboxEntry::from_value(&value).unwrap(), &initial).is_err()
        );
    }

    #[test]
    fn inbox_row_approval_merges_independent_base_records_end_to_end() {
        let root = std::env::temp_dir().join(format!(
            "mochi-base-inbox-approval-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let path = root.join("data.mcb");
            let baseline_document = mochi_core::base::create_base_document();
            let field_id = baseline_document.tables[0].fields[0].id.clone();
            let baseline = mochi_core::base::serialize_base_document(&baseline_document).unwrap();
            std::fs::write(&path, &baseline).unwrap();

            let mut first_document = baseline_document.clone();
            first_document.tables[0]
                .records
                .push(base_record("record-a", &field_id, "甲"));
            let first = mochi_core::base::serialize_base_document(&first_document).unwrap();

            let mut second_document = baseline_document;
            second_document.tables[0]
                .records
                .push(base_record("record-b", &field_id, "乙"));
            let second = mochi_core::base::serialize_base_document(&second_document).unwrap();

            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            assert!(app.shell.open_file(&path));

            let service = AgentInboxService::new(&root);
            let path_text = path.to_string_lossy().into_owned();
            let first_entry = service
                .add(
                    serde_json::json!({
                        "kind": "overwrite",
                        "path": path_text,
                        "status": "pending",
                        "previousContent": baseline,
                        "content": first
                    }),
                    None,
                    Some("测试审批"),
                )
                .unwrap();
            let second_entry = service
                .add(
                    serde_json::json!({
                        "kind": "overwrite",
                        "path": path.to_string_lossy(),
                        "status": "pending",
                        "previousContent": baseline,
                        "content": second
                    }),
                    None,
                    Some("测试审批"),
                )
                .unwrap();
            assert_ne!(first_entry.id(), second_entry.id());

            app.state.ai_panel_open = true;
            app.set_right_panel(RightPanel::AgentInbox);
            assert_eq!(app.panels.inbox.entries.len(), 2);
            assert!(app
                .panels
                .inbox
                .entries
                .iter()
                .all(|entry| entry.is_pending()));
            assert_eq!(app.inbox_approval_group(0).len(), 2);

            // 这里走的正是面板用的紧凑行对勾路径。
            app.approve_inbox_approval_group(0);

            let applied =
                mochi_core::base::parse_base_document(&std::fs::read_to_string(&path).unwrap())
                    .unwrap();
            let records = &applied.tables[0].records;
            assert_eq!(records.len(), 2);
            assert_eq!(
                records
                    .iter()
                    .find(|record| record.id == "record-a")
                    .unwrap()
                    .values[&field_id],
                serde_json::json!("甲")
            );
            assert_eq!(
                records
                    .iter()
                    .find(|record| record.id == "record-b")
                    .unwrap()
                    .values[&field_id],
                serde_json::json!("乙")
            );

            assert!(service.list_pending().is_empty());
            let inbox = serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(root.join(mochi_core::ai::agent_inbox::INBOX_REL_PATH))
                    .unwrap(),
            )
            .unwrap();
            for id in [first_entry.id(), second_entry.id()] {
                let persisted = inbox["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["id"].as_str() == Some(id))
                    .unwrap();
                assert_eq!(persisted["operation"]["status"], "applied");
            }
            assert!(app.commands.review.is_none(), "{}", app.state.status_text);
        }
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        let _ = std::fs::remove_dir_all(resolved);
    }

    #[test]
    fn stale_review_cannot_write_after_another_actor_resolves_the_approval() {
        let root = std::env::temp_dir().join(format!(
            "mochi-stale-inbox-approval-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let path = root.join("note.md");
            let original = "# 原文\n";
            std::fs::write(&path, original).unwrap();
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            assert!(app.shell.open_file(&path));

            let service = AgentInboxService::new(&root);
            let entry = service
                .add(
                    serde_json::json!({
                        "kind": "overwrite",
                        "path": path.to_string_lossy(),
                        "status": "pending",
                        "previousContent": original,
                        "content": "# 不应写入\n"
                    }),
                    None,
                    Some("并发审批测试"),
                )
                .unwrap();
            app.review_file_proposal(entry.clone());
            assert!(app.commands.review.as_ref().unwrap().error.is_empty());

            service.resolve(entry.id(), "rejected", None).unwrap();
            app.apply_reviewed_file();

            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            let review = app.commands.review.as_ref().unwrap();
            assert!(review.error.contains("其他窗口"), "{}", review.error);
            let stored: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(root.join(mochi_core::ai::agent_inbox::INBOX_REL_PATH))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(stored["entries"][0]["operation"]["status"], "rejected");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn new_file_proposal_rejects_target_created_before_first_review() {
        let root = std::env::temp_dir().join(format!(
            "mochi-new-file-inbox-approval-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let path = root.join("created-after-proposal.md");
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);

            let service = AgentInboxService::new(&root);
            let entry = service
                .add(
                    serde_json::json!({
                        "kind": "write",
                        "path": path.to_string_lossy(),
                        "status": "pending",
                        "content": "AI 提案内容\n"
                    }),
                    None,
                    Some("新建文件竞态测试"),
                )
                .unwrap();

            let external = "其他进程先创建的内容\n";
            std::fs::write(&path, external).unwrap();

            assert!(!app.approve_file_proposals(vec![entry.clone()]));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
            assert!(app
                .commands
                .review
                .as_ref()
                .is_some_and(|review| review.error.contains("新建文件提案目标")));
            assert_eq!(
                service
                    .list_pending()
                    .iter()
                    .filter(|pending| pending.id() == entry.id())
                    .count(),
                1
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn proposals_compare_unsaved_base_state_and_accept_written_snapshots() {
        let root = std::env::temp_dir().join(format!(
            "mochi-base-proposal-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        let path = root.join("data.mcb");
        let raw = format!(
            "{}\n",
            mochi_core::base::serialize_base_document(&mochi_core::base::create_base_document())
                .unwrap()
        );
        std::fs::write(&path, &raw).unwrap();
        assert!(app.shell.open_file(&path));
        assert_eq!(
            app.current_proposal_text(&path).as_deref(),
            Some(raw.as_str())
        );
        if let Some(viewer::Content::Base(state)) = app.viewer_content_mut() {
            state.activate(base_view::Hit::ToggleEditing);
            state.activate(base_view::Hit::NewRecord);
            state
                .submit(base_view::Edit::Cell(0, 0), "未保存修改")
                .unwrap();
        }
        let current = app.current_proposal_text(&path).unwrap();
        assert!(current.contains("未保存修改"));
        assert_ne!(current, raw);
        let mut document = mochi_core::base::parse_base_document(&current).unwrap();
        document.tables[0].name = "更新的数据表".into();
        let written = mochi_core::base::serialize_base_document(&document).unwrap();
        std::fs::write(&path, &written).unwrap();
        app.shell.accept_written_text(&path, &written);
        assert_eq!(
            app.current_proposal_text(&path).as_deref(),
            Some(written.as_str())
        );
        assert!(!app.shell.active().unwrap().dirty());
        assert!(
            matches!(app.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.table().name=="更新的数据表")
        );
        drop(app);
        let resolved = root.canonicalize().unwrap();
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        assert!(
            resolved.starts_with(&temporary)
                && resolved
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("mochi-base-proposal-"))
        );
        let _ = std::fs::remove_dir_all(resolved);
    }
}
