use super::*;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "mochi-templates-{}",
            crate::paths::random_base36(12)
        )))
    }
    fn service(&self) -> TemplateService {
        TemplateService::new(&self.0)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn builtins_are_grouped_and_user_groups_are_safe() {
    let workspace = Workspace::new();
    let service = workspace.service();
    service.ensure_builtins().unwrap();
    assert_eq!(service.list().unwrap().len(), 24);
    assert_eq!(service.groups().unwrap().len(), 6);
    for item in BUILTINS {
        assert_eq!(
            crate::document_format::requires_mochi_format(item.content),
            item.extension == "mc",
            "{}",
            item.name
        );
    }
    service.create_group("我的模板").unwrap();
    let template = service
        .create_in_group("草稿", "# 草稿", "我的模板")
        .unwrap();
    assert!(service.delete_group("我的模板").is_err());
    service.delete(&template).unwrap();
    service.delete_group("我的模板").unwrap();
}

#[test]
fn upgrade_preserves_edits_deletions_conversions_and_name_collisions() {
    let workspace = Workspace::new();
    let service = workspace.service();
    for (group, name, body) in LEGACY_BUILTINS {
        service.create_in_group(name, body, group).unwrap();
    }
    std::fs::write(service.root.join(".builtins-v1"), "installed").unwrap();
    let edited = service.root.join("日常/每日复盘.md");
    std::fs::write(&edited, "# 我的复盘\n私人内容").unwrap();
    let deleted = service.root.join("日常/周计划.md");
    std::fs::remove_file(&deleted).unwrap();
    let converted = service.root.join("学习/学习笔记.mc");
    std::fs::rename(service.root.join("学习/学习笔记.md"), &converted).unwrap();
    let collision = service
        .create_in_group("晨间日程", "我已有的模板", "日常")
        .unwrap();
    service.ensure_builtins().unwrap();
    assert_eq!(
        std::fs::read_to_string(&edited).unwrap(),
        "# 我的复盘\n私人内容"
    );
    assert!(!deleted.exists());
    assert!(converted.exists());
    assert!(!service.root.join("学习/学习笔记.md").exists());
    assert_eq!(service.read(&collision).unwrap(), "我已有的模板");
    assert!(!service.root.join("日常/晨间日程.mc").exists());
    let meeting = service
        .list()
        .unwrap()
        .into_iter()
        .find(|t| t.name == "会议纪要")
        .unwrap();
    assert!(service.read(&meeting).unwrap().contains("## 决策记录"));
    let new = service
        .list()
        .unwrap()
        .into_iter()
        .find(|t| t.name == "代码笔记")
        .unwrap();
    service.delete(&new).unwrap();
    service.ensure_builtins().unwrap();
    assert!(
        !new.path.exists(),
        "the v2 marker must preserve later deletions"
    );
}

#[test]
fn instantiation_fills_dates_preserves_source_and_avoids_overwrite() {
    let workspace = Workspace::new();
    let service = workspace.service();
    service.ensure_builtins().unwrap();
    let template = service
        .list()
        .unwrap()
        .into_iter()
        .find(|t| t.name == "晨间日程")
        .unwrap();
    let original = service.read(&template).unwrap();
    let parent = workspace.0.join("知识库");
    std::fs::create_dir_all(&parent).unwrap();
    let first = service.instantiate(&template, &parent).unwrap();
    std::fs::write(&first, "用户已经修改的副本").unwrap();
    let second = service.instantiate(&template, &parent).unwrap();
    assert_ne!(first, second);
    assert_eq!(first.extension().unwrap(), "mc");
    assert_eq!(
        std::fs::read_to_string(first).unwrap(),
        "用户已经修改的副本"
    );
    assert!(!std::fs::read_to_string(second)
        .unwrap()
        .contains("{{日期}}"));
    assert_eq!(service.read(&template).unwrap(), original);
    let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
    assert_eq!(
        fill_date_variables("{{日期}} {{年份}} {{月份}} {{周次}} {{主题}}", date),
        "2026-09-20 2026 09 38 {{主题}}"
    );
}

#[test]
fn saving_and_using_rich_templates_keeps_mochi_format() {
    let workspace = Workspace::new();
    let service = workspace.service();
    let rich = "# 内容\n\n:::mochi-highlight color=\"blue\" title=\"提示\"\n正文\n:::\n";
    let template = service.create("富文本", rich).unwrap();
    assert_eq!(template.path.extension().unwrap(), "mc");
    assert!(service.create("富文本", "plain text").is_err());
    // 旧版本把富文本存成了 .md；实例化副本时要用正确的格式。
    let legacy = service.create("旧模板", "plain text").unwrap();
    std::fs::write(&legacy.path, rich).unwrap();
    let target = service.instantiate(&legacy, &workspace.0).unwrap();
    assert_eq!(target.extension().unwrap(), "mc");
    assert_eq!(std::fs::read_to_string(target).unwrap(), rich);
}
