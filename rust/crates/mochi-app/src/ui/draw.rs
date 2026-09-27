//! 面板只提交 DrawList，不直接访问 D2D。

use super::icons::Icon;
use super::layout::Rect;
use super::text::Emphasis;

/// 文字的字重/字号档位。具体映射到哪个 `IDWriteTextFormat` 由回放层决定——
/// 指令层不该知道 DirectWrite 的存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextStyle {
    /// AI CSS 字体排版：正文、h1-h4、代码和表格；独立于笔记设置。
    Ai {
        kind: u8,
        standalone: bool,
    },
    Small,
    /// 标题栏、面板标题。
    Title,
    /// 正文、列表行。
    Body,
    /// 状态栏、次要说明。
    Caption,
    /// 链接上下文的 `text-[11px]` 文本样式。
    Tiny,
    /// Tailwind 的 `text-sm`（14px）：文件树行、对话框正文、菜单项。
    Label,
    /// Tailwind 的 `text-lg font-semibold`（18px）：对话框标题。
    Large,
    /// 全局搜索输入框的 `text-[15px]`。
    Search,
    /// 番茄钟的 `font-mono text-6xl`（60px 等宽）。
    Clock,
    /// Tailwind 的 `text-base`（16px / 24）：日程控制台标题这类小标题。
    Body16,
    /// 页面大标题 `text-[28px] font-semibold`（收件箱 / 最近）。
    Display,
    /// 文档里的一到三级标题。
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    Document,
    DocumentTitle,
    DocumentMono,
    Table,
    /// 代码块与行内代码。等宽字族由回放层挑。
    Mono,
}

impl TextStyle {
    /// 全部档位。回放层要为每一档预建格式对象。
    pub const ALL: [TextStyle; 36] = [
        TextStyle::Ai {
            kind: 0,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 1,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 2,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 3,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 4,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 5,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 6,
            standalone: false,
        },
        TextStyle::Ai {
            kind: 0,
            standalone: true,
        },
        TextStyle::Ai {
            kind: 1,
            standalone: true,
        },
        TextStyle::Ai {
            kind: 2,
            standalone: true,
        },
        TextStyle::Ai {
            kind: 3,
            standalone: true,
        },
        TextStyle::Ai {
            kind: 4,
            standalone: true,
        },
        TextStyle::Ai {
            kind: 5,
            standalone: true,
        },
        TextStyle::Ai {
            kind: 6,
            standalone: true,
        },
        TextStyle::Small,
        TextStyle::Title,
        TextStyle::Body,
        TextStyle::Caption,
        TextStyle::Tiny,
        TextStyle::Label,
        TextStyle::Large,
        TextStyle::Search,
        TextStyle::Clock,
        TextStyle::Body16,
        TextStyle::Display,
        TextStyle::Heading1,
        TextStyle::Heading2,
        TextStyle::Heading3,
        TextStyle::Heading4,
        TextStyle::Heading5,
        TextStyle::Heading6,
        TextStyle::Document,
        TextStyle::DocumentMono,
        TextStyle::Table,
        TextStyle::DocumentTitle,
        TextStyle::Mono,
    ];

    /// 字号。与 `src/index.css` 的 `.ProseMirror` 规则对齐。
    pub fn font_size(self) -> f32 {
        match self {
            TextStyle::Ai { kind, standalone } => {
                (if standalone {
                    super::settings_values::number("assistant.standaloneFontSize", 15.0)
                } else {
                    super::settings_values::number("assistant.messageFontSize", 14.0)
                }) * match kind {
                    1 => 1.35,
                    2 => 1.2,
                    3 => 1.08,
                    5 => 0.9,
                    6 => 0.95,
                    _ => 1.0,
                }
            }
            TextStyle::Small => super::settings_values::number("appearance.smallFontSize", 12.0),
            TextStyle::Title => super::settings_values::number(
                "appearance.titleFontSize",
                super::theme::TITLE_FONT_SIZE,
            ),
            TextStyle::Body => super::settings_values::number(
                "appearance.uiFontSize",
                super::theme::BODY_FONT_SIZE,
            ),
            TextStyle::Caption => super::settings_values::number(
                "appearance.captionFontSize",
                super::theme::CAPTION_FONT_SIZE,
            ),
            TextStyle::Tiny => super::settings_values::number("appearance.tinyFontSize", 11.0),
            TextStyle::Label => super::settings_values::number("appearance.labelFontSize", 14.0),
            TextStyle::Large => {
                super::settings_values::number("appearance.dialogTitleFontSize", 18.0)
            }
            TextStyle::Search => super::settings_values::number("appearance.searchFontSize", 15.0),
            TextStyle::Clock => 60.0,
            TextStyle::Body16 => {
                super::settings_values::number("appearance.sectionTitleFontSize", 16.0)
            }
            TextStyle::Display => {
                super::settings_values::number("appearance.pageTitleFontSize", 28.0)
            }
            TextStyle::Heading1 => super::editor_preferences::current().heading_size[0],
            TextStyle::Heading2 => super::editor_preferences::current().heading_size[1],
            TextStyle::Heading3 => super::editor_preferences::current().heading_size[2],
            TextStyle::Heading4 => super::editor_preferences::current().heading_size[3],
            TextStyle::Heading5 => super::editor_preferences::current().heading_size[4],
            TextStyle::Heading6 => super::editor_preferences::current().heading_size[5],
            TextStyle::Document => super::editor_preferences::current().font_size,
            TextStyle::DocumentTitle => {
                super::settings_values::number("editorLayout.titleFontSize", 36.0)
            }
            TextStyle::DocumentMono => super::editor_preferences::current().code_size,
            TextStyle::Table => super::editor_preferences::current().table_size,
            // editor.css `.ProseMirror pre`：13px / 1.7
            TextStyle::Mono => super::settings_values::number("appearance.monoFontSize", 13.0),
        }
    }

