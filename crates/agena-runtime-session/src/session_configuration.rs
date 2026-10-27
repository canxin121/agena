use agena_domain::{ExecutionSelection, ModelRef, PermissionConfig, SessionAutoCompactionConfig};
use std::collections::BTreeMap;

/// Configuration consumed by the concrete session manager.
#[derive(Debug, Clone)]
/// Configuration of the runtime session manager.
pub struct RuntimeSessionManagerConfig {
    pub default_selection: ExecutionSelection,
    pub auxiliary_selection: ExecutionSelection,
    /// Project/global instructions. An explicit empty value disables inheritance.
    pub additional_system_prompt: Option<String>,
    /// Provider -> adapter/model route -> configured instructions, including empty values.
    pub model_additional_system_prompts: BTreeMap<String, BTreeMap<String, String>>,
    pub auto_title: agena_runtime_config::SessionAutoTitleConfig,
    pub permission: PermissionConfig,
    pub auto_compaction: SessionAutoCompactionConfig,
    pub max_concurrent_tools: usize,
    /// Cap on model turns within one stable run. `None` falls back to
    /// `DEFAULT_MAX_MODEL_TURNS` (500) in the supervised kernel run.
    pub max_turns: Option<usize>,
}

impl Default for RuntimeSessionManagerConfig {
    fn default() -> Self {
        Self {
            default_selection: Default::default(),
            auxiliary_selection: Default::default(),
            additional_system_prompt: None,
            model_additional_system_prompts: BTreeMap::new(),
            auto_title: Default::default(),
            permission: Default::default(),
            auto_compaction: Default::default(),
            max_concurrent_tools: 32,
            // Same default as `DEFAULT_MAX_MODEL_TURNS` in
            // the supervised kernel run (mirrors gemini's MAX_TURNS).
            max_turns: Some(500),
        }
    }
}

impl RuntimeSessionManagerConfig {
    /// Look up instructions for an already resolved provider/adapter/model route.
    pub fn model_additional_system_prompt(&self, model: &ModelRef) -> Option<&str> {
        let models = self
            .model_additional_system_prompts
            .get(model.provider_id.as_ref())?;
        let route = model
            .adapter_id
            .as_ref()
            .map(|adapter| format!("{adapter}/{}", model.model_id));
        models
            .get(route.as_deref().unwrap_or(model.model_id.as_ref()))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_prompt_lookup_preserves_empty_values_and_route_identity() {
        let config = RuntimeSessionManagerConfig {
            model_additional_system_prompts: BTreeMap::from([(
                "provider".into(),
                BTreeMap::from([
                    ("adapter_a/shared".into(), String::new()),
                    (
                        "adapter_b/shared".into(),
                        "Use the second adapter's instructions.".into(),
                    ),
                ]),
            )]),
            ..Default::default()
        };
        assert_eq!(
            config.model_additional_system_prompt(&ModelRef::new_with_adapter(
                "provider",
                "adapter_a",
                "shared",
            )),
            Some("")
        );
        assert_eq!(
            config.model_additional_system_prompt(&ModelRef::new_with_adapter(
                "provider",
                "adapter_b",
                "shared",
            )),
            Some("Use the second adapter's instructions.")
        );
        for model in [
            ModelRef::new("provider", "shared"),
            ModelRef::new_with_adapter("other", "adapter_a", "shared"),
            ModelRef::new_with_adapter("provider", "adapter_a", "other"),
        ] {
            assert_eq!(config.model_additional_system_prompt(&model), None);
        }
    }
}
