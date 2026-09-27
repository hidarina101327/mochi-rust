//! Win32 窗口、DPI、IME 和系统对话框。

use std::path::PathBuf;
pub mod autostart;
mod clipboard;
mod clipboard_image;
pub mod desktop_files;
pub mod desktop_media;
pub mod desktop_search;
pub mod notifications;
pub use clipboard::read_html as read_clipboard_html;
pub use clipboard::write as copy_payload;
pub use clipboard::write_image as copy_image;
pub use clipboard_image::read_png as read_clipboard_image;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow,
    ImmSetCompositionWindow, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, GCS_CURSORPOS,
    IME_COMPOSITION_STRING,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SetWindowPos, IDC_SIZEWE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WM_APP,
};

use crate::ui::editor;
use crate::ui::layout::{to_dips, Rect};
use crate::ui::theme;

/// 初始窗口尺寸，**逻辑**像素。Electron 的 `BrowserWindow` 也是按 CSS 像素给的，
/// 两版要开出一样大的窗口。
pub const LOGICAL_WIDTH: i32 = 1200;
pub const LOGICAL_HEIGHT: i32 = 800;

/// 文件监听合并后的一批变更已到达，UI 该重画了。
pub const WM_APP_FILES_CHANGED: u32 = WM_APP + 1;
/// 后台线程算好了首页快照。
pub const WM_APP_HOME_READY: u32 = WM_APP + 2;
/// 后台线程搜完了一次全局搜索。
pub const WM_APP_SEARCH_READY: u32 = WM_APP + 3;
/// AI 运行线程有新事件（流式片段、工具结果、结束）。
pub const WM_APP_AI_EVENT: u32 = WM_APP + 4;
/// PDF 工作线程有事件（文档加载完 / 某一页渲染好 / 失败）。
pub const WM_APP_PDF_EVENT: u32 = WM_APP + 5;
pub const WM_APP_LINKS_READY: u32 = WM_APP + 6;
pub const WM_APP_MCP_READY: u32 = WM_APP + 7;
pub const WM_APP_PROVIDER_READY: u32 = WM_APP + 8;
pub const WM_APP_FILE_READY: u32 = WM_APP + 9;
pub const WM_APP_EDITOR_AI_READY: u32 = WM_APP + 10;
pub const WM_APP_OBJECT_PICKER_READY: u32 = WM_APP + 11;
/// 设置页的提供方模型列表请求完成。
pub const WM_APP_PROVIDER_MODELS_READY: u32 = WM_APP + 12;
/// GitHub Releases 更新检查线程已返回。
pub const WM_APP_UPDATE_READY: u32 = WM_APP + 13;
/// 原生安装包已下载并校验完毕，UI 可以退出并交给安装助手。
pub const WM_APP_UPDATE_DOWNLOAD_READY: u32 = WM_APP + 14;
/// 后台全量搜索索引有进度或完成结果，UI 取通道并重绘右上角任务提示。
pub const WM_APP_INDEX_READY: u32 = WM_APP + 15;
pub const WM_APP_NOTIFICATION_DELIVER: u32 = WM_APP + 16;
pub const WM_APP_NOTIFICATION_CLICK: u32 = WM_APP + 17;

/// 全局搜索的去抖计时器 id（`SetTimer` 的 `nIDEvent`）。
pub const TIMER_SEARCH: usize = 1;
pub const TIMER_AI_LOCATOR: usize = 6;
/// 番茄钟的刷新计时器（250ms，与 TSX 的 `setInterval(tick, 250)` 一致）。
pub const TIMER_POMODORO: usize = 2;
/// 自动保存的延时（编辑后停顿 `save.autoSaveDelay` 毫秒落盘）。
pub const TIMER_AUTOSAVE: usize = 3;
pub const TIMER_TOAST: usize = 5;
pub const TIMER_EDITOR_AI: usize = 4;
/// 资源占用采样和字数刷新计时器放在应用层。
pub const TIMER_RESOURCE: usize = 7;
pub const TIMER_WORD_COUNT: usize = 8;
/// 让流式输出的助手看起来仍在活动，并检测是否卡死。
pub const TIMER_AI_ANIMATION: usize = 9;
pub const TIMER_VERSION_JOB: usize = 10;
pub const TIMER_DOCUMENT_LAYOUT: usize = 11;
pub const TIMER_BASE_AUTOMATION: usize = 12;
pub const TIMER_NOTIFICATIONS: usize = 13;
pub const TIMER_WORKFLOWS: usize = 14;
pub const TIMER_BASE_EXPORT: usize = 15;
pub const TIMER_NAVIGATION_DRAG: usize = 16;
pub const TIMER_PROVIDER_INTERACTION: usize = 17;
pub const TIMER_WINDOW_MOTION: usize = 18;
pub const TIMER_TREE_LOAD: usize = 19;
pub const TIMER_SCHEDULE: usize = 20;

