//! 提供新版工作流画布的分类、预览和连线绘制辅助。
use super::painting::{button, status};
use super::*;
use crate::ui::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    text,
    theme::Palette,
};
use mochi_core::workflows::catalog;

pub fn category(kind: &str) -> (&'static str, u32, Icon) {
    match kind {
        "start" => ("触发器", 0x10b981, Icon::PLAY),
        "ai" => ("AI", 0x8b5cf6, Icon::BOT),
        "condition" => ("逻辑", 0x0ea5e9, Icon::GIT_BRANCH),
        "script" | "tool" => ("脚本", 0xec4899, Icon::CODE),
        "end" | "file_write" | "document_append" | "notify" | "schedule_write" | "base_write" => {
            ("输出", 0x3b82f6, Icon::FILE_TEXT)
        }
        _ => ("数据", 0xf59e0b, Icon::TABLE),
    }
}
pub fn preview(n: &Node) -> (String, String) {
    let get = |key: &str| {
        n.config
            .get(key)
            .or_else(|| n.inputs.get(key))
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default()
    };
    match n.kind.as_str() {
        "ai" => (
            format!(
                "提供商：{}",
                if get("provider_id").is_empty() {
                    "当前默认".into()
                } else {
                    get("provider_id")
                }
            ),
            get("prompt"),
        ),
        "script" => (format!("语言：{}", get("language")), get("code")),
        "condition" => (
            format!("条件：{} {}", get("left"), get("operator")),
            format!("比较值：{}", get("right")),
        ),
        "http" => (
            format!("{} · {}", get("method"), get("extract")),
            get("url"),
        ),
        "start" => ("手动 / 定时触发".into(), "接收工作流输入变量".into()),
        "end" => ("返回工作流结果".into(), get("result")),
        _ => (
            get("path"),
            [
                get("content"),
                get("summary"),
                get("message"),
                get("object"),
                get("result"),
            ]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_else(|| "点击配置节点参数".into()),
        ),
    }
}
pub fn curve(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let reach = ((b.0 - a.0).abs() * 0.5).max(48.);
    (0..=40)
        .map(|i| {
            let t = i as f32 / 40.;
            let u = 1. - t;
            (
                u * u * u * a.0
                    + 3. * u * u * t * (a.0 + reach)
                    + 3. * u * t * t * (b.0 - reach)
                    + t * t * t * b.0,
                u * u * u * a.1 + 3. * u * u * t * a.1 + 3. * u * t * t * b.1 + t * t * t * b.1,
            )
        })
        .collect()
}
pub fn graph(list: &mut DrawList, s: &mut State, p: &Palette) {
    let area = s.canvas;
    list.rect(area, p.surface_muted);
    list.push_clip(area);
    let grid = (16. * s.zoom).max(8.);
    let mut x = area.left + s.pan.0.rem_euclid(grid);
    while x < area.right {
        let mut y = area.top + s.pan.1.rem_euclid(grid);
        while y < area.bottom {
            list.rounded_rect(Rect::from_size(x, y, 1.5, 1.5), 0.75, p.border);
            y += grid;
        }
        x += grid;
    }
    if let Some(g) = s.graph().cloned() {
        for (i, e) in g.edges.iter().enumerate() {
            let Some(a) = g.nodes.iter().find(|n| n.id == e.source) else {
                continue;
            };
            let Some(b) = g.nodes.iter().find(|n| n.id == e.target) else {
                continue;
            };
            let a = s.node_rect(a);
            let b = s.node_rect(b);
            let points = curve(
                (
                    a.right,
                    a.top
                        + if e.source_handle.as_deref() == Some("false") {
                            78. * s.zoom
                        } else {
                            32. * s.zoom
                        },
                ),
                (b.left, b.top + 32. * s.zoom),
            );
            let hot = s.hover == Some(Hit::Edge(i))
                || s.hover == Some(Hit::Insert(i))
                || s.selected_edge == Some(i);
            let color = if hot {
                p.accent
            } else if let Some(run) = &s.run {
                match run.nodes.get(&e.source).map(|n| n.status.as_str()) {
                    Some("failed") => p.danger,
                    Some("succeeded") => 0x10b981,
                    _ => p.muted,
                }
            } else {
                p.muted
            };
            list.polyline(points.clone(), color, if hot { 2.5 } else { 1.5 });
            for q in points.windows(2) {
                let r = Rect::new(
                    q[0].0.min(q[1].0) - 5.,
                    q[0].1.min(q[1].1) - 5.,
                    q[0].0.max(q[1].0) + 5.,
                    q[0].1.max(q[1].1) + 5.,
                )
                .intersect(&area);
                if !r.is_empty() {
                    s.hits.push((r, Hit::Edge(i)));
                }
            }
            let last = *points.last().unwrap();
            list.polyline(
                vec![(last.0 - 6., last.1 - 4.), last, (last.0 - 6., last.1 + 4.)],
                color,
                1.5,
            );
            if hot && s.run.is_none() {
                let m = points[20];
                let r = Rect::from_size(m.0 - 12., m.1 - 12., 24., 24.);
                list.rounded_rect(r, 12., p.surface);
                list.rounded_border(r, 12., p.accent);
                button(list, s, r, "＋", Hit::Insert(i), p, false);
            }
        }
        for (target, key, source, output) in s.bindings() {
            let points = curve(
                s.output_center(&g.nodes[source], &output),
                s.input_center(&g.nodes[target], &key),
            );
            let hit = Hit::Binding(target, key.clone());
            let hot = s.hover.as_ref() == Some(&hit)
                || s.selected_binding.as_ref() == Some(&(target, key));
            list.polyline(
                points.clone(),
                if hot { p.accent } else { 0x7794c4 },
                if hot { 2.5 } else { 1.6 },
            );
            for q in points.windows(2) {
                let r = Rect::new(
                    q[0].0.min(q[1].0) - 5.,
                    q[0].1.min(q[1].1) - 5.,
                    q[0].0.max(q[1].0) + 5.,
                    q[0].1.max(q[1].1) + 5.,
                )
                .intersect(&area);
                if !r.is_empty() {
                    s.hits.push((r, hit.clone()));
                }
            }
        }
        for (i, n) in g.nodes.iter().enumerate() {
            let r = s.node_rect(n);
            if r.intersect(&area).is_empty() {
                continue;
            }
            let (_, color, icon) = category(&n.kind);
            let selected = s.selected_node == Some(i) || s.group.contains(&i);
            list.rounded_rect(
                Rect::new(r.left + 2., r.top + 3., r.right + 2., r.bottom + 3.),
                10.,
                p.border,
            );
            list.rounded_rect(r, 10., p.surface_elevated);
            list.rounded_border(r, 10., if selected { p.accent } else { p.border });
            if selected {
                list.rounded_border(
                    Rect::new(r.left - 1., r.top - 1., r.right + 1., r.bottom + 1.),
                    11.,
                    p.accent,
                );
            }
            let text_start = list.cmds().len();
            list.push_clip(r);
            {
                let z = s.zoom;
                let badge = Rect::from_size(r.left + 14. * z, r.top + 14. * z, 30. * z, 30. * z);
                list.rounded_rect_alpha(badge, 7. * z, color, 0.15);
                list.icon_centered(badge, icon, 18. * z, color);
                list.text(
                    Rect::new(
                        r.left + 54. * z,
                        r.top + 10. * z,
                        r.right - 40. * z,
                        r.top + 33. * z,
                    ),
                    text::ellipsize(&n.label, TextStyle::Label, (r.width() - 100. * z) / z),
                    TextStyle::Label,
                    p.foreground,
                );
                list.text(
                    Rect::new(
                        r.left + 54. * z,
                        r.top + 32. * z,
                        r.right - 12. * z,
                        r.top + 54. * z,
                    ),
                    catalog::label(&n.kind),
                    TextStyle::Tiny,
                    p.muted,
                );
                list.hline(
                    r.left + 12. * z,
                    r.right - 12. * z,
                    r.top + 60. * z,
                    p.border,
                );
                let (a, b) = preview(n);
                for (j, value) in [if a.is_empty() {
                    b.replace('\n', " ")
                } else {
                    a
                }]
                .iter()
                .enumerate()
                {
                    list.text(
                        Rect::new(
                            r.left + 14. * z,
                            r.top + (69. + j as f32 * 24.) * z,
                            r.right - 14. * z,
                            r.top + (93. + j as f32 * 24.) * z,
                        ),
                        text::ellipsize(value, TextStyle::Caption, (r.width() - 28. * z) / z),
                        TextStyle::Caption,
                        if j == 0 { p.foreground } else { p.muted },
                    );
                }
                let state = s.run.as_ref().and_then(|run| run.nodes.get(&n.id));
                let footer = if let Some(v) = state {
                    format!(
                        "{}{}",
                        status(&v.status),
                        v.started_at
                            .zip(v.finished_at)
                            .map(|(a, b)| format!(" · {} ms", b.saturating_sub(a)))
                            .unwrap_or_default()
                    )
                } else if !n.enabled {
                    "已停用".into()
                } else {
                    "待运行".into()
                };
                list.text(
                    Rect::new(
                        r.left + 14. * z,
                        r.bottom - 28. * z,
                        r.right - 50. * z,
                        r.bottom - 5. * z,
                    ),
                    footer,
                    TextStyle::Tiny,
                    p.muted,
                );
            }
            list.pop_clip();
            list.scale_text_since(text_start, s.zoom);
            s.hits.push((r.intersect(&area), Hit::Node(i)));
            if s.run.is_none() {
                let menu = Rect::from_size(
                    r.right - 28. * s.zoom,
                    r.top + 10. * s.zoom,
                    24. * s.zoom,
                    24. * s.zoom,
                );
                list.icon_centered(menu, Icon::MORE_HORIZONTAL, 16. * s.zoom, p.muted);
                s.hits.push((menu.intersect(&area), Hit::NodeMenu(i)));
            }
            // 输入和输出行使用实际保存的变量名。
            let z = s.zoom;
            let text_start = list.cmds().len();
            list.text(
                Rect::from_size(r.left + 14. * z, r.top + 101. * z, 110. * z, 20. * z),
                "输入",
                TextStyle::Tiny,
                p.muted,
            );
            list.text(
                Rect::from_size(r.left + 164. * z, r.top + 101. * z, 110. * z, 20. * z),
                "输出",
                TextStyle::Tiny,
                p.muted,
            );
            for key in input_keys(n) {
                let c = s.input_center(n, &key);
                let value = n.inputs[&key]
                    .as_str()
                    .map(|v| s.reference_label(v))
                    .unwrap_or_else(|| n.inputs[&key].to_string());
                let label = if value.is_empty() {
                    key.clone()
                } else {
                    format!("{key} · {value}")
                };
                list.text(
                    Rect::new(
                        r.left + 12. * z,
                        c.1 - 10. * z,
                        r.left + 144. * z,
                        c.1 + 10. * z,
                    ),
                    text::ellipsize(&label, TextStyle::Tiny, 128.),
                    TextStyle::Tiny,
                    p.foreground,
                );
                if n.kind != "start" {
                    port(list, s, c, Hit::InputVariable(i, key), 0x7794c4, p);
                }
            }
            for (index, key) in output_keys(n).iter().enumerate() {
                let c = s.output_center(n, key);
                list.text(
                    Rect::new(
                        r.left + 164. * z,
                        c.1 - 10. * z,
                        r.right - 12. * z,
                        c.1 + 10. * z,
                    ),
                    key,
                    TextStyle::Tiny,
                    p.foreground,
                );
                if n.kind != "end" {
                    port(list, s, c, Hit::OutputVariable(i, index), 0x7794c4, p);
                }
            }
            list.scale_text_since(text_start, z);
            if n.kind != "start" {
                port(
                    list,
                    s,
                    (r.left, r.top + 32. * s.zoom),
                    Hit::Input(i),
                    color,
                    p,
                );
            }
            if n.kind != "end" && n.kind != "condition" {
                port(
                    list,
                    s,
                    (r.right, r.top + 32. * s.zoom),
                    Hit::Port(
                        i,
                        if n.kind == "condition" {
                            Some(true)
                        } else {
                            None
                        },
                    ),
                    color,
                    p,
                );
            }
            if n.kind == "condition" {
                for (branch, offset, icon, tint, label) in [
                    (true, 32., Icon::CHECK, 0x159474, "条件成立"),
                    (false, 78., Icon::X, 0xc28132, "条件不成立"),
                ] {
                    let size = (19. * s.zoom).max(7.);
                    let badge = Rect::from_size(
                        r.right - size * 0.5,
                        r.top + offset * s.zoom - size * 0.5,
                        size,
                        size,
                    );
                    list.rounded_rect(badge, size * 0.5, p.surface);
                    list.rounded_border(badge, size * 0.5, tint);
                    list.icon_centered(badge, icon, size * 0.66, tint);
                    let hit = Hit::Port(i, Some(branch));
                    s.hits.push((badge.intersect(&area), hit));
                    s.tooltips.push((badge, label.into()));
                }
            }
        }
    }
    if let Some(Drag::Wire(i, branch)) = s.drag {
        if let Some(n) = s.graph().and_then(|g| g.nodes.get(i)) {
            let r = s.node_rect(n);
            list.polyline(
                curve(
                    (
                        r.right,
                        r.top
                            + if branch == Some(false) {
                                78. * s.zoom
                            } else {
                                32. * s.zoom
                            },
                    ),
                    s.pointer,
                ),
                p.accent,
                2.,
            );
        }
    }
    if let Some(Drag::Variable(i, output)) = s.drag {
        if let Some(n) = s.graph().and_then(|g| g.nodes.get(i)) {
            if let Some(key) = output_keys(n).get(output) {
                list.polyline(curve(s.output_center(n, key), s.pointer), 0x7794c4, 2.);
            }
        }
    }
    let controls: Vec<_> = [
        (Icon::PLUS, "放大", Hit::ZoomIn),
        (Icon::CROSSHAIR, "重置缩放", Hit::ResetZoom),
        (Icon::MINUS, "缩小", Hit::ZoomOut),
        (Icon::MAXIMIZE2, "显示完整流程", Hit::Fit),
        (Icon::NETWORK, "自动布局", Hit::AutoLayout),
        (Icon::LAYOUT_GRID, "缩略图", Hit::Minimap),
    ]
    .into_iter()
    .filter(|(_, _, hit)| s.run.is_none() || *hit != Hit::AutoLayout)
    .collect();
    if let Some(Drag::Box(x, y)) = s.drag {
        let r = Rect::new(
            x.min(s.pointer.0),
            y.min(s.pointer.1),
            x.max(s.pointer.0),
            y.max(s.pointer.1),
        );
        list.rect_alpha(r, p.accent, 0.08);
        list.rounded_border(r, 1., p.accent);
    }
    let columns = ((area.width() - 32.) / 38.).floor().max(1.) as usize;
    let rows = controls.len().div_ceil(columns);
    let tray = Rect::from_size(
        area.left + 14.,
        area.bottom - 18. - rows as f32 * 36.,
        controls.len().min(columns) as f32 * 38. + 8.,
        rows as f32 * 36. + 8.,
    );
    list.rounded_rect(tray, 10., p.surface_elevated);
    list.rounded_border(tray, 10., p.border);
    for (i, (icon, label, hit)) in controls.into_iter().enumerate() {
        let r = Rect::from_size(
            tray.left + 4. + (i % columns) as f32 * 38.,
            tray.top + 4. + (i / columns) as f32 * 36.,
            36.,
            32.,
        );
        if hit == Hit::ResetZoom {
            button(
                list,
                s,
                r,
                &format!("{}%", (s.zoom * 100.).round()),
                hit,
                p,
                false,
            );
            s.tooltips.push((r, label.into()));
        } else {
            super::chrome::icon_button(list, s, r, icon, label, hit, p);
        }
    }
    s.map_rect = Rect::ZERO;
    if s.minimap && area.width() > 360. && area.height() > 240. {
        minimap(list, s, p);
    }
    list.pop_clip();
}
fn port(list: &mut DrawList, s: &mut State, center: (f32, f32), hit: Hit, color: u32, p: &Palette) {
    let radius = if s.hover.as_ref() == Some(&hit) {
        (7. * s.zoom).clamp(4., 8.)
    } else {
        (5. * s.zoom).clamp(3., 6.)
    };
    let r = Rect::from_size(
        center.0 - radius,
        center.1 - radius,
        radius * 2.,
        radius * 2.,
    );
    list.rounded_rect(r, radius, p.surface);
    list.rounded_border(r, radius, color);
    let hit_radius = (9. * s.zoom).clamp(5., 12.);
    s.hits.push((
        Rect::from_size(
            center.0 - hit_radius,
            center.1 - hit_radius,
            hit_radius * 2.,
            hit_radius * 2.,
        )
        .intersect(&s.canvas),
        hit,
    ));
}
impl State {
    pub fn has_overlaps(&self) -> bool {
        let Some(g) = self.graph() else {
            return false;
        };
        g.nodes.iter().enumerate().any(|(i, a)| {
            g.nodes.iter().skip(i + 1).any(|b| {
                (a.position.x - b.position.x).abs() < 316.
                    && a.position.y < b.position.y + node_height(b) + 16.
                    && b.position.y < a.position.y + node_height(a) + 16.
            })
        })
    }
    pub fn bounds(&self) -> Option<Rect> {
        let g = self.graph()?;
        let first = g.nodes.first()?;
        let mut r = Rect::from_size(first.position.x, first.position.y, 292., node_height(first));
        for n in &g.nodes {
            r.left = r.left.min(n.position.x);
            r.top = r.top.min(n.position.y);
            r.right = r.right.max(n.position.x + 292.);
            r.bottom = r.bottom.max(n.position.y + node_height(n));
        }
        Some(r)
    }
    pub fn map_transform(&self) -> Option<(Rect, f32)> {
        let b = self.bounds()?;
        Some((
            b,
            ((self.map_rect.width() - 16.) / b.width())
                .min((self.map_rect.height() - 16.) / b.height()),
        ))
    }
    pub fn navigate_map(&mut self, x: f32, y: f32) {
        if let Some((b, z)) = self.map_transform() {
            self.pan = (
                self.canvas.width() / 2. - (b.left + (x - self.map_rect.left - 8.) / z) * self.zoom,
                self.canvas.height() / 2. - (b.top + (y - self.map_rect.top - 8.) / z) * self.zoom,
            );
        }
    }
    pub fn auto_layout(&mut self) {
        if self.run.is_some() {
            return;
        }
        let Some(g) = self.draft.as_ref() else {
            return;
        };
        let mut ranks = vec![0usize; g.nodes.len()];
        // 按 Kahn 算法排序，让各分支分列显示；发现循环依赖时拒绝修改，不留下部分结果。
        let mut degree: Vec<usize> = g
            .nodes
            .iter()
            .map(|n| g.edges.iter().filter(|e| e.target == n.id).count())
            .collect();
        let mut queue: std::collections::VecDeque<usize> = degree
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut seen = 0;
        while let Some(i) = queue.pop_front() {
            seen += 1;
            for e in g.edges.iter().filter(|e| e.source == g.nodes[i].id) {
                if let Some(j) = g.nodes.iter().position(|n| n.id == e.target) {
                    ranks[j] = ranks[j].max(ranks[i] + 1);
                    degree[j] = degree[j].saturating_sub(1);
                    if degree[j] == 0 {
                        queue.push_back(j);
                    }
                }
            }
        }
        if seen != g.nodes.len() {
            self.error = "存在循环连线，请先移除循环再自动布局".into();
            return;
        }
        self.checkpoint();
        let mut rows = std::collections::HashMap::<usize, f32>::new();
        for (n, rank) in self.draft.as_mut().unwrap().nodes.iter_mut().zip(ranks) {
            let row = rows.entry(rank).or_default();
            n.position.x = 64. + rank as f32 * 388.;
            n.position.y = 64. + *row;
            *row += node_height(n) + 48.;
        }
        self.fit();
    }
    pub fn duplicate_node(&mut self) {
        if self.run.is_some() {
            return;
        }
        let Some(mut n) = self
            .selected_node
            .and_then(|i| self.draft.as_ref()?.nodes.get(i))
            .cloned()
        else {
            return;
        };
        if self.draft.as_ref().unwrap().nodes.len() >= 100 {
            self.error = "最多 100 个节点".into();
            return;
        }
        self.checkpoint();
        n.id = mochi_core::workflows::new_id(&n.kind);
        n.label.push_str(" 副本");
        n.position.x += 32.;
        n.position.y += 48.;
        let g = self.draft.as_mut().unwrap();
        g.nodes.push(n);
        let i = g.nodes.len() - 1;
        self.select_node(i);
    }
}
fn minimap(list: &mut DrawList, s: &mut State, p: &Palette) {
    s.map_rect = Rect::from_size(s.canvas.right - 188., s.canvas.bottom - 164., 176., 108.);
    let r = s.map_rect;
    list.rounded_rect(r, 8., p.surface_elevated);
    list.rounded_border(r, 8., p.border);
    list.push_clip(r);
    if let Some((b, z)) = s.map_transform() {
        if let Some(g) = s.graph() {
            for n in &g.nodes {
                list.rounded_rect(
                    Rect::from_size(
                        r.left + 8. + (n.position.x - b.left) * z,
                        r.top + 8. + (n.position.y - b.top) * z,
                        292. * z,
                        node_height(n) * z,
                    ),
                    2.,
                    category(&n.kind).1,
                );
            }
        }
        let view = Rect::from_size(
            r.left + 8. + (-s.pan.0 / s.zoom - b.left) * z,
            r.top + 8. + (-s.pan.1 / s.zoom - b.top) * z,
            s.canvas.width() / s.zoom * z,
            s.canvas.height() / s.zoom * z,
        );
        list.rounded_border(view, 1., p.accent);
    }
    list.pop_clip();
    s.hits.push((r, Hit::Map));
}
