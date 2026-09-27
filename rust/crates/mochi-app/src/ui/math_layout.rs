//! 兼容原生 KaTeX 的公式布局。不使用 DOM、JavaScript 或网络。
//! 坐标以 em 为单位，因此字体或 DPI 变化后仍可复用缓存布局。
use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_types::{color::Color, display_item::DisplayList, math_style::MathStyle};
use std::{cell::RefCell, collections::VecDeque, sync::Arc};

const MAX_INPUT: usize = 16 * 1024;
const MAX_ITEMS: usize = 8192;
const CACHE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PIXELS: f32 = 4_000_000.0;
type Entry = (String, u32, Option<Arc<DisplayList>>, usize, Option<u32>);
thread_local! { static CACHE: RefCell<VecDeque<Entry>> = const { RefCell::new(VecDeque::new()) }; }

pub fn formula(tex: &str, color: u32) -> Option<Arc<DisplayList>> {
    formula_with_wrap(tex, color, None)
}
fn formula_with_wrap(tex: &str, color: u32, wrap: Option<f32>) -> Option<Arc<DisplayList>> {
    if wrap.is_some_and(|w| !w.is_finite() || w <= 0.0) {
        return None;
    }
    if tex.len() > MAX_INPUT {
        return None;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(i) = cache
            .iter()
            .position(|(t, c, _, _, w)| t == tex && *c == color && *w == wrap.map(f32::to_bits))
        {
            let entry = cache.remove(i).unwrap();
            let result = entry.2.clone();
            cache.push_back(entry);
            return result;
        }
        // 依赖库会限制输入和 AST 深度为 32，并限制宏展开次数为 1000。
        // 输出仅包含字形、路径、矩形和线条，不会执行 I/O。
        let value = ratex_parser::parser::parse(tex).ok().and_then(|ast| {
            let color = Color::rgb(
                ((color >> 16) & 255) as f32 / 255.0,
                ((color >> 8) & 255) as f32 / 255.0,
                (color & 255) as f32 / 255.0,
            );
            // Electron 的 MathView 始终使用 displayMode:false，即使输入内容使用 $$。
            let options = LayoutOptions::default()
                .with_style(if wrap.is_some() {
                    MathStyle::Display
                } else {
                    MathStyle::Text
                })
                .with_color(color);
            let root = layout(&ast, &options);
            let list = if let Some(width) = wrap {
                super::math_wrap::reflow(&ast, root, &options, f64::from(width))
            } else {
                to_display_list(&root)
            };
            (list.items.len() <= MAX_ITEMS
                && [list.width, list.height, list.depth]
                    .iter()
                    .all(|v| v.is_finite())
                && list.width >= 0.0
                && list.width <= 4096.0
                && list.total_height() >= 0.0
                && list.total_height() <= 512.0
                && list.height.abs() <= 512.0
                && list.depth.abs() <= 512.0)
                .then(|| Arc::new(list))
        });
        let cost = tex.len()
            + value
                .as_ref()
                .and_then(|v| serde_json::to_vec(v.as_ref()).ok())
                .map_or(0, |v| v.len());
        if cost <= CACHE_BYTES {
            while cache.len() >= 64 || cache.iter().map(|e| e.3).sum::<usize>() + cost > CACHE_BYTES
            {
                cache.pop_front();
            }
            cache.push_back((
                tex.to_owned(),
                color,
                value.clone(),
                cost,
                wrap.map(f32::to_bits),
            ));
        }
        value
    })
}

pub fn size(tex: &str, font_size: f32) -> Option<(f32, f32)> {
    let value = formula(tex, 0)?;
    Some((
        value.width as f32 * font_size,
        value.total_height() as f32 * font_size,
    ))
}

pub fn bitmap_key(tex: &str, font_size: f32, color: u32, density: f32) -> String {
    bitmap_key_for(tex, font_size, color, density, None)
}
pub fn bitmap_key_for(
    tex: &str,
    font_size: f32,
    color: u32,
    density: f32,
    wrap: Option<f32>,
) -> String {
    // 缓存键包含完整源码，因此不同公式不会因哈希冲突而共用布局。
    format!(
        "math://{:08x}/{color:06x}/{:08x}/{:?}/{tex}",
        font_size.to_bits(),
        density.to_bits(),
        wrap.map(f32::to_bits)
    )
}