    /// 行高。正文 1.75 倍是从 Electron 版的编辑器样式量出来的；
    /// Tailwind 的 text-sm / text-lg 自带 20px / 28px 行高。
    pub fn line_height(self) -> f32 {
        match self {
            TextStyle::Ai { kind: 1..=4, .. } => self.font_size() * 1.25,
            TextStyle::Ai { standalone, .. } => {
                self.font_size()
                    * if standalone {
                        super::settings_values::number("assistant.standaloneLineHeight", 1.75)
                    } else {
                        super::settings_values::number("assistant.lineHeight", 1.6)
                    }
            }
            TextStyle::Heading1 => self.font_size() * 1.3,
            TextStyle::Heading2 => self.font_size() * 1.4,
            TextStyle::Heading3 | TextStyle::Heading4 => self.font_size() * 1.5,
            TextStyle::Heading5 | TextStyle::Heading6 => self.font_size() * 1.6,
            TextStyle::Label => self.font_size() * (20.0 / 14.0),
            TextStyle::Large => self.font_size() * (28.0 / 18.0),
            TextStyle::Search => self.font_size() * (24.0 / 15.0),
            TextStyle::Clock => 60.0,
            TextStyle::Body16 => self.font_size() * 1.5,
            TextStyle::Display => self.font_size() * (36.0 / 28.0),
            TextStyle::Mono => (self.font_size() * 1.7).round(),
            TextStyle::Document => {
                (self.font_size() * super::editor_preferences::current().line_height).round()
            }
            TextStyle::DocumentTitle => self.font_size() * 1.25,
            TextStyle::DocumentMono => {
                (self.font_size() * super::editor_preferences::current().code_height).round()
            }
            TextStyle::Table => (self.font_size() * (20.0 / 14.0)).round(),
            _ => (self.font_size() * 1.75).round(),
        }
    }

    /// 一个字符大致占几个 em。
    ///
    /// 仅为无图形宿主/超限/成形失败时的估算回退。正常窗口的测量、断行与光标
    /// 使用 measurement 提供的 DirectWrite 字簇；绘制仍按这些视觉行关闭自动换行。
    pub fn advance_em(self, ch: char) -> f32 {
        if matches!(ch as u32,0x200c..=0x200d|0xfe00..=0xfe0f|0x0300..=0x036f|0x1f3fb..=0x1f3ff) {
            return 0.0;
        }
        if is_wide(ch) {
            1.0
        } else if matches!(
            self,
            TextStyle::Mono | TextStyle::DocumentMono | TextStyle::Clock
        ) {
            0.6
        } else {
            0.52
        }
    }
}

/// 全角字符（CJK、假名、全角标点）。宽度按一个 em 算。
pub fn is_wide(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19 | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F000..=0x1FAFF | 0x20000..=0x2FFFD | 0x30000..=0x3FFFD)
}

/// 文字在矩形内的水平对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Leading,
    Center,
    Trailing,
}