pub fn client_area_animations_enabled() -> bool {
    if crate::ui::settings_values::boolean("appearance.reduceMotion", false) {
        return false;
    }
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut enabled = windows::core::BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut windows::core::BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    enabled.as_bool()
}

/// 水平缩放光标，用于可拖动的分隔条。
pub const CURSOR_RESIZE_HORIZONTAL: PCWSTR = IDC_SIZEWE;

/// 页签关闭后，请 Windows 释放被丢弃的文档/图片页。
/// 否则即便 Rust 已经 drop 了所有持有者，分配器仍可能保留很大的
/// 工作集，状态栏的数字就会失真。
pub fn trim_process_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// 原生窗口及其 Direct2D 目标使用的 DPI。
///
/// 感知每显示器 DPI 的窗口在创建过程中可能短暂上报 0；
/// 把它当作进程基线处理，所有坐标换算和渲染目标设置
/// 才能处在同一刻度上。
pub fn dpi_for_window(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96)
}

/// 无边框窗口的边框宽度（物理像素，按 DPI 缩放）。
/// 用户要能抓到窗口边缘调大小，所以客户区不能真的铺满到最外沿。
pub fn frame_border(hwnd: HWND) -> i32 {
    let dpi = dpi_for_window(hwnd) as i32;
    // Windows 自己的可抓取边框大约 8 逻辑像素
    8 * dpi / 96
}

/// 系统窗口控件（最小化/最大化/关闭）的三个按钮矩形，DIP。
///
/// 按钮宽度取自共享布局令牌（138 = 3 × 46），保持标题栏可用区域一致。
pub fn caption_buttons(viewport: Rect) -> [Rect; 3] {
    let l = crate::ui::chrome::configured_layout();
    let h = l.title_bar_height;
    let w = l.title_bar_controls_width / 3.0;
    let right = viewport.right;
    [
        Rect::new(right - w * 3.0, 0.0, right - w * 2.0, h),
        Rect::new(right - w * 2.0, 0.0, right - w, h),
        Rect::new(right - w, 0.0, right, h),
    ]
}

/// 按窗口所在显示器的 DPI 设置初始逻辑尺寸，并限制在显示器工作区内。
///
/// 在首个 `WM_PAINT` 中调用，使初始 DPI 消息处理完成后再调整尺寸。
/// 创建前使用系统 DPI，或显示后立即调整，都可能与最终显示器 DPI 不一致。
/// Windows API 失败时保留现有尺寸，不中断启动。
pub fn scale_window_to_dpi(hwnd: HWND) {
    unsafe {
        let dpi = dpi_for_window(hwnd);
        if dpi == 96 {
            return;
        }
        let mut want_w = LOGICAL_WIDTH * dpi as i32 / 96;
        let mut want_h = LOGICAL_HEIGHT * dpi as i32 / 96;

        // 夹到显示器工作区（已排除任务栏），留一点边不要贴死
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let work = info.rcWork;
            want_w = want_w.min(work.right - work.left);
            want_h = want_h.min(work.bottom - work.top);
        }

        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            want_w,
            want_h,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

// ---------- 坐标 ----------

pub fn lo_word_signed(lparam: LPARAM) -> f32 {
    (lparam.0 & 0xFFFF) as i16 as f32
}

pub fn hi_word_signed(lparam: LPARAM) -> f32 {
    ((lparam.0 >> 16) & 0xFFFF) as i16 as f32
}

/// 鼠标消息的坐标是**物理像素**，而布局全程在 **DIP** 空间（D2D `GetSize()` 的单位）。
/// 两者在 100% 缩放下相等，高 DPI 下会系统性错位——必须在入口换算一次。
/// 详见 `ui::layout::to_dips`。
pub fn mouse_dips(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let dpi = dpi_for_window(hwnd);
    (
        to_dips(lo_word_signed(lparam), dpi),
        to_dips(hi_word_signed(lparam), dpi),
    )
}

/// 屏幕坐标 → 客户区 DIP。
///
/// `WM_MOUSEWHEEL` 和 `WM_NCHITTEST` 给的是**屏幕**坐标，别的鼠标消息都是客户区——
/// 只有这两个不是。不转换的话指针位置永远判在窗口外。
pub fn screen_to_client_dips(hwnd: HWND, screen_x: f32, screen_y: f32) -> (f32, f32) {
    let mut pt = POINT {
        x: screen_x as i32,
        y: screen_y as i32,
    };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut pt);
        let dpi = dpi_for_window(hwnd);
        (to_dips(pt.x as f32, dpi), to_dips(pt.y as f32, dpi))
    }
}

