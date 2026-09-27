//! 缓存 AI Markdown 的解析和排版结果，减少重复计算。
use super::*;

fn build(source: &str, width: f32, standalone: bool) -> Layout {
    let mut builder = Builder {
        layout: Layout::default(),
        standalone,
        base: TextStyle::Ai {
            kind: 0,
            standalone,
        }
        .font_size(),
    };
    if source.is_empty() {
        return builder.layout;
    }
    let root = if source.len() > 4 * 1024 * 1024 {
        None
    } else {
        parse(&normalize_math_delimiters(source.trim_end())).ok()
    };
    builder.layout.height = if let Some(root) = root {
        builder.node(&root, 0.0, 0.0, width.max(1.0), Tone::Normal)
    } else {
        builder.inline(
            &[Run::plain(
                "内容过长或嵌套过深，请复制 Markdown 查看完整回复。",
            )],
            0.0,
            0.0,
            width,
            builder.style(0),
            Tone::Muted,
        )
    };
    builder.layout
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ResetSettings;

    impl Drop for ResetSettings {
        fn drop(&mut self) {
            crate::ui::settings_values::reset();
        }
    }

    #[test]
    fn math_measurement_uses_the_active_ai_body_font() {
        for standalone in [false, true] {
            let style = TextStyle::Ai {
                kind: 0,
                standalone,
            };
            let result = build("$$x$$", 280.0, standalone);
            let size = result.items.iter().find_map(|item| match item {
                Item::Math { size, .. } => Some(*size),
                _ => None,
            });
            assert_eq!(size, Some(style.font_size() * 1.08));
        }
    }

    #[test]
    fn changing_ai_font_setting_invalidates_cached_markdown_geometry() {
        use mochi_core::{
            app_settings::{AppSettings, SettingValue},
            settings::SettingsService,
        };
        use std::sync::Arc;

        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let settings = AppSettings::new(Arc::new(SettingsService::new(Some(
            std::env::temp_dir().join(format!("mochi-ai-markdown-settings-{suffix}.json")),
        ))));
        let font = mochi_core::app_settings::descriptor("assistant.messageFontSize")
            .expect("assistant message font descriptor");
        settings.write(font, &SettingValue::Number(14.0));
        crate::ui::settings_values::load(&settings);
        let _reset = ResetSettings;
        let first = layout("$$x$$", 280.0, false);
        let first_size = first.items.iter().find_map(|item| match item {
            Item::Math { size, .. } => Some(*size),
            _ => None,
        });
        let epoch = crate::ui::measurement::epoch();

        settings.write(font, &SettingValue::Number(18.0));
        crate::ui::settings_values::load(&settings);
        assert_ne!(crate::ui::measurement::epoch(), epoch);
        let second = layout("$$x$$", 280.0, false);
        let second_size = second.items.iter().find_map(|item| match item {
            Item::Math { size, .. } => Some(*size),
            _ => None,
        });

        assert!(!Rc::ptr_eq(&first, &second));
        assert!(first_size.unwrap() < second_size.unwrap());
        assert_eq!(second_size, Some(18.0 * 1.08));
    }
}

pub(super) const CACHE_ENTRIES: usize = 64;

pub(super) const CACHE_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct Cached {
    pub(super) source: String,
    pub(super) width: u32,
    pub(super) standalone: bool,
    pub(super) value: Rc<Layout>,
    pub(super) bytes: usize,
}

#[derive(Default)]
pub(super) struct Cache {
    pub(super) epoch: u64,
    pub(super) entries: VecDeque<Cached>,
    pub(super) bytes: usize,
}

pub fn layout(source: &str, width: f32, standalone: bool) -> Rc<Layout> {
    let width = if width.is_finite() {
        width.max(1.0)
    } else {
        1.0
    };
    let epoch = crate::ui::measurement::epoch();
    if let Some(hit) = CACHE.with(|cache| {
        let mut c = cache.borrow_mut();
        if c.epoch != epoch {
            *c = Cache {
                epoch,
                ..Default::default()
            };
        }
        let index = c.entries.iter().position(|e| {
            e.width == width.to_bits() && e.standalone == standalone && e.source == source
        })?;
        let hit = c.entries.remove(index).unwrap();
        let value = hit.value.clone();
        c.entries.push_back(hit);
        Some(value)
    }) {
        return hit;
    }
    let value = Rc::new(build(source, width, standalone));
    // 统计的是保留下来的分配载荷，不是进程 RSS，也不算外部持有的 Rc。
    let payload = value
        .items
        .iter()
        .map(|i| match i {
            Item::Text { run, .. } => run.text.capacity(),
            Item::CodeHeader { language, .. } => language.capacity(),
            Item::Math { tex, .. } => tex.capacity(),
            _ => 0,
        })
        .sum::<usize>();
    let actions = value.actions.capacity() * std::mem::size_of::<CopyTarget>()
        + value
            .actions
            .iter()
            .map(|a| {
                a.payload.text.capacity() + a.payload.html.as_ref().map_or(0, |h| h.capacity())
            })
            .sum::<usize>();
    let links = value.links.capacity() * std::mem::size_of::<LinkTarget>()
        + value
            .links
            .iter()
            .map(|link| link.target.capacity())
            .sum::<usize>();
    let bytes = source.len()
        + value.items.capacity() * std::mem::size_of::<Item>()
        + payload
        + actions
        + links
        + value.scroll_regions.capacity() * std::mem::size_of::<ScrollRegion>();
    if bytes <= CACHE_BYTES {
        CACHE.with(|cache| {
            let mut c = cache.borrow_mut();
            while c.entries.len() >= CACHE_ENTRIES || c.bytes + bytes > CACHE_BYTES {
                let old = c.entries.pop_front().unwrap();
                c.bytes -= old.bytes;
            }
            c.bytes += bytes;
            c.entries.push_back(Cached {
                source: source.into(),
                width: width.to_bits(),
                standalone,
                value: value.clone(),
                bytes,
            });
        });
    }
    value
}
