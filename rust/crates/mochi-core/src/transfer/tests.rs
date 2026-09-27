use super::*;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mochi-transfer-{}",
            crate::paths::random_base36(12)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn zip(&self, entries: &[(&str, &str)]) -> std::path::PathBuf {
        let path = self.0.join("test.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, text) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn workflow_round_trip_creates_distinct_unscheduled_copies() {
    let f = Fixture::new();
    let flow = crate::workflows::Workflow::blank();
    let package = Package {
        kind: Kind::Workflow,
        name: "中文工作流".into(),
        content: serde_json::to_string(&flow).unwrap(),
    };
    let path = f.0.join("renamed.zip");
    package.write(&path).unwrap();
    let decoded = inspect(&path).unwrap().unwrap();
    assert_eq!(decoded, package);
    decoded.install(&f.0).unwrap();
    decoded.install(&f.0).unwrap();
    let flows = crate::workflows::Store::open(&f.0).unwrap().list().unwrap();
    assert_eq!(flows.len(), 2);
    assert_ne!(flows[0].definition.id, flows[1].definition.id);
    assert!(flows.iter().all(|f| !f.enabled));
    assert!(read_content(&path, Kind::DesktopCards).is_err());
}

#[test]
fn agent_round_trip_preserves_existing_definitions_and_imports_safely() {
    let f = Fixture::new();
    let package = Package {
        kind: Kind::Agent,
        name: "../CON".into(),
        content: "---\nname: 测试助手\n---\n帮助用户。".into(),
    };
    let path = f.0.join("agent.zip");
    package.write(&path).unwrap();
    let decoded = inspect(&path).unwrap().unwrap();
    decoded.install(&f.0).unwrap();
    decoded.install(&f.0).unwrap();
    let service = crate::ai::agent_config::AgentConfigService::new(&f.0);
    let agents = service.load_agents();
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().all(|agent| agent.name == "测试助手"));
    assert_ne!(agents[0].id, agents[1].id);
    assert_eq!(
        fs::read_to_string(&agents[0].source_path).unwrap(),
        package.content
    );
}

#[test]
fn desktop_round_trip_merges_hidden_cards_and_legacy_json_still_works() {
    use crate::desktop_cards::{Card, DesktopConfig, Module};
    let f = Fixture::new();
    let mut config = DesktopConfig::new();
    let mut card = Card::new("测试卡片", Module::Home);
    card.enabled = true;
    config.cards.push(card);
    config.save(&f.0).unwrap();
    let package = Package {
        kind: Kind::DesktopCards,
        name: "布局".into(),
        content: config.export_json().unwrap(),
    };
    let path = f.0.join("cards.zip");
    package.write(&path).unwrap();
    inspect(&path).unwrap().unwrap().install(&f.0).unwrap();
    let merged = DesktopConfig::load(&f.0).unwrap();
    assert_eq!(merged.cards.len(), 2);
    assert!(merged.cards[0].enabled);
    assert!(!merged.cards[1].enabled);
    assert_ne!(merged.cards[0].id, merged.cards[1].id);
    let legacy = f.0.join("cards.json");
    fs::write(&legacy, &package.content).unwrap();
    assert_eq!(
        read_content(&legacy, Kind::DesktopCards).unwrap(),
        package.content
    );
}

#[test]
fn ordinary_zips_are_not_mochi_packages() {
    let f = Fixture::new();
    let path = f.zip(&[("workflow.json", "{}")]);
    assert!(inspect(&path).unwrap().is_none());
}

#[test]
fn rejects_unknown_versions_wrong_entries_extra_files_and_oversized_content() {
    let f = Fixture::new();
    let manifest = serde_json::json!({"format":"mochi-package","version":1,"kind":"agent","name":"Test","entry":"agent.md"});
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("entry", serde_json::json!("../agent.md")),
        ("kind", serde_json::json!("future-type")),
    ] {
        let mut invalid = manifest.clone();
        invalid[key] = value;
        let path = f.zip(&[
            (MANIFEST, &invalid.to_string()),
            ("agent.md", "---\nname: Test\n---\nHello"),
        ]);
        assert!(inspect(&path).is_err());
    }
    let path = f.zip(&[
        (MANIFEST, &manifest.to_string()),
        ("agent.md", "text"),
        ("../escape.md", "bad"),
    ]);
    assert!(inspect(&path).is_err());
    let path = f.zip(&[
        (MANIFEST, &manifest.to_string()),
        ("agent.md", &"a".repeat(512 * 1024 + 1)),
    ]);
    assert!(inspect(&path).is_err());
    let path = f.zip(&[(MANIFEST, &manifest.to_string()), ("wrong.md", "text")]);
    assert!(inspect(&path).is_err());
}
