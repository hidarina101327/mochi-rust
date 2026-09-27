//! 换行时不拆分公式原子，并使用原生排版后的文本字形。
//! 保留源码偏移，并在每个可视行完成后重新测量。
use super::{
    draw::TextStyle,
    text::{self, Emphasis, Run},
};
#[derive(Clone, Copy)]
struct Unit<'a> {
    text: &'a str,
    emphasis: Emphasis,
    math: bool,
    width: f32,
    offset: usize,
}
impl Unit<'_> {
    fn space(self) -> bool {
        !self.math && self.text == " "
    }
    fn break_after(self) -> bool {
        self.math || self.space() || self.text.chars().next().is_some_and(super::draw::is_wide)
    }
}
pub fn wrap(runs: &[Run], style: TextStyle, max_width: f32) -> Vec<Vec<Run>> {
    wrap_mapped(runs, style, max_width)
        .into_iter()
        .map(|(runs, _)| runs)
        .collect()
}
pub fn wrap_mapped(runs: &[Run], style: TextStyle, max_width: f32) -> Vec<(Vec<Run>, usize)> {
    wrap_impl(runs, style, max_width, false)
}
pub fn wrap_source(source: &str, style: TextStyle, max_width: f32) -> Vec<Vec<Run>> {
    wrap_impl(&[Run::plain(source)], style, max_width, true)
        .into_iter()
        .map(|(runs, _)| runs)
        .collect()
}
fn wrap_impl(
    runs: &[Run],
    style: TextStyle,
    max_width: f32,
    preserve_spaces: bool,
) -> Vec<(Vec<Run>, usize)> {
    let mut units = Vec::new();
    let mut offset = 0;
    for run in runs {
        let first = units.len();
        if run.emphasis.base() == Emphasis::Math {
            units.push(Unit {
                text: &run.text,
                emphasis: run.emphasis,
                math: true,
                width: text::run_width(run, style),
                offset,
            });
            offset += text::visible_len(run);
        } else if let Some(shape) = super::measurement::shape(&run.text, style, run.emphasis) {
            for cluster in &shape.clusters {
                let part = &run.text[cluster.start..cluster.end];
                units.push(Unit {
                    text: part,
                    emphasis: run.emphasis,
                    math: false,
                    width: cluster.advance,
                    offset,
                });
                offset += part.chars().count();
            }
        } else {
            for (i, ch) in run.text.char_indices() {
                units.push(Unit {
                    text: &run.text[i..i + ch.len_utf8()],
                    emphasis: run.emphasis,
                    math: false,
                    width: text::advance_for(ch, style, run.emphasis) * style.font_size(),
                    offset,
                });
                offset += 1;
            }
        }
        let padding = text::inline_code_padding(style, run.emphasis);
        if units.len() > first {
            units[first].width += padding;
            units.last_mut().unwrap().width += padding;
        }
    }
    let mut lines: Vec<Vec<Unit<'_>>> = Vec::new();
    let mut line: Vec<Unit<'_>> = Vec::new();
    let mut width = 0.0;
    let mut breakpoint = None;
    for unit in units {
        if width + unit.width > max_width && !line.is_empty() && max_width > style.font_size() {
            match breakpoint {
                Some(at) if at > 0 => {
                    let mut rest = line.split_off(at);
                    while !preserve_spaces && line.last().is_some_and(|u| u.space()) {
                        line.pop();
                    }
                    while !preserve_spaces && rest.first().is_some_and(|u| u.space()) {
                        rest.remove(0);
                    }
                    lines.push(std::mem::take(&mut line));
                    line = rest;
                    width = line.iter().map(|u| u.width).sum();
                    if width + unit.width > max_width && !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                        width = 0.0;
                    }
                }
                _ => {
                    lines.push(std::mem::take(&mut line));
                    width = 0.0;
                }
            }
            breakpoint = None;
        }
        if !preserve_spaces && unit.space() && line.is_empty() && !lines.is_empty() {
            continue;
        }
        width += unit.width;
        line.push(unit);
        if unit.break_after() {
            breakpoint = Some(line.len());
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    // 重新排版完整行：段落被拆分后，制表位或字形的上下文形状可能变化。
    // 原段落的长度测算只能作为
    // 第一步，不能作为最终宽度依据。
    if super::measurement::available() || matches!(style, TextStyle::Ai { .. }) {
        let mut i = 0;
        while i < lines.len() {
            while lines[i].len() > 1
                && text::measure_runs(&regroup_units(&lines[i]), style) > max_width + 0.01
                && max_width > style.font_size()
            {
                let tail = lines[i].pop().unwrap();
                if i + 1 == lines.len() {
                    lines.push(Vec::new());
                }
                lines[i + 1].insert(0, tail);
            }
            i += 1;
        }
    }
    // 与普通文本使用相同的中日韩闭合标点规则。
    let mut i = 1;
    while i < lines.len() {
        for _ in 0..2 {
            if lines[i - 1].is_empty()
                || !lines[i].first().is_some_and(|u| {
                    !u.math
                        && u.text
                            .chars()
                            .next()
                            .is_some_and(|c| text::CLOSING_PUNCTUATION.contains(c))
                })
            {
                break;
            }
            let unit = lines[i].remove(0);
            lines[i - 1].push(unit);
        }
        if lines[i].is_empty() {
            lines.remove(i);
        } else {
            i += 1;
        }
    }
    if lines.is_empty() {
        return vec![(vec![Run::plain("")], 0)];
    }
    lines
        .into_iter()
        .map(|units| {
            let start = units.first().map_or(0, |u| u.offset);
            let runs = regroup_units(&units);
            (runs, start)
        })
        .collect()
}
fn regroup_units(units: &[Unit<'_>]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for unit in units {
        if !unit.math {
            if let Some(last) = runs.last_mut().filter(|r| r.emphasis == unit.emphasis) {
                last.text.push_str(unit.text);
                continue;
            }
        }
        runs.push(Run {
            text: unit.text.to_owned(),
            emphasis: unit.emphasis,
        });
    }
    runs
}
