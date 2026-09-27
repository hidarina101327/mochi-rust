use super::*;

#[test]
fn indexed_block_and_line_ranges_match_linear_lookup() {
    for source in [
        ":::mochi-highlight\n<details open>\n<summary>x</summary>\ninner\n</details>\ntail\n:::",
        ":::mochi-highlight\n:::\n\n<details>\n<summary>empty</summary>\n</details>",
        "---\ntitle: doc\n---\n# 中文🙂\r\n\r\n```rs\r\nlet a=1;\r\n```\r\n\r\n- item\n- next",
        "<!-- mochi-code-block title=\"test\" -->\n```rs\ncode\n```\n\n| a | b |\n| - | - |\n| x | y |\n",
    ] {
        let parsed = parse_ranged(source);
        assert!(parsed.blocks.windows(2).all(|pair| pair[0].end <= pair[1].end));
        for offset in 0..=source.len() {
            let expected = parsed.blocks.iter().position(|b| b.start <= offset && offset <= b.end)
                .or_else(|| parsed.blocks.iter().rposition(|b| matches!(&b.block, Block::Container(p) if p.header.start <= offset && offset <= p.footer.end)));
            assert_eq!(parsed.block_at(offset), expected, "{source:?} at {offset}");
        }
        let layout = layout_editor(&parsed.blocks, source, None, 300.0, &|_| None);
        for block in 0..=parsed.blocks.len() {
            let expected = layout.lines.iter().filter(|line| line.block == block).cloned().collect::<Vec<_>>();
            assert_eq!(layout.lines_for_block(block), expected);
        }
    }
}

#[test]
fn display_math_fences_preserve_exact_source_and_leave_unclosed_text_alone() {
    let source = "前文\r\n$$\r\n\\alpha + x^2\r\n$$\r\n后文";
    let parsed = parse_ranged(source);
    let rb = &parsed.blocks[1];
    assert!(matches!(&rb.block,Block::Math(t) if t=="\\alpha + x^2"));
    assert_eq!(&source[rb.start..rb.end], "$$\r\n\\alpha + x^2\r\n$$");
    assert!(matches!(&parse("\\[x^2\\]")[0],Block::Math(t) if t=="x^2"));
    assert!(matches!(&parse("$$x^2$$")[0],Block::Math(t) if t=="x^2"));
    assert!(matches!(&parse("$$\n未闭合\n后文")[0], Block::Paragraph(_)));
    assert!(matches!(&parse("```\n$$x$$\n```")[0], Block::Code { .. }));
}

#[test]
fn display_math_matches_electron_inline_node_alignment_and_preserves_source() {
    let source = "$$\n\\alpha + x^2\n$$";
    let parsed = parse_ranged(source);
    let lay = layout_live(&parsed.blocks, source, None, 600.0, &|_| None);
    assert_eq!(lay.lines[0].x, 0.0);
    assert_eq!(lay.lines[0].decoration, Decoration::Math);
    assert_eq!(lay.lines[0].runs[0].text, "\\alpha + x^2");
    let raw = layout_live(&parsed.blocks, source, Some(0), 600.0, &|_| None);
    for line in raw.lines {
        let (start, end) = line.source.unwrap();
        assert_eq!(
            line.runs
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            &source[start..end]
        );
    }
}

#[test]
fn rendered_markdown_separators_collapse_without_losing_active_blank_source() {
    let source = "# Title\n\nparagraph\n\n## Section\n\nbody";
    let spacious = source.replace("\n\n", "\n\n\n\n");
    let parsed = parse_ranged(source);
    let layout = layout_live(&parsed.blocks, source, None, 600.0, &|_| None);
    let other = parse_ranged(&spacious);
    assert_eq!(
        layout.height,
        layout_live(&other.blocks, &spacious, None, 600.0, &|_| None).height
    );
    assert_eq!(
        layout.lines[0].y,
        padding_top(),
        "first-child heading has zero top margin"
    );
    let h2 = layout
        .lines
        .iter()
        .find(|l| l.style == TextStyle::Heading2)
        .unwrap();
    assert_eq!(h2.decoration, Decoration::None);
    assert_eq!(h2.height, TextStyle::Heading2.line_height());
    let blank = other
        .blocks
        .iter()
        .position(|b| matches!(b.block, Block::Blank))
        .unwrap();
    let active = layout_live(&other.blocks, &spacious, Some(blank), 600.0, &|_| None);
    assert!(active
        .lines
        .iter()
        .any(|l| l.block == blank && l.source.is_some()));
}

#[test]
fn cached_rendered_editor_keeps_every_hittable_trailing_blank_row() {
    for source in [
        "正文\n",
        "正文\n\n",
        "正文\n\n\n",
        "正文\n   \n",
        "正文\r\n",
        "正文\r\n\r\n",
        ":::mochi-highlight title=\"提示\"\r\n内容\r\n:::\r\n",
        "",
    ] {
        let parsed = parse_ranged(source);
        let first_trailing = parsed
            .blocks
            .iter()
            .rposition(|block| !matches!(block.block, Block::Blank))
            .map(|index| index + 1)
            .unwrap_or(0);
        let expected = first_trailing..parsed.blocks.len();
        let mut cache = LayoutCache::default();
        let layout =
            layout_editor_cached(&parsed.blocks, source, None, 600.0, &|_| None, &mut cache);
        let blank_rows = layout
            .lines
            .iter()
            .filter(|line| matches!(parsed.blocks[line.block].block, Block::Blank))
            .collect::<Vec<_>>();
        assert_eq!(
            blank_rows.len(),
            expected.len(),
            "cached production layout swallowed a trailing empty paragraph in {source:?}"
        );
        assert_eq!(
            blank_rows.iter().map(|line| line.block).collect::<Vec<_>>(),
            expected.collect::<Vec<_>>()
        );
        assert!(blank_rows.iter().all(|line| line.source.is_some()));
    }
}