/// 一条绘制指令。
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    ScaledText {
        rect: Rect,
        text: String,
        style: TextStyle,
        color: u32,
        align: Align,
        scale: f32,
    },
    Polyline {
        points: Vec<(f32, f32)>,
        color: u32,
        width: f32,
    },
    BeginOpacity {
        rect: Rect,
        opacity: f32,
    },
    EndOpacity,
    /// 语义光标：几何与 IME 共享；可见性从不影响布局。
    Caret {
        rect: Rect,
        color: u32,
        visible: bool,
    },
    /// 实心矩形。
    Rect {
        rect: Rect,
        color: u32,
    },
    /// 半透明矩形（遮罩层 `bg-black/40`）。`alpha` 0..1。
    /// 单独一种指令而不是给颜色加 alpha 通道：不透明矩形要关反锯齿、走画刷缓存，
    /// 半透明的很少见，没必要让每条 `Rect` 都背着一个 alpha。
    RectAlpha {
        rect: Rect,
        color: u32,
        alpha: f32,
    },
    /// 一段完整的原生公式：源码原样保留，位图感知 DPI，
    /// 且只由渲染后端缓存。绝不调用浏览器。
    Math {
        rect: Rect,
        tex: String,
        font_size: f32,
        color: u32,
        wrap: Option<f32>,
    },
    /// 文字。`rect` 是排版框，超出部分由当前裁剪区负责切掉。
    /// 永远是**单个视觉行**——断行在 `ui::text` 里做完了，见 `TextStyle::advance_em`。
    Text {
        rect: Rect,
        text: String,
        style: TextStyle,
        color: u32,
        align: Align,
        /// 行内粗/斜/码。回放层据此挑不同的 `IDWriteTextFormat`。
        emphasis: Emphasis,
    },
    /// lucide 图标，描边绘制。`rect` 是图标盒，24 单位视图盒等比缩放到盒内居中。
    Icon {
        rect: Rect,
        icon: Icon,
        color: u32,
    },
    /// 圆角矩形（实心）。对应 Tailwind 的 `rounded-md` 之类。
    RoundedRect {
        rect: Rect,
        radius: f32,
        color: u32,
    },
    /// 主按钮共用的半透明材质，带一圈水晶质感的边缘。
    GlassButton {
        rect: Rect,
        radius: f32,
        dark: bool,
        hovered: bool,
    },
    RoundedRectAlpha {
        rect: Rect,
        radius: f32,
        color: u32,
        alpha: f32,
    },
    /// 圆角矩形描边（1px）。对应 `border` + `rounded`。
    RoundedBorder {
        rect: Rect,
        radius: f32,
        color: u32,
    },
    ShapeBorder {
        rect: Rect,
        ellipse: bool,
        width: f32,
        color: u32,
    },
    /// 品牌标志（`Logo.tsx` 那只章鱼）。带渐变与填充，指令层不拆开，回放层整体画。
    Logo {
        rect: Rect,
    },
    /// 位图：`src` 是**已解析**的本地路径或 URL，回放层按它解码并缓存；
    /// 解不出来就画占位框 + `alt`。`fill` 为假时图片在 `rect` 里按宽度等比缩放、顶部对齐（文档），
    /// 为真时填满 `rect`（查看器已算好尺寸）；`rotation` 是绕矩形中心顺时针旋转的角度（0/90/180/270）。
    Image {
        rect: Rect,
        src: String,
        alt: String,
        rotation: u16,
        fill: bool,
        thumbnail: bool,
    },
    /// 压入裁剪区。回放层对应 `PushAxisAlignedClip`。
    PushClip {
        rect: Rect,
    },
    /// 弹出裁剪区。
    PopClip,
}

/// 一帧的绘制指令。
#[derive(Debug, Default)]
pub struct DrawList {
    cmds: Vec<DrawCmd>,
    /// 裁剪栈。只存矩形，用于 `clip_rect()` 与 [`Self::finish`] 的配对检查。
    clips: Vec<Rect>,
}

impl DrawList {
    pub fn scale_text_since(&mut self, start: usize, scale: f32) {
        for command in self.cmds.iter_mut().skip(start) {
            if let DrawCmd::Text {
                rect,
                text,
                style,
                color,
                align,
                ..
            } = command
            {
                *command = DrawCmd::ScaledText {
                    rect: *rect,
                    text: text.clone(),
                    style: *style,
                    color: *color,
                    align: *align,
                    scale,
                };
            }
        }
    }
    /// 把整个浮层作为一个合成表面淡出，文字包括在内。
    /// 两者成对出现，调用方才不会在提前返回时留下一个没闭合的 D2D 层。
    pub fn fade_since(&mut self, start: usize, rect: Rect, opacity: f32) {
        if start >= self.cmds.len() || opacity >= 1.0 {
            return;
        }
        self.cmds.insert(
            start,
            DrawCmd::BeginOpacity {
                rect,
                opacity: opacity.clamp(0.0, 1.0),
            },
        );
        self.cmds.push(DrawCmd::EndOpacity);
    }
    /// 模态表面即使不含文字输入框，也要接收输入。
    pub fn clear_carets(&mut self) {
        self.cmds
            .retain(|cmd| !matches!(cmd, DrawCmd::Caret { .. }));
    }

    pub fn caret(&mut self, rect: Rect, color: u32) {
        let rect = self.clip_rect().map_or(rect, |clip| rect.intersect(&clip));
        if !rect.is_empty() {
            self.cmds.push(DrawCmd::Caret {
                rect,
                color,
                visible: true,
            });
        }
    }

