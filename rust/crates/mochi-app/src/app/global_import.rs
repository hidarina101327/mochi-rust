//! 管理全局资源包导入的校验、确认和取消流程。
use super::*;
use crate::ui::import_picker::{Folder, Hit, State};

pub(super) struct Pending {
    workspace: PathBuf,
    paths: Vec<PathBuf>,
    previous_focus: Focus,
    pub view: State,
}

impl App {
    /// 包导入优先于普通文件复制，侧边栏拖放也一样。
    pub(super) fn offer_package_import(&mut self, paths: &[PathBuf]) -> bool {
        for (index, path) in paths.iter().enumerate() {
            let package = match mochi_core::transfer::inspect(path) {
                Ok(Some(package)) => package,
                Ok(None) => continue,
                Err(error) => {
                    self.show_global_notice(&format!("无法导入 {}：{error:#}", path.display()));
                    return true;
                }
            };
            let Some(workspace) = self.shell.workspace().map(|ws| ws.root.clone()) else {
                self.show_global_notice("请先打开工作区，再导入 Mochi 包");
                return true;
            };
            let label = package.kind.label();
            let note = match package.kind {
                mochi_core::transfer::Kind::Workflow => "导入为新工作流，定时运行默认关闭。",
                mochi_core::transfer::Kind::Agent => {
                    "添加到 Agent 配置，不覆盖现有定义。引用的 Skill、工具和 MCP 需在此工作区可用。"
                }
                mochi_core::transfer::Kind::DesktopCards => "添加到桌面卡片，导入的卡片默认隐藏。",
            };
            let mut seen = HashSet::new();
            let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
            seen.insert(identity);
            let remaining = paths
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    *i != index && seen.insert(p.canonicalize().unwrap_or_else(|_| (*p).clone()))
                })
                .map(|(_, p)| p.clone())
                .collect();
            self.menu = None;
            self.cancel_sidebar_tree_drag();
            self.cancel_navigation_library_drag();
            self.drag = None;
            self.dialog = Some(Dialog {
                title: format!("导入{label}"),
                description: format!("识别到{label}「{}」。是否导入当前工作区？", package.name),
                field: None,
                error: String::new(),
                note: Some(note.into()),
                buttons: vec![
                    DialogButton {
                        label: "取消".into(),
                        kind: ButtonKind::Ghost,
                        action: DialogAction::Dismiss,
                    },
                    DialogButton {
                        label: format!("导入{label}"),
                        kind: ButtonKind::Primary,
                        action: DialogAction::ImportPackage {
                            workspace,
                            package: Box::new(package),
                            remaining,
                        },
                    },
                ],
                dismiss: DialogAction::Dismiss,
                hover: None,
            });
            self.focus = Focus::Dialog;
            return true;
        }
        false
    }

    pub(super) fn confirm_package_import(
        &mut self,
        workspace: PathBuf,
        package: mochi_core::transfer::Package,
        remaining: Vec<PathBuf>,
    ) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                self.shell
                    .workspace()
                    .is_some_and(|ws| ws.root == workspace),
                "工作区已切换，请重新拖入文件"
            );
            if package.kind == mochi_core::transfer::Kind::DesktopCards {
                self.import_desktop_package(&package)?;
            } else {
                package.install(&workspace)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if let Some(dialog) = &mut self.dialog {
                dialog.error = format!("导入失败：{error:#}");
            }
            return;
        }
        self.close_dialog();
        match package.kind {
            mochi_core::transfer::Kind::Workflow => self.workflows_refresh(),
            mochi_core::transfer::Kind::Agent => self.reload_agent_config(),
            mochi_core::transfer::Kind::DesktopCards => {}
        }
        self.show_global_notice(&format!(
            "已导入{}「{}」",
            package.kind.label(),
            package.name
        ));
        if !remaining.is_empty() && !self.offer_package_import(&remaining) {
            self.open_global_import(remaining);
        }
    }

    pub(super) fn open_global_import(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let Some(workspace) = self.shell.workspace() else {
            self.show_global_notice("请先打开工作区，再拖入文件");
            return;
        };
        let roots = workspace
            .libraries
            .iter()
            .filter_map(|library| {
                let path = PathBuf::from(&library.path);
                path.is_dir()
                    .then(|| Folder::new(path, library.name.clone(), 0))
            })
            .collect();
        let preferred = self.shell.tree_root();
        let mut seen = HashSet::new();
        let paths = paths
            .into_iter()
            .filter(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())))
            .collect::<Vec<_>>();
        let pending = Pending {
            workspace: workspace.root.clone(),
            view: State::new(roots, &paths, preferred.as_deref()),
            paths,
            previous_focus: self.focus,
        };
        self.menu = None;
        self.cancel_sidebar_tree_drag();
        self.cancel_navigation_library_drag();
        self.drag = None;
        self.global_import = Some(pending);
        self.focus = Focus::Dialog;
        if let Some(index) = self
            .global_import
            .as_ref()
            .and_then(|p| p.view.selected_index())
        {
            let viewport = self.renderer.viewport();
            self.global_import
                .as_mut()
                .unwrap()
                .view
                .select(index, viewport);
            self.toggle_import_folder(index);
        }
    }

    pub(super) fn close_global_import(&mut self) {
        if let Some(pending) = self.global_import.take() {
            self.focus = pending.previous_focus;
        }
    }

    fn validate_import_target(&self, target: &Path) -> anyhow::Result<()> {
        let pending = self
            .global_import
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("导入已取消"))?;
        let workspace = self
            .shell
            .workspace()
            .ok_or_else(|| anyhow::anyhow!("请先打开工作区"))?;
        anyhow::ensure!(
            workspace.root == pending.workspace,
            "工作区已切换，请重新拖入文件"
        );
        anyhow::ensure!(target.is_dir(), "目标文件夹已不存在，请重新选择");
        let library = self
            .shell
            .library_root_for_path(target)
            .ok_or_else(|| anyhow::anyhow!("请选择知识库中的文件夹"))?;
        anyhow::ensure!(
            mochi_core::paths::path_is_within(&library.canonicalize()?, &target.canonicalize()?),
            "目标目录指向知识库之外，请重新选择"
        );
        anyhow::ensure!(
            mochi_core::mapped_folders::Service::new(&workspace.root)
                .for_path(target)?
                .is_none(),
            "映射目录不接收导入，请选择实际文件夹"
        );
        Ok(())
    }

    fn toggle_import_folder(&mut self, index: usize) {
        let viewport = self.renderer.viewport();
        let Some(pending) = self.global_import.as_mut() else {
            return;
        };
        let Some(folder) = pending.view.folders.get(index) else {
            return;
        };
        if folder.expanded {
            pending.view.collapse(index, viewport);
            return;
        }
        if !folder.expandable {
            return;
        }
        let path = folder.path.clone();
        let depth = folder.depth + 1;
        let children = self
            .validate_import_target(&path)
            .and_then(|_| mochi_core::files::FileService::new().build_file_tree_shallow(&path));
        let pending = self.global_import.as_mut().unwrap();
        match children {
            Ok(children) => {
                let children = children
                    .into_iter()
                    .filter(|node| node.is_directory())
                    .filter(|node| {
                        std::fs::symlink_metadata(&node.path)
                            .is_ok_and(|metadata| !metadata.file_type().is_symlink())
                    })
                    .map(|node| Folder::new(PathBuf::from(node.path), node.name, depth))
                    .collect::<Vec<_>>();
                pending.view.folders[index].expanded = !children.is_empty();
                pending.view.folders[index].expandable = !children.is_empty();
                pending.view.folders.splice(index + 1..index + 1, children);
                pending.view.error.clear();
                pending.view.hover = None;
            }
            Err(error) => pending.view.error = format!("无法读取目录：{error}"),
        }
    }

    pub(super) fn import_picker_action(&mut self, hit: Hit) {
        match hit {
            Hit::Outside | Hit::Cancel => self.close_global_import(),
            Hit::Select(index) => {
                let viewport = self.renderer.viewport();
                if let Some(pending) = self.global_import.as_mut() {
                    pending.view.select(index, viewport);
                }
            }
            Hit::Toggle(index) => {
                if let Some(pending) = self.global_import.as_mut() {
                    pending.view.focus = crate::ui::import_picker::Focus::Tree;
                }
                self.toggle_import_folder(index);
            }
            Hit::Confirm => {
                let Some(target) = self
                    .global_import
                    .as_ref()
                    .and_then(|p| p.view.selected.clone())
                else {
                    return;
                };
                if let Err(error) = self.validate_import_target(&target) {
                    self.global_import.as_mut().unwrap().view.error = error.to_string();
                    return;
                }
                let pending = self.global_import.take().unwrap();
                self.focus = pending.previous_focus;
                self.request_external_import(&pending.paths, target);
            }
            Hit::Inside => {}
        }
    }

    pub(super) fn import_picker_key(
        &mut self,
        key: u16,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        if alt && key == 0x73 {
            self.close_global_import();
            return true;
        }
        let viewport = self.renderer.viewport();
        if !ctrl && !alt {
            if let Some(action) = self
                .global_import
                .as_mut()
                .and_then(|p| p.view.key(key, shift, viewport))
            {
                self.import_picker_action(action);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
