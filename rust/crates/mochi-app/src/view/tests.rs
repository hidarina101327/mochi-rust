use super::*;

fn state(view: WorkspaceView) -> ChromeState {
    ChromeState {
        view,
        ..Default::default()
    }
}

#[test]
fn without_a_workspace_nothing_else_matters() {
    // 连首页都不显示：快照是按工作区算的，没有工作区就没有内容
    for view in [
        WorkspaceView::Home,
        WorkspaceView::Editor,
        WorkspaceView::Schedule,
    ] {
        let facts = Some(TabFacts::file(0, true));
        assert_eq!(
            MainContent::resolve(&state(view), false, facts),
            MainContent::Welcome
        );
    }
}

#[test]
fn the_editor_view_shows_a_hint_instead_of_falling_back_to_home() {
    // 关掉最后一个标签不该弹回首页——首页藏着左侧栏，弹回去就再也点不开文件
    assert_eq!(
        MainContent::resolve(&state(WorkspaceView::Editor), true, None),
        MainContent::NoTab
    );
}

#[test]
fn source_mode_wins_over_the_rendered_document() {
    let s = state(WorkspaceView::Editor);
    let rendered = Some(TabFacts::file(2, false));
    let source = Some(TabFacts::file(2, true));
    assert_eq!(
        MainContent::resolve(&s, true, rendered),
        MainContent::Document
    );
    assert_eq!(MainContent::resolve(&s, true, source), MainContent::Source);
}

#[test]
fn a_special_tab_wins_over_both_document_modes() {
    // 设置页标签上 source_mode 没有意义，special 必须先判
    let s = state(WorkspaceView::Editor);
    let facts = Some(TabFacts {
        index: 0,
        source_mode: true,
        special: true,
    });
    assert_eq!(MainContent::resolve(&s, true, facts), MainContent::Special);
}

#[test]
fn the_home_view_ignores_whatever_tab_is_open() {
    // 首页独占主区，即使背后开着一个源码模式的标签
    let facts = Some(TabFacts::file(0, true));
    assert_eq!(
        MainContent::resolve(&state(WorkspaceView::Home), true, facts),
        MainContent::Home
    );
}

#[test]
fn standalone_views_carry_which_view_they_are() {
    for view in [
        WorkspaceView::Recent,
        WorkspaceView::Schedule,
        WorkspaceView::MochiAi,
        WorkspaceView::AgentConfig,
        WorkspaceView::Inbox,
    ] {
        assert_eq!(
            MainContent::resolve(&state(view), true, Some(TabFacts::file(0, false))),
            MainContent::Standalone(view)
        );
        assert!(
            placeholder_text(MainContent::Standalone(view)).is_none(),
            "{view:?} 由真实面板处理空态"
        );
    }
}

#[test]
fn only_content_with_a_layout_scrolls() {
    assert!(MainContent::Home.scrolls());
    assert!(MainContent::Document.scrolls());
    assert!(MainContent::Source.scrolls());
    assert!(!MainContent::Welcome.scrolls(), "滚一片提示文字没有意义");
    assert!(!MainContent::NoTab.scrolls());
}

#[test]
fn every_empty_state_says_something() {
    assert!(placeholder_text(MainContent::Welcome).is_some());
    assert!(placeholder_text(MainContent::NoTab).is_some());
    // 有内容的三种自己画，不需要占位文字
    assert!(placeholder_text(MainContent::Document).is_none());
}

#[test]
fn a_fresh_home_pane_reports_no_layout_and_survives_being_painted() {
    let mut pane = HomePane::default();
    let area = Rect::new(0.0, 0.0, 800.0, 600.0);
    let mut list = DrawList::new();
    pane.paint(&mut list, area, theme::tokens().palette(false));
    // 没有快照时给的是「正在统计」，不是空白
    assert!(!list.cmds().is_empty(), "空快照也要画出提示");
    // 也不该因为没有 page 就把滚动改坏
    pane.scroll_by(area, 100.0);
    assert_eq!(pane.scroll, 0.0);
}

