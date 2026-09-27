//! 随程序打包且固定版本的服务方默认配置。不包含凭据，也不会联网查询。
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreset {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub protocol: String,
    pub supported: bool,
}

pub fn all() -> &'static [ProviderPreset] {
    static PRESETS: LazyLock<Vec<ProviderPreset>> = LazyLock::new(|| {
        #[derive(Deserialize)]
        struct Catalog {
            presets: Vec<ProviderPreset>,
        }
        serde_json::from_str::<Catalog>(include_str!("../../assets/provider-presets.json"))
            .expect("bundled provider catalog must be valid")
            .presets
    });
    &PRESETS
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_presets_have_unique_names_and_usable_compatible_defaults() {
        let mut names = std::collections::HashSet::new();
        assert!(all().iter().filter(|p| p.supported).count() >= 40);
        assert!(all()
            .iter()
            .any(|p| p.supported && p.protocol == "anthropic-messages"));
        for p in all() {
            assert!(names.insert(&p.name));
            if p.supported {
                assert!(matches!(
                    p.protocol.as_str(),
                    "openai-completions" | "anthropic-messages"
                ));
                assert!(p.base_url.starts_with("https://"));
                assert!(!p.model.is_empty());
                assert!(!p.base_url.contains(['{', '}', '<', '>']));
            }
        }
    }
}
