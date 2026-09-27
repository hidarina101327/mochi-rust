//! 计算工作流节点输入输出端口的位置和引用标签。
use super::*;
use serde_json::Value;

pub fn input_keys(n: &Node) -> Vec<String> {
    n.inputs
        .as_object()
        .map(|v| v.keys().cloned().collect())
        .unwrap_or_default()
}
pub fn output_keys(n: &Node) -> Vec<String> {
    let fields: &[&str] = if n.for_each.is_some() {
        &["output", "output.items"]
    } else {
        match n.kind.as_str() {
            "http" => match n.config["extract"].as_str().unwrap_or("text") {
                "raw" => &["output", "output.body", "output.status", "output.url"],
                "json" => &["output", "output.data", "output.status", "output.url"],
                _ => &["output", "output.text", "output.status", "output.url"],
            },
            "ai" => &["output", "output.text", "output.data"],
            "file_read" => &["output", "output.text", "output.path"],
            "folder_list" => &["output", "output.items", "output.count"],
            "script" => &["output", "output.data", "output.stdout"],
            "condition" => &["output", "output.branch"],
            _ => &["output"],
        }
    };
    let mut keys: Vec<String> = fields.iter().map(|s| (*s).into()).collect();
    if n.kind == "json" && n.config["operation"].as_str().unwrap_or("identity") == "identity" {
        keys.extend(input_keys(n).into_iter().map(|key| format!("output.{key}")));
    }
    keys
}
pub fn node_height(n: &Node) -> f32 {
    let rows = input_keys(n).len().max(output_keys(n).len());
    158. + rows as f32 * 26.
}

impl State {
    pub fn input_center(&self, n: &Node, key: &str) -> (f32, f32) {
        let r = self.node_rect(n);
        let row = input_keys(n).iter().position(|k| k == key).unwrap_or(0);
        (r.left, r.top + (132. + row as f32 * 26.) * self.zoom)
    }
    pub fn output_center(&self, n: &Node, key: &str) -> (f32, f32) {
        let r = self.node_rect(n);
        let row = output_keys(n).iter().position(|k| k == key).unwrap_or(0);
        (r.right, r.top + (132. + row as f32 * 26.) * self.zoom)
    }
    pub fn reference_label(&self, reference: &str) -> String {
        if let Some(rest) = reference.strip_prefix("$nodes.") {
            if let Some((id, key)) = rest.split_once('.') {
                if let Some(n) = self
                    .graph()
                    .and_then(|g| g.nodes.iter().find(|n| n.id == id))
                {
                    return format!("{} — {}", n.label, key);
                }
            }
        }
        if reference == "$input" {
            return "工作流输入 — 全部".into();
        }
        if let Some(key) = reference.strip_prefix("$input.") {
            return format!("工作流输入 — {key}");
        }
        if reference == "$run.date" {
            return "运行信息 — 日期".into();
        }
        reference.into()
    }
    pub fn variable_options(&self) -> Vec<(String, String)> {
        let mut refs = vec!["$input".to_owned(), "$run.date".into()];
        if let Some(g) = self.graph() {
            let mut keys = std::collections::BTreeSet::new();
            for object in [
                g.defaults.as_object(),
                g.input_schema.get("properties").and_then(Value::as_object),
            ]
            .into_iter()
            .flatten()
            {
                keys.extend(object.keys().cloned());
            }
            refs.extend(keys.into_iter().map(|key| format!("$input.{key}")));
            if let Some(node) = self.selected_node.and_then(|i| g.nodes.get(i)) {
                let mut pending = vec![node.id.clone()];
                let mut seen = std::collections::HashSet::new();
                while let Some(id) = pending.pop() {
                    for e in g.edges.iter().filter(|e| e.target == id) {
                        if seen.insert(e.source.clone()) {
                            pending.push(e.source.clone());
                            if let Some(n) = g.nodes.iter().find(|n| n.id == e.source) {
                                refs.extend(
                                    output_keys(n)
                                        .into_iter()
                                        .map(|key| format!("$nodes.{}.{key}", n.id)),
                                );
                            }
                        }
                    }
                }
            }
        }
        refs.into_iter()
            .map(|r| (self.reference_label(&r), r))
            .collect()
    }
    /// 数据连线会保存为引擎实际使用的输入引用。
    /// 执行连线负责确定顺序，旧文档无需迁移。
    pub fn bindings(&self) -> Vec<(usize, String, usize, String)> {
        let Some(g) = self.graph() else {
            return vec![];
        };
        let mut result = vec![];
        for (target, n) in g.nodes.iter().enumerate() {
            for (key, value) in n.inputs.as_object().into_iter().flatten() {
                let Some(rest) = value.as_str().and_then(|s| s.strip_prefix("$nodes.")) else {
                    continue;
                };
                let Some((id, output)) = rest.split_once('.') else {
                    continue;
                };
                if let Some(source) = g.nodes.iter().position(|n| n.id == id) {
                    result.push((target, key.clone(), source, output.into()));
                }
            }
        }
        result
    }
    pub fn connect_variable(
        &mut self,
        source: usize,
        target: usize,
        input: &str,
        output: &str,
    ) -> Result<(), String> {
        if self.run.is_some() {
            return Err("运行快照只读".into());
        }
        self.apply_node()?;
        let g = self.draft.as_ref().ok_or("工作流不存在")?;
        let from = g.nodes.get(source).ok_or("源节点不存在")?;
        let to = g.nodes.get(target).ok_or("目标节点不存在")?;
        if source == target || from.kind == "end" || to.kind == "start" {
            return Err("不能连接这些端口".into());
        }
        if to.inputs.get(input).is_none() {
            return Err("输入变量不存在".into());
        }
        let mut pending = vec![to.id.as_str()];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if id == from.id {
                return Err("不能连接形成循环".into());
            }
            if seen.insert(id) {
                pending.extend(
                    g.edges
                        .iter()
                        .filter(|e| e.source == id)
                        .map(|e| e.target.as_str()),
                );
            }
        }
        let from_id = from.id.clone();
        let to_id = to.id.clone();
        let branch = (from.kind == "condition").then(|| "true".to_string());
        let needs_edge = !g
            .edges
            .iter()
            .any(|e| e.source == from_id && e.target == to_id);
        if needs_edge && from.kind == "condition" {
            return Err("请先连接条件节点的是 / 否分支，再绑定该分支的输入变量".into());
        }
        if needs_edge && g.edges.len() >= 300 {
            return Err("最多 300 条执行连线".into());
        }
        self.checkpoint();
        let g = self.draft.as_mut().unwrap();
        if needs_edge {
            g.edges.push(Edge {
                id: mochi_core::workflows::new_id("edge"),
                source: from_id.clone(),
                target: to_id,
                source_handle: branch,
            });
        }
        g.nodes[target].inputs[input] = Value::String(format!("$nodes.{from_id}.{output}"));
        self.select_node(target);
        self.status = format!("已连接到输入 {input}");
        Ok(())
    }
    pub fn remove_binding(&mut self, target: usize, key: &str) {
        if self.run.is_some() {
            return;
        }
        self.checkpoint();
        if let Some(n) = self.draft.as_mut().and_then(|g| g.nodes.get_mut(target)) {
            n.inputs[key] = Value::String(String::new());
        }
        self.selected_binding = None;
        self.editor = None;
        self.status = "已断开变量连线，可撤销".into();
    }
}
