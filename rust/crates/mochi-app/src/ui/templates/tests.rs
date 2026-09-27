use super::*;

fn state() -> State {
    State {
        items: mochi_core::templates::BUILTINS
            .iter()
            .map(|b| Template {
                name: b.name.into(),
                group: b.group.into(),
                path: format!("{}.{}", b.name, b.extension).into(),
            })
            .collect(),
        groups: ["日常", "学习", "工作", "研究", "开发", "写作"]
            .into_iter()
            .map(String::from)
            .collect(),
        ..Default::default()
    }
}

#[test]
fn scenario_and_format_filters_keep_original_item_indices() {
    let mut state = state();
    state.selected_group = "学习".into();
    state.format = Format::Mochi;
    let lay = layout(&state, Rect::from_size(0.0, 0.0, 1000.0, 800.0));
    assert_eq!(lay.visible_count, 2);
    for (_, hit) in lay.entries {
        if let Hit::Template(index) = hit {
            assert_eq!(state.items[index].group, "学习");
            assert_eq!(state.items[index].format_label(), "墨池文档");
        }
    }
    state.format = Format::Text;
    assert_eq!(
        layout(&state, Rect::from_size(0.0, 0.0, 1000.0, 800.0)).visible_count,
        0
    );
}

#[test]
fn cards_wrap_and_actions_do_not_overlap_or_intercept_header() {
    for width in [320.0, 420.0, 640.0, 960.0, 1440.0] {
        let mut state = state();
        let area = Rect::from_size(10.0, 20.0, width, 800.0);
        let lay = layout(&state, area);
        let cards: Vec<_> = lay
            .entries
            .iter()
            .filter(|(_, h)| matches!(h, Hit::Template(_)))
            .collect();
        for (index, (r, _)) in cards.iter().enumerate() {
            assert!(r.left >= lay.content.left && r.right <= lay.content.right + 0.1);
            for (other, _) in cards.iter().skip(index + 1) {
                assert!(r.intersect(other).is_empty());
            }
        }
        for (r, hit) in &lay.entries {
            if matches!(hit, Hit::Use(_) | Hit::Delete(_)) && r.bottom <= lay.list.bottom {
                assert_eq!(lay.hit(r.left + 4.0, r.top + 4.0), Some(*hit));
            }
        }
        state.scroll = lay.max_scroll;
        let lay = layout(&state, area);
        let new = lay.entries.iter().find(|(_, h)| *h == Hit::New).unwrap().0;
        assert_eq!(lay.hit(new.left + 6.0, new.top + 6.0), Some(Hit::New));
        for dark in [false, true] {
            let mut list = DrawList::new();
            paint(&mut list, area, &state, &lay, theme::tokens().palette(dark));
            assert!(list.finish().is_ok());
        }
    }
}

#[test]
fn rich_catalog_uses_real_editor_blocks() {
    use crate::ui::{
        containers::Kind,
        document::{self, Block},
    };
    let mut kinds = [false; 4];
    for template in mochi_core::templates::BUILTINS {
        let parsed = document::parse_ranged(template.content);
        assert!(!parsed.blocks.is_empty(), "{}", template.name);
        let mut highlights = 0;
        let mut details = 0;
        for block in &parsed.blocks {
            match &block.block {
                Block::Container(c) if c.kind == Kind::Highlight => {
                    highlights += 1;
                    kinds[0] = true;
                }
                Block::Container(c) if c.kind == Kind::Details => {
                    details += 1;
                    kinds[1] = true;
                }
                Block::Math(_) => kinds[2] = true,
                Block::Code { .. } => kinds[3] = true,
                _ => {}
            }
        }
        assert_eq!(
            highlights,
            template.content.matches(":::mochi-highlight").count(),
            "{}",
            template.name
        );
        assert_eq!(
            details,
            template.content.matches("<details>").count(),
            "{}",
            template.name
        );
        for width in [220.0, 620.0] {
            let layout =
                document::layout_editor(&parsed.blocks, template.content, None, width, &|_| None);
            assert!(
                layout.height.is_finite() && layout.height > 0.0,
                "{}",
                template.name
            );
        }
    }
    assert!(kinds.into_iter().all(|present| present));
}
