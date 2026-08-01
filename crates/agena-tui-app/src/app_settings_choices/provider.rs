impl App {
    pub(crate) fn current_provider_default_model_ref(&self) -> Option<ModelRef> {
        let provider_id = self
            .backend
            .config_json_sources()
            .ok()
            .and_then(|sources| get_json_path(&sources.effective, Some("providers.default")).ok())
            .and_then(|value| value.as_str().map(str::to_owned))?;
        let provider = self
            .backend
            .list_configured_providers()
            .into_iter()
            .find(|provider| provider.provider_id == provider_id)?;
        let model_id = provider.defaults.model.trim();
        if model_id.is_empty() {
            return None;
        }
        Some(
            provider
                .defaults
                .adapter
                .as_deref()
                .map(str::trim)
                .filter(|adapter_id| !adapter_id.is_empty())
                .map(|adapter_id| {
                    ModelRef::new_with_adapter(provider.provider_id.clone(), adapter_id, model_id)
                })
                .unwrap_or_else(|| ModelRef::new(provider.provider_id, model_id)),
        )
    }

    pub(crate) fn apply_provider_default_model(&mut self, model: ModelRef) -> bool {
        match self.persist_provider_default_model(&model) {
            Ok(()) => {
                self.flash_success(self.i18n.text_args(
                    "flash-provider-default-updated",
                    &agena_tui::fl_args!(
                        "provider" => model.provider_id.to_string(),
                        "model" => model.model_id.to_string(),
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

    pub(crate) fn persist_provider_default_model(&self, model: &ModelRef) -> UiResult<()> {
        let provider_id = model.provider_id.to_string();
        let sources = self
            .backend
            .config_json_sources()
            .map_err(crate::UiFailure::internal)?;
        let defaults_path = provider_defaults_settings_path(provider_id.as_str());
        let defaults = get_json_path(&sources.file, Some(defaults_path.as_str()))
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let defaults = provider_defaults_with_model(defaults, model);

        self.block_on_async(
            self.backend
                .set_config_setting(defaults_path.as_str(), JsonValue::Object(defaults)),
        )?;
        self.block_on_async(
            self.backend
                .set_config_setting("providers.default", JsonValue::String(provider_id)),
        )?;
        Ok(())
    }
}

fn provider_defaults_with_model(
    mut defaults: JsonMap<String, JsonValue>,
    model: &ModelRef,
) -> JsonMap<String, JsonValue> {
    if let Some(adapter_id) = model.adapter_id.as_ref().map(ToString::to_string) {
        defaults.insert("adapter".to_string(), JsonValue::String(adapter_id));
    } else {
        defaults.remove("adapter");
    }
    defaults.insert(
        "model".to_string(),
        JsonValue::String(model.model_id.to_string()),
    );
    defaults.remove("thinking_mode");
    defaults.remove("speed_mode");
    defaults.remove("verbosity");
    defaults.remove("parallel_tool_calls");
    defaults
}

#[cfg(test)]
mod tests;

use crate::{
    App, JsonMap, JsonValue, ModelRef, UiResult, get_json_path, provider_defaults_settings_path,
};