    /// 最后获得焦点的控件属于最上层绘制的输入表面。
    /// 即便在闪烁周期的隐藏半程，它的几何也要保留。
    pub fn caret_rect(&self) -> Option<Rect> {
        self.cmds.iter().rev().find_map(|cmd| match cmd {
            DrawCmd::Caret { rect, .. } => Some(*rect),
            _ => None,
        })
    }

    pub fn set_caret_visible(&mut self, show: bool) -> bool {
        let mut primary = true;
        let mut changed = false;
        for cmd in self.cmds.iter_mut().rev() {
            if let DrawCmd::Caret { visible, .. } = cmd {
                let next = primary && show;
                changed |= *visible != next;
                *visible = next;
                primary = false;
            }
        }
        changed
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn cmds(&self) -> &[DrawCmd] {
        &self.cmds
    }
    /// 资源准备必须遵循与实际绘制相同的嵌套裁剪。
    /// 索引保持绘制顺序；clip 指令本身绝不被移除。
    pub fn visible_resources(&self, viewport: Rect) -> Vec<usize> {
        let mut clips = vec![viewport];
        let mut visible = Vec::new();
        for (index, cmd) in self.cmds.iter().enumerate() {
            match cmd {
                DrawCmd::PushClip { rect } => clips.push(rect.intersect(clips.last().unwrap())),
                DrawCmd::PopClip => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                DrawCmd::Math { rect, .. } => {
                    if !rect.intersect(clips.last().unwrap()).is_empty() {
                        visible.push(index);
                    }
                }
                DrawCmd::Image { rect, rotation, .. } => {
                    let bounds = if *rotation == 0 {
                        *rect
                    } else {
                        let (s, c) = f32::from(*rotation).to_radians().sin_cos();
                        let w = rect.width() * c.abs() + rect.height() * s.abs();
                        let h = rect.width() * s.abs() + rect.height() * c.abs();
                        let cx = (rect.left + rect.right) / 2.0;
                        let cy = (rect.top + rect.bottom) / 2.0;
                        Rect::from_size(cx - w / 2.0, cy - h / 2.0, w, h)
                    };
                    if !bounds.intersect(clips.last().unwrap()).is_empty() {
                        visible.push(index);
                    }
                }
                _ => {}
            }
        }
        visible
    }
    pub fn rect_visible(&self, rect: Rect) -> bool {
        !rect.is_empty()
            && self
                .clip_rect()
                .is_none_or(|clip| !rect.intersect(&clip).is_empty())
    }
    pub(super) fn commands_mut(&mut self) -> &mut Vec<DrawCmd> {
        &mut self.cmds
    }
    pub fn rounded_rect_alpha(&mut self, rect: Rect, radius: f32, color: u32, alpha: f32) {
        if !rect.is_empty() {
            self.cmds.push(DrawCmd::RoundedRectAlpha {
                rect,
                radius,
                color,
                alpha: alpha.clamp(0.0, 1.0),
            });
        }
    }

    pub fn glass_button(
        &mut self,
        rect: Rect,
        radius: f32,
        p: &super::theme::Palette,
        hovered: bool,
    ) {
        if !rect.is_empty() {
            self.cmds.push(DrawCmd::GlassButton {
                rect,
                radius,
                dark: super::theme::is_dark(p),
                hovered,
            });
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn clear(&mut self) {
        self.cmds.clear();
        self.clips.clear();
    }

    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    /// 丢掉 `len` 之后画的命令（重画同一区域前用）。调用方负责裁剪栈已配平。
    pub fn truncate(&mut self, len: usize) {
        self.cmds.truncate(len);
    }

    /// 当前有效裁剪区；没压过就是 `None`（不裁剪）。
    /// 面板内部要按它决定「这一行画不画得下」，滚动列表搬过来时接上。
    #[allow(dead_code)]
    pub fn clip_rect(&self) -> Option<Rect> {
        self.clips.last().copied()
    }

    /// 空矩形直接丢弃，不产生指令。
    ///
    /// 这不只是省事：面板被折叠或窗口拖到极窄时会算出零宽矩形，
    /// D2D 画零宽矩形是合法的空操作，但让它进列表会把断言弄脏——
    /// 测试里数出来的指令条数会随窗口尺寸变化。
    pub fn rect(&mut self, rect: Rect, color: u32) {
        if rect.is_empty() {
            return;
        }
        self.cmds.push(DrawCmd::Rect { rect, color });
    }

    pub fn shape_border(&mut self, rect: Rect, ellipse: bool, width: f32, color: u32) {
        if !rect.is_empty() {
            self.cmds.push(DrawCmd::ShapeBorder {
                rect,
                ellipse,
                width,
                color,
            });
        }
    }

    pub fn rect_alpha(&mut self, rect: Rect, color: u32, alpha: f32) {
        if rect.is_empty() || alpha <= 0.0 {
            return;
        }
        self.cmds.push(DrawCmd::RectAlpha {
            rect,
            color,
            alpha: alpha.min(1.0),
        });
    }

    /// 在可透视窗口上，不透明的悬停色或标签填充看起来像一块实心板；
    /// 把 `from` 的填充换成 `color` 的半透明叠加。
    pub fn translucent_fills(&mut self, from: u32, color: u32, alpha: f32) {
        for cmd in &mut self.cmds {
            *cmd = match std::mem::replace(cmd, DrawCmd::EndOpacity) {
                DrawCmd::Rect { rect, color: c } if c == from => {
                    DrawCmd::RectAlpha { rect, color, alpha }
                }
                DrawCmd::RoundedRect {
                    rect,
                    radius,
                    color: c,
                } if c == from => DrawCmd::RoundedRectAlpha {
                    rect,
                    radius,
                    color,
                    alpha,
                },
                other => other,
            };
        }
    }

    /// 磨砂暗色调纹理：自顶部向下渐淡的微弱白色高光，
    /// 以及朝底部略深的阴影。两者都是中性色，色调在壁纸上
    /// 呈现为阴影，而不是一块彩色面板。
    pub fn glass_sheen(&mut self, rect: Rect) {
        const BANDS: usize = 24;
        let glow = rect.height() * 0.4 / BANDS as f32;
        let shade = rect.height() * 0.4 / BANDS as f32;
        for i in 0..BANDS {
            let t = 1.0 - i as f32 / BANDS as f32;
            let top = rect.top + i as f32 * glow;
            self.rect_alpha(
                Rect::new(rect.left, top, rect.right, top + glow),
                0xffffff,
                0.07 * t * t,
            );
            let bottom = rect.bottom - i as f32 * shade;
            self.rect_alpha(
                Rect::new(rect.left, bottom - shade, rect.right, bottom),
                0x000000,
                0.10 * t * t,
            );
        }
        self.rect_alpha(
            Rect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
            0xffffff,
            0.18,
        );
    }

    /// 1px 分隔线（水平）。
    ///
    /// 用 `FillRectangle` 而不是 `DrawLine`：后者在非整数 DPI 缩放下会把
    /// 1 像素的线糊成两像素的灰边。
    pub fn hline(&mut self, left: f32, right: f32, y: f32, color: u32) {
        self.rect(Rect::new(left, y, right, y + 1.0), color);
    }
    pub fn polyline(&mut self, points: Vec<(f32, f32)>, color: u32, width: f32) {
        self.cmds.push(DrawCmd::Polyline {
            points,
            color,
            width,
        });
    }

    /// 1px 分隔线（垂直）。
    pub fn vline(&mut self, x: f32, top: f32, bottom: f32, color: u32) {
        self.rect(Rect::new(x, top, x + 1.0, bottom), color);
    }

    /// 沿矩形的某条边画 1px 描边。对应 CSS 的 `border-b` / `border-r` 之类。
    pub fn border_bottom(&mut self, rect: Rect, color: u32) {
        self.hline(rect.left, rect.right, rect.bottom - 1.0, color);
    }

    pub fn border_right(&mut self, rect: Rect, color: u32) {
        self.vline(rect.right - 1.0, rect.top, rect.bottom, color);
    }

    pub fn border_left(&mut self, rect: Rect, color: u32) {
        self.vline(rect.left, rect.top, rect.bottom, color);
    }

    /// 图标。`size` 是图标盒边长（lucide 的 `w-5 h-5` 就是 20），在 `rect` 内居中。
    pub fn icon_centered(&mut self, rect: Rect, icon: Icon, size: f32, color: u32) {
        if rect.is_empty() {
            return;
        }
        let cx = (rect.left + rect.right) / 2.0;
        let cy = (rect.top + rect.bottom) / 2.0;
        let half = size / 2.0;
        self.icon(
            Rect::new(cx - half, cy - half, cx + half, cy + half),
            icon,
            color,
        );
    }

    /// 图标，铺满给定盒子（盒子应当是正方形；不是的话取短边居中）。
    pub fn icon(&mut self, rect: Rect, icon: Icon, color: u32) {
        if rect.is_empty() {
            return;
        }
        self.cmds.push(DrawCmd::Icon { rect, icon, color });
    }

    /// 画一个八帧的小型加载动画，不依赖浏览器动画层。
    /// 帧序号属于绘制列表的一部分，原生定时器可以确定性地推进，
    /// 布局测试也能验证相邻两帧确实不同。
    pub fn spinner(&mut self, rect: Rect, phase: u8, color: u32) {
        if rect.is_empty() {
            return;
        }
        let cx = (rect.left + rect.right) / 2.0;
        let cy = (rect.top + rect.bottom) / 2.0;
        let radius = (rect.width().min(rect.height()) * 0.34).max(1.0);
        let dot = (rect.width().min(rect.height()) * 0.16).clamp(1.5, 3.0);
        let phase = phase as usize % 8;
        for i in 0..8 {
            let angle = std::f32::consts::TAU * i as f32 / 8.0;
            let x = cx + angle.cos() * radius - dot / 2.0;
            let y = cy + angle.sin() * radius - dot / 2.0;
            let distance = (i + 8 - phase) % 8;
            let alpha = 0.20 + (7 - distance) as f32 / 7.0 * 0.80;
            self.rounded_rect_alpha(Rect::from_size(x, y, dot, dot), dot / 2.0, color, alpha);
        }
    }

    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: u32) {
        if rect.is_empty() {
            return;
        }
        if radius <= 0.0 {
            self.cmds.push(DrawCmd::Rect { rect, color });
        } else {
            self.cmds.push(DrawCmd::RoundedRect {
                rect,
                radius,
                color,
            });
        }
    }

    pub fn rounded_border(&mut self, rect: Rect, radius: f32, color: u32) {
        if rect.is_empty() {
            return;
        }
        self.cmds.push(DrawCmd::RoundedBorder {
            rect,
            radius,
            color,
        });
    }

    pub fn image(&mut self, rect: Rect, src: impl Into<String>, alt: impl Into<String>) {
        if rect.is_empty() {
            return;
        }
        self.cmds.push(DrawCmd::Image {
            rect,
            src: src.into(),
            alt: alt.into(),
            rotation: 0,
            fill: false,
            thumbnail: false,
        });
    }

    /// 查看器用：填满 `rect` 并旋转。
    pub fn image_rotated(
        &mut self,
        rect: Rect,
        src: impl Into<String>,
        alt: impl Into<String>,
        rotation: u16,
    ) {
        if rect.is_empty() {
            return;
        }
        self.cmds.push(DrawCmd::Image {
            rect,
            src: src.into(),
            alt: alt.into(),
            rotation: rotation % 360,
            fill: true,
            thumbnail: false,
        });
    }
    pub fn image_thumbnail(&mut self, rect: Rect, src: impl Into<String>, alt: impl Into<String>) {
        if !rect.is_empty() {
            self.cmds.push(DrawCmd::Image {
                rect,
                src: src.into(),
                alt: alt.into(),
                rotation: 0,
                fill: true,
                thumbnail: true,
            });
        }
    }

    pub fn logo(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        self.cmds.push(DrawCmd::Logo { rect });
    }

    /// 空文本不产生指令——同 `rect()` 的理由。
    pub fn text(&mut self, rect: Rect, text: impl Into<String>, style: TextStyle, color: u32) {
        self.text_aligned(rect, text, style, color, Align::Leading);
    }

    pub fn math(&mut self, rect: Rect, tex: &str, font_size: f32, color: u32) {
        if !rect.is_empty() && !tex.is_empty() {
            self.cmds.push(DrawCmd::Math {
                rect,
                tex: tex.into(),
                font_size,
                color,
                wrap: None,
            });
        }
    }
    pub fn math_wrapped(&mut self, rect: Rect, tex: &str, font_size: f32, color: u32, width: f32) {
        if !rect.is_empty() && !tex.is_empty() {
            self.cmds.push(DrawCmd::Math {
                rect,
                tex: tex.into(),
                font_size,
                color,
                wrap: Some(width),
            });
        }
    }

    pub fn text_aligned(
        &mut self,
        rect: Rect,
        text: impl Into<String>,
        style: TextStyle,
        color: u32,
        align: Align,
    ) {
        self.text_run(rect, text, style, color, align, Emphasis::None);
    }

    pub fn text_run(
        &mut self,
        rect: Rect,
        text: impl Into<String>,
        style: TextStyle,
        color: u32,
        align: Align,
        emphasis: Emphasis,
    ) {
        let mut text = text.into();
        if text.is_empty() || rect.is_empty() {
            return;
        }
        if emphasis.base() == Emphasis::Math {
            let font_size = style.font_size() * super::text::math_scale(style);
            if let Some((width, height)) = super::math_layout::size(&text, font_size) {
                let top = rect.top + (rect.height() - height) / 2.0;
                let foreground = match emphasis {
                    Emphasis::Styled { fg: Some(fg), .. } => fg,
                    _ => color,
                };
                if let Emphasis::Styled { bg: Some(bg), .. } = emphasis {
                    self.rect(rect, bg);
                }
                self.math(
                    Rect::new(rect.left, top, rect.left + width, top + height),
                    &text,
                    font_size,
                    foreground,
                );
                if let Emphasis::Styled { flags, .. } = emphasis {
                    if flags & 4 != 0 {
                        self.hline(rect.left, rect.right, rect.bottom - 4.0, foreground);
                    }
                    if flags & 8 != 0 {
                        self.hline(
                            rect.left,
                            rect.right,
                            (rect.top + rect.bottom) / 2.0,
                            foreground,
                        );
                    }
                }
                return;
            }
            text = super::mathtext::render(&text);
        }
        self.cmds.push(DrawCmd::Text {
            rect,
            text,
            style,
            color,
            align,
            emphasis,
        });
    }

    /// 压入裁剪区。与父裁剪区求交——嵌套面板的裁剪要叠加，
    /// 否则内层面板一压就把外层的限制放开了。
    pub fn push_clip(&mut self, rect: Rect) {
        let effective = match self.clips.last() {
            Some(parent) => rect.intersect(parent),
            None => rect,
        };
        self.clips.push(effective);
        self.cmds.push(DrawCmd::PushClip { rect: effective });
    }

    pub fn pop_clip(&mut self) {
        if self.clips.pop().is_some() {
            self.cmds.push(DrawCmd::PopClip);
        }
    }

    /// 收尾检查：裁剪栈必须是平的。
    ///
    /// 压了不弹在 D2D 那边不会报错，只会让后续所有绘制被莫名其妙地切掉一块——
    /// 这类问题很难从画面判断究竟漏了哪个 `pop`；此处直接改为断言。
    pub fn finish(&self) -> Result<(), UnbalancedClip> {
        if self.clips.is_empty() {
            Ok(())
        } else {
            Err(UnbalancedClip {
                depth: self.clips.len(),
            })
        }
    }
}

/// 裁剪栈没配平。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnbalancedClip {
    pub depth: usize,
}

impl std::fmt::Display for UnbalancedClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "裁剪栈没配平：还剩 {} 层没弹出", self.depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_scaled_text_preserves_grid_alignment() {
        let mut list = DrawList::new();
        list.text_aligned(
            Rect::from_size(0.0, 0.0, 160.0, 40.0),
            "工作台",
            TextStyle::Body16,
            0,
            Align::Center,
        );
        list.scale_text_since(0, 1.5);
        assert!(matches!(
            list.cmds()[0],
            DrawCmd::ScaledText {
                align: Align::Center,
                ..
            }
        ));
    }

