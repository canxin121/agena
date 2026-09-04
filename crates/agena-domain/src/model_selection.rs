use serde::{Deserialize, Serialize};

/// Optional provider, model, and inference-mode selection.
///
/// This value is independent of Agena's identity, tool capability boundary,
/// and permission policy. It is used for an explicit delegated-run override
/// without creating a new kind of agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelSelectionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

/// A concrete model selection used by the permission approval model. Variant
/// fields live beside the provider/adapter/model identity so selecting an
/// approval model is one atomic model-plus-variant choice without changing
/// `ModelRef`'s identity semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalModelSelection {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl ApprovalModelSelection {
    pub fn from_model_ref(model: &crate::ModelRef) -> Self {
        Self {
            provider: model.provider_id.to_string(),
            adapter: model.adapter_id.as_ref().map(ToString::to_string),
            model: model.model_id.to_string(),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            parallel_tool_calls: None,
        }
    }

    pub fn model_ref(&self) -> Result<crate::ModelRef, crate::IdentifierError> {
        match self.adapter.as_deref() {
            Some(adapter_id) => crate::ModelRef::try_new_with_adapter(
                self.provider.clone(),
                adapter_id,
                self.model.clone(),
            ),
            None => crate::ModelRef::try_new(self.provider.clone(), self.model.clone()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.provider.trim().is_empty() || self.model.trim().is_empty()
    }
}

impl ModelSelectionConfig {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.adapter.is_none()
            && self.model.is_none()
            && self.thinking_mode.is_none()
            && self.speed_mode.is_none()
            && self.verbosity.is_none()
            && self.parallel_tool_calls.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::ApprovalModelSelection;
    use crate::ModelRef;

    #[test]
    fn approval_model_selection_uses_current_model_selection_keys() {
        let selection: ApprovalModelSelection = serde_json::from_value(serde_json::json!({
            "provider": "openai",
            "adapter": "responses",
            "model": "gpt-5"
        }))
        .expect("current approval model selection should deserialize");

        assert_eq!(
            selection.model_ref().unwrap(),
            ModelRef::new_with_adapter("openai", "responses", "gpt-5")
        );
        assert!(selection.thinking_mode.is_none());
        assert!(selection.speed_mode.is_none());
        assert!(selection.verbosity.is_none());
    }

    #[test]
    fn approval_model_selection_rejects_removed_id_keys() {
        let error = serde_json::from_value::<ApprovalModelSelection>(serde_json::json!({
            "provider_id": "openai",
            "adapter_id": "responses",
            "model_id": "gpt-5"
        }))
        .expect_err("removed approval-model id keys must be rejected");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn approval_model_selection_round_trips_variants() {
        let selection = ApprovalModelSelection {
            provider: "openai".to_owned(),
            adapter: Some("responses".to_owned()),
            model: "gpt-5".to_owned(),
            thinking_mode: Some("high".to_owned()),
            speed_mode: Some("fast".to_owned()),
            verbosity: Some("compact".to_owned()),
            parallel_tool_calls: Some(false),
        };

        let encoded = serde_json::to_value(&selection).expect("selection should serialize");
        let decoded: ApprovalModelSelection =
            serde_json::from_value(encoded).expect("selection should deserialize");
        assert_eq!(decoded, selection);
    }
}
