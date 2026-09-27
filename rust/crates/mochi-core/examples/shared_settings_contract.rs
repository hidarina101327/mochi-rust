//! 默认只读：逐项核对 Rust 设置映射与明确指定的
//! Electron 共享文件。`--exercise` 只允许用于测试目录。
use anyhow::{bail, Context, Result};
use mochi_core::settings::SettingsService;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = PathBuf::from(args.first().context("Provide an explicit settings file")?);
    let raw: BTreeMap<String, String> = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
    let settings = SettingsService::new(Some(file.clone()));
    let mappings: Vec<Value> =
        serde_json::from_str(include_str!("../../../../shared/settings-map.json"))?;
    let mut checked = 0;
    for mapping in mappings {
        let store = mapping["store"].as_str().unwrap();
        let native = mapping["nativeKey"].as_str().unwrap();
        let Some(stored) = raw.get(store) else {
            continue;
        };
        let path = mapping["path"].as_array().unwrap();
        let expected = if path.is_empty() {
            Some(stored.clone())
        } else {
            let mut value: Value = serde_json::from_str(stored)?;
            for part in path {
                value = value
                    .get(part.as_str().unwrap())
                    .cloned()
                    .unwrap_or(Value::Null);
            }
            match &value {
                Value::Null => None,
                Value::String(text) => Some(text.clone()),
                value => Some(value.to_string()),
            }
        };
        if settings.get(native) != expected {
            bail!("Setting projection mismatch for {native}");
        }
        checked += 1;
    }
    if args.iter().any(|arg| arg == "--exercise") {
        let temp = std::env::temp_dir().canonicalize()?;
        let parent = file
            .parent()
            .context("Settings file has no parent")?
            .canonicalize()?;
        if parent.parent() != Some(temp.as_path())
            || !parent
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mochi-settings-contract-")
        {
            bail!("Mutation is restricted to an isolated contract-test directory");
        }
        if settings
            .get_secret("ai.providers.shared-test.apiKey")
            .as_deref()
            != Some("node-contract-secret")
        {
            bail!("Rust could not decrypt the Electron-created credential");
        }
        settings.set("app.typography.fontSize", "29");
        settings.set("app.appearance.themeMode", "light");
        settings.set_secret("ai.providers.shared-test.apiKey", "rust-contract-secret")?;
        settings.flush()?;
    }
    println!(
        "{}",
        json!({ "mapped_fields_verified": checked, "exercise": args.iter().any(|arg| arg == "--exercise") })
    );
    Ok(())
}
