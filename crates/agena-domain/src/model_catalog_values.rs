use std::{borrow::Cow, cmp::Ordering, collections::BTreeMap};

use serde::{Deserialize, Serialize};

use crate::{
    AdapterId, ModelCapabilities, ModelId, ModelMetadata, ModelRef, ModelSpeedModeRequestOverride,
    ProviderId, ReasoningEffort, ThinkingRequest,
};

macro_rules! define_model_mode {
    ($name:ident, fields { $($extra_fields:tt)* }, init { $($extra_init:tt)* }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        pub struct $name {
            #[serde(default, rename = "default", skip_serializing_if = "std::ops::Not::not")]
            pub is_default: bool,
            pub display_name: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub description: Option<String>,
            $($extra_fields)*
            #[serde(default, skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty")]
            pub request_override: ModelSpeedModeRequestOverride,
            #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
            pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    is_default: false,
                    display_name: None,
                    description: None,
                    $($extra_init)*
                    request_override: ModelSpeedModeRequestOverride::default(),
                    adapter_overrides: BTreeMap::new(),
                }
            }
        }
    };
}

define_model_mode!(
    ModelThinkingMode,
    fields {
        /// Stable selector for modes whose identity cannot be derived from the request itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub preset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub thinking: Option<ThinkingRequest>,
    },
    init { preset: None, thinking: None, }
);

define_model_mode!(ModelSpeedMode, fields {}, init {});

impl ModelThinkingMode {
    /// Returns the selector exposed to users and persisted in execution preferences.
    pub fn selector(&self) -> Option<Cow<'_, str>> {
        if let Some(name) = self
            .preset
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return Some(Cow::Borrowed(name));
        }
        match self.thinking.as_ref() {
            Some(ThinkingRequest::Disabled) => Some(Cow::Borrowed("off")),
            Some(ThinkingRequest::Effort { effort })
            | Some(ThinkingRequest::Adaptive {
                effort: Some(effort),
                ..
            }) => Some(Cow::Borrowed(effort.as_ref())),
            Some(ThinkingRequest::Budget { .. })
            | Some(ThinkingRequest::Adaptive { effort: None, .. })
            | None => None,
        }
    }

    pub fn has_invalid_custom_preset(&self) -> bool {
        self.preset.is_some()
            && matches!(
                self.thinking,
                Some(ThinkingRequest::Disabled)
                    | Some(ThinkingRequest::Effort { .. })
                    | Some(ThinkingRequest::Adaptive {
                        effort: Some(_),
                        ..
                    })
            )
    }
}

/// Orders thinking modes by reasoning strength and always puts an explicit disabled mode first.
pub fn compare_thinking_mode_strength(
    left: &ModelThinkingMode,
    right: &ModelThinkingMode,
) -> Ordering {
    thinking_mode_strength(left)
        .cmp(&thinking_mode_strength(right))
        .then_with(|| left.selector().cmp(&right.selector()))
}

fn thinking_mode_strength(mode: &ModelThinkingMode) -> (u8, u32) {
    match mode.thinking.as_ref() {
        Some(ThinkingRequest::Disabled) => (0, 0),
        Some(ThinkingRequest::Effort { effort })
        | Some(ThinkingRequest::Adaptive {
            effort: Some(effort),
            ..
        }) => (reasoning_effort_tier(*effort), 0),
        Some(ThinkingRequest::Budget { budget_tokens }) => (
            mode.selector()
                .as_deref()
                .and_then(thinking_mode_name_tier)
                .unwrap_or(3),
            *budget_tokens,
        ),
        Some(ThinkingRequest::Adaptive { effort: None, .. }) | None => (
            mode.selector()
                .as_deref()
                .and_then(thinking_mode_name_tier)
                .unwrap_or(3),
            0,
        ),
    }
}

fn thinking_mode_name_tier(name: &str) -> Option<u8> {
    name.to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .find_map(|token| match token {
            "no" | "none" | "off" | "disabled" => Some(0),
            "minimal" => Some(1),
            "low" => Some(2),
            "medium" => Some(3),
            "high" => Some(4),
            "xhigh" => Some(5),
            "max" | "maximum" => Some(6),
            _ => None,
        })
}

fn reasoning_effort_tier(effort: ReasoningEffort) -> u8 {
    match effort {
        ReasoningEffort::Minimal => 1,
        ReasoningEffort::Low => 2,
        ReasoningEffort::Medium => 3,
        ReasoningEffort::High => 4,
        ReasoningEffort::Xhigh => 5,
        ReasoningEffort::Max => 6,
    }
}

/// A configured provider/adapter/model route and its stable catalog metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Model {
    pub provider_id: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<AdapterId>,
    pub id: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_model_id: Option<ModelId>,
    pub display_name: Option<String>,
    #[serde(default = "default_native_compaction")]
    pub native_compaction: bool,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default, skip_serializing_if = "ModelMetadata::is_empty")]
    pub metadata: ModelMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_modes: Vec<ModelThinkingMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ModelSpeedMode>,
}

impl Model {
    pub fn new(provider_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider_id: ProviderId::new(provider_id),
            adapter_id: None,
            id: ModelId::new(id),
            catalog_model_id: None,
            display_name: None,
            native_compaction: true,
            capabilities: ModelCapabilities::default(),
            metadata: ModelMetadata::default(),
            thinking_modes: Vec::new(),
            speed_modes: BTreeMap::new(),
        }
    }

    pub fn reference(&self) -> ModelRef {
        ModelRef {
            provider_id: self.provider_id.clone(),
            adapter_id: self.adapter_id.clone(),
            model_id: self.id.clone(),
        }
    }

    pub fn using_thinking_modes(mut self, thinking_modes: Vec<ModelThinkingMode>) -> Self {
        self.thinking_modes = thinking_modes;
        self
    }

    pub fn using_thinking_mode(mut self, thinking_mode: ModelThinkingMode) -> Self {
        self.thinking_modes.push(thinking_mode);
        self
    }
}

const fn default_native_compaction() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{Model, ModelThinkingMode, compare_thinking_mode_strength};
    use crate::{ReasoningEffort, ThinkingRequest};

    #[test]
    fn native_compaction_is_enabled_by_default_and_always_serialized() {
        let model = Model::new("provider", "model");
        assert!(model.native_compaction);
        assert_eq!(
            serde_json::to_value(&model).unwrap()["native_compaction"],
            true
        );
        let mut disabled = model;
        disabled.native_compaction = false;
        assert_eq!(
            serde_json::to_value(disabled).unwrap()["native_compaction"],
            false
        );
    }

    #[test]
    fn reasoning_selectors_sort_by_semantics() {
        let mut modes = [
            effort_mode(ReasoningEffort::Xhigh),
            effort_mode(ReasoningEffort::Low),
            ModelThinkingMode {
                thinking: Some(ThinkingRequest::Disabled),
                ..Default::default()
            },
            effort_mode(ReasoningEffort::High),
        ];
        modes.sort_by(compare_thinking_mode_strength);
        assert_eq!(
            modes
                .iter()
                .filter_map(|mode| mode.selector().map(|value| value.into_owned()))
                .collect::<Vec<_>>(),
            vec!["off", "low", "high", "xhigh"]
        );
    }

    fn effort_mode(effort: ReasoningEffort) -> ModelThinkingMode {
        ModelThinkingMode {
            thinking: Some(ThinkingRequest::Effort { effort }),
            ..Default::default()
        }
    }
}
