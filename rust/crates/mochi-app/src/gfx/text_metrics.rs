//! 有界的 DirectWrite 成形缓存，与渲染器的精确格式共用。
use super::*;
use crate::ui::measurement::{Backend, Cluster, Shape};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_CLUSTER_METRICS, DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_METRICS,
};
type Formats = HashMap<(TextStyle, Align, Emphasis), IDWriteTextFormat>;
type Key = (String, TextStyle, Emphasis);
const MAX_ENTRIES: usize = 512;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_UNITS: usize = 128 * 1024;
#[derive(Default)]
struct Cache {
    map: HashMap<Key, Rc<Shape>>,
    order: VecDeque<(Key, usize)>,
    bytes: usize,
}
pub(super) struct Metrics {
    factory: IDWriteFactory,
    formats: RefCell<Formats>,
    cache: RefCell<Cache>,
}
impl Metrics {
    pub(super) fn new(factory: IDWriteFactory, formats: Formats) -> Rc<Self> {
        let backend = Rc::new(Self {
            factory,
            formats: RefCell::new(formats),
            cache: RefCell::new(Cache::default()),
        });
        crate::ui::measurement::register(backend.clone());
        backend
    }
    pub(super) fn refresh(&self, formats: Formats) {
        *self.formats.borrow_mut() = formats;
        *self.cache.borrow_mut() = Cache::default();
        crate::ui::measurement::invalidate();
    }
    fn compute(&self, text: &str, style: TextStyle, emphasis: Emphasis) -> Result<Shape> {
        let units = text.encode_utf16().collect::<Vec<_>>();
        let style = if emphasis == Emphasis::Code && style == TextStyle::Body {
            TextStyle::Mono
        } else {
            style
        };
        let formats = self.formats.borrow();
        let format = formats
            .get(&(style, Align::Leading, emphasis))
            .ok_or_else(windows::core::Error::from_thread)?;
        let layout = unsafe {
            self.factory
                .CreateTextLayout(&units, format, 1_000_000.0, 4096.0)?
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout.GetMetrics(&mut metrics)?;
        }
        let mut clusters = vec![DWRITE_CLUSTER_METRICS::default(); units.len()];
        let mut count = 0;
        unsafe {
            layout.GetClusterMetrics(Some(&mut clusters), &mut count)?;
        }
        let mut byte_map = Vec::with_capacity(units.len() + 1);
        for (byte, c) in text.char_indices() {
            for _ in 0..c.len_utf16() {
                byte_map.push(byte);
            }
        }
        byte_map.push(text.len());
        let mut shape = Shape {
            width: metrics.widthIncludingTrailingWhitespace,
            clusters: Vec::new(),
        };
        let mut offset = 0;
        // 渲染文字按 DirectWrite 的簇宽度推进。它的命中位置与这些坐标
        // *并非* 总是相同：回退字体（最明显的是 CJK、emoji 以及
        // CJK/拉丁边界）可能返回「画出的字形内部」的命中点，
        // 于是光标画在一个位置、源码偏移却指向别处。
        // 这里用与 DrawTextLayout 相同的累计推进量构造每个 LTR 插入边界。
        // 簇保持不可分割，连字与组合标记依然安全；被丢弃的只是
        // 那个不可靠的字形内命中坐标。
        // 从右到左的文字沿用 DirectWrite 自己的视觉排序；
        // 原生应用目前没有自己的双向文字重排层。
        let cumulative_cluster_boundaries = !text.chars().any(is_rtl);
        let mut pen_x = 0.0;
        for cluster in &clusters[..count as usize] {
            let end = offset + cluster.length as usize;
            let (left, right) = if cumulative_cluster_boundaries {
                (pen_x, pen_x + cluster.width)
            } else {
                let mut left = 0.0;
                let mut right = 0.0;
                let mut y = 0.0;
                let mut hit = DWRITE_HIT_TEST_METRICS::default();
                unsafe {
                    layout.HitTestTextPosition(
                        offset as u32,
                        false,
                        &mut left,
                        &mut y,
                        &mut hit,
                    )?;
                    layout.HitTestTextPosition(
                        offset as u32,
                        true,
                        &mut right,
                        &mut y,
                        &mut hit,
                    )?;
                }
                (left, right)
            };
            if cluster.width == 0.0 && !shape.clusters.is_empty() {
                // 回退字体单独上报的组合标记属于前一个字素；
                // 不能在它之前换行。
                shape.clusters.last_mut().unwrap().end = byte_map[end];
            } else {
                shape.clusters.push(Cluster {
                    start: byte_map[offset],
                    end: byte_map[end],
                    leading: left,
                    trailing: right,
                    advance: cluster.width,
                });
            }
            pen_x += cluster.width;
            offset = end;
        }
        Ok(shape)
    }
}

