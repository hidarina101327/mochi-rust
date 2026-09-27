//! 独立、不抢焦点的桌面卡片。不注入 Explorer，也不跨进程嵌套窗口。
mod chat;
mod composer;
mod context;
mod dock;
pub mod folder;
#[cfg(test)]
mod folder_tests;
#[cfg(test)]
mod interaction_tests;
mod navigation;
mod painting;
mod shell_layer;
pub(crate) mod shortcut_icon;
mod shortcuts;
#[cfg(test)]
mod tests;
mod tree;
pub mod widgets;

use crate::{
    gfx::Renderer,
    platform,
    ui::{draw::DrawList, layout::Rect},
};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::{
    core::{w, Result},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
};

pub const MESSAGE: u32 = WM_APP + 80;
pub const READY: u32 = WM_APP + 81;
pub const TIMER: usize = 0x4d44_534b;
const LIVE_TIMER: usize = 0x4d44_4c56;

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub appearance: mochi_core::desktop_cards::Appearance,
    pub id: String,
    pub title: String,
    /// 屏幕像素；下面的尺寸是 DIP。
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub locked: bool,
    pub dark: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub checked: Option<bool>,
    pub meta: mochi_core::desktop_cards::RowMeta,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub folder_selected: std::collections::BTreeSet<String>,
    pub workspace: std::path::PathBuf,
    pub tree_rows: Vec<crate::shell::Row>,
    pub tree_loading: Vec<std::path::PathBuf>,
    pub item_styles: BTreeMap<String, mochi_core::desktop_cards::ItemStyle>,
    pub tab_scroll: Option<usize>,
    pub shortcut_insertion: Option<usize>,
    pub studio: mochi_core::desktop_cards::studio::Studio,
    pub live: widgets::Live,
    pub widget_rows: BTreeMap<String, Vec<Row>>,
    pub widget_offsets: BTreeMap<String, usize>,
    pub chat_offsets: BTreeMap<usize, crate::ui::ai_markdown::Offsets>,
    pub module: Option<mochi_core::desktop_cards::Module>,
    pub presentation: mochi_core::desktop_cards::Presentation,
    pub collapsed: std::collections::BTreeSet<String>,
    pub month_offset: i32,
    pub page_id: String,
    pub tabs: Vec<(String, String)>,
    pub subtitle: String,
    pub rows: Vec<Row>,
    pub empty: String,
    pub error: bool,
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    Capsule,
    Folder {
        page: String,
        command: folder::Command,
        paths: Vec<String>,
    },
    Refresh,
    ManagePage(String),
    ShortcutDelete(String),
    ShortcutMove(String, usize),
    ItemStyle {
        page: String,
        item: String,
        style: mochi_core::desktop_cards::ItemStyle,
    },
    TreeOpen(String),
    Timer(bool),
    AiSend(String),
    AiStop,
    AiSession(Option<String>),
    Drop(Vec<String>),
    Widget(String, mochi_core::desktop_cards::studio::Trigger),
    WidgetRow(String, String),
    Manage,
    Hide,
    Lock,
    Pin,
    Undock,
    CalendarExpanded,
    Page(String),
    Open,
    Row {
        page: String,
        row: String,
        toggle: bool,
    },
    Geometry {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone)]
pub struct Event {
    pub card: String,
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Capsule,
    FolderCommand(folder::Command),
    FolderGroup(String),
    TabScroll(i32),
    TreeToggle(String),
    TreeOpen(String),
    AiComposer,
    AiScroll(usize, crate::ui::ai_markdown::ScrollId, u32),
    AiLink(String),
    AiCopy(String),
    Timer(bool),
    AiHistory,
    AiSend,
    Widget(String, mochi_core::desktop_cards::studio::Trigger),
    WidgetRow(String, String),
    WidgetScroll(String, i32),
    Manage,
    Hide,
    Lock,
    Pin,
    Undock,
    CalendarExpanded,
    Month(i32),
    Folder(String),
    Page(String),
    Open,
    Row(String, bool),
    Previous,
    Next,
}

struct State {
    folder_drag: Option<(f32, f32)>,
    folder_anchor: Option<String>,
    tree: tree::Tree,
    shortcut_drag: Option<shortcuts::Drag>,
    spec: Spec,
    view: View,
    owner: HWND,
    tx: Sender<Event>,
    renderer: Renderer,
    list: DrawList,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    focused: Option<Hit>,
    offset: usize,
    allow_z: bool,
    dragging: bool,
    context_active: bool,
    owned: bool,
    dock: dock::DockState,
    composer: Option<HWND>,
    input_brush: HBRUSH,
    input_font: HFONT,
    input_font_size: i32,
    live_timer_period: u32,
    composer_drag: bool,
    input_anchor: usize,
    /// DWM 会对卡片后面的壁纸做模糊；绘制目标是透明的。
    glass: bool,
}

