use std::sync::Arc;

use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::model::{CapabilitySupport, Model, ModelCapabilities, ModelId};

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelProvider, PromptCacheShape,
    StreamResumePolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOverrideMatchMode {
    #[default]
    Exact,
    Prefix,
    Contains,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCapabilityPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calling: Option<CapabilitySupport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<CapabilitySupport>,
}

impl ModelCapabilityPatch {
    pub fn is_empty(&self) -> bool {
        self.text_input.is_none()
            && self.image_input.is_none()
            && self.document_input.is_none()
            && self.audio_input.is_none()
            && self.video_input.is_none()
            && self.file_input.is_none()
            && self.tool_calling.is_none()
            && self.streaming.is_none()
    }

    pub fn apply_to(&self, mut capabilities: ModelCapabilities) -> ModelCapabilities {
        if let Some(value) = self.text_input {
            capabilities.text_input = value;
        }
        if let Some(value) = self.image_input {
            capabilities.image_input = value;
        }
        if let Some(value) = self.document_input {
            capabilities.document_input = value;
        }
        if let Some(value) = self.audio_input {
            capabilities.audio_input = value;
        }
        if let Some(value) = self.video_input {
            capabilities.video_input = value;
        }
        if let Some(value) = self.file_input {
            capabilities.file_input = value;
        }
        if let Some(value) = self.tool_calling {
            capabilities.tool_calling = value;
        }
        if let Some(value) = self.streaming {
            capabilities.streaming = value;
        }
        capabilities
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityOverrideRule {
    pub model: String,
    #[serde(default, rename = "match")]
    pub match_mode: CapabilityOverrideMatchMode,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl ProviderCapabilityOverrideRule {
    pub fn matches(&self, model: &str) -> bool {
        let rule_model = normalize_model(self.model.as_str());
        let candidate = normalize_model(model);
        match self.match_mode {
            CapabilityOverrideMatchMode::Exact => candidate == rule_model,
            CapabilityOverrideMatchMode::Prefix => candidate.starts_with(rule_model.as_str()),
            CapabilityOverrideMatchMode::Contains => candidate.contains(rule_model.as_str()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if normalize_model(self.model.as_str()).is_empty() {
            return Err("capability override model matcher cannot be empty".to_owned());
        }
        if self.capabilities.is_empty() {
            return Err(format!(
                "capability override for model `{}` must set at least one capability field",
                self.model
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct CapabilityOverrideProvider {
    target: Arc<dyn ModelProvider>,
    rules: Arc<[ProviderCapabilityOverrideRule]>,
}

impl CapabilityOverrideProvider {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        target: Arc<dyn ModelProvider>,
        rules: Vec<ProviderCapabilityOverrideRule>,
    ) -> Arc<dyn ModelProvider> {
        if rules.is_empty() {
            target
        } else {
            Arc::new(Self {
                target,
                rules: Arc::from(rules),
            })
        }
    }

    fn apply_overrides(&self, model: &str, capabilities: ModelCapabilities) -> ModelCapabilities {
        self.rules
            .iter()
            .filter(|rule| rule.matches(model))
            .fold(capabilities, |current, rule| {
                rule.capabilities.apply_to(current)
            })
    }
}

#[async_trait]
impl ModelProvider for CapabilityOverrideProvider {
    fn id(&self) -> &str {
        self.target.id()
    }

    fn default_model(&self) -> &ModelId {
        self.target.default_model()
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.apply_overrides(model.as_str(), self.target.model_capabilities(model))
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        self.target.model_metadata(model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.target.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.target.supports_prompt_continuation(model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.target.prompt_cache_shape(model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        for model in &mut models {
            let fallback = self.target.model_capabilities(&model.id);
            let base = model.capabilities.clone().with_fallbacks_from(&fallback);
            model.capabilities = self.apply_overrides(model.id.as_str(), base);
        }
        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.target.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.target.complete_stream(request).await
    }
}

fn normalize_model(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_applies_only_selected_fields() {
        let base = ModelCapabilities::default()
            .with_image_input(CapabilitySupport::Supported)
            .with_streaming(CapabilitySupport::Supported);
        let patch = ModelCapabilityPatch {
            image_input: Some(CapabilitySupport::Unsupported),
            tool_calling: Some(CapabilitySupport::Supported),
            ..ModelCapabilityPatch::default()
        };

        let updated = patch.apply_to(base);
        assert_eq!(updated.image_input, CapabilitySupport::Unsupported);
        assert_eq!(updated.tool_calling, CapabilitySupport::Supported);
        assert_eq!(updated.streaming, CapabilitySupport::Supported);
    }

    #[test]
    fn override_rule_matches_according_to_match_mode() {
        let prefix = ProviderCapabilityOverrideRule {
            model: "gpt-4o".to_owned(),
            match_mode: CapabilityOverrideMatchMode::Prefix,
            capabilities: ModelCapabilityPatch {
                image_input: Some(CapabilitySupport::Supported),
                ..ModelCapabilityPatch::default()
            },
        };
        assert!(prefix.matches("gpt-4o-mini"));
        assert!(!prefix.matches("claude-sonnet-4-5"));
    }
}
