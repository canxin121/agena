use serde::{Deserialize, Serialize};

use crate::{ModelId, ModelLifecycle};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelTokenLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}
impl ModelTokenLimits {
    pub fn is_empty(&self) -> bool {
        self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
    }
    pub fn merged_with_fallbacks_from(self, fallback: &Self) -> Self {
        Self {
            context_window_tokens: self
                .context_window_tokens
                .or(fallback.context_window_tokens),
            max_input_tokens: self.max_input_tokens.or(fallback.max_input_tokens),
            max_output_tokens: self.max_output_tokens.or(fallback.max_output_tokens),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelPricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<ModelPricingTier>,
}
impl ModelPricing {
    pub fn is_empty(&self) -> bool {
        self.input_usd_per_million_tokens.is_none()
            && self.output_usd_per_million_tokens.is_none()
            && self.cache_read_usd_per_million_tokens.is_none()
            && self.cache_write_usd_per_million_tokens.is_none()
            && self.tiers.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelPricingTier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_usd_per_million_tokens: Option<String>,
}
impl ModelPricingTier {
    pub fn is_empty(&self) -> bool {
        self.tier_type.is_none()
            && self.size_tokens.is_none()
            && self.input_usd_per_million_tokens.is_none()
            && self.output_usd_per_million_tokens.is_none()
            && self.cache_read_usd_per_million_tokens.is_none()
            && self.cache_write_usd_per_million_tokens.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "ModelTokenLimits::is_empty")]
    pub limits: ModelTokenLimits,
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
}
impl ModelMetadata {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.limits.is_empty()
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
    }
    pub fn supported_verbosity_levels_for_model(&self, model_id: &ModelId) -> Vec<String> {
        let default = self
            .default_verbosity
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_ascii_lowercase);
        if !self.supports_verbosity.unwrap_or(false) && default.is_none() {
            return Vec::new();
        }
        let mut levels = if model_id
            .as_ref()
            .trim()
            .to_ascii_lowercase()
            .contains("gpt-5")
            && model_id
                .as_ref()
                .trim()
                .to_ascii_lowercase()
                .contains("-chat")
        {
            vec!["medium".to_owned()]
        } else {
            vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()]
        };
        if let Some(default) = default
            && !levels.iter().any(|v| v == &default)
        {
            levels.push(default);
        }
        levels
    }
    pub fn supports_verbosity_level_for_model(&self, model_id: &ModelId, verbosity: &str) -> bool {
        let normalized = verbosity.trim().to_ascii_lowercase();
        self.supported_verbosity_levels_for_model(model_id)
            .into_iter()
            .any(|v| v == normalized)
    }
    pub fn supports_parallel_tool_calls_for_model(&self) -> bool {
        self.supports_parallel_tool_calls.unwrap_or(false)
    }
    pub fn parsed_default_temperature(&self) -> Option<f32> {
        parse_optional_f32(self.default_temperature.as_deref(), |v| {
            v.is_finite() && v >= 0.0
        })
    }
    pub fn parsed_default_top_p(&self) -> Option<f32> {
        parse_optional_f32(self.default_top_p.as_deref(), |v| {
            v.is_finite() && v > 0.0 && v <= 1.0
        })
    }
    pub fn merged_with_fallbacks_from(self, f: &Self) -> Self {
        Self {
            lifecycle: self.lifecycle.or(f.lifecycle),
            limits: self.limits.merged_with_fallbacks_from(&f.limits),
            description: self.description.or_else(|| f.description.clone()),
            knowledge_cutoff: self.knowledge_cutoff.or_else(|| f.knowledge_cutoff.clone()),
            release_date: self.release_date.or_else(|| f.release_date.clone()),
            last_updated: self.last_updated.or_else(|| f.last_updated.clone()),
            open_weights: self.open_weights.or(f.open_weights),
            supports_parallel_tool_calls: self
                .supports_parallel_tool_calls
                .or(f.supports_parallel_tool_calls),
            supports_verbosity: self.supports_verbosity.or(f.supports_verbosity),
            default_verbosity: self
                .default_verbosity
                .or_else(|| f.default_verbosity.clone()),
            default_temperature: self
                .default_temperature
                .or_else(|| f.default_temperature.clone()),
            default_top_p: self.default_top_p.or_else(|| f.default_top_p.clone()),
            default_top_k: self.default_top_k.or(f.default_top_k),
            assistant_reasoning_interleaved: self
                .assistant_reasoning_interleaved
                .or(f.assistant_reasoning_interleaved),
            assistant_reasoning_field: self
                .assistant_reasoning_field
                .or_else(|| f.assistant_reasoning_field.clone()),
            output_modalities: if self.output_modalities.is_empty() {
                f.output_modalities.clone()
            } else {
                self.output_modalities
            },
            pricing: self.pricing.or_else(|| f.pricing.clone()),
        }
    }
}
fn parse_optional_f32(value: Option<&str>, predicate: impl Fn(f32) -> bool) -> Option<f32> {
    value
        .and_then(|v| v.trim().parse().ok())
        .filter(|v| predicate(*v))
}
pub fn normalize_model_default_temperature(value: Option<String>) -> Option<String> {
    normalize_decimal(value, |v| v.is_finite() && v >= 0.0)
}
pub fn normalize_model_default_top_p(value: Option<String>) -> Option<String> {
    normalize_decimal(value, |v| v.is_finite() && v > 0.0 && v <= 1.0)
}
pub fn normalize_model_default_top_k(value: Option<u32>) -> Option<u32> {
    value.filter(|v| *v > 0)
}
pub fn normalize_model_assistant_reasoning_field(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let v = raw.trim().to_ascii_lowercase();
        matches!(v.as_str(), "reasoning_content" | "reasoning_details").then_some(v)
    })
}
pub fn normalize_model_output_modalities(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|v| {
            let v = v.trim();
            (!v.is_empty()).then(|| v.to_owned())
        })
        .collect()
}
pub fn non_empty_model_pricing(pricing: Option<ModelPricing>) -> Option<ModelPricing> {
    pricing.filter(|v| !v.is_empty())
}
fn normalize_decimal(value: Option<String>, predicate: impl Fn(f32) -> bool) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then_some(trimmed).and_then(|v| {
            v.parse::<f32>()
                .ok()
                .filter(|n| predicate(*n))
                .map(|_| v.to_owned())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{ModelMetadata, ModelPricing, ModelTokenLimits};

    #[test]
    fn metadata_merge_and_wire_shape_are_stable() {
        let primary = ModelMetadata {
            limits: ModelTokenLimits {
                max_output_tokens: Some(256),
                ..ModelTokenLimits::default()
            },
            pricing: Some(ModelPricing {
                input_usd_per_million_tokens: Some("1.5".to_owned()),
                ..ModelPricing::default()
            }),
            ..ModelMetadata::default()
        };
        let fallback = ModelMetadata {
            limits: ModelTokenLimits {
                context_window_tokens: Some(8_192),
                ..ModelTokenLimits::default()
            },
            ..ModelMetadata::default()
        };

        let merged = primary.merged_with_fallbacks_from(&fallback);
        assert_eq!(merged.limits.context_window_tokens, Some(8_192));
        assert_eq!(merged.limits.max_output_tokens, Some(256));
        assert_eq!(
            serde_json::to_value(&merged).unwrap()["pricing"]["input_usd_per_million_tokens"],
            "1.5"
        );
    }
}
