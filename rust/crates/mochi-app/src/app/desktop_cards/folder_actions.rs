//! User-triggered file operations and review dialogs for desktop folder cards.
use super::*;
use crate::desktop_window::folder::Command;
use crate::platform::desktop_files::{self as shell_files, ClipboardEffect};
use mochi_core::desktop_cards::{
    folder::{self, FolderConfig, FolderSort, FolderStack},
    folder_operations as files,
};
use std::sync::Mutex;

/// Serialize host file jobs with the background auto-organizer pass.
pub(super) static FILE_OPERATIONS: Mutex<()> = Mutex::new(());

pub(super) struct Outcome {
    message: String,
    folder: Option<FolderConfig>,
    organize: Option<files::OrganizePlan>,
}
pub(super) struct FileJob {
    epoch: u64,
    card: String,
    page: String,
    config: FolderConfig,
    rx: Receiver<std::result::Result<Outcome, String>>,
}
impl Outcome {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            folder: None,
            organize: None,
        }
    }
    fn folder(folder: FolderConfig) -> Self {
        Self {
            message: String::new(),
            folder: Some(folder),
            organize: None,
        }
    }
}

impl App {
    pub(super) fn desktop_folder_command(
        &mut self,
        index: usize,
        page_id: &str,
        command: Command,
        mut paths: Vec<String>,
    ) {
        if self.dialog.is_some() {
            self.desktop_show_main();
            return;
        }
        let Some(page) = self.desktop.config.cards[index]
            .active_page()
            .filter(|p| p.id == page_id && p.module == Module::Folder)
            .cloned()
        else {
            return;
        };
        let card = self.desktop.config.cards[index].id.clone();
        if self.desktop.file_job.is_some() {
            self.desktop_error("文件操作正在进行，请等待完成".into());
            return;
        }
        if command == Command::Refresh {
            self.desktop_refresh();
            return;
        }
        if command == Command::Organize {
            // Dismissing a plan preview must not implicitly approve that stale
            // plan on the next menu invocation.
            self.desktop.organize_plan = None;
        }
        if command == Command::Import && paths.is_empty() {
            paths = platform::pick_files(HWND(self.hwnd_raw as *mut _))
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if paths.is_empty() {
                return;
            }
        }
        // Selection paths must come from this card's current snapshot, never another page.
        if command != Command::Import {
            let key = model::page_key(&card, page_id);
            paths.retain(|path| {
                self.desktop
                    .snapshot
                    .pages
                    .get(&key)
                    .is_some_and(|s| s.rows.iter().any(|r| r.meta.path == *path))
            });
        }
        match command {
            Command::NewFolder
            | Command::Rename
            | Command::Filter
            | Command::Stack
            | Command::Managed
            | Command::Merge
            | Command::AutoOrganize
            | Command::Recycle
            | Command::Import => {
                if matches!(command, Command::Rename) && paths.len() != 1 {
                    return;
                }
                if matches!(command, Command::Recycle) && paths.is_empty()
                    || matches!(command, Command::Stack) && paths.len() < 2
                {
                    return;
                }
                self.desktop_folder_dialog(&card, page_id, command, paths);
            }
            Command::Group | Command::Unstack | Command::MoveEarlier | Command::MoveLater => {
                let current = &mut self.desktop.config.cards[index]
                    .pages
                    .iter_mut()
                    .find(|p| p.id == page_id)
                    .unwrap()
                    .folder;
                let names: Vec<String> = paths
                    .iter()
                    .filter_map(|p| {
                        Path::new(p)
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                    })
                    .collect();
                if command == Command::Group {
                    current.group_by_type = !current.group_by_type;
                } else if command == Command::Unstack {
                    for stack in &mut current.stacks {
                        stack.items.retain(|name| !names.contains(name));
                    }
                    current.stacks.retain(|s| s.items.len() > 1);
                } else {
                    let visible: Vec<String> = self
                        .desktop
                        .snapshot
                        .pages
                        .get(&model::page_key(&card, page_id))
                        .map(|s| {
                            s.rows
                                .iter()
                                .filter_map(|r| {
                                    Path::new(&r.meta.path)
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    current.manual_order = visible;
                    if command == Command::MoveEarlier {
                        for i in 1..current.manual_order.len() {
                            if names.contains(&current.manual_order[i])
                                && !names.contains(&current.manual_order[i - 1])
                            {
                                current.manual_order.swap(i, i - 1);
                            }
                        }
                    } else {
                        for i in (0..current.manual_order.len().saturating_sub(1)).rev() {
                            if names.contains(&current.manual_order[i])
                                && !names.contains(&current.manual_order[i + 1])
                            {
                                current.manual_order.swap(i, i + 1);
                            }
                        }
                    }
                    current.sort = FolderSort::Manual;
                }
                self.desktop_config_changed();
                self.desktop_refresh();
            }
            Command::Split => self.desktop_folder_split(index, page_id),
            _ => self.desktop_folder_work(
                card,
                page_id.into(),
                page.folder,
                command,
                paths,
                String::new(),
                false,
            ),
        }
    }

    fn desktop_folder_dialog(
        &mut self,
        card: &str,
        page: &str,
        command: Command,
        paths: Vec<String>,
    ) {
        let config = self
            .desktop
            .config
            .cards
            .iter()
            .find(|c| c.id == card)
            .and_then(|c| c.pages.iter().find(|p| p.id == page));
        let initial = match command {
            Command::Rename => paths
                .first()
                .and_then(|p| Path::new(p).file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Command::Filter => config.map(|p| p.folder.filter.clone()).unwrap_or_default(),
            Command::Merge => self
                .desktop
                .config
                .cards
                .iter()
                .find(|c| c.id != card && c.pages.iter().all(|p| p.module == Module::Folder))
                .map(|c| c.title.clone())
                .unwrap_or_default(),
            Command::Managed => "收纳文件夹".into(),
            _ => String::new(),
        };
        let (title, description, field_label, button) = match command {
            Command::AutoOrganize => ("自动整理新文件", "开启后只处理随后出现、连续两次检查保持稳定的新文件，按类型移动到当前目录的分类子文件夹；现有文件保持原位。再次确认可关闭。", "", if config.is_some_and(|p|p.folder.auto_organize){"关闭自动整理"}else{"开启自动整理"}),
            Command::Rename => (
                "重命名文件",
                "输入完整名称（包含扩展名）。已存在的名称不会被覆盖。",
                "新名称",
                "重命名",
            ),
            Command::NewFolder => (
                "新建文件夹",
                "在当前浏览的目录中新建一个子文件夹。",
                "文件夹名称",
                "创建",
            ),
            Command::Filter => (
                "筛选文件",
                "按文件名包含的文字筛选，留空显示全部。",
                "名称包含",
                "应用",
            ),
            Command::Stack => (
                "建立叠放",
                "把选中项收在同一显示分组中，文件位置保持原样。",
                "叠放名称",
                "建立",
            ),
            Command::Managed => (
                "创建收纳文件夹",
                "在工作区的桌面收纳目录中创建普通文件夹，并将此格子映射到它。",
                "文件夹名称",
                "创建",
            ),
            Command::Merge => (
                "合并文件格子",
                "输入另一个文件格子的完整标题；其分页将移入当前格子，各文件夹路径保持原样。",
                "来源格子标题",
                "合并",
            ),
            Command::Recycle => (
                "移到回收站",
                "选中的文件及文件夹将进入系统回收站，可在回收站恢复。",
                "",
                "移到回收站",
            ),
            _ => (
                "导入文件",
                "选择复制或移动到当前文件夹。重名项目保留原文件并报告冲突。",
                "",
                "复制到此处",
            ),
        };
        let field = if field_label.is_empty() {
            None
        } else {
            let mut field = TextField::new(field_label);
            field.set_text(&initial);
            Some(field)
        };
        let mut buttons = vec![DialogButton {
            label: "取消".into(),
            kind: ButtonKind::Ghost,
            action: DialogAction::Dismiss,
        }];
        if command == Command::Import {
            buttons.push(DialogButton {
                label: "移动到此处".into(),
                kind: ButtonKind::Ghost,
                action: DialogAction::DesktopFiles {
                    card: card.into(),
                    page: page.into(),
                    command: command.clone(),
                    paths: paths.clone(),
                    move_files: true,
                },
            });
        }
        buttons.push(DialogButton {
            label: button.into(),
            kind: if command == Command::Recycle {
                ButtonKind::Danger
            } else {
                ButtonKind::Primary
            },
            action: DialogAction::DesktopFiles {
                card: card.into(),
                page: page.into(),
                command: command.clone(),
                paths: paths.clone(),
                move_files: false,
            },
        });
        self.desktop_show_main();
        self.dialog = Some(Dialog {
            title: title.into(),
            description: description.into(),
            field,
            error: String::new(),
            note: (!paths.is_empty()).then(|| {
                format!(
                    "共 {} 项\n{}",
                    paths.len(),
                    paths
                        .iter()
                        .take(8)
                        .filter_map(|p| Path::new(p).file_name())
                        .map(|n| n.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
                .into()
            }),
            buttons,
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
        self.invalidate_main();
    }

    pub(in crate::app) fn desktop_folder_confirm(
        &mut self,
        card: &str,
        page: &str,
        command: Command,
        paths: Vec<String>,
        move_files: bool,
    ) {
        let value = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().trim().to_string())
            .unwrap_or_default();
        let Some(index) = self.desktop.config.cards.iter().position(|c| c.id == card) else {
            self.close_dialog();
            return;
        };
        let Some(config) = self.desktop.config.cards[index]
            .pages
            .iter()
            .find(|p| p.id == page)
            .map(|p| p.folder.clone())
        else {
            self.close_dialog();
            return;
        };
        if matches!(
            command,
            Command::Rename
                | Command::NewFolder
                | Command::Stack
                | Command::Managed
                | Command::Merge
        ) && value.is_empty()
        {
            if let Some(d) = &mut self.dialog {
                d.error = "请输入名称".into();
            }
            return;
        }
        if command == Command::Merge {
            let result = self.desktop_folder_merge(index, &value);
            if let Err(error) = result {
                if let Some(d) = &mut self.dialog {
                    d.error = error;
                }
                return;
            }
            self.close_dialog();
            self.desktop_config_changed();
            self.desktop_refresh();
            return;
        }
        if command == Command::AutoOrganize {
            if let Some(p) = self.desktop.config.cards[index]
                .pages
                .iter_mut()
                .find(|p| p.id == page)
            {
                p.folder.auto_organize = !p.folder.auto_organize;
            }
            self.close_dialog();
            self.desktop_config_changed();
            self.desktop_refresh();
            return;
        }
        if matches!(command, Command::Filter | Command::Stack) {
            let mut changed = config;
            if command == Command::Filter {
                changed.filter = value;
            } else {
                let names = paths
                    .iter()
                    .filter_map(|p| {
                        Path::new(p)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .collect::<Vec<_>>();
                for stack in &mut changed.stacks {
                    stack.items.retain(|n| !names.contains(n));
                }
                changed
                    .stacks
                    .retain(|s| s.items.len() > 1 && s.name != value);
                changed.stacks.push(FolderStack {
                    name: value,
                    items: names,
                });
            }
            if let Err(e) = folder::validate(&changed) {
                if let Some(d) = &mut self.dialog {
                    d.error = e.to_string();
                }
                return;
            }
            self.desktop.config.cards[index]
                .pages
                .iter_mut()
                .find(|p| p.id == page)
                .unwrap()
                .folder = changed;
            self.close_dialog();
            self.desktop_config_changed();
            self.desktop_refresh();
            return;
        }
        self.close_dialog();
        self.desktop_folder_work(
            card.into(),
            page.into(),
            config,
            command,
            paths,
            value,
            move_files,
        );
    }

    fn desktop_folder_split(&mut self, index: usize, page: &str) {
        if self.desktop.config.cards.len() >= model::MAX_CARDS
            || self.desktop.config.cards[index].pages.len() < 2
        {
            return;
        }
        let source = &mut self.desktop.config.cards[index];
        let Some(at) = source.pages.iter().position(|p| p.id == page) else {
            return;
        };
        let page = source.pages.remove(at);
        source.active_page = source.pages[0].id.clone();
        let mut card = model::Card::new(&page.title, Module::Folder);
        card.appearance = source.appearance.clone();
        card.x = source.x + 32;
        card.y = source.y + 32;
        card.width = source.width;
        card.height = source.height;
        card.active_page = page.id.clone();
        card.pages = vec![page];
        self.desktop.config.cards.push(card);
        self.desktop_config_changed();
        self.desktop_refresh();
    }

    fn desktop_folder_merge(
        &mut self,
        target: usize,
        title: &str,
    ) -> std::result::Result<(), String> {
        let sources: Vec<_> = self
            .desktop
            .config
            .cards
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                *i != target
                    && c.title == title
                    && c.pages.iter().all(|p| p.module == Module::Folder)
            })
            .map(|(i, _)| i)
            .collect();
        if sources.len() != 1 {
            return Err("请输入唯一的文件格子标题".into());
        }
        let source = sources[0];
        if self.desktop.config.cards[target].pages.len()
            + self.desktop.config.cards[source].pages.len()
            > model::MAX_PAGES_PER_CARD
        {
            return Err("合并后分页数量超过上限".into());
        }
        let pages = self.desktop.config.cards[source].pages.clone();
        self.desktop.config.cards[target].pages.extend(pages);
        self.desktop.config.cards.remove(source);
        Ok(())
    }

    fn desktop_folder_work(
        &mut self,
        card: String,
        page: String,
        config: FolderConfig,
        command: Command,
        paths: Vec<String>,
        value: String,
        move_files: bool,
    ) {
        if self.desktop.file_job.is_some() {
            self.desktop_error("文件操作正在进行".into());
            return;
        }
        let owner = self.hwnd_raw;
        let root = self.desktop.root.clone();
        let organize = self
            .desktop
            .organize_plan
            .take()
            .filter(|(c, p, f, _)| c == &card && p == &page && f == &config)
            .map(|(_, _, _, p)| p);
        let before = config.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawn = std::thread::Builder::new()
            .name("desktop-file-operation".into())
            .spawn(move || {
                use windows::Win32::System::Com::*;
                let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
                let result = match FILE_OPERATIONS.lock() {
                    Ok(_file_operation_guard) => run(
                        HWND(owner as *mut _),
                        root.as_deref(),
                        config,
                        command,
                        paths,
                        value,
                        move_files,
                        organize,
                    )
                    .map_err(|e| format!("{e:#}")),
                    Err(_) => Err("文件操作队列状态异常，当前操作未执行".into()),
                };
                if initialized {
                    unsafe {
                        CoUninitialize();
                    }
                }
                let _ = tx.send(result);
                if owner != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(owner as *mut _)),
                            desktop_window::MESSAGE,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            });
        match spawn {
            Ok(_) => {
                self.desktop.file_job = Some(FileJob {
                    epoch: self.desktop.epoch,
                    card,
                    page,
                    config: before,
                    rx,
                })
            }
            Err(e) => self.desktop_error(e.to_string()),
        }
        self.desktop_timer_state();
    }

    pub(super) fn desktop_take_file_result(&mut self) {
        let Some(job) = &self.desktop.file_job else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(v) => v,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("文件操作线程已停止".into()),
        };
        let FileJob {
            epoch,
            card,
            page,
            config,
            ..
        } = self.desktop.file_job.take().unwrap();
        if epoch != self.desktop.epoch {
            match result {
                Ok(outcome) if outcome.organize.is_some() => {
                    self.show_global_notice("工作区已切换，旧工作区的文件整理预览已丢弃");
                }
                Ok(outcome) if outcome.folder.is_some() => {
                    self.show_global_notice("文件夹操作在旧工作区完成；当前映射视图未更新");
                }
                Ok(outcome) if !outcome.message.is_empty() => {
                    self.show_global_notice(format!("旧工作区的文件操作结果：{}", outcome.message))
                }
                Ok(_) => {}
                Err(error) => self.show_global_notice(format!("旧工作区文件操作失败：{error}")),
            }
            self.desktop_refresh();
            self.invalidate_main();
            return;
        }
        match result {
            Ok(outcome) => {
                if let Some(folder) = outcome.folder {
                    if let Some(p) = self
                        .desktop
                        .config
                        .cards
                        .iter_mut()
                        .find(|c| c.id == card)
                        .and_then(|c| {
                            c.pages
                                .iter_mut()
                                .find(|p| p.id == page && p.folder == config)
                        })
                    {
                        p.folder = folder;
                        self.desktop_config_changed();
                    }
                }
                if let Some(plan) = outcome.organize {
                    self.desktop.organize_plan = Some((card.clone(), page.clone(), config, plan));
                    self.desktop_show_main();
                    self.dialog = Some(Dialog {
                        title: "按类型整理文件".into(),
                        description: "确认后将按预览移动文件。现有同名目标不会被覆盖。".into(),
                        field: None,
                        error: String::new(),
                        note: Some(outcome.message.into()),
                        buttons: vec![
                            DialogButton {
                                label: "取消".into(),
                                kind: ButtonKind::Ghost,
                                action: DialogAction::Dismiss,
                            },
                            DialogButton {
                                label: "执行整理".into(),
                                kind: ButtonKind::Primary,
                                action: DialogAction::DesktopFiles {
                                    card,
                                    page,
                                    command: Command::Organize,
                                    paths: vec![],
                                    move_files: false,
                                },
                            },
                        ],
                        dismiss: DialogAction::Dismiss,
                        hover: None,
                    });
                    self.focus = Focus::Dialog;
                } else if !outcome.message.is_empty() {
                    self.show_global_notice(outcome.message);
                }
                self.desktop_refresh();
            }
            Err(error) => self.desktop_error(error),
        }
        self.invalidate_main();
    }
}

fn run(
    owner: HWND,
    workspace: Option<&Path>,
    config: FolderConfig,
    command: Command,
    paths: Vec<String>,
    value: String,
    move_files: bool,
    organize: Option<files::OrganizePlan>,
) -> anyhow::Result<Outcome> {
    if command == Command::Root {
        return Ok(Outcome::folder(folder::root(&config)?));
    }
    if command == Command::Up {
        return Ok(Outcome::folder(folder::parent(&config)?));
    }
    if command == Command::Managed {
        let root = workspace
            .ok_or_else(|| anyhow::anyhow!("请先打开工作区"))?
            .join(".mochi")
            .join("desktop-files");
        std::fs::create_dir_all(&root)?;
        let base = FolderConfig {
            path: root.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let path = files::create_subdirectory(&base, &value)?;
        return Ok(Outcome::folder(FolderConfig {
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        }));
    }
    if command == Command::Import || command == Command::Paste {
        let (sources, moving) = if command == Command::Paste {
            let data = shell_files::read_file_clipboard()?
                .ok_or_else(|| anyhow::anyhow!("剪贴板中没有文件"))?;
            (data.paths, data.effect == ClipboardEffect::Move)
        } else {
            (paths.into_iter().map(PathBuf::from).collect(), move_files)
        };
        let plan = files::plan_transfer(
            &config,
            &sources,
            if moving {
                files::TransferKind::Move
            } else {
                files::TransferKind::Copy
            },
            files::TransferLimits::default(),
        )?;
        let results = shell_files::execute_transfer(owner, &config, &plan);
        if command == Command::Paste
            && moving
            && !results.is_empty()
            && results.iter().all(|r| r.succeeded)
        {
            let _ = shell_files::clear_file_clipboard_if_matches(&shell_files::FileClipboard {
                paths: sources,
                effect: ClipboardEffect::Move,
                owned_by_mochi: true,
            });
        }
        return Ok(Outcome::message(summarize(&results)));
    }
    if command == Command::Organize {
        if let Some(plan) = organize {
            return Ok(Outcome::message(summarize(&files::apply_organize(
                &config, &plan,
            ))));
        }
        let plan = files::plan_organize(&config)?;
        let preview = plan
            .actions
            .iter()
            .take(60)
            .map(|item| format!("{} → {}", item.source.display(), item.target.display()))
            .collect::<Vec<_>>()
            .join("\n");
        if plan.actions.is_empty() {
            if !plan.failures.is_empty() || plan.truncated {
                let mut report = summarize(&plan.failures);
                if plan.truncated {
                    report.push_str("\n扫描达到项目上限，未检查剩余内容");
                }
                return Ok(Outcome::message(report));
            }
            anyhow::bail!("当前文件夹没有可整理的项目");
        }
        let skipped = if plan.failures.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n跳过 {} 项：\n{}",
                plan.failures.len(),
                summarize(&plan.failures)
            )
        };
        let truncated = if plan.truncated {
            "\n\n扫描达到项目上限，未检查剩余内容"
        } else {
            ""
        };
        return Ok(Outcome {
            message: format!(
                "共 {} 项\n{preview}{skipped}{truncated}",
                plan.actions.len()
            ),
            folder: None,
            organize: Some(plan),
        });
    }
    if command == Command::NewFolder {
        let path = files::create_subdirectory(&config, &value)?;
        return Ok(Outcome::message(format!(
            "已创建 {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    // Resolve multi-selection operations item by item. A stale row should be
    // reported beside that item without preventing valid siblings from opening,
    // being revealed, or entering the recycle bin.
    if command == Command::Open {
        anyhow::ensure!(!paths.is_empty(), "请先选择文件");
        anyhow::ensure!(paths.len() <= 16, "一次最多打开 16 项");
        let mut resolved = Vec::new();
        let mut failed = Vec::new();
        for source in &paths {
            match folder::resolve_entry(&config, source) {
                Ok(path) => resolved.push(path),
                Err(error) => failed.push(format!("{source}：{error:#}")),
            }
        }
        if paths.len() == 1 {
            if let Some(path) = resolved.first().filter(|path| path.is_dir()) {
                return Ok(Outcome::folder(folder::navigate(
                    &config,
                    &path.to_string_lossy(),
                )?));
            }
        }
        let mut opened = 0;
        for path in &resolved {
            if platform::open_external(&path.to_string_lossy()) {
                opened += 1;
            } else {
                failed.push(format!("{}：无法打开", path.display()));
            }
        }
        let mut message = format!("已打开 {opened}/{} 项", paths.len());
        append_item_failures(&mut message, &failed);
        return Ok(Outcome::message(message));
    }
    if command == Command::Recycle {
        anyhow::ensure!(!paths.is_empty(), "请先选择文件");
        let mut done = 0;
        let mut failed = Vec::new();
        let mut cancelled = false;
        for (index, source) in paths.iter().enumerate() {
            let path = match folder::resolve_entry(&config, source) {
                Ok(path) => path,
                Err(error) => {
                    failed.push(format!("{source}：{error:#}"));
                    continue;
                }
            };
            match shell_files::move_to_trash(owner, &path) {
                Ok(()) if !path.exists() => done += 1,
                Ok(()) => failed.push(format!("{}：回收操作结束后来源仍存在", path.display())),
                Err(error) => {
                    cancelled = shell_files::is_shell_cancelled(&error);
                    failed.push(format!("{}：{error:#}", path.display()));
                    if cancelled {
                        failed.extend(
                            paths
                                .iter()
                                .skip(index + 1)
                                .map(|pending| format!("{pending}：已取消，未处理")),
                        );
                        break;
                    }
                }
            }
        }
        let mut message = format!("已移到回收站 {done}/{} 项", paths.len());
        if cancelled {
            message.push_str("（操作已取消）");
        }
        append_item_failures(&mut message, &failed);
        return Ok(Outcome::message(message));
    }
    if command == Command::Preview {
        let source = paths
            .first()
            .ok_or_else(|| anyhow::anyhow!("请先选择文件"))?;
        let path = folder::resolve_entry(&config, source)?;
        anyhow::ensure!(
            shell_files::preview_if_quicklook_running(&path)?,
            "QuickLook 当前不可用，请先启动 QuickLook；也可按 Enter 使用默认程序打开"
        );
        return Ok(Outcome::message("已请求 QuickLook 预览"));
    }
    if command == Command::Reveal {
        anyhow::ensure!(!paths.is_empty(), "请先选择文件");
        let processed = paths.len().min(8);
        let mut revealed = 0;
        let mut failed = Vec::new();
        for source in paths.iter().take(processed) {
            match folder::resolve_entry(&config, source) {
                Ok(path) => {
                    shell_files::reveal(&path);
                    revealed += 1;
                }
                Err(error) => failed.push(format!("{source}：{error:#}")),
            }
        }
        let mut message = format!("已在资源管理器中定位 {revealed}/{processed} 项");
        if paths.len() > processed {
            message.push_str(&format!("（另有 {} 项未处理）", paths.len() - processed));
        }
        append_item_failures(&mut message, &failed);
        return Ok(Outcome::message(message));
    }
    let resolved = paths
        .iter()
        .map(|path| folder::resolve_entry(&config, path))
        .collect::<anyhow::Result<Vec<_>>>()?;
    anyhow::ensure!(!resolved.is_empty(), "请先选择文件");
    match command {
        Command::Copy | Command::Cut => shell_files::set_file_clipboard(
            &resolved,
            if command == Command::Cut {
                ClipboardEffect::Move
            } else {
                ClipboardEffect::Copy
            },
        )?,
        Command::CopyPath => {
            anyhow::ensure!(
                platform::copy_to_clipboard(
                    &resolved
                        .iter()
                        .map(|p| p.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("\r\n")
                ),
                "无法写入剪贴板"
            );
        }
        Command::Rename => {
            files::rename_entry(&config, &resolved[0].to_string_lossy(), &value)?;
        }
        _ => {}
    }
    Ok(Outcome::message(
        if matches!(command, Command::Copy | Command::Cut) {
            "已写入文件剪贴板"
        } else {
            "操作完成"
        },
    ))
}

fn append_item_failures(message: &mut String, failed: &[String]) {
    if failed.is_empty() {
        return;
    }
    message.push_str(&format!("\n失败 {} 项", failed.len()));
    for item in failed.iter().take(12) {
        message.push_str("\n");
        message.push_str(item);
    }
    if failed.len() > 12 {
        message.push_str(&format!("\n另有 {} 项未展开", failed.len() - 12));
    }
}

fn summarize(results: &[files::OperationResult]) -> String {
    let done = results.iter().filter(|r| r.succeeded).count();
    let failed_count = results.iter().filter(|r| !r.succeeded).count();
    let failed = results
        .iter()
        .filter_map(|r| {
            r.error
                .as_ref()
                .map(|e| format!("{}：{e}", r.source.display()))
        })
        .take(12)
        .collect::<Vec<_>>();
    let mut summary = format!(
        "完成 {done}/{} 项{}",
        results.len(),
        if failed.is_empty() {
            String::new()
        } else {
            format!("\n{}", failed.join("\n"))
        }
    );
    if failed_count > failed.len() {
        summary.push_str(&format!("\n另有 {} 项失败", failed_count - failed.len()));
    }
    summary
}