/// 当前指针位置，换算成客户区 DIP。`WM_SETCURSOR` 不带坐标，只能现查。
pub fn cursor_pos_dips(hwnd: HWND) -> (f32, f32) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    screen_to_client_dips(hwnd, pt.x as f32, pt.y as f32)
}

/// 客户区尺寸（DIP）。`WM_NCHITTEST` 里要拿它判四边。
pub fn client_size_dips(hwnd: HWND) -> (f32, f32) {
    let mut rc = RECT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
        let dpi = dpi_for_window(hwnd);
        (
            to_dips(rc.right as f32, dpi),
            to_dips(rc.bottom as f32, dpi),
        )
    }
}

// ---------- 输入法 ----------

/// 遵循 Windows 的辅助功能设置，包括刻意保持稳定的光标。
pub fn caret_blink_period() -> Option<u32> {
    if !crate::ui::settings_values::boolean("appearance.caretBlink", true) {
        return None;
    }
    match unsafe { windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime() } {
        u32::MAX => None,
        0 => Some(500),
        ms => Some(ms),
    }
}

thread_local! {
    static SYSTEM_CARET: std::cell::Cell<Option<(isize, i32, i32)>> = const { std::cell::Cell::new(None) };
}

pub fn clear_system_caret() {
    SYSTEM_CARET.with(|state| {
        if state.take().is_some() {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyCaret();
            }
        }
    });
}

fn caret_pixels(caret: Rect, dpi: u32) -> RECT {
    let scale = dpi as f32 / 96.0;
    let left = (caret.left * scale).round() as i32;
    let top = (caret.top * scale).round() as i32;
    RECT {
        left,
        top,
        right: ((caret.right * scale).round() as i32).max(left + 1),
        bottom: ((caret.bottom * scale).round() as i32).max(top + 1),
    }
}

/// 有些 IME/辅助工具查询的是 Win32 光标而不是 IMM32 窗体。
/// 发布同样的几何信息，但绝不 ShowCaret：光标由 Direct2D 统一绘制。
pub fn sync_system_caret(hwnd: HWND, caret: Option<Rect>) {
    if unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() } != hwnd {
        return;
    }
    let Some(caret) = caret else {
        clear_system_caret();
        return;
    };
    let pixels = caret_pixels(caret, dpi_for_window(hwnd));
    let size = (
        hwnd.0 as isize,
        pixels.right - pixels.left,
        pixels.bottom - pixels.top,
    );
    SYSTEM_CARET.with(|state| unsafe {
        if state.get() != Some(size) {
            clear_system_caret();
            if windows::Win32::UI::WindowsAndMessaging::CreateCaret(hwnd, None, size.1, size.2)
                .is_ok()
            {
                state.set(Some(size));
            }
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::SetCaretPos(pixels.left, pixels.top);
    });
}

