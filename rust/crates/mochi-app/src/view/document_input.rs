//! 处理文档点击、任务勾选、链接打开和文本选择。
use super::*;

impl DocPane {
    /// 点击定位光标。坐标是客户区坐标。
    pub fn click(
        &mut self,
        area: Rect,
        buffer: &mut TextBuffer,
        scroll: f32,
        x: f32,
        y: f32,
        extend: bool,
    ) {
        let tab = self
            .key
            .and_then(|(tab, _, _, _, _)| (tab != usize::MAX).then_some(tab));
        let origin_x = area.left + crate::ui::editor_preferences::current().padding_left;
        let local_x = x - origin_x;
        let local_y = y - area.top + scroll;
        // 指针点击会结束 IME 组合输入，因此应先取消组合，
        // 再进行命中测试并重新排版已提交的源码。若将
        // 预览中的字节位置反向映射到替换前的内容，
        // 这种反向映射本身就有歧义（尤其当前后预览中都出现相同的中日韩文字时）；旧的反向映射可能
        // 会选择段落末尾，但屏幕上显示的光标却在
        // 段落中间。下一个字符只会写入已提交的源码缓冲区，
        // 因此命中测试也必须使用这个缓冲区。
        //
        buffer.cancel_composition();
        // 外部缓冲区更新后、界面重绘前，也可能收到点击事件。
        // `ensure` 通常能命中缓存，但仍须确保命中测试
        // 使用的几何信息与当前源码版本一致。
        self.ensure_buffer(area, tab, buffer, Some(buffer.cursor()));
        let offset = self.live.hit(buffer.text(), local_x, local_y);
        if let Some(offset) = offset {
            let offset = offset.max(self.live.min_offset());
            if !extend && !crate::ui::editor_preferences::current().live_line_source {
                if let Some(rb) = self
                    .live
                    .parsed
                    .block_at(offset)
                    .map(|i| &self.live.parsed.blocks[i])
                {
                    if matches!(
                        rb.block,
                        document::Block::Image { .. } | document::Block::Divider
                    ) {
                        let mut end = rb.end;
                        let source = buffer.text();
                        if source
                            .get(end..)
                            .is_some_and(|tail| tail.starts_with("\r\n"))
                        {
                            end += 2;
                        } else if source.get(end..).is_some_and(|tail| tail.starts_with('\n')) {
                            end += 1;
                        }
                        buffer.set_cursor(rb.start, false);
                        buffer.set_cursor(end, true);
                        return;
                    }
                }
            }
            buffer.set_cursor(offset, extend);
            self.desired_x = self
                .live
                .locate(offset)
                .map(|(i, x)| self.live.layout.lines[i].x + x)
                .unwrap_or(local_x);
        }
    }

    /// 重设目标横坐标。左右移动或编辑之后调（必须在重排之后）。
    pub fn sync_desired_x(&mut self, cursor: usize) {
        if let Some((i, x)) = self.live.locate(cursor) {
            self.desired_x = self.live.layout.lines[i].x + x;
        }
    }