fn is_rtl(ch: char) -> bool {
    matches!(ch as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}
impl Backend for Metrics {
    fn shape(&self, text: &str, style: TextStyle, emphasis: Emphasis) -> Option<Rc<Shape>> {
        if text.len() > MAX_UNITS * 4 || text.encode_utf16().count() > MAX_UNITS {
            return None;
        }
        if text.is_empty() {
            return Some(Rc::new(Shape::default()));
        }
        let key = (text.to_owned(), style, emphasis);
        if let Some(hit) = self.cache.borrow().map.get(&key) {
            return Some(hit.clone());
        }
        let shape = Rc::new(self.compute(text, style, emphasis).ok()?);
        let bytes = key.0.len() * 2 + shape.clusters.len() * std::mem::size_of::<Cluster>();
        if bytes <= MAX_BYTES {
            let mut cache = self.cache.borrow_mut();
            while cache.order.len() >= MAX_ENTRIES || cache.bytes + bytes > MAX_BYTES {
                let Some((old, size)) = cache.order.pop_front() else {
                    break;
                };
                cache.map.remove(&old);
                cache.bytes -= size;
            }
            cache.map.insert(key.clone(), shape.clone());
            cache.order.push_back((key, bytes));
            cache.bytes += bytes;
        }
        Some(shape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{editor, measurement, text};
    #[test]
    fn actual_width_and_source_hits_follow_directwrite_clusters() {
        let renderer = Renderer::new().unwrap();
        let style = TextStyle::Document;
        assert!(text::measure("WWWW", style) > text::measure("iiii", style) * 2.0);
        let source = "  AVATAR office e\u{301} 😀 中文 words words tail  ";
        for width in (20..241).step_by(3).map(|w| w as f32) {
            let laid = editor::layout(source, style, width);
            let all = laid
                .lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect::<String>();
            assert_eq!(all, source);
            for (i, line) in laid.lines.iter().enumerate() {
                let value = line
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                let shape = measurement::shape(&value, style, Emphasis::None).unwrap();
                assert!(
                    shape.width <= width + 0.1,
                    "overflow {value:?}: {} > {width}",
                    shape.width
                );
                assert_eq!(value, &source[line.start..line.end]);
                for cluster in &shape.clusters {
                    let hit = laid.offset_at(i, cluster.leading + 0.001, style);
                    assert!(hit == line.start + cluster.start || hit == line.start + cluster.end);
                    assert!(source.is_char_boundary(hit));
                }
            }
        }
        let shape = renderer
            .measurements
            .shape("e\u{301}😀", style, Emphasis::None)
            .unwrap();
        assert_eq!(shape.clusters[0].end, "e\u{301}".len());
        assert_eq!(shape.clusters.last().unwrap().end, "e\u{301}😀".len());

        // 走真实的 DirectWrite 后端，而不是仅供测试的回退度量。
        // 之前中文光标「看着能点对、实际插错边界」的问题就出在这条路径上。
        let cjk = "当然，也可以在下方的设置中调节自定义背景的透明度";
        let live = crate::ui::live::layout(cjk, None, 1_000.0, &|_| None);
        let line = &live.layout.lines[0];
        let before_ye = cjk.find('也').unwrap();
        let x = measurement::shape("当然，", line.style, Emphasis::None)
            .unwrap()
            .width;
        let offset = live.offset_in_line(cjk, 0, x + 0.01);
        assert_eq!(offset, before_ye);
        let caret = live.caret(offset).unwrap();
        assert!((caret.left - (line.x + x)).abs() < 0.01);
    }
    #[test]
    fn optimized_cluster_boundaries_match_unoptimized_directwrite_reference() {
        let renderer = Renderer::new().unwrap();
        let cases = [
            "当然，也可以在下方的设置中调节自定义背景的透明度",
            "e\u{301}😀中文",
            "אבגדה",
        ];
        for source in cases {
            let actual = renderer
                .measurements
                .compute(source, TextStyle::Document, Emphasis::None)
                .unwrap();
            let expected = directwrite_reference(&renderer, source).unwrap();
            assert_eq!(actual.width, expected.width, "width for {source:?}");
            assert_eq!(
                actual.clusters.len(),
                expected.clusters.len(),
                "clusters for {source:?}"
            );
            for (actual, expected) in actual.clusters.iter().zip(expected.clusters.iter()) {
                assert_eq!(actual.start, expected.start, "start for {source:?}");
                assert_eq!(actual.end, expected.end, "end for {source:?}");
                assert_eq!(actual.leading, expected.leading, "leading for {source:?}");
                assert_eq!(
                    actual.trailing, expected.trailing,
                    "trailing for {source:?}"
                );
                assert_eq!(actual.advance, expected.advance, "advance for {source:?}");
            }
        }
    }

    fn directwrite_reference(renderer: &Renderer, text: &str) -> Result<Shape> {
        let units = text.encode_utf16().collect::<Vec<_>>();
        let format = renderer
            .formats
            .get(&(TextStyle::Document, Align::Leading, Emphasis::None))
            .ok_or_else(windows::core::Error::from_thread)?;
        let layout = unsafe {
            renderer
                .dwrite
                .CreateTextLayout(&units, format, 1_000_000.0, 4096.0)?
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout.GetMetrics(&mut metrics)?;
        }
        let mut clusters = vec![DWRITE_CLUSTER_METRICS::default(); units.len()];
        let mut count = 0;
        unsafe {
            layout.GetClusterMetrics(Some(&mut clusters), &mut count)?;
        }
        let mut byte_map = Vec::with_capacity(units.len() + 1);
        for (byte, c) in text.char_indices() {
            for _ in 0..c.len_utf16() {
                byte_map.push(byte);
            }
        }
        byte_map.push(text.len());
        let mut shape = Shape {
            width: metrics.widthIncludingTrailingWhitespace,
            clusters: Vec::new(),
        };
        let mut offset = 0;
        let cumulative_cluster_boundaries = !text.chars().any(is_rtl);
        let mut pen_x = 0.0;
        for cluster in &clusters[..count as usize] {
            let end = offset + cluster.length as usize;
            let mut left = 0.0;
            let mut right = 0.0;
            let mut y = 0.0;
            let mut hit = DWRITE_HIT_TEST_METRICS::default();
            unsafe {
                layout.HitTestTextPosition(offset as u32, false, &mut left, &mut y, &mut hit)?;
                layout.HitTestTextPosition(offset as u32, true, &mut right, &mut y, &mut hit)?;
            }
            if cumulative_cluster_boundaries {
                left = pen_x;
                right = pen_x + cluster.width;
            }
            if cluster.width == 0.0 && !shape.clusters.is_empty() {
                shape.clusters.last_mut().unwrap().end = byte_map[end];
            } else {
                shape.clusters.push(Cluster {
                    start: byte_map[offset],
                    end: byte_map[end],
                    leading: left,
                    trailing: right,
                    advance: cluster.width,
                });
            }
            pen_x += cluster.width;
            offset = end;
        }
        Ok(shape)
    }
    #[test]
    fn shaping_cache_is_bounded_and_font_refresh_invalidates_it() {
        let mut renderer = Renderer::new().unwrap();
        let before = measurement::shape("WWWW", TextStyle::Document, Emphasis::None).unwrap();
        let again = measurement::shape("WWWW", TextStyle::Document, Emphasis::None).unwrap();
        assert!(Rc::ptr_eq(&before, &again));
        for n in 0..700 {
            measurement::shape(
                &format!("cache entry {n}"),
                TextStyle::Document,
                Emphasis::None,
            )
            .unwrap();
        }
        assert!(renderer.measurements.cache.borrow().map.len() <= MAX_ENTRIES);
        assert!(renderer.measurements.cache.borrow().bytes <= MAX_BYTES);
        let prior = crate::ui::editor_preferences::current();
        let mut changed = prior.clone();
        changed.font_size *= 2.0;
        crate::ui::editor_preferences::set(changed);
        renderer.refresh_text_formats().unwrap();
        let after = measurement::shape("WWWW", TextStyle::Document, Emphasis::None).unwrap();
        crate::ui::editor_preferences::set(prior);
        assert!((after.width / before.width - 2.0).abs() < 0.01);
        assert!(!Rc::ptr_eq(&before, &after));
    }
    #[test]
    fn dropping_renderers_in_any_order_releases_measurement_backend() {
        assert!(!measurement::available());
        let first = Renderer::new().unwrap();
        let second = Renderer::new().unwrap();
        drop(first);
        assert!(measurement::available());
        drop(second);
        assert!(!measurement::available());
    }
    #[test]
    fn mixed_emphasis_and_math_share_real_widths_without_losing_source() {
        let _renderer = Renderer::new().unwrap();
        let runs = text::parse_inline("中文 **WWW iii** 与 *office* `let value = 42;` $x^2$ tail");
        for style in [
            TextStyle::Document,
            TextStyle::Heading1,
            TextStyle::DocumentMono,
        ] {
            for width in [110.0, 180.0, 400.0] {
                let lines = text::wrap_runs(&runs, style, width);
                let joined = lines
                    .iter()
                    .flatten()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                let original = runs.iter().map(|r| r.text.as_str()).collect::<String>();
                assert_eq!(
                    joined
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>(),
                    original
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>()
                );
                for line in lines {
                    assert!(
                        text::measure_runs(&line, style) <= width + 0.1,
                        "mixed run overflow {line:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn wrapped_tabs_are_reshaped_at_their_new_line_origin() {
        let _renderer = Renderer::new().unwrap();
        let source = "WWW WWW WWW \t words words\t尾部";
        for width in (65..201).step_by(3).map(|w| w as f32) {
            let laid = editor::layout(source, TextStyle::Document, width);
            let joined = laid
                .lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect::<String>();
            assert_eq!(joined, source);
            for line in laid.lines {
                assert!(
                    text::measure_runs(&line.runs, TextStyle::Document) <= width + 0.01,
                    "tab overflow {:?}",
                    line.runs
                );
            }
        }
    }
}