/// 读输入法的组合串或结果串。
///
/// `ImmGetCompositionStringW` 返回的是**字节数**而不是字符数——
/// 按字符数分配缓冲区会截掉一半。
pub fn ime_string(hwnd: HWND, kind: u32) -> Option<String> {
    unsafe {
        let ctx = ImmGetContext(hwnd);
        if ctx.is_invalid() {
            return None;
        }
        let bytes = ImmGetCompositionStringW(ctx, IME_COMPOSITION_STRING(kind), None, 0);
        if bytes <= 0 {
            let _ = ImmReleaseContext(hwnd, ctx);
            return None;
        }
        let mut buf = vec![0u16; bytes as usize / 2];
        ImmGetCompositionStringW(
            ctx,
            IME_COMPOSITION_STRING(kind),
            Some(buf.as_mut_ptr() as *mut _),
            bytes as u32,
        );
        let _ = ImmReleaseContext(hwnd, ctx);
        Some(String::from_utf16_lossy(&buf))
    }
}

/// 将 IMM32 的 UTF-16 光标位置换成缓冲区使用的 UTF-8 字节偏移。
pub fn ime_cursor(hwnd: HWND, text: &str) -> usize {
    let units = unsafe {
        let ctx = ImmGetContext(hwnd);
        if ctx.is_invalid() {
            return text.len();
        }
        let n = ImmGetCompositionStringW(ctx, GCS_CURSORPOS, None, 0);
        let _ = ImmReleaseContext(hwnd, ctx);
        n.max(0) as usize
    };
    // 换算放在 editor.rs（与 `utf16_offset` 互为逆运算，测试也在那边）——
    // 在这里再写一遍就是同一段算术的第二份拷贝
    editor::byte_offset_from_utf16(text, units)
}

/// 把候选窗钉在光标下面。`caret` 是客户区 DIP 里的光标矩形。
///
/// 不设的话，候选框会固定在窗口左上角——用户在页面中间打字，候选词却在角落里。
/// `CFS_EXCLUDE` 告诉输入法"这块矩形是光标所在的行，别盖住它"。
pub fn position_ime(hwnd: HWND, caret: Rect) {
    unsafe {
        let pixels = caret_pixels(caret, dpi_for_window(hwnd));
        let x = pixels.left;
        let y = pixels.bottom;

        let ctx = ImmGetContext(hwnd);
        if ctx.is_invalid() {
            return;
        }
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x, y: pixels.top },
            rcArea: RECT::default(),
        };
        let _ = ImmSetCompositionWindow(ctx, &form);
        let cand = CANDIDATEFORM {
            dwIndex: 0,
            dwStyle: CFS_EXCLUDE,
            ptCurrentPos: POINT { x, y },
            rcArea: pixels,
        };
        let _ = ImmSetCandidateWindow(ctx, &cand);
        let _ = ImmReleaseContext(hwnd, ctx);
    }
}

// ---------- 对话框 ----------

/// 文件夹选择对话框。失败/取消一律返回 None，不要把 COM 错误冒到 UI 上。
pub fn pick_folder(owner: HWND) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS).ok()?;
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = wide.to_string().ok()?;
        CoTaskMemFree(Some(wide.0 as *const _));
        Some(PathBuf::from(path))
    }
}

