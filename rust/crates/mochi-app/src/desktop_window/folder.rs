//! File-card selection and commands. Filesystem work belongs to the application worker.
use super::*;
use mochi_core::desktop_cards::Module;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    AutoOrganize,
    Open,
    Reveal,
    Copy,
    Cut,
    Paste,
    Recycle,
    Rename,
    NewFolder,
    Import,
    Organize,
    Filter,
    Group,
    Stack,
    Unstack,
    MoveEarlier,
    MoveLater,
    Up,
    Root,
    Preview,
    CopyPath,
    Managed,
    Split,
    Merge,
    Refresh,
}

pub(super) fn enabled(view: &View) -> bool {
    view.module == Some(Module::Folder)
}

pub(super) fn send(hwnd: HWND, command: Command) {
    unsafe {
        let Some(s) = state(hwnd).filter(|s| enabled(&s.view)) else {
            return;
        };
        let paths = s
            .view
            .rows
            .iter()
            .filter(|row| s.view.folder_selected.contains(&row.id) && !row.meta.path.is_empty())
            .map(|row| row.meta.path.clone())
            .collect();
        emit(
            hwnd,
            EventKind::Folder {
                page: s.view.page_id.clone(),
                command,
                paths,
            },
        );
    }
}

pub(super) fn select(view: &mut View, anchor: Option<&str>, id: &str, ctrl: bool, shift: bool) {
    if shift {
        let start = anchor.and_then(|anchor| view.rows.iter().position(|r| r.id == anchor));
        let end = view.rows.iter().position(|r| r.id == id);
        if let (Some(start), Some(end)) = (start, end) {
            if !ctrl {
                view.folder_selected.clear();
            }
            for row in &view.rows[start.min(end)..=start.max(end)] {
                view.folder_selected.insert(row.id.clone());
            }
            return;
        }
    }
    if ctrl {
        if !view.folder_selected.remove(id) {
            view.folder_selected.insert(id.into());
        }
    } else {
        view.folder_selected.clear();
        view.folder_selected.insert(id.into());
    }
}

pub(super) fn pointer_up(hwnd: HWND, hit: &Hit) -> bool {
    unsafe {
        let Some(s) = state(hwnd).filter(|s| enabled(&s.view)) else {
            return false;
        };
        let Hit::Row(id, _) = hit else { return false };
        let shift = GetKeyState(0x10) < 0;
        let has_anchor = s
            .folder_anchor
            .as_deref()
            .is_some_and(|anchor| s.view.rows.iter().any(|row| row.id == anchor));
        select(
            &mut s.view,
            (shift && has_anchor)
                .then(|| s.folder_anchor.as_deref())
                .flatten(),
            id,
            GetKeyState(0x11) < 0,
            shift,
        );
        if !shift || !has_anchor {
            s.folder_anchor = Some(id.clone());
        }
        s.focused = Some(hit.clone());
        invalidate(hwnd);
        true
    }
}

