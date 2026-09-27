//! 读取、校验和保存数据表视图的展示配置。
use super::*;

pub fn visible_field_indices(table: &BaseTable, view: &BaseView) -> Vec<usize> {
    let hidden: HashSet<&str> = view.hidden_field_ids.iter().map(String::as_str).collect();
    table
        .fields
        .iter()
        .enumerate()
        .filter_map(|(i, field)| (!hidden.contains(field.id.as_str())).then_some(i))
        .collect()
}

pub fn validate_view_presentation(view: &Value, path: &str) -> Result<()> {
    for (key, max) in [("frozenRows", 1_048_575), ("frozenColumns", 16_384)] {
        if let Some(value) = view.get(key) {
            if !value.as_u64().is_some_and(|n| n <= max) {
                bail!("{path}.{key} 必须为 0 到 {max} 的整数")
            }
        }
    }
    if let Some(value) = view.get("hiddenFieldIds") {
        let mut ids = HashSet::new();
        for id in array(value, &format!("{path}.hiddenFieldIds"))? {
            add_id(id, &mut ids, &format!("{path}.hiddenFieldIds"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_roundtrip_without_affecting_old_files() {
        let mut document = create_base_document();
        let old = serialize_base_document(&document).unwrap();
        assert!(!old.contains("frozenRows"));
        assert!(!old.contains("hiddenFieldIds"));
        let table = &mut document.tables[0];
        table
            .fields
            .push(create_base_field(FieldType::Number, "count"));
        table.views[0].hidden_field_ids = vec![table.fields[1].id.clone()];
        table.views[0].frozen_rows = 2;
        table.views[0].frozen_columns = 1;
        let parsed = parse_base_document(&serialize_base_document(&document).unwrap()).unwrap();
        assert_eq!(parsed, document);
        assert_eq!(
            visible_field_indices(&parsed.tables[0], &parsed.tables[0].views[0]),
            vec![0]
        );
    }

    #[test]
    fn reject_invalid_presentation_settings() {
        for value in [
            json!({"frozenRows":-1}),
            json!({"frozenColumns":1.5}),
            json!({"frozenRows":1048576}),
            json!({"hiddenFieldIds":["x","x"]}),
            json!({"hiddenFieldIds":[3]}),
        ] {
            assert!(validate_view_presentation(&value, "view").is_err());
        }
        assert!(validate_view_presentation(&json!({}), "view").is_ok());
    }
}
