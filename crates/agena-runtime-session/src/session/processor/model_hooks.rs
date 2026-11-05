//! Request-local model input hooks. Hooks operate on a projection and never
//! rewrite the persisted session or its provider replay facts.

use agena_plugin_host::{
    ChatDirection, ChatHeadersInput, ChatMessage, ChatMessageInput, ChatMessagesTransformInput,
    ChatSystemTransformInput, HookSubscription, ModelRequestAfterInput, ModelRequestBeforeInput,
    ModelRequestErrorInput, PluginHost,
};
use agena_provider::{CompletionInputPart, CompletionInputProviderState, CompletionInputRun};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::SessionProcessor;
use crate::{
    AppError,
    provider::{ProviderError, ProviderRequestHeaderHook},
};
use agena_domain::ModelRef;
use tokio_util::sync::CancellationToken;

const MAX_HOOK_PROJECTION_BYTES: usize = 16 * 1024 * 1024;

struct ChatHeaderPolicy {
    plugins: Arc<PluginHost>,
    session_id: i64,
}

impl ProviderRequestHeaderHook for ChatHeaderPolicy {
    fn resolve(
        &self,
        provider_id: &str,
        headers: BTreeMap<String, String>,
        cancellation: Option<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = Result<BTreeMap<String, String>, ProviderError>> + Send + '_>>
    {
        let provider = provider_id.to_owned();
        Box::pin(async move {
            let output = self
                .plugins
                .dispatch_chat_headers_cancellable(
                    ChatHeadersInput {
                        provider,
                        headers,
                        session_id: self.session_id,
                    },
                    cancellation,
                )
                .await
                .map_err(|error| {
                    ProviderError::Config(format!(
                        "chat.headers hook: {}",
                        error.diagnostic_message()
                    ))
                })?;
            Ok(output.headers)
        })
    }
}

fn invalid_projection(error: impl std::fmt::Display) -> AppError {
    AppError::Config(format!("invalid model input hook projection: {error}"))
}

/// The model message format can represent exact structured replay parts, while
/// preserving the ergonomic string form for text-only hooks.
fn to_chat_message(run: &CompletionInputRun) -> Result<ChatMessage, AppError> {
    let content = match run.parts.as_slice() {
        [CompletionInputPart::Text { text }] => Value::String(text.clone()),
        parts => serde_json::to_value(parts).map_err(invalid_projection)?,
    };
    let role = serde_json::to_value(run.role)
        .map_err(invalid_projection)?
        .as_str()
        .ok_or_else(|| invalid_projection("role is not a string"))?
        .to_owned();
    let provider_state = (!run.provider_state.is_empty())
        .then(|| serde_json::to_value(&run.provider_state))
        .transpose()
        .map_err(invalid_projection)?;
    Ok(ChatMessage {
        role,
        content,
        provider_state,
    })
}

fn from_chat_message(message: ChatMessage) -> Result<Option<CompletionInputRun>, AppError> {
    if message.content.is_null() {
        return Ok(None);
    }
    let role = serde_json::from_value(Value::String(message.role)).map_err(invalid_projection)?;
    let parts = match message.content {
        Value::String(text) => vec![CompletionInputPart::Text { text }],
        Value::Array(parts) => {
            serde_json::from_value(Value::Array(parts)).map_err(invalid_projection)?
        }
        other => {
            return Err(invalid_projection(format!(
                "message content must be text or structured parts, got {other}"
            )));
        }
    };
    let provider_state = message
        .provider_state
        .map(serde_json::from_value)
        .transpose()
        .map_err(invalid_projection)?
        .unwrap_or_else(CompletionInputProviderState::default);
    Ok(Some(CompletionInputRun {
        role,
        parts,
        provider_state,
    }))
}

/// A model-input hook can contribute textual context, but cannot reassign,
/// forge, remove or regroup committed tool calls, tool results, or system
/// notices. Roles and message grouping remain part of their identities.
fn committed_parts(
    turns: &[CompletionInputRun],
) -> Vec<(agena_domain::Role, Vec<CompletionInputPart>)> {
    turns
        .iter()
        .filter_map(|run| {
            let parts = run
                .parts
                .iter()
                .filter(|part| {
                    matches!(
                        part,
                        CompletionInputPart::ToolCall { .. }
                            | CompletionInputPart::ToolResult { .. }
                            | CompletionInputPart::SystemMessage { .. }
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            (!parts.is_empty()).then_some((run.role, parts))
        })
        .collect()
}

/// Native provider continuations are committed replay facts. Plugins may edit
/// model-facing text but must not forge or discard opaque protocol state.
fn replay_facts(
    turns: &[CompletionInputRun],
) -> Result<Vec<(agena_domain::Role, Value)>, AppError> {
    turns
        .iter()
        .filter(|run| !run.provider_state.is_empty())
        .map(|run| {
            Ok((
                run.role,
                serde_json::to_value(&run.provider_state).map_err(invalid_projection)?,
            ))
        })
        .collect()
}

impl SessionProcessor {
    pub(crate) async fn before_model_request(
        &self,
        session_id: i64,
        route: &ModelRef,
        request_id: String,
    ) -> Result<(), AppError> {
        self.plugins
            .dispatch_model_request_before(ModelRequestBeforeInput {
                session_id,
                provider: route.provider_id.to_string(),
                model: route.model_id.to_string(),
                request_id,
            })
            .await
            .map_err(|error| AppError::Config(error.diagnostic_message().to_owned()))
    }

    pub(crate) async fn after_model_request(
        &self,
        session_id: i64,
        route: &ModelRef,
        request_id: String,
        status: String,
        duration_ms: u64,
        usage: Option<Value>,
    ) {
        self.plugins
            .broadcast_model_request_after(ModelRequestAfterInput {
                session_id,
                provider: route.provider_id.to_string(),
                model: route.model_id.to_string(),
                request_id,
                status,
                duration_ms,
                usage,
            })
            .await;
    }

    pub(crate) async fn recover_model_request(
        &self,
        session_id: i64,
        route: &ModelRef,
        request_id: String,
        message: String,
        attempt: u32,
    ) -> bool {
        self.plugins
            .dispatch_model_request_error(ModelRequestErrorInput {
                session_id,
                provider: route.provider_id.to_string(),
                model: route.model_id.to_string(),
                request_id,
                message,
                attempt,
            })
            .await
    }

    pub(crate) fn request_header_hook(
        &self,
        session_id: i64,
    ) -> Arc<dyn ProviderRequestHeaderHook> {
        Arc::new(ChatHeaderPolicy {
            plugins: Arc::clone(&self.plugins),
            session_id,
        })
    }

    /// Transform a request projection before token budgeting and provider I/O.
    /// All returned content is validated, bounded, and never persisted back into
    /// the authoritative session history.
    pub(crate) async fn transform_model_input(
        &self,
        session_id: i64,
        system: &mut String,
        turns: &mut Vec<CompletionInputRun>,
    ) -> Result<(), AppError> {
        let system_hook = self
            .plugins
            .has_subscription(HookSubscription::CHAT_SYSTEM_TRANSFORM);
        let message_hook = self
            .plugins
            .has_subscription(HookSubscription::CHAT_MESSAGE);
        let messages_hook = self
            .plugins
            .has_subscription(HookSubscription::CHAT_MESSAGES_TRANSFORM);
        if !system_hook && !message_hook && !messages_hook {
            return Ok(());
        }
        if system_hook {
            let updated = self
                .plugins
                .dispatch_chat_system_transform(ChatSystemTransformInput {
                    session_id,
                    current_system: system.clone(),
                })
                .await
                .map_err(|error| invalid_projection(error.diagnostic_message()))?;
            *system = updated.current_system;
            if system.len() > MAX_HOOK_PROJECTION_BYTES {
                return Err(invalid_projection("system prompt exceeds hook byte budget"));
            }
        }
        if !message_hook && !messages_hook {
            return Ok(());
        }

        let original_committed_parts = committed_parts(turns);
        let original_replay_facts = replay_facts(turns)?;
        let mut messages = Vec::with_capacity(turns.len());
        for turn in turns.iter() {
            let message = to_chat_message(turn)?;
            let updated = if message_hook {
                self.plugins
                    .dispatch_chat_message(ChatMessageInput {
                        session_id,
                        direction: ChatDirection::ToProvider,
                        message,
                    })
                    .await
                    .map_err(|error| invalid_projection(error.diagnostic_message()))?
                    .message
            } else {
                message
            };
            if !updated.content.is_null() {
                messages.push(updated);
            }
        }

        if messages_hook {
            messages = self
                .plugins
                .dispatch_chat_messages_transform(ChatMessagesTransformInput {
                    session_id,
                    messages,
                })
                .await
                .map_err(|error| invalid_projection(error.diagnostic_message()))?
                .messages;
        }
        let size = serde_json::to_vec(&messages)
            .map_err(invalid_projection)?
            .len();
        if size > MAX_HOOK_PROJECTION_BYTES {
            return Err(invalid_projection("messages exceed hook byte budget"));
        }
        let updated = messages
            .into_iter()
            .map(from_chat_message)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if committed_parts(&updated) != original_committed_parts
            || replay_facts(&updated)? != original_replay_facts
        {
            return Err(invalid_projection(
                "model input hook cannot alter committed replay facts",
            ));
        }
        *turns = updated;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::Role;

    #[test]
    fn request_projection_preserves_reasoning_and_tool_replay() {
        let original = CompletionInputRun {
            role: Role::Assistant,
            parts: vec![
                CompletionInputPart::Text {
                    text: "hello".into(),
                },
                CompletionInputPart::Reasoning {
                    text: "opaque".into(),
                },
            ],
            provider_state: CompletionInputProviderState {
                response_id: Some("response-1".into()),
                ..Default::default()
            },
        };
        let message = to_chat_message(&original).unwrap();
        assert_eq!(from_chat_message(message).unwrap(), Some(original));
    }

    #[test]
    fn plain_text_can_be_injected_as_a_system_message() {
        let injected = from_chat_message(ChatMessage::system("memory"))
            .unwrap()
            .unwrap();
        assert_eq!(injected.role, Role::System);
        assert_eq!(
            injected.parts,
            vec![CompletionInputPart::Text {
                text: "memory".into()
            }]
        );
    }

    #[test]
    fn committed_notices_cannot_be_reassigned_to_another_role() {
        let original = CompletionInputRun {
            role: Role::System,
            parts: vec![
                CompletionInputPart::Text {
                    text: "context".into(),
                },
                CompletionInputPart::SystemMessage {
                    text: "trusted notification".into(),
                },
            ],
            provider_state: CompletionInputProviderState::default(),
        };
        let reassigned = CompletionInputRun {
            role: Role::User,
            ..original.clone()
        };
        assert_ne!(
            committed_parts(std::slice::from_ref(&original)),
            committed_parts(&[reassigned]),
        );
        let rewritten_text = CompletionInputRun {
            parts: vec![
                CompletionInputPart::Text {
                    text: "rewritten".into(),
                },
                CompletionInputPart::SystemMessage {
                    text: "trusted notification".into(),
                },
            ],
            ..original.clone()
        };
        assert_eq!(
            committed_parts(&[original]),
            committed_parts(&[rewritten_text])
        );
    }

    #[test]
    fn model_hooks_cannot_forge_provider_continuation_state() {
        let source = CompletionInputRun {
            role: Role::Assistant,
            parts: vec![CompletionInputPart::Text {
                text: "hello".into(),
            }],
            provider_state: CompletionInputProviderState {
                response_id: Some("response-one".into()),
                ..Default::default()
            },
        };
        let forged = CompletionInputRun {
            provider_state: CompletionInputProviderState {
                response_id: Some("response-two".into()),
                ..Default::default()
            },
            ..source.clone()
        };
        assert_ne!(
            replay_facts(&[source]).unwrap(),
            replay_facts(&[forged]).unwrap()
        );
    }
}
