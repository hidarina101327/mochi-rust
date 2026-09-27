use super::*;
use serde_json::{json, Value};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestProfile {
    root: PathBuf,
    path: PathBuf,
}

impl TestProfile {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mochi-settings-shared-{}-{}-{}-{tag}",
            std::process::id(),
            file::now_millis(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        // 只清理本夹具自己成功创建的目录。
        fs::create_dir(&root).unwrap();
        Self {
            path: root.join("settings.json"),
            root,
        }
    }

    fn service(&self) -> SettingsService {
        SettingsService::new(Some(self.path.clone()))
    }

    fn write(&self, values: &file::Values) {
        file::write_atomic(&self.path, values).unwrap();
    }

    fn values(&self) -> file::Values {
        file::read(&self.path).unwrap()
    }

    fn bucket(&self, name: &str) -> Value {
        serde_json::from_str(&self.values()[name]).unwrap()
    }
}

impl Drop for TestProfile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn shared_values(buckets: &[(&str, Value)]) -> file::Values {
    let mut values = file::Values::from([
        (projection::VERSION_KEY.into(), "1".into()),
        (projection::SOURCE_KEY.into(), "electron-test".into()),
        ("future-top-level-setting".into(), "preserve me".into()),
    ]);
    for (name, value) in buckets {
        values.insert((*name).into(), value.to_string());
    }
    values
}

fn provider_fixture() -> file::Values {
    shared_values(&[(
        "mochi-ai",
        json!({
            "version": 3,
            "futureEnvelope": {"enabled": true},
            "state": {
                "currentProviderId": "alpha",
                "unrecognizedState": {"keep": [1, 2, 3]},
                "providers": [
                    {
                        "id": "alpha", "name": "Alpha", "baseUrl": "https://alpha.invalid/v1",
                        "model": "old-model", "stream": true,
                        "apiKey": "dpapi:opaque-alpha-fixture",
                        "futureProviderField": {"headers": {"X-Custom": "retained"}}
                    },
                    {
                        "id": "beta", "name": "Beta", "baseUrl": "https://beta.invalid/v1",
                        "model": "beta-model", "stream": false,
                        "apiKey": "dpapi:opaque-beta-fixture"
                    }
                ]
            }
        }),
    )])
}

fn metadata(service: &SettingsService) -> Value {
    serde_json::from_str(&service.get("ai.providers").unwrap()).unwrap()
}

fn provider<'a>(providers: &'a Value, id: &str) -> &'a Value {
    providers
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap()
}

#[test]
fn stale_instances_merge_mapped_and_nested_fields_in_either_flush_order() {
    for reverse in [false, true] {
        let profile = TestProfile::new("mapped-fields");
        profile.write(&shared_values(&[
            ("workspace-storage", json!({
                "version": 1,
                "state": {"navigationWidth": 240, "sidebarWidth": 280, "unknown": {"keep": true}}
            })),
            ("theme-storage", json!({
                "version": 0,
                "state": {
                    "mode": "system",
                    "customBackground": {"position": {"x": 50, "y": 50, "futureAxis": 7}, "scale": 100}
                }
            })),
        ]));
        let left = profile.service();
        let right = profile.service();
        left.set("app.navigation.width", "320");
        left.set("app.background.positionX", "17");
        right.set("app.sidebar.width", "410");
        right.set("app.background.positionY", "83");
        let writers = if reverse {
            [&right, &left]
        } else {
            [&left, &right]
        };
        for writer in writers {
            writer.flush().unwrap();
        }

        let workspace = profile.bucket("workspace-storage");
        assert_eq!(workspace["state"]["navigationWidth"], 320);
        assert_eq!(workspace["state"]["sidebarWidth"], 410);
        assert_eq!(workspace["state"]["unknown"], json!({"keep": true}));
        let theme = profile.bucket("theme-storage");
        assert_eq!(
            theme["state"]["customBackground"]["position"],
            json!({"x": 17, "y": 83, "futureAxis": 7})
        );
        assert_eq!(theme["state"]["customBackground"]["scale"], 100);
        assert_eq!(theme["state"]["mode"], "system");
        let values = profile.values();
        assert_eq!(values["future-top-level-setting"], "preserve me");
        for key in [
            "app.navigation.width",
            "app.sidebar.width",
            "app.background.positionX",
            "app.background.positionY",
        ] {
            assert!(
                !values.contains_key(key),
                "native alias persisted a second copy: {key}"
            );
        }
        assert!(!left.is_dirty());
        assert!(!right.is_dirty());
    }
}