#[test]
fn the_document_cache_keys_on_the_tab_the_width_and_the_content() {
    let mut pane = DocPane::default();
    let wide = Rect::new(0.0, 0.0, 900.0, 600.0);
    pane.ensure(wide, Some(0), Some("# 标题\n正文"), None);
    assert_eq!(pane.headings().len(), 1);
    let key = pane.key;

    // 同一个标签、同一个宽度、同一正文 → 不重排
    pane.ensure(wide, Some(0), Some("# 标题\n正文"), None);
    assert_eq!(pane.key, key);

    // 换标签 → 重排
    pane.ensure(wide, Some(1), Some("没有标题"), None);
    assert_eq!(pane.headings().len(), 0);

    // 换宽度 → 重排
    pane.ensure(
        Rect::new(0.0, 0.0, 500.0, 600.0),
        Some(1),
        Some("## 二级"),
        None,
    );
    assert_eq!(pane.headings().len(), 1);

    // 内容变长（敲了字）→ 重排
    pane.ensure(
        Rect::new(0.0, 0.0, 500.0, 600.0),
        Some(1),
        Some("## 二级\n新段"),
        None,
    );
    assert_eq!(pane.live.parsed.blocks.len(), 2);
}

#[test]
fn sub_pixel_width_changes_do_not_force_a_relayout() {
    // 拖窗口时宽度是连续的浮点数；不取整的话每一帧都判定为「变了」
    let mut pane = DocPane::default();
    pane.ensure(
        Rect::new(0.0, 0.0, 900.0, 600.0),
        Some(0),
        Some("# 标题"),
        None,
    );
    let key = pane.key;
    pane.ensure(
        Rect::new(0.0, 0.0, 900.4, 600.0),
        Some(0),
        Some("# 标题"),
        None,
    );
    assert_eq!(pane.key, key);
}

#[test]
fn equal_length_replacement_always_relayouts_before_hit_testing() {
    // 等长替换也必须自动失效。否则旧文本的行位置会被拿去命中新文本，
    // 视觉选区与实际源码选区就会分叉。
    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    pane.ensure(area, Some(0), Some("# 甲"), None);
    assert_eq!(pane.headings()[0].text, "甲");

    pane.ensure(area, Some(0), Some("# 乙"), None);
    assert_eq!(pane.headings()[0].text, "乙");

    document::reset_parse_count();
    pane.ensure(area, Some(0), Some("# 丙"), None);
    assert_eq!(pane.headings()[0].text, "丙");
    assert_eq!(document::parse_count(), 1, "等长替换也应重新解析一次");
}

#[test]
fn buffer_revisions_distinguish_replacements_clones_and_undo() {
    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let mut first = TextBuffer::new("# 甲");
    pane.ensure_buffer(area, Some(0), &first, None);
    document::reset_parse_count();
    first.set_cursor(first.text().len(), false);
    pane.ensure_buffer(area, Some(0), &first, None);
    assert_eq!(
        document::parse_count(),
        0,
        "caret movement must reuse geometry"
    );
    let second = TextBuffer::new("# 乙");
    pane.ensure_buffer(area, Some(0), &second, None);
    assert_eq!(pane.headings()[0].text, "乙");
    let mut cloned = first.clone();
    cloned.insert("A");
    first.insert("B");
    assert_ne!(cloned.revision(), first.revision());
    pane.ensure_buffer(area, Some(0), &cloned, None);
    assert_eq!(pane.headings()[0].text, "甲A");
    pane.ensure_buffer(area, Some(0), &first, None);
    assert_eq!(pane.headings()[0].text, "甲B");
    first.undo();
    pane.ensure_buffer(area, Some(0), &first, None);
    assert_eq!(pane.headings()[0].text, "甲");
}