pub struct Manager {
    windows: BTreeMap<String, HWND>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    layer: shell_layer::Layer,
    spotlight: Option<std::time::Instant>,
}

impl Default for Manager {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            windows: BTreeMap::new(),
            tx,
            rx,
            layer: Default::default(),
            spotlight: None,
        }
    }
}

impl Manager {
    pub fn refresh_theme(&self, dark: bool) {
        for &hwnd in self.windows.values() {
            unsafe {
                if let Some(s) = state(hwnd) {
                    s.spec.dark = dark;
                    apply_frame(hwnd, &s.spec);
                }
            }
            invalidate(hwnd);
        }
    }

    pub fn clear_composer(&self, id: &str) {
        if let Some(&hwnd) = self.windows.get(id) {
            unsafe {
                if let Some(edit) = state(hwnd).and_then(|s| s.composer) {
                    let _ = SetWindowTextW(edit, w!(""));
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn sync(&mut self, owner: HWND, cards: Vec<(Spec, View)>) -> Vec<String> {
        let obsolete: Vec<_> = self
            .windows
            .keys()
            .filter(|id| !cards.iter().any(|(s, _)| &s.id == *id))
            .cloned()
            .collect();
        for id in obsolete {
            if let Some(hwnd) = self.windows.remove(&id) {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
        let mut errors = Vec::new();
        for (spec, view) in cards {
            if let Some(&hwnd) = self.windows.get(&spec.id) {
                let changed = unsafe {
                    if let Some(s) = state(hwnd) {
                        let changed = s.spec != spec || s.view != view;
                        let resize = s.spec.appearance.edge_dock != spec.appearance.edge_dock
                            || s.spec.appearance.capsule != spec.appearance.capsule
                            || s.spec.width != spec.width
                            || s.spec.height != spec.height
                            || s.spec.x != spec.x
                            || s.spec.y != spec.y;
                        if s.view.page_id != view.page_id {
                            s.offset = 0;
                            s.focused = None;
                            s.folder_anchor = None;
                        }
                        let mut view = view;
                        view.tab_scroll = if s.view.page_id == view.page_id {
                            s.view.tab_scroll
                        } else {
                            None
                        };
                        let follow_chat = widgets::has_chat(&view)
                            && (s.view.live.messages.is_empty()
                                || s.offset + 30
                                    >= painting::scroll_max(&s.spec, &s.view, area(hwnd)));
                        if s.view.page_id == view.page_id {
                            s.folder_anchor = s
                                .folder_anchor
                                .take()
                                .filter(|anchor| view.rows.iter().any(|row| &row.id == anchor));
                            view.folder_selected = s
                                .view
                                .folder_selected
                                .iter()
                                .filter(|id| view.rows.iter().any(|r| &r.id == *id))
                                .cloned()
                                .collect();
                            view.widget_offsets = s.view.widget_offsets.clone();
                            if s.view.live.title == view.live.title {
                                view.chat_offsets = s.view.chat_offsets.clone();
                            }
                            view.collapsed = s.view.collapsed.clone();
                            view.month_offset = s.view.month_offset;
                            for row in &view.rows {
                                if row.meta.directory
                                    && !s.view.rows.iter().any(|old| old.meta.path == row.meta.path)
                                {
                                    view.collapsed.insert(row.meta.path.clone());
                                }
                            }
                        } else {
                            s.folder_anchor = None;
                            view.collapsed = view
                                .rows
                                .iter()
                                .filter(|r| r.meta.directory)
                                .map(|r| r.meta.path.clone())
                                .collect();
                        }
                        s.spec = spec.clone();
                        s.view = view;
                        tree::sync(s);
                        if follow_chat {
                            s.offset = painting::scroll_max(&s.spec, &s.view, area(hwnd));
                        }
                        (changed, resize && !s.dragging && !s.context_active)
                    } else {
                        (false, false)
                    }
                };
                if changed.1 {
                    place(hwnd, &spec);
                }
                if changed.0 {
                    unsafe {
                        widgets::sync_input(hwnd);
                    }
                    apply_opacity(hwnd, &spec);
                    dock::configure(hwnd);
                    invalidate(hwnd);
                }
            } else {
                match create(owner, spec.clone(), view, self.tx.clone()) {
                    Ok(hwnd) => {
                        self.windows.insert(spec.id, hwnd);
                    }
                    Err(e) => errors.push(format!("{}：{e}", spec.title)),
                }
            }
        }
        self.arrange(owner);
        errors
    }

    pub fn arrange(&mut self, owner: HWND) {
        if self
            .spotlight
            .is_some_and(|until| std::time::Instant::now() < until)
        {
            return;
        }
        self.spotlight = None;
        let handles: Vec<_> = self
            .windows
            .values()
            .copied()
            .filter(|hwnd| unsafe {
                state(*hwnd)
                    .is_some_and(|s| !s.spec.appearance.edge_dock && !s.spec.appearance.pinned)
            })
            .collect();
        self.layer.arrange(owner, &handles);
    }

    pub fn events(&mut self) -> Vec<Event> {
        self.rx.try_iter().take(64).collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.windows.contains_key(id)
    }

    pub fn locate(&mut self, id: &str) {
        if let Some(&hwnd) = self.windows.get(id) {
            // 用户显式操作可以让卡片获得焦点，用于键盘导航。
            unsafe {
                if let Some(s) = state(hwnd) {
                    s.allow_z = true;
                }
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                let _ = SetForegroundWindow(hwnd);
                if let Some(s) = state(hwnd) {
                    s.allow_z = false;
                }
                let mut info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd,
                    dwFlags: FLASHW_CAPTION,
                    uCount: 3,
                    dwTimeout: 180,
                };
                let _ = FlashWindowEx(&mut info);
            }
            self.spotlight = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
        }
    }

    pub fn repair_positions(&self) {
        for &hwnd in self.windows.values() {
            let spec = unsafe { state(hwnd).map(|s| s.spec.clone()) };
            if let Some(spec) = spec {
                place(hwnd, &spec);
                emit_geometry(hwnd);
            }
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.layer.stop();
        for &hwnd in self.windows.values() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut State> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut() }
}

fn create(owner: HWND, spec: Spec, view: View, tx: Sender<Event>) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let wc = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        lpszClassName: w!("MochiDesktopCard"),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassW(&wc);
    }
    let s = Box::new(State {
        folder_drag: None,
        folder_anchor: None,
        tree: Default::default(),
        shortcut_drag: None,
        spec: spec.clone(),
        view: {
            let mut v = view;
            v.collapsed = v
                .rows
                .iter()
                .filter(|r| r.meta.directory)
                .map(|r| r.meta.path.clone())
                .collect();
            v
        },
        owner,
        tx,
        renderer: Renderer::new()?,
        list: DrawList::new(),
        hover: None,
        pressed: None,
        focused: None,
        offset: 0,
        allow_z: false,
        dragging: false,
        context_active: false,
        owned: false,
        dock: dock::DockState::default(),
        composer: None,
        input_brush: HBRUSH::default(),
        input_font: HFONT::default(),
        input_font_size: 0,
        live_timer_period: 0,
        composer_drag: false,
        input_anchor: 0,
        glass: false,
    });
    let ptr = Box::into_raw(s);
    // 不设 owner：最小化主应用时卡片才不会被一起最小化。
    let result = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_LAYERED
                | if spec.appearance.edge_dock || spec.appearance.pinned {
                    WS_EX_TOPMOST
                } else {
                    WINDOW_EX_STYLE(0)
                },
            w!("MochiDesktopCard"),
            w!("墨池桌面卡片"),
            WS_POPUP | WS_THICKFRAME,
            spec.x,
            spec.y,
            spec.width as i32,
            spec.height as i32,
            None,
            None,
            Some(instance.into()),
            Some(ptr.cast()),
        )
    };
    let hwnd = match result {
        Ok(hwnd) => hwnd,
        Err(e) => {
            unsafe {
                drop(Box::from_raw(ptr));
            }
            return Err(e);
        }
    };
    unsafe {
        if let Some(s) = state(hwnd) {
            s.owned = true;
        }
    }
    place(hwnd, &spec);
    apply_opacity(hwnd, &spec);
    unsafe {
        windows::Win32::UI::Shell::DragAcceptFiles(hwnd, true);
        widgets::sync_input(hwnd);
    }
    unsafe {
        if let Some(s) = state(hwnd) {
            tree::sync(s);
        }
    }
    dock::configure(hwnd);
    #[cfg(not(test))]
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(hwnd)
}

fn apply_opacity(hwnd: HWND, spec: &Spec) {
    // 玻璃卡片上的文字保持不透明：窗口本身没有淡出，DWM 模糊的是
    // 它后面的内容，卡片在上面画一层半透明色调。
    // 分层窗口（layered window）没有模糊，所以开启玻璃时放弃该样式。
    let glass = painting::is_glass(spec) && set_blur_behind(hwnd, true);
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let layered = WS_EX_LAYERED.0 as isize;
        if glass {
            if style & layered != 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !layered);
            }
        } else {
            set_blur_behind(hwnd, false);
            if style & layered == 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | layered);
            }
            let _ = SetLayeredWindowAttributes(
                hwnd,
                windows::Win32::Foundation::COLORREF(0),
                ((spec.appearance.opacity as u32 * 255) / 100) as u8,
                LWA_ALPHA,
            );
        }
        if let Some(s) = state(hwnd) {
            s.glass = glass;
            s.renderer.set_transparent(glass);
        }
    }
    extend_frame(hwnd, glass);
    apply_frame(hwnd, spec);
}