#[test]
fn favorite_parent_display_uses_the_workspace_storage_mapping() {
    let profile = TestProfile::new("favorite-parent-display");
    profile.write(&shared_values(&[(
        "workspace-storage",
        json!({
            "version": 1,
            "state": {"showFavoriteParents": false}
        }),
    )]));

    let service = profile.service();
    assert_eq!(
        service.get("app.sidebar.showFavoriteParents").as_deref(),
        Some("false")
    );
    service.set("app.sidebar.showFavoriteParents", "true");
    service.flush().unwrap();

    assert_eq!(
        profile.bucket("workspace-storage")["state"]["showFavoriteParents"],
        true
    );
    assert!(!profile
        .values()
        .contains_key("app.sidebar.showFavoriteParents"));
    assert_eq!(
        profile
            .service()
            .get("app.sidebar.showFavoriteParents")
            .as_deref(),
        Some("true")
    );
}

#[test]
fn stale_instances_merge_native_json_leaf_changes_and_removals() {
    let profile = TestProfile::new("native-json");
    profile.write(&shared_values(&[(
        "native.windowLayout",
        json!({
            "split": {"left": 20, "right": 30, "obsolete": true},
            "unchanged": {"keep": "yes"}
        }),
    )]));
    let left = profile.service();
    let right = profile.service();
    left.set(
        "native.windowLayout",
        &json!({
            "split": {"left": 40, "right": 30}, "unchanged": {"keep": "yes"}
        })
        .to_string(),
    );
    right.set(
        "native.windowLayout",
        &json!({
            "split": {"left": 20, "right": 60, "obsolete": true, "newLeaf": 5},
            "unchanged": {"keep": "yes"}
        })
        .to_string(),
    );
    left.flush().unwrap();
    right.flush().unwrap();
    assert_eq!(
        profile.bucket("native.windowLayout"),
        json!({
            "split": {"left": 40, "right": 60, "newLeaf": 5},
            "unchanged": {"keep": "yes"}
        })
    );
}

#[test]
fn stale_provider_edits_merge_fields_and_ids_without_losing_secrets_or_extensions() {
    for reverse in [false, true] {
        let profile = TestProfile::new("provider-merge");
        profile.write(&provider_fixture());
        let left = profile.service();
        let right = profile.service();
        let mut left_metadata = metadata(&left);
        let mut right_metadata = metadata(&right);
        assert!(left_metadata[0].get("apiKey").is_none());
        assert!(left_metadata[0].get("futureProviderField").is_none());

        left_metadata[0]["name"] = json!("Alpha renamed");
        left_metadata
            .as_array_mut()
            .unwrap()
            .retain(|item| item["id"] != "beta");
        left_metadata.as_array_mut().unwrap().push(json!({
            "id": "gamma", "name": "Gamma", "baseUrl": "https://gamma.invalid/v1", "model": "gamma-model", "stream": true
        }));
        right_metadata[0]["model"] = json!("new-model");
        right_metadata.as_array_mut().unwrap().push(json!({
            "id": "delta", "name": "Delta", "baseUrl": "https://delta.invalid/v1", "model": "delta-model", "stream": false
        }));
        left.set("ai.providers", &left_metadata.to_string());
        right.set("ai.providers", &right_metadata.to_string());
        let writers = if reverse {
            [&right, &left]
        } else {
            [&left, &right]
        };
        for writer in writers {
            writer.flush().unwrap();
        }

        let ai = profile.bucket("mochi-ai");
        let providers = &ai["state"]["providers"];
        assert_eq!(providers.as_array().unwrap().len(), 3);
        let alpha = provider(providers, "alpha");
        assert_eq!(alpha["name"], "Alpha renamed");
        assert_eq!(alpha["model"], "new-model");
        assert_eq!(alpha["apiKey"], "dpapi:opaque-alpha-fixture");
        assert_eq!(
            alpha["futureProviderField"],
            json!({"headers": {"X-Custom": "retained"}})
        );
        assert_eq!(provider(providers, "gamma")["model"], "gamma-model");
        assert_eq!(provider(providers, "delta")["model"], "delta-model");
        assert!(!providers
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "beta"));
        assert_eq!(profile.service().get("ai.providers.beta.apiKey"), None);
        assert_eq!(ai["futureEnvelope"], json!({"enabled": true}));
        assert_eq!(ai["state"]["unrecognizedState"], json!({"keep": [1, 2, 3]}));
        let values = profile.values();
        assert!(!values.contains_key("ai.providers"));
        assert!(!values.keys().any(|key| key.starts_with("ai.providers.")));
    }
}

