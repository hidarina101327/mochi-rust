//! 可见工具栏与右键菜单共用的编辑器块控件。
use super::*;
use crate::ui::containers::{self, Action, Container, Kind};

pub(super) struct ImageDrag {
    path: PathBuf,
    base: crate::ui::editor::TextBuffer,
    start: usize,
    x: f32,
    width: f32,
}
pub(super) struct TableColumnDrag {
    pub start: usize,
    pub column: usize,
    pub x: f32,
    pub widths: Vec<f32>,
}

/// Electron 的 `MochiCodeBlock` 输入规则接受 `···`（可选小写语言名）后跟
/// 空白/回车。磁盘格式仍使用标准 Markdown 反引号围栏；这里只在用户按回车
/// 时把触发符正规化，避免平时输入中文省略号被误转换。
pub(super) fn normalize_chinese_code_fence_on_newline(
    buffer: &mut crate::ui::editor::TextBuffer,
) -> bool {
    if buffer.has_selection() {
        return false;
    }
    let cursor = buffer.cursor();
    let text = buffer.text();
    let start = text[..cursor].rfind('\n').map(|at| at + 1).unwrap_or(0);
    let line = &text[start..cursor];
    let count = line.chars().take_while(|ch| *ch == '·').count();
    if count < 3 {
        return false;
    }
    let language = &line[count * '·'.len_utf8()..];
    if !language.chars().all(|ch| ch.is_ascii_lowercase()) {
        return false;
    }
    buffer.replace_range(start..cursor, &format!("{}{language}", "`".repeat(count)));
    true
}

/// 在一行围栏后按 Enter，立刻生成一个**闭合**的空代码块，光标停在两道围栏
/// 之间。用户从三个反引号开始时，自动升级为四个反引号：三反引号可以安全地
/// 出现在代码内容或粘贴内容内，而不会提前关闭刚创建的代码块。
pub(super) fn complete_code_fence_on_newline(buffer: &mut crate::ui::editor::TextBuffer) -> bool {
    if buffer.has_selection() {
        return false;
    }
    let cursor = buffer.cursor();
    let text = buffer.text();
    let start = text[..cursor].rfind('\n').map(|at| at + 1).unwrap_or(0);
    let line = text[start..cursor].trim();
    let Some((marker, count, language)) = crate::ui::document::fence_open(line) else {
        return false;
    };
    // 围栏内部输入任何围栏都交给 Markdown 原样保存：达到外层长度的是关闭围栏，
    // 比外层短（例如四反引号块内的三个反引号）则是普通代码文本。两者都不能再
    // 自动创建嵌套围栏，否则会破坏已有代码块。
    if active_fence_before(&text[..start]).is_some() {
        return false;
    }
    let count = if marker == '`' && count == 3 {
        4
    } else {
        count
    };
    let fence = marker.to_string().repeat(count);
    let opening = format!("{fence}{language}");
    buffer.replace_range(start..cursor, &opening);
    let cursor = start + opening.len();
    buffer.set_cursor(cursor, false);
    buffer.insert(&format!("\n\n{fence}"));
    buffer.set_cursor(cursor + 1, false);
    true
}

/// 两个反引号加 Enter 是 Electron 的行内代码输入手势：补上右侧两个反引号，
/// 光标回到中间；仅此手势会创建空行内代码，普通 ` `` ` 输入仍保持可见。
pub(super) fn complete_inline_code_on_newline(buffer: &mut crate::ui::editor::TextBuffer) -> bool {
    if buffer.has_selection() {
        return false;
    }
    let cursor = buffer.cursor();
    let text = buffer.text();
    let start = text[..cursor].rfind('\n').map(|at| at + 1).unwrap_or(0);
    if &text[start..cursor] != "``" {
        return false;
    }
    buffer.insert("``");
    buffer.set_cursor(cursor, false);
    true
}

fn active_fence_before(before: &str) -> Option<(char, usize)> {
    let mut open: Option<(char, usize)> = None;
    for line in before.lines() {
        let trimmed = line.trim();
        if let Some((active_marker, active_count)) = open {
            let n = trimmed
                .chars()
                .take_while(|current| *current == active_marker)
                .count();
            if n >= active_count && trimmed[n..].trim().is_empty() {
                open = None;
            }
        } else if let Some((next_marker, next_count, _)) = crate::ui::document::fence_open(trimmed)
        {
            open = Some((next_marker, next_count));
        }
    }
    open
}

fn block_end(block: &crate::ui::document::RangedBlock) -> usize {
    match &block.block {
        crate::ui::document::Block::Container(panel) => panel.footer.end,
        _ => block.end,
    }
}

fn line_break_len(source: &str, at: usize) -> usize {
    let Some(tail) = source.get(at..) else {
        return 0;
    };
    if tail.starts_with("\r\n") {
        2
    } else if tail.starts_with('\n') || tail.starts_with('\r') {
        1
    } else {
        0
    }
}

/// 富容器内渲染出来的行，在块级操作上归属该容器。`parse_ranged` 特意保留
/// 了可编辑的正文子块以便映射文本，但拿子块的结尾做"在下方插入"会把新行
/// 插到闭合定界符之前。因此选最内层的容器，嵌套面板才能追加到用户正在
/// 操作的那个面板下方。
fn enclosing_container<'a>(
    parsed: &'a crate::ui::document::Parsed,
    start: usize,
) -> Option<&'a crate::ui::document::RangedBlock> {
    parsed
        .blocks
        .iter()
        .filter(|block| match &block.block {
            crate::ui::document::Block::Container(panel) => {
                panel.header.start <= start && start <= panel.footer.end
            }
            _ => false,
        })
        .max_by_key(|block| block.start)
}

