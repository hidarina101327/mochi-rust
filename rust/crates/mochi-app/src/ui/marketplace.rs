//! 分页显示的社区目录。布局同时决定绘制、鼠标和键盘操作。
use super::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::Palette,
    workspace_ui,
};
use mochi_core::marketplace::{Catalog, Category, Package};

pub const PAGE_SIZE: usize = 12;
#[cfg(any(test, debug_assertions))]
pub mod fixtures;
#[cfg(test)]
mod tests;
#[derive(Default)]
pub struct State {
    pub catalog: Catalog,
    pub category: Option<Category>,
    pub page: usize,
    pub selected: Option<usize>,
    pub scroll: f32,
    pub loading: bool,
    pub downloading: bool,
    pub loaded: bool,
    pub error: String,
    pub notice: String,
    pub hover: Option<Hit>,
    pub focused: Option<Hit>,
}
impl State {
    pub fn filtered(&self) -> Vec<usize> {
        self.catalog
            .packages
            .iter()
            .enumerate()
            .filter(|(_, p)| self.category.is_none_or(|c| p.category == c))
            .map(|(i, _)| i)
            .collect()
    }
    pub fn pages(&self) -> usize {
        self.filtered().len().div_ceil(PAGE_SIZE).max(1)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Category(Option<Category>),
    Item(usize),
    Previous,
    Next,
    Refresh,
    Releases,
    Back,
    Download,
    CopyCommand,
}
#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub content: Rect,
    pub body: Rect,
    pub max_scroll: f32,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, h)| {
                r.contains(x, y) && (!matches!(h, Hit::Item(_)) || self.body.contains(x, y))
            })
            .map(|(_, h)| *h)
    }
}
pub fn layout(state: &State, area: Rect) -> Layout {
    let c = workspace_ui::column(area, f32::MAX);
    let mut out = Layout {
        content: c,
        ..Default::default()
    };
    let mut y = c.top + 54.0;
    if state.selected.is_some() {
        out.entries
            .push((Rect::from_size(c.left, y, 32.0, 32.0), Hit::Back));
        y += 44.0;
        out.body = Rect::new(c.left, y, c.right, (c.bottom - 70.0).max(y));
        if let Some(p) = state.selected.and_then(|i| state.catalog.packages.get(i)) {
            out.max_scroll = (detail_height(p, c.width()) - out.body.height()).max(0.0);
            if !state.downloading {
                out.entries.push((
                    Rect::from_size(c.left, c.bottom - 62.0, 128.0, 36.0),
                    Hit::Download,
                ));
            }
            if p.install_command().is_some() && c.width() >= 290.0 {
                out.entries.push((
                    Rect::from_size(c.left + 142.0, c.bottom - 62.0, 136.0, 36.0),
                    Hit::CopyCommand,
                ));
            }
        }
    } else {
        let mut x = c.left;
        for category in std::iter::once(None).chain(Category::ALL.into_iter().map(Some)) {
            let label = category.map(Category::label).unwrap_or("全部");
            let w = (text::measure(label, TextStyle::Label) + 28.0)
                .max(66.0)
                .min(c.width());
            if x > c.left && x + w > c.right {
                x = c.left;
                y += 38.0;
            }
            out.entries
                .push((Rect::from_size(x, y, w, 32.0), Hit::Category(category)));
            x += w + 6.0;
        }
        y += 46.0;
        out.body = Rect::new(c.left, y, c.right, (c.bottom - 62.0).max(y));
        let indices = state.filtered();
        let page = state.page.min(state.pages() - 1);
        let columns = ((c.width() + 16.0) / 246.0).floor().clamp(1.0, 5.0) as usize;
        let width = (c.width() - (columns - 1) as f32 * 16.0) / columns as f32;
        let items: Vec<_> = indices
            .into_iter()
            .skip(page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .collect();
        out.max_scroll =
            (items.len().div_ceil(columns) as f32 * 222.0 - 16.0 - out.body.height()).max(0.0);
        for (position, i) in items.into_iter().enumerate() {
            out.entries.push((
                Rect::from_size(
                    c.left + (position % columns) as f32 * (width + 16.0),
                    y + (position / columns) as f32 * 222.0
                        - state.scroll.clamp(0.0, out.max_scroll),
                    width,
                    206.0,
                ),
                Hit::Item(i),
            ));
        }
        let fy = c.bottom - 54.0;
        if page > 0 {
            out.entries
                .push((Rect::from_size(c.left, fy, 32.0, 32.0), Hit::Previous));
        }
        if page + 1 < state.pages() {
            out.entries
                .push((Rect::from_size(c.right - 32.0, fy, 32.0, 32.0), Hit::Next));
        }
    }
    let controls_y = c.top;
    if !state.loading && !state.downloading {
        out.entries.push((
            Rect::from_size(c.right - 72.0, controls_y, 32.0, 32.0),
            Hit::Refresh,
        ));
    }
    out.entries.push((
        Rect::from_size(c.right - 32.0, controls_y, 32.0, 32.0),
        Hit::Releases,
    ));
    out
}

fn description(p: &Package) -> &str {
    if p.description.is_empty() {
        &p.summary
    } else {
        &p.description
    }
}
fn wrap(value: &str, style: TextStyle, width: f32) -> Vec<String> {
    value
        .split('\n')
        .flat_map(|line| {
            if line.is_empty() {
                vec![String::new()]
            } else {
                text::wrap_runs(&[text::Run::plain(line)], style, width.max(1.0))
                    .into_iter()
                    .map(|runs| runs.into_iter().map(|r| r.text).collect())
                    .collect()
            }
        })
        .collect()
}
fn detail_height(p: &Package, width: f32) -> f32 {
    264.0
        + wrap(&p.title, TextStyle::Heading2, width).len() as f32 * 32.0
        + wrap(description(p), TextStyle::Body, width).len() as f32 * 25.0
}
fn icon(category: Category) -> Icon {
    match category {
        Category::Document | Category::Template => Icon::FILE_TEXT,
        Category::Base => Icon::TABLE,
        Category::Canvas => Icon::IMAGE,
        Category::Workflow => Icon::PLAY,
        Category::Agent => Icon::BOT,
        Category::KnowledgeBase => Icon::BOOK_OPEN,
        Category::Plugin => Icon::BLOCKS,
    }
}
fn cover(list: &mut DrawList, r: Rect, p: &Package, palette: &Palette) {
    list.rounded_rect(r, 7.0, palette.surface_muted);
    if let Some(src) = &p.image {
        list.image(r, src, &p.title);
    } else {
        list.icon_centered(r, icon(p.category), 36.0, palette.muted);
    }
}
fn lines(
    list: &mut DrawList,
    x: f32,
    y: &mut f32,
    width: f32,
    value: &str,
    style: TextStyle,
    color: u32,
    height: f32,
    limit: usize,
) {
    let wrapped = wrap(value, style, width);
    for (i, line) in wrapped.iter().take(limit).enumerate() {
        let line = if i + 1 == limit && wrapped.len() > limit {
            text::ellipsize(&format!("{line}…"), style, width)
        } else {
            line.clone()
        };
        list.text(Rect::from_size(x, *y, width, height), line, style, color);
        *y += height;
    }
}

pub fn paint(list: &mut DrawList, state: &State, lay: &Layout, p: &Palette) {
    let c = lay.content;
    list.push_clip(c);
    list.text(
        Rect::from_size(c.left, c.top, (c.width() - 88.0).max(1.0), 36.0),
        "官方市场",
        TextStyle::Heading2,
        p.foreground,
    );
    for (r, h) in &lay.entries {
        let hover = state.hover == Some(*h) || state.focused == Some(*h);
        match h {
            Hit::Category(category) => workspace_ui::tab(
                list,
                *r,
                category.map(Category::label).unwrap_or("全部"),
                state.category == *category,
                hover,
                p,
            ),
            Hit::Item(_) => {}
            _ => {
                let label = match h {
                    Hit::Previous => "上一页",
                    Hit::Next => "下一页",
                    Hit::Refresh => "刷新",
                    Hit::Releases => "发布页",
                    Hit::Back => "返回列表",
                    Hit::Download => "下载资源包",
                    Hit::CopyCommand => "复制安装命令",
                    _ => "",
                };
                let icon = match h {
                    Hit::Previous | Hit::Back => Some(Icon::CHEVRON_LEFT),
                    Hit::Next => Some(Icon::CHEVRON_RIGHT),
                    Hit::Refresh => Some(Icon::ROTATE_CCW),
                    Hit::Releases => Some(Icon::EXTERNAL_LINK),
                    _ => None,
                };
                if let Some(icon) = icon {
                    if hover {
                        list.rounded_rect(*r, 6.0, p.surface_muted);
                    }
                    list.icon_centered(*r, icon, 18.0, p.foreground);
                } else {
                    workspace_ui::button(list, *r, label, None, *h == Hit::Download, hover, p);
                }
            }
        }
    }
    list.push_clip(lay.body);
    if let Some(package) = state.selected.and_then(|i| state.catalog.packages.get(i)) {
        let mut y = lay.body.top - state.scroll.clamp(0.0, lay.max_scroll);
        cover(
            list,
            Rect::from_size(c.left, y, c.width(), 180.0),
            package,
            p,
        );
        y += 198.0;
        lines(
            list,
            c.left,
            &mut y,
            c.width(),
            &package.title,
            TextStyle::Heading2,
            p.foreground,
            32.0,
            usize::MAX,
        );
        let meta = format!(
            "{} · {} · v{}{}",
            if package.official {
                "官方"
            } else {
                "第三方"
            },
            package.category.label(),
            package.version,
            if package.author.is_empty() {
                String::new()
            } else {
                format!(" · {}", package.author)
            }
        );
        list.text(
            Rect::from_size(c.left, y, c.width(), 24.0),
            text::ellipsize(&meta, TextStyle::Caption, c.width()),
            TextStyle::Caption,
            p.muted,
        );
        y += 40.0;
        lines(
            list,
            c.left,
            &mut y,
            c.width(),
            description(package),
            TextStyle::Body,
            p.foreground,
            25.0,
            usize::MAX,
        );
    } else {
        for (r, h) in &lay.entries {
            let Hit::Item(i) = h else {
                continue;
            };
            if r.intersect(&lay.body).is_empty() {
                continue;
            }
            let package = &state.catalog.packages[*i];
            let active = state.hover == Some(*h) || state.focused == Some(*h);
            list.rounded_rect(*r, 9.0, if active { p.surface_muted } else { p.surface });
            list.rounded_border(*r, 9.0, if active { p.muted } else { p.border });
            cover(
                list,
                Rect::new(r.left + 10.0, r.top + 10.0, r.right - 10.0, r.top + 78.0),
                package,
                p,
            );
            let x = r.left + 14.0;
            let width = r.width() - 28.0;
            let title_height = TextStyle::Large.line_height().max(28.0);
            let title_extra = title_height - 28.0;
            list.text(
                Rect::from_size(x, r.top + 90.0, width, title_height),
                text::ellipsize(&package.title, TextStyle::Large, width),
                TextStyle::Large,
                p.foreground,
            );
            let mut y = r.top + 122.0 + title_extra;
            lines(
                list,
                x,
                &mut y,
                width,
                &package.summary,
                TextStyle::Caption,
                p.muted,
                21.0,
                2,
            );
            list.text(
                Rect::from_size(x, r.bottom - 30.0, width, 20.0),
                format!(
                    "{}  ·  {}",
                    if package.official {
                        "官方"
                    } else {
                        "第三方"
                    },
                    package.category.label()
                ),
                TextStyle::Caption,
                p.muted,
            );
        }
        if state.filtered().is_empty() {
            let (title, detail) = if state.loading {
                ("正在加载官方市场…", "正在获取最新发布的资源")
            } else if !state.error.is_empty() {
                ("暂时无法加载市场", "点击刷新重试，或前往发布页查看资源")
            } else if state.category == Some(Category::Plugin) {
                ("插件市场即将开放", "此分类已预留，发布后即可在这里发现插件")
            } else {
                ("暂无资源", "新的资源发布后会出现在这里")
            };
            list.icon_centered(
                Rect::from_size(c.left, lay.body.top + 28.0, c.width(), 64.0),
                Icon::BLOCKS,
                36.0,
                p.muted,
            );
            list.text_aligned(
                Rect::from_size(
                    c.left,
                    lay.body.top + 106.0,
                    c.width(),
                    TextStyle::Large.line_height().max(28.0),
                ),
                title,
                TextStyle::Large,
                p.foreground,
                Align::Center,
            );
            list.text_aligned(
                Rect::from_size(c.left, lay.body.top + 147.0, c.width(), 24.0),
                text::ellipsize(detail, TextStyle::Caption, c.width()),
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
        }
    }
    list.pop_clip();
    if lay.max_scroll > 0.0 && lay.body.height() > 0.0 {
        let height = (lay.body.height() * lay.body.height() / (lay.body.height() + lay.max_scroll))
            .max(24.0)
            .min(lay.body.height());
        let top = lay.body.top
            + (lay.body.height() - height) * state.scroll.clamp(0.0, lay.max_scroll)
                / lay.max_scroll;
        list.rounded_rect(
            Rect::from_size(c.right - 3.0, top, 3.0, height),
            1.5,
            p.muted,
        );
    }
    if state.selected.is_none() {
        list.text_aligned(
            Rect::new(
                c.left + 90.0,
                c.bottom - 54.0,
                c.right - 90.0,
                c.bottom - 22.0,
            ),
            format!(
                "{} / {}",
                state.page.min(state.pages() - 1) + 1,
                state.pages()
            ),
            TextStyle::Label,
            p.muted,
            Align::Center,
        );
    }
    let message = if !state.error.is_empty() {
        state.error.clone()
    } else if state.downloading {
        "正在下载并校验资源包…".into()
    } else if state.loading {
        "正在刷新市场…".into()
    } else if !state.notice.is_empty() {
        state.notice.clone()
    } else if let Some(item) = state.selected.and_then(|i| state.catalog.packages.get(i)) {
        format!("{} · {:.1} MB", item.asset, item.size as f64 / 1048576.0)
    } else {
        format!(
            "{} 个资源{}",
            state.filtered().len(),
            if state.catalog.release.is_empty() {
                String::new()
            } else {
                format!(" · {}", state.catalog.release)
            }
        )
    };
    list.text(
        Rect::from_size(c.left, c.bottom - 20.0, c.width(), 20.0),
        text::ellipsize(&message, TextStyle::Caption, c.width()),
        TextStyle::Caption,
        if state.error.is_empty() {
            p.muted
        } else {
            p.danger
        },
    );
    list.pop_clip();
    if let Some((rect, hit)) = lay
        .entries
        .iter()
        .find(|(_, h)| Some(*h) == state.hover.or(state.focused))
    {
        let label = match hit {
            Hit::Refresh => "刷新",
            Hit::Releases => "发布页",
            Hit::Previous => "上一页",
            Hit::Next => "下一页",
            Hit::Back => "返回",
            _ => "",
        };
        if !label.is_empty() {
            let y = if rect.bottom + 34.0 > c.bottom {
                rect.top - 32.0
            } else {
                rect.bottom + 5.0
            };
            let tip = Rect::from_size(
                (rect.right - 76.0).max(c.left).min(c.right - 76.0),
                y,
                76.0,
                26.0,
            );
            list.rounded_rect(tip, 5.0, p.surface);
            list.rounded_border(tip, 5.0, p.border);
            list.text_aligned(tip, label, TextStyle::Caption, p.foreground, Align::Center);
        }
    }
}
