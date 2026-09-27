//! 可搬移引用的维护逻辑，与 `shared/base-reference-paths.ts` 对齐。
use crate::base::{parse_base_reference, BaseDocument, FieldType, FilterOperator};
use crate::mochi_url::{form_decode, form_encode};
use serde_json::Value;
use std::collections::HashSet;

pub struct BaseReferenceRename<'a> {
    pub workspace_path: &'a str,
    pub old_path: &'a str,
    pub new_path: &'a str,
}

fn normalize(value: &str) -> String {
    value
        .replace('\\', "/")
        .replace("/./", "/")
        .trim_end_matches('/')
        .to_owned()
}

fn windows(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with("//")
        || (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1..3] == *b":/")
}

fn absolute(value: &str) -> bool {
    value.starts_with('/') || windows(value)
}

/// 返回工作区重命名后的路径。资源 URL 和专门的块 URL 共用它：
/// 文档路径跟着改名走，块的稳定 ID 原样保留。
fn renamed_path(reference_path: &str, rename: &BaseReferenceRename<'_>) -> Option<String> {
    let root = normalize(rename.workspace_path);
    let resolve = |target: &str| {
        let target = normalize(target);
        if absolute(&target) {
            target
        } else {
            normalize(&format!("{root}/{target}"))
        }
    };
    let old_path = resolve(rename.old_path);
    let new_path = resolve(rename.new_path);
    let target = resolve(reference_path);
    let insensitive = windows(&root) || windows(&old_path);
    let compare = |path: &str| {
        if insensitive {
            path.to_lowercase()
        } else {
            path.to_owned()
        }
    };
    let target_compare = compare(&target);
    let old_compare = compare(&old_path);
    if target_compare != old_compare && !target_compare.starts_with(&format!("{old_compare}/")) {
        return None;
    }
    let suffix = target.get(old_path.len()..)?;
    let moved = format!("{new_path}{suffix}");
    let reference_is_relative = !absolute(reference_path);
    let next_path = if reference_is_relative && compare(&moved) == compare(&root) {
        ".".to_owned()
    } else if reference_is_relative && compare(&moved).starts_with(&format!("{}/", compare(&root)))
    {
        moved.get(root.len() + 1..)?.to_owned()
    } else {
        moved
    };
    Some(next_path)
}