/// 对源码缓冲区执行两种块级右键编辑。
///
/// 解析区间不含行尾换行。在该边界追加一个换行，恰好得到一个可编辑的空行，
/// 同时保留原有分隔线和后面的所有块。删除时会一并吃掉块自身的尾部换行
/// （如果有的话），这样删除文档中间的块就不会留下悬空空行。
fn apply_block_edit(buffer: &mut crate::ui::editor::TextBuffer, start: usize, after: bool) -> bool {
    let parsed = crate::ui::document::parse_ranged(buffer.text());
    let Some(selected) = parsed.blocks.iter().find(|b| b.start == start) else {
        return false;
    };
    if after {
        // 被点中句柄的可能只是正文段落/表格行，而操作明确针对外层功能块。
        // 先取完整容器的页脚，再决定插入边界；普通顶层块仍用自己的结尾。
        let block = enclosing_container(&parsed, start).unwrap_or(selected);
        let end = block_end(block);
        let newline = if buffer.text().contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        buffer.replace_range_select(end..end, newline, newline.len()..newline.len());
    } else {
        let end = block_end(selected);
        let start = if matches!(selected.block, crate::ui::document::Block::Code { .. }) {
            crate::ui::code_blocks::metadata(buffer.text(), selected.start)
                .0
                .start
        } else {
            selected.start
        };
        let delete_end = end + line_break_len(buffer.text(), end);
        let range = crate::ui::block_markers::deletion_range(buffer.text(), start..delete_end);
        buffer.replace_range(range, "");
    }
    true
}

impl App {
    pub(super) fn editor_image_source(&self, start: usize) -> Option<String> {
        let buffer = self.shell.active()?.buffer()?;
        let parsed = crate::ui::document::parse_ranged(buffer.text());
        let block = parsed.blocks.iter().find(|b| b.start == start)?;
        let crate::ui::document::Block::Image { src, .. } = &block.block else {
            return None;
        };
        Some(crate::ui::document::resolve_image_src(
            src,
            self.active_file_path()?.parent(),
        ))
    }

    pub(super) fn editor_block_items(start: usize) -> Vec<MenuItem<MenuAction>> {
        vec![
            MenuItem::new("切换段落类别  ›", MenuAction::BlockParagraphMenu),
            MenuItem::new("切换格式  ›", MenuAction::BlockFormatMenu),
            MenuItem::new("AI 处理  ›", MenuAction::BlockAiMenu),
            MenuItem::new("删除功能块", MenuAction::Block(start, false))
                .icon(Icon::TRASH2)
                .separated(),
            MenuItem::new("在功能块下追加一行", MenuAction::Block(start, true)).icon(Icon::PLUS),
        ]
    }

    pub(super) fn open_block_paragraph_menu(&mut self) {
        let mut items = vec![MenuItem::new(
            "正文",
            MenuAction::Format(Format::Heading(0)),
        )];
        items.extend((1..=6).map(|level| {
            MenuItem::new(
                format!("标题 {level}"),
                MenuAction::Format(Format::Heading(level)),
            )
        }));
        items.extend([
            MenuItem::new("无序列表", MenuAction::Format(Format::BulletList)).separated(),
            MenuItem::new("有序列表", MenuAction::Format(Format::OrderedList)),
            MenuItem::new("任务列表", MenuAction::Format(Format::TaskList)),
            MenuItem::new("引用", MenuAction::Format(Format::Quote)),
            MenuItem::new("代码块", MenuAction::Format(Format::CodeBlock)),
        ]);
        self.open_menu_submenu(items);
    }

