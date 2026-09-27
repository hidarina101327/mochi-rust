use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    settings_values as values, theme,
};
use mochi_core::{
    app_settings::{self, AppSettings},
    settings::SettingsService,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct Preferences {
    settings: AppSettings,
    path: PathBuf,
}
impl Preferences {
    fn new() -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mochi-customization-ui-{}-{}.json",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        values::reset();
        Self {
            settings: AppSettings::new(Arc::new(SettingsService::new(Some(path.clone())))),
            path,
        }
    }
    fn set(&self, key: &str, raw: serde_json::Value) {
        let descriptor = app_settings::descriptor(key).expect(key);
        self.settings.write(
            descriptor,
            &app_settings::coerce(descriptor, &raw).unwrap().value,
        );
        values::load(&self.settings);
    }
}
impl Drop for Preferences {
    fn drop(&mut self) {
        values::reset();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn customization_theme_overrides_are_independent_and_preserve_theme_identity() {
    let prefs = Preferences::new();
    let light = theme::configured_palette(false);
    prefs.set("appearance.dark.foreground", json!("#abc"));
    prefs.set("appearance.dark.background", json!("#ffffff"));
    let dark = theme::configured_palette(true);
    assert_eq!(dark.foreground, 0xaabbcc);
    assert!(
        theme::is_dark(&dark),
        "背景颜色不能把深色主题识别成浅色主题"
    );
    assert_eq!(theme::configured_palette(false), light);
    assert_eq!(theme::workflow_palette(&dark).foreground, 0xaabbcc);
    prefs.set("appearance.dark.foreground", json!("auto"));
    assert_eq!(
        theme::configured_palette(true).foreground,
        theme::tokens().dark.foreground
    );
}

#[test]
fn customization_font_refresh_changes_real_measurement_and_cached_ai_layout() {
    let prefs = Preferences::new();
    let mut renderer = crate::gfx::Renderer::new().unwrap();
    let before = super::text::measure("界面字体与颜色 ABC", TextStyle::Body);
    let cached = super::ai_markdown::layout("可调整文字\n\n第二段文字", 180.0, false);
    prefs.set("appearance.uiFontSize", json!(18));
    prefs.set("assistant.messageFontSize", json!(20));
    renderer.refresh_text_formats().unwrap();
    assert!(super::text::measure("界面字体与颜色 ABC", TextStyle::Body) > before);
    let updated = super::ai_markdown::layout("可调整文字\n\n第二段文字", 180.0, false);
    assert!(!std::rc::Rc::ptr_eq(&cached, &updated));
    assert!(updated.height > cached.height);
}

#[test]
fn customization_menu_geometry_and_hits_follow_spacing() {
    let prefs = Preferences::new();
    prefs.set("menu.itemHeight", json!(52));
    prefs.set("menu.padding", json!(12));
    let menu = super::widgets::Menu::open_at(
        vec![
            super::widgets::MenuItem::new("一", 1),
            super::widgets::MenuItem::new("二", 2),
        ],
        10.0,
        10.0,
        Rect::from_size(0.0, 0.0, 800.0, 600.0),
    );
    let first = menu.item_rect(0).unwrap();
    let second = menu.item_rect(1).unwrap();
    assert_eq!(first.height(), 52.0);
    assert_eq!(second.top, first.bottom);
    assert_eq!(menu.hit(second.left + 5.0, second.top + 25.0), Some(1));
}

#[test]
fn customization_scrollbar_width_visibility_and_drag_share_geometry() {
    let prefs = Preferences::new();
    prefs.set("scrollbar.width", json!(16));
    prefs.set("scrollbar.alwaysVisible", json!(true));
    let bar = super::overlay_scrollbar::Bar::new(
        Rect::from_size(0.0, 0.0, 600.0, 400.0),
        super::overlay_scrollbar::Axis::Vertical,
        2000.0,
        0.0,
        false,
    )
    .unwrap();
    assert_eq!(bar.thumb.width(), 16.0);
    assert!(bar.hotzone.width() >= bar.thumb.width());
    let mut interaction = super::overlay_scrollbar::Interaction::default();
    let mut list = DrawList::new();
    interaction.paint(&mut list, &[((), bar)], &theme::configured_palette(false));
    assert!(!list.is_empty());
    interaction
        .begin(&[((), bar)], bar.thumb.left + 1.0, bar.thumb.top + 1.0)
        .unwrap();
    assert_eq!(
        interaction.drag_to(&[((), bar)], 0.0, 9000.0).unwrap().1,
        2000.0
    );
}

#[test]
fn customization_home_reflows_without_reloading_workspace_data() {
    let prefs = Preferences::new();
    let mut pane = crate::view::HomePane::default();
    pane.set_dashboard(super::home::Dashboard::default());
    let area = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
    let mut first = DrawList::new();
    pane.paint(&mut first, area, &theme::configured_palette(false));
    prefs.set("dashboard.panelGap", json!(40));
    let mut second = DrawList::new();
    pane.paint(&mut second, area, &theme::configured_palette(false));
    assert_ne!(
        format!("{:?}", first.cmds()),
        format!("{:?}", second.cmds())
    );
    assert!(second.finish().is_ok());
}

#[test]
fn customization_reset_hit_is_clipped_to_settings_viewport() {
    let _prefs = Preferences::new();
    let area = Rect::from_size(0.0, 0.0, 800.0, 500.0);
    let layout = super::settings::content_layout(area, "customization", Some("appearance"), 0.0);
    let (control, index) = layout
        .controls
        .iter()
        .find(|(r, _)| r.top > layout.body.top && r.bottom < layout.body.bottom)
        .unwrap();
    assert_eq!(
        layout.reset_hit(control.left - 24.0, control.top + 10.0),
        Some(*index)
    );
    assert_eq!(
        layout.reset_hit(control.left - 24.0, layout.body.top - 1.0),
        None
    );
}

#[test]
fn customization_schedule_zoom_keeps_time_hit_mapping_and_semantic_colors() {
    let prefs = Preferences::new();
    prefs.set("schedule.hourHeight", json!(96));
    prefs.set("schedule.entryColor", json!("#123456"));
    let mut state = super::agenda::State::default();
    state.data = Some(mochi_core::agenda::AgendaData::default());
    let mut list = DrawList::new();
    let palette = theme::configured_palette(false);
    let layout = super::agenda::paint(
        &mut list,
        Rect::from_size(0.0, 0.0, 1200.0, 800.0),
        &mut state,
        None,
        &palette,
    );
    let geometry = layout.geo.unwrap();
    assert_eq!(geometry.hour_h, 96.0);
    assert_eq!(geometry.minute_at(geometry.y_of(9 * 60)), 9 * 60);
    assert_eq!(super::agenda::visual::colors(&palette).entry, 0x123456);
    assert!(list.finish().is_ok());
}

#[test]
fn customization_document_title_reserves_room_when_font_size_changes() {
    let prefs = Preferences::new();
    let area = Rect::from_size(0.0, 0.0, 800.0, 600.0);
    let before = super::file_title::rect(area, 0.0);
    let offset = super::file_title::body_offset();
    prefs.set("editorLayout.titleFontSize", json!(60));
    let after = super::file_title::rect(area, 0.0);
    assert_eq!(after.height(), TextStyle::DocumentTitle.line_height());
    assert_eq!(
        super::file_title::body_offset() - offset,
        after.height() - before.height()
    );
}

#[test]
fn customization_shell_rows_reserve_room_for_large_fonts() {
    let prefs = Preferences::new();
    prefs.set("appearance.uiFontSize", json!(24));
    prefs.set("appearance.titleFontSize", json!(22));
    prefs.set("appearance.captionFontSize", json!(22));
    prefs.set("appearance.labelFontSize", json!(24));
    prefs.set("sidebar.fileTreeItemSize", json!(20));
    let layout = super::chrome::configured_layout();
    assert!(layout.title_bar_height >= TextStyle::Title.line_height());
    assert!(layout.status_bar_height >= TextStyle::Caption.line_height());
    assert!(layout.tab_bar_height >= TextStyle::Body.line_height());
    assert!(layout.tab_bar_compact_height >= TextStyle::Body.line_height());
    assert!(super::sidebar::row_height() >= TextStyle::Label.line_height());
    assert!(super::sidebar::row_height() >= TextStyle::Caption.line_height());
}

#[test]
fn customization_dialog_title_growth_keeps_input_and_button_hits_aligned() {
    use super::widgets::{ButtonKind, Dialog, DialogButton, DialogHit, TextField};
    let prefs = Preferences::new();
    let dialog = Dialog {
        title: "自定义标题".into(),
        description: "输入内容".into(),
        field: Some(TextField::new("")),
        error: String::new(),
        note: None,
        buttons: vec![DialogButton {
            label: "保存".into(),
            kind: ButtonKind::Primary,
            action: (),
        }],
        dismiss: (),
        hover: None,
    };
    let area = Rect::from_size(0.0, 0.0, 800.0, 600.0);
    let (before, before_field, _, _, _) = dialog.parts(area);
    prefs.set("appearance.dialogTitleFontSize", json!(24));
    let (title, field, _, buttons, _) = dialog.parts(area);
    assert!((title.height() - TextStyle::Large.line_height()).abs() < 0.001);
    let field = field.unwrap();
    assert!(
        ((field.top - title.bottom) - (before_field.unwrap().top - before.bottom)).abs() < 0.001
    );
    assert_eq!(
        dialog.hit(area, field.left + 10.0, field.top + 10.0),
        DialogHit::Field
    );
    assert_eq!(
        dialog.hit(area, buttons[0].left + 10.0, buttons[0].top + 10.0),
        DialogHit::Button(0)
    );
}