pub fn pick_file(owner: HWND) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = wide.to_string().ok();
        CoTaskMemFree(Some(wide.0 as *const _));
        text.map(PathBuf::from)
    }
}
pub fn pick_files(owner: HWND) -> Option<Vec<PathBuf>> {
    unsafe {
        use windows::Win32::UI::Shell::{
            FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
        };
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog
            .SetOptions(
                dialog.GetOptions().ok()?
                    | FOS_ALLOWMULTISELECT
                    | FOS_FILEMUSTEXIST
                    | FOS_FORCEFILESYSTEM,
            )
            .ok()?;
        dialog.Show(Some(owner)).ok()?;
        let items = dialog.GetResults().ok()?;
        let mut paths = Vec::new();
        for i in 0..items.GetCount().ok()? {
            let item = items.GetItemAt(i).ok()?;
            let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let text = wide.to_string().ok();
            CoTaskMemFree(Some(wide.0 as *const _));
            paths.push(PathBuf::from(text?));
        }
        Some(paths)
    }
}
pub fn save_file(owner: HWND, name: &str) -> Option<PathBuf> {
    save_file_kind(owner, name, false)
}
pub fn save_xlsx_file(owner: HWND, name: &str) -> Option<PathBuf> {
    save_file_kind(owner, name, true)
}
fn save_file_kind(owner: HWND, name: &str, xlsx: bool) -> Option<PathBuf> {
    unsafe {
        use windows::Win32::UI::Shell::{FileSaveDialog, IFileSaveDialog, FOS_OVERWRITEPROMPT};
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog
            .SetFileName(&windows::core::HSTRING::from(name))
            .ok()?;
        if xlsx || name.to_ascii_lowercase().ends_with(".zip") {
            use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
            let label = windows::core::HSTRING::from(if xlsx {
                "Excel 工作簿 (*.xlsx)"
            } else {
                "Mochi 导入包 (*.zip)"
            });
            let pattern = windows::core::HSTRING::from(if xlsx { "*.xlsx" } else { "*.zip" });
            dialog
                .SetFileTypes(&[COMDLG_FILTERSPEC {
                    pszName: windows::core::PCWSTR(label.as_ptr()),
                    pszSpec: windows::core::PCWSTR(pattern.as_ptr()),
                }])
                .ok()?;
            dialog
                .SetDefaultExtension(&windows::core::HSTRING::from(if xlsx {
                    "xlsx"
                } else {
                    "zip"
                }))
                .ok()?;
        }
        dialog
            .SetOptions(dialog.GetOptions().ok()? | FOS_OVERWRITEPROMPT)
            .ok()?;
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = wide.to_string().ok();
        CoTaskMemFree(Some(wide.0 as *const _));
        text.map(PathBuf::from)
    }
}

// ---------- 外壳操作 ----------

/// 移到回收站。对应 Electron 的 `shell.trashItem`。
///
/// Windows 8+ 明确请求回收；如果系统必须永久删除，保留系统警告，不静默降级。
/// <https://learn.microsoft.com/zh-cn/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags>
pub fn move_to_trash(owner: HWND, path: &std::path::Path) -> windows::core::Result<()> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, FOFX_EARLYFAILURE,
        FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_WANTNUKEWARNING,
    };
    struct RestoreInput(HWND, bool);
    impl Drop for RestoreInput {
        fn drop(&mut self) {
            if self.1 {
                unsafe {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(self.0, true);
                }
            }
        }
    }
    unsafe {
        let enabled = !owner.0.is_null()
            && windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(owner).as_bool();
        if enabled {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
        }
        let _input = RestoreInput(owner, enabled);
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOwnerWindow(owner)?;
        operation.SetOperationFlags(
            FOF_ALLOWUNDO | FOF_WANTNUKEWARNING | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE,
        )?;
        let shell_path = desktop_files::normalized_shell_path(path);
        let item: IShellItem = SHCreateItemFromParsingName(
            &HSTRING::from(shell_path.to_string_lossy().as_ref()),
            None,
        )?;
        operation.DeleteItem(&item, None)?;
        operation.PerformOperations()?;
        if operation.GetAnyOperationsAborted()?.as_bool() {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_ABORT,
                "删除操作已取消",
            ));
        }
    }
    Ok(())
}

/// 在改变自绘控件焦点前请 IME 上屏。调用方必须尚未借用 App，以允许同步 IME 消息重入。
pub fn complete_ime(hwnd: HWND) {
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.0.is_null() {
            return;
        }
        if ImmGetCompositionStringW(
            context,
            windows::Win32::UI::Input::Ime::GCS_COMPSTR,
            None,
            0,
        ) > 0
        {
            let _ = windows::Win32::UI::Input::Ime::ImmNotifyIME(
                context,
                windows::Win32::UI::Input::Ime::NI_COMPOSITIONSTR,
                windows::Win32::UI::Input::Ime::CPS_COMPLETE,
                0,
            );
        }
        let _ = ImmReleaseContext(hwnd, context);
    }
}

/// 解析给 Explorer 的目标：文件返回「选中该文件」，目录直接打开；暂时不存在的
/// 文件则定位到存在的父目录。绝不把无效参数交给 Explorer（它会静默回退到桌面）。
fn explorer_target(path: &std::path::Path) -> Option<(PathBuf, bool)> {
    if let Ok(absolute) = path.canonicalize() {
        return Some((absolute.clone(), absolute.is_file()));
    }
    let parent = path.parent()?.canonicalize().ok()?;
    parent.is_dir().then_some((parent, false))
}