    /// 点击落在任务项的勾选框上：切换 `[ ]` ↔ `[x]`。返回是否切换了。
    /// 勾选框是列表项那一行的「项目符号」行（`visible_start == usize::MAX`）。
    pub fn toggle_task_at(
        &self,
        area: Rect,
        buffer: &mut TextBuffer,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> bool {
        let origin_x = area.left + crate::ui::editor_preferences::current().padding_left;
        let local_x = x - origin_x;
        let local_y = y - area.top + scroll;
        // 项目符号行与正文首行同一个 y，`line_at` 会取后压入的正文行；这里要专门找符号行
        let Some(line) = self.live.layout.lines.iter().find(|l| {
            l.source.is_none()
                && l.visible_start == usize::MAX
                && l.y <= local_y
                && local_y < l.y + l.height
                && matches!(
                    l.runs.first().map(|r| r.text.as_str()),
                    Some("☐") | Some("☑")
                )
        }) else {
            return false;
        };
        // 命中区比 16px 方框略宽一点，好点
        if local_x < line.x - 2.0 || local_x > line.x + document::CHECKBOX_SIZE + 6.0 {
            return false;
        }
        let rb = &self.live.parsed.blocks[line.block];
        let Some(src) = buffer.text().get(rb.start..rb.end) else {
            return false;
        };
        let Some(open) = src.find('[') else {
            return false;
        };
        let Some(close) = src[open..].find(']') else {
            return false;
        };
        let inner = &src[open + 1..open + close];
        let replacement = if inner.trim().is_empty() { "x" } else { " " };
        buffer.replace_range(rb.start + open + 1..rb.start + open + close, replacement);
        true
    }

    /// 点击落在代码块头部的复制按钮上时，返回代码正文（不含围栏）。
    pub fn code_copy_at(
        &self,
        area: Rect,
        text: &str,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> Option<String> {
        let crate::ui::code_blocks::Hit::Copy(start) = self.code_header_at(area, scroll, x, y)?
        else {
            return None;
        };
        let rb = self
            .live
            .parsed
            .blocks
            .iter()
            .find(|rb| rb.start == start)?;
        match &rb.block {
            document::Block::Code { lines, .. } => Some(lines.join("\n")),
            _ => {
                // 活动块：从源码里去掉首尾围栏行
                let src = text.get(rb.start..rb.end)?;
                let mut it: Vec<&str> = src.lines().collect();
                if it
                    .first()
                    .map(|l| l.trim_start().starts_with("```"))
                    .unwrap_or(false)
                {
                    it.remove(0);
                }
                if it
                    .last()
                    .map(|l| l.trim_start().starts_with("```"))
                    .unwrap_or(false)
                {
                    it.pop();
                }
                Some(it.join("\n"))
            }
        }
    }

    /// 点击落在渲染态的链接文字上时，返回链接目标。活动块露着源码，点它是编辑不是跳转。
    pub fn link_at(&self, area: Rect, text: &str, scroll: f32, x: f32, y: f32) -> Option<String> {
        let origin_x = area.left + crate::ui::editor_preferences::current().padding_left;
        let local_x = x - origin_x;
        let local_y = y - area.top + scroll;
        let i = self.live.line_at(local_y)?;
        let line = &self.live.layout.lines[i];
        if line.source.is_some() {
            return None;
        }
        if matches!(
            line.decoration,
            document::Decoration::AiLocator | document::Decoration::ObjectReference { .. }
        ) {
            if !area.contains(x, y)
                || local_y < line.y
                || local_y >= line.y + line.height
                || local_x < line.x
                || local_x >= line.x + document::content_width(area)
            {
                return None;
            }
            let rb = &self.live.parsed.blocks[line.block];
            if let document::Block::ObjectReference(reference) = &rb.block {
                if reference.text_style
                    && local_x
                        >= line.x
                            + (crate::ui::text::measure(
                                &reference.label,
                                crate::ui::draw::TextStyle::Document,
                            ) + 48.0)
                                .min(document::content_width(area))
                {
                    return None;
                }
                return Some(reference.target.clone());
            }
            return text.get(rb.start..rb.end).map(str::to_owned);
        }
        // 先看指针下的 run 是不是链接文字——不是就别费劲回查
        let mut run_x = line.x;
        let mut on_link = false;
        for run in &line.runs {
            let w = crate::ui::text::measure_runs(std::slice::from_ref(run), line.style);
            if local_x >= run_x && local_x < run_x + w {
                on_link = run.emphasis.base() == crate::ui::text::Emphasis::Link;
                break;
            }
            run_x += w;
        }
        if !on_link {
            return None;
        }
        let offset = self.live.offset_in_line(text, i, local_x);
        let rb = &self.live.parsed.blocks[line.block];
        let block_src = text.get(rb.start..rb.end)?;
        crate::ui::text::link_at(block_src, offset.saturating_sub(rb.start)).map(|l| l.target)
    }

    /// 编辑相关的按键。与源码模式同一套键位；只有几何来源不同。
    ///
    /// 复合结构（代码、表格、公式等）先选中当前块，连续第二次 Ctrl+A 才扩展
    /// 到整篇文档。这与编辑器内的嵌套块选择一致，也避免在代码块里想复制时
    /// 误把整篇笔记放进剪贴板。
    pub fn select_current_block_or_all(&self, buffer: &mut TextBuffer) {
        let selection = buffer.selection();
        if let Some(block) = self
            .live
            .parsed
            .block_at(buffer.cursor())
            .and_then(|index| self.live.parsed.blocks.get(index))
            .filter(|block| {
                matches!(
                    block.block,
                    document::Block::Container(_)
                        | document::Block::AiLocator(_)
                        | document::Block::ObjectReference(_)
                        | document::Block::Math(_)
                        | document::Block::Quote(_)
                        | document::Block::Code { .. }
                        | document::Block::Table { .. }
                        | document::Block::Image { .. }
                        | document::Block::Divider
                )
            })
        {
            let block_range = (block.start, block.end);
            if selection != block_range {
                buffer.set_cursor(block.start, false);
                buffer.set_cursor(block.end, true);
                return;
            }
        }
        buffer.set_cursor(self.live.min_offset(), false);
        buffer.set_cursor(buffer.text().len(), true);
    }

    pub fn handle_key(
        &self,
        buffer: &mut TextBuffer,
        key: u16,
        shift: bool,
        ctrl: bool,
    ) -> KeyOutcome {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB,
            VK_UP,
        };
        let min = self.live.min_offset();
        if [VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0].contains(&key) {
            if let Some(offset) = self.adjacent_page_caret(
                buffer.cursor(),
                key == VK_RIGHT.0 || key == VK_DOWN.0,
                key == VK_UP.0 || key == VK_DOWN.0,
            ) {
                buffer.set_cursor(offset, shift);
                return KeyOutcome::MOVED;
            }
        }
        match key {
            k if k == VK_LEFT.0 => {
                self.live.move_horizontal(buffer, false, shift);
                KeyOutcome::MOVED
            }
            k if k == VK_RIGHT.0 => {
                self.live.move_horizontal(buffer, true, shift);
                KeyOutcome::MOVED
            }
            k if k == VK_UP.0 || k == VK_DOWN.0 => {
                let delta = if k == VK_UP.0 { -1 } else { 1 };
                let next = self
                    .live
                    .move_vertical(buffer.text(), buffer.cursor(), delta, self.desired_x)
                    .max(min);
                buffer.set_cursor(next, shift);
                KeyOutcome::MOVED_VERTICALLY
            }
            k if k == VK_HOME.0 => {
                let (start, _) = self.live.line_bounds(buffer.cursor());
                buffer.set_cursor(if ctrl { min } else { start.max(min) }, shift);
                KeyOutcome::MOVED
            }
            k if k == VK_END.0 => {
                let (_, end) = self.live.line_bounds(buffer.cursor());
                buffer.set_cursor(if ctrl { buffer.text().len() } else { end }, shift);
                KeyOutcome::MOVED
            }
            k if k == VK_BACK.0 => {
                if !crate::ui::editor_preferences::current().live_line_source {
                    crate::ui::rich::delete(buffer, true, &self.live.parsed);
                    return KeyOutcome::EDITED;
                }
                if buffer.cursor() > min || buffer.has_selection() {
                    buffer.delete_backward();
                }
                KeyOutcome::EDITED
            }
            k if k == VK_DELETE.0 => {
                if !crate::ui::editor_preferences::current().live_line_source {
                    crate::ui::rich::delete(buffer, false, &self.live.parsed);
                    return KeyOutcome::EDITED;
                }
                buffer.delete_forward();
                KeyOutcome::EDITED
            }
            k if k == VK_RETURN.0 => {
                if crate::ui::editor_preferences::current().live_line_source {
                    buffer.insert("\n");
                } else {
                    crate::ui::rich::enter(buffer, shift);
                }
                KeyOutcome::EDITED
            }
            k if k == VK_TAB.0 => {
                crate::ui::rich::indent(buffer, shift);
                KeyOutcome::EDITED
            }
            _ if ctrl && key == b'A' as u16 => {
                self.select_current_block_or_all(buffer);
                KeyOutcome::MOVED
            }
            _ => KeyOutcome::IGNORED,
        }
    }
}