pub(super) fn key(hwnd: HWND, key: usize) -> bool {
    unsafe {
        let Some(s) = state(hwnd).filter(|s| enabled(&s.view)) else {
            return false;
        };
        let ctrl = GetKeyState(0x11) < 0;
        let shift = GetKeyState(0x10) < 0;
        if ctrl && key == 9 && !s.view.tabs.is_empty() {
            let at = s
                .view
                .tabs
                .iter()
                .position(|(id, _)| id == &s.view.page_id)
                .unwrap_or(0);
            let next = if shift {
                (at + s.view.tabs.len() - 1) % s.view.tabs.len()
            } else {
                (at + 1) % s.view.tabs.len()
            };
            emit(hwnd, EventKind::Page(s.view.tabs[next].0.clone()));
            return true;
        }
        let command = match (key, ctrl) {
            (0x43, true) => Some(Command::Copy),
            (0x58, true) => Some(Command::Cut),
            (0x56, true) => Some(Command::Paste),
            (0x46, true) => Some(Command::Filter),
            (0x4e, true) if shift => Some(Command::NewFolder),
            (0x71, false) => Some(Command::Rename),
            (0x74, false) => Some(Command::Refresh),
            (0x2e, false) => Some(Command::Recycle),
            (13, false) => Some(Command::Open),
            (32, false) => Some(Command::Preview),
            (8, false) => Some(Command::Up),
            _ => None,
        };
        if let Some(command) = command {
            send(hwnd, command);
            return true;
        }
        if ctrl && key == 0x41 {
            s.view.folder_selected = s.view.rows.iter().map(|r| r.id.clone()).collect();
            s.folder_anchor = s
                .focused
                .as_ref()
                .and_then(|h| match h {
                    Hit::Row(id, _) if s.view.folder_selected.contains(id) => Some(id.clone()),
                    _ => None,
                })
                .or_else(|| s.view.rows.first().map(|row| row.id.clone()));
        } else if key == 27 {
            s.view.folder_selected.clear();
            s.folder_anchor = None;
            s.focused = None;
        } else if matches!(key, 35..=40) && !s.view.rows.is_empty() {
            let at = s
                .focused
                .as_ref()
                .and_then(|h| match h {
                    Hit::Row(id, _) => s.view.rows.iter().position(|r| &r.id == id),
                    _ => None,
                })
                .unwrap_or(0);
            let columns = if s.view.presentation.grid {
                s.view.presentation.columns.max(1) as usize
            } else {
                1
            };
            let next = match key {
                35 => s.view.rows.len() - 1,
                36 => 0,
                37 => at.saturating_sub(1),
                38 => at.saturating_sub(columns),
                39 => (at + 1).min(s.view.rows.len() - 1),
                _ => (at + columns).min(s.view.rows.len() - 1),
            };
            let id = s.view.rows[next].id.clone();
            let has_anchor = s
                .folder_anchor
                .as_deref()
                .is_some_and(|anchor| s.view.rows.iter().any(|row| row.id == anchor));
            select(
                &mut s.view,
                (shift && has_anchor)
                    .then(|| s.folder_anchor.as_deref())
                    .flatten(),
                &id,
                ctrl,
                shift,
            );
            if !shift || !has_anchor {
                s.folder_anchor = Some(id.clone());
            }
            s.focused = Some(Hit::Row(id, false));
            let step = if s.view.presentation.grid {
                s.view.presentation.grid_height.max(64) as usize
            } else {
                s.spec.appearance.font_size as usize
                    + 8
                    + 2 * s.spec.appearance.row_padding as usize
                    + if s.spec.appearance.show_details {
                        22
                    } else {
                        0
                    }
            };
            let top = next / columns * step;
            let available = (painting::content_rect(&s.spec, &s.view, area(hwnd)).height() - 32.0)
                .max(1.0) as usize;
            if top < s.offset {
                s.offset = top;
            } else if top + step > s.offset + available {
                s.offset = top + step - available;
            }
            s.offset = s
                .offset
                .min(painting::scroll_max(&s.spec, &s.view, area(hwnd)));
        } else {
            return false;
        }
        invalidate(hwnd);
        true
    }
}

pub(super) fn pointer_down(hwnd: HWND, x: f32, y: f32) {
    unsafe {
        if let Some(s) = state(hwnd).filter(|s| enabled(&s.view)) {
            s.folder_drag =
                matches!(painting::hit(s, area(hwnd), x, y), Some(Hit::Row(..))).then_some((x, y));
        }
    }
}

pub(super) fn pointer_move(hwnd: HWND, x: f32, y: f32) -> bool {
    unsafe {
        let Some(s) = state(hwnd).filter(|s| enabled(&s.view)) else {
            return false;
        };
        let Some((start_x, start_y)) = s.folder_drag else {
            return false;
        };
        if (x - start_x).abs().max((y - start_y).abs()) < 7.0 {
            return false;
        }
        s.folder_drag = None;
        if let Some(Hit::Row(id, _)) = s.pressed.as_ref() {
            if !s.view.folder_selected.contains(id) {
                s.view.folder_selected.clear();
                s.view.folder_selected.insert(id.clone());
                s.folder_anchor = Some(id.clone());
            }
        }
        s.pressed = None;
        let paths: Vec<std::path::PathBuf> = s
            .view
            .rows
            .iter()
            .filter(|r| s.view.folder_selected.contains(&r.id))
            .map(|r| r.meta.path.clone().into())
            .collect();
        let _ = ReleaseCapture();
        // The Shell data object supplies the same file formats as Explorer.
        if !paths.is_empty() {
            let _ = platform::desktop_files::start_drag(hwnd, &paths);
        }
        emit(hwnd, EventKind::Refresh);
        invalidate(hwnd);
        true
    }
}

