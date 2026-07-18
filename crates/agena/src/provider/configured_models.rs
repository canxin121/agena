use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};

use crate::config::{AgenaToolMode, ProviderNativeToolsConfig};
use crate::error::AppError;
use crate::model::{
    AdapterId, CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelPricing, ModelSpeedMode, ModelSpeedModeRequestOverride,
    ModelThinkingMode, non_empty_model_pricing, normalize_model_assistant_reasoning_field,
    normalize_model_default_temperature, normalize_model_default_top_k,
    normalize_model_default_top_p, normalize_model_output_modalities,
};

use super::core::{
    ForwardingModelRuntime, impl_model_runtime_adapter_agnostic_methods,
    impl_model_runtime_base_via_adapter_methods, impl_model_runtime_target_defaults,
    impl_model_runtime_target_methods,
};
use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelRuntime, PromptCacheShape,
    ReasoningEffort, StreamResumePolicy, ThinkingDisplay, ThinkingRequest,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConfiguredModeDefault {
    #[default]
    Inherit,
    Clear,
    Mode(String),
}

impl ConfiguredModeDefault {
    fn is_inherit(&self) -> bool {
        matches!(self, Self::Inherit)
    }
    pub fn mode(&self) -> Option<&str> {
        match self {
            Self::Mode(value) => Some(value),
            _ => None,
        }
    }
}

fn deserialize_mode_default<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ConfiguredModeDefault, D::Error> {
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(value) => ConfiguredModeDefault::Mode(value),
        None => ConfiguredModeDefault::Clear,
    })
}

fn serialize_mode_default<S: serde::Serializer>(
    value: &ConfiguredModeDefault,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        ConfiguredModeDefault::Inherit | ConfiguredModeDefault::Clear => {
            serializer.serialize_none()
        }
        ConfiguredModeDefault::Mode(value) => serializer.serialize_str(value),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct ConfiguredModelModeMap<T> {
    #[serde(
        default,
        deserialize_with = "deserialize_mode_default",
        serialize_with = "serialize_mode_default",
        skip_serializing_if = "ConfiguredModeDefault::is_inherit"
    )]
    pub default: ConfiguredModeDefault,
    #[serde(flatten)]
    pub modes: BTreeMap<String, T>,
}