/// 在资源管理器里选中该文件。对应 `shell.showItemInFolder`。
pub fn show_in_explorer(path: &std::path::Path) {
    let Some((target, select)) = explorer_target(path) else {
        return;
    };
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::{
        ILCreateFromPathW, ILFindLastID, ILFree, ILRemoveLastID, SHOpenFolderAndSelectItems,
    };

    // 不调用 explorer.exe /select：它是命令行兼容入口，参数里的中文/空格在不同
    // Shell 版本上会被重新拆分，失败后便会默认打开桌面。PIDL API 没有这层解析。
    let shell_path = desktop_files::normalized_shell_path(&target);
    let shell_path = HSTRING::from(shell_path.to_string_lossy().as_ref());
    let opened = unsafe {
        let pidl = ILCreateFromPathW(&shell_path);
        if pidl.is_null() {
            false
        } else {
            let ok = if select {
                let child = ILFindLastID(pidl);
                if child.is_null() || !ILRemoveLastID(Some(pidl)).as_bool() {
                    false
                } else {
                    let children = [child as *const _];
                    SHOpenFolderAndSelectItems(pidl, Some(&children), 0).is_ok()
                }
            } else {
                SHOpenFolderAndSelectItems(pidl, None, 0).is_ok()
            };
            ILFree(Some(pidl));
            ok
        }
    };
    if !opened {
        // API 不可用时只打开已验证过的目标目录，绝不再传 /select 让它回退到桌面。
        let directory = if select {
            target.parent().unwrap_or(&target)
        } else {
            &target
        };
        let directory = desktop_files::normalized_shell_path(directory);
        let _ = std::process::Command::new("explorer.exe")
            .arg(directory)
            .spawn();
    }
}

/// Windows「应用模式」是否为深色（`HKCU\...\Themes\Personalize\AppsUseLightTheme == 0`）。
/// 对应 Electron 的 `nativeTheme.shouldUseDarkColors`。读不到就当浅色。
pub fn system_prefers_dark() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 1;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

/// 交给系统默认程序打开（浏览器、邮件客户端）。对应 `shell.openExternal`。
pub fn open_external(url: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // This conversion only recognizes extended Win32 path prefixes and leaves
    // URLs intact. Do not stat a possibly remote target on the caller's thread.
    let target = desktop_files::normalized_shell_path(std::path::Path::new(url));
    let target = HSTRING::from(target.to_string_lossy().as_ref());
    let result = unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            &target,
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // 返回值 > 32 表示成功（这是 ShellExecute 的历史约定）
    result.0 as isize > 32
}

/// 相当于 Electron 的 `shell.showItemInFolder`，用于导入的本地快捷方式。
/// 源路径作为单个带引号的参数交给资源管理器；从不经过 cmd.exe，
/// 快捷方式名因此不可能被当成语法解读。
pub fn show_item_in_folder(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() || !std::path::Path::new(path).exists() {
        return false;
    }
    let shell_path = desktop_files::normalized_shell_path(std::path::Path::new(path));
    std::process::Command::new("explorer.exe")
        .arg(format!(
            "/select,{}",
            quote_windows_argument(&shell_path.to_string_lossy())
        ))
        .spawn()
        .is_ok()
}

/// 使用快捷导航指定的浏览器打开网页。它只接受已经由用户设置页保存的可执行
/// 文件路径，并把 URL 作为独立参数传入，绝不拼接 shell 命令。
pub fn open_with_application(
    executable: &str,
    target: &str,
    arguments: &[String],
    working_directory: &str,
) -> bool {
    if executable.trim().is_empty() || target.trim().is_empty() {
        return false;
    }
    let mut command = std::process::Command::new(executable);
    command.arg(target).args(arguments);
    if !working_directory.trim().is_empty() {
        command.current_dir(working_directory);
    }
    command.spawn().is_ok()
}