/// 只替换 path 查询参数，ID、标签、视图选项和未来的未知参数一概不动。
/// URL 已由调用方校验过，所以这里明确 path 只有一个。
fn replace_path_query(value: &str, next_path: &str) -> Option<String> {
    let (prefix, query) = value.split_once('?')?;
    let query = query
        .split('&')
        .map(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            if form_decode(key) == "path" {
                format!("path={}", form_encode(next_path))
            } else {
                pair.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    Some(format!("{prefix}?{query}"))
}

/// 只改 path 查询参数，其余参数逐字节保留。
pub fn rename_base_reference(value: &str, rename: &BaseReferenceRename<'_>) -> String {
    // parse_base_reference 刻意不收块 URL——它建模的是旧版
    // mochi://open 资源定位符。块链接先单独处理，再用严格的
    // 对象解析器校验改写结果。
    if let Some(reference) = crate::object_reference::ObjectReference::parse(value)
        .filter(|reference| reference.kind == crate::object_reference::ObjectKind::Block)
    {
        let Some(path) = reference.path.as_deref() else {
            return value.to_owned();
        };
        let Some(next_path) = renamed_path(path, rename) else {
            return value.to_owned();
        };
        if next_path == path {
            return value.to_owned();
        }
        let Some(result) = replace_path_query(value, &next_path) else {
            return value.to_owned();
        };
        return crate::object_reference::ObjectReference::parse(&result)
            .filter(|reference| reference.kind == crate::object_reference::ObjectKind::Block)
            .map_or_else(|| value.to_owned(), |_| result);
    }

    let Some(reference) = parse_base_reference(value) else {
        return value.to_owned();
    };
    let Some(next_path) = renamed_path(&reference.path, rename) else {
        return value.to_owned();
    };
    if next_path == reference.path {
        return value.to_owned();
    }
    let Some(result) = replace_path_query(value, &next_path) else {
        return value.to_owned();
    };
    // 扩展名改了之后，记录 URL 仍必须指向 .mcb。
    if parse_base_reference(&result).is_some() {
        result
    } else {
        value.to_owned()
    }
}

/// 改写引用类型单元格及其过滤条件，不替换 ID 和扩展名。
pub fn rename_base_document_references(
    document: &mut BaseDocument,
    rename: &BaseReferenceRename<'_>,
) -> usize {
    fn rewrite(value: &mut Value, rename: &BaseReferenceRename<'_>) -> usize {
        let Some(values) = value.as_array_mut() else {
            return 0;
        };
        let mut count = 0;
        for value in values.iter_mut() {
            let Some(link) = value.as_str() else { continue };
            let next = rename_base_reference(link, rename);
            if next != link {
                *value = Value::String(next);
                count += 1;
            }
        }
        if count > 0 {
            let mut seen = HashSet::new();
            values.retain(|value| {
                value
                    .as_str()
                    .is_none_or(|link| seen.insert(link.to_owned()))
            });
        }
        count
    }
    let mut count = 0;
    for table in &mut document.tables {
        let fields = table
            .fields
            .iter()
            .filter(|field| matches!(field.field_type, FieldType::Reference | FieldType::Document))
            .map(|field| field.id.clone())
            .collect::<HashSet<_>>();
        for record in &mut table.records {
            for (id, value) in &mut record.values {
                if fields.contains(id) {
                    count += rewrite(value, rename);
                }
            }
        }
        for view in &mut table.views {
            for filter in &mut view.filters {
                if fields.contains(&filter.field_id) {
                    if let Some(value) = &mut filter.value {
                        if matches!(
                            filter.operator,
                            FilterOperator::Equals | FilterOperator::NotEquals
                        ) {
                            if let Some(link) = value.as_str() {
                                let next = rename_base_reference(link, rename);
                                if next != link {
                                    *value = Value::String(next);
                                    count += 1;
                                }
                                continue;
                            }
                        }
                        count += rewrite(value, rename);
                    }
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::*;
    use serde_json::json;

    #[test]
    fn base_reference_rename_preserves_encoded_ids_and_unknown_parameters() {
        let rename = BaseReferenceRename {
            workspace_path: "D:/ws",
            old_path: "知识库/旧.mcb",
            new_path: "知识库/新 名称.mcb",
        };
        let old = "MOCHI://open?x=a%2fb&%70ath=%E7%9F%A5%E8%AF%86%E5%BA%93%2F%E6%97%A7.mcb&table=t%2f1&record=r%2B2&field=f&label=%E5%88%AB%E5%90%8D&x=2";
        let next = rename_base_reference(old, &rename);
        assert_eq!(
            parse_base_reference(&next).unwrap().path,
            "知识库/新 名称.mcb"
        );
        assert!(next.starts_with("MOCHI://open?x=a%2fb&path="));
        assert!(next.ends_with("&table=t%2f1&record=r%2B2&field=f&label=%E5%88%AB%E5%90%8D&x=2"));
        assert_eq!(rename_base_reference(&next, &rename), next);
    }

    #[test]
    fn block_reference_rename_moves_path_and_preserves_stable_identity() {
        let rename = BaseReferenceRename {
            workspace_path: "D:/ws",
            old_path: "旧文档",
            new_path: "新文档",
        };
        let old = "MOCHI://block?path=%E6%97%A7%E6%96%87%E6%A1%A3%2Fnote.md&id=block_123e4567-e89b-12d3-a456-426614174000&label=%E5%9D%97%E5%90%8D&view=card&future=x%2Fy";
        let next = rename_base_reference(old, &rename);
        assert_eq!(
            next,
            "MOCHI://block?path=%E6%96%B0%E6%96%87%E6%A1%A3%2Fnote.md&id=block_123e4567-e89b-12d3-a456-426614174000&label=%E5%9D%97%E5%90%8D&view=card&future=x%2Fy"
        );
        let parsed = crate::object_reference::ObjectReference::parse(&next).unwrap();
        assert_eq!(parsed.kind, crate::object_reference::ObjectKind::Block);
        assert_eq!(parsed.path.as_deref(), Some("新文档/note.md"));
        assert_eq!(
            parsed.block_id.as_deref(),
            Some("block_123e4567-e89b-12d3-a456-426614174000")
        );
        assert_eq!(rename_base_reference(&next, &rename), next);
    }

    #[test]
    fn base_reference_rename_observes_folder_boundaries_and_path_case() {
        let rename = BaseReferenceRename {
            workspace_path: "D:\\WS",
            old_path: "D:/ws/Old",
            new_path: "D:/ws/New",
        };
        for (old, expected) in [
            ("old/a.md", "New/a.md"),
            ("D:/WS/OLD/a.md", "D:/ws/New/a.md"),
            ("older/a.md", "older/a.md"),
        ] {
            let link = format!("mochi://open?path={}", form_encode(old));
            assert_eq!(
                parse_base_reference(&rename_base_reference(&link, &rename))
                    .unwrap()
                    .path,
                expected
            );
        }
        let unix = BaseReferenceRename {
            workspace_path: "/ws",
            old_path: "/ws/Old",
            new_path: "/ws/New",
        };
        let link = "mochi://open?path=old%2Fa.md";
        assert_eq!(rename_base_reference(link, &unix), link);
        let outside = BaseReferenceRename {
            workspace_path: "D:/ws",
            old_path: "old",
            new_path: "D:/elsewhere",
        };
        assert_eq!(
            parse_base_reference(&rename_base_reference(
                "mochi://open?path=old%2Fa.md",
                &outside
            ))
            .unwrap()
            .path,
            "D:/elsewhere/a.md"
        );
    }

    #[test]
    fn base_reference_rename_updates_filters_deduplicates_and_preserves_document_extensions() {
        let mut document = create_base_document();
        document.extra.insert("future".into(), json!({"kept":true}));
        let table = &mut document.tables[0];
        let text_id = table.fields[0].id.clone();
        let reference = create_base_field(FieldType::Reference, "关联资料");
        let id = reference.id.clone();
        table.fields.push(reference);
        let old = "mochi://open?path=old%2Fa.md";
        let next = "mochi://open?path=new%2Fa.md";
        table.records.push(BaseRecord {
            id: "stable-record".into(),
            values: [
                (id.clone(), json!([old, next])),
                (text_id.clone(), json!(old)),
            ]
            .into_iter()
            .collect(),
            extra: [("future".into(), json!(42))].into_iter().collect(),
        });
        table.views[0].filters.push(BaseFilter {
            field_id: id.clone(),
            operator: FilterOperator::Contains,
            value: Some(json!([old])),
            extra: [("future".into(), json!("filter"))].into_iter().collect(),
        });
        let original = document.clone();
        let rename = BaseReferenceRename {
            workspace_path: "D:/ws",
            old_path: "old",
            new_path: "new",
        };
        assert_eq!(rename_base_document_references(&mut document, &rename), 2);
        assert_eq!(document.tables[0].records[0].values[&id], json!([next]));
        assert_eq!(document.tables[0].records[0].values[&text_id], json!(old));
        assert_eq!(
            document.tables[0].views[0].filters[0].value,
            Some(json!([next]))
        );
        assert_eq!(document.extra, original.extra);
        assert_eq!(document.tables[0].id, original.tables[0].id);
        assert_eq!(
            document.tables[0].records[0].extra,
            original.tables[0].records[0].extra
        );
        assert_eq!(
            document.tables[0].views[0].filters[0].extra,
            original.tables[0].views[0].filters[0].extra
        );
        serialize_base_document(&document).unwrap();
    }

    #[test]
    fn base_reference_rename_keeps_invalid_or_incompatible_links_intact() {
        let rename = BaseReferenceRename {
            workspace_path: "/ws",
            old_path: "old.mcb",
            new_path: "new.txt",
        };
        for link in [
            "https://example.com/old.mcb",
            "mochi://open?path=old.mcb&table=t&record=r",
            "mochi://open?path=old.mcb&path=other",
            "mochi://open?path=..%2Fold.mcb",
        ] {
            assert_eq!(rename_base_reference(link, &rename), link);
        }
    }

    #[test]
    fn base_reference_rename_updates_scalar_membership_filters_but_keeps_contains_text() {
        let mut document = create_base_document();
        let field = create_base_field(FieldType::Reference, "关联");
        let id = field.id.clone();
        document.tables[0].fields.push(field);
        let old = "mochi://open?path=old%2Fnote.md&label=Note&future=x%2fy";
        let next = "mochi://open?path=new%2Fnote.md&label=Note&future=x%2fy";
        for (operator, value) in [
            (FilterOperator::Equals, old),
            (FilterOperator::NotEquals, old),
            (FilterOperator::Contains, old),
            (FilterOperator::Contains, "old/note.md"),
        ] {
            document.tables[0].views[0].filters.push(BaseFilter {
                field_id: id.clone(),
                operator,
                value: Some(json!(value)),
                ..Default::default()
            });
        }
        let rename = BaseReferenceRename {
            workspace_path: "D:/ws",
            old_path: "old",
            new_path: "new",
        };
        assert_eq!(rename_base_document_references(&mut document, &rename), 2);
        let filters = &document.tables[0].views[0].filters;
        assert_eq!(filters[0].value, Some(json!(next)));
        assert_eq!(filters[1].value, Some(json!(next)));
        assert_eq!(filters[2].value, Some(json!(old)));
        assert_eq!(filters[3].value, Some(json!("old/note.md")));
        serialize_base_document(&document).unwrap();
    }
}
