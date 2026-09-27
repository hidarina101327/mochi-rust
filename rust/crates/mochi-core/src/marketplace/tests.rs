use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mochi-market-test-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn archive(&self, files: &[(&str, &str)], kind: &str) -> (Package, PathBuf) {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let mut package = package(kind);
        package.sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        package.size = bytes.len() as u64;
        let path = self.0.join(&package.asset);
        fs::write(&path, bytes).unwrap();
        (package, path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn package(kind: &str) -> Package {
    serde_json::from_value(serde_json::json!({"id":"coding-agent","kind":kind,"category":"template","title":"编码助手","version":"1.0.0","summary":"编码助手模版","asset":format!("coding-agent-{kind}.zip"),"sha256":"a".repeat(64)})).unwrap()
}
fn release(package: &Package) -> Release {
    Release {
        tag_name: "market-v1".into(),
        assets: vec![Asset {
            name: package.asset.clone(),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/market-v1/{}",
                package.asset
            ),
            size: 123,
        }],
    }
}

#[test]
fn marketplace_resolves_only_the_independent_asset_and_defaults_to_third_party() {
    assert_eq!(
        client().config().tls_config().provider(),
        ureq::tls::TlsProvider::NativeTls
    );
    let p = package("template");
    assert!(!p.official);
    let catalog = resolve_index(
        Index {
            schema_version: 1,
            packages: vec![p.clone()],
        },
        release(&p),
    )
    .unwrap();
    assert!(catalog.packages[0]
        .download_url
        .ends_with("coding-agent-template.zip"));
    assert_eq!(
        catalog.packages[0].install_command().unwrap(),
        "mochi-community install template coding-agent"
    );
}
#[test]
fn marketplace_rejects_duplicate_missing_foreign_and_unsupported_catalog_assets() {
    let p = package("template");
    assert!(resolve_index(
        Index {
            schema_version: 1,
            packages: vec![p.clone(), p.clone()]
        },
        release(&p)
    )
    .is_err());
    assert!(resolve_index(
        Index {
            schema_version: 2,
            packages: vec![p.clone()]
        },
        release(&p)
    )
    .is_err());
    let mut rel = release(&p);
    rel.assets.clear();
    assert!(resolve_index(
        Index {
            schema_version: 1,
            packages: vec![p.clone()]
        },
        rel
    )
    .is_err());
    let mut rel = release(&p);
    rel.assets[0].browser_download_url = "https://example.com/package.zip".into();
    assert!(resolve_index(
        Index {
            schema_version: 1,
            packages: vec![p]
        },
        rel
    )
    .is_err());
}
#[test]
fn marketplace_installs_template_visible_in_template_center_and_preserves_existing() {
    let fixture = Fixture::new();
    let (p, archive) = fixture.archive(
        &[("编码助手.md", "# 编码助手"), ("assets/note.txt", "资源")],
        "template",
    );
    install_archive(&p, &archive, &fixture.0).unwrap();
    let templates = crate::templates::TemplateService::new(&fixture.0)
        .list()
        .unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name, "编码助手");
    fs::write(&templates[0].path, "用户修改").unwrap();
    assert!(install_archive(&p, &archive, &fixture.0).is_err());
    assert_eq!(fs::read_to_string(&templates[0].path).unwrap(), "用户修改");
}
#[test]
fn marketplace_rejects_tampering_before_installing() {
    let fixture = Fixture::new();
    let (mut p, archive) = fixture.archive(&[("note.md", "original")], "template");
    p.sha256 = "0".repeat(64);
    assert!(install_archive(&p, &archive, &fixture.0).is_err());
    assert!(!fixture.0.join(".mochi").exists());
}
#[test]
fn marketplace_rejects_unsafe_archive_paths_before_writing() {
    for name in [
        "../escaped.md",
        "/escaped.md",
        "C:/escaped.md",
        "assets\\escaped.md",
        "assets/CON.txt",
        "assets/trailing./note.md",
    ] {
        let fixture = Fixture::new();
        let (p, archive) = fixture.archive(&[("valid.md", "ok"), (name, "bad")], "template");
        assert!(install_archive(&p, &archive, &fixture.0).is_err(), "{name}");
        assert!(!fixture.0.join(".mochi").exists());
    }
}
#[test]
fn marketplace_imported_workflow_is_available_but_not_scheduled() {
    let fixture = Fixture::new();
    let mut flow = crate::workflows::Workflow::blank();
    flow.name = "市场工作流".into();
    let text = serde_json::to_string(&flow).unwrap();
    let (p, archive) = fixture.archive(&[("workflow.json", &text)], "workflow");
    install_archive(&p, &archive, &fixture.0).unwrap();
    let items = crate::workflows::Store::open(&fixture.0)
        .unwrap()
        .summaries()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(!items[0].enabled);
    assert_ne!(items[0].id, flow.id);
}
#[test]
fn marketplace_plugin_identity_must_match_catalog() {
    let fixture = Fixture::new();
    let manifest = r#"{"manifestVersion":2,"id":"different","name":"Other","version":"1.0.0"}"#;
    let (p, archive) = fixture.archive(&[("manifest.json", manifest)], "plugin");
    assert!(install_archive(&p, &archive, &fixture.0).is_err());
    assert!(!fixture.0.join(".mochi/plugins").exists());
}

#[test]
fn marketplace_installs_only_selected_native_plugin_and_preserves_existing() {
    let fixture = Fixture::new();
    let manifest = r#"{"manifestVersion":2,"id":"coding-agent","name":"Coding","version":"1.0.0"}"#;
    let (p, archive) = fixture.archive(&[("manifest.json", manifest), ("ui.json", "{}")], "plugin");
    install_archive(&p, &archive, &fixture.0).unwrap();
    let plugins = crate::plugins::PluginService::new(&fixture.0)
        .list()
        .unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].manifest.id, "coding-agent");
    assert!(install_archive(&p, &archive, &fixture.0).is_err());
    assert_eq!(
        fs::read_to_string(plugins[0].directory.join("ui.json")).unwrap(),
        "{}"
    );
}
