//! 节点放在 arena，父子只存下标。只实现外壳需要的 Fixed/Grow 和交叉轴拉伸，不支持 wrap。

use std::ops::Range;

/// 节点句柄。裸 `usize` 会和行号、子节点下标混作一谈，包一层让类型系统挡住。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

impl NodeId {
    /// 根节点恒为 0——`Tree::new` 保证第一个插入的就是它。
    pub const ROOT: NodeId = NodeId(0);
}

/// 主轴方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

/// 单轴尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    /// 固定像素。
    Fixed(f32),
    /// 按权重瓜分剩余空间。权重相同即等分。
    Grow(f32),
}

impl Size {
    fn fixed_part(self) -> f32 {
        match self {
            Size::Fixed(v) => v,
            Size::Grow(_) => 0.0,
        }
    }

    fn weight(self) -> f32 {
        match self {
            Size::Fixed(_) => 0.0,
            Size::Grow(w) => w.max(0.0),
        }
    }
}

/// 四边内边距。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Edges {
    pub const ZERO: Edges = Edges {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    pub const fn xy(x: f32, y: f32) -> Self {
        Edges {
            left: x,
            top: y,
            right: x,
            bottom: y,
        }
    }
}

/// 定位方式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    /// 参与主轴分配，占据空间。
    Flow,
    /// 脱离流，相对父节点内容区按四边偏移定位——对应 CSS 的 `position: absolute`。
    ///
    /// 不是为了通用性才有这个：`MainLayout.tsx` 里三个元素真的这么写着——
    /// 两条拖动手柄（`absolute top-0 right-0 w-1 h-full`）压在相邻面板的边缘上，
    /// 标题栏的 AI 按钮（`absolute right-[138px]`）躲开 Windows 的窗口控制按钮。
    /// 用 flow 排是排不出「压在边界上」这个效果的。
    Overlay {
        left: Option<f32>,
        top: Option<f32>,
        right: Option<f32>,
        bottom: Option<f32>,
    },
}

impl Position {
    /// 四边全给，等于拉满父内容区。
    /// 分栏拖动手柄要用（`z-10` 压在两栏之间），分栏视图搬过来时接上。
    #[allow(dead_code)]
    pub const fn inset(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Position::Overlay {
            left: Some(left),
            top: Some(top),
            right: Some(right),
            bottom: Some(bottom),
        }
    }
}

/// 布局样式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// 子节点沿哪个轴排列。叶子节点无所谓。
    pub axis: Axis,
    pub position: Position,
    pub width: Size,
    pub height: Size,
    pub padding: Edges,
    /// 子节点之间的间距。
    pub gap: f32,
    /// 主轴最小尺寸。`Grow` 被挤到 0 之前先守住这个值——
    /// 对应 CSS 里到处写的 `min-w-0` 的反面：编辑器列可以被挤扁，但侧栏不行。
    pub min_width: f32,
    pub min_height: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            axis: Axis::Column,
            position: Position::Flow,
            width: Size::Grow(1.0),
            height: Size::Grow(1.0),
            padding: Edges::ZERO,
            gap: 0.0,
            min_width: 0.0,
            min_height: 0.0,
        }
    }
}

impl Style {
    pub fn row() -> Self {
        Style {
            axis: Axis::Row,
            ..Default::default()
        }
    }

    pub fn column() -> Self {
        Style {
            axis: Axis::Column,
            ..Default::default()
        }
    }

    pub fn w(mut self, size: Size) -> Self {
        self.width = size;
        self
    }

    pub fn h(mut self, size: Size) -> Self {
        self.height = size;
        self
    }

    pub fn padding(mut self, e: Edges) -> Self {
        self.padding = e;
        self
    }

    pub fn gap(mut self, g: f32) -> Self {
        self.gap = g;
        self
    }

    pub fn min_w(mut self, v: f32) -> Self {
        self.min_width = v;
        self
    }

    /// 主轴最小尺寸的纵向版本。与 `min_w` 成对，横排面板用不上但竖排列表要
    /// （工具条/状态栏被挤到 0 高会让整列内容错位）。
    #[allow(dead_code)]
    pub fn min_h(mut self, v: f32) -> Self {
        self.min_height = v;
        self
    }