/// Electron 的 `shortcuts.launch` 会把目标、独立参数和工作目录一起交给 Windows
/// Shell。这里保持同样的调用边界：参数来自结构化数组，逐项按 Windows 规则引用，
/// 不拼接 cmd / PowerShell 字符串。
pub fn launch_shortcut(target: &str, arguments: &[String], working_directory: &str) -> bool {
    launch_shortcut_with_verb("open", target, arguments, working_directory)
}

/// 与快速导航里 Electron 的「管理员运行」动作一致。`runas` 把
/// 授权与提权交给 Windows 处理；Mochi 自身不创建命令解释器
/// 或提权的宿主进程。
pub fn launch_shortcut_as_admin(
    target: &str,
    arguments: &[String],
    working_directory: &str,
) -> bool {
    launch_shortcut_with_verb("runas", target, arguments, working_directory)
}

fn launch_shortcut_with_verb(
    verb: &str,
    target: &str,
    arguments: &[String],
    working_directory: &str,
) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if target.trim().is_empty() {
        return false;
    }
    let verb = HSTRING::from(verb);
    let target = target.trim();
    let target = if std::path::Path::new(target).exists() {
        desktop_files::normalized_shell_path(std::path::Path::new(target))
            .to_string_lossy()
            .into_owned()
    } else {
        target.to_owned()
    };
    let target = HSTRING::from(target.as_str());
    let parameters = HSTRING::from(
        arguments
            .iter()
            .map(|argument| quote_windows_argument(argument))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let working_directory = working_directory.trim();
    let working_directory = if std::path::Path::new(working_directory).is_dir() {
        desktop_files::normalized_shell_path(std::path::Path::new(working_directory))
            .to_string_lossy()
            .into_owned()
    } else {
        working_directory.to_owned()
    };
    let directory = HSTRING::from(working_directory.as_str());
    let result =
        unsafe { ShellExecuteW(None, &verb, &target, &parameters, &directory, SW_SHOWNORMAL) };
    result.0 as isize > 32
}

fn quote_windows_argument(argument: &str) -> String {
    if argument.is_empty() {
        return "\"\"".into();
    }
    if !argument.chars().any(|ch| ch.is_whitespace() || ch == '\"') {
        return argument.into();
    }
    let mut out = String::from("\"");
    let mut slashes = 0usize;
    for ch in argument.chars() {
        if ch == '\\' {
            slashes += 1;
        } else if ch == '\"' {
            out.push_str(&"\\".repeat(slashes.saturating_mul(2) + 1));
            out.push(ch);
            slashes = 0;
        } else {
            out.push_str(&"\\".repeat(slashes));
            out.push(ch);
            slashes = 0;
        }
    }
    out.push_str(&"\\".repeat(slashes.saturating_mul(2)));
    out.push('\"');
    out
}

#[cfg(test)]
mod shortcut_tests {
    use super::quote_windows_argument;

    #[test]
    fn shortcut_arguments_keep_spaces_quotes_and_trailing_slashes_as_one_argument() {
        assert_eq!(quote_windows_argument("--profile"), "--profile");
        assert_eq!(quote_windows_argument("My Profile"), "\"My Profile\"");
        assert_eq!(
            quote_windows_argument("C:\\Work Folder\\"),
            "\"C:\\Work Folder\\\\\""
        );
        assert_eq!(quote_windows_argument("a\"b"), "\"a\\\"b\"");
    }
}

/// 通过 Windows 自带 SAPI 播放英语句子。应用主线程本来就是 STA COM apartment，
/// 因此不引入 WebView、Node 或在线服务。朗读在独立 STA 线程中同步执行，COM
/// 对象会一直存活到播放结束，且不会阻塞 UI 主线程。
pub fn speak_text(text: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::{ISpVoice, SpVoice, SPF_DEFAULT};

    let text = text.trim();
    if text.is_empty() {
        return Err("没有可播放的文本".into());
    }
    let speech = text.to_owned();
    std::thread::Builder::new()
        .name("mochi-sapi".into())
        .spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let voice: windows::core::Result<ISpVoice> =
                CoCreateInstance(&SpVoice, None, CLSCTX_INPROC_SERVER);
            if let Ok(voice) = voice {
                let phrase = HSTRING::from(speech);
                let _ = voice.Speak(&phrase, SPF_DEFAULT.0 as u32, None);
            }
        })
        .map_err(|error| format!("启动语音线程失败：{error}"))?;
    Ok(())
}

