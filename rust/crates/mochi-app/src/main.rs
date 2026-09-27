//! Win32 消息入口；状态和输入交给 App，绘制交给 gfx。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(
    dead_code,
    clippy::large_enum_variant,
    clippy::too_many_arguments,
    clippy::type_complexity
)]
#![cfg_attr(
    test,
    allow(clippy::field_reassign_with_default, clippy::items_after_test_module)
)]

mod ai_runtime;
mod app;
mod backlinks_runtime;
mod capture_window;
mod desktop_tray;
mod desktop_window;
mod export_requests;
mod file_runtime;
mod follow_up_runtime;
mod gfx;
mod legacy;
mod memory_runtime;
mod pdf;
mod platform;
mod shell;
mod ui;
mod view;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::Ime::{GCS_COMPSTR, GCS_RESULTSTR, ISC_SHOWUICOMPOSITIONWINDOW};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_CONTROL, VK_PROCESSKEY, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::NCCALCSIZE_PARAMS;
use windows::Win32::UI::WindowsAndMessaging::*;

use app::{App, CaptionAction};
use platform::{
    caption_buttons, client_size_dips, cursor_pos_dips, dpi_for_window, frame_border,
    hi_word_signed, lo_word_signed, mouse_dips, screen_to_client_dips, LOGICAL_HEIGHT,
    LOGICAL_WIDTH, WM_APP_AI_EVENT, WM_APP_FILES_CHANGED, WM_APP_HOME_READY, WM_APP_INDEX_READY,
    WM_APP_PDF_EVENT, WM_APP_SEARCH_READY, WM_APP_UPDATE_DOWNLOAD_READY, WM_APP_UPDATE_READY,
};
use ui::layout::{to_dips, Rect};

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--install-bundled-guides") {
        let settings = mochi_core::settings::SettingsService::new(None);
        if let Some(root) = settings.get("workspace.lastPath") {
            let root = std::path::Path::new(&root);
            if root.is_dir() {
                mochi_core::bundled_guides::install(root, true).map_err(|e| {
                    windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
                })?;
            }
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--connect-web-clipper") {
        if !connect_web_clipper() {
            return Ok(());
        }
    }
    if std::env::args().any(|arg| arg == "--register-web-clipper") {
        return mochi_core::web_clipper::native::register(
            &mochi_core::settings::SettingsService::new(None),
        )
        .map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()));
    }
    unsafe {
        // 必须在创建任何窗口前设置，否则高 DPI 下会被系统位图拉伸糊掉
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // 文件对话框要 STA
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let instance = unsafe { GetModuleHandleW(None)? };
    #[cfg(debug_assertions)]
    {
        let args = std::env::args().collect::<Vec<_>>();
        if args.get(1).is_some_and(|a| a == "--export-pdf") {
            let export = || -> anyhow::Result<()> {
                if args.len() != 4 {
                    anyhow::bail!("用法：--export-pdf <输入.md 或 :verification:> <输出.pdf>")
                }
                if args[2] == ":print-verification:" {
                    let mut source = String::from("# 打印分页验证\n\n");
                    for i in 0..39 {
                        source.push_str(&format!("填充段落 FILL{i:03}。\n\n"));
                    }
                    source.push_str("```rust\n");
                    for i in 0..6 {
                        source.push_str(&format!("let CODE{i:03} = {i};\n"));
                    }
                    source.push_str("```\n\n| 表格编号 | 内容 |\n| - | - |\n");
                    for i in 0..18 {
                        source.push_str(&format!("| TABLE{i:03} | 完整表格行 |\n"));
                    }
                    source.push_str("\n> QUOTEBEGIN ");
                    source.push_str(&"引用内容保持整体分页。".repeat(50));
                    source.push_str("QUOTEEND\n\n```text\n");
                    for i in 0..80 {
                        source.push_str(&format!("LONG{i:03} 长代码块续页背景。\n"));
                    }
                    source.push_str("```\n\nPRINTEND\n");
                    return gfx::export_pdf(&source, std::path::Path::new(&args[3]), None);
                }
                if args[2] == ":verification:" {
                    let mut source =
                        include_str!("../tests/fixtures/pdf-verification.md").to_owned();
                    for i in 0..100 {
                        source.push_str(&format!(
                            "\n\n分页编号 ROW{i:03} 中文完整行，跨页不能重复或遗漏。"
                        ));
                    }
                    source.push_str("\n\n字符映射子集：");
                    for i in 0..300 {
                        source.push(char::from_u32(0x4e00 + i).unwrap());
                    }
                    return gfx::export_pdf(&source, std::path::Path::new(&args[3]), None);
                }
                let source = std::path::Path::new(&args[2]);
                gfx::export_pdf(
                    &std::fs::read_to_string(source)?,
                    std::path::Path::new(&args[3]),
                    source.parent(),
                )
            };
            return export().map_err(|e| {
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
            });
        }
        if args.get(1).is_some_and(|a| a == "--snapshot") {
            let scenario = args.get(2).map(String::as_str).unwrap_or("document");
            let path = args
                .get(3)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("snapshot.png"));
            if scenario.starts_with("capture-hub") {
                return capture_window::snapshot(scenario, &path);
            }
            if scenario.starts_with("desktop-manager") {
                return App::desktop_snapshot(scenario, &path);
            }
            if scenario.starts_with("desktop-card") {
                return desktop_window::snapshot(scenario, &path);
            }
            return App::snapshot(scenario, &path);
        }
    }
    let login_launch = std::env::args().any(|a| a == "--autostart");
    let _instance = if !cfg!(debug_assertions) && !desktop_tray::isolated() {
        match desktop_tray::acquire(login_launch)? {
            Some(guard) => Some(guard),
            None => return Ok(()),
        }
    } else {
        None
    };
    let class_name = w!("MochiNativeWindow");

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        hIcon: unsafe {
            LoadIconW(Some(instance.into()), PCWSTR(101usize as *const u16))
                .or_else(|_| LoadIconW(None, IDI_APPLICATION))?
        },
        lpszClassName: class_name,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err(windows::core::Error::from_thread());
    }

    let app = Box::new(App::new()?);
    // 验收实例仍走真实 HWND/消息循环，但不覆盖用户桌面或抢焦点。
    let offscreen = std::env::var_os("MOCHI_VERIFY_OFFSCREEN").is_some();

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("墨池"),
            WS_OVERLAPPEDWINDOW,
            if offscreen { -32000 } else { CW_USEDEFAULT },
            if offscreen { -32000 } else { CW_USEDEFAULT },
            LOGICAL_WIDTH,
            LOGICAL_HEIGHT,
            None,
            None,
            Some(instance.into()),
            Some(Box::into_raw(app) as *const _),
        )?
    };

    unsafe {
        let _ = ShowWindow(
            hwnd,
            if login_launch && app_from(hwnd).is_some_and(|a| a.desktop_tray_ready()) {
                SW_HIDE
            } else if offscreen {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOWNORMAL
            },
        );
        let mut msg = MSG::default();
        loop {
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 <= 0 {
                break; // 0 = WM_QUIT，-1 = 错误
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

// 先处理配对请求，再检查是否已有实例运行，避免浏览器的授权请求被已有实例吞掉。
// 设置会在进程之间同步。
fn connect_web_clipper() -> bool {
    use mochi_core::web_clipper::{native, pairing};
    let result = (|| -> anyhow::Result<bool> {
        let args: Vec<String> = std::env::args().collect();
        anyhow::ensure!(args.len() == 3, "无效的网页剪藏连接请求");
        let id = pairing::extension_from_url(&args[2])?;
        let text = "允许此浏览器扩展连接墨池并保存网页？\n\n仅在你刚刚点击了扩展中的“连接墨池”时允许。已有浏览器连接会保留。";
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let answer = unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                w!("连接网页剪藏"),
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND,
            )
        };
        if answer != IDYES {
            return Ok(false);
        }
        let settings = mochi_core::settings::SettingsService::new(None);
        pairing::approve(&settings, &id)?;
        native::register(&settings)?;
        // 调试版或便携版可能没有正式版的互斥锁，但仍在运行。
        Ok(!native::receiver_running())
    })();
    match result {
        Ok(approved) => approved,
        Err(error) => {
            let text: Vec<u16> = format!("连接失败：{error:#}")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    w!("网页剪藏"),
                    MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
                );
            }
            false
        }
    }
}