#[test]
fn stale_provider_metadata_preserves_a_concurrently_replaced_secret() {
    let profile = TestProfile::new("provider-secret");
    profile.write(&provider_fixture());
    let metadata_writer = profile.service();
    let secret_writer = profile.service();
    let mut edited = metadata(&metadata_writer);
    edited[0]["name"] = json!("Renamed after secret snapshot");
    metadata_writer.set("ai.providers", &edited.to_string());
    // 用一段不可读的加密内容当夹具，任何平台都能测试合并逻辑：
    // 不需要真实凭据，也不逼非 Windows 宿主用 DPAPI 加密。
    secret_writer.set(
        "ai.providers.alpha.apiKey",
        "dpapi:opaque-replacement-fixture",
    );
    secret_writer.flush().unwrap();
    metadata_writer.flush().unwrap();

    let ai = profile.bucket("mochi-ai");
    let alpha = provider(&ai["state"]["providers"], "alpha");
    assert_eq!(alpha["name"], "Renamed after secret snapshot");
    assert_eq!(alpha["apiKey"], "dpapi:opaque-replacement-fixture");
    assert_eq!(
        alpha["futureProviderField"],
        json!({"headers": {"X-Custom": "retained"}})
    );
}

#[test]
fn stale_provider_field_edit_does_not_revive_a_provider_deleted_before_flush() {
    let profile = TestProfile::new("deleted-provider-edit");
    profile.write(&provider_fixture());
    let editing = profile.service();
    let deleting = profile.service();
    let mut edited = metadata(&editing);
    edited[0]["name"] = json!("Unflushed local name");
    editing.set("ai.providers", &edited.to_string());
    editing.set("ai.providers.alpha.apiKey", "dpapi:unflushed-new-key");
    let mut removed = metadata(&deleting);
    removed
        .as_array_mut()
        .unwrap()
        .retain(|item| item["id"] != "alpha");
    deleting.set("ai.providers", &removed.to_string());
    deleting.flush().unwrap();
    editing.flush().unwrap();
    assert!(profile.bucket("mochi-ai")["state"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["id"] != "alpha"));
}

#[test]
fn reader_observes_external_changes_and_retains_the_revision_for_focus_refresh() {
    let profile = TestProfile::new("external-reader");
    profile.write(&shared_values(&[(
        "workspace-storage",
        json!({
            "version": 1, "state": {"sidebarWidth": 280}
        }),
    )]));
    let reader = profile.service();
    assert_eq!(reader.get("app.sidebar.width").as_deref(), Some("280"));
    let old_revision = reader.revision();
    let mut external = profile.values();
    external.insert(
        "workspace-storage".into(),
        json!({
            "version": 1, "state": {"sidebarWidth": 425, "workspacePath": "D:/external/workspace"}
        })
        .to_string(),
    );
    external.insert(
        "new-external-setting".into(),
        "visible after refresh".into(),
    );
    profile.write(&external);
    assert!(reader.reload().unwrap());
    assert_eq!(reader.get("app.sidebar.width").as_deref(), Some("425"));
    assert!(reader.revision() > old_revision);
    let observed_revision = reader.revision();
    assert_eq!(
        reader.get("workspace.lastPath").as_deref(),
        Some("D:/external/workspace")
    );
    assert!(reader
        .keys()
        .iter()
        .any(|key| key == "new-external-setting"));
    assert!(!reader.reload().unwrap());
    assert_eq!(reader.revision(), observed_revision);
    assert!(!reader.is_dirty());
}