    #[test]
    fn only_topmost_caret_blinks_and_modal_without_input_clears_it() {
        let mut list = DrawList::new();
        list.caret(Rect::from_size(1.0, 1.0, 1.0, 20.0), 0);
        let clip = Rect::from_size(20.0, 20.0, 100.0, 20.0);
        list.push_clip(clip);
        list.caret(Rect::from_size(30.0, 10.0, 1.0, 40.0), 0);
        list.pop_clip();
        let caret = Rect::from_size(30.0, 20.0, 1.0, 20.0);
        assert_eq!(list.caret_rect(), Some(caret));
        list.set_caret_visible(true);
        assert_eq!(
            list.cmds()
                .iter()
                .filter(|c| matches!(c, DrawCmd::Caret { visible: true, .. }))
                .count(),
            1
        );
        list.set_caret_visible(false);
        assert_eq!(list.caret_rect(), Some(caret));
        assert!(list
            .cmds()
            .iter()
            .all(|c| !matches!(c, DrawCmd::Caret { visible: true, .. })));
        list.clear_carets();
        assert_eq!(list.caret_rect(), None);
        assert!(list.finish().is_ok());
    }

    #[test]
    fn a_zero_sized_rect_produces_no_command() {
        let mut list = DrawList::new();
        list.rect(Rect::new(10.0, 10.0, 10.0, 50.0), 0xFF0000);
        list.rect(Rect::new(10.0, 10.0, 50.0, 10.0), 0xFF0000);
        assert!(list.is_empty());
    }