/// 写入系统剪贴板（Unicode 文本）。
pub fn copy_to_clipboard(text: &str) -> bool {
    let owner = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow() };
    clipboard::write(owner, text, None)
}

/// 读系统剪贴板里的 Unicode 文本。没有文本（或是图片等别的格式）返回 `None`。
/// `\r\n` 归一成 `\n`——缓冲区里只认 `\n`，混进 `\r` 会让行范围与光标映射都乱掉。
pub fn read_clipboard_text() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        if OpenClipboard(None).is_err() {
            return None;
        }
        let result = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
            let mem = HGLOBAL(handle.0);
            let ptr = GlobalLock(mem) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let units = GlobalSize(mem) / 2;
            let slice = std::slice::from_raw_parts(ptr, units);
            let end = slice.iter().position(|&c| c == 0).unwrap_or(units);
            let text = String::from_utf16_lossy(&slice[..end]);
            let _ = GlobalUnlock(mem);
            Some(text.replace("\r\n", "\n").replace('\r', "\n"))
        })();
        let _ = CloseClipboard();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_and_system_caret_share_client_pixel_geometry_at_fractional_dpi() {
        let caret = Rect::from_size(100.5, 60.25, 1.0, 20.0);
        for dpi in [96, 120, 144, 192] {
            let pixels = caret_pixels(caret, dpi);
            let scale = dpi as f32 / 96.0;
            assert_eq!(pixels.left, (caret.left * scale).round() as i32);
            assert_eq!(pixels.top, (caret.top * scale).round() as i32);
            assert_eq!(pixels.bottom, (caret.bottom * scale).round() as i32);
            assert!(pixels.right > pixels.left);
            assert!(pixels.bottom > pixels.top);
        }
    }

    #[test]
    fn the_three_caption_buttons_tile_the_control_area_flush_to_the_right_edge() {
        let viewport = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let b = caption_buttons(viewport);
        let l = theme::tokens().layout;

        assert_eq!(b[2].right, 1200.0, "最右一个必须贴住窗口右缘");
        assert_eq!(b[0].left, 1200.0 - l.title_bar_controls_width);
        // 三个等宽、首尾相接，中间不留缝——留缝的那一列点不动
        assert_eq!(b[0].right, b[1].left);
        assert_eq!(b[1].right, b[2].left);
        assert_eq!(b[0].width(), l.title_bar_controls_width / 3.0);
        // 高度就是标题栏高度，不多不少
        assert!(b
            .iter()
            .all(|r| r.top == 0.0 && r.bottom == l.title_bar_height));
    }

    #[test]
    fn caption_buttons_track_the_right_edge_when_the_window_narrows() {
        let wide = caption_buttons(Rect::new(0.0, 0.0, 1200.0, 800.0));
        let narrow = caption_buttons(Rect::new(0.0, 0.0, 600.0, 400.0));
        assert_eq!(narrow[2].right, 600.0);
        assert_eq!(
            wide[0].width(),
            narrow[0].width(),
            "宽度是固定令牌，不随窗口缩放"
        );
    }

    #[test]
    fn word_extraction_treats_coordinates_as_signed() {
        // 指针拖到窗口左侧/上方时坐标是负数，按无符号读会变成 65000 多
        let lparam = LPARAM(((-3i16 as u16 as isize) << 16) | (-7i16 as u16 as isize & 0xFFFF));
        assert_eq!(lo_word_signed(lparam), -7.0);
        assert_eq!(hi_word_signed(lparam), -3.0);
    }

    #[test]
    fn explorer_target_selects_existing_file_and_opens_existing_parent_for_missing_file() {
        let root = std::env::temp_dir().join(format!(
            "mochi-explorer-target-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(8)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("带 空格 的文档.mc");
        std::fs::write(&file, "内容").unwrap();
        let (selected, select) = explorer_target(&file).unwrap();
        assert!(select);
        assert_eq!(selected, file.canonicalize().unwrap());
        let (parent, select) = explorer_target(&root.join("尚未保存.md")).unwrap();
        assert!(!select);
        assert_eq!(parent, root.canonicalize().unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
