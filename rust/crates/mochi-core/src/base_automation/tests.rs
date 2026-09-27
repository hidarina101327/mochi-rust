use super::*;
const TABLE: &str = "table_main";
const VIEW: &str = "view_grid";
fn doc() -> BaseDocument {
    base::parse_base_document(include_str!("../../../../../tests/fixtures/base-v1.mcb")).unwrap()
}
fn rule(trigger: Trigger) -> Rule {
    Rule {
        id: "auto_1".into(),
        name: "自动标记".into(),
        enabled: true,
        trigger,
        conditions: vec![],
        actions: vec![Action::UpdateRecord {
            values: BTreeMap::from([("checked".into(), Input::Literal { value: json!(true) })]),
        }],
    }
}
fn install(d: &mut BaseDocument, r: Rule) -> BTreeSet<String> {
    put(d, TABLE, VIEW, r.clone()).unwrap();
    let (t, v) = scope(d, TABLE, VIEW).unwrap();
    BTreeSet::from([signature(d, t, v, &r)])
}
fn add(d: &mut BaseDocument, id: &str) {
    d.tables[0].records.push(BaseRecord {
        id: id.into(),
        values: serde_json::from_value(json!({"name":"新任务","checked":false})).unwrap(),
        ..Default::default()
    });
}

