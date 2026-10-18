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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        PendingProviderNativeToolCall, PendingToolCall,
        pending_provider_native_tool_call_stream_key, pending_tool_call_stream_key,
    };

    #[test]
    fn provider_call_id_merges_changing_adapter_stream_keys() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();
        let mut aliases = BTreeMap::new();

        let first = pending_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:0".to_string(),
            Some("call_shared"),
        );
        assert_eq!(first, "id:call_shared");
        pending.insert(
            first.clone(),
            PendingToolCall {
                id: Some("call_shared".to_string()),
                ..Default::default()
            },
        );

        let replay = pending_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:6".to_string(),
            Some("call_shared"),
        );
        assert_eq!(replay, first);
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn distinct_provider_call_ids_do_not_merge_even_when_stream_key_repeats() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();
        let mut aliases = BTreeMap::new();

        let first = pending_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:0".to_string(),
            Some("call_one"),
        );
        pending.insert(
            first.clone(),
            PendingToolCall {
                id: Some("call_one".to_string()),
                ..Default::default()
            },
        );

        let second = pending_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:0".to_string(),
            Some("call_two"),
        );
        assert_ne!(first, second);
        pending.insert(
            second.clone(),
            PendingToolCall {
                id: Some("call_two".to_string()),
                ..Default::default()
            },
        );

        assert_eq!(pending.len(), 2);
        assert!(pending.contains_key(first.as_str()));
        assert!(pending.contains_key(second.as_str()));
    }

    #[test]
    fn provider_id_rekeys_an_earlier_idless_stream_without_a_second_call() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();
        let mut aliases = BTreeMap::new();
        pending.insert("idx:0".to_string(), PendingToolCall::default());

        let key = pending_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:0".to_string(),
            Some("call_shared"),
        );

        assert_eq!(key, "id:call_shared");
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key("id:call_shared"));
        assert!(!pending.contains_key("idx:0"));
        assert_eq!(
            aliases.get("idx:0").map(String::as_str),
            Some("id:call_shared")
        );
    }

    #[test]
    fn idless_trailer_uses_the_original_stream_key_alias_after_provider_id_rekey() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();
        let mut aliases = BTreeMap::new();

        let first = pending_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:0".to_string(),
            Some("call_shared"),
        );
        pending.insert(
            first.clone(),
            PendingToolCall {
                id: Some("call_shared".to_string()),
                ..Default::default()
            },
        );

        let trailer =
            pending_tool_call_stream_key(&mut pending, &mut aliases, "idx:0".to_string(), None);
        assert_eq!(trailer, first);
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn provider_native_item_id_merges_started_and_done_events_with_different_keys() {
        let mut pending = BTreeMap::<String, PendingProviderNativeToolCall>::new();
        let mut aliases = BTreeMap::new();

        let started = pending_provider_native_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "item:ws_1".to_owned(),
            Some("ws_1"),
        );
        pending.insert(
            started.clone(),
            PendingProviderNativeToolCall {
                id: Some("ws_1".to_owned()),
                call_id: Some(9),
                ..Default::default()
            },
        );

        let completed = pending_provider_native_tool_call_stream_key(
            &mut pending,
            &mut aliases,
            "idx:4".to_owned(),
            Some("ws_1"),
        );
        assert_eq!(completed, started);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[&completed].call_id, Some(9));
    }
}
