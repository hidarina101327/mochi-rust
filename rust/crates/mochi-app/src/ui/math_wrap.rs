//! 根据 RaTeX 已排版的公式框，查找 KaTeX 根节点级别的换行位置。
//! 保留完整的原子和原子间空隙；不要重新序列化 TeX 片段。
use ratex_layout::{
    hbox::make_hbox,
    layout_box::BoxContent,
    spacing::{atom_spacing, MathClass as C},
    to_display_list, LayoutBox, LayoutOptions,
};
use ratex_parser::parse_node::{AtomFamily as A, ParseNode as N};
use ratex_types::{DisplayItem, DisplayList};

// 分类方式与固定版本的 RaTeX 引擎（MIT 许可）保持一致；间距映射会
// 先与输出的根节点子项数量核对，通过后才尝试重新排版。
fn class(node: &N) -> Option<C> {
    let mut pending = vec![node];
    while let Some(n) = pending.pop() {
        return match n {
            N::Atom { family, .. } => Some(match family {
                A::Bin => C::Bin,
                A::Rel => C::Rel,
                A::Open => C::Open,
                A::Close => C::Close,
                A::Punct => C::Punct,
                A::Inner => C::Inner,
            }),
            N::OpToken { .. } | N::Op { .. } | N::OperatorName { .. } => Some(C::Op),
            N::GenFrac {
                left_delim,
                right_delim,
                ..
            } => Some(
                if left_delim
                    .iter()
                    .chain(right_delim)
                    .any(|d| !d.is_empty() && d != ".")
                {
                    C::Ord
                } else {
                    C::Inner
                },
            ),
            N::SupSub { base, .. } => {
                if let Some(base) = base {
                    pending.push(base);
                }
                continue;
            }
            N::HtmlMathMl { html, .. } => {
                pending.extend(html.iter().rev());
                continue;
            }
            N::Html { body, .. } => {
                pending.extend(body.iter().rev());
                continue;
            }
            N::MClass { mclass, .. } | N::DelimSizing { mclass, .. } => {
                Some(match mclass.as_str() {
                    "mbin" => C::Bin,
                    "mrel" => C::Rel,
                    "mop" => C::Op,
                    "mopen" => C::Open,
                    "mclose" => C::Close,
                    "mpunct" => C::Punct,
                    "minner" => C::Inner,
                    _ => C::Ord,
                })
            }
            N::SpacingNode { .. } | N::Kern { .. } | N::Lap { .. } => continue,
            N::LeftRight { .. } => Some(C::Inner),
            N::XArrow { .. } | N::CdArrow { .. } => Some(C::Rel),
            _ => Some(C::Ord),
        };
    }
    None
}
fn effective(raw: &[Option<C>], skip_spaces: bool) -> Vec<Option<C>> {
    raw.iter()
        .enumerate()
        .map(|(i, c)| {
            if *c != Some(C::Bin) {
                return *c;
            }
            let before = if skip_spaces {
                raw[..i].iter().rev().find_map(|c| *c)
            } else {
                i.checked_sub(1).and_then(|j| raw[j])
            };
            let after = if skip_spaces {
                raw[i + 1..].iter().find_map(|c| *c)
            } else {
                raw.get(i + 1).copied().flatten()
            };
            if matches!(
                before,
                None | Some(C::Bin | C::Open | C::Rel | C::Op | C::Punct)
            ) || matches!(after, None | Some(C::Rel | C::Close | C::Punct))
            {
                Some(C::Ord)
            } else {
                *c
            }
        })
        .collect()
}
fn space(n: &N) -> bool {
    matches!(n, N::SpacingNode { .. } | N::Kern { .. })
}
pub fn reflow(nodes: &[N], root: LayoutBox, options: &LayoutOptions, width: f64) -> DisplayList {
    // 声明包装层在 HTML 根节点处会被视为透明结构，与 {...} 不同。
    if let [N::Styling { style, body, .. }] = nodes {
        use ratex_parser::parse_node::StyleStr;
        if matches!(style, StyleStr::Display | StyleStr::Text) {
            let opts = options.with_style(if *style == StyleStr::Display {
                ratex_types::MathStyle::Display
            } else {
                ratex_types::MathStyle::Text
            });
            if let BoxContent::HBox(children) = &root.content {
                if children.len() == 1 {
                    return reflow(body, children[0].clone(), &opts, width);
                }
            }
        }
    }
    if let [N::Color { body, .. }] = nodes {
        if let BoxContent::HBox(children) = &root.content {
            if children.len() == 1 {
                return reflow(body, children[0].clone(), options, width);
            }
        }
    }
    let original = || to_display_list(&root);
    if !width.is_finite() || width <= 0.0 || nodes.len() > 8192 {
        return original();
    }
    let BoxContent::HBox(children) = &root.content else {
        return original();
    };
    let raw = nodes.iter().map(class).collect::<Vec<_>>();
    let engine = effective(&raw, false);
    let breaks = effective(&raw, true);
    let mut starts = Vec::new();
    let mut count = 0;
    let mut previous = None;
    let mut previous_middle = false;
    for (i, n) in nodes.iter().enumerate() {
        let current = engine[i];
        let middle = matches!(n, N::Middle { .. });
        if let (Some(p), Some(c)) = (previous, current) {
            if !previous_middle && !middle && atom_spacing(p, c, options.style.is_tight()) > 0.0 {
                count += 1
            }
        }
        starts.push(count);
        count += 1;
        if current.is_some() {
            previous = current;
            previous_middle = middle;
        }
    }
    if count != children.len() {
        return original();
    }
    starts.push(count);
    let mut cuts = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if !matches!(breaks[i], Some(C::Bin | C::Rel))
            && !matches!(n,N::SpacingNode{text,..}if text=="\\allowbreak")
        {
            continue;
        }
        let mut next = i + 1;
        let mut prohibited = false;
        while next < nodes.len() && space(&nodes[next]) {
            prohibited |= matches!(&nodes[next],N::SpacingNode{text,..}if text=="\\nobreak");
            next += 1;
        }
        if !prohibited {
            cuts.push(starts[next]);
        }
    }
    cuts.push(count);
    cuts.sort_unstable();
    cuts.dedup();
    let mut rows = Vec::new();
    let mut current = Vec::new();
    let mut used = 0.0;
    let mut start = 0;
    for end in cuts {
        let chunk = &children[start..end];
        let advance = chunk.iter().map(|b| b.width).sum::<f64>();
        if used + advance > width + 1e-6 && !current.is_empty() {
            rows.push(make_hbox(std::mem::take(&mut current)));
            used = 0.0;
        }
        current.extend_from_slice(chunk);
        used += advance;
        start = end;
    }
    if !current.is_empty() {
        rows.push(make_hbox(current));
    }
    if rows.len() <= 1 {
        return original();
    }
    let full_width = rows.iter().map(|r| r.width).fold(width, f64::max);
    let mut out = DisplayList::new();
    out.width = full_width;
    let mut top = 0.0;
    for row in rows {
        let list = to_display_list(&row);
        let height = list.total_height().max(1.2);
        let dx = (full_width - list.width) / 2.0;
        let dy = top + (height - list.total_height()) / 2.0;
        for mut item in list.items {
            match &mut item {
                DisplayItem::GlyphPath { x, y, .. }
                | DisplayItem::Line { x, y, .. }
                | DisplayItem::Rect { x, y, .. }
                | DisplayItem::Path { x, y, .. } => {
                    *x += dx;
                    *y += dy;
                }
            }
            out.items.push(item);
        }
        top += height;
    }
    out.height = top;
    out.depth = 0.0;
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wrap(tex: &str, w: f64) -> DisplayList {
        let nodes = ratex_parser::parser::parse(tex).unwrap();
        let opts = LayoutOptions::default().with_style(ratex_types::MathStyle::Display);
        let root = ratex_layout::layout(&nodes, &opts);
        reflow(&nodes, root, &opts, w)
    }
    #[test]
    fn binary_relations_wrap_without_losing_glyphs() {
        let tex = "a+b+c+d+e+f=g+h+i+j+k";
        let whole = wrap(tex, 100.0);
        let small = wrap(tex, 4.0);
        assert!(small.height > whole.total_height());
        assert!(small.width <= 4.001);
        assert_eq!(whole.items.len(), small.items.len());
    }
    #[test]
    fn fraction_delimiters_and_groups_are_indivisible() {
        for tex in [
            r"\frac{a+b+c+d+e}{x+y}",
            r"\left(a+b+c+d+e\right)",
            "{a+b+c+d+e}",
        ] {
            let whole = wrap(tex, 100.0);
            let narrow = wrap(tex, 1.0);
            assert_eq!(whole.items, narrow.items);
            assert_eq!(whole.width, narrow.width);
        }
    }
    #[test]
    fn nobreak_and_explicit_allowbreak_follow_root_boundaries() {
        let no = wrap(r"a+\nobreak b", 0.5);
        assert!(no.width > 0.5);
        let yes = wrap(r"ab\allowbreak cd", 1.2);
        assert!(yes.height > 1.2);
        let unary = wrap("-x", 0.5);
        assert!(unary.width > 0.5);
    }
    #[test]
    fn display_declaration_and_color_do_not_hide_breaks() {
        for tex in [
            r"\displaystyle a+b+c+d+e+f",
            r"\color{red}a+b+c+d+e+f",
            r"\color{red}\displaystyle a+b+c+d+e+f",
        ] {
            let whole = wrap(tex, 100.0);
            let small = wrap(tex, 2.0);
            assert!(small.total_height() > whole.total_height());
            assert_eq!(whole.items.len(), small.items.len());
            let colors = |list: &DisplayList| {
                list.items
                    .iter()
                    .filter_map(|i| {
                        if let DisplayItem::GlyphPath { color, .. } = i {
                            Some(*color)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let ast = ratex_parser::parser::parse(tex).unwrap();
            let original = to_display_list(&ratex_layout::layout(&ast, &LayoutOptions::default()));
            assert_eq!(colors(&original), colors(&small));
        }
    }
}
