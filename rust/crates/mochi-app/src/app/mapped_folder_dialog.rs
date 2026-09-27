//! 管理映射文件夹的选择、创建和配置对话框。
use super::*;

impl App {
    pub(super) fn open_link_dialog(&mut self, parent: PathBuf) {
        self.link_create = Some(link_create::State::new(parent));
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_mapped_folder_dialog(&mut self, library: PathBuf) {
        let rules = self.default_mapping_rules();
        self.mapped_folder = Some(mapped_folder::State::new(
            library,
            &rules.include,
            &rules.exclude,
        ));
        self.focus = Focus::Dialog;
    }

    pub(super) fn close_mapped_folder_dialog(&mut self) {
        self.mapped_folder = None;
        self.focus = Focus::Main;
    }

    pub(super) fn on_mapped_folder_click(&mut self, x: f32, y: f32) {
        let viewport = self.renderer.viewport();
        let Some(dialog) = self.mapped_folder.as_mut() else {
            return;
        };
        let hit = dialog.hit(viewport, x, y);
        if !matches!(
            hit,
            mapped_folder::Hit::Help | mapped_folder::Hit::RulesPopover
        ) {
            dialog.close_rule_help();
        }
        match hit {
            mapped_folder::Hit::Close
            | mapped_folder::Hit::Outside
            | mapped_folder::Hit::Cancel => {
                self.close_mapped_folder_dialog();
            }
            mapped_folder::Hit::Source => {
                if let Some(source) = platform::pick_folder(HWND(self.hwnd_raw as *mut _)) {
                    if let Some(dialog) = self.mapped_folder.as_mut() {
                        dialog.set_source(source);
                    }
                }
            }
            mapped_folder::Hit::Name
            | mapped_folder::Hit::Include
            | mapped_folder::Hit::Exclude => {
                let field = match dialog.hit(viewport, x, y) {
                    mapped_folder::Hit::Name => mapped_folder::Field::Name,
                    mapped_folder::Hit::Include => mapped_folder::Field::Include,
                    mapped_folder::Hit::Exclude => mapped_folder::Field::Exclude,
                    _ => unreachable!(),
                };
                dialog.active = field;
                let left = dialog.field_text_left(viewport, field);
                match field {
                    mapped_folder::Field::Name => dialog.name.click(x - left, false),
                    mapped_folder::Field::Include => dialog.include.click(x - left, false),
                    mapped_folder::Field::Exclude => dialog.exclude.click(x - left, false),
                }
            }
            mapped_folder::Hit::Advanced => dialog.advanced = !dialog.advanced,
            mapped_folder::Hit::Help => dialog.toggle_rule_help(x, y),
            mapped_folder::Hit::Create => self.create_mapped_folder_from_dialog(),
            mapped_folder::Hit::RulesPopover => {}
            mapped_folder::Hit::Inside => {}
        }
    }

    pub(super) fn create_mapped_folder_from_dialog(&mut self) {
        let Some(dialog) = self.mapped_folder.as_mut() else {
            return;
        };
        let Some(source) = dialog.source.clone() else {
            dialog.error = "请选择计算机上的文件夹".into();
            return;
        };
        let name = dialog.name.text().trim().to_owned();
        if name.is_empty() {
            dialog.error = "请输入显示名称".into();
            dialog.active = mapped_folder::Field::Name;
            return;
        }
        let library = dialog.library.clone();
        let rules = mochi_core::mapped_folders::MappingRules {
            include: mapped_folder::State::patterns(&dialog.include),
            exclude: mapped_folder::State::patterns(&dialog.exclude),
        };
        let Some(workspace) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            dialog.error = "请先打开工作区".into();
            return;
        };
        match mochi_core::mapped_folders::Service::new(workspace)
            .add(&library, &source, name, rules)
        {
            Ok(_) => {
                self.close_mapped_folder_dialog();
                self.shell.refresh_tree();
                self.sync_state();
                self.show_global_notice("映射文件夹已添加；内容会在展开时按需读取");
            }
            Err(error) => dialog.error = error.to_string(),
        }
    }
}
