//! 处理工作流库中的选择和上下文菜单操作。
use super::*;
use std::result::Result;

impl App {
    pub(super) fn workflows_library_action(&mut self, hit: &ui::Hit) -> Result<bool, String> {
        use ui::{Editor, Hit};
        let mut items = vec![];
        match hit {
            Hit::NewFolder => {
                self.workflows.view.editing_folder = None;
                self.workflows
                    .view
                    .set_editor(Editor::Folder, String::new());
                self.focus = Focus::Workflow;
            }
            Hit::Folder(id) => {
                self.workflows.view.folder = id.clone();
                self.workflows.view.scroll = 0.;
            }
            Hit::RenameFolder(id) => {
                let folder = self
                    .workflows
                    .view
                    .folders
                    .iter()
                    .find(|f| f.id == *id)
                    .ok_or("文件夹不存在")?
                    .clone();
                self.workflows.view.editing_folder = Some(folder.id);
                self.workflows.view.set_editor(Editor::Folder, folder.name);
                self.focus = Focus::Workflow;
            }
            Hit::DeleteFolder(id) => {
                self.workflows
                    .store
                    .as_ref()
                    .ok_or("请先打开工作区")?
                    .delete_folder(id)?;
                if self.workflows.view.folder.as_ref() == Some(id) {
                    self.workflows.view.folder = None;
                }
                self.workflows_refresh();
                self.workflows.view.status = "文件夹已移除，工作流保留在总览中".into();
            }
            Hit::FolderMenu(id) => {
                items = vec![
                    MenuItem::new(
                        "重命名文件夹",
                        MenuAction::WorkflowAction(Hit::RenameFolder(id.clone())),
                    ),
                    MenuItem::new(
                        "移除文件夹，保留工作流",
                        MenuAction::WorkflowAction(Hit::DeleteFolder(id.clone())),
                    ),
                ];
            }
            Hit::WorkflowMenu(i) => {
                let flow = self
                    .workflows
                    .view
                    .workflows
                    .get(*i)
                    .ok_or("工作流不存在")?;
                items.push(MenuItem::new(
                    "打开工作流",
                    MenuAction::WorkflowAction(Hit::Open(*i)),
                ));
                items.push(MenuItem::new(
                    "移至总览",
                    MenuAction::WorkflowAction(Hit::MoveWorkflow(flow.id.clone(), None)),
                ));
                for folder in &self.workflows.view.folders {
                    items.push(MenuItem::new(
                        format!("移至文件夹 · {}", folder.name),
                        MenuAction::WorkflowAction(Hit::MoveWorkflow(
                            flow.id.clone(),
                            Some(folder.id.clone()),
                        )),
                    ));
                }
            }
            Hit::MoveWorkflow(id, folder) => {
                self.workflows
                    .store
                    .as_ref()
                    .ok_or("请先打开工作区")?
                    .move_to_folder(id, folder.as_deref())?;
                self.workflows_refresh();
                self.workflows.view.status = "工作流已移动".into();
            }
            _ => return Ok(false),
        }
        if !items.is_empty() {
            let anchor = self
                .workflows
                .view
                .hits
                .iter()
                .find(|(_, h)| h == hit)
                .map(|(r, _)| *r)
                .unwrap_or(self.workflows.view.area);
            self.menu = Some(Menu::open_anchored(items, anchor, self.renderer.viewport()));
        }
        Ok(true)
    }

    pub(in crate::app) fn workflows_context_menu(&mut self, x: f32, y: f32) {
        use ui::Hit;
        if self.workflows.view.run.is_some() {
            return;
        }
        let hit = self.workflows.view.hit(x, y);
        let action = match hit {
            Some(Hit::Edge(i) | Hit::Insert(i)) => {
                let v = &mut self.workflows.view;
                v.selected_edge = Some(i);
                v.selected_node = None;
                v.selected_binding = None;
                v.group.clear();
                Some(("删除执行连线", Hit::Delete))
            }
            Some(Hit::Binding(i, key)) => Some(("断开变量连线", Hit::RemoveBinding(i, key))),
            Some(Hit::Open(i)) => {
                if let Err(error) = self.workflows_action(Hit::WorkflowMenu(i)) {
                    self.workflows.view.error = error;
                }
                return;
            }
            Some(Hit::Folder(Some(id))) => {
                if let Err(error) = self.workflows_action(Hit::FolderMenu(id)) {
                    self.workflows.view.error = error;
                }
                return;
            }
            _ => None,
        };
        if let Some((label, action)) = action {
            self.menu = Some(Menu::open_at(
                vec![MenuItem::new(label, MenuAction::WorkflowAction(action))],
                x,
                y,
                self.renderer.viewport(),
            ));
        }
    }
}
