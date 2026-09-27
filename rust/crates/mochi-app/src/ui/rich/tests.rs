use super::*;

#[test]
fn shared_mapping_survives_original_source_drop_and_independent_rebasing() {
    let source = String::from("**中文🙂** 与 $x^2$ 和 \\*转义");
    let map = Mapping::inline(&source, 0..source.len());
    let expected = map
        .units
        .iter()
        .map(|unit| (unit.source, unit.text.as_str().to_owned()))
        .collect::<Vec<_>>();
    let mut moved = map.clone();
    assert!(Rc::ptr_eq(&map.units.data, &moved.units.data));
    drop(source);
    moved.rebase(123);
    assert_eq!(
        map.units
            .iter()
            .map(|unit| (unit.source, unit.text.as_str().to_owned()))
            .collect::<Vec<_>>(),
        expected
    );
    moved.rebase(-123);
    drop(map);
    assert_eq!(
        moved
            .units
            .iter()
            .map(|unit| (unit.source, unit.text.as_str().to_owned()))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(moved.units.iter().any(|unit| unit.text == "$x^2$"));
}

#[test]
fn rebasing_cached_mapping_moves_hidden_wrappers_with_visible_units() {
    let source = "**中文** and [link](note.md)";
    let prefix = "前一段\n\n";
    let combined = format!("{prefix}{source}");
    let mut cached = Mapping::inline(source, 0..source.len());
    cached.rebase(prefix.len() as isize);
    let fresh = Mapping::inline(&combined, prefix.len()..combined.len());
    assert_eq!(cached.wrappers, fresh.wrappers);
    assert_eq!((cached.start, cached.end), (fresh.start, fresh.end));
    assert_eq!(
        cached
            .units
            .iter()
            .map(|u| (u.source, u.text.as_str().to_owned()))
            .collect::<Vec<_>>(),
        fresh
            .units
            .iter()
            .map(|u| (u.source, u.text.as_str().to_owned()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn composing_with_prior_edits_keeps_the_real_undo_history_intact() {
    let original = "**中文** tail";
    let mut buffer = TextBuffer::new(original);
    buffer.set_cursor(buffer.text().len(), false);
    buffer.insert(" first edit");
    let edited = buffer.text().to_owned();
    let at = edited.find("中文").unwrap();
    buffer.set_cursor(at, false);
    buffer.set_cursor(at + "中文".len(), true);
    compose(&mut buffer, "输入😀", "输入😀".len());
    assert!(buffer.display_text().0.contains("输入😀"));
    buffer.cancel_composition();
    assert_eq!(buffer.text(), edited);
    assert!(buffer.undo());
    assert_eq!(buffer.text(), original);
}

fn selected(source: &str, range: Range<usize>) -> TextBuffer {
    let mut b = TextBuffer::new(source);
    b.set_cursor(range.start, false);
    b.set_cursor(range.end, true);
    b
}

#[test]
fn selected_markdown_keeps_adjacent_editor_paragraphs_separate() {
    for (newline, expected) in [("\n", "a\n\nb"), ("\r\n", "a\r\n\r\nb")] {
        let source = format!("a{newline}b");
        let buffer = selected(&source, 0..source.len());
        let selection = buffer.selection();
        assert_eq!(selected_markdown(&buffer), expected);
        assert_eq!(buffer.text(), source);
        assert_eq!(buffer.selection(), selection);
    }
}

#[test]
fn selected_markdown_restores_prefixes_when_copying_multiple_list_items() {
    let source = "- 一\n- 二";
    let start = source.find('一').unwrap();
    let end = source.find('二').unwrap() + '二'.len_utf8();
    let buffer = selected(source, start..end);
    assert_eq!(selected_markdown(&buffer), "- 一\n- 二");
    assert_eq!(buffer.text(), source);
}

#[test]
fn selected_markdown_fences_partial_code_with_language_and_safe_marker() {
    let source = "```rust\nlet s = \"```\";\n后文\n```\n尾";
    let start = source.find("let").unwrap();
    let end = source.find("\n后文").unwrap();
    let buffer = selected(source, start..end);
    assert_eq!(
        selected_markdown(&buffer),
        "````rust\nlet s = \"```\";\n````"
    );
    assert_eq!(buffer.text(), source);
}

#[test]
fn selected_markdown_strips_container_shell_and_keeps_nested_marks() {
    let source =
        "<details open>\r\n<summary>标题</summary>\r\n<u>**内😀**</u>\r\n</details>\r\n后文";
    let body_start = source.find('内').unwrap();
    let body_end = body_start + "内😀".len();
    let partial = selected(source, body_start..body_end);
    assert_eq!(selected_markdown(&partial), "<u>**内😀**</u>");
    assert_eq!(partial.text(), source);

    let parsed = super::super::document::parse_ranged(source);
    let container = parsed
        .blocks
        .iter()
        .find_map(|block| match &block.block {
            Block::Container(panel) => Some(panel),
            _ => None,
        })
        .unwrap();
    let whole = selected(source, container.header.start..container.footer.end);
    assert_eq!(selected_markdown(&whole), "标题\r\n\r\n<u>**内😀**</u>\r\n");
    assert_eq!(whole.text(), source);
}

#[test]
fn selected_code_inside_a_container_keeps_literal_markdown() {
    let source = "<details open>\n<summary>代码示例</summary>\n```rust\nlet s = \"**literal**\";\n```\n</details>";
    let start = source.find("let s").unwrap();
    let end = start + "let s = \"**literal**\";".len();
    let buffer = selected(source, start..end);
    let copied = selected_markdown(&buffer);
    assert_eq!(copied, "```rust\nlet s = \"**literal**\";\n```");
    let whole = selected(source, 0..source.len());
    let copied = selected_markdown(&whole);
    assert!(copied.starts_with("代码示例\n\n"));
    assert!(copied.contains("```rust\nlet s = \"**literal**\";\n```"));
    assert!(!copied.contains("</details>"));
    assert_eq!(whole.text(), source);
}

#[test]
fn formatted_copy_excludes_hidden_frontmatter() {
    let source = "---\ntitle: metadata\n---\n\n**可见正文**";
    let buffer = selected(source, 0..source.len());
    let copied = selected_markdown(&buffer);
    assert!(!copied.contains("metadata"));
    assert!(copied.contains("**可见正文**"));
    assert_eq!(buffer.text(), source);
}

#[test]
fn maps_styled_unicode_links_and_aliases_to_their_label_not_target() {
    for (source, label) in [
        ("**中文😀**", "中文😀"),
        ("[[alias-target|alias]]", "alias"),
        ("<span style=\"color: red\">red</span>", "red"),
        ("[label](label.md)", "label"),
    ] {
        let map = Mapping::inline(source, 0..source.len());
        let start = source.rfind(label).unwrap();
        let start = if source.starts_with('[') && !source.starts_with("[[") {
            1
        } else {
            start
        };
        assert_eq!(map.start, start, "{source}");
        for u in &map.units {
            assert_eq!(u.text.as_str(), &source[u.source.clone()]);
        }
        assert_eq!(
            map.units
                .iter()
                .map(|u| u.text.as_str())
                .collect::<String>(),
            label
        );
    }
}

#[test]
fn partial_deletion_preserves_paired_syntax_and_undo() {
    let source = "**hello** world";
    let mut b = selected(source, 3..11);
    insert(&mut b, "");
    assert_eq!(b.text(), "**h**orld");
    assert!(b.undo());
    assert_eq!(b.text(), source);
    assert_eq!(b.selection(), (3, 11));
    assert!(b.redo());
    assert_eq!(b.text(), "**h**orld");
}

#[test]
fn replacement_across_html_closers_preserves_the_remaining_style() {
    let source = "<u>hello</u> world";
    let mut b = selected(source, 4..14);
    insert(&mut b, "中");
    assert_eq!(b.text(), "<u>h中</u>orld");
}

#[test]
fn deleting_all_visible_text_removes_markers_and_link_targets() {
    for source in [
        "**中文**",
        "[中文](note.md)",
        "<u>中文</u>",
        "<mark><u>中文</u></mark>",
    ] {
        let start = source.find("中文").unwrap();
        let mut b = selected(source, start..start + 6);
        insert(&mut b, "");
        assert_eq!(b.text(), "", "{source}");
        assert!(b.undo());
        assert_eq!(b.text(), source);
    }
}

#[test]
fn backspace_at_mark_end_deletes_a_character_and_then_the_empty_mark() {
    let mut b = selected("**中😀**", 9..9);
    let parsed = super::super::document::parse_ranged(b.text());
    delete(&mut b, true, &parsed);
    assert_eq!(b.text(), "**中**");
    let parsed = super::super::document::parse_ranged(b.text());
    delete(&mut b, true, &parsed);
    assert_eq!(b.text(), "");
}

#[test]
fn deleting_last_character_consumes_its_persisted_marker_without_reparsing() {
    let marker = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    let next_marker = "<!-- mochi:block block_00000000-0000-4000-8000-000000000002 -->";
    let source = format!("{marker}\r\n中\r\n{next_marker}\r\n后");
    let at = source.find('中').unwrap() + '中'.len_utf8();
    let mut buffer = selected(&source, at..at);
    let parsed = super::super::document::parse_ranged(buffer.text());
    super::super::document::reset_parse_count();
    delete(&mut buffer, true, &parsed);
    assert_eq!(
        buffer.text(),
        format!("{next_marker}\r\n后"),
        "the removed block's marker/newline must not become an orphan"
    );
    assert_eq!(super::super::document::parse_count(), 0);
    let document = mochi_blocks::model::Document::import("editor", buffer.text()).unwrap();
    assert_eq!(
        document.blocks[0].id.as_str(),
        "block_00000000-0000-4000-8000-000000000002"
    );
}

#[test]
fn merging_marked_paragraphs_consumes_only_the_merged_block_markers() {
    let one = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    let two = "<!-- mochi:block block_00000000-0000-4000-8000-000000000002 -->";
    let three = "<!-- mochi:block block_00000000-0000-4000-8000-000000000003 -->";
    let source = format!("{one}\n甲\n{two}\n乙\n{three}\n丙");
    let at = source.find("乙").unwrap();
    let mut buffer = selected(&source, at..at);
    let parsed = super::super::document::parse_ranged(buffer.text());
    delete(&mut buffer, true, &parsed);
    assert_eq!(buffer.text(), format!("{one}\n甲乙\n{three}\n丙"));
    let at = buffer.text().find("丙").unwrap();
    buffer.set_cursor(at, false);
    let parsed = super::super::document::parse_ranged(buffer.text());
    delete(&mut buffer, true, &parsed);
    assert_eq!(buffer.text(), format!("{one}\n甲乙丙"));
    let document = mochi_blocks::model::Document::import("editor", buffer.text()).unwrap();
    assert_eq!(document.blocks.len(), 1);
    assert_eq!(
        document.blocks[0].id.as_str(),
        "block_00000000-0000-4000-8000-000000000001"
    );
}

#[test]
fn entering_at_a_marked_block_start_moves_the_marker_with_the_old_content() {
    let marker = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    for (source, body, expected_prefix) in [
        (
            format!("{marker}\r\n中文"),
            "中文",
            format!("\r\n{marker}\r\n"),
        ),
        (
            format!("{marker}\n- 中文"),
            "中文",
            format!("- \n{marker}\n- "),
        ),
    ] {
        let at = source.find(body).unwrap();
        let mut buffer = selected(&source, at..at);
        enter(&mut buffer, false);
        assert!(
            buffer.text().starts_with(&expected_prefix),
            "marker moved across the new block boundary: {}",
            buffer.text()
        );
        let document = mochi_blocks::model::Document::import("editor", buffer.text()).unwrap();
        let block = document
            .blocks
            .iter()
            .find(|block| block.id.as_str() == "block_00000000-0000-4000-8000-000000000001")
            .unwrap();
        assert!(block.content.contains(body));
    }
}

#[test]
fn deleting_formula_and_divider_consumes_their_markers() {
    let marker = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    for body in ["$$x^2$$", "---"] {
        let source = format!("{marker}\n{body}\n后文");
        let at = source.find(body).unwrap();
        let mut buffer = selected(&source, at..at);
        let parsed = super::super::document::parse_ranged(buffer.text());
        delete(&mut buffer, false, &parsed);
        assert_eq!(buffer.text(), "\n后文", "{body}");
        assert!(!buffer.text().contains("mochi:block"));
        mochi_blocks::model::Document::import("editor", buffer.text()).unwrap();
    }
}

#[test]
fn copying_a_marked_document_never_leaks_hidden_ids() {
    let one = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    let two = "<!-- mochi:block block_00000000-0000-4000-8000-000000000002 -->";
    let source = format!("{one}\r\n中文\r\n{two}\r\n<u>表格</u>");
    let buffer = selected(&source, 0..source.len());
    let copied = selected_markdown(&buffer);
    assert!(!copied.contains("mochi:block"));
    assert!(copied.contains("中文"));
    assert!(copied.contains("<u>表格</u>"));
    assert_eq!(buffer.text(), source);
}

#[test]
fn typing_from_ctrl_home_skips_a_leading_marker_without_reparsing() {
    let marker = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    let source = format!("{marker}\n正文");
    let mut buffer = TextBuffer::new(&source);
    let parsed = super::super::document::parse_ranged(buffer.text());
    super::super::document::reset_parse_count();
    insert_parsed(&mut buffer, "前", &parsed);
    assert_eq!(
        buffer.text(),
        format!("{marker}\n前正文"),
        "input at byte zero must remain in the marked original block"
    );
    assert_eq!(super::super::document::parse_count(), 0);
    let document = mochi_blocks::model::Document::import("editor", buffer.text()).unwrap();
    assert_eq!(
        document.blocks[0].id.as_str(),
        "block_00000000-0000-4000-8000-000000000001"
    );
}

#[test]
fn list_enter_continues_and_empty_enter_exits_with_one_undo() {
    for (source, expected) in [
        ("- 一", "- 一\n- "),
        ("2. 二", "2. 二\n3. "),
        ("- [x] 任务", "- [x] 任务\n- [ ] "),
        ("> 引用", "> 引用\n> "),
    ] {
        let mut b = selected(source, source.len()..source.len());
        enter(&mut b, false);
        assert_eq!(b.text(), expected);
        enter(&mut b, false);
        assert!(
            !b.text().ends_with("- ") && !b.text().ends_with("> ") && !b.text().ends_with("3. ")
        );
        assert!(b.undo());
        assert_eq!(b.text(), expected);
    }
}

#[test]
fn enter_splits_a_mark_and_exits_headings_at_the_end() {
    let mut b = selected("**abcd**", 4..4);
    enter(&mut b, false);
    assert_eq!(b.text(), "**ab**\n**cd**");
    let mut b = selected("# **标题**", 10..10);
    enter(&mut b, false);
    assert_eq!(b.text(), "# **标题**\n");
}

#[test]
fn join_paragraphs_and_lift_list_preserve_frontmatter() {
    let source = "---\ntitle: 保留\n---\n前段\n\n- 后段";
    let at = source.find("后段").unwrap();
    let mut b = selected(source, at..at);
    delete(&mut b, true, &super::super::document::parse_ranged(source));
    assert_eq!(b.text(), source.replace("- 后段", "后段"));
    let parsed = super::super::document::parse_ranged(b.text());
    delete(&mut b, true, &parsed);
    assert_eq!(b.text(), source.replace("前段\n\n- 后段", "前段\n后段"));
    let parsed = super::super::document::parse_ranged(b.text());
    delete(&mut b, true, &parsed);
    assert_eq!(b.text(), source.replace("前段\n\n- 后段", "前段后段"));
}

#[test]
fn blank_line_deletion_removes_only_one_boundary_and_can_be_undone() {
    for newline in ["\n", "\r\n"] {
        for (prefix, suffix) in [
            ("", ""),
            ("**前文**", "后文"),
            ("---\ntitle: 保留\n---\n前文", ""),
        ] {
            for backward in [false, true] {
                let source = format!("{prefix}{}{suffix}", newline.repeat(3));
                let at = prefix.len() + if backward { newline.len() * 3 } else { 0 };
                let mut buffer = selected(&source, at..at);
                for remaining in (0..3).rev() {
                    let before = buffer.text().to_owned();
                    let cursor = buffer.cursor();
                    let parsed = super::super::document::parse_ranged(&before);
                    delete(&mut buffer, backward, &parsed);
                    let expected = format!("{prefix}{}{suffix}", newline.repeat(remaining));
                    assert_eq!(buffer.text(), expected);
                    assert_eq!(
                        buffer.cursor(),
                        cursor - if backward { newline.len() } else { 0 }
                    );
                    assert!(buffer.undo());
                    assert_eq!(buffer.text(), before);
                    assert_eq!(buffer.cursor(), cursor);
                    assert!(buffer.redo());
                    assert_eq!(buffer.text(), expected);
                }
            }
        }
    }
}

#[test]
fn typing_marks_do_not_turn_into_horizontal_rules() {
    let mut b = TextBuffer::new("");
    apply_format(&mut b, super::super::format::Format::Bold);
    let live = super::super::live::layout(b.text(), Some(b.cursor()), 400.0, &|_| None);
    assert!(live.caret(b.cursor()).is_some());
    assert!(!live
        .parsed
        .blocks
        .iter()
        .any(|b| matches!(b.block, Block::Divider)));
    insert(&mut b, "中文");
    assert_eq!(
        text::parse_inline(b.text())[0].emphasis.base(),
        text::Emphasis::Bold
    );
}

#[test]
fn ime_replacement_can_cancel_without_data_loss_and_commit_in_one_undo() {
    let source = "**hello** world";
    let mut b = selected(source, 3..11);
    compose(&mut b, "zhongwen", 8);
    assert_eq!(b.text(), source);
    assert!(!b.dirty());
    assert_eq!(b.display_text().0, "**hzhongwen**orld");
    b.cancel_composition();
    assert_eq!(b.text(), source);
    assert_eq!(b.selection(), (3, 11));
    compose(&mut b, "zhongwen", 8);
    b.cancel_composition();
    insert(&mut b, "中文😀");
    assert_eq!(b.text(), "**h中文😀**orld");
    b.undo();
    assert_eq!(b.text(), source);
    assert_eq!(b.selection(), (3, 11));
}

#[test]
fn nested_mark_rendering_mapping_and_deletion_agree() {
    for source in ["***中文***", "**加粗和*斜体***", "<u>**中文**</u>"] {
        let map = Mapping::inline(source, 0..source.len());
        let shown = text::parse_inline(source)
            .iter()
            .map(|r| r.text.as_str())
            .collect::<String>();
        assert!(!shown.contains('*'), "{shown}");
        assert_eq!(
            map.units
                .iter()
                .map(|u| u.text.as_str())
                .collect::<String>(),
            shown
        );
        let mut b = selected(source, map.start..map.end);
        insert(&mut b, "");
        assert_eq!(b.text(), "", "{source}");
    }
}

#[test]
fn enter_at_both_edges_of_a_mark_preserves_the_pair() {
    let mut b = selected("**bold** tail", 6..6);
    enter(&mut b, false);
    assert_eq!(b.text(), "**bold**\n tail");
    let mut b = selected("**bold** tail", 2..2);
    enter(&mut b, false);
    assert_eq!(b.text(), "\n**bold** tail");
}

#[test]
fn toggling_existing_bold_preserves_text_on_both_sides() {
    for source in ["**abcdef**", "<strong>abcdef</strong>"] {
        let at = source.find('c').unwrap();
        let mut b = selected(source, at..at + 2);
        apply_format(&mut b, super::super::format::Format::Bold);
        let spans = text::parse_inline(b.text());
        assert_eq!(
            spans.iter().map(|r| r.text.as_str()).collect::<String>(),
            "abcdef"
        );
        assert!(spans
            .iter()
            .any(|r| r.text == "cd" && r.emphasis == text::Emphasis::None));
        b.undo();
        assert_eq!(b.text(), source);
    }
}

#[test]
fn italic_selection_uses_stable_rich_markup_and_keeps_text_visible() {
    let source = "前文 selected text 后文";
    let start = source.find("selected").unwrap();
    let end = start + "selected text".len();
    let mut b = selected(source, start..end);
    apply_format(&mut b, super::super::format::Format::Italic);
    assert_eq!(b.text(), "前文 <em>selected text</em> 后文");
    assert_eq!(b.selected_text(), "selected text");
    assert_eq!(
        text::parse_inline(b.text())
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>(),
        source
    );
    apply_format(&mut b, super::super::format::Format::Italic);
    assert_eq!(b.text(), source);
}

#[test]
fn italic_markup_keeps_an_entire_cjk_paragraph_visible() {
    let source = "提示词与 Skills";
    let mut buffer = selected(source, 0..source.len());
    apply_format(&mut buffer, super::super::format::Format::Italic);
    let parsed = super::super::document::parse_ranged(buffer.text());
    let layout =
        super::super::document::layout_editor(&parsed.blocks, buffer.text(), None, 600.0, &|_| {
            None
        });
    let visible = layout
        .lines
        .iter()
        .flat_map(|line| line.runs.iter())
        .map(|run| run.text.as_str())
        .collect::<String>();
    assert_eq!(visible, source);
    assert!(layout
        .lines
        .iter()
        .flat_map(|line| &line.runs)
        .any(|run| { run.emphasis.base() == super::super::text::Emphasis::Italic }));
}

#[test]
fn container_boundaries_remain_paired_during_editing() {
    for newline in ["\n", "\r\n"] {
        let source = [
            "before",
            ":::mochi-highlight title=\"Tips\"",
            "inside",
            ":::",
            "after",
        ]
        .join(newline);
        for (at, backwards) in [
            (source.find("inside").unwrap() + 6, false),
            (source.find("after").unwrap(), true),
        ] {
            let mut b = selected(&source, at..at);
            delete(
                &mut b,
                backwards,
                &super::super::document::parse_ranged(&source),
            );
            assert_eq!(b.text(), source, "deletion crossed a container boundary");
        }
        let at = source.find("inside").unwrap();
        let mut b = selected(&source, at..at);
        delete(&mut b, true, &super::super::document::parse_ranged(&source));
        assert_eq!(b.text(), ["before", "inside", "after"].join(newline));
        b.undo();
        assert_eq!(b.text(), source);
        for range in [0..at + 2, at + 2..source.len()] {
            let mut b = selected(&source, range);
            insert(&mut b, "new");
            assert_eq!(
                super::super::containers::scan(b.text()).len(),
                1,
                "{}",
                b.text()
            );
            b.undo();
            assert_eq!(b.text(), source);
        }
    }
}

#[test]
fn typing_and_composing_on_closed_header_open_its_body_atomically() {
    let source = "<details>\n<summary>标题</summary>\n正文\n</details>";
    let mut b = selected(source, 0..0);
    compose(&mut b, "中文", 6);
    assert_eq!(b.text(), source);
    assert!(b.display_text().0.contains("<details open>"));
    b.cancel_composition();
    insert(&mut b, "中文");
    assert!(b.text().contains("中文正文\n</details>"));
    assert_eq!(super::super::containers::scan(b.text()).len(), 1);
    b.undo();
    assert_eq!(b.text(), source);
}

#[test]
fn escaped_punctuation_is_deleted_with_its_escape() {
    let mut b = selected("\\*tail", 2..2);
    let parsed = super::super::document::parse_ranged(b.text());
    delete(&mut b, true, &parsed);
    assert_eq!(b.text(), "tail");
    b.undo();
    assert_eq!(b.text(), "\\*tail");
    let source = "[a\\[b\\]](note.md)";
    let map = Mapping::inline(source, 0..source.len());
    assert_eq!(
        map.units
            .iter()
            .map(|u| u.text.as_str())
            .collect::<String>(),
        "a[b]"
    );
}

fn reference_inline_code_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let delimiter = bytes[start..]
        .iter()
        .take_while(|byte| **byte == b'`')
        .count();
    if delimiter == 0 {
        return None;
    }
    let mut at = start + delimiter;
    while at < bytes.len() {
        let next = source[at..].find('`')? + at;
        let closing = bytes[next..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        if closing >= delimiter {
            return Some(next + delimiter);
        }
        at = next + closing;
    }
    (delimiter == 4 && source.get(start..start + delimiter) == Some("````"))
        .then_some(start + delimiter)
}

#[test]
fn rich_code_closing_index_matches_legacy_scan_including_four_tick_fallback() {
    let alphabet = ['`', 'a', '中', '🙂', '\u{301}'];
    let mut state = 0x243f_6a88_u64;
    for _ in 0..512 {
        let length = (next_rich_test_state(&mut state) % 40) as usize;
        let mut source = String::new();
        for _ in 0..length {
            source.push(
                alphabet[(next_rich_test_state(&mut state) % alphabet.len() as u64) as usize],
            );
        }
        let closings = text::CodeClosings::new(&source, 0);
        for (start, _) in source.char_indices().filter(|(_, ch)| *ch == '`') {
            assert_eq!(
                inline_code_end(&closings, &source, 0, start),
                reference_inline_code_end(&source, start),
                "source={source:?}, start={start}"
            );
        }
    }
    for source in ["````", "````a", "````a``", "````a````", "`a`🙂"] {
        let closings = text::CodeClosings::new(source, 0);
        for (start, _) in source.char_indices().filter(|(_, ch)| *ch == '`') {
            assert_eq!(
                inline_code_end(&closings, source, 0, start),
                reference_inline_code_end(source, start),
                "source={source:?}, start={start}"
            );
        }
    }
}

fn next_rich_test_state(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1);
    *state
}

#[test]
fn long_plain_mapping_compresses_units_and_keeps_offset_queries() {
    let source = "a".repeat(1_000_000);
    let map = Mapping::inline(&source, 0..source.len());

    assert_eq!(map.units.len(), source.len());
    assert!(map.memory_bytes() < source.len() * 2);
    assert_eq!(map.source_at(0), 0);
    assert_eq!(map.source_at(255), 255);
    assert_eq!(map.source_at(256), 256);
    assert_eq!(map.source_at(source.len()), source.len());
    assert_eq!(map.visible_at(0), 0);
    assert_eq!(map.visible_at(255), 255);
    assert_eq!(map.visible_at(256), 256);
    assert_eq!(map.visible_at(source.len()), source.len());

    let mut units = map.units.iter();
    assert_eq!(units.len(), source.len());
    assert_eq!(units.next().unwrap().text, "a");
    assert_eq!(units.next_back().unwrap().text, "a");
    assert_eq!(units.len(), source.len() - 2);
}
