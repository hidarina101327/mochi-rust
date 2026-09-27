//! 纯函数、带校验的编辑，供控制台事务及其测试使用。
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
pub fn base_batch(
    document: &crate::base::BaseDocument,
    ops: &[Value],
) -> Result<crate::base::BaseDocument> {
    use crate::base::*;
    ensure!(!ops.is_empty() && ops.len() <= 64, "需要 1–64 项操作");
    let mut next = document.clone();
    for o in ops {
        let op = o["op"].as_str().context("缺少 op")?;
        let id = || o["id"].as_str().context("缺少 id");
        if op == "table_create" {
            next.tables
                .push(create_base_table(o["name"].as_str().context("缺少 name")?));
            continue;
        }
        if op == "table_delete" {
            let id = id()?;
            ensure!(next.tables.iter().any(|t| t.id == id), "表不存在");
            next.tables.retain(|t| t.id != id);
            if next.active_table_id.as_deref() == Some(id) {
                next.active_table_id = next.tables.first().map(|t| t.id.clone());
            }
            continue;
        }
        let t = next
            .tables
            .iter_mut()
            .find(|t| Some(t.id.as_str()) == o["tableId"].as_str())
            .context("tableId 不存在")?;
        match op {
            "table_rename" => t.name = o["name"].as_str().context("缺少 name")?.into(),
            "record_update" => {
                let r = t
                    .records
                    .iter_mut()
                    .find(|r| r.id == id().unwrap_or(""))
                    .context("记录不存在")?;
                r.values.extend(
                    o["values"]
                        .as_object()
                        .context("values 必须是对象")?
                        .clone(),
                );
            }
            "record_delete" => {
                let id = id()?;
                ensure!(t.records.iter().any(|r| r.id == id), "记录不存在");
                t.records.retain(|r| r.id != id);
            }
            "field_save" => {
                let f: BaseField = serde_json::from_value(o["field"].clone())?;
                if let Some(existing) = t.fields.iter_mut().find(|v| v.id == f.id) {
                    *existing = f;
                } else {
                    t.fields.push(f);
                }
            }
            "field_delete" => {
                let id = id()?;
                ensure!(t.fields.iter().any(|f| f.id == id), "字段不存在");
                t.fields.retain(|f| f.id != id);
                for r in &mut t.records {
                    r.values.remove(id);
                }
                for v in &mut t.views {
                    v.hidden_field_ids.retain(|f| f != id);
                    v.sorts.retain(|s| s.field_id != id);
                    v.filters.retain(|f| f.field_id != id);
                    if v.group_by.as_deref() == Some(id) {
                        v.group_by = None;
                    }
                }
            }
            "view_save" => {
                let v: BaseView = serde_json::from_value(o["view"].clone())?;
                if let Some(existing) = t.views.iter_mut().find(|e| e.id == v.id) {
                    *existing = v;
                } else {
                    t.views.push(v);
                }
            }
            "view_delete" => {
                let id = id()?;
                ensure!(t.views.iter().any(|v| v.id == id), "视图不存在");
                t.views.retain(|v| v.id != id);
                if t.active_view_id.as_deref() == Some(id) {
                    t.active_view_id = t.views.first().map(|v| v.id.clone());
                }
            }
            _ => bail!("未知表格操作 {op}"),
        }
    }
    validate_base_document(&next)?;
    Ok(next)
}
pub fn canvas_batch(
    doc: &crate::canvas::CanvasDocument,
    args: &Value,
) -> Result<crate::canvas::CanvasDocument> {
    ensure!(
        args["revision"].as_str() == Some(crate::canvas::drawing::revision(doc).as_str()),
        "画布版本已变化，请重读"
    );
    let ops = args["operations"].as_array().context("缺少 operations")?;
    ensure!(!ops.is_empty() && ops.len() <= 64, "需要 1–64 项操作");
    let mut value = serde_json::to_value(doc)?;
    for o in ops {
        let kind = o["kind"].as_str().context("缺少 kind")?;
        if kind == "viewport" {
            value["viewport"] = o["value"].clone();
            continue;
        }
        ensure!(
            matches!(kind, "cards" | "texts" | "strokes"),
            "未知对象类型"
        );
        let items = value[kind].as_array_mut().context("对象集合缺失")?;
        let id = o["id"].as_str().context("缺少 id")?;
        let at = items
            .iter()
            .position(|i| i["id"] == id)
            .context("对象不存在")?;
        match o["op"].as_str() {
            Some("delete") => {
                items.remove(at);
            }
            Some("update") => {
                let changes = o["changes"].as_object().context("缺少 changes")?;
                ensure!(!changes.contains_key("id"), "不能修改对象 ID");
                for (k, v) in changes {
                    ensure!(items[at].get(k).is_some(), "未知属性 {k}");
                    items[at][k] = v.clone();
                }
            }
            _ => bail!("op 需要 update/delete"),
        }
    }
    let next: crate::canvas::CanvasDocument = serde_json::from_value(value)?;
    next.validate()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn base_batch_validates_all_operations_before_returning_changes() {
        let mut doc = crate::base::create_base_document();
        let table = &mut doc.tables[0];
        let tid = table.id.clone();
        let fid = table.fields[0].id.clone();
        table.records.push(crate::base::BaseRecord {
            id: "r1".into(),
            values: serde_json::from_value(json!({fid.clone():"before"})).unwrap(),
            ..Default::default()
        });
        let before = doc.clone();
        let ops = vec![
            json!({"op":"record_update","tableId":tid,"id":"r1","values":{fid.clone():"after"}}),
            json!({"op":"record_delete","tableId":tid,"id":"missing"}),
        ];
        assert!(base_batch(&doc, &ops).is_err());
        assert_eq!(doc, before);
        let next = base_batch(&doc, &ops[..1]).unwrap();
        assert_eq!(next.tables[0].records[0].values[&fid], "after");
        assert!(base_batch(
            &doc,
            &[json!({"op":"record_update","tableId":tid,"id":"r1","values":{"unknown":1}})]
        )
        .is_err());
    }
    #[test]
    fn deleting_field_cleans_view_dependencies() {
        let mut doc = crate::base::create_base_document();
        let t = &mut doc.tables[0];
        let f = crate::base::BaseField {
            id: "extra".into(),
            name: "Extra".into(),
            ..Default::default()
        };
        t.fields.push(f);
        t.views[0].sorts.push(crate::base::BaseSort {
            field_id: "extra".into(),
            ..Default::default()
        });
        t.views[0].hidden_field_ids.push("extra".into());
        let next = base_batch(
            &doc,
            &[json!({"op":"field_delete","tableId":doc.tables[0].id,"id":"extra"})],
        )
        .unwrap();
        assert!(next.tables[0].views[0].sorts.is_empty());
        assert!(next.tables[0].views[0].hidden_field_ids.is_empty());
        assert_eq!(doc.tables[0].views[0].sorts.len(), 1);
    }
    #[test]
    fn canvas_pure_delete_viewport_and_stale_revision() {
        let mut doc = crate::canvas::CanvasDocument::empty();
        doc.add_pdf_annotation("library/paper.pdf", "a1").unwrap();
        let id = doc.cards[0].id.clone();
        let rev = crate::canvas::drawing::revision(&doc);
        let args = json!({"revision":rev,"operations":[{"kind":"cards","op":"delete","id":id},{"kind":"viewport","value":{"x":20,"y":30,"zoom":2}}]});
        let next = canvas_batch(&doc, &args).unwrap();
        assert!(next.cards.is_empty());
        assert_eq!(next.viewport.zoom, 2.0);
        assert_eq!(doc.cards.len(), 1);
        assert!(canvas_batch(&next, &args).is_err());
        let bad = json!({"revision":rev,"operations":[{"kind":"cards","id":id,"op":"update","changes":{"width":-1}}]});
        assert!(canvas_batch(&doc, &bad).is_err());
    }
}
