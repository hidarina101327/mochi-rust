//! 处理工作流画布的拖动、缩放和视图定位。
use super::*;

impl State {
    pub fn begin(&mut self, hit: Option<&Hit>, x: f32, y: f32) {
        match hit {
            Some(Hit::OutputVariable(i, output)) if self.run.is_none() => {
                self.drag = Some(Drag::Variable(*i, *output));
                self.pointer = (x, y);
            }
            Some(Hit::Binding(i, key)) => {
                self.selected_binding = Some((*i, key.clone()));
                self.selected_node = None;
                self.selected_edge = None;
                self.group.clear();
                self.editor = None;
            }
            Some(Hit::Kind(i)) => {
                self.drag = Some(Drag::Template(*i, x, y));
                self.searching = false;
            }
            Some(Hit::Resize) => {
                self.drag = Some(Drag::Resize(x, self.panel_width));
            }
            Some(Hit::Map) => self.navigate_map(x, y),
            Some(Hit::Node(i)) => {
                if self.shift {
                    if let Some(index) = self.selected_node.take() {
                        self.group.insert(index);
                    }
                    if !self.group.remove(i) {
                        self.group.insert(*i);
                    }
                    self.editor = None;
                    return;
                }
                let group = if self.group.contains(i) {
                    self.group.clone()
                } else {
                    Default::default()
                };
                self.select_node(*i);
                self.group = group;
                if self.run.is_none() {
                    if let Some(n) = self.draft.as_ref().and_then(|d| d.nodes.get(*i)) {
                        let r = self.node_rect(n);
                        self.drag = Some(Drag::Node(*i, x - r.left, y - r.top, false));
                    }
                }
            }
            Some(Hit::Port(i, branch)) if self.run.is_none() => {
                self.drag = Some(Drag::Wire(*i, *branch));
                self.pointer = (x, y);
            }
            Some(Hit::Edge(i)) => {
                self.selected_binding = None;
                self.group.clear();
                self.selected_edge = Some(*i);
                self.selected_node = None;
                self.editor = None;
            }
            None if self.canvas.contains(x, y) => {
                if self.shift {
                    self.drag = Some(Drag::Box(x, y));
                    self.pointer = (x, y);
                    return;
                }
                self.group.clear();
                if self.editor.is_some() && self.field.text() == self.field_before {
                    self.editor = None;
                    self.selected_node = None;
                    self.settings_open = false;
                }
                self.drag = Some(Drag::Pan(x, y, self.pan.0, self.pan.1));
            }
            _ => {}
        }
    }
    pub fn motion(&mut self, x: f32, y: f32) -> bool {
        self.pointer = (x, y);
        let hover = self.hit(x, y);
        let changed = hover != self.hover;
        self.hover = hover;
        match self.drag {
            Some(Drag::Box(..)) => true,
            Some(Drag::Template(..)) => true,
            Some(Drag::Resize(start, width)) => {
                self.panel_width = (width + start - x).clamp(260., 600.);
                true
            }
            Some(Drag::Node(i, dx, dy, moved)) => {
                if !moved {
                    let Some(n) = self.draft.as_ref().and_then(|d| d.nodes.get(i)) else {
                        return changed;
                    };
                    let r = self.node_rect(n);
                    if (x - dx - r.left).abs() < 2. && (y - dy - r.top).abs() < 2. {
                        return changed;
                    }
                    self.checkpoint();
                    self.drag = Some(Drag::Node(i, dx, dy, true));
                }
                let mut delta = (0., 0.);
                if let Some(n) = self.draft.as_mut().and_then(|d| d.nodes.get_mut(i)) {
                    let before = (n.position.x, n.position.y);
                    n.position.x = (((x - dx - self.canvas.left - self.pan.0) / self.zoom) / 16.)
                        .round()
                        * 16.;
                    n.position.x = n.position.x.clamp(-100000., 100000.);
                    n.position.y =
                        (((y - dy - self.canvas.top - self.pan.1) / self.zoom) / 16.).round() * 16.;
                    n.position.y = n.position.y.clamp(-100000., 100000.);
                    delta = (n.position.x - before.0, n.position.y - before.1);
                }
                if let Some(g) = &mut self.draft {
                    for j in &self.group {
                        if *j != i {
                            if let Some(n) = g.nodes.get_mut(*j) {
                                n.position.x = (n.position.x + delta.0).clamp(-100000., 100000.);
                                n.position.y = (n.position.y + delta.1).clamp(-100000., 100000.);
                            }
                        }
                    }
                }
                true
            }
            Some(Drag::Pan(sx, sy, px, py)) => {
                self.pan = (px + x - sx, py + y - sy);
                true
            }
            Some(Drag::Wire(..) | Drag::Variable(..)) => true,
            None => changed,
        }
    }
    pub fn release(&mut self, x: f32, y: f32) {
        if let Some(Drag::Variable(source, output)) = self.drag {
            self.drag = None;
            if let Some(Hit::InputVariable(target, key)) = self.hit(x, y) {
                if let Some(output) = self
                    .graph()
                    .and_then(|g| g.nodes.get(source))
                    .and_then(|n| output_keys(n).get(output).cloned())
                {
                    if let Err(error) = self.connect_variable(source, target, &key, &output) {
                        self.error = error;
                    }
                }
            }
            return;
        }
        if let Some(Drag::Box(sx, sy)) = self.drag {
            let r = Rect::new(sx.min(x), sy.min(y), sx.max(x), sy.max(y));
            self.group = self
                .graph()
                .map(|g| {
                    g.nodes
                        .iter()
                        .enumerate()
                        .filter_map(|(i, n)| {
                            (!self.node_rect(n).intersect(&r).is_empty()).then_some(i)
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.selected_node = None;
            self.editor = None;
            self.drag = None;
            return;
        }
        if let Some(Drag::Template(i, sx, sy)) = self.drag {
            self.drag = None;
            let moved = (x - sx).abs() + (y - sy).abs() > 6.;
            if !moved
                || self.canvas.contains(x, y)
                    && self.hit(x, y).is_none_or(|h| {
                        !matches!(h, Hit::Kind(_) | Hit::PaletteSearch | Hit::Parameters)
                    })
            {
                if let Some(kind) = mochi_core::workflows::catalog::KINDS.get(i) {
                    let position = (
                        (x - self.canvas.left - self.pan.0) / self.zoom,
                        (y - self.canvas.top - self.pan.1) / self.zoom,
                    );
                    self.add_node(kind);
                    if moved {
                        if let Some(n) = self
                            .selected_node
                            .and_then(|i| self.draft.as_mut()?.nodes.get_mut(i))
                        {
                            n.position.x = (position.0 / 16.).round() * 16.;
                            n.position.y = (position.1 / 16.).round() * 16.;
                        }
                    }
                }
            }
            return;
        }
        if let Some(Drag::Wire(source, branch)) = self.drag.take() {
            if let Some(Hit::Input(target)) = self.hit(x, y) {
                if source == target {
                    return;
                }
                let g = self.draft.as_ref().unwrap();
                let from = &g.nodes[source].id;
                let to = &g.nodes[target].id;
                let mut pending = vec![to.as_str()];
                let mut visited = std::collections::HashSet::new();
                while let Some(id) = pending.pop() {
                    if id == from {
                        self.error = "不能连接形成循环".into();
                        return;
                    }
                    if visited.insert(id) {
                        for e in g.edges.iter().filter(|e| e.source == id) {
                            pending.push(&e.target);
                        }
                    }
                }
                if g.edges.iter().any(|e| {
                    e.source == *from
                        && e.target == *to
                        && e.source_handle.as_deref()
                            == branch.map(|v| if v { "true" } else { "false" })
                }) {
                    return;
                }
                self.checkpoint();
                if let Some(d) = &mut self.draft {
                    let from = d.nodes[source].id.clone();
                    let to = d.nodes[target].id.clone();
                    if !d.edges.iter().any(|e| {
                        e.source == from
                            && e.target == to
                            && e.source_handle.as_deref()
                                == branch.map(|v| if v { "true" } else { "false" })
                    }) {
                        d.edges.push(Edge {
                            id: mochi_core::workflows::new_id("edge"),
                            source: from,
                            target: to,
                            source_handle: branch.map(|v| {
                                if v {
                                    "true".into()
                                } else {
                                    "false".into()
                                }
                            }),
                        });
                    }
                }
            }
        }
    }
    pub fn zoom_at(&mut self, x: f32, y: f32, factor: f32) {
        let previous = self.zoom;
        self.zoom = (previous * factor).clamp(0.15, 1.6);
        self.pan.0 =
            x - self.canvas.left - (x - self.canvas.left - self.pan.0) * self.zoom / previous;
        self.pan.1 =
            y - self.canvas.top - (y - self.canvas.top - self.pan.1) * self.zoom / previous;
    }
    pub fn fit(&mut self) {
        let Some(g) = self.graph() else {
            return;
        };
        if g.nodes.is_empty() {
            return;
        }
        let left = g
            .nodes
            .iter()
            .map(|n| n.position.x)
            .fold(f32::INFINITY, f32::min);
        let right = g
            .nodes
            .iter()
            .map(|n| n.position.x + 292.)
            .fold(f32::NEG_INFINITY, f32::max);
        let top = g
            .nodes
            .iter()
            .map(|n| n.position.y)
            .fold(f32::INFINITY, f32::min);
        let bottom = g
            .nodes
            .iter()
            .map(|n| n.position.y + node_height(n))
            .fold(f32::NEG_INFINITY, f32::max);
        self.zoom = ((self.canvas.width() - 64.) / (right - left).max(1.))
            .min((self.canvas.height() - 64.) / (bottom - top).max(1.))
            .clamp(0.15, 1.6);
        self.pan = (
            (self.canvas.width() - (right - left) * self.zoom) * 0.5 - left * self.zoom,
            (self.canvas.height() - (bottom - top) * self.zoom) * 0.5 - top * self.zoom,
        );
        // 窗口较窄时，优先保证选中节点清晰可读；其余节点仍可通过
        // 平移画布或手动缩小来查看。
        if self.canvas.width() < 400. && self.zoom < 0.55 {
            self.zoom = 0.55;
            if let Some(n) = self
                .selected_node
                .and_then(|i| self.graph()?.nodes.get(i))
                .cloned()
            {
                self.pan = (
                    self.canvas.width() * 0.5 - (n.position.x + 146.) * self.zoom,
                    self.canvas.height() * 0.5 - (n.position.y + node_height(&n) * 0.5) * self.zoom,
                );
            } else {
                self.pan = (32. - left * self.zoom, 48. - top * self.zoom);
            }
        }
    }
}