#[test]
fn progressive_open_remains_editable_and_finishes_with_identical_geometry() {
    let source = (0..4_000)
        .map(|i| format!("## Heading {i}\n\n正文 {i} **中文🙂** and `code` words words words.\n\n"))
        .collect::<String>();
    let mut buffer = TextBuffer::new(source);
    let mut pane = DocPane::default();
    pane.set_code_document(Path::new("progressive.md"));
    pane.full_documents.insert(PathBuf::from("progressive.md"));
    pane.set_progressive(true);
    let area = Rect::new(0.0, 0.0, 600.0, 800.0);
    pane.ensure_buffer(area, Some(0), &buffer, None);
    assert!(
        pane.is_loading(),
        "opening should yield before all blocks are laid out"
    );
    assert!(
        !pane.live.layout.lines.is_empty(),
        "the first chunk is immediately visible"
    );
    buffer.set_cursor(buffer.text().find("正文").unwrap(), false);
    pane.insert(&mut buffer, "输入🙂 ");
    pane.ensure_buffer(area, Some(0), &buffer, Some(buffer.cursor()));
    assert!(
        pane.live.caret(buffer.cursor()).is_some(),
        "loaded text stays editable during loading"
    );
    buffer.set_cursor(buffer.text().len(), false);
    pane.ensure_buffer(area, Some(0), &buffer, Some(buffer.cursor()));
    let mut steps = 0;
    while pane.is_loading() {
        pane.advance_loading(area);
        steps += 1;
        assert!(steps < 10_000, "every chunk must make forward progress");
    }
    assert!(pane.take_ready_loading_caret(buffer.cursor()));
    let mut full = DocPane::default();
    full.set_code_document(Path::new("progressive.md"));
    full.full_documents.insert(PathBuf::from("progressive.md"));
    full.ensure_buffer(area, Some(0), &buffer, None);
    assert_eq!(pane.live.parsed, full.live.parsed);
    assert_eq!(pane.live.source(), buffer.text());
    assert_eq!(pane.live.layout.lines.len(), full.live.layout.lines.len());
    for (a, b) in pane.live.layout.lines.iter().zip(&full.live.layout.lines) {
        assert!((a.y - b.y).abs() <= 0.1);
        let mut a = a.clone();
        a.y = b.y;
        assert_eq!(&a, b);
    }
    assert!((pane.live.layout.height - full.live.layout.height).abs() <= 0.1);
}

#[test]
fn progressive_code_collapse_and_container_geometry_match_full_layout() {
    let source = "# Title\n\n:::mochi-highlight title=\"Panel\"\n正文🙂\n\n<!-- mochi-code-block title=\"Hidden\" collapsed=\"true\" -->\n```rust\nlet a = 1;\nlet b = 2;\n```\n\n后文\n:::\n\n## Next\n\n| A | B |\n| - | - |\n| x | y |\n".repeat(60);
    let buffer = TextBuffer::new(source);
    let area = Rect::new(0.0, 0.0, 600.0, 800.0);
    let mut chunked = DocPane::default();
    chunked.set_code_document(Path::new("mixed.md"));
    chunked.set_progressive(true);
    chunked.ensure_buffer(area, Some(0), &buffer, None);
    while chunked.is_loading() {
        chunked.advance_loading(area);
    }
    let mut full = DocPane::default();
    full.set_code_document(Path::new("mixed.md"));
    full.ensure_buffer(area, Some(0), &buffer, None);
    assert_eq!(
        chunked.live.layout.lines.len(),
        full.live.layout.lines.len()
    );
    for (a, b) in chunked
        .live
        .layout
        .lines
        .iter()
        .zip(&full.live.layout.lines)
    {
        assert!((a.y - b.y).abs() <= 0.1, "{} vs {}", a.y, b.y);
        let mut a = a.clone();
        a.y = b.y;
        assert_eq!(&a, b);
    }
    assert!((chunked.live.layout.height - full.live.layout.height).abs() <= 0.1);
}

#[test]
fn rendered_chinese_selection_deletes_its_exact_source_text() {
    let source = "管理知识，并持续看见自己的成长。";
    let selected = "持续看见自己的成长";
    let prefix = "管理知识，并";
    let area = Rect::new(0.0, 0.0, 1200.0, 600.0);
    let mut pane = DocPane::default();
    let mut buffer = TextBuffer::new(source);
    pane.ensure(area, Some(0), Some(source), Some(0));

    let line = pane
        .live
        .layout
        .lines
        .iter()
        .find(|line| {
            line.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                == source
        })
        .expect("中文段落应有一条可命中的渲染行");
    let origin_x = area.left + crate::ui::editor_preferences::current().padding_left;
    let start_x = origin_x + line.x + crate::ui::text::measure(prefix, line.style) + 0.1;
    let end_x = start_x + crate::ui::text::measure(selected, line.style);
    let y = area.top + line.y + line.height / 2.0;

    pane.click(area, &mut buffer, 0.0, start_x, y, false);
    pane.click(area, &mut buffer, 0.0, end_x, y, true);
    let start = source.find(selected).unwrap();
    assert_eq!(buffer.selection(), (start, start + selected.len()));

    crate::ui::rich::delete(&mut buffer, false, &pane.live.parsed);
    assert_eq!(
        buffer.text(),
        "管理知识，并。",
        "删除必须作用于视觉选中的原文"
    );
}

