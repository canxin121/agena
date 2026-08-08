//! Gemini model-list response projection.

use agena_domain::{ModelMetadata, ModelTokenLimits};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
/// Response of the Gemini model list endpoint.
pub struct GeminiModelListResponse {
    #[serde(default)]
    pub models: Vec<GeminiModel>,
}

#[derive(Debug, Deserialize)]
/// A Gemini model descriptor.
pub struct GeminiModel {
    pub name: String,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "inputTokenLimit")]
    pub input_token_limit: Option<u64>,
    #[serde(default, rename = "outputTokenLimit")]
    pub output_token_limit: Option<u64>,
}

impl GeminiModel {
    /// Gemini exposes input/output limits rather than a separate context
    /// window; its input ceiling is the prompt-window budget.
    pub fn metadata(&self) -> ModelMetadata {
        let input_limit = self.input_token_limit.map(clamp_u64_to_u32);
        ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits {
                context_window_tokens: input_limit,
                max_input_tokens: input_limit,
                max_output_tokens: self.output_token_limit.map(clamp_u64_to_u32),
            },
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        }
    }
}

fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}