pub(super) fn menu(hwnd: HWND, hit: Option<&Hit>, point: POINT) -> bool {
    unsafe {
        let Some(s) = state(hwnd).filter(|s| enabled(&s.view)) else {
            return false;
        };
        if let Some(Hit::Row(id, _)) = hit {
            if !s.view.folder_selected.contains(id) {
                s.view.folder_selected.clear();
                s.view.folder_selected.insert(id.clone());
                s.folder_anchor = Some(id.clone());
            }
            s.focused = hit.cloned();
        }
        let has = !s.view.folder_selected.is_empty();
        let Ok(menu) = CreatePopupMenu() else {
            return true;
        };
        let items = [
            ("打开\tEnter", Command::Open, has),
            ("在资源管理器中显示", Command::Reveal, has),
            ("QuickLook 预览\tSpace", Command::Preview, has),
            ("复制\tCtrl+C", Command::Copy, has),
            ("剪切\tCtrl+X", Command::Cut, has),
            ("粘贴\tCtrl+V", Command::Paste, true),
            ("复制路径", Command::CopyPath, has),
            (
                "重命名…\tF2",
                Command::Rename,
                s.view.folder_selected.len() == 1,
            ),
            ("移到回收站…\tDelete", Command::Recycle, has),
            ("新建文件夹…", Command::NewFolder, true),
            ("导入文件…", Command::Import, true),
            ("按类型整理文件…", Command::Organize, true),
            ("自动整理新文件…", Command::AutoOrganize, true),
            ("筛选名称…\tCtrl+F", Command::Filter, true),
            ("按类型叠放 / 取消", Command::Group, true),
            (
                "把选中项叠放…",
                Command::Stack,
                s.view.folder_selected.len() > 1,
            ),
            ("移出手动叠放", Command::Unstack, has),
            ("顺序向前", Command::MoveEarlier, has),
            ("顺序向后", Command::MoveLater, has),
            ("返回上层\tBackspace", Command::Up, true),
            ("返回映射根目录", Command::Root, true),
            ("创建收纳文件夹…", Command::Managed, true),
            ("合并文件格子…", Command::Merge, true),
            (
                "将此分页拆成独立格子",
                Command::Split,
                s.view.tabs.len() > 1,
            ),
            ("刷新\tF5", Command::Refresh, true),
        ];
        for (i, (label, _, available)) in items.iter().enumerate() {
            let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(
                menu,
                MF_STRING | if *available { MF_ENABLED } else { MF_GRAYED },
                100 + i,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 200, windows::core::w!("设置此分页"));
        let _ = AppendMenuW(menu, MF_STRING, 201, windows::core::w!("隐藏格子"));
        let _ = AppendMenuW(menu, MF_STRING, 202, windows::core::w!("锁定 / 解锁"));
        let _ = AppendMenuW(menu, MF_STRING, 203, windows::core::w!("收起 / 展开胶囊"));
        s.context_active = true;
        s.dock.leave_at = None;
        let _ = SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = DestroyMenu(menu);
        if let Some(s) = state(hwnd) {
            s.context_active = false;
            s.dock.leave_at = None;
        }
        if let Some((_, command, _)) = selected.checked_sub(100).and_then(|i| items.get(i)) {
            send(hwnd, command.clone());
        }
        if selected == 200 {
            if let Some(s) = state(hwnd) {
                emit(hwnd, EventKind::ManagePage(s.view.page_id.clone()));
            }
        }
        match selected {
            201 => emit(hwnd, EventKind::Hide),
            202 => emit(hwnd, EventKind::Lock),
            203 => emit(hwnd, EventKind::Capsule),
            _ => {}
        }
        dock::configure(hwnd);
        invalidate(hwnd);
        true
    }
}
