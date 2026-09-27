//! 执行文件和文件夹的创建、复制、移动、重命名与删除。
use super::*;

impl Shell {
    /// 新建文件。内容照 Electron：`# 文件名\n\n`。成功后刷新树并返回路径。
    pub fn create_file(&mut self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        if path.exists() {
            anyhow::bail!("同名文件已存在");
        }
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        let content = if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mcb"))
        {
            mochi_core::base::serialize_base_document(&mochi_core::base::create_base_document())?
        } else {
            format!("# {stem}\n\n")
        };
        FileService::new().write_file_safe(&path, &content)?;
        self.expanded.insert(parent.to_path_buf());
        self.refresh_tree();
        Ok(path)
    }

    /// 新建 Mochi 画布。`.mcanvas` 是 Mochi 专属 JSON 格式，卡片只保存
    /// 对原有对象的引用与位置，绝不复制文档内容。
    pub fn create_canvas(&mut self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        if path.exists() {
            anyhow::bail!("同名文件已存在");
        }
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(mochi_core::canvas::EXTENSION))
        {
            anyhow::bail!("画布文件必须使用 .{} 后缀", mochi_core::canvas::EXTENSION);
        }
        let content = mochi_core::canvas::serialize(&mochi_core::canvas::CanvasDocument::empty())?;
        FileService::new().write_file_safe(&path, &content)?;
        self.expanded.insert(parent.to_path_buf());
        self.refresh_tree();
        Ok(path)
    }

    pub fn create_folder(&mut self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        if path.exists() {
            anyhow::bail!("同名文件夹已存在");
        }
        std::fs::create_dir_all(&path)?;
        self.expanded.insert(parent.to_path_buf());
        self.refresh_tree();
        Ok(path)
    }

    /// 复制单个文档为同目录副本。副本只包含文档本身；子文档伴生夹和其他
    /// 路径关联数据不应与原文档共用。首个名称为“源名称 1.ext”，冲突时递增。
    pub fn duplicate_file(&mut self, source: &Path) -> Result<PathBuf> {
        if !source.is_file() {
            anyhow::bail!("只能复制文件");
        }
        let parent = source.parent().context("无法确定副本文档所在目录")?;
        let name = source
            .file_name()
            .context("无法确定源文档名称")?
            .to_string_lossy();
        let (stem, extension) = name
            .rfind('.')
            .filter(|dot| *dot > 0)
            .map(|dot| (&name[..dot], &name[dot..]))
            .unwrap_or((&name, ""));
        let target = (1..=1000)
            .map(|number| parent.join(format!("{stem} {number}{extension}")))
            .find(|candidate| !candidate.exists())
            .context("无法生成不重复的副本文档名称")?;
        std::fs::copy(source, &target)
            .with_context(|| format!("复制文档失败: {}", source.display()))?;
        if let Some(workspace) = &self.workspace {
            let _ = workspace.index.index_single_file(&target);
        }
        self.refresh_tree();
        Ok(target)
    }

    pub fn move_path(&mut self, from: &Path, to: &Path) -> Result<PathBuf> {
        if self.is_mapped_folder_root(from) {
            anyhow::bail!("映射文件夹是虚拟入口，不能作为实际目录移动");
        }
        let to = to.to_path_buf();
        if to == from {
            return Ok(to);
        }
        if to.exists() {
            anyhow::bail!("同名文件已存在");
        }
        // 在文件监听清掉旧目标记录之前取得反链；正文改写放在重命名成功之后。
        let links = self
            .workspace
            .as_ref()
            .and_then(|ws| {
                let _ = ws.index.index_single_file(from);
                ws.index.get_links_to(from).ok()
            })
            .unwrap_or_default();
        mochi_core::rename::with_companions(from, &to)?;
        if to.is_dir() {
            if let Some(ws) = &self.workspace {
                let library = to
                    .parent()
                    .and_then(|parent| self.library_root_for_path(parent));
                if let Err(error) = mochi_core::mapped_folders::Service::new(&ws.root).move_mounts(
                    from,
                    &to,
                    library.as_deref(),
                ) {
                    mochi_core::rename::with_companions(&to, from)
                        .context("更新映射挂载位置失败，回滚目录移动也失败")?;
                    return Err(error).context("更新映射挂载位置失败，目录移动已回滚");
                }
            }
        }
        let warning = self
            .workspace
            .as_ref()
            .and_then(|ws| {
                mochi_core::sub_documents::update_path(
                    &ws.root,
                    &from.to_string_lossy(),
                    &to.to_string_lossy(),
                )
                .err()
            })
            .map(|e| format!("文件已改名，但子文档索引更新失败：{e}"));
        if let Some(ws) = &self.workspace {
            if let Err(error) =
                mochi_core::document_unread::Store::new(&ws.root).relocate(from, &to)
            {
                self.status = format!("文件已移动，但未读状态更新失败：{error}");
            }
        }
        let old_side = PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &from.to_string_lossy(),
        ));
        let new_side = PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &to.to_string_lossy(),
        ));
        for tab in &mut self.tabs {
            tab.history.remap(from, &to, &old_side, &new_side);
            let Some(path) = tab.path() else { continue };
            let changed = if path == from {
                Some(to.clone())
            } else {
                path.strip_prefix(from)
                    .ok()
                    .map(|rest| to.join(rest))
                    .or_else(|| {
                        path.strip_prefix(&old_side)
                            .ok()
                            .map(|rest| new_side.join(rest))
                    })
            };
            if let Some(next) = changed {
                tab.title = next
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                match &mut tab.kind {
                    TabKind::File { path, .. } => *path = next,
                    TabKind::Viewer { path, content } => {
                        if !matches!(content, crate::ui::viewer::Content::Base(_)) {
                            *content =
                                crate::ui::viewer::load(&next, &crate::ui::viewer::detect(&next));
                        }
                        *path = next;
                    }
                    _ => {}
                }
            }
        }
        remap_expanded_paths(&mut self.expanded, from, &to);
        remap_expanded_paths(&mut self.favorites_expanded, from, &to);
        let mut link_warning = None;
        if let Some(ws) = &self.workspace {
            let index = ws.index.clone();
            let root = ws.root.canonicalize().unwrap_or_else(|_| ws.root.clone());
            let link_root = index.workspace_path().to_path_buf();
            let _ = index.index_single_file(&to);
            let mut sources = std::collections::BTreeMap::<
                PathBuf,
                Vec<mochi_core::metadata_index::NoteLink>,
            >::new();
            for link in links {
                let path = if link.source_path == from {
                    to.clone()
                } else {
                    link.source_path
                        .strip_prefix(&old_side)
                        .map(|rest| new_side.join(rest))
                        .unwrap_or_else(|_| link.source_path.clone())
                };
                sources.entry(path).or_default().push(link);
            }
            // 遍历所有已打开的文本缓冲区，这样索引更新后才插入的链接
            // 也能被改写。核心改写逻辑会检查
            // Mochi URL 对应的确切移动路径；已索引的链接则继续
            // 沿用普通维基链接的现有保护逻辑。
            for tab in &self.tabs {
                if tab.buffer().is_some() {
                    if let Some(path) = tab.path() {
                        sources.entry(path.to_path_buf()).or_default();
                    }
                }
            }
            for (path, links) in sources {
                if !path
                    .canonicalize()
                    .is_ok_and(|p| mochi_core::paths::path_is_within(&root, &p))
                {
                    continue;
                }
                if let Some(i) = self.tabs.iter().position(|t| {
                    t.path()
                        .is_some_and(|open| same_native_path(open, path.as_path()))
                        && t.buffer().is_some()
                }) {
                    let tab = &mut self.tabs[i];
                    let TabKind::File { buffer, .. } = &mut tab.kind else {
                        continue;
                    };
                    let dirty = buffer.dirty();
                    let source = buffer.text().to_owned();
                    let edits = mochi_core::rename_links::edits_for_move(
                        &path, &source, &links, from, &to, &link_root,
                    );
                    if edits.is_empty() {
                        continue;
                    }
                    let (a, b) = buffer.selection();
                    let cursor = buffer.cursor();
                    let anchor = if cursor == a { b } else { a };
                    let updated = mochi_core::rename_links::apply(&source, &edits);
                    buffer.replace_range(0..source.len(), &updated);
                    buffer.set_cursor(mochi_core::rename_links::map_offset(anchor, &edits), false);
                    buffer.set_cursor(mochi_core::rename_links::map_offset(cursor, &edits), true);
                    if !dirty && !self.save_tab_content(i, false, false) {
                        link_warning = Some("文件已改名，部分引用更新尚未保存".to_owned());
                    }
                    continue;
                }
                let updated = (|| -> Result<()> {
                    let source = std::fs::read_to_string(&path)?;
                    let edits = mochi_core::rename_links::edits_for_move(
                        &path, &source, &links, from, &to, &link_root,
                    );
                    if edits.is_empty() {
                        return Ok(());
                    }
                    let updated = mochi_core::rename_links::apply(&source, &edits);
                    if std::fs::read_to_string(&path)? != source {
                        anyhow::bail!("引用文件同时被其它程序修改")
                    };
                    FileService::new().write_file_safe(&path, &updated)?;
                    index.index_single_file(&path)?;
                    Ok(())
                })();
                if let Err(error) = updated {
                    link_warning = Some(format!("文件已改名，引用更新失败：{error}"));
                }
            }
        }
        let base_warning = self.update_base_references_for_rename(from, &to);
        let favorites_warning = self.remap_favorites(from, &to);
        self.refresh_tree();
        let warnings = [warning, link_warning, base_warning, favorites_warning]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if !warnings.is_empty() {
            self.status = warnings.join("；");
        }
        Ok(to)
    }

    /// 删除（永久）。相关标签一并关掉——留着会在保存时把文件写回来。
    pub fn delete(&mut self, path: &Path) -> Result<()> {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
        if let Some(ws) = &self.workspace {
            mochi_core::document_unread::Store::new(&ws.root).remove_tree(path)?;
        }
        self.close_tabs_under(path);
        remove_expanded_under(&mut self.expanded, path);
        remove_expanded_under(&mut self.favorites_expanded, path);
        self.refresh_tree();
        self.remove_favorites_under(path);
        Ok(())
    }

    /// 已经在外部（回收站）删除后的收尾。
    pub fn after_external_delete(&mut self, path: &Path) {
        if let Some(ws) = &self.workspace {
            let _ = mochi_core::document_unread::Store::new(&ws.root).remove_tree(path);
        }
        self.close_tabs_under(path);
        remove_expanded_under(&mut self.expanded, path);
        remove_expanded_under(&mut self.favorites_expanded, path);
        self.refresh_tree();
        self.remove_favorites_under(path);
    }

    pub(super) fn close_tabs_under(&mut self, path: &Path) {
        let mut i = 0;
        while i < self.tabs.len() {
            let gone = self.tabs[i]
                .path()
                .is_some_and(|open| native_path_is_same_or_under(open, path));
            if gone {
                self.discard_tab(i);
            } else {
                i += 1;
            }
        }
    }
}