#[test]
fn external_refresh_replays_unsaved_edits_before_flush() {
    let profile = TestProfile::new("external-pending");
    profile.write(&shared_values(&[(
        "workspace-storage",
        json!({
            "version": 1, "state": {"navigationWidth": 240, "sidebarWidth": 280}
        }),
    )]));
    let local = profile.service();
    local.set("app.navigation.width", "350");
    let mut external = profile.values();
    external.insert("workspace-storage".into(), json!({
        "version": 1, "state": {"navigationWidth": 240, "sidebarWidth": 430, "futureField": "external"}
    }).to_string());
    profile.write(&external);

    assert!(local.reload().unwrap());
    assert!(local.is_dirty());
    assert_eq!(local.get("app.navigation.width").as_deref(), Some("350"));
    assert_eq!(local.get("app.sidebar.width").as_deref(), Some("430"));
    local.flush().unwrap();
    assert!(!local.is_dirty());
    let workspace = profile.bucket("workspace-storage");
    assert_eq!(workspace["state"]["navigationWidth"], 350);
    assert_eq!(workspace["state"]["sidebarWidth"], 430);
    assert_eq!(workspace["state"]["futureField"], "external");
}

#[test]
fn corrupt_file_blocks_flush_without_consuming_pending_edits_or_overwriting_bytes() {
    let profile = TestProfile::new("corrupt-pending");
    let initial = shared_values(&[(
        "workspace-storage",
        json!({
            "version": 1, "state": {"sidebarWidth": 280, "futureField": "preserved"}
        }),
    )]);
    profile.write(&initial);
    let service = profile.service();
    service.set("app.sidebar.width", "470");
    let corrupt = b"{ invalid shared settings: leave these exact bytes alone\n";
    fs::write(&profile.path, corrupt).unwrap();

    assert!(service.reload().is_err());
    assert!(service.flush().is_err());
    assert!(service.is_dirty());
    assert_eq!(service.get("app.sidebar.width").as_deref(), Some("470"));
    assert_eq!(fs::read(&profile.path).unwrap(), corrupt);
    assert!(!file::suffix(&profile.path, ".lock").exists());

    profile.write(&initial);
    service.flush().unwrap();
    assert!(!service.is_dirty());
    assert_eq!(
        profile.bucket("workspace-storage")["state"]["sidebarWidth"],
        470
    );
    assert_eq!(
        profile.bucket("workspace-storage")["state"]["futureField"],
        "preserved"
    );
}

#[test]
fn directory_at_file_path_blocks_flush_and_preserves_pending_edits_for_retry() {
    let profile = TestProfile::new("directory-pending");
    let initial = shared_values(&[]);
    profile.write(&initial);
    let service = profile.service();
    service.set("workspace.lastPath", "D:/pending/workspace");
    fs::remove_file(&profile.path).unwrap();
    fs::create_dir(&profile.path).unwrap();
    let sentinel = profile.path.join("owned-by-another-component.txt");
    fs::write(&sentinel, "do not replace or remove").unwrap();

    assert!(service.flush().is_err());
    assert!(service.is_dirty());
    assert_eq!(
        service.get("workspace.lastPath").as_deref(),
        Some("D:/pending/workspace")
    );
    assert!(profile.path.is_dir());
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        "do not replace or remove"
    );
    assert!(!file::suffix(&profile.path, ".lock").exists());

    fs::remove_file(&sentinel).unwrap();
    fs::remove_dir(&profile.path).unwrap();
    profile.write(&initial);
    service.flush().unwrap();
    assert!(!service.is_dirty());
    assert_eq!(
        profile.bucket("workspace-storage")["state"]["workspacePath"],
        "D:/pending/workspace"
    );
}

