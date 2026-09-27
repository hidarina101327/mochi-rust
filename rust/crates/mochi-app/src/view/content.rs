//! 根据活动标签页解析主内容类型，并确定其滚动区域。
use super::*;

/// 主区此刻实际渲染哪一种内容。
///
/// 注意这**不是** `WorkspaceView`：后者是用户选的视图（首页/编辑器），
/// 这里还要叠上「有没有打开的标签」「是不是源码模式」「有没有工作区」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainContent {
    /// 还没打开工作区。
    Welcome,
    /// 首页仪表盘。
    Home,
    /// 编辑器视图，但一个标签都没开。
    NoTab,
    /// 渲染后的文档。
    Document,
    /// 源码编辑面（Ctrl+Shift+E 切进来）。
    Source,
    /// 非文件标签（设置页、Agent 配置）。
    Special,
    /// 独立视图：最近 / 日程 / 墨池 AI / 收件箱。
    Standalone(WorkspaceView),
}

impl MainContent {
    /// 判定主区内容。**唯一一处**——绘制、点击、滚轮都问这个函数。
    ///
    /// 顺序有讲究：先看有没有工作区，再看视图，最后才看标签状态。
    /// 曾经按「有没有打开的标签」推导视图，结果关掉最后一个标签就会弹回首页，
    /// 而首页藏着左侧栏，于是再也点不开任何文件。
    pub fn resolve(state: &ChromeState, has_workspace: bool, tab: Option<TabFacts>) -> MainContent {
        if matches!(
            state.view,
            WorkspaceView::Marketplace | WorkspaceView::DesktopCards
        ) {
            return MainContent::Standalone(state.view);
        }
        if !has_workspace {
            return MainContent::Welcome;
        }
        match state.view {
            WorkspaceView::Editor | WorkspaceView::QuickNote => match tab {
                Some(t) if t.special => MainContent::Special,
                Some(t) if t.source_mode => MainContent::Source,
                Some(_) => MainContent::Document,
                None => MainContent::NoTab,
            },
            WorkspaceView::Home => MainContent::Home,
            other => MainContent::Standalone(other),
        }
    }

    /// 这种内容是不是可滚动的。空态不滚——滚一片提示文字没有意义。
    #[cfg(test)]
    pub fn scrolls(self) -> bool {
        matches!(
            self,
            MainContent::Home
                | MainContent::Document
                | MainContent::Source
                | MainContent::Special
                | MainContent::Standalone(_)
        )
    }
}

/// 判定主区内容需要知道的、关于当前标签的全部事实。
///
/// 传值而不是传 `&OpenTab`：这样 `resolve` 不依赖 `shell` 模块，
/// 测试里构造一个判定用例不需要真的开一个工作区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabFacts {
    pub index: usize,
    pub source_mode: bool,
    /// 不是文件（设置页 / Agent 配置）。
    pub special: bool,
}

#[cfg(test)]
impl TabFacts {
    /// 普通文件标签。
    pub fn file(index: usize, source_mode: bool) -> Self {
        TabFacts {
            index,
            source_mode,
            special: false,
        }
    }
}

/// 主区的空态提示。
///
/// 一律给一句话而不是留白——空白面板让人以为程序坏了。
pub fn placeholder(list: &mut DrawList, area: Rect, text: &str, p: &Palette) {
    list.text(
        Rect::new(
            area.left + 48.0,
            area.top + 32.0,
            area.right,
            area.top + 64.0,
        ),
        text,
        TextStyle::Body,
        p.muted,
    );
}

/// 空态该显示哪句话。
pub fn placeholder_text(content: MainContent) -> Option<&'static str> {
    match content {
        MainContent::Welcome => Some("Ctrl+O 打开工作区"),
        MainContent::NoTab => Some("双击左侧文件打开"),
        // 已接通的独立面板自己处理空态；这里只保留异常标签的降级提示。
        MainContent::Special => Some("无法识别此标签，请关闭后重新打开"),
        _ => None,
    }
}