/// 升级格式前，先检查伴随文件的目标位置，避免覆盖已有文件。
/// `mochi_core::rename::with_companions` 移动文件时也会执行相同检查；
/// 这里先检查，可以避免升级失败后源码已经被改写，
/// 而缓冲区仍保留旧的已保存快照。
pub(super) fn companion_target_conflict(from: &Path, to: &Path) -> Option<PathBuf> {
    if to.exists() {
        return Some(to.to_path_buf());
    }
    if !from.is_file() {
        return None;
    }

    let old = from.to_string_lossy();
    let new = to.to_string_lossy();
    let pairs = [
        (
            PathBuf::from(format!("{old}.blocks-backup")),
            PathBuf::from(format!("{new}.blocks-backup")),
        ),
        (
            PathBuf::from(format!("{old}.blocks-restore-backup")),
            PathBuf::from(format!("{new}.blocks-restore-backup")),
        ),
        (
            PathBuf::from(mochi_core::sidecars::comment_sidecar_path(&old)),
            PathBuf::from(mochi_core::sidecars::comment_sidecar_path(&new)),
        ),
        (
            PathBuf::from(mochi_core::sidecars::comment_assets_dir(&old)),
            PathBuf::from(mochi_core::sidecars::comment_assets_dir(&new)),
        ),
        (
            PathBuf::from(mochi_core::sidecars::annotation_sidecar_path(&old)),
            PathBuf::from(mochi_core::sidecars::annotation_sidecar_path(&new)),
        ),
        (
            PathBuf::from(mochi_core::sub_documents::sidecar_path(&old)),
            PathBuf::from(mochi_core::sub_documents::sidecar_path(&new)),
        ),
    ];
    pairs
        .into_iter()
        .find(|(source, target)| source.exists() && source != target && target.exists())
        .map(|(_, target)| target)
}