#[test]
fn select_all_scopes_code_and_tables_before_the_whole_document() {
    let area = Rect::new(0.0, 0.0, 1200.0, 600.0);
    for (source, cursor_text) in [
        ("前文\n\n```rust\nlet value = 1;\n```\n\n后文", "value"),
        (
            "前文\n\n| 名称 | 数值 |\n| --- | --- |\n| 苹果 | 3 |\n\n后文",
            "苹果",
        ),
    ] {
        let mut pane = DocPane::default();
        let mut buffer = TextBuffer::new(source);
        let cursor = source.find(cursor_text).unwrap();
        pane.ensure(area, Some(0), Some(source), Some(cursor));
        buffer.set_cursor(cursor, false);
        let block = pane.live.parsed.block_at(cursor).unwrap();
        let expected = &pane.live.parsed.blocks[block];

        assert!(
            pane.handle_key(&mut buffer, b'A' as u16, false, true)
                .handled
        );
        assert_eq!(buffer.selection(), (expected.start, expected.end));

        assert!(
            pane.handle_key(&mut buffer, b'A' as u16, false, true)
                .handled
        );
        assert_eq!(buffer.selection(), (0, source.len()));
    }
}

#[test]
fn typing_after_a_clicked_nonfinal_paragraph_uses_that_source_offset() {
    let source = "# Agent\n\nAgent\n\n后续内容";
    let area = Rect::new(0.0, 0.0, 1200.0, 600.0);
    let mut pane = DocPane::default();
    let mut buffer = TextBuffer::new(source);
    pane.ensure(area, Some(0), Some(source), None);

    let line = pane
        .live
        .layout
        .lines
        .iter()
        .find(|line| {
            line.block == 2
                && line
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    == "Agent"
        })
        .expect("middle paragraph should be rendered");
    let x = area.left
        + crate::ui::editor_preferences::current().padding_left
        + line.x
        + crate::ui::text::measure("Agent", line.style);
    let y = area.top + line.y + line.height / 2.0;

    pane.click(area, &mut buffer, 0.0, x, y, false);
    assert_eq!(
        buffer.cursor(),
        source.find("Agent\n\n后续").unwrap() + "Agent".len()
    );
    pane.insert(&mut buffer, "X");
    assert_eq!(buffer.text(), "# Agent\n\nAgentX\n\n后续内容");
}

#[test]
fn clicking_during_an_ime_preview_rebuilds_hit_mapping_from_committed_source() {
    let source = "# Agent\n\nAgent\n\n后续内容";
    let area = Rect::new(0.0, 0.0, 1200.0, 600.0);
    let mut pane = DocPane::default();
    let mut buffer = TextBuffer::new(source);
    // 取消输入法组合后才会处理点击，因此目标位置应使用
    // 已提交文档的几何信息。预览可能会插入
    // 或删除实际显示行，预览中的字节偏移无法
    // 明确映射回源码。
    pane.ensure(area, Some(0), Some(source), Some(0));
    let line = pane
        .live
        .layout
        .lines
        .iter()
        .find(|line| {
            line.block == 2
                && line
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    == "Agent"
        })
        .expect("middle paragraph should be rendered")
        .clone();
    let x = area.left
        + crate::ui::editor_preferences::current().padding_left
        + line.x
        + crate::ui::text::measure("Agent", line.style);
    let y = area.top + line.y + line.height / 2.0;

    buffer.set_cursor(0, false);
    crate::ui::rich::compose(&mut buffer, "预览", "预览".len());
    let shown = buffer.display_text().0.into_owned();
    pane.ensure(area, Some(0), Some(&shown), Some(buffer.display_cursor()));

    pane.click(area, &mut buffer, 0.0, x, y, false);
    let expected = source.find("Agent\n\n后续").unwrap() + "Agent".len();
    assert!(buffer.composition().is_none());
    assert_eq!(buffer.cursor(), expected);
    pane.insert(&mut buffer, "X");
    assert_eq!(buffer.text(), "# Agent\n\nAgentX\n\n后续内容");
}

#[test]
fn quick_note_uses_the_same_rendered_and_source_editor_content() {
    let state = state(WorkspaceView::QuickNote);
    assert_eq!(
        MainContent::resolve(&state, true, Some(TabFacts::file(0, false))),
        MainContent::Document
    );
    assert_eq!(
        MainContent::resolve(&state, true, Some(TabFacts::file(0, true))),
        MainContent::Source
    );
    assert_eq!(MainContent::resolve(&state, true, None), MainContent::NoTab);
}