#[test]
fn preview_is_pure_and_manual_run_is_logged() {
    let d = doc();
    let r = rule(Trigger::Manual);
    let before = d.clone();
    let (p, report) = preview(&d, TABLE, VIEW, &r, &["record_a".into()], 1).unwrap();
    assert_eq!(d, before);
    assert!(runtime(&p).runs.is_empty());
    assert_eq!(report.updates, 1);
    let (written, _) = manual_run(&d, TABLE, VIEW, &r, &["record_a".into()], 1).unwrap();
    assert_eq!(runtime(&written).runs.len(), 1);
    assert!(written.tables[0].records[0].values["checked"]
        .as_bool()
        .unwrap());
}
#[test]
fn fresh_baseline_does_not_backfill_but_new_records_fire_once() {
    let mut d = doc();
    let consent = install(&mut d, rule(Trigger::RecordCreated));
    let (same, s) = evaluate(&d, &Snapshot::default(), &consent, 0).unwrap();
    assert_eq!(same, d);
    add(&mut d, "fresh");
    let (written, next) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(runtime(&written).runs[0].record_ids, ["fresh"]);
    let (again, _) = evaluate(&written, &next, &consent, 2).unwrap();
    assert_eq!(again, written);
}
#[test]
fn unapproved_imports_cannot_run_even_with_enabled_flag() {
    let mut d = doc();
    install(
        &mut d,
        rule(Trigger::Interval {
            minutes: 1,
            start_at: 0,
        }),
    );
    let (same, _) = evaluate(&d, &Snapshot::default(), &BTreeSet::new(), 60_000).unwrap();
    assert_eq!(same, d);
}
#[test]
fn watched_fields_ignore_unrelated_changes_and_record_creation() {
    let mut d = doc();
    let consent = install(
        &mut d,
        rule(Trigger::RecordUpdated {
            field_ids: vec!["amount".into()],
        }),
    );
    let s = snapshot(&d, &consent);
    d.tables[0].records[0]
        .values
        .insert("name".into(), json!("rename"));
    add(&mut d, "fresh");
    let (same, s) = evaluate(&d, &s, &consent, 0).unwrap();
    assert_eq!(same, d);
    d.tables[0].records[0]
        .values
        .insert("amount".into(), json!(12));
    let (written, _) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(runtime(&written).runs[0].record_ids, ["record_a"]);
}
#[test]
fn enters_view_is_transition_and_exit_then_reentry_retriggers() {
    let mut d = doc();
    let mut r = rule(Trigger::EnterView);
    r.conditions = vec![BaseFilter {
        field_id: "group".into(),
        operator: base::FilterOperator::Equals,
        value: Some(json!("option_a")),
        ..Default::default()
    }];
    let consent = install(&mut d, r);
    let s = snapshot(&d, &consent);
    d.tables[0].records[1]
        .values
        .insert("group".into(), json!("option_a"));
    let (mut written, s) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(runtime(&written).runs[0].record_ids, ["record_b"]);
    written.tables[0].records[1]
        .values
        .insert("name".into(), json!("changed"));
    let (same, s) = evaluate(&written, &s, &consent, 2).unwrap();
    assert_eq!(same, written);
    written.tables[0].records[1]
        .values
        .insert("group".into(), json!("option_b"));
    let (_, s) = evaluate(&written, &s, &consent, 3).unwrap();
    written.tables[0].records[1]
        .values
        .insert("group".into(), json!("option_a"));
    let (again, _) = evaluate(&written, &s, &consent, 4).unwrap();
    assert_eq!(runtime(&again).runs.len(), 2);
}
#[test]
fn schedule_is_durable_once_per_slot_and_skips_catchup_flood() {
    let mut d = doc();
    let consent = install(
        &mut d,
        rule(Trigger::Interval {
            minutes: 1,
            start_at: 1000,
        }),
    );
    let (same, _) = evaluate(&d, &Snapshot::default(), &consent, 999).unwrap();
    assert_eq!(same, d);
    let (first, _) = evaluate(&d, &Snapshot::default(), &consent, 1000).unwrap();
    assert_eq!(runtime(&first).runs.len(), 1);
    let (same, _) = evaluate(&first, &Snapshot::default(), &consent, 59_000).unwrap();
    assert_eq!(same, first);
    let (later, _) = evaluate(&first, &Snapshot::default(), &consent, 900_000).unwrap();
    assert_eq!(runtime(&later).runs.len(), 2);
}
#[test]
fn actions_do_not_cascade_and_manual_effects_are_suppressed() {
    let mut d = doc();
    let mut r = rule(Trigger::RecordCreated);
    r.actions.push(Action::CreateRecord {
        table_id: TABLE.into(),
        values: BTreeMap::from([(
            "name".into(),
            Input::Literal {
                value: json!("generated"),
            },
        )]),
    });
    let consent = install(&mut d, r.clone());
    let s = snapshot(&d, &consent);
    add(&mut d, "new");
    let (written, s) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(written.tables[0].records.len(), 6);
    let (again, _) = evaluate(&written, &s, &consent, 2).unwrap();
    assert_eq!(again, written);
    let (manual, _) = manual_run(&written, TABLE, VIEW, &r, &["record_a".into()], 3).unwrap();
    let (again, _) = evaluate(&manual, &s, &consent, 4).unwrap();
    assert_eq!(again, manual);
}
#[test]
fn invalid_later_action_rolls_back_entire_run_and_auto_pauses() {
    let mut d = doc();
    let mut r = rule(Trigger::RecordCreated);
    r.actions.push(Action::UpdateRecord {
        values: BTreeMap::from([(
            "amount".into(),
            Input::Field {
                field_id: "name".into(),
            },
        )]),
    });
    let consent = install(&mut d, r.clone());
    let s = snapshot(&d, &consent);
    add(&mut d, "fresh");
    assert!(preview(&d, TABLE, VIEW, &r, &["fresh".into()], 1).is_err());
    let (result, _) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(result.tables[0].records, d.tables[0].records);
    assert!(!rules(&result.tables[0].views[0]).unwrap()[0].enabled);
    assert!(runtime(&result).runs[0].error.is_some());
}
#[test]
fn expressions_read_original_record_across_actions_and_cross_table_create() {
    let d = doc();
    let mut r = rule(Trigger::Manual);
    r.actions = vec![
        Action::UpdateRecord {
            values: BTreeMap::from([(
                "name".into(),
                Input::Literal {
                    value: json!("new"),
                },
            )]),
        },
        Action::CreateRecord {
            table_id: "table_secondary".into(),
            values: BTreeMap::from([(
                "name".into(),
                Input::Field {
                    field_id: "name".into(),
                },
            )]),
        },
    ];
    let (result, run) = preview(&d, TABLE, VIEW, &r, &["record_a".into()], 1).unwrap();
    assert_eq!(result.tables[0].records[0].values["name"], "new");
    assert_eq!(result.tables[1].records[0].values["name"], "Alpha");
    assert_eq!(run.creates, 1);
}
#[test]
fn config_and_filter_changes_invalidate_consent_and_reset_edges() {
    let mut d = doc();
    let mut r = rule(Trigger::EnterView);
    let consent = install(&mut d, r.clone());
    let s = snapshot(&d, &consent);
    r.name = "changed".into();
    let new = install(&mut d, r);
    let (same, _) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(same, d);
    let (same, _) = evaluate(&d, &s, &new, 1).unwrap();
    assert_eq!(same, d);
    d.tables[0].views[0].filters.push(BaseFilter {
        field_id: "checked".into(),
        operator: base::FilterOperator::Equals,
        value: Some(json!(true)),
        ..Default::default()
    });
    let (same, _) = evaluate(&d, &s, &new, 1).unwrap();
    assert_eq!(same, d);
}
#[test]
fn record_scope_duplicates_and_limits_are_enforced() {
    let d = doc();
    let r = rule(Trigger::Manual);
    assert!(preview(&d, TABLE, "view_empty", &r, &["record_a".into()], 0).is_err());
    assert!(preview(
        &d,
        TABLE,
        VIEW,
        &r,
        &["record_a".into(), "record_a".into()],
        0
    )
    .is_err());
    assert!(preview(&d, TABLE, VIEW, &r, &[], 0).is_err());
    assert!(preview(&d, TABLE, VIEW, &r, &vec!["record_a".into(); 201], 0).is_err());
}
#[test]
fn unknown_fields_scripts_and_invalid_values_are_rejected() {
    let mut d = doc();
    let mut r = rule(Trigger::Manual);
    r.actions = vec![Action::UpdateRecord {
        values: BTreeMap::from([(
            "amount".into(),
            Input::Literal {
                value: json!("bad"),
            },
        )]),
    }];
    assert!(put(&mut d, TABLE, VIEW, r).is_err());
    assert!(serde_json::from_value::<Trigger>(json!({"type":"script","code":"danger"})).is_err());
    assert!(serde_json::from_value::<Input>(
        json!({"type":"literal","value":true,"script":"extra"})
    )
    .is_err());
}
#[test]
fn portable_roundtrip_preserves_rules_and_bounded_history() {
    let mut d = doc();
    let r = rule(Trigger::Manual);
    install(&mut d, r.clone());
    for n in 0..105 {
        d = manual_run(&d, TABLE, VIEW, &r, &["record_a".into()], n)
            .unwrap()
            .0;
    }
    assert_eq!(runtime(&d).runs.len(), 100);
    let parsed = base::parse_base_document(&base::serialize_base_document(&d).unwrap()).unwrap();
    assert_eq!(parsed, d);
    assert_eq!(parsed.extra["extension"]["preserved"], true);
}
#[test]
fn later_rules_can_restore_values_changed_by_earlier_rules() {
    let mut d = doc();
    let a = rule(Trigger::RecordCreated);
    let mut consent = install(&mut d, a);
    let mut b = rule(Trigger::RecordCreated);
    b.id = "auto_2".into();
    b.actions = vec![Action::UpdateRecord {
        values: BTreeMap::from([(
            "checked".into(),
            Input::Literal {
                value: json!(false),
            },
        )]),
    }];
    consent.extend(install(&mut d, b));
    let s = snapshot(&d, &consent);
    add(&mut d, "fresh");
    let (result, _) = evaluate(&d, &s, &consent, 1).unwrap();
    assert_eq!(
        result.tables[0].records.last().unwrap().values["checked"],
        false
    );
    assert_eq!(runtime(&result).runs.len(), 2);
}
#[test]
fn current_time_matches_native_datetime_cell_contract() {
    let d = doc();
    let mut r = rule(Trigger::Manual);
    r.actions = vec![Action::UpdateRecord {
        values: BTreeMap::from([("time_point".into(), Input::Now)]),
    }];
    let (result, _) =
        preview(&d, TABLE, VIEW, &r, &["record_a".into()], 1_790_000_000_000).unwrap();
    assert!(base::valid_date_time(
        result.tables[0].records[0].values["time_point"]
            .as_str()
            .unwrap()
    ));
}

#[test]
fn file_lock_excludes_peer_instances_and_releases_on_drop() {
    let root = std::env::temp_dir().join(base::create_id("mochi-auto-lock"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("test.mcb");
    let first = FileLock::acquire(&path).unwrap();
    assert!(FileLock::acquire(&path).is_err());
    drop(first);
    let next = FileLock::acquire(&path).unwrap();
    drop(next);
    let _ = std::fs::remove_dir_all(&root);
}
