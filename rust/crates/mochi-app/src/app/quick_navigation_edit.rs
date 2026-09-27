//! 处理快速导航项和分组的新增、编辑、重命名与删除。
use super::*;

impl App {
    pub(super) fn open_quick_nav_dialog(&mut self, group: bool) {
        self.quick_nav_group_parent = if group {
            self.shell.workspace().and_then(|ws| {
                let service = mochi_core::quick_navigation::Service::new(&ws.root);
                service.load().ok().and_then(|model| {
                    model
                        .groups
                        .iter()
                        .any(|row| row.id == model.ui.selected)
                        .then_some(model.ui.selected)
                })
            })
        } else {
            None
        };
        let group_description = if self.quick_nav_group_parent.is_some() {
            "将在当前选中的分组下新建子分组。"
        } else {
            "将在根级创建分组；选中一个分组后可创建子分组。"
        };
        self.dialog = Some(Dialog {
            title: if group {
                "新建快捷导航分组"
            } else {
                "添加快捷方式"
            }
            .into(),
            description: if group {
                group_description
            } else {
                "输入网址、程序、文件或文件夹路径；不会复制或移动原文件。"
            }
            .into(),
            field: Some(TextField::new(if group {
                "例如：开发工具"
            } else {
                "https://example.com 或 C:\\Program Files\\App.exe"
            })),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: if group { "创建" } else { "添加" }.into(),
                    kind: ButtonKind::Primary,
                    action: if group {
                        DialogAction::QuickNavAddGroup
                    } else {
                        DialogAction::QuickNavAddItem
                    },
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_quick_nav_edit_dialog(&mut self, id: &str) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let service = mochi_core::quick_navigation::Service::new(root);
        let Ok(model) = service.load() else {
            return;
        };
        let Some(item) = model.items.iter().find(|item| item.id == id) else {
            return;
        };
        let value = format!(
            "{} | {} | {} | {}",
            item.name,
            item.target,
            item.note,
            item.background_color.as_deref().unwrap_or("")
        );
        self.dialog = Some(Dialog {
            title: "编辑快捷方式".into(),
            description: "格式：名称 | 网址或路径 | 备注 | 卡片颜色。颜色可填 #EAF4FF，也可留空；不会移动或删除目标文件。"
                .into(),
            field: Some(TextField::new("名称 | 位置 | 备注 | #RRGGBB").with_text(&value)),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "保存".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::QuickNavEditItem(id.to_owned()),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_quick_nav_rename_group_dialog(&mut self, id: &str) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let service = mochi_core::quick_navigation::Service::new(root);
        let Ok(model) = service.load() else {
            return;
        };
        let Some(group) = model.groups.iter().find(|group| group.id == id) else {
            return;
        };
        self.dialog = Some(Dialog {
            title: "重命名快捷导航分组".into(),
            description: "分组内的快捷方式与排序不会改变。".into(),
            field: Some(TextField::new("分组名称").with_text(&group.name)),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "保存".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::QuickNavRenameGroup(id.to_owned()),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_quick_nav_delete_group_dialog(&mut self, id: &str) {
        self.dialog = Some(Dialog {
            title: "删除快捷导航分组".into(),
            description: "将删除该分组及其子分组；其中的快捷方式不会被删除，会移至“未分组”。"
                .into(),
            field: None,
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "删除分组".into(),
                    kind: ButtonKind::Danger,
                    action: DialogAction::QuickNavDeleteGroup(id.to_owned()),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_quick_nav_browser_dialog(&mut self) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let service = mochi_core::quick_navigation::Service::new(root);
        let Ok(model) = service.load() else {
            return;
        };
        let value = format!(
            "{} | {}",
            model.settings.browser.mode, model.settings.browser.executable_path
        );
        self.dialog = Some(Dialog {
            title: "设置网页浏览器".into(),
            description: "格式：system | （留空使用系统默认），或 custom | C:\\Program Files\\Browser\\browser.exe。".into(),
            field: Some(TextField::new("system | 或 custom | 浏览器路径").with_text(&value)),
            error: String::new(), note: None,
            buttons: vec![DialogButton { label: "取消".into(), kind: ButtonKind::Ghost, action: DialogAction::Dismiss }, DialogButton { label: "保存".into(), kind: ButtonKind::Primary, action: DialogAction::QuickNavSetBrowser }],
            dismiss: DialogAction::Dismiss, hover: None,
        });
        self.focus = Focus::Dialog;
    }
}
