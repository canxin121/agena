use agena_domain::{ModelId, ModelSpeedModeRequestOverride, ThinkingRequest};
use agena_provider::{
    CompletionInputMessage, CompletionRequest, ProviderNativeToolsConfig, ToolApiDefinition,
};

/// Runtime-neutral inputs assembled after core has projected persisted messages
/// and local tool bindings into provider contracts.
pub struct CompletionRequestInputs {
    pub model: ModelId,
    pub system: Option<String>,
    pub messages: Vec<CompletionInputMessage>,
    pub tool_api_functions: Vec<ToolApiDefinition>,
    pub provider_native_tools: ProviderNativeToolsConfig,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub prompt_cache_key: Option<String>,
    pub previous_response_id: Option<String>,
    pub prompt_window_generation: Option<u64>,
    pub thinking: Option<ThinkingRequest>,
    pub verbosity: Option<String>,
    pub request_override: ModelSpeedModeRequestOverride,
}

pub fn build_completion_request(inputs: CompletionRequestInputs) -> CompletionRequest {
    CompletionRequest {
        model: inputs.model,
        system: inputs.system,
        messages: inputs.messages,
        tool_api_functions: inputs.tool_api_functions,
        provider_native_tools: inputs.provider_native_tools,
        disable_tools: false,
        temperature: inputs.temperature,
        max_output_tokens: inputs.max_output_tokens,
        prompt_cache_key: inputs.prompt_cache_key,
        previous_response_id: inputs.previous_response_id,
        prompt_window_generation: inputs.prompt_window_generation,
        provider_compaction: None,
        stop_sequences: Vec::new(),
        top_p: None,
        top_k: None,
        seed: None,
        thinking: inputs.thinking,
        verbosity: inputs.verbosity,
        response_format: None,
        responses_api_metadata: None,
        request_override: inputs.request_override,
    }
}