/// 没有玻璃板边框时，DWM 会忽略渲染目标的 alpha 通道，
/// 清除后的像素会变成实心黑，而不是透出模糊。
fn extend_frame(hwnd: HWND, glass: bool) {
    use windows::Win32::{Graphics::Dwm::DwmExtendFrameIntoClientArea, UI::Controls::MARGINS};
    let m = if glass { -1 } else { 0 };
    let margins = MARGINS {
        cxLeftWidth: m,
        cxRightWidth: m,
        cyTopHeight: m,
        cyBottomHeight: m,
    };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

/// 切换 DWM 的「背后模糊」强调效果（`SetWindowCompositionAttribute`，
/// 任务栏和开始菜单用的就是它）。这是未公开 API，因此运行时查找；
/// 返回 `false` 表示系统不支持，卡片保持不透明。
fn set_blur_behind(hwnd: HWND, enable: bool) -> bool {
    use windows::{core::s, Win32::System::LibraryLoader::GetProcAddress};
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct CompositionData {
        attribute: u32,
        data: *mut std::ffi::c_void,
        size: usize,
    }
    type SetComposition = unsafe extern "system" fn(HWND, *mut CompositionData) -> i32;
    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_DISABLED: u32 = 0;
    const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else {
            return false;
        };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else {
            return false;
        };
        let set: SetComposition = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            state: if enable {
                ACCENT_ENABLE_BLURBEHIND
            } else {
                ACCENT_DISABLED
            },
            flags: 0,
            gradient: 0,
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: (&mut policy as *mut AccentPolicy).cast(),
            size: std::mem::size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data) != 0
    }
}

