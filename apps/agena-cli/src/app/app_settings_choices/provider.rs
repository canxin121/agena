impl App {
    pub(in crate::app) fn open_provider_default_wizard(&mut self) {
        let provider_id = self
            .backend
            .config_json_sources()
            .ok()
            .and_then(|sources| get_json_path(&sources.effective, Some("providers.default")).ok())
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        let draft = ProviderDefaultWizardDraft {
            provider_id,
            ..Default::default()
        };
        self.open_provider_default_provider_step(draft);
    }

    pub(in crate::app) fn open_provider_default_provider_step(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        let providers = self.configured_defaultable_provider_summaries();
        if providers.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-default-no-providers",
            ));
            return false;
        }
        let items = providers
            .iter()
            .map(|provider| {
                choice_item(
                    provider.provider_id.clone(),
                    provider_default_route_summary(&self.i18n, provider),
                )
            })
            .collect();
        self.open_provider_default_choice_overlay(
            "overlay-provider-default-provider-title",
            "overlay-provider-default-provider-prompt",
            Editor::from_text(draft.provider_id.clone()),
            items,
            ProviderDefaultWizardStep::Provider,
            draft,
            ChoiceOverlayStyle::SearchableSelect,
        )
    }

    pub(in crate::app) fn open_provider_default_adapter_step(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        let Some(provider) = self.configured_provider_summary(draft.provider_id.as_str()) else {
            self.flash_warning(self.i18n.text_args(
                "flash-provider-default-provider-missing",
                &crate::fl_args!("provider" => draft.provider_id.clone()),
            ));
            return false;
        };
        let mut items = provider
            .adapters
            .iter()
            .filter(|adapter| adapter.enabled)
            .map(|adapter| {
                choice_item(
                    adapter.adapter_id.clone(),
                    provider_default_adapter_detail(&self.i18n, adapter.configured_model_count),
                )
            })
            .collect::<Vec<_>>();
        if items.is_empty()
            && let Some(adapter) = provider
                .defaults
                .adapter
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        {
            items.push(choice_item(
                adapter.to_owned(),
                ui_text::t(
                    &self.i18n,
                    "settings-provider-default-current-adapter-detail",
                ),
            ));
        }
        let input = draft
            .adapter_id
            .clone()
            .or_else(|| provider.defaults.adapter.clone())
            .unwrap_or_default();
        self.open_provider_default_choice_overlay(
            "overlay-provider-default-adapter-title",
            "overlay-provider-default-adapter-prompt",
            Editor::from_text(input),
            items,
            ProviderDefaultWizardStep::Adapter,
            draft,
            ChoiceOverlayStyle::SearchableSelect,
        )
    }

    pub(in crate::app) fn open_provider_default_model_step(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        let Some(adapter_id) = draft.adapter_id.as_deref() else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-default-adapter-required",
            ));
            return false;
        };
        let Some(provider) = self.configured_provider_summary(draft.provider_id.as_str()) else {
            self.flash_warning(self.i18n.text_args(
                "flash-provider-default-provider-missing",
                &crate::fl_args!("provider" => draft.provider_id.clone()),
            ));
            return false;
        };
        let items = match self.provider_default_model_choice_items(
            provider.provider_id.as_str(),
            adapter_id,
            provider.defaults.adapter.as_deref(),
        ) {
            Ok(items) => items,
            Err(error) => {
                self.flash_warning(error);
                Vec::new()
            }
        };
        let input = draft
            .model_id
            .clone()
            .or_else(|| {
                (!provider.defaults.model.trim().is_empty())
                    .then(|| provider.defaults.model.clone())
            })
            .unwrap_or_default();
        self.open_provider_default_choice_overlay(
            "overlay-provider-default-model-title",
            "overlay-provider-default-model-prompt",
            Editor::from_text(input),
            items,
            ProviderDefaultWizardStep::Model,
            draft,
            ChoiceOverlayStyle::SearchableSelect,
        )
    }

    pub(in crate::app) fn open_provider_default_thinking_step_or_next(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        match self
            .provider_default_mode_choice_items(&draft, ProviderDefaultWizardStep::ThinkingMode)
        {
            Ok(items) if !items.is_empty() => self.open_provider_default_choice_overlay(
                "overlay-provider-default-thinking-title",
                "overlay-provider-default-thinking-prompt",
                Editor::from_text(
                    draft
                        .thinking_mode
                        .clone()
                        .unwrap_or_else(|| ui_text::t(&self.i18n, "value-default")),
                ),
                items,
                ProviderDefaultWizardStep::ThinkingMode,
                draft,
                ChoiceOverlayStyle::SearchableSelect,
            ),
            Ok(_) => self.open_provider_default_speed_step_or_finish(draft),
            Err(error) => {
                self.flash_warning(error);
                self.open_provider_default_speed_step_or_finish(draft)
            }
        }
    }

    pub(in crate::app) fn open_provider_default_speed_step_or_finish(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        match self.provider_default_mode_choice_items(&draft, ProviderDefaultWizardStep::SpeedMode)
        {
            Ok(items) if !items.is_empty() => self.open_provider_default_choice_overlay(
                "overlay-provider-default-speed-title",
                "overlay-provider-default-speed-prompt",
                Editor::from_text(
                    draft
                        .speed_mode
                        .clone()
                        .unwrap_or_else(|| ui_text::t(&self.i18n, "value-default")),
                ),
                items,
                ProviderDefaultWizardStep::SpeedMode,
                draft,
                ChoiceOverlayStyle::SearchableSelect,
            ),
            Ok(_) => self.finish_provider_default_wizard(draft),
            Err(error) => {
                self.flash_warning(error);
                self.finish_provider_default_wizard(draft)
            }
        }
    }

    pub(in crate::app) fn open_provider_default_choice_overlay(
        &mut self,
        title_key: &str,
        prompt_key: &str,
        input: Editor,
        items: Vec<ChoiceItem>,
        step: ProviderDefaultWizardStep,
        draft: ProviderDefaultWizardDraft,
        style: ChoiceOverlayStyle,
    ) -> bool {
        if items.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-provider-default-empty-step"));
            return false;
        }
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, title_key),
            ui_text::t(&self.i18n, prompt_key),
            input,
            items,
            ChoiceOverlayAction::ProviderDefaultWizard(step, draft),
            false,
            style,
        ));
        true
    }

    pub(in crate::app) fn configured_provider_summary(
        &self,
        provider_id: &str,
    ) -> Option<ProviderSummaryResource> {
        self.configured_defaultable_provider_summaries()
            .into_iter()
            .find(|provider| provider.provider_id == provider_id.trim())
    }

    pub(in crate::app) fn configured_defaultable_provider_summaries(
        &self,
    ) -> Vec<ProviderSummaryResource> {
        let active_provider_ids = self
            .backend
            .list_providers()
            .into_iter()
            .map(|provider| provider.provider_id)
            .collect::<HashSet<_>>();
        self.backend
            .list_configured_providers()
            .into_iter()
            .filter(|provider| active_provider_ids.contains(provider.provider_id.as_str()))
            .collect()
    }

    pub(in crate::app) fn provider_default_model_choice_items(
        &self,
        provider_id: &str,
        adapter_id: &str,
        default_adapter: Option<&str>,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match self.block_on_async(self.backend.list_provider_models(provider_id)) {
            Ok(models) => models
                .into_iter()
                .filter(|model| {
                    let model_adapter = model
                        .adapter_id
                        .as_ref()
                        .map(ToString::to_string)
                        .or_else(|| default_adapter.map(str::to_owned));
                    model_adapter.as_deref() == Some(adapter_id)
                })
                .map(|model| {
                    choice_item(
                        model.id.to_string(),
                        provider_default_model_detail(&self.i18n, &model),
                    )
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                let fallback = self.configured_provider_model_choice_items(provider_id, adapter_id);
                if fallback.is_empty() {
                    return Err(error);
                }
                fallback
            }
        };

        items.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(dedupe_choice_items(items))
    }

    pub(in crate::app) fn configured_provider_model_choice_items(
        &self,
        provider_id: &str,
        adapter_id: &str,
    ) -> Vec<ChoiceItem> {
        self.backend
            .configured_provider_adapter_models(Some(provider_id))
            .into_iter()
            .find(|models| models.adapter_id == adapter_id)
            .map(|adapter_models| {
                adapter_models
                    .models
                    .into_iter()
                    .map(|model| {
                        choice_item(
                            model.id.to_string(),
                            ui_text::t(&self.i18n, "overlay-provider-studio-configured"),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub(in crate::app) fn provider_default_mode_choice_items(
        &self,
        draft: &ProviderDefaultWizardDraft,
        step: ProviderDefaultWizardStep,
    ) -> UiResult<Vec<ChoiceItem>> {
        let Some(model) = provider_default_wizard_model_ref(draft) else {
            return Ok(Vec::new());
        };
        let request = RunOptions {
            model: Some(model),
            ..Default::default()
        };
        let rows = match step {
            ProviderDefaultWizardStep::ThinkingMode => {
                self.backend.runtime_thinking_mode_rows(&request)
            }
            ProviderDefaultWizardStep::SpeedMode => self.backend.runtime_speed_mode_rows(&request),
            _ => Ok(Vec::new()),
        }
        .map_err(|error| error.to_string())?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = vec![choice_item_with_value(
            ui_text::t(&self.i18n, "value-default"),
            PROVIDER_DEFAULT_WIZARD_INHERIT,
            ui_text::t(&self.i18n, "settings-provider-default-mode-inherit-detail"),
        )];
        items.extend(match step {
            ProviderDefaultWizardStep::ThinkingMode => {
                inspector_rows_to_mode_choice_items(rows, ui_text::thinking_mode_display_value)
            }
            ProviderDefaultWizardStep::SpeedMode => {
                inspector_rows_to_mode_choice_items(rows, ui_text::speed_mode_display_value)
            }
            _ => inspector_rows_to_choice_items(rows),
        });
        Ok(items)
    }

    pub(in crate::app) fn commit_provider_default_wizard_step(
        &mut self,
        step: ProviderDefaultWizardStep,
        mut draft: ProviderDefaultWizardDraft,
        input: String,
    ) -> bool {
        let value = input.trim();
        if value.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-default-selection-required",
            ));
            return false;
        }

        match step {
            ProviderDefaultWizardStep::Provider => {
                draft.provider_id = value.to_owned();
                draft.adapter_id = None;
                draft.model_id = None;
                draft.thinking_mode = None;
                draft.speed_mode = None;
                self.open_provider_default_adapter_step(draft)
            }
            ProviderDefaultWizardStep::Adapter => {
                draft.adapter_id = Some(value.to_owned());
                draft.model_id = None;
                draft.thinking_mode = None;
                draft.speed_mode = None;
                self.open_provider_default_model_step(draft)
            }
            ProviderDefaultWizardStep::Model => {
                draft.model_id = Some(value.to_owned());
                draft.thinking_mode = None;
                draft.speed_mode = None;
                self.open_provider_default_thinking_step_or_next(draft)
            }
            ProviderDefaultWizardStep::ThinkingMode => {
                draft.thinking_mode = provider_default_wizard_optional_value(value);
                self.open_provider_default_speed_step_or_finish(draft)
            }
            ProviderDefaultWizardStep::SpeedMode => {
                draft.speed_mode = provider_default_wizard_optional_value(value);
                self.finish_provider_default_wizard(draft)
            }
        }
    }

    pub(in crate::app) fn finish_provider_default_wizard(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        match self.persist_provider_default_wizard(draft.clone()) {
            Ok(()) => {
                self.flash_success(self.i18n.text_args(
                    "flash-provider-default-updated",
                    &crate::fl_args!(
                        "provider" => draft.provider_id,
                        "model" => draft.model_id.unwrap_or_default(),
                    ),
                ));
                self.refresh_current_route_after_local_edit();
                true
            }
            Err(error) => {
                self.flash_error(error);
                false
            }
        }
    }

    pub(in crate::app) fn persist_provider_default_wizard(
        &self,
        draft: ProviderDefaultWizardDraft,
    ) -> UiResult<()> {
        let provider_id = draft.provider_id.trim();
        let adapter_id = draft
            .adapter_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ui_text::t(&self.i18n, "flash-provider-default-adapter-required"))?;
        let model_id = draft
            .model_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ui_text::t(&self.i18n, "flash-provider-default-model-required"))?;
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let defaults_path = provider_defaults_settings_path(provider_id);
        let mut defaults = get_json_path(&sources.file, Some(defaults_path.as_str()))
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        defaults.insert(
            "adapter".to_string(),
            JsonValue::String(adapter_id.to_owned()),
        );
        defaults.insert("model".to_string(), JsonValue::String(model_id.to_owned()));
        set_optional_string_object_value(
            &mut defaults,
            "thinking_mode",
            draft.thinking_mode.as_deref(),
        );
        set_optional_string_object_value(&mut defaults, "speed_mode", draft.speed_mode.as_deref());

        self.block_on_async(
            self.backend
                .set_config_setting(defaults_path.as_str(), JsonValue::Object(defaults)),
        )?;
        self.block_on_async(self.backend.set_config_setting(
            "providers.default",
            JsonValue::String(provider_id.to_owned()),
        ))?;
        Ok(())
    }
}
use crate::app::{
    App, ChoiceItem, ChoiceOverlayAction, ChoiceOverlayStyle, Editor, HashSet, JsonValue,
    PROVIDER_DEFAULT_WIZARD_INHERIT, ProviderDefaultWizardDraft, ProviderDefaultWizardStep,
    ProviderSummaryResource, RunOptions, UiResult, choice_item, choice_item_with_value,
    dedupe_choice_items, get_json_path, inspector_rows_to_choice_items,
    inspector_rows_to_mode_choice_items, provider_default_adapter_detail,
    provider_default_model_detail, provider_default_route_summary,
    provider_default_wizard_model_ref, provider_default_wizard_optional_value,
    provider_defaults_settings_path, set_optional_string_object_value, ui_text,
};
