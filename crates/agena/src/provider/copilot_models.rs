use serde::Deserialize;

use crate::model::{CapabilitySupport, ModelCapabilities, ModelMetadata};

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct CopilotModelExtension {
    #[serde(default)]
    model_picker_enabled: Option<bool>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    supported_endpoints: Vec<String>,
    #[serde(default)]
    policy: Option<CopilotModelPolicy>,
    #[serde(default)]
    capabilities: Option<CopilotModelCapabilities>,
}

impl CopilotModelExtension {
    pub(crate) fn visible(&self) -> bool {
        self.model_picker_enabled.unwrap_or(true)
            && !self
                .policy
                .as_ref()
                .and_then(|policy| policy.state.as_deref())
                .is_some_and(|state| state.eq_ignore_ascii_case("disabled"))
    }

    pub(crate) fn uses_messages_endpoint(&self) -> bool {
        self.supported_endpoints
            .iter()
            .any(|endpoint| endpoint == "/v1/messages")
    }

    pub(crate) fn metadata(&self, model_id: &str) -> ModelMetadata {
        let mut metadata = ModelMetadata::default();
        let Some(capabilities) = self.capabilities.as_ref() else {
            return self.apply_release_date(metadata, model_id);
        };
        if let Some(limits) = capabilities.limits.as_ref() {
            if let Some(value) = limits.max_context_window_tokens {
                metadata = metadata.with_context_window_tokens(clamp_u64_to_u32(value));
            }
            if let Some(value) = limits.max_prompt_tokens {
                metadata = metadata.with_max_input_tokens(clamp_u64_to_u32(value));
            }
            if let Some(value) = limits.max_output_tokens {
                metadata = metadata.with_max_output_tokens(clamp_u64_to_u32(value));
            }
        }
        self.apply_release_date(metadata, model_id)
    }

    pub(crate) fn capabilities(&self) -> ModelCapabilities {
        let mut result = ModelCapabilities::default();
        let Some(capabilities) = self.capabilities.as_ref() else {
            return result;
        };
        let Some(supports) = capabilities.supports.as_ref() else {
            return result;
        };

        result = result
            .with_streaming(support_bool(supports.streaming))
            .with_tool_calling(support_bool(supports.tool_calls))
            .with_structured_output(support_bool(supports.structured_outputs));

        let image = supports.vision.unwrap_or(false)
            || capabilities
                .limits
                .as_ref()
                .and_then(|limits| limits.vision.as_ref())
                .is_some_and(|vision| {
                    vision
                        .supported_media_types
                        .iter()
                        .any(|media_type| media_type.starts_with("image/"))
                });
        result = result.with_image_input(if image {
            CapabilitySupport::Supported
        } else {
            CapabilitySupport::Unsupported
        });

        let reasoning = supports.adaptive_thinking.unwrap_or(false)
            || supports
                .reasoning_effort
                .as_ref()
                .is_some_and(|efforts| !efforts.is_empty())
            || supports.max_thinking_budget.is_some()
            || supports.min_thinking_budget.is_some();
        result.with_reasoning(if reasoning {
            CapabilitySupport::Supported
        } else {
            CapabilitySupport::Unsupported
        })
    }

    fn apply_release_date(&self, metadata: ModelMetadata, model_id: &str) -> ModelMetadata {
        let Some(version) = self.version.as_ref().map(String::as_str) else {
            return metadata;
        };
        let release_date = version
            .strip_prefix(model_id)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .unwrap_or(version);
        metadata.with_release_date(release_date)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelPolicy {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelCapabilities {
    #[serde(default)]
    limits: Option<CopilotModelLimits>,
    #[serde(default)]
    supports: Option<CopilotModelSupports>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelLimits {
    #[serde(default)]
    max_context_window_tokens: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    max_prompt_tokens: Option<u64>,
    #[serde(default)]
    vision: Option<CopilotVisionLimits>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotVisionLimits {
    #[serde(default)]
    supported_media_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelSupports {
    #[serde(default)]
    adaptive_thinking: Option<bool>,
    #[serde(default)]
    max_thinking_budget: Option<u64>,
    #[serde(default)]
    min_thinking_budget: Option<u64>,
    #[serde(default)]
    reasoning_effort: Option<Vec<String>>,
    #[serde(default)]
    streaming: Option<bool>,
    #[serde(default)]
    structured_outputs: Option<bool>,
    #[serde(default)]
    tool_calls: Option<bool>,
    #[serde(default)]
    vision: Option<bool>,
}

fn support_bool(value: Option<bool>) -> CapabilitySupport {
    match value {
        Some(true) => CapabilitySupport::Supported,
        Some(false) => CapabilitySupport::Unsupported,
        None => CapabilitySupport::Unknown,
    }
}

fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct Fixture {
        id: String,
        #[serde(flatten)]
        copilot: CopilotModelExtension,
    }

    #[test]
    fn parses_copilot_model_extension() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "model_picker_enabled": true,
            "id": "claude-sonnet-4.5",
            "name": "Claude Sonnet 4.5",
            "version": "claude-sonnet-4.5-2026-01-15",
            "supported_endpoints": ["/v1/messages"],
            "capabilities": {
                "limits": {
                    "max_context_window_tokens": 144000,
                    "max_output_tokens": 64000,
                    "max_prompt_tokens": 128000,
                    "vision": {
                        "supported_media_types": ["image/png"]
                    }
                },
                "supports": {
                    "adaptive_thinking": true,
                    "streaming": true,
                    "structured_outputs": false,
                    "tool_calls": true
                }
            }
        }))
        .expect("fixture should parse");

        assert_eq!(fixture.id, "claude-sonnet-4.5");
        assert!(fixture.copilot.visible());
        assert!(fixture.copilot.uses_messages_endpoint());

        let metadata = fixture.copilot.metadata(fixture.id.as_str());
        assert_eq!(metadata.limits.context_window_tokens, Some(144_000));
        assert_eq!(metadata.limits.max_input_tokens, Some(128_000));
        assert_eq!(metadata.limits.max_output_tokens, Some(64_000));
        assert_eq!(metadata.release_date.as_deref(), Some("2026-01-15"));

        let capabilities = fixture.copilot.capabilities();
        assert_eq!(capabilities.streaming, CapabilitySupport::Supported);
        assert_eq!(capabilities.tool_calling, CapabilitySupport::Supported);
        assert_eq!(
            capabilities.structured_output,
            CapabilitySupport::Unsupported
        );
        assert_eq!(capabilities.image_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.reasoning, CapabilitySupport::Supported);
    }

    #[test]
    fn hides_disabled_copilot_models() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "model_picker_enabled": true,
            "id": "disabled-model",
            "policy": {
                "state": "disabled"
            }
        }))
        .expect("fixture should parse");

        assert!(!fixture.copilot.visible());

        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "model_picker_enabled": false,
            "id": "hidden-model"
        }))
        .expect("fixture should parse");

        assert!(!fixture.copilot.visible());
    }
}