#[test]
fn code_card_metadata_survives_switching_documents() {
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let first_path = Path::new("notes/first.md");
    let second_path = Path::new("notes/second.md");
    let first =
        "前文\n<!-- mochi-code-block title=\"第一段\" collapsed=\"true\" -->\n```rust\nx\n```\n";
    let second = "另一篇\n<!-- mochi-code-block title=\"第二段\" collapsed=\"false\" -->\n```python\ny\n```\n";
    let first_start = first.find("```rust").unwrap();
    let second_start = second.find("```python").unwrap();
    let header_collapsed = |pane: &DocPane, start: usize| {
        let block = pane
            .live
            .parsed
            .blocks
            .iter()
            .position(|block| block.start == start)
            .unwrap();
        pane.live.layout.lines.iter().any(|line| {
            line.block == block
                && matches!(
                    line.decoration,
                    document::Decoration::CodeHeader {
                        collapsed: true,
                        ..
                    }
                )
        })
    };

    let mut pane = DocPane::default();
    pane.set_code_document(first_path);
    pane.ensure(area, Some(0), Some(first), None);
    assert_eq!(pane.code_title(first_start), "第一段");
    assert!(header_collapsed(&pane, first_start));

    pane.set_code_document(second_path);
    pane.ensure(area, Some(1), Some(second), None);
    assert_eq!(pane.code_title(second_start), "第二段");
    assert!(!header_collapsed(&pane, second_start));

    // 源码注释才是持久状态；切回来时必须重新构建
    // 相同的标题和折叠显示状态，而不依赖每个路径单独保存的映射。
    pane.set_code_document(first_path);
    pane.ensure(area, Some(0), Some(first), None);
    assert_eq!(pane.code_title(first_start), "第一段");
    assert!(header_collapsed(&pane, first_start));
}

#[test]
fn moving_the_cursor_does_not_relayout_when_source_mode_is_off() {
    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let text = "# 标题\n\n正文一行";
    pane.ensure(area, Some(0), Some(text), Some(0));
    let key = pane.key;
    pane.ensure(area, Some(0), Some(text), Some(2));
    assert_eq!(pane.key, key, "同一个块内移动不重排");
    pane.ensure(area, Some(0), Some(text), Some(text.len()));
    assert_eq!(pane.key, key, "所见即所得模式跨块移动也只更新叠层");
    // 移动焦点时保留富文本；只有空行需要精确的源码范围。
    assert!(pane.live.layout.lines.iter().all(|l| l.source.is_some()
        == matches!(
            pane.live.parsed.blocks[l.block].block,
            document::Block::Blank
        )));
}

#[test]
fn moving_between_blocks_relayouts_when_source_mode_is_on() {
    struct Restore(crate::ui::editor_preferences::Preferences);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::ui::editor_preferences::set(self.0.clone());
        }
    }
    let old = crate::ui::editor_preferences::current();
    let _restore = Restore(old.clone());
    let mut prefs = old;
    prefs.live_line_source = true;
    crate::ui::editor_preferences::set(prefs);

    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let text = "# 标题\n\n正文一行";
    pane.ensure(area, Some(0), Some(text), Some(0));
    let key = pane.key;
    pane.ensure(area, Some(0), Some(text), Some(text.len()));
    assert_ne!(pane.key, key, "块源码模式跨块后需要切换活动块的原样排版");
}