#[test]
fn cached_rendered_editor_keeps_physical_middle_empty_rows() {
    for newline in ["\n", "\r\n"] {
        let one = format!("前{newline}{newline}后");
        let many = format!("前{newline}{newline}{newline}{newline}后");
        for (source, expected_visible_blanks) in [(one, 1), (many, 3)] {
            let parsed = parse_ranged(&source);
            let mut cache = LayoutCache::default();
            let layout =
                layout_editor_cached(&parsed.blocks, &source, None, 600.0, &|_| None, &mut cache);
            assert_eq!(
                layout
                    .lines
                    .iter()
                    .filter(|line| matches!(parsed.blocks[line.block].block, Block::Blank))
                    .count(),
                expected_visible_blanks,
                "cached rich layout swallowed a middle empty row in {source:?}"
            );
        }
    }
}

use crate::ui::draw::DrawCmd;
use crate::ui::theme;

const AREA: Rect = Rect {
    left: 480.0,
    top: 68.0,
    right: 1200.0,
    bottom: 776.0,
};

/// 把一行的 run 拼回纯文本，便于断言。
fn plain(line: &LaidOutLine) -> String {
    line.runs.iter().map(|r| r.text.as_str()).collect()
}

fn texts(list: &DrawList) -> Vec<String> {
    list.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn text_payloads_in_window(list: &DrawList, window: Rect) -> Vec<String> {
    list.cmds()
        .iter()
        .filter_map(|command| match command {
            DrawCmd::Text { rect, text, .. } if !rect.intersect(&window).is_empty() => {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn pipe_tables_need_a_separator_row_and_pad_ragged_rows() {
    let b = parse("| 名 | 值 |\n| --- | :-: |\n| a | 1 |\n| b |\n\n后文");
    assert_eq!(
        b[0],
        Block::Table {
            rows: vec![
                vec!["名".into(), "值".into()],
                vec!["a".into(), "1".into()],
                vec!["b".into(), String::new()]
            ],
            header: true,
        }
    );
    assert_eq!(b[1], Block::Blank);
    assert_eq!(b[2], Block::Paragraph("后文".into()));
    // 没有分隔行就只是带竖线的段落
    assert_eq!(parse("a | b\nc | d")[0], Block::Paragraph("a | b".into()));
    // 转义的竖线留在格子里
    assert_eq!(
        split_table_row(r"| x \| y | z |"),
        vec!["x | y".to_owned(), "z".to_owned()]
    );
}

#[test]
fn a_table_lays_out_one_row_per_line_with_the_css_geometry() {
    let lay = layout(&parse("| 名 | 值 |\n| --- | --- |\n| a | 1 |"), 700.0);
    let rows: Vec<&LaidOutLine> = lay
        .lines
        .iter()
        .filter(|l| matches!(l.decoration, Decoration::TableRow { .. }))
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].decoration,
        Decoration::TableRow {
            cols: 2,
            header: true
        }
    );
    assert_eq!(
        rows[1].decoration,
        Decoration::TableRow {
            cols: 2,
            header: false
        }
    );
    assert_eq!(rows[0].height, TABLE_ROW_HEIGHT);
    assert_eq!(rows[1].y - rows[0].y, TABLE_ROW_HEIGHT);
    assert_eq!(
        rows[0].y,
        padding_top() + TABLE_MARGIN,
        "表格上方 20px 外边距"
    );
    assert_eq!(table_column_width(2, 700.0), 350.0, "两列均分填满");
    assert_eq!(table_column_width(8, 700.0), 160.0, "列太多时不低于 160");
    let mut list = DrawList::new();
    paint(&mut list, AREA, &lay, 0.0, theme::tokens().palette(false));
    let t = texts(&list);
    assert!(t.contains(&"名".to_owned()) && t.contains(&"1".to_owned()));
    assert!(list.finish().is_ok());
}

#[test]
fn task_markers_paint_as_boxes_not_glyphs_and_table_cells_honour_inline_markup() {
    let lay = layout(
        &parse("- [ ] 待办\n- [x] 完成\n\n| h |\n| - |\n| **粗** |"),
        700.0,
    );
    let mut list = DrawList::new();
    paint(&mut list, AREA, &lay, 0.0, theme::tokens().palette(false));
    let t = texts(&list);
    assert!(
        !t.iter().any(|s| s == "☐" || s == "☑"),
        "勾选框不该当文字画：{t:?}"
    );
    assert!(t.contains(&"待办".to_owned()));
    assert_eq!(
        list.cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Icon { .. }))
            .count(),
        1,
        "只有已完成的那个画勾"
    );
    assert!(
        t.contains(&"粗".to_owned()) && !t.iter().any(|s| s.contains("**")),
        "单元格里的星号要被吃掉：{t:?}"
    );
    assert!(list.finish().is_ok());
}

#[test]
fn standalone_images_become_image_blocks_sized_from_the_probe() {
    let b = parse("![示意图](./a.png \"标题\")\n文字 ![行内](x.png) 后面");
    assert_eq!(
        b[0],
        Block::Image {
            alt: "示意图".into(),
            src: "./a.png".into(),
            width: None
        }
    );
    assert!(
        matches!(b[1], Block::Paragraph(_)),
        "行内图片不独占一行，仍是段落"
    );
    let ranged = parse_ranged("![a](pic.png)");
    let sized = layout_live(&ranged.blocks, "![a](pic.png)", None, 600.0, &|src| {
        (src == "pic.png").then_some((1200, 300))
    });
    let img = sized
        .lines
        .iter()
        .find(|l| l.decoration == Decoration::Image)
        .unwrap();
    assert_eq!(img.height, 150.0, "1200 宽缩到 600，高按比例 300 → 150");
    assert_eq!(
        img.runs[2].text, "600",
        "绘制/命中宽度应与缩放后的实际宽度一致"
    );
    let natural = layout_live(&ranged.blocks, "![a](pic.png)", None, 600.0, &|src| {
        (src == "pic.png").then_some((200, 100))
    });
    let natural_img = natural
        .lines
        .iter()
        .find(|l| l.decoration == Decoration::Image)
        .unwrap();
    assert_eq!(natural_img.runs[2].text, "200", "小图不应被拉到整行宽度");
    assert_eq!(natural_img.height, 100.0);
    let unknown = layout_live(&ranged.blocks, "![a](pic.png)", None, 600.0, &|_| None);
    assert_eq!(
        unknown
            .lines
            .iter()
            .find(|l| l.decoration == Decoration::Image)
            .unwrap()
            .height,
        IMAGE_PLACEHOLDER_HEIGHT
    );
    // 相对路径按文档目录解析，URL 原样
    assert_eq!(
        resolve_image_src("./a%20b.png", Some(std::path::Path::new(r"D:\ws\笔记"))),
        r"D:\ws\笔记\a b.png"
    );
    assert_eq!(
        resolve_image_src("https://x/y.png", Some(std::path::Path::new(r"D:\ws"))),
        "https://x/y.png"
    );
}

#[test]
fn headings_are_recognised_by_level() {
    let b = parse("# 一级\n## 二级\n### 三级\n#### 四级");
    assert_eq!(
        b,
        vec![
            Block::Heading {
                level: 1,
                text: "一级".into()
            },
            Block::Heading {
                level: 2,
                text: "二级".into()
            },
            Block::Heading {
                level: 3,
                text: "三级".into()
            },
            Block::Heading {
                level: 4,
                text: "四级".into()
            },
        ]
    );
}

#[test]
fn a_hash_without_a_space_is_a_tag_not_a_heading() {
    // 墨池的笔记里 `#标签` 很常见，误判成标题会把整行放大成一级标题
    let b = parse("#标签 在句首");
    assert_eq!(b, vec![Block::Paragraph("#标签 在句首".into())]);
}

#[test]
fn frontmatter_is_skipped_entirely() {
    let b = parse("---\ntitle: 测试\ntags: [a, b]\n---\n\n正文");
    assert!(!b
        .iter()
        .any(|x| matches!(x, Block::Paragraph(p) if p.contains("title"))));
    assert!(b.contains(&Block::Paragraph("正文".into())));
}

#[test]
fn an_unclosed_frontmatter_fence_is_treated_as_content() {
    // 只有一行 `---` 的文件是分隔线，不是残缺的文档头部——
    // 若将其误当作文档头部，就会吞掉整篇文档。
    let b = parse("---\n这是正文不是元数据");
    assert!(b.contains(&Block::Paragraph("这是正文不是元数据".into())));
}

#[test]
fn a_fenced_code_block_keeps_its_lines_verbatim() {
    let b = parse("```rust\nfn main() {\n    println!(\"hi\");\n}\n```");
    assert_eq!(
        b,
        vec![Block::Code {
            lang: "rust".into(),
            lines: vec![
                "fn main() {".into(),
                "    println!(\"hi\");".into(),
                "}".into()
            ],
        }]
    );
}

#[test]
fn markdown_inside_a_code_block_is_not_parsed() {
    // 代码块里的 `# 注释` 是注释，不是标题
    let b = parse("```sh\n# 这是注释\n- 这不是列表\n```");
    assert_eq!(b.len(), 1);
    assert!(matches!(&b[0], Block::Code { lines, .. } if lines.len() == 2));
}

#[test]
fn an_unclosed_code_fence_stays_literal_and_does_not_swallow_the_rest() {
    let b = parse("```\n没有闭合\n还有一行");
    assert_eq!(
        b,
        vec![
            Block::Paragraph("```".into()),
            Block::Paragraph("没有闭合".into()),
            Block::Paragraph("还有一行".into()),
        ]
    );
}

#[test]
fn a_fence_becomes_code_only_after_the_user_starts_the_next_line() {
    assert!(matches!(parse("```" ).as_slice(), [Block::Paragraph(value)] if value == "```"));
    assert!(
        matches!(parse("```\n").as_slice(), [Block::Paragraph(value), Block::Blank] if value == "```")
    );
}

#[test]
fn task_list_items_are_detected_before_plain_bullets() {
    let b = parse("- [ ] 未完成\n- [x] 已完成\n- 普通项");
    let markers: Vec<&str> = b
        .iter()
        .filter_map(|x| match x {
            Block::ListItem { marker, .. } => Some(marker.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(markers, vec!["☐", "☑", "•"]);
}

#[test]
fn ordered_list_markers_follow_the_source_not_a_recount() {
    // 手写 1. 1. 1. 是常见写法，自动重编号会和原文对不上
    let b = parse("1. 甲\n1. 乙\n1. 丙");
    let markers: Vec<&str> = b
        .iter()
        .filter_map(|x| match x {
            Block::ListItem { marker, .. } => Some(marker.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(markers, vec!["1.", "1.", "1."]);
}

#[test]
fn nested_list_items_carry_their_depth() {
    let b = parse("- 顶层\n  - 二层\n    - 三层");
    let depths: Vec<usize> = b
        .iter()
        .filter_map(|x| match x {
            Block::ListItem { depth, .. } => Some(*depth),
            _ => None,
        })
        .collect();
    assert_eq!(depths, vec![0, 1, 2]);
}

#[test]
fn dividers_and_quotes_are_recognised() {
    let b = parse("---\n\n> 引用一句\n\n***");
    assert!(b.contains(&Block::Divider));
    assert!(b.contains(&Block::Quote("引用一句".into())));
    assert_eq!(b.iter().filter(|x| **x == Block::Divider).count(), 2);
}

#[test]
fn blank_lines_survive_so_paragraphs_keep_their_breathing_room() {
    let b = parse("甲\n\n乙");
    assert_eq!(b.len(), 3);
    assert_eq!(b[1], Block::Blank);
}

struct PreferencesGuard(super::super::editor_preferences::Preferences);

impl Drop for PreferencesGuard {
    fn drop(&mut self) {
        super::super::editor_preferences::set(self.0.clone());
    }
}

fn electron_spacing_preferences() -> PreferencesGuard {
    let old = super::super::editor_preferences::current();
    let mut prefs = old.clone();
    prefs.live_line_source = false;
    prefs.paragraph_spacing = 8.0;
    prefs.block_spacing = 20.0;
    prefs.list_spacing = 4.0;
    super::super::editor_preferences::set(prefs);
    PreferencesGuard(old)
}

#[test]
fn live_source_single_line_keeps_following_block_geometry_stable() {
    let old = super::super::editor_preferences::current();
    let _restore = PreferencesGuard(old.clone());
    let mut prefs = old;
    prefs.live_line_source = true;
    // 下划线让位置跳动更明显：源码原本没有预留已渲染标题下划线所占的空间。
    //
    prefs.heading_underline[0] = true;
    super::super::editor_preferences::set(prefs);

    let source = "# 标题\n\n后续段落";
    let parsed = parse_ranged(source);
    let mut cache = LayoutCache::default();
    let rendered = layout_editor_cached(&parsed.blocks, source, None, 600.0, &|_| None, &mut cache);
    let source_layout = layout_editor_cached(
        &parsed.blocks,
        source,
        Some(0),
        600.0,
        &|_| None,
        &mut cache,
    );
    let y_for = |layout: &Layout, block| {
        layout
            .lines
            .iter()
            .find(|line| line.block == block)
            .map(|line| line.y)
            .unwrap()
    };
    assert_eq!(rendered.height, source_layout.height);
    assert_eq!(y_for(&rendered, 2), y_for(&source_layout, 2));
    assert!(source_layout.lines.iter().any(|line| {
        line.block == 0 && line.source.is_some() && line.runs[0].text == "# 标题"
    }));
}

#[test]
fn ordered_list_numbers_leave_space_before_wrapped_text() {
    let _prefs = electron_spacing_preferences();
    let source =
        "9. 九\n10. 番茄钟\n99. 九十九\n100. 一百\n    1000. 嵌套列表的长正文需要换行并且保持对齐";
    let parsed = parse_ranged(source);
    let layout = layout_editor(&parsed.blocks, source, None, 240.0, &|_| None);
    for bi in 0..parsed.blocks.len() {
        let rows = layout.lines_for_block(bi);
        let marker = &rows[0];
        let body = &rows[1..];
        assert!(!body.is_empty());
        let marker_end = marker.x + text::measure_runs(&marker.runs, marker.style);
        assert!(body[0].x >= marker_end + marker.style.font_size() * 0.35 - 0.001);
        assert!(body.iter().all(|line| line.x == body[0].x));
    }
    assert!(
        layout.lines_for_block(4).len() > 2,
        "nested item should wrap"
    );
}

#[test]
fn cached_rich_layout_keeps_empty_rows_stable_across_focus_changes() {
    let _prefs = electron_spacing_preferences();
    let source = "- 一\n\n- 二\n\n- 三";
    let parsed = parse_ranged(source);
    let mut cache = LayoutCache::default();
    let layout = layout_editor_cached(&parsed.blocks, source, None, 600.0, &|_| None, &mut cache);
    let items = layout
        .lines
        .iter()
        .filter(|line| line.visible_start == usize::MAX)
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 3, "each bullet has one marker row");
    assert_eq!(
        layout
            .lines
            .iter()
            .filter(|line| matches!(parsed.blocks[line.block].block, Block::Blank))
            .count(),
        2
    );

    // 聚焦空行时，不能增删其他行或移动无关
    // 内容。之前的正式缓存会折叠除当前行以外的所有行，
    // 导致连续按 Enter 看起来没有效果。
    let blank = parsed
        .blocks
        .iter()
        .position(|block| matches!(block.block, Block::Blank))
        .unwrap();
    let source_layout = layout_editor_cached(
        &parsed.blocks,
        source,
        Some(blank),
        600.0,
        &|_| None,
        &mut cache,
    );
    assert_eq!(
        source_layout
            .lines
            .iter()
            .filter(|line| {
                line.source.is_some() && matches!(parsed.blocks[line.block].block, Block::Blank)
            })
            .count(),
        2
    );
    assert_eq!(layout.height, source_layout.height);
    let first_item_y = layout
        .lines
        .iter()
        .find(|line| line.block == 0 && line.visible_start == usize::MAX)
        .map(|line| line.y)
        .unwrap();
    let active_first_item_y = source_layout
        .lines
        .iter()
        .find(|line| line.block == 0 && line.visible_start == usize::MAX)
        .map(|line| line.y)
        .unwrap();
    assert_eq!(
        first_item_y, active_first_item_y,
        "entering a later blank must not move preceding blocks"
    );

    // 源码行模式保留相同的空行几何位置，同时显示
    // 当前非空块的 Markdown 标记。
    let mut source_prefs = super::super::editor_preferences::current();
    source_prefs.live_line_source = true;
    super::super::editor_preferences::set(source_prefs);
    let source_layout = layout_editor_cached(
        &parsed.blocks,
        source,
        Some(blank),
        600.0,
        &|_| None,
        &mut cache,
    );
    assert_eq!(
        source_layout
            .lines
            .iter()
            .filter(|line| {
                line.source.is_some() && matches!(parsed.blocks[line.block].block, Block::Blank)
            })
            .count(),
        2
    );
}

#[test]
fn persisted_marker_lines_and_crlf_do_not_split_a_visual_list() {
    let _prefs = electron_spacing_preferences();
    let marker =
        |id: u8| format!("<!-- mochi:block block_00000000-0000-4000-8000-00000000000{id} -->");
    let marked = format!(
        "{}\r\n- one\r\n{}\r\n- two\r\n{}\r\n- three",
        marker(1),
        marker(2),
        marker(3)
    );
    let plain = "- one\r\n- two\r\n- three";
    let parsed = parse_ranged(&marked);
    assert_eq!(
        parsed
            .blocks
            .iter()
            .filter(|block| matches!(block.block, Block::ListItem { .. }))
            .count(),
        3
    );
    assert!(parsed.blocks.iter().all(|block| {
        !matches!(&block.block, Block::Paragraph(text) if text.contains("mochi:block"))
    }));

    let mut marked_cache = LayoutCache::default();
    let marked_layout = layout_editor_cached(
        &parsed.blocks,
        &marked,
        None,
        600.0,
        &|_| None,
        &mut marked_cache,
    );
    let plain_parsed = parse_ranged(plain);
    let mut plain_cache = LayoutCache::default();
    let plain_layout = layout_editor_cached(
        &plain_parsed.blocks,
        plain,
        None,
        600.0,
        &|_| None,
        &mut plain_cache,
    );
    assert_eq!(marked_layout.height, plain_layout.height);
    assert_eq!(
        marked_layout
            .lines
            .iter()
            .map(|line| line.y)
            .collect::<Vec<_>>(),
        plain_layout
            .lines
            .iter()
            .map(|line| line.y)
            .collect::<Vec<_>>()
    );
}

#[test]
fn reading_layout_collapses_repeated_markdown_blank_lines_but_separates_blocks() {
    let _prefs = electron_spacing_preferences();
    let one = "- one\r\n\r\n- two";
    let many = "- one\r\n\r\n\r\n\r\n- two";
    let one_parsed = parse_ranged(one);
    let many_parsed = parse_ranged(many);
    let one_layout = layout_live(&one_parsed.blocks, one, None, 600.0, &|_| None);
    let many_layout = layout_live(&many_parsed.blocks, many, None, 600.0, &|_| None);
    assert_eq!(one_layout.height, many_layout.height);
    assert_eq!(
        one_layout
            .lines
            .iter()
            .map(|line| line.y)
            .collect::<Vec<_>>(),
        many_layout
            .lines
            .iter()
            .map(|line| line.y)
            .collect::<Vec<_>>()
    );

    let paragraph = layout_live(
        &parse_ranged("甲\n\n乙").blocks,
        "甲\n\n乙",
        None,
        600.0,
        &|_| None,
    );
    let first = &paragraph.lines[0];
    let second = &paragraph.lines[1];
    assert_eq!(
        second.y - (first.y + first.height),
        8.0,
        "one Markdown separator keeps one paragraph margin"
    );
}

#[test]
fn block_margins_collapse_against_previous_paragraph_like_css_margins() {
    let _prefs = electron_spacing_preferences();
    let source = "段落\n\n> 引用\n\n```text\n代码\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |";
    let parsed = parse_ranged(source);
    let layout = layout_live(&parsed.blocks, source, None, 600.0, &|_| None);
    let paragraph = layout
        .lines
        .iter()
        .find(|line| line.runs.first().is_some_and(|run| run.text == "段落"))
        .unwrap();
    let quote = layout
        .lines
        .iter()
        .find(|line| line.decoration == Decoration::QuoteBar)
        .unwrap();
    assert_eq!(
        quote.y - (paragraph.y + paragraph.height),
        20.0,
        "blockquote margin should collapse with paragraph margin"
    );
    let code = layout
        .lines
        .iter()
        .find(|line| matches!(line.decoration, Decoration::CodeHeader { .. }))
        .unwrap();
    assert_eq!(
        code.y - (quote.y + quote.height),
        20.0,
        "code card margin should be one block margin"
    );
    let table = layout
        .lines
        .iter()
        .find(|line| matches!(line.decoration, Decoration::TableRow { .. }))
        .unwrap();
    let code_end = layout
        .lines
        .iter()
        .filter(|line| matches!(line.decoration, Decoration::CodeBackground { .. }))
        .next_back()
        .unwrap();
    assert_eq!(
        table.y - (code_end.y + code_end.height),
        code_pad_y() + 1.0 + CODE_MARGIN,
        "table should not add a second block margin after the code card"
    );
}

#[test]
fn a_long_paragraph_is_wrapped_into_several_visual_lines() {
    let long = "墨池是一个 AI 原生的桌面笔记应用".repeat(6);
    let doc = layout(&parse(&long), 400.0);
    assert!(
        doc.lines.len() > 3,
        "应当断成多行，实际 {}",
        doc.lines.len()
    );
    // 每一行都在最大行宽以内
    for line in &doc.lines {
        assert!(text::measure_runs(&line.runs, line.style) <= max_line_width() + 1.0);
    }
}

#[test]
fn lines_advance_monotonically_and_never_overlap() {
    // 布局最容易错的地方：某个块忘了推进 y，后面的内容叠上去
    let doc = layout(
        &parse("# 标题\n\n段落一\n\n- 列表\n- 列表二\n\n> 引用\n\n```\n代码\n```\n\n---\n\n结尾"),
        600.0,
    );
    // 布局最容易错的地方：某个块忘了推进 y，后面的内容叠上去。
    //
    // 唯一允许共享 y 的是列表项的「符号 + 首行文字」——它们本就该在同一条
    // 基线上，靠 x 分开。所以判据是：y 不倒退，且同 y 的两行 x 必须不同。
    assert!(doc.lines.len() > 5);
    for pair in doc.lines.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(b.y >= a.y - 0.01, "行 {:?} 的 y 倒退了", plain(b));
        if b.y < a.y + a.height - 0.01 {
            assert!(
                (b.x - a.x).abs() > 0.01,
                "行 {:?} 与 {:?} 完全重叠在 y={}",
                plain(b),
                plain(a),
                b.y
            );
        }
    }
    let last = doc.lines.last().unwrap();
    assert!(doc.height > last.y + last.height - 0.01);
}

#[test]
fn a_heading_takes_more_vertical_space_than_a_paragraph() {
    let h = layout(&parse("# 标题"), 600.0);
    let p = layout(&parse("标题"), 600.0);
    assert!(h.height > p.height);
}

#[test]
fn code_lines_wrap_by_default_and_keep_their_code_indent() {
    let long_code = format!("```\n{}\n```", "x".repeat(400));
    let doc = layout(&parse(&long_code), 300.0);
    let code: Vec<&LaidOutLine> = doc
        .lines
        .iter()
        .filter(|l| matches!(l.decoration, Decoration::CodeBackground { .. }))
        .collect();
    assert!(code.len() > 1, "默认应在代码卡片内自动换行");
    assert_eq!(
        code.iter()
            .map(|line| line
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>())
            .collect::<String>(),
        "x".repeat(400)
    );
    assert_eq!(
        code.first().unwrap().decoration,
        Decoration::CodeBackground {
            line_number: Some(1)
        }
    );
}

#[test]
fn code_overflow_and_line_number_preferences_change_the_layout() {
    let old = super::super::editor_preferences::current();
    let _restore = PreferencesGuard(old.clone());
    let long_code = format!("```\n{}\n```", "x".repeat(400));
    let mut prefs = old;
    prefs.code_wrap = false;
    prefs.code_show_line_numbers = false;
    super::super::editor_preferences::set(prefs);

    let doc = layout(&parse(&long_code), 300.0);
    let code = doc
        .lines
        .iter()
        .filter(|line| matches!(line.decoration, Decoration::CodeBackground { .. }))
        .collect::<Vec<_>>();
    assert_eq!(code.len(), 1, "横向滚动模式不应改写代码结构");
    assert_eq!(code[0].x, code_pad_x(), "隐藏行号后正文应贴回内边距");
    assert_eq!(
        code[0].decoration,
        Decoration::CodeBackground { line_number: None }
    );
    assert!(code_horizontal_overflow(&doc, 0, 300.0) > 0.0);

    let mut scrolled = doc.clone();
    for line in &mut scrolled.lines {
        if matches!(line.decoration, Decoration::CodeBackground { .. }) {
            line.x -= 40.0;
        }
    }
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &scrolled,
        0.0,
        theme::tokens().palette(false),
    );
    let code_y = code[0].y;
    assert!(list.cmds().iter().any(|command| {
        matches!(command, DrawCmd::PushClip { rect }
            if (rect.top - (AREA.top + code_y)).abs() < 0.01
                && (rect.left - (AREA.left + padding_x() + code_pad_x())).abs() < 0.01)
    }));
    assert!(list.finish().is_ok());
}

#[test]
fn only_the_visible_slice_is_painted() {
    // 一篇几千行的笔记，画完整篇会把每帧拖垮
    let long = (0..2000)
        .map(|i| format!("第 {i} 行"))
        .collect::<Vec<_>>()
        .join("\n");
    let doc = layout(&parse(&long), content_width(AREA));
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, AREA, &doc, 0.0, &p);

    let drawn = texts(&list).len();
    assert!(drawn > 0);
    assert!(drawn < 60, "只该画视口内的行，实际画了 {drawn} 行");
}

#[test]
fn scrolling_moves_which_lines_are_visible() {
    let long = (0..500)
        .map(|i| format!("第 {i} 行"))
        .collect::<Vec<_>>()
        .join("\n");
    let doc = layout(&parse(&long), content_width(AREA));
    let p = *theme::tokens().palette(false);

    let mut top = DrawList::new();
    paint(&mut top, AREA, &doc, 0.0, &p);
    let mut scrolled = DrawList::new();
    paint(&mut scrolled, AREA, &doc, 2000.0, &p);

    assert!(texts(&top).contains(&"第 0 行".to_owned()));
    assert!(!texts(&scrolled).contains(&"第 0 行".to_owned()));
    assert!(!texts(&scrolled).is_empty());
}

#[test]
fn clipped_paint_keeps_nested_backgrounds_and_text_equivalent() {
    let source = ":::mochi-highlight color=\"blue\" title=\"外层\"\n外层中文\n<details open>\n<summary>内层</summary>\n内层中文 😀\n</details>\n```rust\nlet 中文 = \"😀\";\n```\n:::";
    let parsed = parse_ranged(source);
    let layout = layout_live(&parsed.blocks, source, None, content_width(AREA), &|_| None);
    let code_header = layout
        .lines
        .iter()
        .find(|line| matches!(line.decoration, Decoration::CodeHeader { .. }))
        .unwrap();
    let viewport = Rect::new(0.0, 0.0, 720.0, 180.0);
    let scroll = code_header.y + code_header.height + code_pad_y() * 0.5;
    let full_area = Rect::new(
        viewport.left,
        viewport.top,
        viewport.right,
        viewport.top + layout.height + viewport.height(),
    );
    let palette = theme::tokens().palette(false);
    let mut full = DrawList::new();
    paint(&mut full, full_area, &layout, 0.0, palette);
    let mut clipped = DrawList::new();
    paint(&mut clipped, viewport, &layout, scroll, palette);

    let expected_window = Rect::new(
        viewport.left,
        viewport.top + scroll,
        viewport.right,
        viewport.bottom + scroll,
    );
    assert_eq!(
        text_payloads_in_window(&clipped, viewport),
        text_payloads_in_window(&full, expected_window),
        "clipped text should match the full render's visible slice"
    );
    assert!(
        text_payloads_in_window(&clipped, viewport)
            .iter()
            .any(|text| text.contains("中文") || text.contains("😀")),
        "the CJK/emoji body should remain visible after scrolling"
    );
    assert!(clipped.cmds().iter().any(
        |command| matches!(command, DrawCmd::RoundedRect { color, .. } if *color == 0xeaf3ff)
    ));
    assert!(clipped
        .cmds()
        .iter()
        .any(|command| matches!(command, DrawCmd::RoundedRect { color, .. } if *color == CODE_BG)));
    assert!(full.finish().is_ok());
    assert!(clipped.finish().is_ok());
}

#[test]
fn scrolling_is_bounded_by_the_document_height() {
    let doc = layout(&parse("只有一行"), content_width(AREA));
    // 内容不足一屏时根本不该能滚
    assert_eq!(max_scroll(&doc, AREA), 0.0);

    let long = (0..500)
        .map(|i| format!("第 {i} 行"))
        .collect::<Vec<_>>()
        .join("\n");
    let big = layout(&parse(&long), content_width(AREA));
    assert!(max_scroll(&big, AREA) > 0.0);
    assert!(max_scroll(&big, AREA) < big.height);
}

#[test]
fn a_quote_gets_a_bar_and_code_gets_a_card_with_a_header() {
    let doc = layout(
        &parse("> 引用\n\n```rust\nlet a = 1; // hi\n```"),
        content_width(AREA),
    );
    let header = doc
        .lines
        .iter()
        .find(|l| matches!(l.decoration, Decoration::CodeHeader { .. }))
        .unwrap();
    assert_eq!(
        header.decoration,
        Decoration::CodeHeader {
            body_lines: 1,
            collapsed: false
        }
    );
    assert_eq!(header.height, CODE_HEADER_H);
    let code = doc
        .lines
        .iter()
        .find(|l| matches!(l.decoration, Decoration::CodeBackground { .. }))
        .unwrap();
    assert_eq!(
        code.y,
        header.y + CODE_HEADER_H + code_pad_y(),
        "正文在头部 + 20px 留白之后"
    );
    assert_eq!(code.x, code_pad_x() + code_line_number_width(1));
    assert!(
        code.tokens
            .iter()
            .any(|t| t.kind == highlight::Token::Keyword),
        "`let` 要着色"
    );
    assert!(code
        .tokens
        .iter()
        .any(|t| t.kind == highlight::Token::Comment));

    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, AREA, &doc, 0.0, &p);
    // 卡片底色是 CSS 写死的 #f8f9fa；语言名与着色文字都画了出来
    assert!(list
        .cmds()
        .iter()
        .any(|c| matches!(c, DrawCmd::RoundedRect { color, .. } if *color == CODE_BG)));
    let t = texts(&list);
    assert!(t.contains(&"rust".to_owned()));
    assert!(
        t.contains(&"let".to_owned()) && t.contains(&"// hi".to_owned()),
        "{t:?}"
    );
    assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { color, .. } if *color == highlight::color(highlight::Token::Keyword, false))));
    assert!(list
        .cmds()
        .iter()
        .any(|c| matches!(c, DrawCmd::Rect { rect, color }
            if *color == p.border && rect.width() <= quote_bar() + 0.01)));
    assert!(list.finish().is_ok());
}