/// Windows 11 会给卡片圆角和系统阴影，让它看起来像桌面小组件
/// 而不是一块方板。可选的描边由 DWM 沿圆角轮廓绘制；
/// 旧系统会忽略这两个属性。
fn apply_frame(hwnd: HWND, spec: &Spec) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
    };
    let border = if spec.appearance.show_border {
        let c = painting::palette(spec).border;
        ((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff)
    } else {
        DWMWA_COLOR_NONE
    };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&DWMWCP_ROUND as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&DWMWCP_ROUND) as u32,
        );
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&border as *const u32).cast(), 4);
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

#[cfg(debug_assertions)]
pub fn snapshot(scenario: &str, path: &std::path::Path) -> Result<()> {
    let width = if scenario.contains("small") { 280 } else { 480 };
    let height = if scenario.contains("small") { 220 } else { 480 };
    let mut renderer = Renderer::new()?;
    let target = renderer.prepare_snapshot(width * 2, height * 2, 192.0)?;
    let spec = Spec {
        appearance: Default::default(),
        id: "test".into(),
        title: "今日工作台".into(),
        x: 0,
        y: 0,
        width,
        height,
        locked: false,
        dark: scenario.contains("dark"),
    };
    let view = View {
        page_id: "schedule".into(),
        tabs: vec![
            ("schedule".into(), "日程".into()),
            ("inbox".into(), "收件箱".into()),
            ("focus".into(), "专注".into()),
        ],
        subtitle: "今天 · 3 项待办".into(),
        rows: vec![
            Row {
                id: "1".into(),
                title: "整理本周学习笔记".into(),
                detail: "今天 14:00 · 学习计划".into(),
                checked: Some(false),
                meta: Default::default(),
            },
            Row {
                id: "2".into(),
                title: "阅读《设计心理学》第三章".into(),
                detail: "今天 16:30 · 阅读".into(),
                checked: Some(false),
                meta: Default::default(),
            },
            Row {
                id: "3".into(),
                title: "回顾本周项目进展与待办".into(),
                detail: "今天 20:00 · 工作".into(),
                checked: Some(false),
                meta: Default::default(),
            },
            Row {
                id: "4".into(),
                title: "已完成：整理收件箱".into(),
                detail: "点击可重新标记为未完成".into(),
                checked: Some(true),
                meta: Default::default(),
            },
        ],
        ..Default::default()
    };
    let mut list = DrawList::new();
    painting::paint(
        &mut list,
        &spec,
        &view,
        Rect::new(0.0, 0.0, width as f32, height as f32),
        0,
        None,
        None,
    );
    renderer.present(HWND::default(), 0xffffff, &list)?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    renderer.save_snapshot(&target, path)?;
    Ok(())
}

fn area(hwnd: HWND) -> Rect {
    let (w, h) = platform::client_size_dips(hwnd);
    Rect::new(0.0, 0.0, w, h)
}

