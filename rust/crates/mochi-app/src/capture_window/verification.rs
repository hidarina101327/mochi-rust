//! 生成捕获窗口各类验证场景的界面快照。
use super::*;
pub fn snapshot(scenario: &str, path: &std::path::Path) -> Result<()> {
    let mut renderer = Renderer::new()?;
    let target = renderer.prepare_snapshot(1200, 780, 144.0)?;
    let dark = scenario.contains("dark");
    let page = if scenario.contains("recent") {
        1
    } else if scenario.contains("favorites") {
        2
    } else if scenario.contains("schedule") {
        3
    } else if scenario.contains("chat") {
        4
    } else {
        0
    };
    let rows = match page {
        1 => vec![
            ("产品设计 · 九月迭代", "知识库 / 产品 · 刚刚修改"),
            ("阅读笔记：把想法变成行动", "个人空间 / 阅读 · 12 分钟前"),
            ("本周复盘", "个人空间 / 日记 · 昨天"),
        ],
        2 => vec![
            ("我的灵感清单", "个人空间 / 灵感"),
            ("项目常用链接", "知识库 / 项目"),
        ],
        3 => vec![
            ("整理本周计划", "2026-09-21 09:00"),
            ("产品设计评审", "2026-09-21 14:30"),
        ],
        4 => vec![
            ("一起梳理新项目", "12 条消息"),
            ("整理这周的阅读笔记", "6 条消息"),
        ],
        _ => vec![],
    };
    let entries = rows
        .into_iter()
        .map(|(title, detail)| Entry {
            title: title.into(),
            detail: detail.into(),
            action: Action::Page(page),
        })
        .collect::<Vec<_>>();
    let mut list = DrawList::new();
    capture_view::paint(
        &mut list,
        Rect::new(0.0, 0.0, 800.0, 520.0),
        Some("mochi"),
        "",
        dark,
        page,
        &entries,
        0,
    );
    renderer.present(
        HWND::default(),
        if dark { 0x20242b } else { 0xf4f7fb },
        &list,
    )?;
    renderer.save_snapshot(&target, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_keep_draft_and_open_selected_item() {
        let handle = Handle::new(HWND::default()).unwrap();
        unsafe {
            let s = state(handle.0).unwrap();
            SetWindowTextW(s.edit, &windows::core::HSTRING::from("保留草稿 😀")).unwrap();
            s.pages[1] = (0..18)
                .map(|i| Entry {
                    title: format!("文档 {i}"),
                    detail: String::new(),
                    action: Action::File(PathBuf::from(format!("{i}.md"))),
                })
                .collect();
            // 编辑文本时保留普通方向键；IME 组合输入也继续使用这些按键。
            assert!(!handle_key(handle.0, s, 39, true));
            s.composing = true;
            assert!(!handle_key(handle.0, s, 39, false));
            s.composing = false;
            assert!(handle_key(handle.0, s, 39, false));
            assert_eq!(s.page, 1);
            assert!(!IsWindowVisible(s.edit).as_bool());
            for _ in 0..30 {
                handle_key(handle.0, s, 40, false);
            }
            assert_eq!(s.selected, 17);
            handle_key(handle.0, s, 13, false);
            assert!(matches!(&s.pending,Some(Action::File(p)) if p == &PathBuf::from("17.md")));
            switch_page(handle.0, s, 0);
            let mut text = [0u16; 64];
            let n = GetWindowTextW(s.edit, &mut text);
            assert_eq!(String::from_utf16_lossy(&text[..n as usize]), "保留草稿 😀");
            handle_key(handle.0, s, 37, false);
            assert_eq!(s.page, 4);
            handle_key(handle.0, s, 39, false);
            assert_eq!(s.page, 0);
        }
    }
}
