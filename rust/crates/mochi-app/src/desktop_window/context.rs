//! 处理桌面卡片窗口中的颜色选择和上下文菜单。
use super::*;
use mochi_core::desktop_cards::{item_key, ItemStyle, Module};

pub(super) fn choose_color(hwnd: HWND, current: u32) -> Option<u32> {
    use windows::Win32::UI::Controls::Dialogs::*;
    let swap = |rgb: u32| (rgb & 0xff00) | ((rgb >> 16) & 0xff) | ((rgb & 0xff) << 16);
    let mut colors = [windows::Win32::Foundation::COLORREF(0xffffff); 16];
    let mut picker = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: hwnd,
        rgbResult: windows::Win32::Foundation::COLORREF(swap(current)),
        lpCustColors: colors.as_mut_ptr(),
        Flags: CC_FULLOPEN | CC_RGBINIT,
        ..Default::default()
    };
    unsafe {
        ChooseColorW(&mut picker)
            .as_bool()
            .then(|| swap(picker.rgbResult.0))
    }
}
pub(super) fn menu(hwnd: HWND) {
    unsafe {
        let Some(s) = state(hwnd) else { return };
        if s.context_active {
            return;
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mut local = pt;
        let _ = ScreenToClient(hwnd, &mut local);
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let hit = painting::hit(
            s,
            area(hwnd),
            local.x as f32 / scale,
            local.y as f32 / scale,
        );
        if folder::menu(hwnd, hit.as_ref(), pt) { return; }
        let page = if let Some(Hit::Page(id)) = &hit {
            id.clone()
        } else {
            s.view.page_id.clone()
        };
        let item = match &hit {
            Some(Hit::Row(id, _)) => s
                .view
                .rows
                .iter()
                .find(|r| &r.id == id)
                .map(|r| item_key(id, &r.meta.path)),
            Some(Hit::Widget(id, _)) => Some(format!("node:{id}")),
            Some(Hit::WidgetRow(node, id)) => s
                .view
                .widget_rows
                .get(node)
                .and_then(|rows| rows.iter().find(|r| &r.id == id))
                .map(|r| item_key(id, &r.meta.path)),
            Some(Hit::TreeOpen(path) | Hit::TreeToggle(path)) => {
                Some(tree::style_key(&s.view, path))
            }
            _ if matches!(s.view.module, Some(Module::Ai | Module::Pomodoro)) => {
                Some("page".into())
            }
            _ => None,
        };
        let shortcut = if s.view.module == Some(Module::Shortcuts) {
            if let Some(Hit::Widget(id, _)) = &hit {
                Some(id.clone())
            } else {
                None
            }
        } else {
            None
        };
        let mut style = item
            .as_ref()
            .and_then(|key| s.view.item_styles.get(key))
            .cloned()
            .unwrap_or_default();
        let palette = painting::palette(&s.spec);
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, 1, w!("设置此分页"));
        if s.view.module == Some(Module::Folder) {
            let _ = AppendMenuW(menu, MF_STRING, 15, w!("刷新文件夹"));
            let _ = AppendMenuW(menu, MF_STRING, 4, w!("打开映射文件夹"));
        }
        if item.is_some() {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, 10, w!("文字颜色…"));
            let _ = AppendMenuW(menu, MF_STRING, 11, w!("背景颜色…"));
            let _ = AppendMenuW(menu, MF_STRING, 12, w!("恢复分页默认颜色"));
        }
        if shortcut.is_some() {
            let _ = AppendMenuW(menu, MF_STRING, 13, w!("删除快捷引用"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            2,
            if s.spec.locked {
                w!("解锁")
            } else {
                w!("锁定")
            },
        );
        let _ = AppendMenuW(menu, MF_STRING, 3, w!("隐藏"));
        if s.spec.appearance.edge_dock {
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                6,
                if s.dock.suspended {
                    w!("恢复吸附")
                } else {
                    w!("停止吸附")
                },
            );
        }
        if s.view.module == Some(Module::Schedule) {
            let _ = AppendMenuW(menu, MF_STRING, 4, w!("进入日程"));
            if s.view.presentation.schedule_view == 1 {
                let _ = AppendMenuW(
                    menu,
                    MF_STRING,
                    5,
                    if s.view.presentation.calendar_expanded {
                        w!("收纳每周日程")
                    } else {
                        w!("展开全部日程")
                    },
                );
            }
        }
        if tree::enabled(&s.view) {
            let _ = AppendMenuW(menu, MF_STRING, 14, w!("刷新目录树"));
        }
        // `TrackPopupMenu` 会处理各类消息，包括前台通知和
        // 刷新计时器。模态循环期间，不要让这些消息改变所属窗口的位置。
        s.context_active = true;
        s.dock.leave_at = None;
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        )
        .0;
        let _ = DestroyMenu(menu);
        // 原生颜色对话框接管指针操作时，保持抽屉打开。
        match command {
            1 => emit(hwnd, EventKind::ManagePage(page.clone())),
            2 => activate(hwnd, Hit::Lock),
            3 => activate(hwnd, Hit::Hide),
            4 => activate(hwnd, Hit::Open),
            5 => activate(hwnd, Hit::CalendarExpanded),
            6 => dock::toggle_suspended(hwnd),
            15 => emit(hwnd, EventKind::Refresh),
            10 | 11 | 12 => {
                if let Some(item) = item {
                    let changed = if command == 12 {
                        style = ItemStyle::default();
                        true
                    } else if let Some(color) = choose_color(
                        hwnd,
                        if command == 11 {
                            style.background.unwrap_or(palette.surface)
                        } else {
                            style.foreground.unwrap_or(palette.foreground)
                        },
                    ) {
                        if command == 11 {
                            style.background = Some(color)
                        } else {
                            style.foreground = Some(color)
                        }
                        true
                    } else {
                        false
                    };
                    if changed {
                        emit(hwnd, EventKind::ItemStyle { page, item, style });
                    }
                }
            }
            13 => {
                if let Some(id) = shortcut {
                    emit(hwnd, EventKind::ShortcutDelete(id));
                }
            }
            14 => {
                if let Some(s) = state(hwnd) {
                    s.tree = Default::default();
                    tree::sync(s);
                    invalidate(hwnd);
                }
            }
            _ => {}
        }
        if let Some(s) = state(hwnd) {
            s.context_active = false;
            s.dock.leave_at = None;
        }
        dock::configure(hwnd);
        if let Some(s) = state(hwnd) {
            let _ = PostMessageW(Some(s.owner), MESSAGE, WPARAM(0), LPARAM(0));
        }
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    }
}