/// 请求重画。窗口过程里几乎每个改了状态的分支都要调它一次。
fn redraw(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// 窗口过程消费 App 的定时器请求；搜索输入每次重置去抖计时。
fn after_input(hwnd: HWND, app: &mut App) {
    app.input_activity();
    apply_timers(hwnd, app);
    // 新输入框的光标要等下一次绘制完成排版和滚动后才能确定位置。
    redraw(hwnd);
}
const CARET_TIMER: usize = 0x4d43_4152;

fn apply_caret_timer(hwnd: HWND, app: &App) {
    unsafe {
        let _ = KillTimer(Some(hwnd), CARET_TIMER);
        if let Some(ms) = app.caret_timer_delay() {
            SetTimer(Some(hwnd), CARET_TIMER, ms.max(10), None);
        }
    }
}
fn apply_timers(hwnd: HWND, app: &mut App) {
    for (id, ms) in app.take_timer_requests() {
        unsafe {
            SetTimer(Some(hwnd), id, ms, None);
        }
    }
}

/// 把候选窗钉到当前光标下。没有光标（不在编辑面）就不动。
fn sync_ime_position_for_app(hwnd: HWND, app: &App) {
    if unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() } != hwnd {
        return;
    }
    let caret = app.caret_in_client();
    platform::sync_system_caret(hwnd, caret);
    let Some(caret) = caret else {
        return;
    };
    platform::position_ime(hwnd, caret);
}