/// 钳制到最近的工作区，已断开和负坐标的显示器也不例外。
fn clamp_geometry(x: i32, y: i32, width: i32, height: i32, work: RECT) -> RECT {
    let w = width.max(1).min((work.right - work.left).max(1));
    let h = height.max(1).min((work.bottom - work.top).max(1));
    let x = x.clamp(work.left, (work.right - w).max(work.left));
    let y = y.clamp(work.top, (work.bottom - h).max(work.top));
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

fn place(hwnd: HWND, spec: &Spec) {
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: spec.x,
                y: spec.y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let r = clamp_geometry(
            spec.x,
            spec.y,
            (spec.width as f32 * scale) as i32,
            (if spec.appearance.capsule {
                44.0
            } else {
                spec.height as f32
            } * scale) as i32,
            info.rcWork,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

fn emit(hwnd: HWND, kind: EventKind) {
    let event = unsafe {
        state(hwnd).map(|s| {
            (
                s.tx.clone(),
                s.owner,
                Event {
                    card: s.spec.id.clone(),
                    kind,
                },
            )
        })
    };
    if let Some((tx, owner, event)) = event {
        if tx.send(event).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(owner), MESSAGE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn emit_geometry(hwnd: HWND) {
    unsafe {
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let width = ((r.right - r.left) as f32 / scale).round() as u32;
        let height = if state(hwnd).is_some_and(|s| s.spec.appearance.capsule) {
            state(hwnd).unwrap().spec.height
        } else {
            ((r.bottom - r.top) as f32 / scale).round() as u32
        };
        if let Some(s) = state(hwnd) {
            s.spec.x = r.left;
            s.spec.y = r.top;
            s.spec.width = width;
            s.spec.height = height;
        }
        emit(
            hwnd,
            EventKind::Geometry {
                x: r.left,
                y: r.top,
                width,
                height,
            },
        );
    }
}

fn activate(hwnd: HWND, hit: Hit) {
    let kind = match hit {
        Hit::Capsule => EventKind::Capsule,
        Hit::FolderCommand(command) => {
            folder::send(hwnd, command);
            return;
        }
        Hit::FolderGroup(name) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    if !s.view.collapsed.remove(&name) {
                        s.view.collapsed.insert(name);
                    }
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::AiScroll(message, region, offset) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    s.view
                        .chat_offsets
                        .entry(message)
                        .or_default()
                        .insert(region, offset as f32);
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::AiComposer => {
            unsafe {
                if let Some(edit) = state(hwnd).and_then(|s| s.composer) {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
                }
            }
            return;
        }
        Hit::AiLink(url) => {
            if url.starts_with("https://") || url.starts_with("http://") {
                platform::open_external(&url);
            }
            return;
        }
        Hit::AiCopy(value) => {
            platform::copy_to_clipboard(&value);
            return;
        }
        Hit::WidgetScroll(node, delta) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    let max = s
                        .view
                        .widget_rows
                        .get(&node)
                        .map_or(0, |rows| rows.len().saturating_sub(1));
                    let offset = s.view.widget_offsets.entry(node).or_default();
                    *offset = if delta > 0 {
                        offset.saturating_add(3).min(max)
                    } else {
                        offset.saturating_sub(3)
                    };
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::WidgetRow(node, row) => EventKind::WidgetRow(node, row),
        Hit::Timer(reset) => EventKind::Timer(reset),
        Hit::Widget(id, trigger) => EventKind::Widget(id, trigger),
        Hit::AiSend => {
            widgets::send(hwnd);
            return;
        }
        Hit::AiHistory => {
            widgets::history(hwnd);
            return;
        }
        Hit::Manage => EventKind::Manage,
        Hit::Hide => EventKind::Hide,
        Hit::Lock => EventKind::Lock,
        Hit::Pin => EventKind::Pin,
        Hit::Undock => {
            dock::toggle_suspended(hwnd);
            return;
        }
        Hit::TreeToggle(path) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    tree::toggle(s, &path);
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::TreeOpen(path) => EventKind::TreeOpen(path),
        Hit::TabScroll(delta) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    navigation::scroll(&s.spec, &mut s.view, area(hwnd), delta);
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::CalendarExpanded => EventKind::CalendarExpanded,
        Hit::Month(delta) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    s.view.month_offset = (s.view.month_offset + delta).clamp(-1200, 1200);
                    s.offset = 0;
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::Folder(path) => {
            unsafe {
                if let Some(s) = state(hwnd) {
                    if !s.view.collapsed.remove(&path) {
                        s.view.collapsed.insert(path);
                    }
                }
            }
            invalidate(hwnd);
            return;
        }
        Hit::Page(id) => EventKind::Page(id),
        Hit::Open => EventKind::Open,
        Hit::Row(row, toggle) => {
            let Some(page) = (unsafe { state(hwnd).map(|s| s.view.page_id.clone()) }) else {
                return;
            };
            EventKind::Row { page, row, toggle }
        }
        Hit::Previous | Hit::Next => {
            let a = area(hwnd);
            unsafe {
                if let Some(s) = state(hwnd) {
                    let capacity = (a.height() * 0.8) as usize;
                    let max = painting::scroll_max(&s.spec, &s.view, a);
                    s.offset = if hit == Hit::Previous {
                        s.offset.saturating_sub(capacity)
                    } else {
                        (s.offset + capacity).min(max)
                    };
                }
            }
            invalidate(hwnd);
            return;
        }
    };
    emit(hwnd, kind);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lp.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCDESTROY => {
                if let Some(s) = state(hwnd) {
                    if !s.input_font.is_invalid() {
                        let _ = DeleteObject(s.input_font.into());
                    }
                    if !s.input_brush.is_invalid() {
                        let _ = DeleteObject(s.input_brush.into());
                    }
                }
                let _ = KillTimer(Some(hwnd), dock::TIMER_ID);
                let _ = KillTimer(Some(hwnd), LIVE_TIMER);
                let _ = KillTimer(Some(hwnd), tree::TIMER_ID);
                let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut State;
                if !p.is_null() && (*p).owned {
                    drop(Box::from_raw(p));
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCCALCSIZE => LRESULT(0),
            WM_ERASEBKGND | WM_NCACTIVATE => LRESULT(1),
            WM_NCPAINT => LRESULT(0),
            WM_ACTIVATE => {
                invalidate(hwnd);
                LRESULT(0)
            }
            shortcut_icon::READY => {
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == tree::TIMER_ID => {
                if let Some(s) = state(hwnd) {
                    if tree::poll(s) {
                        invalidate(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == dock::TIMER_ID => {
                dock::tick(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == LIVE_TIMER => {
                let repaint = state(hwnd).is_some_and(|s| {
                    let now = chrono::Local::now().timestamp();
                    let tick = s.view.live.now != now;
                    s.view.live.now = now;
                    s.view.live.streaming
                        || s.composer.is_some_and(|e| {
                            windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == e
                        })
                        || tick
                });
                if repaint {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_CONTEXTMENU => {
                context::menu(hwnd);
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(if state(hwnd)
                .is_some_and(|s| widgets::has_chat(&s.view) || folder::enabled(&s.view))
            {
                MA_ACTIVATE
            } else {
                MA_NOACTIVATE
            } as isize),
            WM_DROPFILES => {
                widgets::drop_files(hwnd, wp);
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                if let Some(s) = state(hwnd) {
                    let p = painting::palette(&widgets::chat_spec(&s.spec, &s.view));
                    let dc = HDC(wp.0 as *mut _);
                    let rgb = |c: u32| {
                        windows::Win32::Foundation::COLORREF(
                            ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255),
                        )
                    };
                    SetTextColor(dc, rgb(p.foreground));
                    SetBkColor(dc, rgb(p.surface));
                    if !s.input_brush.is_invalid() {
                        let _ = DeleteObject(s.input_brush.into());
                    }
                    s.input_brush = CreateSolidBrush(rgb(p.surface));
                    return LRESULT(s.input_brush.0 as isize);
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = platform::mouse_dips(hwnd, lp);
                if let Some(hit @ Hit::Row(..)) = state(hwnd)
                    .filter(|s| folder::enabled(&s.view))
                    .and_then(|s| painting::hit(s, area(hwnd), x, y))
                {
                    folder::pointer_up(hwnd, &hit);
                    folder::send(hwnd, folder::Command::Open);
                    return LRESULT(0);
                }
                if let Some(Hit::Widget(id, _)) =
                    state(hwnd).and_then(|s| painting::hit(s, area(hwnd), x, y))
                {
                    activate(
                        hwnd,
                        Hit::Widget(id, mochi_core::desktop_cards::studio::Trigger::DoubleClick),
                    );
                }
                LRESULT(0)
            }
            WM_WINDOWPOSCHANGING => {
                if state(hwnd).is_some_and(|s| {
                    !s.allow_z && !s.spec.appearance.edge_dock && !s.spec.appearance.pinned
                }) {
                    (*(lp.0 as *mut WINDOWPOS)).flags |= SWP_NOZORDER;
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCHITTEST => {
                let (x, y) = platform::screen_to_client_dips(
                    hwnd,
                    (lp.0 as u16 as i16) as f32,
                    ((lp.0 >> 16) as u16 as i16) as f32,
                );
                let a = area(hwnd);
                if let Some(s) = state(hwnd) {
                    if !s.spec.locked && (!s.spec.appearance.edge_dock || s.dock.progress >= 1.0) {
                        if s.spec.appearance.capsule {
                            return LRESULT(if painting::hit(s, a, x, y).is_none() {
                                HTCAPTION
                            } else {
                                HTCLIENT
                            } as isize);
                        }
                        let edge = 6.0;
                        let l = x < edge;
                        let r = x >= a.right - edge;
                        let t = y < edge;
                        let b = y >= a.bottom - edge;
                        let resize = match (l, r, t, b) {
                            (true, _, true, _) => HTTOPLEFT,
                            (_, true, true, _) => HTTOPRIGHT,
                            (true, _, _, true) => HTBOTTOMLEFT,
                            (_, true, _, true) => HTBOTTOMRIGHT,
                            (true, _, _, _) => HTLEFT,
                            (_, true, _, _) => HTRIGHT,
                            (_, _, true, _) => HTTOP,
                            (_, _, _, true) => HTBOTTOM,
                            _ => 0,
                        };
                        if resize != 0 {
                            return LRESULT(resize as isize);
                        }
                        if y < 44.0 && painting::hit(s, a, x, y).is_none() {
                            return LRESULT(HTCAPTION as isize);
                        }
                    }
                }
                LRESULT(HTCLIENT as isize)
            }
            WM_GETMINMAXINFO => {
                let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize = POINT {
                    x: (280.0 * scale) as i32,
                    y: (if state(hwnd).is_some_and(|s| s.spec.appearance.capsule) {
                        44.0
                    } else {
                        220.0
                    } * scale) as i32,
                };
                m.ptMaxTrackSize = POINT {
                    x: (720.0 * scale) as i32,
                    y: (900.0 * scale) as i32,
                };
                LRESULT(0)
            }
            WM_ENTERSIZEMOVE => {
                if let Some(s) = state(hwnd) {
                    s.dragging = true;
                }
                LRESULT(0)
            }
            WM_EXITSIZEMOVE => {
                if let Some(s) = state(hwnd) {
                    s.dragging = false;
                }
                emit_geometry(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                if let Some(s) = state(hwnd) {
                    s.renderer
                        .set_window_dpi((wp.0 & 0xffff) as u32, (wp.0 >> 16) as u32);
                }
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                if state(hwnd).is_some_and(|s| s.spec.appearance.edge_dock && !s.dragging) {
                    dock::tick(hwnd);
                } else {
                    emit_geometry(hwnd);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_SIZE => {
                widgets::sync_input(hwnd);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                if let Some(s) = state(hwnd) {
                    s.renderer.resize(
                        (r.right - r.left).max(1) as u32,
                        (r.bottom - r.top).max(1) as u32,
                    );
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                if state(hwnd).is_some_and(|s| s.spec.appearance.edge_dock) {
                    dock::tick(hwnd);
                } else {
                    emit_geometry(hwnd);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                let a = area(hwnd);
                if let Some(s) = state(hwnd) {
                    s.list.clear();
                    painting::paint(
                        &mut s.list,
                        &s.spec,
                        &s.view,
                        a,
                        s.offset,
                        s.hover.as_ref(),
                        s.focused.as_ref(),
                    );
                    composer::paint(s.composer, &mut s.list, &s.spec, &s.view, a);
                    let p = painting::palette(&s.spec);
                    if s.glass {
                        s.list.translucent_fills(p.surface_muted, 0xffffff, 0.10);
                    }
                    let _ = s.renderer.present(hwnd, p.surface, &s.list);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = platform::mouse_dips(hwnd, lp);
                if folder::pointer_move(hwnd, x, y) {
                    return LRESULT(0);
                }
                if shortcuts::pointer_move(hwnd, x, y) {
                    return LRESULT(0);
                }
                if state(hwnd).is_some_and(|s| s.composer_drag) {
                    composer::pointer(hwnd, x, y, true);
                    return LRESULT(0);
                }
                let a = area(hwnd);
                let changed = state(hwnd).is_some_and(|s| {
                    let h = painting::hit(s, a, x, y);
                    let c = s.hover != h;
                    if c {
                        if let Some(Hit::Widget(id, _)) = &s.hover {
                            let _ = s.tx.send(Event {
                                card: s.spec.id.clone(),
                                kind: EventKind::Widget(
                                    id.clone(),
                                    mochi_core::desktop_cards::studio::Trigger::MouseLeave,
                                ),
                            });
                        }
                        if let Some(Hit::Widget(id, _)) = &h {
                            let _ = s.tx.send(Event {
                                card: s.spec.id.clone(),
                                kind: EventKind::Widget(
                                    id.clone(),
                                    mochi_core::desktop_cards::studio::Trigger::MouseEnter,
                                ),
                            });
                        }
                        let _ = PostMessageW(Some(s.owner), MESSAGE, WPARAM(0), LPARAM(0));
                    }
                    s.hover = h;
                    c
                });
                let mut t = windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<
                        windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT,
                    >() as u32,
                    dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                    hwndTrack: hwnd,
                    ..Default::default()
                };
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut t);
                if changed {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if let Some(s) = state(hwnd) {
                    if let Some(Hit::Widget(id, _)) = s.hover.take() {
                        emit(
                            hwnd,
                            EventKind::Widget(
                                id,
                                mochi_core::desktop_cards::studio::Trigger::MouseLeave,
                            ),
                        );
                    }
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = platform::mouse_dips(hwnd, lp);
                folder::pointer_down(hwnd, x, y);
                shortcuts::pointer_down(hwnd, x, y);
                if composer::pointer(hwnd, x, y, false) {
                    if let Some(s) = state(hwnd) {
                        s.composer_drag = true;
                    }
                    windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                    return LRESULT(0);
                }
                let a = area(hwnd);
                if let Some(s) = state(hwnd) {
                    s.pressed = painting::hit(s, a, x, y);
                }
                windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if shortcuts::pointer_up(hwnd) {
                    return LRESULT(0);
                }
                if state(hwnd).is_some_and(|s| std::mem::take(&mut s.composer_drag)) {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                    return LRESULT(0);
                }
                let (x, y) = platform::mouse_dips(hwnd, lp);
                let a = area(hwnd);
                let h = state(hwnd).and_then(|s| {
                    let h = painting::hit(s, a, x, y);
                    let pressed = s.pressed.take();
                    h.filter(|v| Some(v) == pressed.as_ref())
                });
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                if let Some(h) = h {
                    if !folder::pointer_up(hwnd, &h) {
                        activate(hwnd, h);
                    }
                }
                if let Some(s) = state(hwnd) {
                    s.folder_drag = None;
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                if let Some(s) = state(hwnd) {
                    s.folder_drag = None;
                    s.composer_drag = false;
                    s.pressed = None;
                    s.shortcut_drag = None;
                    s.view.shortcut_insertion = None;
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16;
                let a = area(hwnd);
                if msg == WM_MOUSEHWHEEL || wp.0 & 4 != 0 {
                    let mut point = POINT {
                        x: lp.0 as u16 as i16 as i32,
                        y: (lp.0 >> 16) as u16 as i16 as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut point);
                    let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                    if let Some(s) = state(hwnd) {
                        chat::scroll(
                            &s.spec,
                            &mut s.view,
                            a,
                            s.offset,
                            point.x as f32 / scale,
                            point.y as f32 / scale,
                            delta as f32 / 120.0
                                * 48.0
                                * if msg == WM_MOUSEHWHEEL { 1.0 } else { -1.0 },
                        );
                    }
                    invalidate(hwnd);
                    return LRESULT(0);
                }
                if let Some(s) = state(hwnd) {
                    let max = painting::scroll_max(&s.spec, &s.view, a);
                    s.offset = if delta > 0 {
                        s.offset.saturating_sub(36)
                    } else {
                        (s.offset + 36).min(max)
                    };
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if folder::key(hwnd, wp.0) {
                    return LRESULT(0);
                }
                let a = area(hwnd);
                let mut action = None;
                if let Some(s) = state(hwnd) {
                    match wp.0 {
                        9 | 38 | 40 => {
                            let hits = painting::controls(&s.spec, &s.view, a, s.offset);
                            let prev = wp.0 == 38
                                || (wp.0 == 9
                                    && windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                                        0x10,
                                    ) < 0);
                            let at = s
                                .focused
                                .as_ref()
                                .and_then(|h| hits.iter().position(|(_, v)| v == h));
                            if !hits.is_empty() {
                                let i = match (at, prev) {
                                    (Some(i), true) => (i + hits.len() - 1) % hits.len(),
                                    (Some(i), false) => (i + 1) % hits.len(),
                                    (None, true) => hits.len() - 1,
                                    _ => 0,
                                };
                                s.focused = Some(hits[i].1.clone());
                            }
                        }
                        13 | 32 => action = s.focused.clone(),
                        27 => s.focused = None,
                        _ => {}
                    }
                }
                if let Some(h) = action {
                    activate(hwnd, h);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_SETCURSOR if lp.0 & 0xffff == HTCLIENT as isize => {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                let _ = ScreenToClient(hwnd, &mut p);
                let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                let a = area(hwnd);
                let clickable = state(hwnd).is_some_and(|s| {
                    painting::hit(s, a, p.x as f32 / scale, p.y as f32 / scale).is_some()
                });
                if let Ok(c) = LoadCursorW(None, if clickable { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_CLOSE => {
                emit(hwnd, EventKind::Hide);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