    pub fn overlay(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    fn is_overlay(&self) -> bool {
        matches!(self.position, Position::Overlay { .. })
    }
}

/// 矩形。左上闭、右下开——命中测试用 `left <= x < right`，
/// 相邻两个面板的边界像素才不会同时命中两边。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn from_size(left: f32, top: f32, width: f32, height: f32) -> Self {
        Rect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    pub fn width(&self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    pub fn height(&self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    /// 收缩四边，得到内容区。
    pub fn inset(&self, e: Edges) -> Rect {
        Rect {
            left: self.left + e.left,
            top: self.top + e.top,
            right: (self.right - e.right).max(self.left + e.left),
            bottom: (self.bottom - e.bottom).max(self.top + e.top),
        }
    }

    /// 与另一个矩形求交。用于裁剪——子节点画到父节点外面时要被切掉。
    pub fn intersect(&self, other: &Rect) -> Rect {
        let r = Rect {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        if r.left >= r.right || r.top >= r.bottom {
            Rect::ZERO
        } else {
            r
        }
    }

    /// 沿主轴切出前 `amount` 像素，返回 (切出的, 剩下的)。
    /// 面板内部分行/分列时比手算坐标可靠——编辑器分栏、AI 消息气泡都要用。
    #[allow(dead_code)]
    pub fn split_top(&self, amount: f32) -> (Rect, Rect) {
        let cut = (self.top + amount).min(self.bottom);
        (
            Rect {
                bottom: cut,
                ..*self
            },
            Rect { top: cut, ..*self },
        )
    }

    #[allow(dead_code)]
    pub fn split_left(&self, amount: f32) -> (Rect, Rect) {
        let cut = (self.left + amount).min(self.right);
        (
            Rect {
                right: cut,
                ..*self
            },
            Rect { left: cut, ..*self },
        )
    }
}

/// 节点身份。命中测试要回答的是「点到哪个区域」，返回 `NodeId` 没用——
/// 那是个跟着树重建而变的下标。用语义化的 key，调用方才敢 `match`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKey {
    /// 纯容器，不参与命中。
    None,
    TitleBar,
    TitleBarThemeToggle,
    TitleBarBack,
    TitleBarAiToggle,
    TitleBarNotifications,
    TitleBarDesktop,
    TitleBarMarketplace,
    TitleBarTemplates,
    TitleBarAutomations,
    Navigation,
    NavigationResize,
    Sidebar,
    SidebarResize,
    EditorColumn,
    TabBar,
    Editor,
    StatusBar,
    RightSidebar,
    RightSidebarResize,
    RightSidebarToolbar,
    RightSidebarBody,
}

struct Node {
    style: Style,
    key: NodeKey,
    /// 隐藏节点不占空间、不参与命中测试。对应 React 里的条件渲染
    /// （`showWorkspaceLeftPanel && <div/>`）——注意是**不占空间**，
    /// 不是画成透明；后者会让 flex 分配结果不一样。
    hidden: bool,
    children: Vec<NodeId>,
    /// 布局解出来的绝对矩形。`layout()` 之前是 `ZERO`。
    rect: Rect,
    /// 与所有祖先裁剪区求交后的可见矩形。命中测试用它，不是 `rect`——
    /// 滚出面板外的行在屏幕上看不见，就不该能被点到。
    clip: Rect,
}

/// 控件树。
pub struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    /// 建树并插入根节点。根的样式决定整棵树的主轴方向。
    pub fn new(root_style: Style) -> Self {
        Tree {
            nodes: vec![Node {
                style: root_style,
                key: NodeKey::None,
                hidden: false,
                children: Vec::new(),
                rect: Rect::ZERO,
                clip: Rect::ZERO,
            }],
        }
    }

    /// 追加一个子节点，返回它的句柄。
    pub fn add(&mut self, parent: NodeId, key: NodeKey, style: Style) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            style,
            key,
            hidden: false,
            children: Vec::new(),
            rect: Rect::ZERO,
            clip: Rect::ZERO,
        });
        self.nodes[parent.0].children.push(id);
        id
    }

    /// 条件渲染。隐藏的子树整体不占空间。
    pub fn set_hidden(&mut self, id: NodeId, hidden: bool) {
        self.nodes[id.0].hidden = hidden;
    }

    pub fn is_hidden(&self, id: NodeId) -> bool {
        self.nodes[id.0].hidden
    }

    pub fn rect(&self, id: NodeId) -> Rect {
        self.nodes[id.0].rect
    }

    /// 与祖先裁剪区求交后的可见矩形。滚动面板画内容前要按它压裁剪。
    #[allow(dead_code)]
    pub fn clip(&self, id: NodeId) -> Rect {
        self.nodes[id.0].clip
    }

    /// 下面三个访问器现在没人调，但都是马上要用的：拖动手柄改宽度走 `style_mut`，
    /// 焦点/键盘导航要按 `key` 找节点，遍历子树要 `children`。
    /// 与其等到那时再补，不如现在留着——它们的语义此刻最清楚。
    #[allow(dead_code)]
    pub fn style_mut(&mut self, id: NodeId) -> &mut Style {
        &mut self.nodes[id.0].style
    }

    #[allow(dead_code)]
    pub fn key(&self, id: NodeId) -> NodeKey {
        self.nodes[id.0].key
    }

    #[allow(dead_code)]
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0].children
    }

    /// 解算整棵树。`viewport` 是窗口客户区。
    pub fn layout(&mut self, viewport: Rect) {
        self.place(NodeId::ROOT, viewport, viewport);
    }

    /// 把 `id` 放进 `rect`，再排它的子节点。`clip` 是祖先累积下来的可见区。
    fn place(&mut self, id: NodeId, rect: Rect, clip: Rect) {
        let clip = rect.intersect(&clip);
        {
            let node = &mut self.nodes[id.0];
            node.rect = rect;
            node.clip = clip;
        }

        let padding = self.nodes[id.0].style.padding;
        let content = rect.inset(padding);

        // 只排可见子节点；隐藏的整棵子树塌成零矩形，免得残留上一帧的坐标
        // 被命中测试摸到（`clip` 为 ZERO 时 `contains` 恒假，但 rect 也一起清掉更干净）。
        let mut flow: Vec<NodeId> = Vec::new();
        let mut overlay: Vec<NodeId> = Vec::new();
        let mut hidden: Vec<NodeId> = Vec::new();
        for c in self.nodes[id.0].children.iter().copied() {
            let n = &self.nodes[c.0];
            if n.hidden {
                hidden.push(c);
            } else if n.style.is_overlay() {
                overlay.push(c);
            } else {
                flow.push(c);
            }
        }
        for h in hidden {
            self.collapse(h);
        }

        self.place_flow(id, content, clip, &flow);

        // 覆盖层最后排，也最后画——它们本来就是压在同级 flow 节点上面的。
        //
        // 基准是父节点的 **padding 盒**（即 `rect`）而不是内容盒（`content`）：
        // CSS 的 `position: absolute` 就是相对 padding 盒定位的。标题栏写着 `px-4`，
        // 拿内容盒当基准会让 `right-[138px]` 的 AI 按钮再往左挪 16px。
        for o in overlay {
            let r = self.overlay_rect(o, rect);
            self.place(o, r, clip);
        }
    }

    /// 覆盖层的绝对矩形：给了哪边就贴哪边，两边都给就拉伸，都不给就退回内容区原点。
    fn overlay_rect(&self, id: NodeId, content: Rect) -> Rect {
        let style = self.nodes[id.0].style;
        let Position::Overlay {
            left,
            top,
            right,
            bottom,
        } = style.position
        else {
            return content;
        };

        let axis_span =
            |lo: Option<f32>, hi: Option<f32>, size: Size, min: f32, start: f32, end: f32| {
                let extent = end - start;
                match (lo, hi) {
                    // 两边都锚定 → 拉伸，忽略 width/height
                    (Some(a), Some(b)) => (start + a, (end - b).max(start + a)),
                    (Some(a), None) => {
                        let s = start + a;
                        let len = match size {
                            Size::Fixed(v) => v.max(min),
                            Size::Grow(_) => (extent - a).max(min),
                        };
                        (s, s + len)
                    }
                    (None, Some(b)) => {
                        let e = end - b;
                        let len = match size {
                            Size::Fixed(v) => v.max(min),
                            Size::Grow(_) => (extent - b).max(min),
                        };
                        (e - len, e)
                    }
                    (None, None) => {
                        let len = match size {
                            Size::Fixed(v) => v.max(min),
                            Size::Grow(_) => extent,
                        };
                        (start, start + len)
                    }
                }
            };

        let (l, r) = axis_span(
            left,
            right,
            style.width,
            style.min_width,
            content.left,
            content.right,
        );
        let (t, b) = axis_span(
            top,
            bottom,
            style.height,
            style.min_height,
            content.top,
            content.bottom,
        );
        Rect {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    /// 沿主轴排列参与流的子节点。
    fn place_flow(&mut self, id: NodeId, content: Rect, clip: Rect, visible: &[NodeId]) {
        let (axis, gap) = {
            let s = &self.nodes[id.0].style;
            (s.axis, s.gap)
        };
        if visible.is_empty() {
            return;
        }

        let main_total = match axis {
            Axis::Row => content.width(),
            Axis::Column => content.height(),
        };
        let gaps = gap * (visible.len() - 1) as f32;

        // 主轴：固定的先扣掉，剩下的按权重分给 Grow，并守住各自的最小值。
        let sizes = solve_main_axis(
            visible
                .iter()
                .map(|c| {
                    let s = &self.nodes[c.0].style;
                    match axis {
                        Axis::Row => (s.width, s.min_width),
                        Axis::Column => (s.height, s.min_height),
                    }
                })
                .collect::<Vec<_>>()
                .as_slice(),
            (main_total - gaps).max(0.0),
        );

        let mut cursor = match axis {
            Axis::Row => content.left,
            Axis::Column => content.top,
        };
        for (child, main) in visible.iter().zip(sizes) {
            // 交叉轴一律拉伸到内容区；`Fixed` 则取固定值。
            let child_rect = match axis {
                Axis::Row => {
                    let h = match self.nodes[child.0].style.height {
                        Size::Fixed(v) => v.min(content.height()),
                        Size::Grow(_) => content.height(),
                    };
                    Rect::from_size(cursor, content.top, main, h)
                }
                Axis::Column => {
                    let w = match self.nodes[child.0].style.width {
                        Size::Fixed(v) => v.min(content.width()),
                        Size::Grow(_) => content.width(),
                    };
                    Rect::from_size(content.left, cursor, w, main)
                }
            };
            cursor += main + gap;
            self.place(*child, child_rect, clip);
        }
    }

    /// 把子树的矩形全部清零。
    fn collapse(&mut self, id: NodeId) {
        self.nodes[id.0].rect = Rect::ZERO;
        self.nodes[id.0].clip = Rect::ZERO;
        let children = self.nodes[id.0].children.clone();
        for c in children {
            self.collapse(c);
        }
    }

    /// 命中测试。返回最深、最靠上的那个带 key 的节点。
    ///
    /// 「最靠上」有两层：同为 flow 的兄弟里后画的盖在先画的上面，所以倒序找；
    /// 而**覆盖层整体盖在 flow 兄弟之上**，所以先查覆盖层。后者不是我随手定的顺序——
    /// TSX 里两条拖动手柄写着 `z-10`，压在相邻面板的边缘上就是它们存在的意义。
    pub fn hit(&self, x: f32, y: f32) -> Option<(NodeId, NodeKey)> {
        self.hit_in(NodeId::ROOT, x, y)
    }

    fn hit_in(&self, id: NodeId, x: f32, y: f32) -> Option<(NodeId, NodeKey)> {
        let node = &self.nodes[id.0];
        if node.hidden || !node.clip.contains(x, y) {
            return None;
        }
        let overlay_first = node
            .children
            .iter()
            .rev()
            .filter(|c| self.nodes[c.0].style.is_overlay())
            .chain(
                node.children
                    .iter()
                    .rev()
                    .filter(|c| !self.nodes[c.0].style.is_overlay()),
            );
        for child in overlay_first {
            if let Some(found) = self.hit_in(*child, x, y) {
                return Some(found);
            }
        }
        if node.key == NodeKey::None {
            None
        } else {
            Some((id, node.key))
        }
    }
}

/// 主轴分配：固定尺寸照付，剩余按权重分，最小值守住。
///
/// 分两遍是必要的：先按权重分一遍，把被最小值顶回去的项**固定住**再分剩下的，
/// 否则一个 `min_width: 340` 的 AI 面板会在窗口很窄时把编辑器挤成负数。
fn solve_main_axis(items: &[(Size, f32)], available: f32) -> Vec<f32> {
    let mut out: Vec<f32> = items
        .iter()
        .map(|(s, min)| s.fixed_part().max(*min))
        .collect();
    let mut settled: Vec<bool> = items
        .iter()
        .map(|(s, _)| matches!(s, Size::Fixed(_)))
        .collect();

    loop {
        let used: f32 = out
            .iter()
            .zip(&settled)
            .filter(|(_, s)| **s)
            .map(|(v, _)| *v)
            .sum();
        let remaining = (available - used).max(0.0);
        let total_weight: f32 = items
            .iter()
            .zip(&settled)
            .filter(|(_, s)| !**s)
            .map(|((s, _), _)| s.weight())
            .sum();

        if total_weight <= 0.0 {
            // 没有可伸缩的项了：剩下的未定项按各自最小值收尾。
            for i in 0..out.len() {
                if !settled[i] {
                    out[i] = items[i].1;
                    settled[i] = true;
                }
            }
            break;
        }

        // 分一遍，看有没有谁掉到最小值以下；有就把它钉住重来。
        let mut violated = None;
        for i in 0..items.len() {
            if settled[i] {
                continue;
            }
            let share = remaining * items[i].0.weight() / total_weight;
            if share < items[i].1 {
                violated = Some(i);
                break;
            }
            out[i] = share;
        }
        match violated {
            Some(i) => {
                out[i] = items[i].1;
                settled[i] = true;
            }
            None => break,
        }
    }
    out
}

/// 行列表的可视区计算。文件树、标签页、消息列表都要这套算术，
/// 每个面板各写一遍就意味着每个面板各错一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleRows {
    /// 首个应当绘制的行下标。
    pub start: usize,
    /// 末行的**开区间**上界。
    pub end: usize,
}