    #[test]
    fn empty_text_produces_no_command() {
        let mut list = DrawList::new();
        list.text(
            Rect::new(0.0, 0.0, 100.0, 20.0),
            "",
            TextStyle::Body,
            0x000000,
        );
        assert!(list.is_empty());
    }

    #[test]
    fn spinner_frames_change_the_drawn_alpha_sequence() {
        let rect = Rect::from_size(0.0, 0.0, 20.0, 20.0);
        let mut first = DrawList::new();
        first.spinner(rect, 0, 0x123456);
        let mut next = DrawList::new();
        next.spinner(rect, 1, 0x123456);
        assert_ne!(first.cmds(), next.cmds());
        assert_eq!(first.cmds().len(), 8);
        assert!(first.finish().is_ok() && next.finish().is_ok());
    }
    #[test]
    fn resource_visibility_intersects_viewport_and_all_parent_clips() {
        let mut list = DrawList::new();
        let viewport = Rect::from_size(0.0, 0.0, 100.0, 100.0);
        list.push_clip(Rect::from_size(0.0, 0.0, 50.0, 50.0));
        list.math(Rect::from_size(10.0, 10.0, 20.0, 20.0), "visible", 16.0, 0);
        list.push_clip(Rect::from_size(70.0, 0.0, 30.0, 30.0));
        list.image(viewport, "hidden.png", "");
        list.pop_clip();
        list.math(Rect::from_size(60.0, 10.0, 20.0, 20.0), "right", 16.0, 0);
        list.pop_clip();
        list.image(Rect::from_size(0.0, 150.0, 50.0, 50.0), "below.png", "");
        let resources = list.visible_resources(viewport);
        assert_eq!(resources.len(), 1);
        assert!(matches!(&list.cmds()[resources[0]],DrawCmd::Math{tex,..}if tex=="visible"));
        assert!(list.finish().is_ok());
    }
    #[test]
    fn rotated_images_use_rotated_bounds_and_partial_pixels_are_kept() {
        let mut list = DrawList::new();
        let viewport = Rect::from_size(0.0, 0.0, 100.0, 100.0);
        list.image_rotated(
            Rect::from_size(120.0, 0.0, 20.0, 100.0),
            "rotated.png",
            "",
            90,
        );
        list.math(Rect::from_size(99.5, 20.0, 30.0, 20.0), "partial", 16.0, 0);
        list.math(
            Rect::from_size(100.0, 20.0, 30.0, 20.0),
            "touching",
            16.0,
            0,
        );
        assert_eq!(list.visible_resources(viewport), vec![0, 1]);
    }

