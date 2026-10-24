//! Provider orchestration contains no transcript accumulator or mutable session.
use crate::{AppError, provider_state::PartProviderState};
use agena_domain::{AssistantReasoningField, FinishReason, StructuredObject, ToolInvocation};
use agena_provider::{CompletionFinishReason, CompletionRequest};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct SessionProcessor {
    plugins: Arc<agena_plugin_host::PluginHost>,
}

mod helpers;
mod tool_call_helpers;
pub(crate) use helpers::*;
pub(crate) use tool_call_helpers::*;

impl SessionProcessor {
    pub fn new(plugins: Arc<agena_plugin_host::PluginHost>) -> Self {
        Self { plugins }
    }

    /// Apply the `chat.params` plugin hook chain to a [`CompletionRequest`]
    /// before sending it to the provider.
    pub(crate) async fn apply_chat_params_hook(
        &self,
        provider_id: &str,
        model_id: &str,
        session_id: i64,
        request: &mut CompletionRequest,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) {
        let plugins = &self.plugins;
        if plugins.is_empty() {
            return;
        }
        let mut params = serde_json::Map::new();
        if let Some(t) = request.temperature {
            params.insert("temperature".into(), serde_json::json!(t));
        }
        if let Some(m) = request.max_output_tokens {
            params.insert("max_output_tokens".into(), serde_json::json!(m));
        }
        let input = agena_plugin_host::ChatParamsInput {
            provider: provider_id.to_string(),
            model: model_id.to_string(),
            params: serde_json::Value::Object(params),
            session_id: Some(session_id),
        };
        match plugins
            .dispatch_chat_params_cancellable(input, cancellation)
            .await
        {
            Ok(updated) => {
                if let Some(t) = updated.params.get("temperature").and_then(|v| v.as_f64()) {
                    request.temperature = Some(t as f32);
                }
                if let Some(m) = updated
                    .params
                    .get("max_output_tokens")
                    .and_then(|v| v.as_u64())
                {
                    request.max_output_tokens = Some(m as u32);
                }
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::chat_params",
                    "chat.params hook failed: {err}"
                );
            }
        }
    }
}