impl VisibleRows {
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// 定高行列表：给定总行数、行高、滚动偏移和可视高度，算出该画哪几行。
///
/// `scroll_top` 是**行数**而非像素——与 `shell.rs` 现有的滚动模型一致。
/// 多画一行是有意的：滚动到半行时底部那行露出一截，少画就会看到空白边。
pub fn visible_rows(
    total: usize,
    row_height: f32,
    scroll_top: usize,
    viewport: f32,
) -> VisibleRows {
    if total == 0 || row_height <= 0.0 || viewport <= 0.0 {
        return VisibleRows { start: 0, end: 0 };
    }
    let start = scroll_top.min(total);
    let count = (viewport / row_height).ceil() as usize + 1;
    VisibleRows {
        start,
        end: (start + count).min(total),
    }
}

/// 等高行命中；超出列表返回 None，不能夹到末行。带间距的文件树使用 sidebar 布局表。
#[allow(dead_code)]
pub fn row_at(y: f32, row_height: f32, scroll_top: usize, total: usize) -> Option<usize> {
    if y < 0.0 || row_height <= 0.0 {
        return None;
    }
    let index = scroll_top + (y / row_height) as usize;
    (index < total).then_some(index)
}

/// 物理像素 → DIP。
///
/// **这是一处真会错的换算**：Direct2D 的 `GetSize()` 给的是 DIP，而 Win32 鼠标消息
/// （`WM_LBUTTONDOWN` 的 lParam）给的是物理像素。两者在 100% 缩放下相等，一旦上了
/// 高 DPI 就系统性错位——实测这台机器 144 DPI，不换算的话点击整体偏 1.5 倍，
/// 点侧栏中间会命中导航轨。布局全程在 DIP 空间里做，所以换算要在**入口**完成。
pub fn to_dips(physical: f32, dpi: u32) -> f32 {
    if dpi == 0 {
        return physical; // 拿不到 DPI 时按 96 处理，总比除以零好
    }
    physical * 96.0 / dpi as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_fixed_child_keeps_its_size_and_the_grow_child_takes_the_rest() {
        let mut tree = Tree::new(Style::column());
        let bar = tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::column().h(Size::Fixed(32.0)),
        );
        let body = tree.add(NodeId::ROOT, NodeKey::EditorColumn, Style::row());
        tree.layout(Rect::new(0.0, 0.0, 1200.0, 800.0));

        assert_eq!(tree.rect(bar), Rect::new(0.0, 0.0, 1200.0, 32.0));
        assert_eq!(tree.rect(body), Rect::new(0.0, 32.0, 1200.0, 800.0));
    }

