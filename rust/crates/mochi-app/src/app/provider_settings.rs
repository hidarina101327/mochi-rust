//! 管理 AI 服务配置，并处理连通性测试。
use super::*;

impl App {
    pub(super) fn reload_providers(&mut self) {
        self.prefs.providers.actions = self
            .settings
            .get("ai.actionPermissions")
            .map(|raw| mochi_core::ai::permission::ActionPermissions::from_setting(&raw))
            .unwrap_or_default();
        self.prefs.providers.inline_enabled = self.inline_completion_enabled();
        self.prefs.providers.all = mochi_core::ai::providers::list(&self.settings);
        self.prefs.providers.selected =
            mochi_core::ai::providers::selected(&self.settings).map(|p| p.id);
        self.prefs.providers.loaded = true;
        self.ai.panel.provider_missing = ai_runtime::load_provider(&self.settings).is_none();
    }

    pub(super) fn paint_provider_settings(&mut self, area: Rect, p: &Palette) {
        if !self.prefs.providers.loaded {
            self.reload_providers();
        }
        let s = &mut self.prefs.providers;
        s.interaction.tick(Self::now_ms());
        let mut lay = providers::layout(s, area);
        if s.scroll > lay.max_scroll() {
            s.scroll = lay.max_scroll();
            lay = providers::layout(s, area);
        }
        providers::paint(
            &mut self.list,
            area,
            s,
            &lay,
            self.focus == Focus::ProviderField,
            p,
        );
        self.prefs.provider_layout = lay;
    }

    pub(super) fn save_provider_form(&mut self) -> anyhow::Result<()> {
        let Some(form) = &self.prefs.providers.form else {
            return Ok(());
        };
        let current = mochi_core::ai::providers::list(&self.settings)
            .into_iter()
            .find(|provider| provider.id == form.id);
        let value = match current {
            Some(current) => form.merge_edited_fields(current),
            None if form.editing_existing => {
                anyhow::bail!("该提供商已在另一个窗口删除，请取消后重新添加")
            }
            None => form.value(),
        };
        mochi_core::ai::providers::save(&self.settings, value)?;
        // 保存会关闭编辑器。先停掉进行中的提供方探测，避免它完成时更新
        // 一个已经不存在的表单。
        self.cancel_provider_test();
        self.cancel_provider_models();
        self.prefs.providers.form = None;
        self.focus = Focus::Main;
        self.reload_providers();
        Ok(())
    }

