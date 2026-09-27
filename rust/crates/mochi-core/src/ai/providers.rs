//! 原生服务方配置。列表只保存非敏感字段，密钥则通过 SettingsService 使用 DPAPI 分别加密存储。
use super::models::AiProvider;
use crate::settings::SettingsService;
use anyhow::{bail, Result};
const LIST: &str = "ai.providers";
const ACTIVE: &str = "ai.currentProviderId";
fn key(id: &str) -> String {
    format!("ai.providers.{id}.apiKey")
}
pub fn list(settings: &SettingsService) -> Vec<AiProvider> {
    if let Some(raw) = settings.get(LIST) {
        let mut providers: Vec<AiProvider> = serde_json::from_str(&raw).unwrap_or_default();
        for p in &mut providers {
            p.api_key = settings.get_secret(&key(&p.id)).unwrap_or_default();
        }
        return providers;
    }
    match (
        settings.get("ai.provider.baseUrl"),
        settings.get("ai.provider.model"),
    ) {
        (Some(base_url), Some(model))
            if !base_url.trim().is_empty() && !model.trim().is_empty() =>
        {
            vec![AiProvider {
                id: "default".into(),
                name: settings
                    .get("ai.provider.name")
                    .unwrap_or_else(|| "默认".into()),
                base_url,
                model,
                api_key: settings
                    .get_secret("ai.provider.apiKey")
                    .unwrap_or_default(),
                stream: true,
                protocol: "openai-completions".into(),
            }]
        }
        _ => vec![],
    }
}
pub fn selected(settings: &SettingsService) -> Option<AiProvider> {
    let all = list(settings);
    let current = settings.get(ACTIVE);
    all.iter()
        .find(|p| Some(&p.id) == current.as_ref())
        .or_else(|| all.first())
        .cloned()
}
fn write(settings: &SettingsService, providers: &[AiProvider]) -> Result<()> {
    let metadata=providers.iter().map(|p|serde_json::json!({"id":p.id,"name":p.name,"baseUrl":p.base_url,"model":p.model,"stream":p.stream,"protocol":p.protocol})).collect::<Vec<_>>();
    for p in providers {
        settings.set_secret(&key(&p.id), &p.api_key)?;
    }
    settings.set(LIST, &serde_json::to_string(&metadata)?);
    settings.flush()
}
pub fn save(settings: &SettingsService, provider: AiProvider) -> Result<()> {
    if !matches!(
        provider.protocol.as_str(),
        "" | "openai-completions" | "anthropic-messages"
    ) {
        bail!("不支持的 AI API 协议")
    }
    if provider.name.trim().is_empty() || provider.model.trim().is_empty() {
        bail!("名称和模型不能为空")
    }
    if !provider.base_url.starts_with("https://") && !provider.base_url.starts_with("http://") {
        bail!("Base URL 必须为 HTTP/HTTPS 地址")
    }
    if provider.id.is_empty() {
        bail!("provider id 不能为空")
    }
    let mut providers = list(settings);
    let id = provider.id.clone();
    if let Some(old) = providers.iter_mut().find(|p| p.id == provider.id) {
        *old = provider;
    } else {
        providers.push(provider);
    }
    write(settings, &providers)?;
    if settings.get(ACTIVE).is_none() {
        select(settings, &id)?;
    }
    Ok(())
}
pub fn select(settings: &SettingsService, id: &str) -> Result<()> {
    if !list(settings).iter().any(|p| p.id == id) {
        bail!("提供商不存在")
    }
    settings.set(ACTIVE, id);
    settings.flush()
}
pub fn remove(settings: &SettingsService, id: &str) -> Result<()> {
    let mut providers = list(settings);
    providers.retain(|p| p.id != id);
    write(settings, &providers)?;
    settings.remove(&key(id));
    if settings.get(ACTIVE).as_deref() == Some(id) {
        if let Some(p) = providers.first() {
            settings.set(ACTIVE, &p.id);
        } else {
            settings.remove(ACTIVE);
        }
    }
    settings.flush()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_settings(tag: &str) -> (std::path::PathBuf, SettingsService) {
        let path = std::env::temp_dir().join(format!(
            "mochi-providers-{}-{tag}-{}.json",
            std::process::id(),
            crate::jstime::now_millis()
        ));
        let settings = SettingsService::new(Some(path.clone()));
        (path, settings)
    }

    fn provider(id: &str) -> AiProvider {
        AiProvider {
            id: id.into(),
            name: id.into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: format!("secret-test-key-{id}"),
            model: "local".into(),
            stream: true,
            protocol: "openai-completions".into(),
        }
    }

    #[test]
    fn anthropic_protocol_survives_shared_settings_and_provider_edits() {
        let (path, settings) = temp_settings("anthropic-protocol");
        let mut anthropic = provider("anthropic");
        anthropic.protocol = "anthropic-messages".into();
        save(&settings, anthropic.clone()).unwrap();
        drop(settings);

        let reloaded = SettingsService::new(Some(path.clone()));
        assert_eq!(selected(&reloaded), Some(anthropic.clone()));
        let shared: serde_json::Value =
            serde_json::from_str(&reloaded.get("mochi-ai").unwrap()).unwrap();
        assert_eq!(
            shared["state"]["providers"][0]["protocol"],
            "anthropic-messages"
        );
        anthropic.name = "Renamed Claude".into();
        save(&reloaded, anthropic.clone()).unwrap();
        drop(reloaded);

        let reopened = SettingsService::new(Some(path.clone()));
        assert_eq!(selected(&reopened), Some(anthropic));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_shared_provider_defaults_to_openai_protocol() {
        let (path, settings) = temp_settings("legacy-protocol");
        settings.set("mochi-ai", r#"{"state":{"providers":[{"id":"legacy","name":"Legacy","baseUrl":"http://localhost/v1","model":"local","apiKey":"","stream":true}]},"version":3}"#);
        settings.flush().unwrap();
        assert_eq!(selected(&settings).unwrap().protocol, "openai-completions");
        drop(settings);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unsupported_protocol_is_not_saved() {
        let (path, settings) = temp_settings("invalid-protocol");
        let mut invalid = provider("invalid");
        invalid.protocol = "unknown-protocol".into();
        assert!(save(&settings, invalid).is_err());
        assert!(list(&settings).is_empty());
        drop(settings);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(windows)]
    #[test]
    fn adding_providers_persists_all_fields_and_initial_selection() {
        let (path, settings) = temp_settings("add-reload");
        let first = provider("first");
        let second = provider("second");

        save(&settings, first.clone()).unwrap();
        save(&settings, second.clone()).unwrap();
        let metadata: serde_json::Value =
            serde_json::from_str(&settings.get(LIST).unwrap()).unwrap();
        assert!(metadata
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("apiKey").is_none()));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        for p in [&first, &second] {
            assert!(
                !on_disk.contains(&p.api_key),
                "API Key must not be stored as plaintext"
            );
            assert!(settings.get(&key(&p.id)).unwrap().starts_with("dpapi:"));
        }
        drop(settings);

        let reloaded = SettingsService::new(Some(path.clone()));
        assert_eq!(list(&reloaded), vec![first.clone(), second]);
        assert_eq!(reloaded.get(ACTIVE).as_deref(), Some("first"));
        assert_eq!(selected(&reloaded), Some(first));
        drop(reloaded);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(windows)]
    #[test]
    fn editing_provider_replaces_fields_and_secret_without_changing_selection() {
        let (path, settings) = temp_settings("edit-reload");
        let first = provider("first");
        let original = provider("second");
        save(&settings, first.clone()).unwrap();
        save(&settings, original.clone()).unwrap();
        select(&settings, &original.id).unwrap();
        drop(settings);

        let settings = SettingsService::new(Some(path.clone()));
        let edited = AiProvider {
            name: "已修改提供商".into(),
            base_url: "https://example.invalid/v1".into(),
            api_key: "secret-test-replacement-key".into(),
            model: "updated-model".into(),
            stream: false,
            ..original.clone()
        };
        save(&settings, edited.clone()).unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        for secret in [&first.api_key, &original.api_key, &edited.api_key] {
            assert!(
                !on_disk.contains(secret),
                "API Key must not be stored as plaintext"
            );
        }
        drop(settings);

        let reloaded = SettingsService::new(Some(path.clone()));
        assert_eq!(list(&reloaded), vec![first, edited.clone()]);
        assert_eq!(reloaded.get(ACTIVE).as_deref(), Some("second"));
        assert_eq!(selected(&reloaded), Some(edited));
        drop(reloaded);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn secret_is_not_in_metadata_and_removing_active_selects_remaining() {
        let path = std::env::temp_dir().join(format!(
            "mochi-providers-{}-{}.json",
            std::process::id(),
            crate::jstime::now_millis()
        ));
        let settings = SettingsService::new(Some(path.clone()));
        let provider = |id: &str| AiProvider {
            id: id.into(),
            name: id.into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: "secret-test-key".into(),
            model: "local".into(),
            stream: true,
            protocol: "openai-completions".into(),
        };
        save(&settings, provider("a")).unwrap();
        save(&settings, provider("b")).unwrap();
        assert!(!settings.get(LIST).unwrap().contains("secret-test-key"));
        assert_eq!(selected(&settings).unwrap().id, "a");
        remove(&settings, "a").unwrap();
        assert_eq!(selected(&settings).unwrap().id, "b");
        assert_eq!(selected(&settings).unwrap().api_key, "secret-test-key");
        drop(settings);
        let _ = std::fs::remove_file(path);
    }
}