    #[test]
    fn two_grow_children_split_the_space_by_weight() {
        let mut tree = Tree::new(Style::row());
        let a = tree.add(
            NodeId::ROOT,
            NodeKey::Editor,
            Style::row().w(Size::Grow(1.0)),
        );
        let b = tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::row().w(Size::Grow(3.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 400.0, 100.0));

        assert!(approx(tree.rect(a).width(), 100.0));
        assert!(approx(tree.rect(b).width(), 300.0));
    }

    #[test]
    fn gap_is_inserted_between_siblings_but_not_at_the_edges() {
        let mut tree = Tree::new(Style::row().gap(10.0));
        let a = tree.add(
            NodeId::ROOT,
            NodeKey::Editor,
            Style::row().w(Size::Fixed(30.0)),
        );
        let b = tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::row().w(Size::Fixed(30.0)),
        );
        let c = tree.add(
            NodeId::ROOT,
            NodeKey::TabBar,
            Style::row().w(Size::Fixed(30.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 200.0, 50.0));

        assert!(approx(tree.rect(a).left, 0.0));
        assert!(approx(tree.rect(b).left, 40.0));
        assert!(approx(tree.rect(c).left, 80.0));
    }

    #[test]
    fn padding_shrinks_the_content_box_on_all_four_sides() {
        let mut tree = Tree::new(Style::column().padding(Edges::xy(8.0, 4.0)));
        let child = tree.add(NodeId::ROOT, NodeKey::Editor, Style::column());
        tree.layout(Rect::new(0.0, 0.0, 100.0, 100.0));

        assert_eq!(tree.rect(child), Rect::new(8.0, 4.0, 92.0, 96.0));
    }