fn write_legacy_source(base: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = base.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn initialization_uses_dev_authority_backs_up_source_and_does_not_overwrite_on_retry() {
    let profile = TestProfile::new("initialization-authority");
    let base = profile.root.join("isolated-appdata");
    let dev = file::Values::from([
        ("workspace-storage".into(), json!({
            "version": 1, "state": {"sidebarWidth": 305, "futureField": {"keep": true}}
        }).to_string()),
        ("mochi-ai".into(), json!({
            "version": 3, "state": {
                "currentProviderId": "dev-provider",
                "providers": [{"id": "dev-provider", "name": "Dev", "apiKey": "dpapi:opaque-dev-fixture"}]
            }
        }).to_string()),
        ("native.customPreference".into(), "dev owns this value".into()),
    ]);
    let dev_bytes = serde_json::to_vec_pretty(&dev).unwrap();
    let dev_path = write_legacy_source(&base, "mochi-dev/mochi-dev-settings.json", &dev_bytes);
    let release =
        file::Values::from([("release-only-setting".into(), "must not be imported".into())]);
    let release_bytes = serde_json::to_vec(&release).unwrap();
    let release_path =
        write_legacy_source(&base, "mochi-desktop/mochi-settings.json", &release_bytes);
    let native = file::Values::from([
        ("app.sidebar.width".into(), "999".into()),
        ("app.background.positionX".into(), "99".into()),
        ("ai.currentProviderId".into(), "native-provider".into()),
        (
            "ai.providers".into(),
            json!([{"id": "native-provider", "name": "Native"}]).to_string(),
        ),
        (
            "ai.providers.native-provider.apiKey".into(),
            "dpapi:opaque-native-fixture".into(),
        ),
        (
            "native.customPreference".into(),
            "stale native value".into(),
        ),
        (
            "native.exclusivePreference".into(),
            "retained native-only value".into(),
        ),
    ]);
    let native_bytes = serde_json::to_vec_pretty(&native).unwrap();
    let native_path = write_legacy_source(&base, "mochi-native/settings.json", &native_bytes);
    write_legacy_source(
        &base,
        "mochi-dev/mochi-performance.json",
        br#"{"renderer":"dev","gpu":true}"#,
    );
    write_legacy_source(
        &base,
        "mochi-desktop/mochi-performance.json",
        br#"{"renderer":"release","gpu":false}"#,
    );

    initialize_shared_profile_at(&profile.path, &base).unwrap();
    let migrated = profile.values();
    assert_eq!(migrated[projection::SOURCE_KEY], "electron-dev");
    assert_eq!(migrated[projection::VERSION_KEY], "1");
    assert!(migrated.contains_key("__mochi_shared_settings_migrated_at"));
    assert_eq!(migrated["native.customPreference"], "dev owns this value");
    assert_eq!(
        migrated["native.exclusivePreference"],
        "retained native-only value"
    );
    assert!(!migrated.contains_key("release-only-setting"));
    assert_eq!(
        profile.bucket("electron.performance"),
        json!({"renderer": "dev", "gpu": true})
    );
    let service = profile.service();
    assert_eq!(service.get("app.sidebar.width").as_deref(), Some("305"));
    assert_eq!(service.get("app.background.positionX"), None);
    assert_eq!(
        service.get("ai.currentProviderId").as_deref(),
        Some("dev-provider")
    );
    assert_eq!(service.get("ai.providers.native-provider.apiKey"), None);
    assert_eq!(
        service.get("ai.providers.dev-provider.apiKey").as_deref(),
        Some("dpapi:opaque-dev-fixture")
    );
    for key in native.keys().filter(|key| projection::is_alias(key)) {
        assert!(
            !migrated.contains_key(key),
            "legacy alias overrode Electron authority: {key}"
        );
    }
    let backups: Vec<_> = fs::read_dir(profile.root.join("settings-backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    assert!(backups[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("-electron-dev.json"));
    assert_eq!(fs::read(&backups[0]).unwrap(), dev_bytes);
    assert_eq!(fs::read(&dev_path).unwrap(), dev_bytes);
    assert_eq!(fs::read(&release_path).unwrap(), release_bytes);
    assert_eq!(fs::read(&native_path).unwrap(), native_bytes);

    service.set("app.sidebar.width", "480");
    service.flush().unwrap();
    let before_retry = fs::read(&profile.path).unwrap();
    initialize_shared_profile_at(&profile.path, &base).unwrap();
    assert_eq!(fs::read(&profile.path).unwrap(), before_retry);
    assert_eq!(
        fs::read_dir(profile.root.join("settings-backups"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn corrupt_dev_source_prevents_initialization_without_falling_back_or_changing_sources() {
    let profile = TestProfile::new("initialization-corrupt-dev");
    let base = profile.root.join("isolated-appdata");
    let corrupt_bytes = b"{ broken authoritative development profile";
    let dev_path = write_legacy_source(&base, "mochi-dev/mochi-dev-settings.json", corrupt_bytes);
    let fallback = file::Values::from([(
        "fallback-setting".into(),
        "must not silently replace dev".into(),
    )]);
    let fallback_bytes = serde_json::to_vec(&fallback).unwrap();
    let release_path =
        write_legacy_source(&base, "mochi-desktop/mochi-settings.json", &fallback_bytes);
    let native_path = write_legacy_source(&base, "mochi-native/settings.json", &fallback_bytes);

    assert!(initialize_shared_profile_at(&profile.path, &base).is_err());
    assert!(!profile.path.exists());
    assert!(!profile.root.join("settings-backups").exists());
    assert!(!file::suffix(&profile.path, ".lock").exists());
    assert_eq!(fs::read(&dev_path).unwrap(), corrupt_bytes);
    assert_eq!(fs::read(&release_path).unwrap(), fallback_bytes);
    assert_eq!(fs::read(&native_path).unwrap(), fallback_bytes);
}

#[test]
fn background_image_roundtrips_bytes_through_one_canonical_data_url() {
    let profile = TestProfile::new("background-data-url");
    profile.write(&shared_values(&[("theme-storage", json!({
        "version": 0,
        "state": {"customBackground": {"enabled": true, "opacity": 70, "futureField": "retained"}}
    }))]));
    let source = profile.root.join("source.png");
    let image_bytes = b"\x89PNG\r\n\x1a\nroundtrip-fixture\x00\xff";
    fs::write(&source, image_bytes).unwrap();
    let data_url = background_image_data_url(&source).unwrap();
    assert!(data_url.starts_with("data:image/png;base64,"));
    let service = profile.service();
    service.set("background.imagePath", &data_url);
    service.flush().unwrap();
    fs::remove_file(&source).unwrap();

    let cached_path = service.background_image_path().unwrap().unwrap();
    assert!(cached_path.starts_with(profile.root.join("background-cache")));
    assert_eq!(fs::read(&cached_path).unwrap(), image_bytes);
    let reloaded = profile.service();
    assert_eq!(reloaded.background_image_path().unwrap(), Some(cached_path));
    assert_eq!(
        reloaded.get("background.imagePath").as_deref(),
        Some(data_url.as_str())
    );
    let theme = profile.bucket("theme-storage");
    assert_eq!(theme["state"]["customBackground"]["imageUrl"], data_url);
    assert_eq!(theme["state"]["customBackground"]["opacity"], 70);
    assert_eq!(
        theme["state"]["customBackground"]["futureField"],
        "retained"
    );
    assert!(!profile.values().contains_key("background.imagePath"));
    assert!(!reloaded.is_dirty());
}

#[test]
fn blank_secret_edit_preserves_unreadable_dpapi_ciphertext() {
    let profile = TestProfile::new("unreadable-secret");
    let ciphertext = "dpapi:bm90LWEtRFBBUEktYmxvYg==";
    profile.write(&shared_values(&[("mochi-ai", json!({
        "version": 3,
        "state": {"providers": [{"id": "unreadable", "name": "Keep this provider", "apiKey": ciphertext}]}
    }))]));
    let service = profile.service();
    let original_bytes = fs::read(&profile.path).unwrap();
    assert_eq!(service.get_secret("ai.providers.unreadable.apiKey"), None);

    service
        .set_secret("ai.providers.unreadable.apiKey", "")
        .unwrap();
    assert!(!service.is_dirty());
    assert_eq!(
        service.get("ai.providers.unreadable.apiKey").as_deref(),
        Some(ciphertext)
    );
    service.flush().unwrap();
    assert_eq!(fs::read(&profile.path).unwrap(), original_bytes);
    assert_eq!(
        profile.bucket("mochi-ai")["state"]["providers"][0]["apiKey"],
        ciphertext
    );
}
