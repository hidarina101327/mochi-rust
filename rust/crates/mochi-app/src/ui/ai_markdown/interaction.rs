//! 处理 AI Markdown 内容中的文本选择、命中检测和链接交互。
use super::*;

impl Layout {
    /// 按绘制顺序返回可见文本 run。这里始终从与 `paint_scrolled` 相同的
    /// `Item::Text` 记录派生，保证 Markdown 换行、加样式或代码/表格区域
    /// 横向滚动时，拖拽选区不会跟着漂移。
    pub fn selectable_text(&self, offsets: Option<&Offsets>) -> Vec<SelectableText> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(item_index, item)| {
                let Item::Text {
                    rect, run, style, ..
                } = item
                else {
                    return None;
                };
                if run.text.is_empty() {
                    return None;
                }
                let offset = self
                    .scroll_regions
                    .iter()
                    .find(|region| region.items.contains(&item_index))
                    .map(|region| region.offset(offsets))
                    .unwrap_or(0.0);
                Some(SelectableText {
                    rect: Rect::new(
                        rect.left - offset,
                        rect.top,
                        rect.right - offset,
                        rect.bottom,
                    ),
                    text: run.text.clone(),
                    style: *style,
                    emphasis: run.emphasis,
                })
            })
            .collect()
    }

    /// 命中一个可见文本 run，返回最近的 UTF-8 光标边界。
    /// 指针坐标是正文局部坐标，当前横向偏移由助手消息布局提供。
    pub fn text_point_at(&self, x: f32, y: f32, offsets: Option<&Offsets>) -> Option<TextPoint> {
        let fragments = self.selectable_text(offsets);
        if fragments.is_empty() {
            return None;
        }
        let in_line = fragments
            .iter()
            .enumerate()
            .filter(|(_, fragment)| y >= fragment.rect.top && y <= fragment.rect.bottom)
            .min_by(|(_, a), (_, b)| {
                horizontal_distance(a.rect, x).total_cmp(&horizontal_distance(b.rect, x))
            });
        let (index, fragment) = in_line.or_else(|| {
            fragments.iter().enumerate().min_by(|(_, a), (_, b)| {
                vertical_distance(a.rect, y)
                    .total_cmp(&vertical_distance(b.rect, y))
                    .then_with(|| {
                        horizontal_distance(a.rect, x).total_cmp(&horizontal_distance(b.rect, x))
                    })
            })
        })?;
        let offset = if x <= fragment.rect.left {
            0
        } else if x >= fragment.rect.right {
            fragment.text.len()
        } else {
            text::offset_at_x_with_emphasis(
                &fragment.text,
                fragment.style,
                fragment.emphasis,
                x - fragment.rect.left,
            )
        };
        Some(TextPoint {
            fragment: index,
            offset,
        })
    }

    /// 需要绘制在选区底下的矩形。相邻 run 的强调样式不同时，
    /// 同一行可能出现多个矩形。
    pub fn text_selection_rects(
        &self,
        first: TextPoint,
        last: TextPoint,
        offsets: Option<&Offsets>,
    ) -> Vec<Rect> {
        let fragments = self.selectable_text(offsets);
        if fragments.is_empty() {
            return Vec::new();
        }
        let (first, last) = if first <= last {
            (first, last)
        } else {
            (last, first)
        };
        if first.fragment >= fragments.len() || last.fragment >= fragments.len() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (index, fragment) in fragments.iter().enumerate() {
            if index < first.fragment || index > last.fragment {
                continue;
            }
            let from = if index == first.fragment {
                first.offset
            } else {
                0
            };
            let to = if index == last.fragment {
                last.offset
            } else {
                fragment.text.len()
            };
            let from = utf8_boundary(&fragment.text, from);
            let to = utf8_boundary(&fragment.text, to);
            if from >= to {
                continue;
            }
            let left = fragment.rect.left
                + text::caret_x_with_emphasis(
                    &fragment.text,
                    from,
                    fragment.style,
                    fragment.emphasis,
                );
            let right = fragment.rect.left
                + text::caret_x_with_emphasis(
                    &fragment.text,
                    to,
                    fragment.style,
                    fragment.emphasis,
                );
            let rect = Rect::new(
                left.min(right),
                fragment.rect.top,
                left.max(right),
                fragment.rect.bottom,
            );
            if !rect.is_empty() {
                out.push(rect);
            }
        }
        out
    }

    /// 提取拖拽选区对应的已渲染文本。选区跨过可见行时插入换行；
    /// 渲染时隐藏的 Markdown 语法因此永远不会被复制。
    pub fn selected_text(
        &self,
        first: TextPoint,
        last: TextPoint,
        offsets: Option<&Offsets>,
    ) -> String {
        let fragments = self.selectable_text(offsets);
        if fragments.is_empty() {
            return String::new();
        }
        let (first, last) = if first <= last {
            (first, last)
        } else {
            (last, first)
        };
        if first.fragment >= fragments.len() || last.fragment >= fragments.len() {
            return String::new();
        }
        let mut output = String::new();
        let mut previous_top = None;
        for (index, fragment) in fragments.iter().enumerate() {
            if index < first.fragment || index > last.fragment {
                continue;
            }
            let from = utf8_boundary(
                &fragment.text,
                if index == first.fragment {
                    first.offset
                } else {
                    0
                },
            );
            let to = utf8_boundary(
                &fragment.text,
                if index == last.fragment {
                    last.offset
                } else {
                    fragment.text.len()
                },
            );
            if from >= to {
                continue;
            }
            if previous_top.is_some_and(|top: f32| (top - fragment.rect.top).abs() > 0.1) {
                output.push('\n');
            }
            output.push_str(&fragment.text[from..to]);
            previous_top = Some(fragment.rect.top);
        }
        output
    }

    pub(super) fn target_contains(
        &self,
        a: &CopyTarget,
        x: f32,
        y: f32,
        offsets: Option<&Offsets>,
        area: bool,
    ) -> bool {
        if let Some(region) = a.scroll.and_then(|i| self.scroll_regions.get(i)) {
            if !region.viewport.contains(x, y) {
                return false;
            }
            if area && matches!(a.payload.kind, CopyKind::Table | CopyKind::Code) {
                return true;
            }
            return a.hit.contains(x + region.offset(offsets), y);
        }
        if area {
            a.area.contains(x, y)
        } else {
            a.hit.contains(x, y)
        }
    }
    pub fn content_at(
        &self,
        x: f32,
        y: f32,
        offsets: Option<&Offsets>,
        hover: bool,
    ) -> Option<usize> {
        if let Some((i, _, _)) = self
            .copy_targets()
            .rev()
            .find(|(i, _, _)| self.target_contains(&self.actions[*i], x, y, offsets, false))
        {
            return Some(i);
        }
        if hover {
            self.actions
                .iter()
                .enumerate()
                .rev()
                .find(|(_, a)| self.target_contains(a, x, y, offsets, true))
                .map(|(i, _)| i)
        } else {
            None
        }
    }
    pub fn link_at(&self, x: f32, y: f32, offsets: Option<&Offsets>) -> Option<String> {
        self.links.iter().rev().find_map(|link| {
            if let Some(region) = link.scroll.and_then(|index| self.scroll_regions.get(index)) {
                if region.viewport.contains(x, y)
                    && link.hit.contains(x + region.offset(offsets), y)
                {
                    return Some(link.target.clone());
                }
                return None;
            }
            link.hit.contains(x, y).then(|| link.target.clone())
        })
    }
    #[cfg(test)]
    pub fn hover_at(&self, x: f32, y: f32) -> Option<usize> {
        self.content_at(x, y, None, true)
    }
    #[cfg(test)]
    pub fn hit_at(&self, x: f32, y: f32) -> Option<usize> {
        self.content_at(x, y, None, false)
    }
    pub fn copy_payload(&self, index: usize) -> Option<&CopyPayload> {
        self.actions.get(index).map(|a| &a.payload)
    }
    pub fn copy_targets(&self) -> impl DoubleEndedIterator<Item = (usize, Rect, CopyKind)> + '_ {
        self.actions
            .iter()
            .enumerate()
            .map(|(i, a)| (i, a.hit, a.payload.kind))
    }
    #[cfg(test)]
    pub fn paint(&self, list: &mut DrawList, origin: (f32, f32), clip: Rect, p: &Palette) {
        self.paint_with_hover(list, origin, clip, p, None)
    }
    #[cfg(test)]
    pub fn paint_with_hover(
        &self,
        list: &mut DrawList,
        origin: (f32, f32),
        clip: Rect,
        p: &Palette,
        hover: Option<usize>,
    ) {
        self.paint_interactive(list, origin, clip, p, hover, false)
    }
    #[cfg(test)]
    pub fn paint_interactive(
        &self,
        list: &mut DrawList,
        origin: (f32, f32),
        clip: Rect,
        p: &Palette,
        hover: Option<usize>,
        hot: bool,
    ) {
        self.paint_scrolled(list, origin, clip, p, hover, hot, None, None)
    }
    pub fn paint_scrolled(
        &self,
        list: &mut DrawList,
        origin: (f32, f32),
        clip: Rect,
        p: &Palette,
        hover: Option<usize>,
        hot: bool,
        offsets: Option<&Offsets>,
        hover_region: Option<ScrollId>,
    ) {
        list.push_clip(clip);
        let moved = |r: Rect| {
            Rect::new(
                r.left + origin.0,
                r.top + origin.1,
                r.right + origin.0,
                r.bottom + origin.1,
            )
        };
        let mut region_cursor = 0;
        for (item_index, item) in self.items.iter().enumerate() {
            while self
                .scroll_regions
                .get(region_cursor)
                .is_some_and(|r| r.items.end <= item_index)
            {
                region_cursor += 1;
            }
            let region = self
                .scroll_regions
                .get(region_cursor)
                .filter(|r| r.items.contains(&item_index));
            if let Some(region) = region.filter(|r| r.items.start == item_index) {
                list.push_clip(moved(region.viewport));
            }
            let offset = region.map_or(0.0, |r| r.offset(offsets));
            let shifted = |r: Rect| Rect::new(r.left - offset, r.top, r.right - offset, r.bottom);
            let moved = |r: Rect| moved(shifted(r));
            match item {
                Item::Text {
                    rect,
                    run,
                    style,
                    tone,
                    syntax,
                } => {
                    let mut r = moved(*rect);
                    if r.bottom < clip.top
                        || r.top > clip.bottom
                        || (run.emphasis.base() == Emphasis::Math && !list.rect_visible(r))
                    {
                        if region.is_some_and(|r| r.items.end == item_index + 1) {
                            list.pop_clip();
                        }
                        continue;
                    }
                    let link = run.emphasis.base() == Emphasis::Link
                        || matches!(run.emphasis,Emphasis::Styled{flags,..}if flags&32!=0);
                    let color = if let Some(token) = syntax {
                        // AI 代码卡在两套主题下都用同一深色底，
                        // 因此用深色高亮配色，而不是随消息主题走。
                        highlight::color(*token, true)
                    } else if link {
                        p.accent
                    } else {
                        match tone {
                            Tone::Code => 0xf7f5f1,
                            Tone::Muted => p.muted,
                            _ => p.foreground,
                        }
                    };
                    if run.emphasis.base() == Emphasis::Code {
                        let font = style.font_size() * 0.9;
                        let cy = (r.top + r.bottom) / 2.0;
                        list.rounded_rect(
                            Rect::new(r.left, cy - font * 0.65, r.right, cy + font * 0.65),
                            4.0,
                            theme::mix(p.accent, p.area_assistant_default, 0.1),
                        );
                        let padding = text::inline_code_padding(*style, run.emphasis);
                        r.left += padding;
                        r.right -= padding;
                    }
                    if run.emphasis.base() == Emphasis::Math
                        && hover
                            .and_then(|i| self.actions.get(i))
                            .is_some_and(|a| a.payload.kind == CopyKind::Formula && a.hit == *rect)
                    {
                        list.rounded_rect_alpha(r, 4.0, p.accent, 0.12);
                    }
                    list.text_run(
                        r,
                        run.text.clone(),
                        *style,
                        color,
                        Align::Leading,
                        run.emphasis,
                    );
                }
                Item::CodeBox(r) => {
                    if crate::ui::settings_values::boolean("background.enabled", false) {
                        list.rounded_rect_alpha(
                            moved(*r),
                            8.0,
                            0x1f2933,
                            if theme::is_dark(p) { 0.5 } else { 0.6 },
                        );
                    } else {
                        list.rounded_rect(moved(*r), 8.0, 0x1f2933);
                    }
                }
                Item::CodeHeader { rect, language } => {
                    let r = moved(*rect);
                    // 顶部只留一条克制的色带，让语言标识可读，
                    // 又不会把代码再包一层卡片。保留卡片顶部圆角；
                    // 从圆角往下填满，让色带下缘与代码主体平齐相接，
                    // 不留下弧形接缝。
                    list.rounded_rect(r, 8.0, 0x283646);
                    list.rect(Rect::new(r.left, r.top + 8.0, r.right, r.bottom), 0x283646);
                    let available = (r.width() - 86.0).max(1.0);
                    list.text_aligned(
                        Rect::new(r.left + 12.0, r.top, r.left + 12.0 + available, r.bottom),
                        text::ellipsize(language, TextStyle::Small, available),
                        TextStyle::Small,
                        0xcbd5e1,
                        Align::Leading,
                    );
                }
                Item::ParagraphCard(r) => {
                    let r = moved(*r);
                    list.rounded_rect_alpha(r, 4.0, p.accent, 0.05);
                    list.rect(Rect::new(r.left, r.top, r.left + 3.0, r.bottom), p.accent);
                }
                Item::QuoteBar(r) => list.rect(moved(*r), p.border),
                Item::Rule(r) => list.rect(moved(*r), p.border),
                Item::Cell(r, header) => {
                    let r = moved(*r);
                    if *header {
                        list.rect(r, p.surface);
                    }
                    list.rounded_border(r, 0.0, p.border);
                }
                Item::Math {
                    rect,
                    tex,
                    size,
                    wrap,
                } => {
                    let r = moved(*rect);
                    if list.rect_visible(r) {
                        if hover
                            .and_then(|i| self.actions.get(i))
                            .is_some_and(|a| a.payload.kind == CopyKind::Formula && a.hit == *rect)
                        {
                            list.rounded_rect_alpha(moved(*rect), 4.0, p.accent, 0.12);
                        }
                        list.math_wrapped(r, tex, *size, p.foreground, *wrap)
                    }
                }
                Item::PushClip(r) => list.push_clip(moved(*r)),
                Item::PopClip => list.pop_clip(),
            }
            if region.is_some_and(|r| r.items.end == item_index + 1) {
                list.pop_clip();
            }
        }
        if let Some(target) = hover.and_then(|i| self.actions.get(i)) {
            match target.payload.kind {
                CopyKind::Formula => {}
                CopyKind::Table | CopyKind::Code => {
                    let region = target.scroll.and_then(|i| self.scroll_regions.get(i));
                    if let Some(region) = region {
                        list.push_clip(moved(region.viewport));
                    }
                    let offset = region.map_or(0.0, |r| r.offset(offsets));
                    let r = moved(Rect::new(
                        target.hit.left - offset,
                        target.hit.top,
                        target.hit.right - offset,
                        target.hit.bottom,
                    ));
                    if hot {
                        list.glass_button(r, 6.0, p, true);
                    } else {
                        list.rounded_rect(r, 6.0, p.surface);
                        list.rounded_border(r, 6.0, p.border);
                    }
                    let label = if target.payload.kind == CopyKind::Code {
                        "复制代码"
                    } else {
                        "复制表格"
                    };
                    let color = if hot {
                        p.button_foreground()
                    } else {
                        p.foreground
                    };
                    if target.payload.kind == CopyKind::Code {
                        list.icon_centered(
                            Rect::new(r.left + 5.0, r.top, r.left + 21.0, r.bottom),
                            Icon::COPY,
                            13.0,
                            color,
                        );
                        list.text_aligned(
                            Rect::new(r.left + 21.0, r.top, r.right - 4.0, r.bottom),
                            label,
                            TextStyle::Small,
                            color,
                            Align::Center,
                        );
                    } else {
                        list.text_aligned(r, label, TextStyle::Small, color, Align::Center);
                    }
                    if region.is_some() {
                        list.pop_clip();
                    }
                }
            }
        }
        for region in &self.scroll_regions {
            if hover_region == Some(region.id) {
                let r = moved(region.thumb(region.offset(offsets)));
                list.rounded_rect(
                    Rect::new(r.left + 2.0, r.top + 2.0, r.right - 2.0, r.bottom - 2.0),
                    4.0,
                    p.border,
                );
            }
        }
        list.pop_clip();
    }
}