    #[test]
    fn a_hidden_child_takes_no_space_at_all() {
        let mut tree = Tree::new(Style::row());
        let side = tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::row().w(Size::Fixed(260.0)),
        );
        let main = tree.add(NodeId::ROOT, NodeKey::Editor, Style::row());
        tree.set_hidden(side, true);
        tree.layout(Rect::new(0.0, 0.0, 1000.0, 100.0));

        // 不是「宽度为 0 但仍占位」，是整条被跳过——主区从 0 起算
        assert_eq!(tree.rect(side), Rect::ZERO);
        assert_eq!(tree.rect(main), Rect::new(0.0, 0.0, 1000.0, 100.0));
    }

    #[test]
    fn hiding_a_container_collapses_its_whole_subtree() {
        let mut tree = Tree::new(Style::row());
        let panel = tree.add(
            NodeId::ROOT,
            NodeKey::RightSidebar,
            Style::column().w(Size::Fixed(420.0)),
        );
        let toolbar = tree.add(
            panel,
            NodeKey::RightSidebarToolbar,
            Style::row().h(Size::Fixed(40.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 1000.0, 600.0));
        assert!(!tree.rect(toolbar).is_empty());

        tree.set_hidden(panel, true);
        tree.layout(Rect::new(0.0, 0.0, 1000.0, 600.0));
        // 子孙也要清零，否则上一帧的坐标会被命中测试摸到
        assert_eq!(tree.rect(toolbar), Rect::ZERO);
        assert_eq!(tree.hit(10.0, 10.0), None);
    }

    #[test]
    fn min_width_wins_over_the_weighted_share() {
        // 窄窗口：AI 面板 min 340，编辑器该被挤到剩下的 60，而不是各分 200
        let mut tree = Tree::new(Style::row());
        let editor = tree.add(
            NodeId::ROOT,
            NodeKey::Editor,
            Style::row().w(Size::Grow(1.0)),
        );
        let ai = tree.add(
            NodeId::ROOT,
            NodeKey::RightSidebar,
            Style::row().w(Size::Grow(1.0)).min_w(340.0),
        );
        tree.layout(Rect::new(0.0, 0.0, 400.0, 100.0));

        assert!(approx(tree.rect(ai).width(), 340.0));
        assert!(approx(tree.rect(editor).width(), 60.0));
    }

    #[test]
    fn a_grow_child_is_never_given_negative_space() {
        let mut tree = Tree::new(Style::row());
        let fixed = tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::row().w(Size::Fixed(500.0)),
        );
        let grow = tree.add(
            NodeId::ROOT,
            NodeKey::Editor,
            Style::row().w(Size::Grow(1.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 300.0, 100.0));

        assert!(approx(tree.rect(fixed).width(), 500.0));
        assert_eq!(tree.rect(grow).width(), 0.0);
    }

    #[test]
    fn hit_test_returns_the_deepest_keyed_node() {
        let mut tree = Tree::new(Style::column());
        let bar = tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row().h(Size::Fixed(32.0)),
        );
        let _toggle = tree.add(
            bar,
            NodeKey::TitleBarAiToggle,
            Style::row().w(Size::Fixed(32.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 1200.0, 800.0));

        // 按钮在标题栏左上角（本例没做右对齐），点它应当拿到按钮而不是标题栏
        assert_eq!(
            tree.hit(10.0, 10.0).map(|h| h.1),
            Some(NodeKey::TitleBarAiToggle)
        );
        assert_eq!(tree.hit(500.0, 10.0).map(|h| h.1), Some(NodeKey::TitleBar));
    }

    #[test]
    fn an_overlay_anchors_to_the_padding_box_not_the_content_box() {
        // CSS 的 position:absolute 相对 padding 盒定位，父节点的 padding 不参与。
        // 拿内容盒当基准的话，标题栏的 px-4 会让 right-[138px] 的按钮再左移 16px。
        let mut tree = Tree::new(Style::column());
        let bar = tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row()
                .h(Size::Fixed(32.0))
                .padding(Edges::xy(16.0, 0.0)),
        );
        let toggle = tree.add(
            bar,
            NodeKey::TitleBarAiToggle,
            Style::row()
                .w(Size::Fixed(32.0))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(138.0),
                    bottom: Some(0.0),
                }),
        );
        tree.layout(Rect::new(0.0, 0.0, 1200.0, 800.0));

        // 1200 - 138 = 1062，而不是 (1200-16) - 138 = 1046
        assert_eq!(tree.rect(toggle).right, 1062.0);
    }

    #[test]
    fn an_overlay_wins_the_hit_test_over_a_later_flow_sibling() {
        // AI 面板的拖动手柄先于工具条/正文加入，但 TSX 给它写了 z-10。
        // 若只按「后加入的赢」，手柄会被正文盖住，永远拖不动。
        let mut tree = Tree::new(Style::row());
        let panel = tree.add(
            NodeId::ROOT,
            NodeKey::RightSidebar,
            Style::column().w(Size::Fixed(420.0)),
        );
        let handle = tree.add(
            panel,
            NodeKey::RightSidebarResize,
            Style::column()
                .w(Size::Fixed(4.0))
                .overlay(Position::Overlay {
                    left: Some(0.0),
                    top: Some(0.0),
                    right: None,
                    bottom: Some(0.0),
                }),
        );
        // 手柄之后才加入的 flow 子节点，铺满整个面板
        tree.add(
            panel,
            NodeKey::RightSidebarBody,
            Style::column().h(Size::Grow(1.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 420.0, 600.0));

        assert_eq!(tree.rect(handle), Rect::new(0.0, 0.0, 4.0, 600.0));
        assert_eq!(
            tree.hit(2.0, 300.0).map(|h| h.1),
            Some(NodeKey::RightSidebarResize)
        );
        // 手柄之外仍归正文
        assert_eq!(
            tree.hit(200.0, 300.0).map(|h| h.1),
            Some(NodeKey::RightSidebarBody)
        );
    }

    #[test]
    fn an_overlay_child_sits_on_top_of_its_flow_siblings() {
        // 导航面板右缘的拖动手柄：absolute top-0 right-0 w-1 h-full
        let mut tree = Tree::new(Style::row());
        let nav = tree.add(
            NodeId::ROOT,
            NodeKey::Navigation,
            Style::column().w(Size::Fixed(220.0)),
        );
        let handle = tree.add(
            nav,
            NodeKey::NavigationResize,
            Style::column()
                .w(Size::Fixed(4.0))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(0.0),
                    bottom: Some(0.0),
                }),
        );
        tree.layout(Rect::new(0.0, 0.0, 1000.0, 600.0));

        // 贴住导航面板的右缘，纵向拉满
        assert_eq!(tree.rect(handle), Rect::new(216.0, 0.0, 220.0, 600.0));
        // 手柄压在导航面板上面：点手柄拿到手柄，点旁边拿到面板
        assert_eq!(
            tree.hit(218.0, 300.0).map(|h| h.1),
            Some(NodeKey::NavigationResize)
        );
        assert_eq!(
            tree.hit(100.0, 300.0).map(|h| h.1),
            Some(NodeKey::Navigation)
        );
    }

    #[test]
    fn an_overlay_child_consumes_no_main_axis_space() {
        // 覆盖层若参与分配，下面这个 Grow 兄弟就会少拿 4px
        let mut tree = Tree::new(Style::column());
        let overlay = tree.add(
            NodeId::ROOT,
            NodeKey::NavigationResize,
            Style::column()
                .h(Size::Fixed(4.0))
                .overlay(Position::inset(0.0, 0.0, 0.0, 0.0)),
        );
        let body = tree.add(NodeId::ROOT, NodeKey::Editor, Style::column());
        tree.layout(Rect::new(0.0, 0.0, 100.0, 600.0));

        assert_eq!(tree.rect(body).height(), 600.0);
        // 四边全锚定 → 拉满，忽略 Fixed(4)
        assert_eq!(tree.rect(overlay), Rect::new(0.0, 0.0, 100.0, 600.0));
    }

    #[test]
    fn an_overlay_anchored_to_the_right_offsets_from_the_right_edge() {
        // 标题栏的 AI 按钮：absolute top-0 right-[138px]，32x32
        let mut tree = Tree::new(Style::column());
        let bar = tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row().h(Size::Fixed(32.0)),
        );
        let toggle = tree.add(
            bar,
            NodeKey::TitleBarAiToggle,
            Style::row()
                .w(Size::Fixed(32.0))
                .h(Size::Fixed(32.0))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(138.0),
                    bottom: None,
                }),
        );
        tree.layout(Rect::new(0.0, 0.0, 1200.0, 800.0));

        assert_eq!(tree.rect(toggle), Rect::new(1030.0, 0.0, 1062.0, 32.0));
        assert_eq!(
            tree.hit(1040.0, 16.0).map(|h| h.1),
            Some(NodeKey::TitleBarAiToggle)
        );
        // 让开的那 138px 归标题栏（真实版里是 Windows 的窗口控制按钮区）
        assert_eq!(tree.hit(1100.0, 16.0).map(|h| h.1), Some(NodeKey::TitleBar));
    }

    #[test]
    fn hit_test_misses_outside_the_viewport() {
        let mut tree = Tree::new(Style::column());
        tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row().h(Size::Fixed(32.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 100.0, 100.0));

        assert_eq!(tree.hit(-1.0, 10.0), None);
        assert_eq!(tree.hit(10.0, 200.0), None);
    }

    #[test]
    fn a_child_drawn_past_its_parent_is_clipped_out_of_hit_testing() {
        // 父面板只有 50 高，子节点要 200——溢出的部分屏幕上看不见，就不该能点到
        let mut tree = Tree::new(Style::column());
        let panel = tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::column().h(Size::Fixed(50.0)),
        );
        let tall = tree.add(
            panel,
            NodeKey::Editor,
            Style::column().h(Size::Fixed(200.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 100.0, 300.0));

        assert_eq!(tree.rect(tall).bottom, 200.0);
        assert_eq!(tree.clip(tall).bottom, 50.0);
        assert_eq!(tree.hit(10.0, 20.0).map(|h| h.1), Some(NodeKey::Editor));
        assert_eq!(tree.hit(10.0, 80.0), None);
    }

    #[test]
    fn the_boundary_pixel_belongs_to_the_right_hand_panel() {
        // left <= x < right：x=260 正好是侧栏右边界，应当命中主区而不是侧栏
        let mut tree = Tree::new(Style::row());
        tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::row().w(Size::Fixed(260.0)),
        );
        tree.add(NodeId::ROOT, NodeKey::Editor, Style::row());
        tree.layout(Rect::new(0.0, 0.0, 1000.0, 100.0));

        assert_eq!(tree.hit(259.0, 10.0).map(|h| h.1), Some(NodeKey::Sidebar));
        assert_eq!(tree.hit(260.0, 10.0).map(|h| h.1), Some(NodeKey::Editor));
    }

    #[test]
    fn rect_split_helpers_never_overshoot_the_source() {
        let r = Rect::new(0.0, 0.0, 100.0, 40.0);
        let (top, rest) = r.split_top(500.0);
        assert_eq!(top, r);
        assert_eq!(rest.height(), 0.0);

        let (left, right) = r.split_left(30.0);
        assert_eq!(left.right, 30.0);
        assert_eq!(right.left, 30.0);
    }

    #[test]
    fn visible_rows_draws_one_extra_row_so_a_half_scrolled_row_is_not_blank() {
        // 视口 100px、行高 26px：完整可见 3 行，第 4 行露出 22px，所以要画 4+1
        let v = visible_rows(100, 26.0, 0, 100.0);
        assert_eq!(v.start, 0);
        assert_eq!(v.end, 5);
    }

    #[test]
    fn visible_rows_is_clamped_by_the_total_count() {
        let v = visible_rows(3, 26.0, 0, 1000.0);
        assert_eq!(v.range().len(), 3);

        let v = visible_rows(10, 26.0, 8, 1000.0);
        assert_eq!(v.start, 8);
        assert_eq!(v.end, 10);
    }

    #[test]
    fn visible_rows_of_an_empty_list_is_an_empty_range() {
        assert_eq!(visible_rows(0, 26.0, 0, 500.0).range().len(), 0);
        assert_eq!(visible_rows(10, 26.0, 0, 0.0).range().len(), 0);
    }

    #[test]
    fn row_at_maps_pixels_to_indices_and_misses_below_the_list() {
        assert_eq!(row_at(0.0, 26.0, 0, 5), Some(0));
        assert_eq!(row_at(25.9, 26.0, 0, 5), Some(0));
        assert_eq!(row_at(26.0, 26.0, 0, 5), Some(1));
        // 列表只有 5 行，点在第 6 行的位置是空白
        assert_eq!(row_at(26.0 * 5.0, 26.0, 0, 5), None);
        assert_eq!(row_at(-1.0, 26.0, 0, 5), None);
    }

    #[test]
    fn row_at_accounts_for_the_scroll_offset() {
        assert_eq!(row_at(0.0, 26.0, 7, 100), Some(7));
        assert_eq!(row_at(26.0, 26.0, 7, 100), Some(8));
    }

    #[test]
    fn physical_pixels_convert_to_dips_by_the_dpi_ratio() {
        // 96 DPI = 100% 缩放，物理即 DIP
        assert_eq!(to_dips(400.0, 96), 400.0);
        // 144 DPI = 150%（实测这台机器就是）
        assert_eq!(to_dips(600.0, 144), 400.0);
        // 192 DPI = 200%
        assert_eq!(to_dips(800.0, 192), 400.0);
    }

    #[test]
    fn a_zero_dpi_falls_back_to_one_to_one_instead_of_dividing_by_zero() {
        // GetDpiForWindow 在窗口还没进入消息循环时可能返回 0
        assert_eq!(to_dips(400.0, 0), 400.0);
    }

    #[test]
    fn without_the_dpi_conversion_a_click_lands_in_the_wrong_panel() {
        // 144 DPI 下 785×496 物理像素只对应 523×331 DIP。
        let mut tree = Tree::new(Style::row());
        tree.add(
            NodeId::ROOT,
            NodeKey::Navigation,
            Style::row().w(Size::Fixed(220.0)),
        );
        tree.add(
            NodeId::ROOT,
            NodeKey::Sidebar,
            Style::row().w(Size::Fixed(260.0)),
        );
        tree.add(NodeId::ROOT, NodeKey::EditorColumn, Style::row());
        tree.layout(Rect::new(
            0.0,
            0.0,
            to_dips(785.0, 144),
            to_dips(496.0, 144),
        ));

        // 失败模式一：点到**错的面板**。物理 300 该是导航轨（DIP 200），
        // 不换算却落进侧栏。
        assert_eq!(tree.hit(300.0, 100.0).map(|h| h.1), Some(NodeKey::Sidebar));
        assert_eq!(
            tree.hit(to_dips(300.0, 144), to_dips(100.0, 144))
                .map(|h| h.1),
            Some(NodeKey::Navigation)
        );

        // 失败模式二：整个**掉出窗口**。物理 600 是侧栏右半（DIP 400），
        // 不换算就超出 523 宽的 DIP 视口，点击直接被吞掉。
        assert_eq!(tree.hit(600.0, 100.0), None);
        assert_eq!(
            tree.hit(to_dips(600.0, 144), to_dips(100.0, 144))
                .map(|h| h.1),
            Some(NodeKey::Sidebar)
        );
    }

    #[test]
    fn the_full_main_layout_matches_the_electron_geometry() {
        // 对着 MainLayout.tsx 搭一遍，验证四个区域的绝对坐标
        // 1200x800 窗口、导航 220、侧栏 260、AI 面板 420、标题栏 32
        let mut tree = Tree::new(Style::column());
        let title = tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row().h(Size::Fixed(32.0)),
        );
        let body = tree.add(NodeId::ROOT, NodeKey::None, Style::row());
        let nav = tree.add(
            body,
            NodeKey::Navigation,
            Style::column().w(Size::Fixed(220.0)),
        );
        let side = tree.add(
            body,
            NodeKey::Sidebar,
            Style::column().w(Size::Fixed(260.0)),
        );
        let editor = tree.add(
            body,
            NodeKey::EditorColumn,
            Style::column().w(Size::Grow(1.0)),
        );
        let ai = tree.add(
            body,
            NodeKey::RightSidebar,
            Style::column().w(Size::Fixed(420.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 1200.0, 800.0));

        assert_eq!(tree.rect(title), Rect::new(0.0, 0.0, 1200.0, 32.0));
        assert_eq!(tree.rect(body), Rect::new(0.0, 32.0, 1200.0, 800.0));
        assert_eq!(tree.rect(nav), Rect::new(0.0, 32.0, 220.0, 800.0));
        assert_eq!(tree.rect(side), Rect::new(220.0, 32.0, 480.0, 800.0));
        assert_eq!(tree.rect(editor), Rect::new(480.0, 32.0, 780.0, 800.0));
        assert_eq!(tree.rect(ai), Rect::new(780.0, 32.0, 1200.0, 800.0));
    }

    #[test]
    fn the_status_bar_lives_inside_the_editor_column_not_across_the_window() {
        // MainLayout.tsx 里 StatusBar 是编辑器列的子节点，不铺满整个窗口宽度。
        // 现有 main.rs 把它画成通栏，是一处 1:1 走样——这条测试钉住正确的形状。
        let mut tree = Tree::new(Style::column());
        tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row().h(Size::Fixed(32.0)),
        );
        let body = tree.add(NodeId::ROOT, NodeKey::None, Style::row());
        tree.add(
            body,
            NodeKey::Sidebar,
            Style::column().w(Size::Fixed(260.0)),
        );
        let editor_col = tree.add(
            body,
            NodeKey::EditorColumn,
            Style::column().w(Size::Grow(1.0)),
        );
        tree.add(
            editor_col,
            NodeKey::TabBar,
            Style::row().h(Size::Fixed(36.0)),
        );
        tree.add(
            editor_col,
            NodeKey::Editor,
            Style::column().h(Size::Grow(1.0)),
        );
        let status = tree.add(
            editor_col,
            NodeKey::StatusBar,
            Style::row().h(Size::Fixed(24.0)),
        );
        tree.layout(Rect::new(0.0, 0.0, 1000.0, 600.0));

        assert_eq!(tree.rect(status), Rect::new(260.0, 576.0, 1000.0, 600.0));
        // 侧栏底部那一格属于侧栏，不属于状态栏
        assert_eq!(tree.hit(100.0, 590.0).map(|h| h.1), Some(NodeKey::Sidebar));
    }
}
