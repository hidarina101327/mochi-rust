use super::*;

#[test]
fn provider_preset_fills_fields_and_drops_cross_service_credentials() {
    let mut form = Form::new(AiProvider {
        id: "test".into(),
        name: "old".into(),
        base_url: "https://old.example/v1".into(),
        model: "old-model".into(),
        api_key: "private-key".into(),
        stream: false,
        protocol: "openai-completions".into(),
    });
    let index = mochi_core::ai::provider_presets::all()
        .iter()
        .position(|p| p.name == "DeepSeek")
        .unwrap();
    let preset = &mochi_core::ai::provider_presets::all()[index];
    assert!(form.apply_preset(Some(index)));
    assert_eq!(form.value().name, preset.name);
    assert_eq!(form.value().base_url, preset.base_url);
    assert_eq!(form.value().model, preset.model);
    assert_eq!(form.value().protocol, preset.protocol);
    assert!(form.value().api_key.is_empty());
    assert!(!form.stream);
    form.fields[3].set_text("new-key");
    assert!(form.apply_preset(Some(index)));
    assert_eq!(form.value().api_key, "new-key");
    form.fields[2].set_text("my-model");
    assert!(form.preset_label().contains("已修改"));
    assert!(form.apply_preset(None));
    assert_eq!(form.value().model, "my-model");
    assert!(form.preset_label().starts_with("自定义"));
    let unsupported = mochi_core::ai::provider_presets::all()
        .iter()
        .position(|p| !p.supported)
        .unwrap();
    assert!(!form.apply_preset(Some(unsupported)));
    assert_eq!(form.value().api_key, "new-key");
}

#[test]
fn anthropic_messages_preset_is_selectable_and_protocol_is_persisted() {
    let mut form = Form::new(AiProvider {
        id: "claude".into(),
        name: "旧配置".into(),
        base_url: "https://old.example/v1".into(),
        model: "old-model".into(),
        api_key: "anthropic-key".into(),
        stream: true,
        protocol: OPENAI_PROTOCOL.into(),
    });
    let index = mochi_core::ai::provider_presets::all()
        .iter()
        .position(|p| p.name == "Anthropic Official")
        .expect("the official Anthropic Messages preset is bundled");
    let preset = &mochi_core::ai::provider_presets::all()[index];
    assert!(preset.supported);
    assert_eq!(preset.protocol, ANTHROPIC_PROTOCOL);
    assert!(form.apply_preset(Some(index)));
    let value = form.value();
    assert_eq!(value.protocol, ANTHROPIC_PROTOCOL);
    assert_eq!(value.base_url, preset.base_url);
    assert_eq!(value.model, preset.model);
    assert!(
        value.api_key.is_empty(),
        "switching service must drop the old key"
    );
}

#[test]
fn switching_protocol_keeps_key_and_updates_only_builtin_defaults() {
    let mut form = Form::new(AiProvider {
        id: "claude".into(),
        base_url: OPENAI_BASE_URL.into(),
        model: OPENAI_DEFAULT_MODEL.into(),
        api_key: "shared-gateway-key".into(),
        protocol: OPENAI_PROTOCOL.into(),
        ..Default::default()
    });
    assert!(form.apply_protocol(ANTHROPIC_PROTOCOL));
    assert_eq!(form.value().protocol, ANTHROPIC_PROTOCOL);
    assert_eq!(form.value().base_url, ANTHROPIC_BASE_URL);
    assert_eq!(form.value().model, ANTHROPIC_DEFAULT_MODEL);
    assert!(
        form.value().api_key.is_empty(),
        "auto-switching endpoint must drop the old key"
    );

    form.fields[1].set_text("https://gateway.example/v1");
    form.fields[2].set_text("custom-model");
    form.fields[3].set_text("shared-gateway-key");
    assert!(form.apply_protocol(OPENAI_PROTOCOL));
    assert_eq!(form.value().base_url, "https://gateway.example/v1");
    assert_eq!(form.value().model, "custom-model");
    assert_eq!(form.value().api_key, "shared-gateway-key");
}

#[test]
fn merging_a_protocol_edit_keeps_concurrent_fields_and_saves_protocol() {
    let original = AiProvider {
        id: "provider".into(),
        name: "Original".into(),
        base_url: "https://gateway.example/v1".into(),
        model: "original-model".into(),
        api_key: "shared-key".into(),
        protocol: OPENAI_PROTOCOL.into(),
        stream: true,
    };
    let mut form = Form::new(original.clone());
    assert!(form.apply_protocol(ANTHROPIC_PROTOCOL));
    let mut current = original;
    current.name = "Renamed elsewhere".into();
    current.model = "new-model".into();
    let merged = form.merge_edited_fields(current);
    assert_eq!(merged.protocol, ANTHROPIC_PROTOCOL);
    assert_eq!(merged.name, "Renamed elsewhere");
    assert_eq!(merged.model, "new-model");
    assert_eq!(merged.api_key, "shared-key");
}

#[test]
fn stale_form_preserves_a_protocol_changed_concurrently() {
    let original = AiProvider {
        id: "provider".into(),
        name: "Original".into(),
        base_url: "https://gateway.example/v1".into(),
        model: "original-model".into(),
        api_key: "shared-key".into(),
        protocol: OPENAI_PROTOCOL.into(),
        stream: true,
    };
    let form = Form::new(original.clone());
    let mut current = original;
    current.protocol = ANTHROPIC_PROTOCOL.into();
    current.model = "new-model".into();

    let merged = form.merge_edited_fields(current);
    assert_eq!(merged.protocol, ANTHROPIC_PROTOCOL);
    assert_eq!(merged.model, "new-model");
    assert_eq!(merged.api_key, "shared-key");
}

#[test]
fn provider_endpoint_change_does_not_merge_a_concurrently_added_secret() {
    let mut original = AiProvider::default();
    original.base_url = "https://old.example/v1".into();
    original.protocol = OPENAI_PROTOCOL.into();
    let mut form = Form::new(original.clone());
    form.fields[1].set_text("https://new.example/v1");
    original.api_key = "concurrent-secret".into();
    assert!(form.merge_edited_fields(original).api_key.is_empty());
}