    #[test]
    fn a_separator_is_exactly_one_pixel_thick() {
        let mut list = DrawList::new();
        list.hline(0.0, 100.0, 32.0, 0xE7E8EA);
        list.vline(260.0, 0.0, 600.0, 0xE7E8EA);

        assert_eq!(
            list.cmds()[0],
            DrawCmd::Rect {
                rect: Rect::new(0.0, 32.0, 100.0, 33.0),
                color: 0xE7E8EA
            }
        );
        assert_eq!(
            list.cmds()[1],
            DrawCmd::Rect {
                rect: Rect::new(260.0, 0.0, 261.0, 600.0),
                color: 0xE7E8EA
            }
        );
    }

    #[test]
    fn borders_land_inside_the_rect_not_outside_it() {
        // CSS 的 border-b 画在盒子内侧（box-sizing: border-box），
        // 画到外面会盖住下一个面板的第一行像素
        let mut list = DrawList::new();
        let r = Rect::new(0.0, 0.0, 100.0, 32.0);
        list.border_bottom(r, 0x111111);
        list.border_right(r, 0x111111);

        assert_eq!(
            list.cmds()[0],
            DrawCmd::Rect {
                rect: Rect::new(0.0, 31.0, 100.0, 32.0),
                color: 0x111111
            }
        );
        assert_eq!(
            list.cmds()[1],
            DrawCmd::Rect {
                rect: Rect::new(99.0, 0.0, 100.0, 32.0),
                color: 0x111111
            }
        );
    }

