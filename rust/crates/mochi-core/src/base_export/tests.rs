use super::*;
use calamine::{Data, Reader, Xlsx};
use serde_json::json;
use std::io::{Cursor, Read};

#[test]
fn xlsx_roundtrip_preserves_types_order_visible_columns_and_freeze() {
    let mut t = create_base_table("测试");
    t.fields = [
        FieldType::Text,
        FieldType::Number,
        FieldType::Progress,
        FieldType::Checkbox,
        FieldType::Date,
        FieldType::DateTime,
        FieldType::SingleSelect,
        FieldType::Text,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, kind)| create_base_field(kind, &format!("列{i}")))
    .collect();
    t.fields[6].options.push(BaseOption {
        id: "opt".into(),
        label: "掌握中".into(),
        ..Default::default()
    });
    let values = [
        json!("=HYPERLINK(\"https://example.invalid\")"),
        json!(42.5),
        json!(45),
        json!(true),
        json!("2026-09-20"),
        json!("2026-09-20T09:15"),
        json!("opt"),
        json!("不可导出的隐藏列"),
    ];
    for i in 0..3 {
        let mut record = BaseRecord {
            id: format!("r{i}"),
            ..Default::default()
        };
        for (field, value) in t.fields.iter().zip(&values) {
            record.values.insert(field.id.clone(), value.clone());
        }
        record
            .values
            .insert(t.fields[1].id.clone(), json!(i as f64 + 0.5));
        t.records.push(record);
    }
    let mut v = create_base_view("不合法/[名称]", ViewType::Grid);
    v.frozen_columns = 2;
    v.frozen_rows = 1;
    v.hidden_field_ids.push(t.fields[7].id.clone());
    let fields = visible_field_indices(&t, &v);
    let bytes = export_view_xlsx(&t, &v, &[2, 0], &fields).unwrap();
    let mut book: Xlsx<_> = Xlsx::new(Cursor::new(bytes.clone())).unwrap();
    let sheet = book.worksheet_range_at(0).unwrap().unwrap();
    assert_eq!(sheet.get_size(), (3, 7));
    assert_eq!(
        sheet.get_value((1, 0)),
        Some(&Data::String(values[0].as_str().unwrap().into()))
    );
    assert_eq!(sheet.get_value((1, 1)), Some(&Data::Float(2.5)));
    assert_eq!(sheet.get_value((2, 1)), Some(&Data::Float(0.5)));
    assert_eq!(sheet.get_value((1, 2)), Some(&Data::Float(0.45)));
    assert_eq!(sheet.get_value((1, 3)), Some(&Data::Bool(true)));
    assert!(matches!(sheet.get_value((1, 4)), Some(Data::DateTime(_))));
    assert!(matches!(sheet.get_value((1, 5)), Some(Data::DateTime(_))));
    assert_eq!(
        sheet.get_value((1, 6)),
        Some(&Data::String("掌握中".into()))
    );
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains("xSplit=\"2\""));
    assert!(xml.contains("ySplit=\"2\""));
    assert!(!xml.contains("<f>"));
}

#[test]
fn empty_export_and_invalid_indices_are_explicit() {
    let t = create_base_table("test");
    assert!(export_view_xlsx(&t, &t.views[0], &[], &[0]).is_ok());
    assert!(export_view_xlsx(&t, &t.views[0], &[], &[]).is_err());
    assert!(export_view_xlsx(&t, &t.views[0], &[0], &[0]).is_err());
}

#[test]
fn oversized_text_fails_without_truncation_and_large_integer_stays_exact() {
    let mut t = create_base_table("test");
    let field = t.fields[0].id.clone();
    let mut r = BaseRecord {
        id: "record".into(),
        ..Default::default()
    };
    r.values.insert(field.clone(), json!("x".repeat(32_768)));
    t.records.push(r);
    assert!(export_view_xlsx(&t, &t.views[0], &[0], &[0]).is_err());
    t.fields[0].field_type = FieldType::Number;
    t.records[0]
        .values
        .insert(field, json!(9_007_199_254_740_993u64));
    let bytes = export_view_xlsx(&t, &t.views[0], &[0], &[0]).unwrap();
    let mut book: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).unwrap();
    assert_eq!(
        book.worksheet_range_at(0)
            .unwrap()
            .unwrap()
            .get_value((1, 0)),
        Some(&Data::String("9007199254740993".into()))
    );
}