pub fn png(tex: &str, font_size: f32, color: u32, density: f32) -> Result<Vec<u8>, String> {
    png_for(tex, font_size, color, density, None)
}
pub fn size_wrapped(tex: &str, font_size: f32, width: f32) -> Option<(f32, f32)> {
    if !font_size.is_finite() || font_size <= 0.0 || !width.is_finite() || width <= 0.0 {
        return None;
    }
    let list = formula_with_wrap(tex, 0, Some(width / font_size))?;
    Some((
        list.width as f32 * font_size,
        list.total_height() as f32 * font_size,
    ))
}
pub fn png_for(
    tex: &str,
    font_size: f32,
    color: u32,
    density: f32,
    wrap: Option<f32>,
) -> Result<Vec<u8>, String> {
    if !font_size.is_finite() || font_size <= 0.0 || !density.is_finite() {
        return Err("公式尺寸无效".into());
    }
    let value = formula_with_wrap(tex, color, wrap.map(|width| width / font_size))
        .ok_or("公式无法排版或超出安全限制")?;
    let density = density.clamp(1.0, 4.0);
    let width = (value.width as f32 * font_size * density).ceil().max(1.0);
    let height = (value.total_height() as f32 * font_size * density)
        .ceil()
        .max(1.0);
    if width > 8192.0 || height > 8192.0 || width * height > MAX_PIXELS {
        return Err("公式图像过大".into());
    }
    ratex_render::render_to_png(
        &value,
        &ratex_render::RenderOptions {
            font_size,
            padding: 0.0,
            device_pixel_ratio: density,
            background_color: Color::new(0.0, 0.0, 0.0, 0.0),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapping_width_is_in_metrics_and_bitmap_cache_keys() {
        let source = "a+b+c+d+e+f+g+h+i+j";
        let wide = size_wrapped(source, 16.0, 500.0).unwrap();
        let small = size_wrapped(source, 16.0, 60.0).unwrap();
        assert!(small.1 > wide.1);
        assert_ne!(
            bitmap_key_for(source, 16.0, 0, 2.0, Some(60.0)),
            bitmap_key_for(source, 16.0, 0, 2.0, Some(500.0))
        );
        let png = png_for(source, 16.0, 0, 2.0, Some(60.0)).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(size_wrapped(source, 0.0, 60.0).is_none());
        assert!(size_wrapped(source, 16.0, f32::NAN).is_none());
    }
    #[test]
    fn full_math_constructs_produce_native_display_lists() {
        for tex in [
            r"\frac{-b\pm\sqrt{b^2-4ac}}{2a}",
            r"\int_0^\infty e^{-x^2}\,dx",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\begin{cases}x^2&x>0\\0&x\le0\end{cases}",
            r"\sum_{i=1}^{n}\frac{1}{i^2}",
        ] {
            let list = formula(tex, 0).unwrap_or_else(|| panic!("{tex}"));
            assert!(list.width > 0.0 && list.total_height() > 0.0 && !list.items.is_empty());
        }
    }
    #[test]
    fn fractions_are_stacked_and_cached_metrics_scale_without_reparse() {
        let a = formula(r"\frac{1}{2}", 0).unwrap();
        let b = formula(r"\frac{1}{2}", 0).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(a
            .items
            .iter()
            .any(|i| matches!(i, ratex_types::display_item::DisplayItem::Line { .. })));
        let small = size(r"\frac{1}{2}", 20.0).unwrap();
        let large = size(r"\frac{1}{2}", 40.0).unwrap();
        assert_eq!(large, (small.0 * 2.0, small.1 * 2.0));
    }
    #[test]
    fn malformed_recursive_and_oversized_inputs_fail_safely() {
        assert!(formula(r"\notARealCommand", 0).is_none());
        assert!(formula(r"\def\a{\a}\a", 0).is_none());
        assert!(formula(&"{".repeat(200), 0).is_none());
        assert!(formula(&"x".repeat(MAX_INPUT + 1), 0).is_none());
        assert!(png("x", 1e6, 0, 4.0).is_err());
    }
    #[test]
    fn bundled_fonts_render_offline_with_transparent_png_output() {
        let png = png(r"\frac{1}{2}+\sqrt{x}", 29.04, 0xeeeeee, 2.0).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
