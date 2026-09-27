//! 处理文件树中的重命名、移动和手动排序保存。
use super::*;

impl Shell {
    /// 重命名。打开着的标签跟着改路径与标题，否则保存会写回旧路径。
    pub fn rename(&mut self, from: &Path, new_name: &str) -> Result<PathBuf> {
        let Some(dir) = from.parent() else {
            anyhow::bail!("无法重命名根目录")
        };
        let to = dir.join(new_name);
        self.move_path(from, &to)
    }

    /// 处理文件树中的一次拖放。移动目录时复用 `move_path`，因此打开的标签、
    /// 收藏、子文档和引用都会随路径一起更新；纯重排只写入手动排序配置。
    pub fn move_for_tree_drop(
        &mut self,
        source: &Path,
        target: &Path,
        position: TreeDropPosition,
    ) -> Result<PathBuf> {
        let root = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root.clone())
            .context("尚未打开工作区")?;
        if self.is_mapped_folder_root(source) {
            anyhow::bail!("映射文件夹是虚拟入口，不能作为实际目录移动");
        }
        if self.is_mapped_folder_root(target) {
            anyhow::bail!("不能将项目拖放到映射文件夹");
        }
        if self.favorites_selected {
            anyhow::bail!("收藏视图中不能调整目录结构");
        }
        if !source.starts_with(&root) || !target.starts_with(&root) {
            anyhow::bail!("只能在当前工作区内移动项目");
        }
        if source == target {
            anyhow::bail!("不能把项目拖放到自身");
        }
        if source.is_dir() && target.starts_with(source) {
            anyhow::bail!("不能把文件夹移动到自身内部");
        }
        if matches!(position, TreeDropPosition::Before | TreeDropPosition::After)
            && self.sort_mode != "manual"
        {
            anyhow::bail!("当前按名称排序，切换为手动排序后才能调整顺序");
        }

        let target_parent = match position {
            TreeDropPosition::Into => {
                if !target.is_dir() {
                    anyhow::bail!("只能把项目放入文件夹");
                }
                target.to_path_buf()
            }
            TreeDropPosition::Before | TreeDropPosition::After => target
                .parent()
                .map(Path::to_path_buf)
                .context("无法移动工作区根目录")?,
        };
        if !target_parent.starts_with(&root) {
            anyhow::bail!("不能移出当前工作区");
        }
        // 拖放只能从可见行开始，源目录的父级必然是展开的。move_path 会刷新整棵
        // 树，因此明确保留这些展开状态，避免一次文件移动把用户正在浏览的目录收起。
        let source_parent = source.parent().map(Path::to_path_buf);
        let keep_source_parent_open = source_parent
            .as_ref()
            .is_some_and(|parent| self.active_expanded().contains(parent));
        let keep_target_parent_open = self.active_expanded().contains(&target_parent);
        let file_name = source.file_name().context("无法移动工作区根目录")?;
        let destination = target_parent.join(file_name);
        if destination != source
            && self.workspace.as_ref().is_some_and(|ws| {
                ws.libraries
                    .iter()
                    .any(|library| same_native_path(Path::new(&library.path), source))
            })
        {
            anyhow::bail!("知识库根目录只支持同级排序，不能拖入其他文件夹");
        }
        if destination != source {
            if destination.exists() {
                anyhow::bail!("目标位置已存在同名文件");
            }
            self.move_path(source, &destination)?;
        }

        if self.sort_mode == "manual" {
            let old_parent = source.parent().map(Path::to_path_buf);
            if old_parent.as_deref() != Some(target_parent.as_path()) {
                if let Some(parent) = old_parent {
                    self.remove_from_manual_order(&parent, source);
                }
            }
            let mut order = self.manual_order_with_children(&target_parent)?;
            order.retain(|path| path != &path_key(&destination));
            match position {
                TreeDropPosition::Before | TreeDropPosition::After => {
                    let target_key = path_key(target);
                    let index = order
                        .iter()
                        .position(|path| path == &target_key)
                        .context("拖放目标已不在目录中")?;
                    let insert_at = if position == TreeDropPosition::After {
                        index + 1
                    } else {
                        index
                    };
                    order.insert(insert_at, path_key(&destination));
                }
                TreeDropPosition::Into => order.push(path_key(&destination)),
            }
            self.manual_sort_orders
                .insert(path_key(&target_parent), order);
            self.write_manual_sort_orders()?;
        }
        if keep_source_parent_open {
            if let Some(parent) = source_parent {
                self.active_expanded_mut().insert(parent);
            }
        }
        if keep_target_parent_open {
            self.active_expanded_mut().insert(target_parent);
        }
        self.refresh_tree();
        Ok(destination)
    }

    pub(super) fn is_mapped_folder_root(&self, path: &Path) -> bool {
        self.workspace.as_ref().is_some_and(|workspace| {
            mochi_core::mapped_folders::Service::new(&workspace.root)
                .load()
                .map(|config| {
                    config.folders.into_iter().any(|folder| {
                        // `canonicalize` 在 Windows 可能给映射源附加 `\\?\` 前缀；
                        // 用 Shell 的规范化比较，不能让这个表示差异绕过根目录保护。
                        same_native_path(Path::new(&folder.source), path)
                    })
                })
                .unwrap_or(false)
        })
    }

    pub(super) fn remove_from_manual_order(&mut self, parent: &Path, child: &Path) {
        if let Some(order) = self.manual_sort_orders.get_mut(&path_key(parent)) {
            order.retain(|path| path != &path_key(child));
        }
    }

    pub(super) fn manual_order_with_children(&self, parent: &Path) -> Result<Vec<String>> {
        let mut order = self
            .manual_sort_orders
            .get(&path_key(parent))
            .cloned()
            .unwrap_or_else(|| {
                self.rows
                    .iter()
                    .filter(|row| {
                        row.path
                            .parent()
                            .is_some_and(|path| same_native_path(path, parent))
                    })
                    .map(|row| path_key(&row.path))
                    .collect()
            });
        // 缺失或未展开的子项与所显示的树用同一套确定性默认排序（目录在前），
        // 绝不依赖文件系统的枚举顺序。
        let children = FileService::new().build_file_tree_shallow(parent)?;
        for child in children {
            let key = path_key(Path::new(&child.path));
            if !order.contains(&key) {
                order.push(key);
            }
        }
        Ok(order)
    }

    pub(super) fn write_manual_sort_orders(&self) -> Result<()> {
        let root = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root.clone())
            .context("尚未打开工作区")?;
        let directory = root.join(".mochi");
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("无法创建排序目录：{}", directory.display()))?;
        let json = serde_json::to_string_pretty(&self.manual_sort_orders)?;
        std::fs::write(directory.join("drag-sort.json"), json).context("无法保存文件树排序")?;
        Ok(())
    }
}