#[test]
fn code_card_without_title_keeps_only_a_top_right_language_label() {
    let old = super::super::editor_preferences::current();
    let _restore = PreferencesGuard(old.clone());
    let mut prefs = old;
    prefs.code_show_title = false;
    super::super::editor_preferences::set(prefs);

    let doc = layout(&parse("```rust\nlet value = 1;\n```"), content_width(AREA));
    let header = doc
        .lines
        .iter()
        .find(|line| matches!(line.decoration, Decoration::CodeHeader { .. }))
        .unwrap();
    let code = doc
        .lines
        .iter()
        .find(|line| matches!(line.decoration, Decoration::CodeBackground { .. }))
        .unwrap();
    assert_eq!(header.height, 0.0);
    assert_eq!(code.y, header.y + code_pad_y());

    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, AREA, &doc, 0.0, &p);
    assert!(texts(&list).contains(&"rust".to_owned()));
    assert!(!texts(&list).contains(&"代码块名称".to_owned()));
}

#[test]
fn an_empty_document_paints_nothing_and_balances_the_clip_stack() {
    let doc = layout(&parse(""), content_width(AREA));
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, AREA, &doc, 0.0, &p);
    assert!(texts(&list).is_empty());
    assert!(list.finish().is_ok());
}

#[test]
fn a_zero_sized_area_is_skipped_entirely() {
    let doc = layout(&parse("内容"), 300.0);
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, Rect::new(0.0, 0.0, 0.0, 0.0), &doc, 0.0, &p);
    assert!(list.is_empty());
    assert!(list.finish().is_ok());
}

