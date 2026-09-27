use super::*;
use crate::ui::chrome::{Chrome, ChromeState, WorkspaceView};

#[test]
fn marketplace_entry_is_immediately_left_of_desktop_control_and_not_draggable() {
    let chrome = Chrome::build(
        &ChromeState::default(),
        Rect::from_size(0.0, 0.0, 1200.0, 800.0),
    );
    let market = chrome.tree.rect(chrome.marketplace_toggle);
    assert_eq!(market.right, chrome.tree.rect(chrome.desktop_toggle).left);
    assert_eq!(
        chrome.hit(market.left + 12.0, market.top + 12.0),
        Some(crate::ui::layout::NodeKey::TitleBarMarketplace)
    );
    let state = ChromeState {
        view: WorkspaceView::Marketplace,
        ..Default::default()
    };
    assert!(state.view.hides_left_panel());
    assert_eq!(
        crate::view::MainContent::resolve(&state, false, None),
        crate::view::MainContent::Standalone(WorkspaceView::Marketplace)
    );
}
#[test]
fn marketplace_pagination_and_all_eight_filters_select_real_items() {
    let mut state = State {
        catalog: fixtures::catalog(),
        ..Default::default()
    };
    let area = Rect::from_size(0.0, 0.0, 1000.0, 800.0);
    assert_eq!(state.pages(), 2);
    for (page, count) in [(0, 12), (1, 8)] {
        state.page = page;
        let lay = layout(&state, area);
        let items: Vec<_> = lay
            .entries
            .iter()
            .filter_map(|(_, h)| if let Hit::Item(i) = h { Some(*i) } else { None })
            .collect();
        assert_eq!(items.len(), count);
        assert_eq!(items[0], page * PAGE_SIZE);
        assert_eq!(
            lay.entries.iter().any(|(_, h)| *h == Hit::Previous),
            page > 0
        );
        assert_eq!(lay.entries.iter().any(|(_, h)| *h == Hit::Next), page < 1);
    }
    for category in Category::ALL {
        state.category = Some(category);
        state.page = 0;
        assert!(state
            .filtered()
            .iter()
            .all(|i| state.catalog.packages[*i].category == category));
        assert!(!state.filtered().is_empty());
    }
}
#[test]
fn marketplace_responsive_grid_never_intercepts_footer_or_filter_controls() {
    let mut state = State {
        catalog: fixtures::catalog(),
        ..Default::default()
    };
    for width in [320.0, 460.0, 620.0, 960.0, 1440.0] {
        let area = Rect::from_size(16.0, 40.0, width, 740.0);
        for scroll in [0.0, 9999.0] {
            state.scroll = scroll;
            let lay = layout(&state, area);
            for (rect, hit) in &lay.entries {
                assert!(
                    rect.left >= lay.content.left && rect.right <= lay.content.right + 0.1,
                    "{width}: {hit:?}"
                );
                if !matches!(hit, Hit::Item(_)) {
                    assert_eq!(
                        lay.hit(rect.left + 3.0, rect.top + 3.0),
                        Some(*hit),
                        "{width}: {hit:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn marketplace_detail_scroll_reaches_description_and_download_remains_visible() {
    let mut state = State {
        catalog: fixtures::catalog(),
        selected: Some(0),
        ..Default::default()
    };
    let area = Rect::from_size(0.0, 0.0, 460.0, 640.0);
    let lay = layout(&state, area);
    assert!(lay.max_scroll > 0.0);
    let download = lay
        .entries
        .iter()
        .find(|(_, h)| *h == Hit::Download)
        .unwrap()
        .0;
    assert!(download.top > lay.body.bottom);
    assert_eq!(
        lay.hit(download.left + 5.0, download.top + 5.0),
        Some(Hit::Download)
    );
    state.downloading = true;
    assert!(!layout(&state, area)
        .entries
        .iter()
        .any(|(_, h)| matches!(h, Hit::Download | Hit::Refresh)));
}

#[test]
fn workspace_tools_remain_distinct_and_ordered_in_the_titlebar() {
    use crate::ui::layout::NodeKey;
    for width in [760.0, 1200.0, 1800.0] {
        let chrome = Chrome::build(
            &ChromeState::default(),
            Rect::from_size(0.0, 0.0, width, 800.0),
        );
        let tools = [
            (chrome.automations_toggle, NodeKey::TitleBarAutomations),
            (chrome.templates_toggle, NodeKey::TitleBarTemplates),
            (chrome.marketplace_toggle, NodeKey::TitleBarMarketplace),
            (chrome.desktop_toggle, NodeKey::TitleBarDesktop),
        ];
        let mut right = 0.0;
        for (node, key) in tools {
            let rect = chrome.tree.rect(node);
            assert!(rect.left >= right);
            assert_eq!(chrome.hit(rect.left + 10.0, rect.top + 10.0), Some(key));
            right = rect.right;
        }
        let state = ChromeState {
            view: WorkspaceView::DesktopCards,
            ..Default::default()
        };
        assert_eq!(
            crate::view::MainContent::resolve(&state, false, None),
            crate::view::MainContent::Standalone(WorkspaceView::DesktopCards)
        );
    }
}
