use super::super::draw::DrawCmd;
use super::*;
#[test]
fn rendered_text_selection_tracks_wrapped_runs_and_hides_markdown_delimiters() {
    let result = layout("**粗体** 普通\n第二行", 180.0, false);
    let fragments = result.selectable_text(None);
    assert!(fragments.iter().any(|f| f.text == "粗体"));
    assert!(fragments.iter().all(|f| !f.text.contains("**")));
    let first = result.text_point_at(1.0, 1.0, None).unwrap();
    let last_fragment = fragments.len() - 1;
    let last = TextPoint {
        fragment: last_fragment,
        offset: fragments[last_fragment].text.len(),
    };
    let selected = result.selected_text(first, last, None);
    assert!(selected.contains("粗体"));
    assert!(selected.contains("第二行"));
    assert!(selected.contains('\n'));
    assert!(result
        .text_selection_rects(first, last, None)
        .iter()
        .all(|r| !r.is_empty()));
}

#[test]
fn text_selection_uses_active_horizontal_offset_for_scrolled_code() {
    let result = layout("```rust\nlet selected_value = 1;\n```", 150.0, false);
    assert!(!result.scroll_regions.is_empty());
    let region = &result.scroll_regions[0];
    let mut offsets = Offsets::new();
    offsets.insert(region.id, region.max_x());
    let fragments = result.selectable_text(Some(&offsets));
    let (index, fragment) = fragments
        .iter()
        .enumerate()
        .find(|(_, fragment)| fragment.text.contains("selected"))
        .unwrap();
    let point = result
        .text_point_at(
            fragment.rect.left + 4.0,
            fragment.rect.top + 2.0,
            Some(&offsets),
        )
        .unwrap();
    assert_eq!(point.fragment, index);
    assert!(!result
        .selected_text(
            point,
            TextPoint {
                fragment: index,
                offset: fragment.text.len()
            },
            Some(&offsets)
        )
        .is_empty());
}

