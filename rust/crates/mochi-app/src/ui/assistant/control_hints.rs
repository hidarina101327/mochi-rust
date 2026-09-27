//! 绘制 AI 助手控件的提示信息。
use super::*;

pub(super) fn control_tooltip(hit: Hit, streaming: bool) -> Option<&'static str> {
    match hit {
        Hit::Conversations => Some("会话"),
        Hit::PreviousQuestion => Some("上一个问题"),
        Hit::SearchToggle => Some("搜索会话内容"),
        Hit::NewSession => Some("新建会话"),
        Hit::Clear => Some("清空当前会话"),
        Hit::Settings | Hit::OpenSettings => Some("AI 设置"),
        Hit::AgentConfig => Some("Agent配置"),
        Hit::Close => Some("关闭面板"),
        Hit::Float => Some("切换悬浮窗"),
        Hit::SearchPrevious => Some("上一个匹配"),
        Hit::SearchNext => Some("下一个匹配"),
        Hit::SearchClose => Some("关闭搜索"),
        Hit::AttachFiles => Some("附加文件"),
        Hit::MountSession => Some("挂载当前会话到文档"),
        Hit::ApproveMode => Some("修改需确认"),
        Hit::AutomaticMode => Some("自动执行修改"),
        Hit::AgentPicker => Some("选择智能体与追问频率"),
        Hit::Send => Some(if streaming {
            "停止生成"
        } else {
            "发送消息"
        }),
        Hit::ScrollToBottom => Some("回到底部"),
        _ => None,
    }
}

pub(super) fn paint_control_tooltip(
    list: &mut DrawList,
    area: Rect,
    rect: Rect,
    label: &str,
    p: &Palette,
) {
    let width = (text::measure(label, TextStyle::Tiny) + 16.0).clamp(56.0, 190.0);
    let left = (rect.left + rect.width() / 2.0 - width / 2.0).clamp(
        area.left + 4.0,
        (area.right - width - 4.0).max(area.left + 4.0),
    );
    // 只有较宽的布局才在头部控件下方有空间。紧凑面板尽量把提示放在
    // 上方，避免压住搜索行和消息内容。
    let top = if rect.top - area.top >= 34.0 {
        rect.top - 30.0
    } else {
        rect.bottom + 4.0
    };
    let tooltip = Rect::new(left, top, left + width, top + 24.0);
    list.rounded_rect(tooltip, 5.0, p.surface_muted);
    list.rounded_border(tooltip, 5.0, p.border);
    list.text_aligned(tooltip, label, TextStyle::Tiny, p.foreground, Align::Center);
}