#[test]
fn electron_html_tables_render_as_native_tables() {
    let source = r#"<table class="border-collapse"><colgroup><col style="width:25px"></colgroup><tbody><tr><th><p>标题</p></th><th>说明</th></tr><tr><td>1</td><td>a &amp; b</td></tr></tbody></table>"#;
    let parsed = parse(source);
    let expected: Vec<Vec<String>> = vec![
        vec!["标题".to_owned(), "说明".to_owned()],
        vec!["1".to_owned(), "a & b".to_owned()],
    ];
    assert!(matches!(&parsed[0], Block::Table { rows, header: true } if rows == &expected));
}

#[test]
fn cached_editor_layout_reuses_unchanged_blocks_after_an_edit() {
    let source = (0..320)
        .map(|i| format!("稳定段落 {i} 中文 words **重点** tail"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let parsed = parse_ranged(&source);
    let mut cache = LayoutCache::default();
    let first = layout_editor_cached(&parsed.blocks, &source, None, 420.0, &|_| None, &mut cache);
    let (_, first_hits, first_misses) = cache.stats();
    assert_eq!(first_hits, 0);
    assert!(first_misses >= 300, "每个新段落都应先建立局部缓存");

    let edited = source.replacen("稳定段落 0", "稳定段落 0 修改", 1);
    let next = parse_ranged(&edited);
    let second = layout_editor_cached(&next.blocks, &edited, None, 420.0, &|_| None, &mut cache);
    let (_, second_hits, second_misses) = cache.stats();
    assert!(
        second_hits >= 300,
        "编辑一个块后应命中其余段落缓存: {second_hits}"
    );
    assert!(
        second_misses <= first_misses + 1,
        "只有被编辑的段落需要重新断行"
    );

    // 缓存保存的是局部行；重组时仍会重新生成当前的
    // 块索引和 y 坐标，因此源码偏移改变后，未变的块
    // 看起来仍与之前完全一致。
    let first_stable = first
        .lines
        .iter()
        .filter(|line| line.block == 2)
        .map(|line| (line.runs.clone(), line.visible_start, line.height))
        .collect::<Vec<_>>();
    let second_stable = second
        .lines
        .iter()
        .filter(|line| line.block == 2)
        .map(|line| (line.runs.clone(), line.visible_start, line.height))
        .collect::<Vec<_>>();
    assert_eq!(first_stable, second_stable);
}

#[test]
fn cached_editor_layout_keeps_an_active_snapshot_beyond_fifo_limit() {
    let count = MAX_WRAPPED_ENTRIES + 64;
    let source = (0..count)
        .map(|i| format!("# 唯一标题 {i}\n稳定段落 {i} words **重点** tail"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let parsed = parse_ranged(&source);
    let mut cache = LayoutCache::default();
    layout_editor_cached(&parsed.blocks, &source, None, 420.0, &|_| None, &mut cache);
    let (first_entries, first_hits, first_misses) = cache.stats();
    assert_eq!(first_hits, 0);
    assert!(
        first_entries > MAX_WRAPPED_ENTRIES,
        "an active document should not be truncated at the legacy FIFO limit"
    );
    assert!(first_misses > MAX_WRAPPED_ENTRIES);

    let edited = source.replacen("唯一标题 0", "唯一标题 0 修改", 1);
    let next = parse_ranged(&edited);
    layout_editor_cached(&next.blocks, &edited, None, 420.0, &|_| None, &mut cache);
    let (second_entries, second_hits, second_misses) = cache.stats();
    assert_eq!(second_entries, first_entries);
    assert_eq!(second_misses - first_misses, 1);
    assert_eq!(second_hits - first_hits, first_misses - 1);
    assert!(cache.active_generation.is_none());
}

#[test]
fn layout_cache_closes_cancelled_generations_before_the_next_pass() {
    let mut cache = LayoutCache::default();
    cache.prepare();
    for i in 0..32 {
        let text = format!("取消前的缓存 {i}");
        cache.wrapped(&text, TextStyle::Document, 420.0);
    }
    assert_eq!(cache.entries, 32);

    // 模拟渐进加载过程在调用
    // finish_generation 之前被取消。下一轮会先结束旧过程，再开始新的
    // 活动快照。
    cache.prepare();
    for i in 0..3 {
        let text = format!("新的缓存 {i}");
        cache.wrapped(&text, TextStyle::Document, 420.0);
    }
    assert_eq!(cache.entries, 35);

    // 再次重启时，应丢弃第一次被放弃的快照，
    // 不要一直保留已经取消的编辑内容。
    cache.prepare();
    assert_eq!(cache.entries, 3);
    assert!(cache.active_generation.is_some());
}

#[test]
fn editor_chunks_match_full_layout_for_mixed_markdown() {
    let source = concat!(
        "前文🙂\r\n\r\n",
        "# 标题\r\n\r\n",
        "- 列表一\r\n- 列表二\r\n\r\n",
        "| A | B |\r\n| --- | --- |\r\n| 中文 | 2 |\r\n\r\n",
        "```rust\r\nlet value = 1;\r\n```\r\n\r\n",
        ":::mochi-highlight title=\"容器\"\r\n容器内文\r\n:::\r\n\r\n",
        "尾部空白\r\n",
    );
    let parsed = parse_ranged(source);
    let mut full_cache = LayoutCache::default();
    let full = layout_editor_cached(
        &parsed.blocks,
        source,
        None,
        420.0,
        &|_| None,
        &mut full_cache,
    );

    let mut chunk_cache = LayoutCache::default();
    let mut progress = LayoutProgress::default();
    let mut lines = Vec::new();
    let mut headings = Vec::new();
    while !progress.is_complete(parsed.blocks.len()) {
        let before = progress.next_block;
        let chunk = layout_editor_chunk(
            &parsed.blocks,
            source,
            420.0,
            &|_| None,
            &mut chunk_cache,
            &mut progress,
            Duration::from_micros(50),
        );
        lines.extend(chunk.lines);
        headings.extend(chunk.headings);
        if progress.next_block == before {
            let chunk = layout_editor_chunk(
                &parsed.blocks,
                source,
                420.0,
                &|_| None,
                &mut chunk_cache,
                &mut progress,
                Duration::from_millis(1),
            );
            lines.extend(chunk.lines);
            headings.extend(chunk.headings);
            assert!(progress.next_block > before || progress.is_complete(parsed.blocks.len()));
        }
    }
    assert_eq!(lines, full.lines);
    assert_eq!(headings, full.headings);
    assert_eq!(progress.y + padding_bottom(), full.height);
    assert_eq!(chunk_cache.active_generation, None);
}

#[test]
fn chunk_panel_events_match_linear_lookup_for_nested_adjacent_and_empty_containers() {
    let source = concat!(
        "<details>\n",
        "<summary>关闭外层</summary>\n",
        "隐藏外层\n",
        "<details open>\n",
        "<summary>打开内层</summary>\n",
        "隐藏内层\n",
        "</details>\n",
        "</details>\n",
        "<details open>\n",
        "<summary>相邻 A</summary>\n",
        "A\n",
        "</details>\n",
        "<details open>\n",
        "<summary>相邻 B</summary>\n",
        "</details>\n",
        ":::mochi-highlight title=\"外层\"\n",
        "外层\n",
        "<details open>\n",
        "<summary>内层</summary>\n",
        "内层\n",
        "</details>\n",
        ":::\n",
        "尾部\n",
    );
    let parsed = parse_ranged(source);
    let reference_state = |bi: usize| {
        let offset = parsed.blocks[bi].start;
        parsed
            .blocks
            .iter()
            .filter_map(|block| match &block.block {
                Block::Container(panel) if panel.body.contains(&offset) => {
                    Some((!panel.open) as usize)
                }
                _ => None,
            })
            .fold((0usize, 0usize), |(depth, closed), is_closed| {
                (depth + 1, closed + is_closed)
            })
    };

    let mut cache = LayoutCache::default();
    let mut progress = LayoutProgress::default();
    let mut lines = Vec::new();
    let mut headings = Vec::new();
    while !progress.is_complete(parsed.blocks.len()) {
        let before = progress.next_block;
        let chunk = layout_editor_chunk(
            &parsed.blocks,
            source,
            420.0,
            &|_| None,
            &mut cache,
            &mut progress,
            Duration::ZERO,
        );
        assert_eq!(progress.next_block, before + 1);
        let (depth, closed) = reference_state(before);
        assert_eq!(progress.panel_depth, depth, "panel depth at block {before}");
        assert_eq!(
            progress.closed_panels, closed,
            "closed panels at block {before}"
        );
        lines.extend(chunk.lines);
        headings.extend(chunk.headings);
    }

    let mut full_cache = LayoutCache::default();
    let full = layout_editor_cached(
        &parsed.blocks,
        source,
        None,
        420.0,
        &|_| None,
        &mut full_cache,
    );
    assert_eq!(lines, full.lines);
    assert_eq!(headings, full.headings);
    assert_eq!(progress.y + padding_bottom(), full.height);
}

#[test]
fn layout_cache_has_hard_entry_and_memory_limits() {
    let mut cache = LayoutCache::default();
    for i in 0..(MAX_WRAPPED_ENTRIES + 512) {
        cache.wrapped(&format!("cache entry {i}"), TextStyle::Document, 400.0);
    }
    assert!(cache.entries <= MAX_WRAPPED_ENTRIES);
    assert!(cache.bytes <= MAX_WRAPPED_BYTES);
    assert_eq!(cache.order.len(), cache.entries);
}