impl<T> ConfiguredModelModeMap<T> {
    pub fn is_empty(&self) -> bool {
        self.default.is_inherit() && self.modes.is_empty()
    }
}
impl<T> Deref for ConfiguredModelModeMap<T> {
    type Target = BTreeMap<String, T>;
    fn deref(&self) -> &Self::Target {
        &self.modes
    }
}
impl<T> DerefMut for ConfiguredModelModeMap<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.modes
    }
}
impl<T> From<BTreeMap<String, T>> for ConfiguredModelModeMap<T> {
    fn from(modes: BTreeMap<String, T>) -> Self {
        Self {
            default: ConfiguredModeDefault::Inherit,
            modes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfiguredThinkingStrategy {
    Disabled,
    Effort,
    Budget,
    Adaptive,
    RequestOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct CapabilitySelectionPatchBody<T> {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported: Vec<T>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<T>,
}

impl<T> CapabilitySelectionPatchBody<T> {
    pub fn is_empty(&self) -> bool {
        self.supported.is_empty() && self.unsupported.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub enum CapabilitySelectionPatch<T> {
    Supported(Vec<T>),
    Patch(CapabilitySelectionPatchBody<T>),
}

impl<T> CapabilitySelectionPatch<T> {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Supported(values) => values.is_empty(),
            Self::Patch(values) => values.is_empty(),
        }
    }

    pub fn supported(&self) -> &[T] {
        match self {
            Self::Supported(values) => values.as_slice(),
            Self::Patch(values) => values.supported.as_slice(),
        }
    }

    pub fn unsupported(&self) -> &[T] {
        match self {
            Self::Supported(_) => &[],
            Self::Patch(values) => values.unsupported.as_slice(),
        }
    }

    pub fn from_supported_unsupported(supported: Vec<T>, unsupported: Vec<T>) -> Self {
        if unsupported.is_empty() {
            Self::Supported(supported)
        } else {
            Self::Patch(CapabilitySelectionPatchBody {
                supported,
                unsupported,
            })
        }
    }

    pub fn optional_from_supported_unsupported(
        supported: Vec<T>,
        unsupported: Vec<T>,
    ) -> Option<Self> {
        (!supported.is_empty() || !unsupported.is_empty())
            .then(|| Self::from_supported_unsupported(supported, unsupported))
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

impl AsRef<str> for ModelCapabilityFeature {
    fn as_ref(&self) -> &str {
        match self {
            Self::ToolCalling => "tool_calling",
            Self::Streaming => "streaming",
            Self::Reasoning => "reasoning",
            Self::StructuredOutput => "structured_output",
            Self::Temperature => "temperature",
        }
    }
}

impl std::fmt::Display for ModelCapabilityFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCapabilityPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<CapabilitySelectionPatch<ModelInputModality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<CapabilitySelectionPatch<ModelCapabilityFeature>>,
}

impl ModelCapabilityPatch {
    pub fn is_empty(&self) -> bool {
        self.input.as_ref().is_none_or(|patch| patch.is_empty())
            && self.features.as_ref().is_none_or(|patch| patch.is_empty())
    }

    pub fn input_support(&self, modality: ModelInputModality) -> Option<CapabilitySupport> {
        if let Some(selection) = &self.input {
            if selection.supported().contains(&modality) {
                return Some(CapabilitySupport::Supported);
            }
            if selection.unsupported().contains(&modality) {
                return Some(CapabilitySupport::Unsupported);
            }
        }
        None
    }

    pub fn feature_support(&self, feature: ModelCapabilityFeature) -> Option<CapabilitySupport> {
        if let Some(selection) = &self.features {
            if selection.supported().contains(&feature) {
                return Some(CapabilitySupport::Supported);
            }
            if selection.unsupported().contains(&feature) {
                return Some(CapabilitySupport::Unsupported);
            }
        }
        None
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_capability_selection_patch(
            "input",
            self.input.as_ref(),
            |modality| match modality {
                ModelInputModality::Text => "text",
                ModelInputModality::Image => "image",
                ModelInputModality::Document => "document",
                ModelInputModality::Audio => "audio",
                ModelInputModality::Video => "video",
                ModelInputModality::File => "file",
            },
        )?;
        validate_capability_selection_patch("features", self.features.as_ref(), |feature| {
            match feature {
                ModelCapabilityFeature::ToolCalling => "tool_calling",
                ModelCapabilityFeature::Streaming => "streaming",
                ModelCapabilityFeature::Reasoning => "reasoning",
                ModelCapabilityFeature::StructuredOutput => "structured_output",
                ModelCapabilityFeature::Temperature => "temperature",
            }
        })?;
        Ok(())
    }

    pub fn normalize_compact_patch(&mut self) {
        *self = self.normalized_resolved_patch();
    }

    pub fn apply_to(&self, mut capabilities: ModelCapabilities) -> ModelCapabilities {
        apply_capability_selection_patch(self.input.as_ref(), |modality, support| {
            set_input_capability(&mut capabilities, modality, support);
        });
        apply_capability_selection_patch(self.features.as_ref(), |feature, support| {
            set_feature_capability(&mut capabilities, feature, support);
        });
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
            input: capability_selection_patch(supported_inputs, unsupported_inputs),
            features: capability_selection_patch(supported_features, unsupported_features),
        }
    }
}

fn capability_selection_patch<T>(
    supported: Vec<T>,
    unsupported: Vec<T>,
) -> Option<CapabilitySelectionPatch<T>> {
    CapabilitySelectionPatch::optional_from_supported_unsupported(supported, unsupported)
}

fn apply_capability_selection_patch<T: Clone>(
    patch: Option<&CapabilitySelectionPatch<T>>,
    mut apply: impl FnMut(T, CapabilitySupport),
) {
    let Some(patch) = patch else {
        return;
    };
    for value in patch.supported() {
        apply(value.clone(), CapabilitySupport::Supported);
    }
    for value in patch.unsupported() {
        apply(value.clone(), CapabilitySupport::Unsupported);
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

fn validate_capability_selection_patch<T>(
    group: &str,
    patch: Option<&CapabilitySelectionPatch<T>>,
    name: impl Fn(&T) -> &'static str,
) -> Result<(), String> {
    let Some(patch) = patch else {
        return Ok(());
    };
    validate_named_patch(
        group,
        patch.supported().iter().map(&name).collect(),
        patch.unsupported().iter().map(name).collect(),
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

fn merge_mode_adapter_overrides(
    target: &mut BTreeMap<String, ModelSpeedModeRequestOverride>,
    overrides: &BTreeMap<String, ModelSpeedModeRequestOverride>,
) {
    for (adapter_id, override_patch) in overrides {
        let merged = target
            .get(adapter_id)
            .cloned()
            .unwrap_or_default()
            .merged_with(override_patch);
        target.insert(adapter_id.clone(), merged);
    }
}

macro_rules! define_configured_model_mode {
    (
        $name:ident,
        $mode:ident,
        fields { $($extra_fields:tt)* },
        empty |$empty_self:ident| $extra_empty:expr,
        apply |$apply_self:ident, $apply_mode:ident| $extra_apply:block
    ) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            #[serde(skip)]
            pub is_default: Option<bool>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub display_name: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub description: Option<String>,
            $($extra_fields)*
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

        impl $name {
            pub fn is_empty(&self) -> bool {
                let $empty_self = self;
                self.is_default.is_none()
                    && self.display_name.is_none()
                    && self.description.is_none()
                    && $extra_empty
                    && self.request_override.is_empty()
                    && self.adapter_overrides.is_empty()
                    && !self.disabled
            }

            pub(crate) fn apply_to_mode(&self, base: Option<&$mode>) -> Option<$mode> {
                if self.disabled {
                    return None;
                }
                let mut mode = base.cloned().unwrap_or_default();
                if let Some(is_default) = self.is_default {
                    mode.is_default = is_default;
                }
                if let Some(display_name) = self.display_name.clone() {
                    mode.display_name = Some(display_name);
                }
                if let Some(description) = self.description.clone() {
                    mode.description = Some(description);
                }
                let $apply_self = self;
                let $apply_mode = &mut mode;
                $extra_apply
                mode.request_override = mode.request_override.merged_with(&self.request_override);
                merge_mode_adapter_overrides(&mut mode.adapter_overrides, &self.adapter_overrides);
                Some(mode)
            }
        }
    };
}

define_configured_model_mode!(
    ConfiguredModelThinkingMode,
    ModelThinkingMode,
    fields {
        #[serde(skip)]
        pub preset: Option<String>,
        #[serde(skip)]
        pub thinking: Option<ThinkingRequest>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub strategy: Option<ConfiguredThinkingStrategy>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub effort: Option<ReasoningEffort>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub budget_tokens: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub display: Option<ThinkingDisplay>,
    },
    empty |configured| configured.preset.is_none() && configured.thinking.is_none() && configured.strategy.is_none() && configured.effort.is_none() && configured.budget_tokens.is_none() && configured.display.is_none(),
    apply |configured, mode| {
        if let Some(preset) = configured.preset.clone() {
            mode.preset = Some(preset);
        }
        if let Some(thinking) = configured.thinking.clone() {
            mode.thinking = Some(thinking);
        }
    }
);

define_configured_model_mode!(
    ConfiguredModelSpeedMode,
    ModelSpeedMode,
    fields {},
    empty | _configured | true,
    apply | _configured,
    _mode | {}
);

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
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub thinking_modes: ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub speed_modes: ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
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
        let output_modalities = if self.output_modalities.is_empty() {
            Vec::new()
        } else {
            normalize_model_output_modalities(self.output_modalities.clone())
        };
        ModelMetadata {
            lifecycle: self.lifecycle,
            limits: crate::model::ModelTokenLimits {
                context_window_tokens: self.context_window_tokens,
                max_input_tokens: self.max_input_tokens,
                max_output_tokens: self.max_output_tokens,
            },
            description: self.description.clone(),
            knowledge_cutoff: self.knowledge_cutoff.clone(),
            release_date: self.release_date.clone(),
            last_updated: self.last_updated.clone(),
            open_weights: self.open_weights,
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.supports_verbosity,
            default_verbosity: self.default_verbosity.clone(),
            default_temperature: normalize_model_default_temperature(
                self.default_temperature.clone(),
            ),
            default_top_p: normalize_model_default_top_p(self.default_top_p.clone()),
            default_top_k: normalize_model_default_top_k(self.default_top_k),
            assistant_reasoning_interleaved: self.assistant_reasoning_interleaved,
            assistant_reasoning_field: normalize_model_assistant_reasoning_field(
                self.assistant_reasoning_field.clone(),
            ),
            output_modalities,
            pricing: non_empty_model_pricing(self.pricing.clone()),
        }
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
            .merged_with_fallbacks_from(capability_fallback);
        model.capabilities = self.capabilities.apply_to(base_capabilities);
        let base_metadata = model
            .metadata
            .clone()
            .merged_with_fallbacks_from(metadata_fallback);
        model.metadata = self.metadata().merged_with_fallbacks_from(&base_metadata);
        model.thinking_modes =
            apply_configured_thinking_modes(model.thinking_modes, &self.thinking_modes);
        model.speed_modes = apply_configured_speed_modes(model.speed_modes, &self.speed_modes);
        model
    }
}

pub(crate) fn apply_configured_modes<'a, Mode, ConfiguredMode: 'a, F>(
    mut modes: BTreeMap<String, Mode>,
    configured_modes: impl Iterator<Item = (&'a String, &'a ConfiguredMode)>,
    apply_to_mode: F,
) -> BTreeMap<String, Mode>
where
    F: Fn(&ConfiguredMode, Option<&Mode>) -> Option<Mode>,
{
    for (name, configured) in configured_modes {
        match apply_to_mode(configured, modes.get(name)) {
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

pub(crate) fn apply_configured_thinking_modes(
    modes: Vec<ModelThinkingMode>,
    configured_modes: &ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
) -> Vec<ModelThinkingMode> {
    let configured_default = configured_modes.default.mode().map(ToOwned::to_owned);
    let mut modes = modes
        .into_iter()
        .filter_map(|mode| {
            let selector = mode.selector().map(|selector| selector.into_owned());
            selector.map(|selector| (selector, mode))
        })
        .collect::<BTreeMap<_, _>>();

    for (selector, configured) in configured_modes.iter() {
        match configured.apply_to_mode(modes.get(selector.as_str())) {
            Some(mut mode) => {
                apply_configured_thinking_payload(selector, configured, &mut mode);
                mode.preset = Some(selector.clone());
                modes.insert(selector.clone(), mode);
            }
            None => {
                modes.remove(selector.as_str());
            }
        }
    }

    if let Some(default_selector) = configured_default {
        for (selector, mode) in &mut modes {
            mode.is_default = selector == &default_selector;
        }
    } else if !matches!(configured_modes.default, ConfiguredModeDefault::Clear) {
        retain_first_default(modes.values_mut());
    }

    modes.into_values().collect()
}

pub(crate) fn apply_configured_speed_modes(
    modes: BTreeMap<String, ModelSpeedMode>,
    configured_modes: &ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
) -> BTreeMap<String, ModelSpeedMode> {
    let configured_default = configured_modes.default.mode().map(ToOwned::to_owned);
    let mut modes = apply_configured_modes(modes, configured_modes.iter(), |configured, base| {
        configured.apply_to_mode(base)
    });
    if let Some(default_name) = configured_default {
        for (name, mode) in &mut modes {
            mode.is_default = name == &default_name;
        }
    } else if !matches!(configured_modes.default, ConfiguredModeDefault::Clear) {
        retain_first_default(modes.values_mut());
    }
    modes
}

fn retain_first_default<'a, Mode>(modes: impl Iterator<Item = &'a mut Mode>)
where
    Mode: 'a + ModeDefault,
{
    let mut found = false;
    for mode in modes {
        if mode.is_default() {
            if found {
                mode.set_default(false);
            } else {
                found = true;
            }
        }
    }
}

trait ModeDefault {
    fn is_default(&self) -> bool;
    fn set_default(&mut self, is_default: bool);
}

macro_rules! impl_mode_default {
    ($($mode:ty),+ $(,)?) => {
        $(
            impl ModeDefault for $mode {
                fn is_default(&self) -> bool {
                    self.is_default
                }

                fn set_default(&mut self, is_default: bool) {
                    self.is_default = is_default;
                }
            }
        )+
    };
}

impl_mode_default!(ModelThinkingMode, ModelSpeedMode);

pub fn configured_thinking_mode_selector(
    name: &str,
    _mode: &ConfiguredModelThinkingMode,
) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

pub fn configured_thinking_payload_selector(mode: &ConfiguredModelThinkingMode) -> Option<String> {
    ModelThinkingMode {
        preset: mode.preset.clone(),
        thinking: mode.thinking.clone(),
        ..Default::default()
    }
    .selector()
    .map(|value| value.into_owned())
}

impl From<Vec<ConfiguredModelThinkingMode>>
    for ConfiguredModelModeMap<ConfiguredModelThinkingMode>
{
    fn from(values: Vec<ConfiguredModelThinkingMode>) -> Self {
        let mut result = Self::default();
        for mut mode in values {
            if let Some(name) = configured_thinking_payload_selector(&mode) {
                if mode.is_default == Some(true) {
                    result.default = ConfiguredModeDefault::Mode(name.clone());
                }
                match mode.thinking.take() {
                    Some(ThinkingRequest::Disabled) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Disabled);
                    }
                    Some(ThinkingRequest::Effort { effort }) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Effort);
                        mode.effort = Some(effort);
                    }
                    Some(ThinkingRequest::Budget { budget_tokens }) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Budget);
                        mode.budget_tokens = Some(budget_tokens);
                    }
                    Some(ThinkingRequest::Adaptive { effort, display }) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Adaptive);
                        mode.effort = effort;
                        mode.display = display;
                    }
                    None => {
                        mode.strategy
                            .get_or_insert(ConfiguredThinkingStrategy::RequestOnly);
                    }
                }
                mode.preset = None;
                result.modes.insert(name, mode);
            }
        }
        result
    }
}

