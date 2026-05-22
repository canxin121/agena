use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::model::{
    AdapterId, CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelPricing, ModelSpeedMode, ModelSpeedModeRequestOverride,
    ModelThinkingMode,
};

use super::core::ForwardingModelRuntime;
use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelRuntime, PromptCacheShape,
    StreamResumePolicy, ThinkingRequest,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InputCapabilityPatchBody {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported: Vec<ModelInputModality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<ModelInputModality>,
}

impl InputCapabilityPatchBody {
    pub fn is_empty(&self) -> bool {
        self.supported.is_empty() && self.unsupported.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputCapabilityPatch {
    Supported(Vec<ModelInputModality>),
    Patch(InputCapabilityPatchBody),
}

impl InputCapabilityPatch {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Supported(values) => values.is_empty(),
            Self::Patch(values) => values.is_empty(),
        }
    }

    fn supported_values(&self) -> Vec<ModelInputModality> {
        match self {
            Self::Supported(values) => values.clone(),
            Self::Patch(values) => values.supported.clone(),
        }
    }

    fn unsupported_values(&self) -> Vec<ModelInputModality> {
        match self {
            Self::Supported(_) => Vec::new(),
            Self::Patch(values) => values.unsupported.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeatureCapabilityPatchBody {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported: Vec<ModelCapabilityFeature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<ModelCapabilityFeature>,
}

impl FeatureCapabilityPatchBody {
    pub fn is_empty(&self) -> bool {
        self.supported.is_empty() && self.unsupported.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FeatureCapabilityPatch {
    Supported(Vec<ModelCapabilityFeature>),
    Patch(FeatureCapabilityPatchBody),
}

impl FeatureCapabilityPatch {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Supported(values) => values.is_empty(),
            Self::Patch(values) => values.is_empty(),
        }
    }

    fn supported_values(&self) -> Vec<ModelCapabilityFeature> {
        match self {
            Self::Supported(values) => values.clone(),
            Self::Patch(values) => values.supported.clone(),
        }
    }

    fn unsupported_values(&self) -> Vec<ModelCapabilityFeature> {
        match self {
            Self::Supported(_) => Vec::new(),
            Self::Patch(values) => values.unsupported.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapabilityFeature {
    ToolCalling,
    Streaming,
    Reasoning,
    StructuredOutput,
    #[serde(rename = "temperature")]
    Temperature,
}

impl ModelCapabilityFeature {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ToolCalling => "tool_calling",
            Self::Streaming => "streaming",
            Self::Reasoning => "reasoning",
            Self::StructuredOutput => "structured_output",
            Self::Temperature => "temperature",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCapabilityPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputCapabilityPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<FeatureCapabilityPatch>,
    #[serde(default, skip_serializing)]
    pub text_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub image_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub document_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub audio_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub video_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub file_input: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub tool_calling: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub streaming: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub reasoning: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub structured_output: Option<CapabilitySupport>,
    #[serde(default, skip_serializing)]
    pub temperature_supported: Option<CapabilitySupport>,
}

impl ModelCapabilityPatch {
    pub fn is_empty(&self) -> bool {
        self.input
            .as_ref()
            .is_none_or(InputCapabilityPatch::is_empty)
            && self
                .features
                .as_ref()
                .is_none_or(FeatureCapabilityPatch::is_empty)
            && self.text_input.is_none()
            && self.image_input.is_none()
            && self.document_input.is_none()
            && self.audio_input.is_none()
            && self.video_input.is_none()
            && self.file_input.is_none()
            && self.tool_calling.is_none()
            && self.streaming.is_none()
            && self.reasoning.is_none()
            && self.structured_output.is_none()
            && self.temperature_supported.is_none()
    }

    pub fn input_support(&self, modality: ModelInputModality) -> Option<CapabilitySupport> {
        if let Some(selection) = &self.input {
            if selection.supported_values().contains(&modality) {
                return Some(CapabilitySupport::Supported);
            }
            if selection.unsupported_values().contains(&modality) {
                return Some(CapabilitySupport::Unsupported);
            }
        }
        match modality {
            ModelInputModality::Text => self.text_input,
            ModelInputModality::Image => self.image_input,
            ModelInputModality::Document => self.document_input,
            ModelInputModality::Audio => self.audio_input,
            ModelInputModality::Video => self.video_input,
            ModelInputModality::File => self.file_input,
        }
    }

    pub fn feature_support(&self, feature: ModelCapabilityFeature) -> Option<CapabilitySupport> {
        if let Some(selection) = &self.features {
            if selection.supported_values().contains(&feature) {
                return Some(CapabilitySupport::Supported);
            }
            if selection.unsupported_values().contains(&feature) {
                return Some(CapabilitySupport::Unsupported);
            }
        }
        match feature {
            ModelCapabilityFeature::ToolCalling => self.tool_calling,
            ModelCapabilityFeature::Streaming => self.streaming,
            ModelCapabilityFeature::Reasoning => self.reasoning,
            ModelCapabilityFeature::StructuredOutput => self.structured_output,
            ModelCapabilityFeature::Temperature => self.temperature_supported,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_modality_patch(self.input.as_ref())?;
        validate_feature_patch(self.features.as_ref())?;
        Ok(())
    }

    pub fn normalize_compact_patch(&mut self) {
        for modality in [
            ModelInputModality::Text,
            ModelInputModality::Image,
            ModelInputModality::Document,
            ModelInputModality::Audio,
            ModelInputModality::Video,
            ModelInputModality::File,
        ] {
            if let Some(value) = self.input_support(modality) {
                match modality {
                    ModelInputModality::Text => self.text_input = Some(value),
                    ModelInputModality::Image => self.image_input = Some(value),
                    ModelInputModality::Document => self.document_input = Some(value),
                    ModelInputModality::Audio => self.audio_input = Some(value),
                    ModelInputModality::Video => self.video_input = Some(value),
                    ModelInputModality::File => self.file_input = Some(value),
                }
            }
        }

        for feature in [
            ModelCapabilityFeature::ToolCalling,
            ModelCapabilityFeature::Streaming,
            ModelCapabilityFeature::Reasoning,
            ModelCapabilityFeature::StructuredOutput,
            ModelCapabilityFeature::Temperature,
        ] {
            if let Some(value) = self.feature_support(feature) {
                match feature {
                    ModelCapabilityFeature::ToolCalling => self.tool_calling = Some(value),
                    ModelCapabilityFeature::Streaming => self.streaming = Some(value),
                    ModelCapabilityFeature::Reasoning => self.reasoning = Some(value),
                    ModelCapabilityFeature::StructuredOutput => {
                        self.structured_output = Some(value);
                    }
                    ModelCapabilityFeature::Temperature => {
                        self.temperature_supported = Some(value);
                    }
                }
            }
        }
    }

    pub fn apply_to(&self, mut capabilities: ModelCapabilities) -> ModelCapabilities {
        apply_legacy_capability_patch(self, &mut capabilities);
        apply_modality_patch(self.input.as_ref(), &mut capabilities);
        apply_feature_patch(self.features.as_ref(), &mut capabilities);
        capabilities
    }

    pub fn normalized_resolved_patch(&self) -> Self {
        let mut supported_inputs = Vec::new();
        let mut unsupported_inputs = Vec::new();
        for modality in [
            ModelInputModality::Text,
            ModelInputModality::Image,
            ModelInputModality::Document,
            ModelInputModality::Audio,
            ModelInputModality::Video,
            ModelInputModality::File,
        ] {
            match self.input_support(modality) {
                Some(CapabilitySupport::Supported) => supported_inputs.push(modality),
                Some(CapabilitySupport::Unsupported) => unsupported_inputs.push(modality),
                Some(CapabilitySupport::Unknown) | None => {}
            }
        }

        let mut supported_features = Vec::new();
        let mut unsupported_features = Vec::new();
        for feature in [
            ModelCapabilityFeature::ToolCalling,
            ModelCapabilityFeature::Streaming,
            ModelCapabilityFeature::Reasoning,
            ModelCapabilityFeature::StructuredOutput,
            ModelCapabilityFeature::Temperature,
        ] {
            match self.feature_support(feature) {
                Some(CapabilitySupport::Supported) => supported_features.push(feature),
                Some(CapabilitySupport::Unsupported) => unsupported_features.push(feature),
                Some(CapabilitySupport::Unknown) | None => {}
            }
        }

        Self {
            input: capability_input_patch(supported_inputs, unsupported_inputs),
            features: capability_feature_patch(supported_features, unsupported_features),
            ..Self::default()
        }
    }
}

fn capability_input_patch(
    supported: Vec<ModelInputModality>,
    unsupported: Vec<ModelInputModality>,
) -> Option<InputCapabilityPatch> {
    if supported.is_empty() && unsupported.is_empty() {
        return None;
    }
    if unsupported.is_empty() {
        Some(InputCapabilityPatch::Supported(supported))
    } else {
        Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
            supported,
            unsupported,
        }))
    }
}

fn capability_feature_patch(
    supported: Vec<ModelCapabilityFeature>,
    unsupported: Vec<ModelCapabilityFeature>,
) -> Option<FeatureCapabilityPatch> {
    if supported.is_empty() && unsupported.is_empty() {
        return None;
    }
    if unsupported.is_empty() {
        Some(FeatureCapabilityPatch::Supported(supported))
    } else {
        Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
            supported,
            unsupported,
        }))
    }
}

fn apply_legacy_capability_patch(
    patch: &ModelCapabilityPatch,
    capabilities: &mut ModelCapabilities,
) {
    if let Some(value) = patch.text_input {
        capabilities.text_input = value;
    }
    if let Some(value) = patch.image_input {
        capabilities.image_input = value;
    }
    if let Some(value) = patch.document_input {
        capabilities.document_input = value;
    }
    if let Some(value) = patch.audio_input {
        capabilities.audio_input = value;
    }
    if let Some(value) = patch.video_input {
        capabilities.video_input = value;
    }
    if let Some(value) = patch.file_input {
        capabilities.file_input = value;
    }
    if let Some(value) = patch.tool_calling {
        capabilities.tool_calling = value;
    }
    if let Some(value) = patch.streaming {
        capabilities.streaming = value;
    }
    if let Some(value) = patch.reasoning {
        capabilities.reasoning = value;
    }
    if let Some(value) = patch.structured_output {
        capabilities.structured_output = value;
    }
    if let Some(value) = patch.temperature_supported {
        capabilities.temperature_supported = value;
    }
}

fn apply_modality_patch(
    patch: Option<&InputCapabilityPatch>,
    capabilities: &mut ModelCapabilities,
) {
    let Some(patch) = patch else {
        return;
    };
    for modality in patch.supported_values() {
        set_input_capability(capabilities, modality, CapabilitySupport::Supported);
    }
    for modality in patch.unsupported_values() {
        set_input_capability(capabilities, modality, CapabilitySupport::Unsupported);
    }
}

fn set_input_capability(
    capabilities: &mut ModelCapabilities,
    modality: ModelInputModality,
    support: CapabilitySupport,
) {
    match modality {
        ModelInputModality::Text => capabilities.text_input = support,
        ModelInputModality::Image => capabilities.image_input = support,
        ModelInputModality::Document => capabilities.document_input = support,
        ModelInputModality::Audio => capabilities.audio_input = support,
        ModelInputModality::Video => capabilities.video_input = support,
        ModelInputModality::File => capabilities.file_input = support,
    }
}

fn apply_feature_patch(
    patch: Option<&FeatureCapabilityPatch>,
    capabilities: &mut ModelCapabilities,
) {
    let Some(patch) = patch else {
        return;
    };
    for feature in patch.supported_values() {
        set_feature_capability(capabilities, feature, CapabilitySupport::Supported);
    }
    for feature in patch.unsupported_values() {
        set_feature_capability(capabilities, feature, CapabilitySupport::Unsupported);
    }
}

fn set_feature_capability(
    capabilities: &mut ModelCapabilities,
    feature: ModelCapabilityFeature,
    support: CapabilitySupport,
) {
    match feature {
        ModelCapabilityFeature::ToolCalling => capabilities.tool_calling = support,
        ModelCapabilityFeature::Streaming => capabilities.streaming = support,
        ModelCapabilityFeature::Reasoning => capabilities.reasoning = support,
        ModelCapabilityFeature::StructuredOutput => capabilities.structured_output = support,
        ModelCapabilityFeature::Temperature => capabilities.temperature_supported = support,
    }
}

fn validate_modality_patch(patch: Option<&InputCapabilityPatch>) -> Result<(), String> {
    let Some(patch) = patch else {
        return Ok(());
    };
    validate_named_patch(
        "input",
        patch
            .supported_values()
            .into_iter()
            .map(ModelInputModality::as_str)
            .collect(),
        patch
            .unsupported_values()
            .into_iter()
            .map(ModelInputModality::as_str)
            .collect(),
    )
}

fn validate_feature_patch(patch: Option<&FeatureCapabilityPatch>) -> Result<(), String> {
    let Some(patch) = patch else {
        return Ok(());
    };
    validate_named_patch(
        "features",
        patch
            .supported_values()
            .into_iter()
            .map(ModelCapabilityFeature::as_str)
            .collect(),
        patch
            .unsupported_values()
            .into_iter()
            .map(ModelCapabilityFeature::as_str)
            .collect(),
    )
}

fn validate_named_patch(
    group: &str,
    supported: Vec<&'static str>,
    unsupported: Vec<&'static str>,
) -> Result<(), String> {
    let mut supported_set = std::collections::BTreeSet::new();
    for value in supported {
        if !supported_set.insert(value) {
            return Err(format!(
                "{group} capability `{value}` listed more than once"
            ));
        }
    }

    let mut unsupported_set = std::collections::BTreeSet::new();
    for value in unsupported {
        if !unsupported_set.insert(value) {
            return Err(format!(
                "{group} capability `{value}` listed more than once"
            ));
        }
    }

    if let Some(value) = supported_set.intersection(&unsupported_set).next() {
        return Err(format!(
            "{group} capability `{value}` cannot be both supported and unsupported"
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConfiguredModelThinkingMode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequest>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

impl ConfiguredModelThinkingMode {
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.description.is_none()
            && self.thinking.is_none()
            && self.request_override.is_empty()
            && self.adapter_overrides.is_empty()
            && !self.disabled
    }

    pub(crate) fn apply_to_mode(
        &self,
        base: Option<&ModelThinkingMode>,
    ) -> Option<ModelThinkingMode> {
        if self.disabled {
            return None;
        }
        let mut mode = base.cloned().unwrap_or_default();
        if let Some(display_name) = self.display_name.clone() {
            mode.display_name = Some(display_name);
        }
        if let Some(description) = self.description.clone() {
            mode.description = Some(description);
        }
        if let Some(thinking) = self.thinking.clone() {
            mode.thinking = Some(thinking);
        }
        mode.request_override = mode.request_override.merged_with(&self.request_override);
        for (adapter_id, override_patch) in &self.adapter_overrides {
            let merged = mode
                .adapter_overrides
                .get(adapter_id)
                .cloned()
                .unwrap_or_default()
                .merged_with(override_patch);
            mode.adapter_overrides.insert(adapter_id.clone(), merged);
        }
        Some(mode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConfiguredModelSpeedMode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

impl ConfiguredModelSpeedMode {
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.description.is_none()
            && self.request_override.is_empty()
            && self.adapter_overrides.is_empty()
            && !self.disabled
    }

    pub(crate) fn apply_to_mode(&self, base: Option<&ModelSpeedMode>) -> Option<ModelSpeedMode> {
        if self.disabled {
            return None;
        }
        let mut mode = base.cloned().unwrap_or_default();
        if let Some(display_name) = self.display_name.clone() {
            mode.display_name = Some(display_name);
        }
        if let Some(description) = self.description.clone() {
            mode.description = Some(description);
        }
        mode.request_override = mode.request_override.merged_with(&self.request_override);
        for (adapter_id, override_patch) in &self.adapter_overrides {
            let merged = mode
                .adapter_overrides
                .get(adapter_id)
                .cloned()
                .unwrap_or_default()
                .merged_with(override_patch);
            mode.adapter_overrides.insert(adapter_id.clone(), merged);
        }
        Some(mode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConfiguredModelDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub thinking_modes: BTreeMap<String, ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl ConfiguredModelDefinition {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.display_name.is_none()
            && self.description.is_none()
            && self.knowledge_cutoff.is_none()
            && self.release_date.is_none()
            && self.last_updated.is_none()
            && self.open_weights.is_none()
            && self.default_thinking_mode.is_none()
            && self.supports_parallel_tool_calls.is_none()
            && self.supports_verbosity.is_none()
            && self.default_verbosity.is_none()
            && self.default_temperature.is_none()
            && self.default_top_p.is_none()
            && self.default_top_k.is_none()
            && self.assistant_reasoning_interleaved.is_none()
            && self.assistant_reasoning_field.is_none()
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
            && self.thinking_modes.is_empty()
            && self.speed_modes.is_empty()
            && self.capabilities.is_empty()
    }

    pub fn metadata(&self) -> ModelMetadata {
        let mut metadata = ModelMetadata::default();
        if let Some(lifecycle) = self.lifecycle {
            metadata = metadata.with_lifecycle(lifecycle);
        }
        if let Some(context_window_tokens) = self.context_window_tokens {
            metadata = metadata.with_context_window_tokens(context_window_tokens);
        }
        if let Some(max_input_tokens) = self.max_input_tokens {
            metadata = metadata.with_max_input_tokens(max_input_tokens);
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            metadata = metadata.with_max_output_tokens(max_output_tokens);
        }
        if let Some(description) = self.description.clone() {
            metadata = metadata.with_description(description);
        }
        if let Some(knowledge_cutoff) = self.knowledge_cutoff.clone() {
            metadata = metadata.with_knowledge_cutoff(knowledge_cutoff);
        }
        if let Some(release_date) = self.release_date.clone() {
            metadata = metadata.with_release_date(release_date);
        }
        if let Some(last_updated) = self.last_updated.clone() {
            metadata = metadata.with_last_updated(last_updated);
        }
        if let Some(open_weights) = self.open_weights {
            metadata = metadata.with_open_weights(open_weights);
        }
        if let Some(default_thinking_mode) = self.default_thinking_mode.clone() {
            metadata = metadata.with_default_thinking_mode(default_thinking_mode);
        }
        if let Some(supports_parallel_tool_calls) = self.supports_parallel_tool_calls {
            metadata = metadata.with_supports_parallel_tool_calls(supports_parallel_tool_calls);
        }
        if let Some(supports_verbosity) = self.supports_verbosity {
            metadata = metadata.with_supports_verbosity(supports_verbosity);
        }
        if let Some(default_verbosity) = self.default_verbosity.clone() {
            metadata = metadata.with_default_verbosity(default_verbosity);
        }
        if let Some(default_temperature) = self.default_temperature.clone() {
            metadata = metadata.with_default_temperature(default_temperature);
        }
        if let Some(default_top_p) = self.default_top_p.clone() {
            metadata = metadata.with_default_top_p(default_top_p);
        }
        if let Some(default_top_k) = self.default_top_k {
            metadata = metadata.with_default_top_k(default_top_k);
        }
        if let Some(assistant_reasoning_interleaved) = self.assistant_reasoning_interleaved {
            metadata =
                metadata.with_assistant_reasoning_interleaved(assistant_reasoning_interleaved);
        }
        if let Some(assistant_reasoning_field) = self.assistant_reasoning_field.clone() {
            metadata = metadata.with_assistant_reasoning_field(assistant_reasoning_field);
        }
        if !self.output_modalities.is_empty() {
            metadata = metadata.with_output_modalities(self.output_modalities.clone());
        }
        if let Some(pricing) = self.pricing.clone() {
            metadata = metadata.with_pricing(pricing);
        }
        metadata
    }

    pub(crate) fn apply_to_model(
        &self,
        mut model: Model,
        capability_fallback: &ModelCapabilities,
        metadata_fallback: &ModelMetadata,
    ) -> Model {
        if let Some(display_name) = self.display_name.clone() {
            model.display_name = Some(display_name);
        }
        let base_capabilities = model
            .capabilities
            .clone()
            .with_fallbacks_from(capability_fallback);
        model.capabilities = self.capabilities.apply_to(base_capabilities);
        let base_metadata = model
            .metadata
            .clone()
            .with_fallbacks_from(metadata_fallback);
        model.metadata = self.metadata().with_fallbacks_from(&base_metadata);
        model.thinking_modes =
            apply_configured_thinking_modes(model.thinking_modes, self.thinking_modes.iter());
        model.speed_modes =
            apply_configured_speed_modes(model.speed_modes, self.speed_modes.iter());
        model
    }
}

fn apply_configured_thinking_modes<'a>(
    mut modes: BTreeMap<String, ModelThinkingMode>,
    configured_modes: impl Iterator<Item = (&'a String, &'a ConfiguredModelThinkingMode)>,
) -> BTreeMap<String, ModelThinkingMode> {
    for (name, configured) in configured_modes {
        match configured.apply_to_mode(modes.get(name)) {
            Some(mode) => {
                modes.insert(name.clone(), mode);
            }
            None => {
                modes.remove(name);
            }
        }
    }
    modes
}

fn apply_configured_speed_modes<'a>(
    mut modes: BTreeMap<String, ModelSpeedMode>,
    configured_modes: impl Iterator<Item = (&'a String, &'a ConfiguredModelSpeedMode)>,
) -> BTreeMap<String, ModelSpeedMode> {
    for (name, configured) in configured_modes {
        match configured.apply_to_mode(modes.get(name)) {
            Some(mode) => {
                modes.insert(name.clone(), mode);
            }
            None => {
                modes.remove(name);
            }
        }
    }
    modes
}

#[derive(Clone)]
pub struct ConfiguredModelsProvider {
    target: Arc<dyn ModelRuntime>,
    models: Arc<BTreeMap<String, ConfiguredModelDefinition>>,
}

impl ConfiguredModelsProvider {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        target: Arc<dyn ModelRuntime>,
        models: BTreeMap<String, ConfiguredModelDefinition>,
    ) -> Arc<dyn ModelRuntime> {
        if models.is_empty() {
            target
        } else {
            Arc::new(Self {
                target,
                models: Arc::new(models),
            })
        }
    }

    fn configured_model(&self, model: &ModelId) -> Option<&ConfiguredModelDefinition> {
        self.models.get(model.as_str())
    }

    fn configured_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        let base = self.target.model_capabilities(model);
        self.configured_model(model)
            .map(|configured| configured.capabilities.apply_to(base.clone()))
            .unwrap_or(base)
    }

    fn configured_metadata(&self, model: &ModelId) -> ModelMetadata {
        let base = self.target.model_metadata(model);
        self.configured_model(model)
            .map(|configured| configured.metadata().with_fallbacks_from(&base))
            .unwrap_or(base)
    }

    fn configured_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        let mut base = self
            .target
            .model_thinking_modes_for_adapter(adapter_id, model);
        if let Some(family) = self.target.capability_family() {
            let metadata = self.model_metadata_for_adapter(adapter_id, model);
            for (name, mode) in crate::provider::default_model_mode_registry()
                .thinking_modes_for_family(family, adapter_id, model.as_str(), &metadata)
            {
                base.entry(name).or_insert(mode);
            }
        }
        apply_configured_thinking_modes(
            base,
            self.configured_model(model)
                .map(|configured| configured.thinking_modes.iter())
                .into_iter()
                .flatten(),
        )
    }

    fn configured_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        apply_configured_speed_modes(
            self.target.model_speed_modes(model),
            self.configured_model(model)
                .map(|configured| configured.speed_modes.iter())
                .into_iter()
                .flatten(),
        )
    }
}

#[async_trait]
impl ForwardingModelRuntime for ConfiguredModelsProvider {
    fn target(&self) -> &dyn ModelRuntime {
        self.target.as_ref()
    }

    fn prepare_request(&self, adapter_id: Option<&AdapterId>, request: &mut CompletionRequest) {
        ModelRuntime::backfill_assistant_reasoning_field(self, adapter_id, request);
    }
}

#[async_trait]
impl ModelRuntime for ConfiguredModelsProvider {
    fn id(&self) -> &str {
        self.target.id()
    }

    fn default_model(&self) -> &ModelId {
        self.target.default_model()
    }

    fn default_adapter(&self) -> Option<&AdapterId> {
        self.target.default_adapter()
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.configured_capabilities(model)
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let _ = adapter_id;
        self.configured_capabilities(model)
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        self.configured_metadata(model)
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        let _ = adapter_id;
        self.configured_metadata(model)
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        self.configured_thinking_modes_for_adapter(None, model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        self.configured_thinking_modes_for_adapter(adapter_id, model)
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        self.configured_speed_modes(model)
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        let _ = adapter_id;
        self.configured_speed_modes(model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.target.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.target.supports_prompt_continuation(model)
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.target
            .supports_prompt_continuation_for_adapter(adapter_id, model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.target.prompt_cache_shape(model)
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<PromptCacheShape> {
        self.target
            .prompt_cache_shape_for_adapter(adapter_id, model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        let mut listed_ids = std::collections::BTreeSet::new();

        for model in &mut models {
            listed_ids.insert(model.id.to_string());
            if let Some(configured) = self.models.get(model.id.as_str()) {
                let capability_fallback = self.target.model_capabilities(&model.id);
                let metadata_fallback = self.target.model_metadata(&model.id);
                *model = configured.apply_to_model(
                    model.clone(),
                    &capability_fallback,
                    &metadata_fallback,
                );
            } else {
                model.thinking_modes = self
                    .configured_thinking_modes_for_adapter(model.adapter_id.as_ref(), &model.id);
                model.speed_modes = self.configured_speed_modes(&model.id);
            }
        }

        for (model_id, configured) in self.models.iter() {
            if listed_ids.contains(model_id.as_str()) {
                continue;
            }

            let model_id_obj = ModelId::new(model_id.clone());
            let capability_fallback = self.target.model_capabilities(&model_id_obj);
            let metadata_fallback = self.target.model_metadata(&model_id_obj);
            let model = configured.apply_to_model(
                Model::new(self.target.id(), model_id.clone()),
                &capability_fallback,
                &metadata_fallback,
            );
            models.push(model);
        }

        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.forward_complete(None, request).await
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        self.forward_complete(adapter_id, request).await
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        self.forward_compact_conversation(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        self.forward_compact_conversation(adapter_id, request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.forward_complete_stream(None, request).await
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.forward_complete_stream(adapter_id, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        message::{Message, PartContent, ReasoningPart},
        provider::{CompletionFinishReason, CompletionResponse},
        role::Role,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct StaticProvider {
        default_model: ModelId,
        listed_models: Vec<Model>,
        fallback_capabilities: ModelCapabilities,
        fallback_metadata: ModelMetadata,
        captured_requests: Option<Arc<Mutex<Vec<CompletionRequest>>>>,
        capability_family: Option<crate::provider::CapabilityFamily>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for StaticProvider {
        fn id(&self) -> &str {
            "test-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
            self.capability_family
        }

        fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
            self.fallback_capabilities.clone()
        }

        fn model_metadata(&self, _model: &ModelId) -> ModelMetadata {
            self.fallback_metadata.clone()
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(self.listed_models.clone())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            if let Some(captured_requests) = &self.captured_requests {
                captured_requests
                    .lock()
                    .expect("captured requests lock should not be poisoned")
                    .push(request.clone());
            }
            Ok(CompletionResponse {
                provider_id: crate::model::ProviderId::new("test-provider"),
                model: request.model,
                text: "ok".to_owned(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }
    }

    #[test]
    fn patch_applies_only_selected_fields() {
        let base = ModelCapabilities::default()
            .with_image_input(CapabilitySupport::Supported)
            .with_streaming(CapabilitySupport::Supported);
        let patch = ModelCapabilityPatch {
            input: Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: Vec::new(),
                unsupported: vec![ModelInputModality::Image],
            })),
            features: Some(FeatureCapabilityPatch::Supported(vec![
                ModelCapabilityFeature::ToolCalling,
            ])),
            ..ModelCapabilityPatch::default()
        };

        let updated = patch.apply_to(base);
        assert_eq!(updated.image_input, CapabilitySupport::Unsupported);
        assert_eq!(updated.tool_calling, CapabilitySupport::Supported);
        assert_eq!(updated.streaming, CapabilitySupport::Supported);
    }

    #[test]
    fn configured_model_definition_reports_empty_state() {
        assert!(ConfiguredModelDefinition::default().is_empty());
        assert!(
            !ConfiguredModelDefinition {
                lifecycle: Some(ModelLifecycle::Preview),
                ..ConfiguredModelDefinition::default()
            }
            .is_empty()
        );
    }

    #[tokio::test]
    async fn configured_provider_lists_models_defined_only_in_config() {
        let target = std::sync::Arc::new(StaticProvider {
            default_model: ModelId::new("base-model"),
            listed_models: vec![
                Model::new("test-provider", "base-model")
                    .with_display_name("Base Model")
                    .with_capabilities(
                        ModelCapabilities::default().with_streaming(CapabilitySupport::Supported),
                    ),
            ],
            fallback_capabilities: ModelCapabilities::default()
                .with_streaming(CapabilitySupport::Supported),
            fallback_metadata: ModelMetadata::default().with_description("fallback metadata"),
            captured_requests: None,
            capability_family: None,
        });

        let provider = ConfiguredModelsProvider::new(
            target,
            BTreeMap::from([(
                "configured-only".to_owned(),
                ConfiguredModelDefinition {
                    lifecycle: Some(ModelLifecycle::Preview),
                    capabilities: ModelCapabilityPatch {
                        input: Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                            supported: Vec::new(),
                            unsupported: vec![ModelInputModality::Image],
                        })),
                        ..ModelCapabilityPatch::default()
                    },
                    ..ConfiguredModelDefinition::default()
                },
            )]),
        );

        let models = provider
            .list_models()
            .await
            .expect("configured provider should list models");

        assert_eq!(models.len(), 2);
        let configured = models
            .iter()
            .find(|model| model.id.as_str() == "configured-only")
            .expect("configured-only model should be present");
        assert_eq!(configured.display_name, None);
        assert_eq!(
            configured.capabilities.image_input,
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            configured.capabilities.streaming,
            CapabilitySupport::Supported
        );
        assert_eq!(
            configured.metadata.description.as_deref(),
            Some("fallback metadata")
        );
        assert_eq!(configured.metadata.lifecycle, Some(ModelLifecycle::Preview));
    }

    #[tokio::test]
    async fn configured_provider_applies_model_modes() {
        let target = std::sync::Arc::new(StaticProvider {
            default_model: ModelId::new("base-model"),
            listed_models: vec![
                Model::new("test-provider", "base-model"),
                Model::new("test-provider", "other-model"),
            ],
            fallback_capabilities: ModelCapabilities::default(),
            fallback_metadata: ModelMetadata::default(),
            captured_requests: None,
            capability_family: None,
        });

        let provider = ConfiguredModelsProvider::new(
            target,
            BTreeMap::from([(
                "base-model".to_owned(),
                ConfiguredModelDefinition {
                    thinking_modes: BTreeMap::from([(
                        "deep".to_owned(),
                        ConfiguredModelThinkingMode {
                            display_name: None,
                            description: Some("More reasoning".to_owned()),
                            thinking: Some(ThinkingRequest::Budget {
                                budget_tokens: 30_000,
                            }),
                            request_override: Default::default(),
                            adapter_overrides: BTreeMap::new(),
                            disabled: false,
                        },
                    )]),
                    speed_modes: BTreeMap::from([(
                        "fast".to_owned(),
                        ConfiguredModelSpeedMode {
                            display_name: Some("Fast".to_owned()),
                            description: Some("Priority route".to_owned()),
                            request_override: Default::default(),
                            adapter_overrides: BTreeMap::new(),
                            disabled: false,
                        },
                    )]),
                    ..ConfiguredModelDefinition::default()
                },
            )]),
        );

        let models = provider
            .list_models()
            .await
            .expect("configured provider should list models");
        let model = models
            .iter()
            .find(|model| model.id.as_str() == "base-model")
            .expect("base model should be listed");

        assert_eq!(
            model
                .thinking_modes
                .get("deep")
                .and_then(|mode| mode.thinking.clone()),
            Some(ThinkingRequest::Budget {
                budget_tokens: 30_000
            })
        );
        assert_eq!(
            model
                .speed_modes
                .get("fast")
                .and_then(|mode| mode.display_name.as_deref()),
            Some("Fast")
        );
        let other_model = models
            .iter()
            .find(|model| model.id.as_str() == "other-model")
            .expect("other model should be listed");
        assert!(!other_model.thinking_modes.contains_key("deep"));
        assert!(!other_model.speed_modes.contains_key("fast"));
    }

    #[tokio::test]
    async fn configured_provider_uses_config_metadata_to_enrich_release_gated_thinking_modes() {
        let target = std::sync::Arc::new(StaticProvider {
            default_model: ModelId::new("gpt-5"),
            listed_models: vec![Model::new("test-provider", "gpt-5")],
            fallback_capabilities: ModelCapabilities::default(),
            fallback_metadata: ModelMetadata::default(),
            captured_requests: None,
            capability_family: Some(crate::provider::CapabilityFamily::OpenAi),
        });

        let provider = ConfiguredModelsProvider::new(
            target,
            BTreeMap::from([(
                "gpt-5".to_owned(),
                ConfiguredModelDefinition {
                    release_date: Some("2025-12-04".to_owned()),
                    ..ConfiguredModelDefinition::default()
                },
            )]),
        );

        let modes = provider.model_thinking_modes(&ModelId::new("gpt-5"));
        assert!(modes.contains_key("no-thinking"));
        assert!(modes.contains_key("thinking-xhigh"));
        assert!(modes.contains_key("thinking-minimal"));
    }

    #[tokio::test]
    async fn configured_provider_backfills_assistant_reasoning_field_from_config_metadata() {
        let captured_requests = Arc::new(Mutex::new(Vec::new()));
        let target = Arc::new(StaticProvider {
            default_model: ModelId::new("deepseek-v4-pro"),
            listed_models: vec![Model::new("test-provider", "deepseek-v4-pro")],
            fallback_capabilities: ModelCapabilities::default(),
            fallback_metadata: ModelMetadata::default(),
            captured_requests: Some(Arc::clone(&captured_requests)),
            capability_family: None,
        });

        let provider = ConfiguredModelsProvider::new(
            target,
            BTreeMap::from([(
                "deepseek-v4-pro".to_owned(),
                ConfiguredModelDefinition {
                    assistant_reasoning_field: Some("reasoning_content".to_owned()),
                    ..ConfiguredModelDefinition::default()
                },
            )]),
        );

        provider
            .complete(CompletionRequest {
                model: ModelId::new("deepseek-v4-pro"),
                system: None,
                messages: vec![
                    Message::prompt_parts(
                        Role::Assistant,
                        vec![
                            PartContent::Reasoning(ReasoningPart {
                                summary: vec!["Prior chain".to_owned()],
                                raw_content: Vec::new(),
                                encrypted_content: None,
                            }),
                            PartContent::text("Prior answer"),
                        ],
                    ),
                    Message::prompt_text(Role::User, "continue"),
                ],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                verbosity: None,
                request_override: Default::default(),
                response_format: None,
            })
            .await
            .expect("configured provider completion should succeed");

        let captured = captured_requests
            .lock()
            .expect("captured requests lock should not be poisoned");
        let assistant = captured[0]
            .messages
            .iter()
            .find(|message| matches!(message.role, Role::Assistant))
            .expect("assistant message should be present");
        assert_eq!(
            assistant
                .metadata
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("assistant_reasoning_field"))
                .and_then(|value| value.as_str()),
            Some("reasoning_content")
        );
    }

    #[test]
    fn compact_patch_validates_overlapping_entries() {
        let patch = ModelCapabilityPatch {
            input: Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: vec![ModelInputModality::Image],
                unsupported: vec![ModelInputModality::Image],
            })),
            ..ModelCapabilityPatch::default()
        };

        let err = patch
            .validate()
            .expect_err("overlapping input patch should fail");
        assert!(err.contains("input capability `image` cannot be both supported and unsupported"));
    }

    #[test]
    fn compact_patch_normalizes_into_internal_fields() {
        let mut patch = ModelCapabilityPatch {
            input: Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: vec![ModelInputModality::Document],
                unsupported: vec![ModelInputModality::Image],
            })),
            features: Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                supported: vec![ModelCapabilityFeature::ToolCalling],
                unsupported: vec![ModelCapabilityFeature::Temperature],
            })),
            ..ModelCapabilityPatch::default()
        };

        patch.normalize_compact_patch();

        assert_eq!(patch.image_input, Some(CapabilitySupport::Unsupported));
        assert_eq!(patch.document_input, Some(CapabilitySupport::Supported));
        assert_eq!(patch.tool_calling, Some(CapabilitySupport::Supported));
        assert_eq!(
            patch.temperature_supported,
            Some(CapabilitySupport::Unsupported)
        );
    }

    #[test]
    fn normalized_resolved_patch_removes_compact_conflicts() {
        let patch = ModelCapabilityPatch {
            input: Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: vec![ModelInputModality::Audio],
                unsupported: vec![ModelInputModality::Audio, ModelInputModality::Image],
            })),
            features: Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                supported: vec![ModelCapabilityFeature::Reasoning],
                unsupported: vec![
                    ModelCapabilityFeature::Reasoning,
                    ModelCapabilityFeature::Streaming,
                ],
            })),
            ..ModelCapabilityPatch::default()
        };

        let normalized = patch.normalized_resolved_patch();

        normalized
            .validate()
            .expect("normalized patch should be conflict-free");
        assert_eq!(
            normalized.input_support(ModelInputModality::Audio),
            Some(CapabilitySupport::Supported)
        );
        assert_eq!(
            normalized.input_support(ModelInputModality::Image),
            Some(CapabilitySupport::Unsupported)
        );
        assert_eq!(
            normalized.feature_support(ModelCapabilityFeature::Reasoning),
            Some(CapabilitySupport::Supported)
        );
        assert_eq!(
            normalized.feature_support(ModelCapabilityFeature::Streaming),
            Some(CapabilitySupport::Unsupported)
        );
    }
}