#[test]
fn long_reply_only_emits_visible_formulas_and_reveals_them_on_scroll() {
    let source = (0..120)
        .map(|i| format!("$$x_{{{i}}}$$\n\n"))
        .collect::<String>();
    let result = layout(&source, 340.0, false);
    let p = theme::tokens().palette(false);
    let clip = Rect::from_size(0.0, 0.0, 340.0, 120.0);
    let mut list = DrawList::new();
    result.paint(&mut list, (0.0, 0.0), clip, p);
    let math = list
        .cmds()
        .iter()
        .filter_map(|c| {
            if let DrawCmd::Math { tex, .. } = c {
                Some(tex.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(!math.is_empty() && math.len() < 10);
    assert!(math.contains(&"x_{0}"));
    assert!(!math.contains(&"x_{119}"));
    assert!(list.finish().is_ok());
    let mut tail = DrawList::new();
    result.paint(&mut tail, (0.0, -(result.height - 120.0)), clip, p);
    assert!(tail
        .cmds()
        .iter()
        .any(|c| matches!(c,DrawCmd::Math{tex,..}if tex=="x_{119}")));
    assert!(tail.finish().is_ok());
    assert_eq!(result.copy_targets().count(), 120);
}
#[test]
fn wrapped_display_keeps_complete_copy_payload_without_horizontal_scroll() {
    let source = include_str!("../../../tests/fixtures/ai-math-wrap-verification.md");
    let result = layout(source, 340.0, false);
    assert!(result.scroll_regions.is_empty());
    let (_, r, _) = result
        .copy_targets()
        .find(|(_, _, kind)| *kind == CopyKind::Formula)
        .unwrap();
    assert!(r.height() > 35.0);
    assert!(r.width() <= 340.01);
    let action = result.hit_at(r.left + 2.0, r.bottom - 2.0).unwrap();
    assert_eq!(
        result.copy_payload(action).unwrap().text,
        source.split("$$").nth(1).unwrap().trim()
    );
}
#[test]
fn fractional_viewport_sizes_do_not_create_roundoff_scrollbars() {
    let source = include_str!("../../../tests/fixtures/ai-math-wrap-verification.md");
    for width in [301.1, 333.33, 617.3333, 679.9] {
        let result = layout(source, width, false);
        assert!(result.scroll_regions.is_empty(), "width {width}");
    }
}
#[test]
fn horizontal_regions_paint_clipped_and_keep_formula_copy_coordinates() {
    let result = layout(
        include_str!("../../../tests/fixtures/ai-scroll-verification.md"),
        340.0,
        false,
    );
    assert_eq!(result.scroll_regions.len(), 3);
    let offsets = result
        .scroll_regions
        .iter()
        .map(|r| (r.id, r.max_x()))
        .collect::<Offsets>();
    let p = theme::tokens().palette(false);
    for clip in [
        Rect::new(0.0, 0.0, 340.0, 1000.0),
        Rect::new(0.0, 1000.0, 340.0, 1100.0),
    ] {
        let mut list = DrawList::new();
        result.paint_scrolled(
            &mut list,
            (0.0, 0.0),
            clip,
            p,
            None,
            false,
            Some(&offsets),
            Some(result.scroll_regions[1].id),
        );
        assert!(list.finish().is_ok());
    }
    let region = &result.scroll_regions[2];
    let x = region.viewport.left + 10.0;
    let y = (region.viewport.top + region.viewport.bottom) / 2.0;
    let action = result.content_at(x, y, Some(&offsets), false).unwrap();
    let payload = result.copy_payload(action).unwrap();
    assert_eq!(payload.kind, CopyKind::Formula);
    assert!(payload.text.ends_with("x_{30}}{1}"));
    assert!(result
        .content_at(region.viewport.right + 1.0, y, Some(&offsets), false)
        .is_none());
}
#[test]
fn formula_copy_is_original_tex_and_code_gets_own_target() {
    let result = layout(
        "行内 $x^2$\n\n$$\\int_0^1 x dx$$\n\n```tex\n$ignored$\n```",
        420.0,
        false,
    );
    let targets = result.copy_targets().collect::<Vec<_>>();
    assert_eq!(targets.len(), 3);
    assert_eq!(result.copy_payload(targets[0].0).unwrap().text, "x^2");
    assert_eq!(
        result.copy_payload(targets[1].0).unwrap().text,
        "\\int_0^1 x dx"
    );
    assert_eq!(targets[2].2, CopyKind::Code);
    assert_eq!(result.copy_payload(targets[2].0).unwrap().text, "$ignored$");
    for (i, r, _) in targets {
        let x = (r.left + r.right) / 2.0;
        let y = (r.top + r.bottom) / 2.0;
        assert_eq!(result.hit_at(x, y), Some(i));
        assert_eq!(result.hover_at(x, y), Some(i));
    }
}
#[test]
fn table_copy_has_tsv_and_safe_formatted_html() {
    let result = layout(
        "| 名字 | 内容 |\n| --- | --- |\n| **中文** | `a < b & c` |\n| *斜体* | ~~删~~ |",
        400.0,
        false,
    );
    let (i, _, _) = result
        .copy_targets()
        .find(|(_, _, k)| *k == CopyKind::Table)
        .unwrap();
    let data = result.copy_payload(i).unwrap();
    assert_eq!(data.text, "名字\t内容\n中文\ta < b & c\n斜体\t删");
    let html = data.html.as_ref().unwrap();
    assert!(html.contains("<strong>中文</strong>"));
    assert!(html.contains("<code>a &lt; b &amp; c</code>"));
    assert!(html.contains("<em>斜体</em>"));
    assert!(html.contains("<s>删</s>"));
    assert!(!html.contains("<script"));
    assert_eq!(html.matches("<tr>").count(), 3);
}
#[test]
fn table_button_is_hover_only_and_has_priority_over_covered_math() {
    let result = layout("| a | b |\n| --- | --- |\n| x | $y$ |", 300.0, false);
    let (i, r, _) = result
        .copy_targets()
        .find(|(_, _, k)| *k == CopyKind::Table)
        .unwrap();
    let center = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert_eq!(result.hover_at(center.0, center.1), Some(i));
    assert_eq!(result.hit_at(center.0, center.1), Some(i));
    assert_eq!(result.hover_at(2.0, 2.0), Some(i));
    assert!(result.hit_at(2.0, 2.0).is_none());
    let p = theme::tokens().palette(false);
    let mut list = DrawList::new();
    let clip = Rect::new(0.0, 0.0, 300.0, 500.0);
    result.paint(&mut list, (0.0, 0.0), clip, p);
    assert!(!list
        .cmds()
        .iter()
        .any(|c| matches!(c,DrawCmd::Text{text,..}if text=="复制表格")));
    let mut hovered = DrawList::new();
    result.paint_with_hover(&mut hovered, (0.0, 0.0), clip, p, Some(i));
    assert!(hovered.finish().is_ok());
    assert!(hovered
        .cmds()
        .iter()
        .any(|c| matches!(c,DrawCmd::Text{text,..}if text=="复制表格")));
}
fn texts(layout: &Layout) -> Vec<(&str, Rect, TextStyle, Emphasis)> {
    layout
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text {
                rect, run, style, ..
            } => Some((run.text.as_str(), *rect, *style, run.emphasis)),
            _ => None,
        })
        .collect()
}
#[test]
fn headings_and_styles_follow_ai_css_not_document_fonts() {
    let source="# 一级\n\n## 二级\n\n### 三级\n\n#### 四级\n\n正文 **粗体** *斜体* ***两者*** ~~删除~~ `代码`";
    let side = layout(source, 700.0, false);
    let whole = layout(source, 700.0, true);
    let a = texts(&side);
    let b = texts(&whole);
    for (i, factor) in [1.35, 1.2, 1.08, 1.0].iter().enumerate() {
        assert!((a[i].2.font_size() - 14.0 * factor).abs() < 0.001);
        assert!((b[i].2.font_size() - 15.0 * factor).abs() < 0.001);
    }
    assert!(a
        .iter()
        .any(|t| t.0 == "两者" && t.3.base() == Emphasis::BoldItalic));
    assert!(a
        .iter()
        .any(|t| t.0 == "代码" && t.3.base() == Emphasis::Code));
    assert!(a
        .iter()
        .any(|t| t.0 == "删除" && matches!(t.3,Emphasis::Styled{flags,..}if flags&8!=0)));
    assert_eq!(a[0].1.top, 0.0);
    assert!(whole.height > side.height);
}
#[test]
fn tight_lists_preserve_nested_order_and_quoted_text() {
    let result = layout(
        "3. 先\n   - 内层\n4. 后\n\n> 引用\n>\n> 第二段",
        400.0,
        false,
    );
    let t = texts(&result);
    let content = t.iter().map(|t| t.0).collect::<Vec<_>>();
    assert_eq!(content, vec!["先", "内层", "后", "引用", "第二段"]);
    assert!(t[1].1.left > t[0].1.left);
    assert!(t[2].1.top > t[1].1.top);
    assert!(result.items.iter().any(|i| matches!(i, Item::QuoteBar(_))));
}
#[test]
fn tables_have_cells_headers_and_wrapped_row_height() {
    let result = layout(
        "| 标题 | 列 |\n| --- | --- |\n| 一段很长很长很长很长的中文 | 值 |",
        180.0,
        false,
    );
    let cells = result
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Cell(r, h) = i {
                Some((*r, *h))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 4);
    assert!(cells[0].1 && cells[1].1 && !cells[2].1);
    assert_eq!(cells[0].0.bottom, cells[2].0.top);
    assert!(cells[2].0.height() > cells[0].0.height());
    assert!(texts(&result)
        .iter()
        .any(|t| t.0 == "标题" && t.3 == Emphasis::Bold));
    assert_eq!(result.height, cells[2].0.bottom);
}
#[test]
fn code_is_literal_preserves_spaces_and_clips_scroll_viewport() {
    let source = "~~~md\n  **not bold** $not math$\n\n\t尾行\n~~~";
    let result = layout(source, 140.0, false);
    let t = texts(&result);
    assert_eq!(
        t.iter().map(|t| t.0).collect::<Vec<_>>(),
        vec!["  **not bold** $not math$", "", "\t尾行"]
    );
    assert!(t.iter().all(|t| t.3 == Emphasis::None));
    assert!(result
        .items
        .iter()
        .any(|i| matches!(i,Item::PushClip(r)if r.left==0.0&&r.right==140.0)));
    assert!(!result.items.iter().any(|i| matches!(i, Item::Math { .. })));
}
#[test]
fn code_card_exposes_language_highlights_and_copy_payload() {
    let result = layout("```rust\nlet answer = 42; // keep this\n```", 420.0, false);
    let (action, hit, kind) = result
        .copy_targets()
        .find(|(_, _, kind)| *kind == CopyKind::Code)
        .expect("code blocks have a copy action");
    assert_eq!(kind, CopyKind::Code);
    assert_eq!(
        result.copy_payload(action).unwrap().text,
        "let answer = 42; // keep this"
    );
    assert!(result.hover_at(hit.left + 2.0, hit.top + 2.0) == Some(action));
    assert!(result.hit_at(hit.left + 2.0, hit.top + 2.0) == Some(action));
    let card = result
        .items
        .iter()
        .find(|item| matches!(item, Item::CodeBox(_)))
        .unwrap();
    let card = match card {
        Item::CodeBox(rect) => *rect,
        _ => unreachable!(),
    };
    assert!(result.hover_at(card.left + 2.0, card.bottom - 2.0) == Some(action));
    assert!(result.hit_at(card.left + 2.0, card.bottom - 2.0).is_none());

    let p = theme::tokens().palette(false);
    let mut list = DrawList::new();
    result.paint_with_hover(
        &mut list,
        (0.0, 0.0),
        Rect::new(0.0, 0.0, 420.0, 300.0),
        p,
        Some(action),
    );
    assert!(list.cmds().iter().any(|cmd| matches!(
        cmd,
        DrawCmd::Text { text, color, .. }
            if text == "rust" && *color == 0xcbd5e1
    )));
    assert!(list.cmds().iter().any(|cmd| matches!(
        cmd,
        DrawCmd::Text { text, color, .. }
            if text == "let" && *color == highlight::color(highlight::Token::Keyword, true)
    )));
    assert!(list.cmds().iter().any(|cmd| matches!(
        cmd,
        DrawCmd::Text { text, .. } if text == "复制代码"
    )));
    assert!(list.finish().is_ok());
}
#[test]
fn unfinished_fence_keeps_latex_delimiters_literal() {
    let result = layout("```rust\nlet source = r#\"\\(x^2\\)\"#", 320.0, false);
    assert!(!result
        .items
        .iter()
        .any(|item| matches!(item, Item::Math { .. })));
    let (action, _, kind) = result
        .copy_targets()
        .find(|(_, _, kind)| *kind == CopyKind::Code)
        .unwrap();
    assert_eq!(kind, CopyKind::Code);
    assert_eq!(
        result.copy_payload(action).unwrap().text,
        r##"let source = r#"\(x^2\)"#"##
    );
    assert!(texts(&result)
        .iter()
        .any(|(text, _, _, _)| text.contains(r"\(x^2\)")));
}
#[test]
fn long_code_scrolls_only_body_and_keeps_header_action_fixed() {
    let line = "let value = 012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789";
    let source = format!("```rust\n{line}\nsecond line\n```");
    let result = layout(&source, 120.0, false);
    let (action, hit, kind) = result
        .copy_targets()
        .find(|(_, _, kind)| *kind == CopyKind::Code)
        .unwrap();
    let region = result
        .scroll_regions
        .iter()
        .find(|region| region.id.kind == 0)
        .expect("long code has a body scroll region");
    assert!(region.max_x() > 0.0);
    assert_eq!(kind, CopyKind::Code);
    assert_eq!(
        result.hit_at((hit.left + hit.right) / 2.0, (hit.top + hit.bottom) / 2.0),
        Some(action)
    );
    assert!(result.hover_at(region.viewport.left + 2.0, region.viewport.top + 2.0) == Some(action));
    assert!(result
        .hit_at(region.viewport.left + 2.0, region.viewport.top + 2.0)
        .is_none());

    let offsets = [(region.id, region.max_x())]
        .into_iter()
        .collect::<Offsets>();
    let p = theme::tokens().palette(false);
    let mut list = DrawList::new();
    result.paint_scrolled(
        &mut list,
        (0.0, 0.0),
        Rect::new(0.0, 0.0, 120.0, 180.0),
        p,
        Some(action),
        false,
        Some(&offsets),
        None,
    );
    assert!(list.finish().is_ok());
}
#[test]
fn unfinished_math_stays_text_and_paints_with_balanced_clips() {
    let result = layout("正在计算 \\(x^2 + y^2", 320.0, false);
    assert!(!result
        .items
        .iter()
        .any(|item| matches!(item, Item::Math { .. })));
    assert!(texts(&result)
        .iter()
        .any(|(text, _, _, _)| text.contains(r"\(x^2 + y^2")));
    let p = theme::tokens().palette(false);
    let mut list = DrawList::new();
    result.paint(&mut list, (0.0, 0.0), Rect::new(0.0, 0.0, 320.0, 100.0), p);
    assert!(list.finish().is_ok());
}
#[test]
fn display_math_between_text_keeps_source_order_and_no_overlap() {
    let source = "之前 $$x^2$$ 之后\n\n行内 \\(y+1\\) 结束";
    let before = source.to_owned();
    let result = layout(source, 500.0, false);
    let t = texts(&result);
    let first = t.iter().find(|t| t.0.contains("之前")).unwrap();
    let last = t.iter().find(|t| t.0.contains("之后")).unwrap();
    let math = result
        .items
        .iter()
        .find_map(|i| {
            if let Item::Math { rect, tex, .. } = i {
                Some((rect, tex))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(math.1, "x^2");
    assert!(math.0.top >= first.1.bottom);
    assert!(last.1.top >= math.0.bottom);
    assert!(t
        .iter()
        .any(|t| t.0 == "y+1" && t.3.base() == Emphasis::Math));
    assert_eq!(source, before);
}
#[test]
fn painting_keeps_clips_balanced_and_bold_links_accented() {
    let p = theme::tokens().palette(false);
    let result = layout(
        "> [**链接**](https://example.com)\n\n```txt\nlong long long long long\n```",
        160.0,
        false,
    );
    let mut list = DrawList::new();
    result.paint(
        &mut list,
        (20.0, 30.0),
        Rect::new(20.0, 30.0, 180.0, 300.0),
        p,
    );
    assert!(list.finish().is_ok());
    assert!(list.cmds().iter().any(|c|matches!(c,DrawCmd::Text{text,color,emphasis,..}if text=="链接"&&*color==p.accent&&emphasis.base()==Emphasis::Bold)));
}
#[test]
fn links_keep_inline_flow_and_resolve_their_exact_targets() {
    let result = layout(
        "前文 [**链接一**](https://example.com/one) 与 [链接二](mochi://ai-locate?id=2) 后文",
        420.0,
        false,
    );
    let link_items = result
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { rect, run, .. } if run.text.contains("链接") => {
                Some((*rect, run.text.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        link_items
            .iter()
            .filter(|(_, text)| text == "链接一")
            .count(),
        1
    );
    assert_eq!(
        link_items
            .iter()
            .filter(|(_, text)| text == "链接二")
            .count(),
        1
    );
    let first = link_items
        .iter()
        .find(|(_, text)| text == "链接一")
        .unwrap()
        .0;
    assert_eq!(
        result.link_at((first.left + first.right) / 2.0, first.top + 2.0, None),
        Some("https://example.com/one".into())
    );
    assert!(result
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { rect, run, .. } if run.text == "前文 " || run.text == " 与 " =>
                Some(rect.top),
            _ => None,
        })
        .all(|top| (top - first.top).abs() < 0.001));
}
#[test]
fn links_in_horizontal_regions_follow_the_same_scroll_offset_as_text() {
    let result = layout(
        "| 项目 | 内容 |\n| --- | --- |\n| 前置文字前置文字前置文字 [横向链接](https://example.com/scroll) | SECOND_COLUMN_ABCDEFGHIJKLMNOPQRSTUVWXYZ_ABCDEFGHIJKLMNOPQRSTUVWXYZ_END |",
        150.0,
        false,
    );
    let link = result
        .links
        .iter()
        .find(|link| link.target == "https://example.com/scroll")
        .unwrap();
    let scroll = link.scroll.unwrap();
    let region = &result.scroll_regions[scroll];
    assert!(region.max_x() > 0.0);
    let offset = (link.hit.left - region.viewport.left - 10.0).clamp(0.0, region.max_x());
    let offsets = [(region.id, offset)].into_iter().collect::<Offsets>();
    let x = link.hit.left - offset + 2.0;
    assert!(region.viewport.contains(x, link.hit.top + 2.0));
    assert_eq!(
        result.link_at(x, link.hit.top + 2.0, Some(&offsets)),
        Some(link.target.clone())
    );
    assert!(result
        .link_at(
            region.viewport.right + 1.0,
            link.hit.top + 2.0,
            Some(&offsets)
        )
        .is_none());
}
#[test]
fn cache_reuses_only_same_source_geometry_and_shaping_epoch() {
    let first = layout("缓存测试", 200.0, false);
    let same = layout("缓存测试", 200.0, false);
    assert!(Rc::ptr_eq(&first, &same));
    assert!(!Rc::ptr_eq(&first, &layout("缓存测试", 201.0, false)));
    assert!(!Rc::ptr_eq(&first, &layout("缓存测试", 200.0, true)));
    super::super::measurement::invalidate();
    assert!(!Rc::ptr_eq(&first, &layout("缓存测试", 200.0, false)));
    for i in 0..80 {
        layout(&format!("缓存 {i}"), 200.0, false);
    }
    CACHE.with(|c| {
        let c = c.borrow();
        assert!(c.entries.len() <= CACHE_ENTRIES);
        assert!(c.bytes <= CACHE_BYTES);
    });
}
#[test]
fn excessive_depth_and_input_have_visible_bounded_fallback() {
    let result = layout(&format!("{}正文", "> ".repeat(80)), 200.0, false);
    assert!(texts(&result).iter().any(|t| t.0.contains("内容过长")));
    let huge = "x".repeat(4 * 1024 * 1024 + 1);
    let result = layout(&huge, 200.0, false);
    assert!(result.height < 200.0);
    CACHE.with(|c| assert!(!c.borrow().entries.iter().any(|e| e.source == huge)));
}
#[test]
fn inline_code_padding_participates_in_wrap_and_paint() {
    let style = TextStyle::Ai {
        kind: 0,
        standalone: false,
    };
    let result = layout("`abcdefghijklmnopqrst`", 85.0, false);
    let t = texts(&result);
    assert!(t.len() > 1);
    for (_, r, _, _) in &t {
        assert!(r.width() <= 85.01);
    }
    assert_eq!(
        t.iter().map(|t| t.0).collect::<String>(),
        "abcdefghijklmnopqrst"
    );
    let p = theme::tokens().palette(false);
    let mut list = DrawList::new();
    result.paint(&mut list, (0.0, 0.0), Rect::new(0.0, 0.0, 85.0, 300.0), p);
    let drawn = list
        .cmds()
        .iter()
        .find_map(|c| {
            if let DrawCmd::Text { rect, .. } = c {
                Some(rect)
            } else {
                None
            }
        })
        .unwrap();
    assert!(
        (drawn.left - t[0].1.left - text::inline_code_padding(style, Emphasis::Code)).abs() < 0.001
    );
    assert!(list.finish().is_ok());
}
#[test]
fn observed_electron_paragraph_code_and_display_math_geometry() {
    let result = layout(
        "**粗体** 和 *斜体*\n\n```txt\n一\n二\n三\n```\n\n$$\\int_0^1 x^2 dx$$",
        600.0,
        true,
    );
    let card = result
        .items
        .iter()
        .find_map(|i| {
            if let Item::ParagraphCard(r) = i {
                Some(r)
            } else {
                None
            }
        })
        .unwrap();
    assert!((card.height() - (26.25 + 16.0)).abs() < 0.001);
    let code = result
        .items
        .iter()
        .find_map(|i| {
            if let Item::CodeBox(r) = i {
                Some(r)
            } else {
                None
            }
        })
        .unwrap();
    assert!((code.height() - 106.75).abs() < 0.001);
    let math = result
        .items
        .iter()
        .find_map(|i| {
            if let Item::Math {
                rect, tex, size, ..
            } = i
            {
                Some((rect, tex, *size))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(math.1, "\\int_0^1 x^2 dx");
    assert!(math.0.height() > super::super::math_layout::size(math.1, math.2).unwrap().1);
    let small = layout("##### 五级\n\n###### 六级", 600.0, true);
    assert!(texts(&small).iter().all(|t| t.2
        == TextStyle::Ai {
            kind: 0,
            standalone: true
        }));
}
#[test]
fn table_intrinsic_columns_do_not_force_equal_widths() {
    let result = layout(
        "| 短 | 这一列有很长很长的内容 |\n| --- | --- |\n| 值 | 还有比较长的另一行内容 |",
        500.0,
        false,
    );
    let cells = result
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Cell(r, _) = i {
                Some(r)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(cells[0].width() < cells[1].width());
    assert!((cells[1].right - 500.0).abs() < 0.001);
}