    pub(super) fn open_block_format_menu(&mut self) {
        let items = [
            ("加粗", Format::Bold),
            ("斜体", Format::Italic),
            ("下划线", Format::Underline),
            ("删除线", Format::Strike),
            ("行内代码", Format::Code),
            ("清除格式", Format::ClearFormat),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, format))| {
            let item = MenuItem::new(label, MenuAction::Format(format));
            if index == 5 {
                item.separated()
            } else {
                item
            }
        })
        .collect::<Vec<_>>();
        let mut items = items;
        items.extend([
            MenuItem::new("字体颜色  ›", MenuAction::BlockColorMenu(false)).separated(),
            MenuItem::new("背景颜色  ›", MenuAction::BlockColorMenu(true)),
            MenuItem::new("左对齐", MenuAction::BlockAlign(Align::Leading)).separated(),
            MenuItem::new("居中", MenuAction::BlockAlign(Align::Center)),
            MenuItem::new("右对齐", MenuAction::BlockAlign(Align::Trailing)),
            MenuItem::new("减少缩进", MenuAction::BlockIndent(true)).separated(),
            MenuItem::new("增加缩进", MenuAction::BlockIndent(false)),
        ]);
        self.open_menu_submenu(items);
    }

    pub(super) fn open_block_color_menu(&mut self, highlight: bool) {
        let colors = [
            ("默认", if highlight { 0xFFF2CC } else { 0x000000 }),
            ("红色", 0xEF4444),
            ("橙色", 0xF97316),
            ("黄色", 0xEAB308),
            ("绿色", 0x22C55E),
            ("青色", 0x14B8A6),
            ("蓝色", 0x3B82F6),
            ("靛蓝", 0x6366F1),
            ("紫色", 0xA855F7),
            ("粉色", 0xEC4899),
            ("灰色", 0x64748B),
            ("白色", 0xFFFFFF),
        ];
        let items = colors
            .into_iter()
            .map(|(name, color)| {
                MenuItem::new(
                    format!("●  {name}"),
                    MenuAction::TextColor(highlight, color),
                )
            })
            .collect();
        self.open_menu_submenu(items);
    }

    pub(super) fn apply_block_alignment(&mut self, alignment: Align) {
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        let (a, b) = buffer.selection();
        let (start, _) = crate::ui::format::line_range(buffer.text(), a);
        let (_, end) = crate::ui::format::line_range(buffer.text(), b);
        let replacement = buffer.text()[start..end]
            .split('\n')
            .map(|line| crate::ui::styles::align_line(line, alignment))
            .collect::<Vec<_>>()
            .join("\n");
        buffer.replace_range(start..end, &replacement);
        self.after_doc_edit(false);
    }

    pub(super) fn apply_block_indent(&mut self, outdent: bool) {
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        crate::ui::rich::indent(buffer, outdent);
        self.after_doc_edit(false);
    }

    pub(super) fn open_block_ai_menu(&mut self) {
        let configured = self
            .shell
            .workspace()
            .map(|workspace| {
                mochi_core::ai::agent_config::AgentConfigService::new(&workspace.root)
                    .load_quick_actions()
            })
            .unwrap_or_default();
        let items = if configured.is_empty() {
            [
                ("继续写作", "继续写作以下内容，保持风格和格式一致，直接输出续写部分：\n\n{{selection}}", true),
                ("润色", "润色以下文本，修正错别字，优化表达，保持原有格式和结构不变，直接输出完整内容：\n\n{{selection}}", false),
                ("总结", "用简洁的语言总结以下内容：\n\n{{selection}}", false),
                ("翻译", "将以下文本翻译成{{targetLanguage}}，保持原有格式：\n\n{{selection}}", false),
                ("格式调整", "调整以下内容的 Markdown 格式使其更易读，保持内容不变：\n\n{{selection}}", false),
                ("重点标注", "用**粗体**标记关键概念和核心术语，保持原有结构不变：\n\n{{selection}}", false),
            ]
            .into_iter()
            .map(|(label, prompt, append)| {
                MenuItem::new(label, MenuAction::SelectionAi(prompt.into(), append))
                    .icon(Icon::SPARKLES)
            })
            .collect()
        } else {
            configured
                .into_iter()
                .filter(|action| action.enabled)
                .map(|action| {
                    MenuItem::new(
                        action.name,
                        MenuAction::SelectionAi(action.prompt_template, action.apply == "append"),
                    )
                    .icon(Icon::SPARKLES)
                })
                .collect()
        };
        self.open_menu_submenu(items);
    }

    pub(super) fn editor_image_items(start: usize) -> Vec<MenuItem<MenuAction>> {
        let mut items = Self::clipboard_menu_items();
        items.extend([
            MenuItem::new("预览图片", MenuAction::PreviewEditorImage(start)).icon(Icon::IMAGE),
            MenuItem::new("复制图片", MenuAction::CopyEditorImage(start)).icon(Icon::COPY),
            MenuItem::new("调整图片宽度", MenuAction::ImageWidth(start)).icon(Icon::IMAGE),
        ]);
        items.extend(Self::editor_block_items(start));
        items
    }
    pub(super) fn copy_editor_image(&mut self, start: usize) {
        let Some(src) = self.editor_image_source(start) else {
            return;
        };
        let bytes = self
            .renderer
            .image_bytes(&src)
            .map(<[u8]>::to_vec)
            .or_else(|| std::fs::read(&src).ok());
        let copied =
            bytes.is_some_and(|bytes| platform::copy_image(HWND(self.hwnd_raw as *mut _), &bytes));
        self.show_global_notice(if copied {
            "已复制图片"
        } else {
            "图片尚未加载或剪贴板不可用，请重试"
        });
    }
    pub(super) fn paint_editor_controls(&mut self, palette: &Palette) {
        if self.content() != MainContent::Document {
            return;
        }
        self.list.push_clip(self.editor_area);
        for (_, track, thumb, _) in self
            .doc
            .table_scrollbars(self.editor_area, self.shell.active_scroll())
        {
            self.list.rounded_rect(track, 5.0, palette.surface_muted);
            self.list.rounded_rect(thumb, 5.0, palette.muted);
        }
        if let Some((mx, my)) = self.block_pointer {
            for (_, _, handle, _) in self
                .doc
                .table_column_handles(self.editor_area, self.shell.active_scroll())
            {
                if handle.contains(mx, my) {
                    self.list.rect_alpha(
                        Rect::from_size(handle.left + 3.0, handle.top, 2.0, handle.height()),
                        palette.accent,
                        0.82,
                    );
                }
            }
        }
        let selected = self.image_drag.as_ref().map(|d| d.start).or_else(|| {
            self.block_pointer.and_then(|(x, y)| {
                self.doc
                    .image_at(self.editor_area, self.shell.active_scroll(), x, y)
            })
        });
        if let Some(rect) = selected.and_then(|s| {
            self.doc
                .image_rect(self.editor_area, self.shell.active_scroll(), s)
        }) {
            self.list.rounded_border(
                rect,
                8.0,
                theme::mix(palette.accent, palette.foreground, 0.08),
            );
            let handle = Rect::from_size(rect.right - 11.0, rect.bottom - 11.0, 10.0, 10.0);
            self.list.rounded_rect(handle, 5.0, palette.surface);
            self.list.rounded_border(handle, 5.0, palette.accent);
        }
        self.list.pop_clip();
    }

    pub(super) fn begin_editor_control_drag(&mut self, x: f32, y: f32) -> bool {
        if !self.editor_area.contains(x, y) {
            return false;
        }
        for (start, column, handle, widths) in self
            .doc
            .table_column_handles(self.editor_area, self.shell.active_scroll())
        {
            if handle.contains(x, y) {
                self.table_column_drag = Some(TableColumnDrag {
                    start,
                    column,
                    x,
                    widths,
                });
                self.drag = Some(Drag {
                    target: DragTarget::EditorTableColumn,
                    grab_offset: 0.0,
                });
                return true;
            }
        }
        for (start, track, thumb, max) in self
            .doc
            .table_scrollbars(self.editor_area, self.shell.active_scroll())
        {
            if track.contains(x, y) {
                let grab = if thumb.contains(x, y) {
                    x - thumb.left
                } else {
                    thumb.width() / 2.0
                };
                self.table_drag = Some((start, track, thumb.width(), max));
                self.drag = Some(Drag {
                    target: DragTarget::EditorTable,
                    grab_offset: grab,
                });
                self.doc.set_table_scroll(
                    start,
                    ((x - track.left - grab) / (track.width() - thumb.width()).max(1.0) * max)
                        .clamp(0.0, max),
                );
                return true;
            }
        }
        let Some((start, handle)) =
            self.doc
                .image_handle(self.editor_area, self.shell.active_scroll(), x, y)
        else {
            return false;
        };
        if !handle.contains(x, y) {
            return false;
        }
        let Some(path) = self.active_file_path() else {
            return false;
        };
        let base = self.shell.active().unwrap().buffer().unwrap().clone();
        let width = self
            .doc
            .image_rect(self.editor_area, self.shell.active_scroll(), start)
            .unwrap()
            .width();
        self.image_drag = Some(ImageDrag {
            path,
            base,
            start,
            x,
            width,
        });
        self.drag = Some(Drag {
            target: DragTarget::EditorImage,
            grab_offset: 0.0,
        });
        true
    }

    pub(super) fn resize_editor_table_column(&mut self, x: f32) -> bool {
        let Some(drag) = &self.table_column_drag else {
            return false;
        };
        let mut widths = drag.widths.clone();
        let Some(right) = drag.column.checked_add(1) else {
            return false;
        };
        if right >= widths.len() {
            return false;
        }
        let delta = x - drag.x;
        let total = widths[drag.column] + widths[right];
        let left = (widths[drag.column] + delta).clamp(72.0, total - 72.0);
        widths[drag.column] = left;
        widths[right] = total - left;
        self.doc.set_table_widths(drag.start, widths);
        true
    }

    pub(super) fn resize_editor_image(&mut self, x: f32) -> bool {
        let Some(drag) = &self.image_drag else {
            return false;
        };
        if self.active_file_path().as_ref() != Some(&drag.path) {
            return false;
        }
        let width = (drag.width + x - drag.x).round().clamp(
            80.0,
            crate::ui::document::content_width(self.editor_area).max(80.0),
        );
        let mut next = drag.base.clone();
        if !crate::ui::editor_images::resize(&mut next, drag.start, &format!("{width:.0}")) {
            return false;
        }
        *self.shell.active_buffer_mut().unwrap() = next;
        self.editor_engaged = false;
        self.after_doc_edit(false);
        true
    }

    pub(super) fn editor_insert_items() -> Vec<MenuItem<MenuAction>> {
        let mut items = vec![
            MenuItem::new("Mochi 对象", MenuAction::InsertObject).icon(Icon::LINK2),
            MenuItem::new("公式", MenuAction::InsertMath).icon(Icon::CODE),
            MenuItem::new("代码块", MenuAction::Format(Format::CodeBlock)).icon(Icon::CODE2),
            MenuItem::new("表格", MenuAction::Format(Format::Table)).icon(Icon::TABLE),
            MenuItem::new("任务列表", MenuAction::Format(Format::TaskList))
                .icon(Icon::CHECK_SQUARE),
            MenuItem::new("引用", MenuAction::Format(Format::Quote)).icon(Icon::QUOTE),
            MenuItem::new("高亮块", MenuAction::Format(Format::HighlightBlock))
                .icon(Icon::PENCIL_LINE),
            MenuItem::new("折叠块", MenuAction::Format(Format::Details)).icon(Icon::CHEVRON_RIGHT),
            MenuItem::new("分割线", MenuAction::Format(Format::Rule)).icon(Icon::MINUS),
            MenuItem::new("图片", MenuAction::Format(Format::Image)).icon(Icon::IMAGE),
            MenuItem::new("链接", MenuAction::Format(Format::Link)).icon(Icon::LINK),
        ];
        if crate::ui::settings_values::boolean("editor.simpleDocumentMode", true) {
            items.retain(|item| {
                !matches!(
                    item.action,
                    MenuAction::InsertObject
                        | MenuAction::Format(
                            Format::HighlightBlock | Format::Details | Format::Underline
                        )
                )
            });
        }
        items
    }

    pub(super) fn slash_insert_items() -> Vec<MenuItem<MenuAction>> {
        let mut items = Self::editor_insert_items()
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let key = if index < 9 {
                    Some((index + 1).to_string())
                } else if index == 9 {
                    Some("0".to_owned())
                } else {
                    None
                };
                match key {
                    Some(key) => item.shortcut(key),
                    None => item,
                }
            })
            .collect::<Vec<_>>();
        items.push(
            MenuItem::new("Esc 退出", MenuAction::Format(Format::ClearFormat))
                .separated()
                .hint(),
        );
        items
    }

    pub(super) fn remove_slash_at(
        buffer: &mut crate::ui::editor::TextBuffer,
        offset: usize,
    ) -> bool {
        if buffer.text().as_bytes().get(offset) != Some(&b'/') {
            return false;
        }
        buffer.replace_range(offset..offset + 1, "");
        buffer.set_cursor(offset, false);
        true
    }

    /// 调用前已插入 `/`。选中命令会移除触发符；直接关掉菜单则把 `/` 保留为
    /// 普通文档内容。
    pub(super) fn open_slash_insert_menu(&mut self) -> bool {
        if self.content() != MainContent::Document || !self.editor_engaged {
            return false;
        }
        let Some(buffer) = self.shell.active().and_then(|tab| tab.buffer()) else {
            return false;
        };
        if buffer.has_selection() {
            return false;
        }
        let cursor = buffer.cursor();
        if cursor == 0 || &buffer.text()[cursor - 1..cursor] != "/" {
            return false;
        }
        let Some(path) = self.active_file_path() else {
            return false;
        };
        let anchor = self
            .doc
            .caret_rect(self.editor_area, buffer, self.shell.active_scroll())
            .map(|caret| (caret.left, caret.bottom + 6.0))
            .unwrap_or((self.editor_area.left + 24.0, self.editor_area.top + 8.0));
        self.menu = Some(Menu::open_at(
            Self::slash_insert_items(),
            anchor.0,
            anchor.1,
            self.renderer.viewport(),
        ));
        self.slash_trigger = Some((path, cursor - 1));
        self.toolbar_menu = Some(ToolbarMenu::Insert);
        true
    }

    pub(super) fn open_block_menu(&mut self, start: usize, empty: bool, x: f32, y: f32) {
        if empty {
            if let Some(buffer) = self.shell.active_buffer_mut() {
                buffer.set_cursor(start, false);
            }
            self.menu = Some(Menu::open_at(
                Self::editor_insert_items(),
                x,
                y,
                self.renderer.viewport(),
            ));
            return;
        }
        if self.editor_container(start).is_some() {
            self.open_container_menu(start, x, y);
            return;
        }
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        let parsed = crate::ui::document::parse_ranged(buffer.text());
        let Some(block) = parsed.blocks.iter().find(|b| b.start == start) else {
            return;
        };
        buffer.set_cursor(block.start, false);
        buffer.set_cursor(block.end, true);
        if matches!(
            block.block,
            crate::ui::document::Block::ObjectReference(_)
                | crate::ui::document::Block::AiLocator(_)
        ) {
            let mut items = Self::clipboard_menu_items();
            items.extend([
                MenuItem::new("卡片型", MenuAction::ObjectPresentation(start, false)),
                MenuItem::new("文本型", MenuAction::ObjectPresentation(start, true)),
            ]);
            items.extend(Self::editor_block_items(start));
            self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
            self.focus = Focus::Main;
            self.editor_engaged = true;
            return;
        }
        let items = if matches!(block.block, crate::ui::document::Block::Image { .. }) {
            Self::editor_image_items(start)
        } else {
            let mut items = Self::clipboard_menu_items();
            items.push(
                MenuItem::new("评论选中内容", MenuAction::CommentSelection)
                    .icon(Icon::MESSAGE_SQUARE),
            );
            items.extend(Self::editor_block_items(start));
            items
        };
        self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.after_doc_edit(false);
    }

    pub(super) fn edit_block(&mut self, start: usize, after: bool) {
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        if !apply_block_edit(buffer, start, after) {
            return;
        }
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.after_doc_edit(true);
    }

    pub(super) fn set_object_presentation(&mut self, start: usize, text: bool) {
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        let parsed = crate::ui::document::parse_ranged(buffer.text());
        let Some(block) = parsed.blocks.iter().find(|b| b.start == start) else {
            return;
        };
        let Some(reference) =
            crate::ui::object_link::ObjectLink::parse(&buffer.text()[block.start..block.end])
        else {
            return;
        };
        buffer.replace_range(block.start..block.end, &reference.with_presentation(text));
        self.editor_engaged = false;
        self.after_doc_edit(true);
    }
    pub(super) fn import_editor_image(&mut self, path: &Path) {
        let result = std::fs::read(path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| self.insert_editor_image_bytes(&bytes));
        if let Err(error) = result {
            self.state.status_text = format!("插入图片失败：{error}");
        }
    }

    pub(super) fn insert_editor_image_bytes(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        let note = self
            .active_file_path()
            .ok_or_else(|| anyhow::anyhow!("请先打开笔记"))?;
        anyhow::ensure!(
            self.shell.active().and_then(|t| t.buffer()).is_some(),
            "请先打开可编辑笔记"
        );
        let reference = crate::ui::editor_images::store(&note, bytes)?;
        let buffer = self.shell.active_buffer_mut().unwrap();
        format::insert_block(buffer, &format!("![]({reference})"));
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.after_doc_edit(true);
        Ok(())
    }

    pub(super) fn paste_editor_image_with(
        &mut self,
        read: impl FnOnce() -> anyhow::Result<Option<Vec<u8>>>,
    ) -> bool {
        if !self.editing_file() || self.content() != MainContent::Document {
            return false;
        }
        let result = match read() {
            Ok(Some(bytes)) => self.insert_editor_image_bytes(&bytes),
            Ok(None) => return false,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.state.status_text = format!("粘贴图片失败：{error}");
        }
        true
    }

    pub(super) fn open_image_width(&mut self, start: usize) {
        let Some(path) = self.active_file_path() else {
            return;
        };
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        let parsed = crate::ui::document::parse_ranged(buffer.text());
        let Some(block) = parsed.blocks.iter().find(|b| b.start == start) else {
            return;
        };
        let crate::ui::document::Block::Image { width, .. } = &block.block else {
            return;
        };
        let original = buffer.text()[block.start..block.end].to_owned();
        let mut field = TextField::new("例如：320 或 50%");
        field.set_text(width.as_deref().unwrap_or("100%"));
        self.dialog = Some(Dialog {
            title: "调整图片宽度".into(),
            description: "输入像素宽度或相对正文的百分比".into(),
            field: Some(field),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "保存".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::ImageWidth {
                        path,
                        start,
                        original,
                    },
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn finish_image_width(&mut self, path: &Path, start: usize, original: &str) {
        if self.active_file_path().as_deref() != Some(path) {
            return;
        }
        let width = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().trim().to_owned())
            .unwrap_or_default();
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        if buffer.text().get(start..start + original.len()) != Some(original) {
            return;
        }
        if !crate::ui::editor_images::resize(buffer, start, &width) {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.error = "请输入大于零的像素宽度或百分比".into();
            }
            return;
        }
        self.close_dialog();
        self.editor_engaged = false;
        self.after_doc_edit(false);
    }

    pub(super) fn insert_editor_table(&mut self, rows: usize, cols: usize) {
        if let Some(buffer) = self.shell.active_buffer_mut() {
            table_edit::insert(buffer, rows, cols);
            self.focus = Focus::Main;
            self.editor_engaged = true;
            self.after_doc_edit(true);
        }
    }
    fn editor_container(&self, start: usize) -> Option<Container> {
        let buffer = self.shell.active()?.buffer()?;
        containers::scan(buffer.text())
            .into_iter()
            .find(|p| p.header.start == start)
    }

    pub(super) fn open_container_menu(&mut self, start: usize, x: f32, y: f32) {
        let Some(panel) = self.editor_container(start) else {
            return;
        };
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(panel.header.start, false);
            buffer.set_cursor(panel.footer.end, true);
        }
        self.focus = Focus::Main;
        self.editor_engaged = true;
        let mut items = Self::clipboard_menu_items();
        items.push(
            MenuItem::new("编辑标题", MenuAction::Container(start, Action::Title))
                .icon(Icon::PENCIL_LINE),
        );
        if panel.kind == Kind::Details {
            items.push(
                MenuItem::new(
                    if panel.open { "收起" } else { "展开" },
                    MenuAction::Container(start, Action::Toggle),
                )
                .icon(Icon::CHEVRON_DOWN),
            );
        } else {
            for (color, label, ..) in containers::COLORS {
                let label = if panel.color == *color {
                    format!("{label}   ✓")
                } else {
                    (*label).into()
                };
                items.push(
                    MenuItem::new(
                        label,
                        MenuAction::Container(start, Action::Color((*color).into())),
                    )
                    .icon(Icon::PALETTE),
                );
            }
        }
        items.push(
            MenuItem::new(
                "转换为普通段落",
                MenuAction::Container(start, Action::Unwrap),
            )
            .icon(Icon::PILCROW),
        );
        items.extend(Self::editor_block_items(start));
        self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
    }

    pub(super) fn container_action(&mut self, start: usize, action: Action) {
        let Some(mut panel) = self.editor_container(start) else {
            return;
        };
        if action == Action::Title {
            let Some(path) = self.active_file_path() else {
                return;
            };
            let original = self.shell.active().unwrap().buffer().unwrap().text()
                [panel.header.clone()]
            .to_owned();
            let mut field = TextField::new("标题");
            field.set_text(&panel.title);
            self.dialog = Some(Dialog {
                title: "编辑块标题".into(),
                description: String::new(),
                field: Some(field),
                error: String::new(),
                note: None,
                buttons: vec![
                    DialogButton {
                        label: "取消".into(),
                        kind: ButtonKind::Ghost,
                        action: DialogAction::Dismiss,
                    },
                    DialogButton {
                        label: "保存".into(),
                        kind: ButtonKind::Primary,
                        action: DialogAction::ContainerTitle {
                            path,
                            start,
                            original,
                        },
                    },
                ],
                dismiss: DialogAction::Dismiss,
                hover: None,
            });
            self.focus = Focus::Dialog;
            return;
        }
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        let editing_body = action == Action::Unwrap;
        match action {
            Action::Toggle => {
                panel.open = !panel.open;
                containers::update(buffer, &panel);
            }
            Action::Color(color) => {
                panel.color = color;
                containers::update(buffer, &panel);
            }
            Action::Unwrap => containers::unwrap(buffer, &panel),
            Action::Title => unreachable!(),
        }
        self.focus = Focus::Main;
        self.editor_engaged = editing_body;
        self.after_doc_edit(true);
    }

    pub(super) fn finish_container_title(&mut self, path: &Path, start: usize, original: &str) {
        if self.active_file_path().as_deref() != Some(path) {
            return;
        }
        let Some(mut panel) = self.editor_container(start) else {
            return;
        };
        let title = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().trim().to_owned())
            .unwrap_or_default();
        if title.is_empty() {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.error = "标题不能为空".into();
            }
            return;
        }
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        if buffer.text().get(panel.header.clone()) != Some(original) {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.error = "内容已变化，请重新打开菜单".into();
            }
            return;
        }
        panel.title = title;
        containers::update(buffer, &panel);
        self.close_dialog();
        self.editor_engaged = false;
        self.after_doc_edit(true);
    }

    pub(super) fn insert_editor_math(&mut self) {
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        let selected = buffer.selected_text().to_owned();
        let content = if selected.trim().is_empty() {
            "x"
        } else {
            selected.trim()
        };
        let block = format!("\n$$\n{content}\n$$\n");
        let inner = block.find(content).unwrap_or(4);
        crate::ui::rich::insert(buffer, &block);
        let end = buffer.cursor();
        let start = end.saturating_sub(block.len()).saturating_add(inner);
        buffer.set_cursor(start, false);
        buffer.set_cursor(start + content.len(), true);
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.after_doc_edit(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        document::{self, Block},
        editor::TextBuffer,
    };

    #[derive(Clone, Copy)]
    enum BlockKind {
        Code,
        Container,
        Table,
        Math,
        Divider,
    }

    fn block_for(parsed: &document::Parsed, kind: BlockKind) -> &document::RangedBlock {
        parsed
            .blocks
            .iter()
            .find(|block| {
                matches!(
                    (&block.block, kind),
                    (Block::Code { .. }, BlockKind::Code)
                        | (Block::Container(_), BlockKind::Container)
                        | (Block::Table { .. }, BlockKind::Table)
                        | (Block::Math(_), BlockKind::Math)
                        | (Block::Divider, BlockKind::Divider)
                )
            })
            .expect("fixture block missing")
    }

    #[test]
    fn block_menu_items_share_the_two_requested_actions() {
        let items = App::editor_block_items(37);
        assert!(items
            .iter()
            .any(|item| item.action == MenuAction::BlockParagraphMenu));
        assert!(items
            .iter()
            .any(|item| item.action == MenuAction::BlockFormatMenu));
        assert!(items
            .iter()
            .any(|item| item.action == MenuAction::BlockAiMenu));
        assert!(items
            .iter()
            .any(|item| item.label == "删除功能块" && item.action == MenuAction::Block(37, false)));
        assert!(items.iter().any(|item| {
            item.label == "在功能块下追加一行" && item.action == MenuAction::Block(37, true)
        }));
    }

    #[test]
    fn enter_after_a_fence_creates_a_closed_editable_code_block() {
        let mut buffer = TextBuffer::new("```rust");
        buffer.set_cursor(buffer.text().len(), false);
        assert!(complete_code_fence_on_newline(&mut buffer));
        assert_eq!(buffer.text(), "````rust\n\n````");
        assert_eq!(buffer.cursor(), "````rust\n".len());
        assert!(matches!(
            document::parse(buffer.text()).as_slice(),
            [Block::Code { lang, lines }] if lang == "rust" && lines == &vec![String::new()]
        ));

        let mut closing = TextBuffer::new("````\nbody\n````");
        closing.set_cursor(closing.text().len(), false);
        assert!(!complete_code_fence_on_newline(&mut closing));
    }

    #[test]
    fn chinese_middle_dot_fence_is_equivalent_to_three_backticks() {
        let mut buffer = TextBuffer::new("···python");
        buffer.set_cursor(buffer.text().len(), false);
        assert!(normalize_chinese_code_fence_on_newline(&mut buffer));
        assert!(complete_code_fence_on_newline(&mut buffer));
        assert_eq!(buffer.text(), "````python\n\n````");
        assert!(matches!(
            document::parse(buffer.text()).as_slice(),
            [Block::Code { lang, lines }] if lang == "python" && lines == &vec![String::new()]
        ));

        // 在已经打开的代码块中，它会变成同样的关闭围栏，而不是再嵌套一个块。
        let mut closing = TextBuffer::new("````\nbody\n····");
        closing.set_cursor(closing.text().len(), false);
        assert!(normalize_chinese_code_fence_on_newline(&mut closing));
        assert!(!complete_code_fence_on_newline(&mut closing));
        assert_eq!(closing.text(), "````\nbody\n````");
    }

    #[test]
    fn triple_backticks_inside_an_auto_completed_block_are_literal_code() {
        let mut buffer = TextBuffer::new("```");
        buffer.set_cursor(3, false);
        assert!(complete_code_fence_on_newline(&mut buffer));
        buffer.insert("pasted body\n```\nmore body");

        let parsed = document::parse(buffer.text());
        assert!(matches!(
            parsed.as_slice(),
            [Block::Code { lines, .. }]
                if lines == &vec![
                    "pasted body".to_owned(),
                    "```".to_owned(),
                    "more body".to_owned(),
                ]
        ));
    }

    #[test]
    fn an_existing_four_tick_fence_does_not_autocomplete_inner_triples() {
        let mut buffer = TextBuffer::new("````\nbody\n```\n````");
        buffer.set_cursor("````\nbody\n```".len(), false);
        assert!(!complete_code_fence_on_newline(&mut buffer));
        buffer.insert("\nnext");
        assert!(matches!(
            document::parse(buffer.text()).as_slice(),
            [Block::Code { lines, .. }]
                if lines == &vec![
                    "body".to_owned(),
                    "```".to_owned(),
                    "next".to_owned(),
                ]
        ));
    }

    #[test]
    fn inline_code_waits_for_enter_before_hiding_its_two_markers() {
        assert_eq!(
            crate::ui::text::parse_inline("``"),
            vec![crate::ui::text::Run::plain("``")]
        );
        let mut buffer = TextBuffer::new("``");
        buffer.set_cursor(2, false);
        assert!(complete_inline_code_on_newline(&mut buffer));
        assert_eq!(buffer.text(), "````");
        assert_eq!(buffer.cursor(), 2);
        buffer.insert("value");
        let code = crate::ui::text::parse_inline(buffer.text());
        assert!(
            matches!(code.as_slice(), [run] if run.text == "value" && run.emphasis == crate::ui::text::Emphasis::Code)
        );
    }

    #[test]
    fn slash_menu_exposes_digit_accelerators_and_a_noninteractive_escape_hint() {
        let items = App::slash_insert_items();
        for (index, item) in items.iter().take(items.len() - 1).take(10).enumerate() {
            let expected = if index < 9 {
                (index + 1).to_string()
            } else {
                "0".to_owned()
            };
            assert_eq!(item.shortcut.as_deref(), Some(expected.as_str()));
        }
        let hint = items.last().unwrap();
        assert_eq!(hint.label, "Esc 退出");
        assert!(hint.disabled && hint.hint);
    }

    #[test]
    fn slash_trigger_is_removed_only_by_explicit_activation() {
        let mut buffer = TextBuffer::new("正文/");
        let offset = buffer.text().len() - 1;
        assert!(App::remove_slash_at(&mut buffer, offset));
        assert_eq!(buffer.text(), "正文");
        assert_eq!(buffer.cursor(), offset);

        let mut ordinary = TextBuffer::new("正文");
        assert!(!App::remove_slash_at(&mut ordinary, offset));
        assert_eq!(ordinary.text(), "正文");
    }

    #[test]
    fn append_block_row_preserves_unicode_following_blocks_and_one_undo() {
        let fixtures=[
            ("前文\n<!-- mochi-code-block title=\"代码\" collapsed=\"true\" -->\n```rust\n内容😀\n```\n后文",BlockKind::Code),
            ("前文\r\n:::mochi-highlight color=\"blue\" title=\"提示\"\r\n内容😀\r\n:::\r\n后文",BlockKind::Container),
            ("前文\n| 表头 | 状态 |\n| --- | --- |\n| 😀 | 好 |\n后文",BlockKind::Table),
            ("前文\r\n$$\r\n公式😀\r\n$$\r\n后文",BlockKind::Math),
            ("前文\n---\n后文",BlockKind::Divider),
        ];
        for (source, kind) in fixtures {
            let parsed = document::parse_ranged(source);
            let block = block_for(&parsed, kind);
            let end = block_end(block);
            let newline = if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let expected = format!("{}{}{}", &source[..end], newline, &source[end..]);
            let mut buffer = TextBuffer::new(source);
            assert!(apply_block_edit(&mut buffer, block.start, true));
            assert_eq!(buffer.text(), expected, "append changed fixture");
            assert_eq!(
                buffer.selection(),
                (end + newline.len(), end + newline.len())
            );
            assert!(buffer.undo());
            assert_eq!(buffer.text(), source, "append did not undo in one step");
        }
    }

    #[test]
    fn delete_block_removes_code_metadata_and_container_footer_without_swallowing_tail() {
        let fixtures=[
            ("前文\r\n<!-- mochi-code-block title=\"代码\" collapsed=\"true\" -->\r\n```rust\r\n内容😀\r\n```\r\n后文",BlockKind::Code),
            ("前文\n:::mochi-highlight color=\"blue\" title=\"提示\"\n内容😀\n:::\n后文",BlockKind::Container),
            ("前文\r\n| 表头 | 状态 |\r\n| --- | --- |\r\n| 😀 | 好 |\r\n后文",BlockKind::Table),
            ("前文\n$$\n公式😀\n$$\n后文",BlockKind::Math),
            ("前文\r\n---\r\n后文",BlockKind::Divider),
        ];
        for (source, kind) in fixtures {
            let parsed = document::parse_ranged(source);
            let block = block_for(&parsed, kind);
            let start = if matches!(block.block, Block::Code { .. }) {
                crate::ui::code_blocks::metadata(source, block.start)
                    .0
                    .start
            } else {
                block.start
            };
            let end = block_end(block);
            let delete_end = end + line_break_len(source, end);
            let expected = format!("{}{}", &source[..start], &source[delete_end..]);
            let mut buffer = TextBuffer::new(source);
            assert!(apply_block_edit(&mut buffer, block.start, false));
            assert_eq!(buffer.text(), expected, "delete changed fixture");
            assert!(buffer.undo());
            assert_eq!(buffer.text(), source, "delete did not undo in one step");
        }
    }

    #[test]
    fn append_at_eof_uses_document_newline_and_keeps_the_empty_row_editable() {
        for source in ["末行😀", "末行😀\n", "末行😀\r\n"] {
            let parsed = document::parse_ranged(source);
            let block = parsed
                .blocks
                .iter()
                .find(|block| matches!(block.block, Block::Paragraph(_)))
                .unwrap();
            let end = block_end(block);
            let newline = if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let expected = format!("{}{}{}", &source[..end], newline, &source[end..]);
            let mut buffer = TextBuffer::new(source);
            assert!(apply_block_edit(&mut buffer, block.start, true));
            assert_eq!(buffer.text(), expected);
            assert_eq!(buffer.cursor(), end + newline.len());
            assert!(buffer.undo());
            assert_eq!(buffer.text(), source);
        }
    }

    #[test]
    fn append_from_a_container_child_lands_below_the_complete_functional_block() {
        for source in [
            "前文\n:::mochi-highlight title=\"提示\"\n容器内正文\n:::\n后文",
            "前文\r\n:::mochi-highlight title=\"提示\"\r\n容器内正文\r\n:::\r\n后文",
        ] {
            let parsed = document::parse_ranged(source);
            let child = parsed
                .blocks
                .iter()
                .find(
                    |block| matches!(&block.block, Block::Paragraph(text) if text == "容器内正文"),
                )
                .expect("container child missing");
            let panel = parsed
                .blocks
                .iter()
                .find_map(|block| match &block.block {
                    Block::Container(panel) => Some(panel.clone()),
                    _ => None,
                })
                .expect("container missing");
            let newline = if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let expected = format!(
                "{}{}{}",
                &source[..panel.footer.end],
                newline,
                &source[panel.footer.end..]
            );
            let mut buffer = TextBuffer::new(source);
            assert!(apply_block_edit(&mut buffer, child.start, true));
            assert_eq!(buffer.text(), expected);
            assert_eq!(buffer.cursor(), panel.footer.end + newline.len());

            let next = containers::scan(buffer.text()).remove(0);
            assert!(
                buffer.cursor() > next.footer.end,
                "the appended row was still mapped inside the container"
            );
            assert!(buffer.undo());
            assert_eq!(buffer.text(), source);
        }
    }
}