#[test]
fn clicking_below_the_heading_lands_in_the_paragraph_not_the_heading() {
    // 真机复现：1200×799 DIP 窗口，编辑内容区从 (478,109) 起；点 (520,220) 应落在第二段
    let mut pane = DocPane::default();
    let area = Rect::new(478.0, 109.0, 1200.0, 780.0);
    let mut buffer = TextBuffer::new("# 两数之和\n\n哈希表一遍扫描。\n");
    pane.ensure(area, Some(0), Some(buffer.text()), None);
    let heading = pane.live.layout.lines[0].clone();
    let para = pane
        .live
        .layout
        .lines
        .iter()
        .find(|l| {
            l.runs
                .first()
                .map(|r| r.text.starts_with("哈希"))
                .unwrap_or(false)
        })
        .unwrap()
        .clone();
    assert!(para.y > heading.y + heading.height);
    // 点在段落行的中间高度
    let y = area.top + para.y + para.height / 2.0;
    pane.click(area, &mut buffer, 0.0, 520.0, y, false);
    let para_start = "# 两数之和\n\n".len();
    assert!(
        buffer.cursor() >= para_start,
        "光标 {} 应在段落内（起点 {para_start}）",
        buffer.cursor()
    );
    // 点在标题与段落之间的空隙：归上一行（标题）末尾附近，但绝不能落在 `#` 与空格之间
    pane.click(
        area,
        &mut buffer,
        0.0,
        520.0,
        area.top + heading.y + heading.height + 2.0,
        false,
    );
    assert_ne!(buffer.cursor(), 1, "`#` 与空格之间不是合法落点");
}

#[test]
fn clicking_a_task_checkbox_toggles_the_source_marker() {
    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let mut buffer = TextBuffer::new("- [ ] 事项\n- [x] 完成");
    pane.ensure(area, Some(0), Some(buffer.text()), None);
    let box_line = pane
        .live
        .layout
        .lines
        .iter()
        .find(|l| l.runs[0].text == "☐")
        .unwrap()
        .clone();
    let x = area.left + crate::ui::editor_preferences::current().padding_left + box_line.x + 4.0;
    let y = area.top + box_line.y + 3.0;
    assert!(pane.toggle_task_at(area, &mut buffer, 0.0, x, y));
    assert_eq!(buffer.text(), "- [x] 事项\n- [x] 完成");
    // 点在文字上不算勾选
    let text_x =
        area.left + crate::ui::editor_preferences::current().padding_left + box_line.x + 60.0;
    assert!(!pane.toggle_task_at(area, &mut buffer, 0.0, text_x, y));
    // 再点一次回到未完成；一步撤销
    pane.invalidate();
    pane.ensure(area, Some(0), Some(buffer.text()), None);
    assert!(pane.toggle_task_at(area, &mut buffer, 0.0, x, y));
    assert_eq!(buffer.text(), "- [ ] 事项\n- [x] 完成");
    buffer.undo();
    assert_eq!(buffer.text(), "- [x] 事项\n- [x] 完成");
}

#[test]
fn clicking_link_text_resolves_the_target_but_plain_text_does_not() {
    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let text = "前面 [文档](https://a.b/c) 后面";
    pane.ensure(area, Some(0), Some(text), None);
    let line = pane.live.layout.lines[0].clone();
    let origin = area.left + crate::ui::editor_preferences::current().padding_left + line.x;
    let y = area.top + line.y + 2.0;
    // 「前面 」之后就是链接文字
    let before = crate::ui::text::measure("前面 ", TextStyle::Body);
    assert_eq!(
        pane.link_at(area, text, 0.0, origin + before + 3.0, y),
        Some("https://a.b/c".to_owned())
    );
    assert_eq!(pane.link_at(area, text, 0.0, origin + 3.0, y), None);
}

#[test]
fn clicking_the_document_places_the_caret_and_typing_edits_the_source() {
    let mut pane = DocPane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let mut buffer = TextBuffer::new("# 标题\n\n正文");
    pane.ensure(area, Some(0), Some(buffer.text()), Some(0));
    // 点在段落那一行（渲染态）的行首
    let para_line = pane
        .live
        .layout
        .lines
        .iter()
        .find(|l| l.runs.first().map(|r| r.text == "正文").unwrap_or(false))
        .unwrap()
        .clone();
    let x = area.left + crate::ui::editor_preferences::current().padding_left + para_line.x + 1.0;
    let y = area.top + para_line.y + 2.0;
    pane.click(area, &mut buffer, 0.0, x, y, false);
    assert_eq!(buffer.cursor(), "# 标题\n\n".len());
    buffer.insert("新");
    assert_eq!(buffer.text(), "# 标题\n\n新正文");
    assert!(buffer.dirty());
}

#[test]
fn the_source_cache_keys_on_content_length_so_typing_relayouts() {
    let mut pane = SourcePane::default();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let mut buffer = TextBuffer::new("abc".to_owned());
    pane.ensure(area, 0, &buffer);
    let key = pane.key;

    buffer.insert("d");
    pane.ensure(area, 0, &buffer);
    assert_ne!(pane.key, key, "内容变长了就该重排");
}