    #[test]
    fn a_nested_clip_intersects_with_its_parent_instead_of_replacing_it() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.push_clip(Rect::new(50.0, 50.0, 300.0, 300.0));

        // 内层想要 50..300，但外层只给到 100——生效的是交集
        assert_eq!(list.clip_rect(), Some(Rect::new(50.0, 50.0, 100.0, 100.0)));
        assert_eq!(
            list.cmds()[1],
            DrawCmd::PushClip {
                rect: Rect::new(50.0, 50.0, 100.0, 100.0)
            }
        );
    }

    #[test]
    fn popping_restores_the_parent_clip() {
        let mut list = DrawList::new();
        let outer = Rect::new(0.0, 0.0, 100.0, 100.0);
        list.push_clip(outer);
        list.push_clip(Rect::new(10.0, 10.0, 20.0, 20.0));
        list.pop_clip();
        assert_eq!(list.clip_rect(), Some(outer));
        list.pop_clip();
        assert_eq!(list.clip_rect(), None);
    }

    #[test]
    fn an_extra_pop_is_ignored_rather_than_panicking() {
        // 多弹一次是调用方的错误，但绘制阶段发生 panic 会直接导致整个窗口崩溃，
        // 代价远大于少画一帧——收下并忽略，配平问题交给 finish() 报
        let mut list = DrawList::new();
        list.pop_clip();
        assert!(list.is_empty());
        assert!(list.finish().is_ok());
    }

    #[test]
    fn finish_rejects_a_clip_that_was_never_popped() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(list.finish(), Err(UnbalancedClip { depth: 1 }));

        list.pop_clip();
        assert!(list.finish().is_ok());
    }

    #[test]
    fn clear_resets_the_clip_stack_too() {
        // 上一帧漏了 pop 时，下一帧不该继承那个残留的裁剪区
        let mut list = DrawList::new();
        list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        list.clear();
        assert_eq!(list.clip_rect(), None);
        assert!(list.finish().is_ok());
    }
}