pub fn configured_thinking_mode_to_model(
    name: &str,
    mode: &ConfiguredModelThinkingMode,
) -> ModelThinkingMode {
    let mut model = ModelThinkingMode {
        is_default: mode.is_default.unwrap_or(false),
        preset: Some(name.to_owned()),
        display_name: mode.display_name.clone(),
        description: mode.description.clone(),
        thinking: mode.thinking.clone(),
        request_override: mode.request_override.clone(),
        adapter_overrides: mode.adapter_overrides.clone(),
    };
    apply_configured_thinking_payload(name, mode, &mut model);
    model
}

fn apply_configured_thinking_payload(
    _name: &str,
    configured: &ConfiguredModelThinkingMode,
    mode: &mut ModelThinkingMode,
) {
    if configured.thinking.is_some() {
        return;
    }
    mode.thinking = match configured.strategy {
        None => None,
        Some(ConfiguredThinkingStrategy::Disabled) => Some(ThinkingRequest::Disabled),
        Some(ConfiguredThinkingStrategy::Effort) => configured
            .effort
            .map(|effort| ThinkingRequest::Effort { effort }),
        Some(ConfiguredThinkingStrategy::Budget) => configured
            .budget_tokens
            .map(|budget_tokens| ThinkingRequest::Budget { budget_tokens }),
        Some(ConfiguredThinkingStrategy::Adaptive) => Some(ThinkingRequest::Adaptive {
            effort: configured.effort,
            display: configured.display,
        }),
        Some(ConfiguredThinkingStrategy::RequestOnly) => None,
    };
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
        self.models.get(model.as_ref())
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
            .map(|configured| configured.metadata().merged_with_fallbacks_from(&base))
            .unwrap_or(base)
    }

    fn configured_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        let mut base = self
            .target
            .model_thinking_modes_for_adapter(adapter_id, model);
        if let Some(family) = self.target.capability_family() {
            let metadata = self.model_metadata_for_adapter(adapter_id, model);
            for mode in crate::provider::default_model_mode_registry().thinking_modes_for_family(
                family,
                adapter_id,
                model.as_ref(),
                &metadata,
            ) {
                let selector = mode.selector().map(|selector| selector.into_owned());
                if selector.is_some_and(|selector| {
                    base.iter()
                        .any(|existing| existing.selector().as_deref() == Some(selector.as_str()))
                }) {
                    continue;
                }
                base.push(mode);
            }
        }
        self.configured_model(model)
            .map(|configured| {
                apply_configured_thinking_modes(base.clone(), &configured.thinking_modes)
            })
            .unwrap_or(base)
    }

    fn configured_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        let base = self.target.model_speed_modes(model);
        self.configured_model(model)
            .map(|configured| apply_configured_speed_modes(base.clone(), &configured.speed_modes))
            .unwrap_or(base)
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

    impl_model_runtime_target_defaults!();

    impl_model_runtime_adapter_agnostic_methods! {
        fn model_capabilities / model_capabilities_for_adapter (self, model) -> ModelCapabilities {
            self.configured_capabilities(model)
        }

        fn model_metadata / model_metadata_for_adapter (self, model) -> ModelMetadata {
            self.configured_metadata(model)
        }

        fn model_speed_modes / model_speed_modes_for_adapter (self, model) -> BTreeMap<String, ModelSpeedMode> {
            self.configured_speed_modes(model)
        }
    }

    impl_model_runtime_base_via_adapter_methods! {
        fn model_thinking_modes / model_thinking_modes_for_adapter (&self, model: &ModelId) -> Vec<ModelThinkingMode>;
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        self.configured_thinking_modes_for_adapter(adapter_id, model)
    }

    impl_model_runtime_target_methods! {
        fn supports_prompt_continuation / supports_prompt_continuation_for_adapter (&self, model: &ModelId) -> bool;
        fn native_compaction_enabled / native_compaction_enabled_for_adapter (&self, model: &ModelId) -> bool;
        fn prompt_cache_shape / prompt_cache_shape_for_adapter (&self, model: &ModelId) -> Option<PromptCacheShape>;
        fn provider_native_tools_config / provider_native_tools_config_for_adapter (&self, model: &ModelId) -> ProviderNativeToolsConfig;
        fn agena_tool_mode / agena_tool_mode_for_adapter (&self, model: &ModelId) -> AgenaToolMode;
    }

    fn validate_provider_native_tools_request(
        &self,
        adapter_id: Option<&AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        self.target
            .validate_provider_native_tools_request(adapter_id, request)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        let mut listed_ids = std::collections::BTreeSet::new();

        for model in &mut models {
            listed_ids.insert(model.id.to_string());
            if let Some(configured) = self.models.get(model.id.as_ref()) {
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
    ) -> Result<Option<crate::provider::ProviderCompactionOutput>, AppError> {
        self.forward_compact_conversation(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<Option<crate::provider::ProviderCompactionOutput>, AppError> {
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
    use super::{ConfiguredModelDefinition, apply_configured_thinking_modes};

    #[test]
    fn named_mode_maps_round_trip() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": "medium",
                "low": { "strategy": "effort", "effort": "low" },
                "medium": { "strategy": "effort", "effort": "medium" },
                "high": { "strategy": "effort", "effort": "high" }
            },
            "speed_modes": {
                "default": "fast", "standard": {}, "fast": {}
            }
        }))
        .unwrap();
        assert_eq!(definition.thinking_modes.default.mode(), Some("medium"));
        assert_eq!(definition.speed_modes.default.mode(), Some("fast"));
        let serialized = serde_json::to_value(definition).expect("definition should serialize");
        assert_eq!(serialized["thinking_modes"]["default"], "medium");
        assert_eq!(serialized["speed_modes"]["default"], "fast");

        assert!(
            serde_json::from_value::<ConfiguredModelDefinition>(serde_json::json!({
                "thinking_modes": [{ "thinking": { "type": "effort", "effort": "high" } }]
            }))
            .is_err()
        );

        let cleared: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": null,
                "low": { "strategy": "effort", "effort": "low" }
            }
        }))
        .unwrap();
        assert!(matches!(
            cleared.thinking_modes.default,
            super::ConfiguredModeDefault::Clear
        ));
        assert_eq!(
            serde_json::to_value(cleared).unwrap()["thinking_modes"]["default"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn named_modes_use_explicit_payload_and_default() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": "high",
                "off": { "strategy": "disabled" },
                "high": { "strategy": "effort", "effort": "high" }
            }
        }))
        .unwrap();
        let modes = apply_configured_thinking_modes(Vec::new(), &definition.thinking_modes);
        assert_eq!(
            modes
                .iter()
                .find(|mode| mode.is_default)
                .unwrap()
                .selector()
                .as_deref(),
            Some("high")
        );
        assert!(
            modes
                .iter()
                .any(|mode| mode.selector().as_deref() == Some("off"))
        );
    }

    #[test]
    fn named_budget_mode_uses_flat_strategy_fields() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": "deep",
                "deep": { "strategy": "budget", "budget_tokens": 16000 }
            }
        }))
        .unwrap();
        let modes = apply_configured_thinking_modes(Vec::new(), &definition.thinking_modes);
        let mode = modes.first().unwrap();
        assert_eq!(mode.selector().as_deref(), Some("deep"));
        assert_eq!(
            mode.thinking,
            Some(crate::provider::ThinkingRequest::Budget {
                budget_tokens: 16000
            })
        );
        assert!(mode.is_default);
    }

    #[test]
    fn runtime_modes_serialize_with_explicit_strategies() {
        let modes: super::ConfiguredModelModeMap<super::ConfiguredModelThinkingMode> = vec![
            super::ConfiguredModelThinkingMode {
                thinking: Some(crate::provider::ThinkingRequest::Disabled),
                ..Default::default()
            },
            super::ConfiguredModelThinkingMode {
                thinking: Some(crate::provider::ThinkingRequest::Effort {
                    effort: crate::provider::ReasoningEffort::High,
                }),
                ..Default::default()
            },
        ]
        .into();

        let value = serde_json::to_value(modes).unwrap();
        assert_eq!(value["off"]["strategy"], "disabled");
        assert_eq!(value["high"]["strategy"], "effort");
        assert_eq!(value["high"]["effort"], "high");
    }
}