    pub(super) fn on_provider_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.prefs.provider_layout.hit(x, y) else {
            return;
        };
        self.prefs.providers.interaction.reduce_motion =
            !platform::client_area_animations_enabled();
        self.prefs.providers.interaction.press(hit, Self::now_ms());
        let result = match hit {
            providers::Hit::Protocol => {
                if let Some(form) = &self.prefs.providers.form {
                    let current = providers::protocol_label(&form.protocol);
                    let mut items = Vec::with_capacity(providers::PROTOCOLS.len());
                    for (protocol, label) in providers::PROTOCOLS {
                        let mut item = MenuItem::new(
                            label,
                            MenuAction::SelectProviderProtocol(
                                form.id.clone(),
                                protocol.to_owned(),
                            ),
                        );
                        if label == current {
                            item = item.icon(Icon::CHECK);
                        }
                        items.push(item);
                    }
                    let anchor = self
                        .prefs
                        .provider_layout
                        .entries
                        .iter()
                        .find(|(_, entry)| *entry == hit)
                        .map(|(rect, _)| *rect)
                        .unwrap_or_else(|| self.renderer.viewport());
                    self.menu = Some(Menu::open_anchored(items, anchor, self.renderer.viewport()));
                }
                return;
            }
            providers::Hit::Preset => {
                if let Some(form) = &self.prefs.providers.form {
                    let mut items = vec![MenuItem::new(
                        "自定义",
                        MenuAction::SelectProviderPreset(form.id.clone(), None),
                    )];
                    for supported in [true, false] {
                        for (i, p) in mochi_core::ai::provider_presets::all()
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| p.supported == supported)
                        {
                            let label = if supported {
                                p.name.clone()
                            } else {
                                format!("{} · 暂不支持 {}", p.name, p.protocol)
                            };
                            items.push(
                                MenuItem::new(
                                    label,
                                    MenuAction::SelectProviderPreset(form.id.clone(), Some(i)),
                                )
                                .disabled(!supported),
                            );
                        }
                    }
                    let anchor = self
                        .prefs
                        .provider_layout
                        .entries
                        .iter()
                        .find(|(_, h)| *h == hit)
                        .unwrap()
                        .0;
                    self.menu = Some(Menu::open_searchable_anchored(
                        items,
                        anchor,
                        self.renderer.viewport(),
                        "搜索提供商",
                    ));
                }
                return;
            }
            providers::Hit::Options => {
                self.settings_overlay = Some(("ai".into(), "parameters".into()));
                self.prefs.scroll = 0.0;
                self.focus = Focus::Main;
                self.invalidate_main();
                Ok(())
            }
            providers::Hit::Permission(i) => {
                let action = mochi_core::ai::permission::AiToolAction::ALL[i];
                let allowed = !self.prefs.providers.actions.is_allowed(action);
                self.prefs.providers.actions.set(action, allowed);
                self.settings.set(
                    "ai.actionPermissions",
                    &self.prefs.providers.actions.to_setting(),
                );
                if let Some(s) = &self.ai.permissions {
                    s.set_action_permissions(self.prefs.providers.actions.clone());
                }
                self.settings.flush()
            }
            providers::Hit::ToggleInline => {
                let enabled = !self.prefs.providers.inline_enabled;
                self.set_inline_completion_enabled(enabled)
            }
            providers::Hit::Add => {
                self.cancel_provider_models();
                self.prefs.providers.form =
                    Some(providers::Form::new(mochi_core::ai::models::AiProvider {
                        id: uuid_v4(),
                        base_url: providers::OPENAI_BASE_URL.into(),
                        model: providers::OPENAI_DEFAULT_MODEL.into(),
                        stream: true,
                        protocol: providers::OPENAI_PROTOCOL.into(),
                        ..Default::default()
                    }));
                self.prefs.providers.scroll = 0.0;
                self.focus = Focus::ProviderField;
                Ok(())
            }
            providers::Hit::Edit(i) => {
                self.cancel_provider_models();
                if let Some(p) = self.prefs.providers.all.get(i).cloned() {
                    let mut form = providers::Form::new(p);
                    form.editing_existing = true;
                    self.prefs.providers.form = Some(form);
                    self.prefs.providers.scroll = 0.0;
                    self.focus = Focus::ProviderField;
                }
                Ok(())
            }
            providers::Hit::Field(i) => {
                if let (Some(form), Some(r)) = (
                    &mut self.prefs.providers.form,
                    self.prefs.provider_layout.field(i),
                ) {
                    form.focus = i;
                    if i == 3 {
                        form.fields[i].click_masked(x - r.left - 12.0, shift_down());
                    } else {
                        form.fields[i].click(x - r.left - 12.0, shift_down());
                    }
                    self.drag = Some(Drag {
                        target: DragTarget::ProviderFieldSelect,
                        grab_offset: 0.0,
                    });
                    self.focus = Focus::ProviderField;
                }
                Ok(())
            }
            providers::Hit::Stream => {
                if let Some(f) = self.prefs.providers.form.as_mut() {
                    f.stream = !f.stream;
                }
                Ok(())
            }
            providers::Hit::Cancel => {
                self.cancel_provider_models();
                self.prefs.providers.form = None;
                self.focus = Focus::Main;
                Ok(())
            }
            providers::Hit::Save => self.save_provider_form(),
            providers::Hit::Select(i) => {
                self.cancel_provider_test();
                self.cancel_provider_models();
                let p = self.prefs.providers.all.get(i).cloned();
                if let Some(p) = p {
                    let r = mochi_core::ai::providers::select(&self.settings, &p.id);
                    self.reload_providers();
                    r
                } else {
                    Ok(())
                }
            }
            providers::Hit::Delete(i) => {
                if let Some(p) = self.prefs.providers.all.get(i) {
                    self.dialog = Some(Dialog {
                        title: "删除 AI 提供商".into(),
                        description: format!("确定要删除 {} 吗？", p.name),
                        field: None,
                        error: String::new(),
                        note: None,
                        buttons: vec![
                            DialogButton {
                                label: "取消".into(),
                                kind: ButtonKind::Ghost,
                                action: DialogAction::Dismiss,
                            },
                            DialogButton {
                                label: "删除".into(),
                                kind: ButtonKind::Danger,
                                action: DialogAction::DeleteProvider(p.id.clone()),
                            },
                        ],
                        dismiss: DialogAction::Dismiss,
                        hover: None,
                    });
                    self.focus = Focus::Dialog;
                }
                Ok(())
            }
            providers::Hit::Test(i) => {
                if let Some(p) = self.prefs.providers.all.get(i).cloned() {
                    if self.prefs.providers.test_provider_id.as_deref() == Some(p.id.as_str()) {
                        self.cancel_provider_test();
                        self.prefs.providers.status = "已取消连接测试".into();
                    } else {
                        self.start_provider_test(p);
                    }
                }
                return;
            }
            providers::Hit::Models => {
                if self.prefs.providers.models_provider_id.is_some() {
                    self.cancel_provider_models();
                    self.prefs.providers.status = "已取消模型列表请求".into();
                } else if let Some(form) = self.prefs.providers.form.as_ref() {
                    self.start_provider_models(form.value());
                }
                return;
            }
        };
        self.prefs.providers.status = match result {
            Ok(()) if hit == providers::Hit::Save => "✓ 配置已保存".into(),
            Ok(()) if matches!(hit, providers::Hit::Select(_)) => "✓ 已切换当前提供商".into(),
            Ok(()) => String::new(),
            Err(e) => format!("操作失败：{e}"),
        };
    }

    pub(super) fn cancel_provider_test(&mut self) {
        if let Some(cancel) = self.prefs.provider_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.prefs.provider_rx = None;
        self.prefs.provider_generation = self.prefs.provider_generation.wrapping_add(1);
        self.prefs.providers.test_provider_id = None;
    }

    pub(super) fn start_provider_test(&mut self, provider: mochi_core::ai::models::AiProvider) {
        self.cancel_provider_models();
        self.cancel_provider_test();
        let generation = self.prefs.provider_generation;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = channel();
        self.prefs.provider_cancel = Some(cancel.clone());
        self.prefs.provider_rx = Some(rx);
        self.prefs.providers.test_provider_id = Some(provider.id.clone());
        self.prefs.providers.status = format!(
            "正在测试连接（最多 {} 秒）…",
            mochi_core::ai::service::CONNECTION_TIMEOUT_SECONDS
        );
        let hwnd = self.hwnd_raw;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mochi_core::ai::service::AiService::with_cancel_timeout(
                    provider,
                    cancel.clone(),
                    std::time::Duration::from_secs(
                        mochi_core::ai::service::CONNECTION_TIMEOUT_SECONDS,
                    ),
                )
                .test_connection()
            }))
            .unwrap_or_else(|_| {
                (
                    false,
                    Some("连接测试内部异常，网络线程已安全结束".to_owned()),
                )
            });
            if !cancel.load(std::sync::atomic::Ordering::Relaxed)
                && tx.send((generation, result.0, result.1)).is_ok()
            {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        platform::WM_APP_PROVIDER_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    pub fn take_provider_test(&mut self) {
        let Some(rx) = &self.prefs.provider_rx else {
            return;
        };
        let Ok((generation, ok, error)) = rx.try_recv() else {
            return;
        };
        if generation != self.prefs.provider_generation {
            return;
        }
        self.prefs.provider_rx = None;
        self.prefs.provider_cancel = None;
        self.prefs.providers.test_provider_id = None;
        self.prefs.providers.status = if ok {
            "连接成功".into()
        } else {
            format!(
                "连接失败：{}",
                error.unwrap_or_else(|| "provider 未返回可用响应".into())
            )
        };
    }

    pub(super) fn cancel_provider_models(&mut self) {
        if let Some(cancel) = self.prefs.provider_models_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.prefs.provider_models_rx = None;
        self.prefs.provider_models_generation =
            self.prefs.provider_models_generation.wrapping_add(1);
        self.prefs.providers.models_provider_id = None;
    }

    pub(super) fn start_provider_models(&mut self, provider: mochi_core::ai::models::AiProvider) {
        self.cancel_provider_test();
        self.cancel_provider_models();
        let generation = self.prefs.provider_models_generation;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = channel();
        let provider_id = provider.id.clone();
        let base_url = provider.base_url.clone();
        self.prefs.provider_models_cancel = Some(cancel.clone());
        self.prefs.provider_models_rx = Some(rx);
        self.prefs.providers.models_provider_id = Some(provider_id.clone());
        self.prefs.providers.status = format!(
            "正在获取模型列表（最多 {} 秒）…",
            mochi_core::ai::service::CONNECTION_TIMEOUT_SECONDS
        );
        let hwnd = self.hwnd_raw;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mochi_core::ai::service::AiService::with_cancel_timeout(
                    provider,
                    cancel.clone(),
                    std::time::Duration::from_secs(
                        mochi_core::ai::service::CONNECTION_TIMEOUT_SECONDS,
                    ),
                )
                .list_models()
                .map_err(|error| error.to_string())
            }))
            .unwrap_or_else(|_| Err("获取模型列表时网络线程发生内部异常".to_owned()));
            if !cancel.load(std::sync::atomic::Ordering::Relaxed)
                && tx.send((generation, provider_id, base_url, result)).is_ok()
            {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        platform::WM_APP_PROVIDER_MODELS_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    pub fn take_provider_models(&mut self) {
        let Some(rx) = &self.prefs.provider_models_rx else {
            return;
        };
        let Ok((generation, provider_id, base_url, result)) = rx.try_recv() else {
            return;
        };
        if generation != self.prefs.provider_models_generation {
            return;
        }
        self.prefs.provider_models_rx = None;
        self.prefs.provider_models_cancel = None;
        self.prefs.providers.models_provider_id = None;
        match result {
            Ok(models) => {
                let Some(form) = self
                    .prefs
                    .providers
                    .form
                    .as_ref()
                    .filter(|form| form.id == provider_id && form.value().base_url == base_url)
                else {
                    return;
                };
                let current = form.fields[2].text().trim().to_owned();
                let count = models.len();
                let items = models
                    .into_iter()
                    .map(|model| {
                        let mut item = MenuItem::new(
                            model.clone(),
                            MenuAction::SelectProviderModel(provider_id.clone(), model.clone()),
                        );
                        if current == model {
                            item = item.icon(Icon::CHECK);
                        }
                        item
                    })
                    .collect::<Vec<_>>();
                let anchor = self
                    .prefs
                    .provider_layout
                    .field(2)
                    .unwrap_or_else(|| self.renderer.viewport());
                self.menu = Some(Menu::open_anchored(items, anchor, self.renderer.viewport()));
                self.prefs.providers.status = format!("已获取 {count} 个模型，请选择");
            }
            Err(error) => {
                self.prefs.providers.status = format!("获取模型列表失败：{error}");
            }
        }
    }
}