fn sync_ime_position(hwnd: HWND) {
    if let Some(app) = unsafe { app_from(hwnd) } {
        sync_ime_position_for_app(hwnd, app);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // ---------- 无边框窗口 ----------
            //
            // 把非客户区压成 0，标题栏与边框就从视觉上消失了，但窗口仍是标准窗口
            // ——投影、贴靠、Aero Snap、最大化动画全都还在。
            // 这是 Electron 的 `titleBarStyle: 'hidden'` 在 Win32 上的等价做法。
            WM_NCCALCSIZE if wparam.0 != 0 => {
                let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
                let rect = &mut (*params).rgrc[0];
                // 最大化时必须把边框让出来，否则内容会被挤到屏幕外
                if IsZoomed(hwnd).as_bool() {
                    let b = frame_border(hwnd);
                    rect.left += b;
                    rect.right -= b;
                    rect.top += b;
                    rect.bottom -= b;
                } else {
                    // 左右下保留 1px 让系统仍画出细边；顶部全吃掉（那才是标题栏）
                    rect.bottom -= 1;
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                // 坐标是**屏幕**坐标
                let (x, y) =
                    screen_to_client_dips(hwnd, lo_word_signed(lparam), hi_word_signed(lparam));
                let (w, h) = client_size_dips(hwnd);
                let dpi = dpi_for_window(hwnd);
                let border = to_dips(frame_border(hwnd) as f32, dpi);

                // 四边/四角的调整区。最大化时不给——那时窗口不该能拖边
                if !IsZoomed(hwnd).as_bool() {
                    if let Some(zone) = resize_zone(x, y, w, h, border) {
                        return LRESULT(zone as isize);
                    }
                }

                // 标题栏是拖动区，但要挖掉三个窗口按钮和 AI 开关，
                // 否则按钮点不动——HTCAPTION 会把点击吞掉变成拖窗口
                if y < ui::chrome::configured_layout().title_bar_height {
                    let viewport = Rect::new(0.0, 0.0, w, h);
                    if caption_buttons(viewport).iter().any(|b| b.contains(x, y)) {
                        return LRESULT(HTCLIENT as isize);
                    }
                    if let Some(app) = app_from(hwnd) {
                        if app.hit_is_titlebar_button(x, y) {
                            return LRESULT(HTCLIENT as isize);
                        }
                    }
                    return LRESULT(HTCAPTION as isize);
                }
                LRESULT(HTCLIENT as isize)
            }

            // ---------- 生命周期 ----------
            WM_NCCREATE => {
                // 把 App 指针从 CreateWindowExW 的 lpParam 挪到 GWLP_USERDATA
                let cs = lparam.0 as *const CREATESTRUCTW;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CREATE => {
                windows::Win32::UI::Shell::DragAcceptFiles(hwnd, true);
                if let Some(app) = app_from(hwnd) {
                    app.restore_last_workspace(hwnd);
                    app.install_capture(hwnd);
                    app.desktop_initialize(hwnd);
                    if std::env::var_os("MOCHI_VERIFY_OFFSCREEN").is_none() {
                        app.start_update_check(false);
                    }
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                capture_window::release_hotkey(hwnd.0 as isize);
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut App;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_SETFOCUS => {
                if let Some(app) = app_from(hwnd) {
                    app.input_window_focus(true);
                    app.refresh_shared_settings();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                let _ = KillTimer(Some(hwnd), CARET_TIMER);
                platform::clear_system_caret();
                if let Some(app) = app_from(hwnd) {
                    app.input_window_focus(false);
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if wparam.0 & 0xffff != WA_INACTIVE as usize {
                    if let Some(app) = app_from(hwnd) {
                        if app.refresh_shared_settings() {
                            redraw(hwnd);
                        }
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            desktop_window::MESSAGE => {
                if app_from(hwnd).is_some_and(|app| app.desktop_handle_events()) {
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            desktop_window::READY => {
                if let Some(app) = app_from(hwnd) {
                    app.desktop_take_results();
                }
                redraw(hwnd);
                LRESULT(0)
            }
            desktop_tray::ACTIVATE => {
                if let Some(app) = app_from(hwnd) {
                    app.desktop_show_main();
                }
                redraw(hwnd);
                LRESULT(0)
            }
            desktop_tray::MESSAGE => {
                if let Some(event) = desktop_tray::event(lparam) {
                    let action = if desktop_tray::activation(event) {
                        1
                    } else if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                        let paused = app_from(hwnd).is_some_and(|a| a.desktop_paused());
                        desktop_tray::menu(hwnd, paused)
                    } else {
                        0
                    };
                    if let Some(app) = app_from(hwnd) {
                        app.desktop_tray_action(action);
                    }
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                if let Some(app) = app_from(hwnd) {
                    app.desktop_display_changed();
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            // ---------- 后台线程投回来的消息 ----------
            platform::WM_APP_NOTIFICATION_DELIVER => {
                if let Some(app) = app_from(hwnd) {
                    app.deliver_native_notification(hwnd);
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_NOTIFICATION_CLICK => {
                if platform::notifications::is_activation(lparam) {
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                    }
                    let _ = SetForegroundWindow(hwnd);
                    if let Some(app) = app_from(hwnd) {
                        app.open_notification_center();
                        after_input(hwnd, app);
                    }
                }
                LRESULT(0)
            }
            WM_APP_HOME_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_home_analytics();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_FILES_CHANGED => {
                if let Some(app) = app_from(hwnd) {
                    app.on_files_changed();
                    app.desktop_mark_stale();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_INDEX_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_workspace_index_updates();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_UPDATE_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_update_result();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_UPDATE_DOWNLOAD_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_update_download_result();
                    redraw(hwnd);
                }
                LRESULT(0)
            }

            // ---------- 绘制与尺寸 ----------
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let mut painted = false;
                if let Some(app) = app_from(hwnd) {
                    // 首帧才做 DPI 调整，见 platform::scale_window_to_dpi 的注释
                    if app.claim_initial_size() {
                        platform::scale_window_to_dpi(hwnd);
                    }
                    let _ = app.paint(hwnd);
                    apply_timers(hwnd, app);
                    apply_caret_timer(hwnd, app);
                    painted = true;
                }
                let _ = EndPaint(hwnd, &ps);
                // 组合串可能让文本换行、滚动或改变 AI 输入框高度。只有这一帧
                // 完成排版后拿到的 caret 才是输入法候选窗的最终坐标。
                if painted {
                    sync_ime_position(hwnd);
                }
                LRESULT(0)
            }
            WM_SIZE => {
                if let Some(app) = app_from(hwnd) {
                    app.resize(
                        (lparam.0 & 0xFFFF) as u32,
                        ((lparam.0 >> 16) & 0xFFFF) as u32,
                    );
                }
                LRESULT(0)
            }
            // 擦除交给 D2D 的 Clear，让系统擦会闪
            WM_ERASEBKGND => LRESULT(1),
            WM_DPICHANGED => {
                // 系统给出建议矩形，照做即可保持逻辑尺寸不跳。下一帧的
                // Renderer::ensure_target 会在构建 Chrome/layout 前把 D2D
                // target DPI 同步到新的窗口 DPI；这里必须主动失效，否则
                // 同尺寸的跨显示器移动可能没有 WM_SIZE/重绘可依赖。
                let dpi_x = (wparam.0 & 0xFFFF) as u32;
                let dpi_y = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if let Some(app) = app_from(hwnd) {
                    app.on_dpi_changed(dpi_x, dpi_y);
                }
                let rc = lparam.0 as *const RECT;
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    (*rc).left,
                    (*rc).top,
                    (*rc).right - (*rc).left,
                    (*rc).bottom - (*rc).top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                redraw(hwnd);
                LRESULT(0)
            }

            // ---------- 鼠标 ----------
            WM_LBUTTONDOWN => {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd));
                platform::complete_ime(hwnd);
                let (x, y) = mouse_dips(hwnd, lparam);
                if let Some(app) = app_from(hwnd) {
                    app.on_click(x, y);
                    // 拖动期间指针会移出手柄甚至移出窗口，不捕获就会中途丢掉
                    if app.is_dragging() {
                        SetCapture(hwnd);
                    }
                    if let Some(action) = app.take_caption_action() {
                        apply_caption_action(hwnd, action);
                    }
                    after_input(hwnd, app);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut tracking);
                let (x, y) = mouse_dips(hwnd, lparam);
                if let Some(app) = app_from(hwnd) {
                    if app.on_mouse_move(x, y) {
                        redraw(hwnd);
                    }
                }
                LRESULT(0)
            }
            windows::Win32::UI::Controls::WM_MOUSELEAVE => {
                if let Some(app) = app_from(hwnd) {
                    if app.on_pointer_leave() {
                        redraw(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(app) = app_from(hwnd) {
                    let (x, y) = mouse_dips(hwnd, lparam);
                    if app.release_command_review(x, y) {
                        after_input(hwnd, app);
                        return LRESULT(0);
                    }
                    if app.is_dragging() {
                        app.end_drag_at(x, y);
                        let _ = ReleaseCapture();
                        redraw(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                platform::complete_ime(hwnd);
                let (x, y) = mouse_dips(hwnd, lparam);
                if let Some(app) = app_from(hwnd) {
                    app.on_double_click(x, y);
                    after_input(hwnd, app);
                }
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                platform::complete_ime(hwnd);
                let (x, y) = mouse_dips(hwnd, lparam);
                if let Some(app) = app_from(hwnd) {
                    app.on_right_click(x, y);
                    if !app.is_dragging()
                        && windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() == hwnd
                    {
                        let _ = ReleaseCapture();
                    }
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                if let Some(app) = app_from(hwnd) {
                    if app.cancel_horizontal_drag()
                        | app.cancel_scrollbar_drag()
                        | app.cancel_sidebar_tree_drag()
                        | app.cancel_navigation_library_drag()
                        | app.cancel_canvas_drag()
                    {
                        if windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() == hwnd {
                            let _ = ReleaseCapture();
                        }
                        redraw(hwnd);
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DROPFILES => {
                use std::os::windows::ffi::OsStringExt;
                use windows::Win32::UI::Shell::{
                    DragFinish, DragQueryFileW, DragQueryPoint, HDROP,
                };
                let drop = HDROP(wparam.0 as *mut _);
                let mut point = windows::Win32::Foundation::POINT::default();
                let client = DragQueryPoint(drop, &mut point).as_bool();
                let count = DragQueryFileW(drop, u32::MAX, None);
                let mut paths = Vec::new();
                if client {
                    for i in 0..count {
                        let length = DragQueryFileW(drop, i, None);
                        if length > 0 && length < 32768 {
                            let mut buffer = vec![0u16; length as usize + 1];
                            let n = DragQueryFileW(drop, i, Some(&mut buffer));
                            paths.push(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                                &buffer[..n as usize],
                            )));
                        }
                    }
                }
                DragFinish(drop);
                if client {
                    if let Some(app) = app_from(hwnd) {
                        let dpi = dpi_for_window(hwnd);
                        app.on_dropped_files(
                            paths,
                            to_dips(point.x as f32, dpi),
                            to_dips(point.y as f32, dpi),
                        );
                        redraw(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEHWHEEL => {
                let (x, y) =
                    screen_to_client_dips(hwnd, lo_word_signed(lparam), hi_word_signed(lparam));
                if let Some(app) = app_from(hwnd) {
                    if app.on_horizontal_wheel(x, y, ((wparam.0 >> 16) & 0xffff) as i16) {
                        redraw(hwnd);
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_MOUSEWHEEL => {
                // 滚轮的坐标是**屏幕**坐标，不是客户区坐标——别的鼠标消息都是客户区，
                // 只有这个不是。不转换的话指针位置永远判在窗口外。
                let (x, y) =
                    screen_to_client_dips(hwnd, lo_word_signed(lparam), hi_word_signed(lparam));
                if let Some(app) = app_from(hwnd) {
                    app.on_wheel(x, y, ((wparam.0 >> 16) & 0xFFFF) as i16);
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_SETCURSOR => {
                // lParam 低字是命中区域；只在客户区自己管光标，边框/标题栏交给系统
                if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                    let (x, y) = cursor_pos_dips(hwnd);
                    if let Some(app) = app_from(hwnd) {
                        if let Some(cursor) = app.cursor_for(x, y) {
                            if let Ok(h) = LoadCursorW(None, cursor) {
                                SetCursor(Some(h));
                                return LRESULT(1);
                            }
                        }
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            // ---------- 键盘 ----------
            WM_SYSKEYDOWN => {
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                if let Some(app) = app_from(hwnd) {
                    if app.on_accelerator(hwnd, wparam.0 as u16, shift, ctrl, true) {
                        after_input(hwnd, app);
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_SYSCHAR => {
                if app_from(hwnd).is_some_and(|app| app.take_suppressed_alt_char()) {
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_KEYDOWN => {
                // VK_PROCESSKEY 必须交给 DefWindowProc，由 IME 处理按键并发送组合消息。
                if wparam.0 as u16 == VK_PROCESSKEY.0 {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let key = wparam.0 as u16;
                if ctrl {
                    platform::complete_ime(hwnd);
                }
                if let Some(app) = app_from(hwnd) {
                    // 全局快捷键优先于焦点控件：Ctrl+S 在输入框里也要能存
                    let alt =
                        GetKeyState(windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU.0 as i32)
                            < 0;
                    let handled = app.on_accelerator(hwnd, key, shift, ctrl, alt)
                        || (!alt && app.on_edit_key(key, shift, ctrl));
                    if handled {
                        if key == 0x1b
                            && !app.is_dragging()
                            && windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() == hwnd
                        {
                            let _ = ReleaseCapture();
                        }
                        after_input(hwnd, app);
                        return LRESULT(0);
                    }
                }
                // 没消费的按键交回系统——快捷键、菜单键、无障碍都靠它
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CHAR => {
                if let Some(app) = app_from(hwnd) {
                    if app.on_utf16_char(wparam.0 as u16) {
                        after_input(hwnd, app);
                    }
                }
                LRESULT(0)
            }
            WM_UNICHAR => {
                if wparam.0 == 0xffff {
                    return LRESULT(1);
                }
                if let (Some(app), Some(ch)) = (app_from(hwnd), char::from_u32(wparam.0 as u32)) {
                    if app.on_char(ch) {
                        after_input(hwnd, app);
                    }
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == desktop_window::TIMER {
                    if let Some(app) = app_from(hwnd) {
                        app.desktop_tick();
                    }
                    return LRESULT(0);
                }
                if wparam.0 == CARET_TIMER {
                    let _ = KillTimer(Some(hwnd), CARET_TIMER);
                    if let Some(app) = app_from(hwnd) {
                        app.paint_caret_tick(hwnd);
                        apply_caret_timer(hwnd, app);
                    }
                    return LRESULT(0);
                }
                // 一次性计时器：到点先杀掉，要继续（番茄钟）由 App 再申请一次
                let _ = KillTimer(Some(hwnd), wparam.0);
                if let Some(app) = app_from(hwnd) {
                    app.on_timer(hwnd, wparam.0);
                    apply_timers(hwnd, app);
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_SEARCH_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_search_result();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_APP_AI_EVENT => {
                if let Some(app) = app_from(hwnd) {
                    app.take_ai_events();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_LINKS_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_backlinks();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                if app_from(hwnd).is_some_and(|app| app.desktop_close_to_tray()) {
                    redraw(hwnd);
                    return LRESULT(0);
                }
                if app_from(hwnd).is_none_or(|app| app.prepare_close()) {
                    let _ = DestroyWindow(hwnd);
                } else {
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            crate::capture_window::OPEN_ITEM => {
                if let Some(app) = app_from(hwnd) {
                    app.open_capture_item();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_HOTKEY if wparam.0 == 1 => {
                platform::complete_ime(hwnd);
                if let Some(app) = app_from(hwnd) {
                    app.show_capture();
                }
                LRESULT(0)
            }
            platform::WM_APP_MCP_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_mcp_test();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            desktop_window::shortcut_icon::READY => {
                if let Some(app) = app_from(hwnd) {
                    app.desktop_icons_ready();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_OBJECT_PICKER_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.poll_object_picker();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_PROVIDER_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_provider_test();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_PROVIDER_MODELS_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_provider_models();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_FILE_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_file_jobs();
                    if app.take_deferred_close() {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            platform::WM_APP_EDITOR_AI_READY => {
                if let Some(app) = app_from(hwnd) {
                    app.take_editor_ai();
                    after_input(hwnd, app);
                }
                LRESULT(0)
            }
            WM_APP_PDF_EVENT => {
                if let Some(app) = app_from(hwnd) {
                    app.on_pdf_event();
                    redraw(hwnd);
                }
                LRESULT(0)
            }

            // ---------- 输入法 ----------
            //
            // IME 组合串由编辑器绘制；候选窗口位置随文本光标更新。
            WM_IME_STARTCOMPOSITION => {
                // 处理组合开始消息，抑制系统默认组合窗口，避免与编辑器重复绘制。
                let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                sync_ime_position(hwnd);
                LRESULT(0)
            }
            WM_IME_COMPOSITION => {
                let flags = lparam.0 as u32;
                if let Some(app) = app_from(hwnd) {
                    app.input_activity();
                    // 先看有没有提交结果。同一条消息可能既带结果又带新的组合串
                    // （连续上屏时），所以两个分支都要查，不能 else if
                    if flags & GCS_RESULTSTR.0 != 0 {
                        if let Some(text) = platform::ime_string(hwnd, GCS_RESULTSTR.0) {
                            app.on_ime_commit(&text);
                        }
                    }
                    if flags & (GCS_COMPSTR.0 | windows::Win32::UI::Input::Ime::GCS_CURSORPOS.0)
                        != 0
                    {
                        let text = platform::ime_string(hwnd, GCS_COMPSTR.0).unwrap_or_default();
                        let cursor = platform::ime_cursor(hwnd, &text);
                        app.on_ime_composition(&text, cursor);
                    }
                    redraw(hwnd);
                }
                sync_ime_position(hwnd);
                // 返回 0 表示"我处理了"，系统不再画自己的组合窗口
                LRESULT(0)
            }
            WM_IME_ENDCOMPOSITION => {
                if let Some(app) = app_from(hwnd) {
                    app.on_ime_cancel();
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_IME_SETCONTEXT => {
                // 清掉 ISC_SHOWUICOMPOSITIONWINDOW，否则系统会在自己选的位置
                // 再画一遍组合串，和我们画的重影
                let masked = LPARAM(lparam.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize));
                DefWindowProcW(hwnd, msg, wparam, masked)
            }
            WM_IME_NOTIFY => {
                use windows::Win32::UI::Input::Ime::{IMN_CHANGECANDIDATE, IMN_OPENCANDIDATE};
                if wparam.0 == IMN_OPENCANDIDATE as usize
                    || wparam.0 == IMN_CHANGECANDIDATE as usize
                {
                    sync_ime_position(hwnd);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_INPUTLANGCHANGE => {
                let result = DefWindowProcW(hwnd, msg, wparam, lparam);
                sync_ime_position(hwnd);
                result
            }

            _ if app_from(hwnd).is_some_and(|a| {
                a.desktop_taskbar_message() != 0 && a.desktop_taskbar_message() == msg
            }) =>
            {
                if let Some(app) = app_from(hwnd) {
                    app.desktop_taskbar_recreated();
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 指针落在哪条边/哪个角的调整区上。`None` 表示不在任何调整区。
///
/// 抽成纯函数是为了能测——差一个像素的边界判定会让窗口某条边拖不动，
/// 这类问题很难靠手动操作稳定复现。
fn resize_zone(x: f32, y: f32, w: f32, h: f32, border: f32) -> Option<u32> {
    let left = x < border;
    let right = x > w - border;
    let top = y < border;
    let bottom = y > h - border;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(HTTOPLEFT),
        (true, _, _, true) => Some(HTTOPRIGHT),
        (_, true, true, _) => Some(HTBOTTOMLEFT),
        (_, true, _, true) => Some(HTBOTTOMRIGHT),
        (true, ..) => Some(HTTOP),
        (_, true, ..) => Some(HTBOTTOM),
        (_, _, true, _) => Some(HTLEFT),
        (_, _, _, true) => Some(HTRIGHT),
        _ => None,
    }
}

fn apply_caption_action(hwnd: HWND, action: CaptionAction) {
    unsafe {
        match action {
            CaptionAction::Minimize => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            CaptionAction::ToggleMaximize => {
                let cmd = if IsZoomed(hwnd).as_bool() {
                    SW_RESTORE
                } else {
                    SW_MAXIMIZE
                };
                let _ = ShowWindow(hwnd, cmd);
            }
            CaptionAction::Close => {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// 取回挂在窗口上的 `App`。返回引用而非所有权——所有权由 `WM_DESTROY` 释放。
unsafe fn app_from<'a>(hwnd: HWND) -> Option<&'a mut App> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &mut *ptr })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1200.0;
    const H: f32 = 800.0;
    const B: f32 = 8.0;

    #[test]
    fn the_middle_of_the_window_is_not_a_resize_zone() {
        assert_eq!(resize_zone(600.0, 400.0, W, H, B), None);
    }

    #[test]
    fn each_edge_maps_to_its_own_zone() {
        assert_eq!(resize_zone(2.0, 400.0, W, H, B), Some(HTLEFT));
        assert_eq!(resize_zone(1198.0, 400.0, W, H, B), Some(HTRIGHT));
        assert_eq!(resize_zone(600.0, 2.0, W, H, B), Some(HTTOP));
        assert_eq!(resize_zone(600.0, 798.0, W, H, B), Some(HTBOTTOM));
    }

    #[test]
    fn corners_win_over_the_edges_they_touch() {
        // 角落必须比边优先：判成边的话对角拖动就只能改一个方向
        assert_eq!(resize_zone(2.0, 2.0, W, H, B), Some(HTTOPLEFT));
        assert_eq!(resize_zone(1198.0, 2.0, W, H, B), Some(HTTOPRIGHT));
        assert_eq!(resize_zone(2.0, 798.0, W, H, B), Some(HTBOTTOMLEFT));
        assert_eq!(resize_zone(1198.0, 798.0, W, H, B), Some(HTBOTTOMRIGHT));
    }

    #[test]
    fn the_zone_is_exactly_border_wide() {
        // 边界上那一像素归客户区，不归调整区——多算一像素会让贴边的控件点不动
        assert_eq!(resize_zone(B, 400.0, W, H, B), None);
        assert_eq!(resize_zone(B - 0.5, 400.0, W, H, B), Some(HTLEFT));
        assert_eq!(resize_zone(W - B, 400.0, W, H, B), None);
        assert_eq!(resize_zone(W - B + 0.5, 400.0, W, H, B), Some(HTRIGHT));
    }
}
