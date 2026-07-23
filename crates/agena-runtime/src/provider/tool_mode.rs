use agena_domain::ModelId;
use agena_provider::{CompletionStreamEvent, ProviderToolModeViolation};
use futures_util::StreamExt;

use crate::error::AppError;

use super::core::CompletionEventStream;

/// Runtime owns the application-stream wrapper only. Request transformation and
/// response validation live in `agena-provider` with the provider contract.
pub(crate) fn guard_disabled_stream(
    mut stream: CompletionEventStream,
    provider_id: String,
    model: ModelId,
) -> CompletionEventStream {
    Box::pin(async_stream::stream! {
        while let Some(item) = stream.next().await {
            match item {
                Ok(
                    CompletionStreamEvent::ToolCallDelta { .. }
                    | CompletionStreamEvent::ToolCallSnapshot { .. },
                ) => {
                    yield Err(disabled_tool_response_error(
                        provider_id.as_str(),
                        &model,
                        "the backend returned a native tool call",
                    ));
                    break;
                }
                Ok(
                    CompletionStreamEvent::ProviderNativeToolCallStarted { .. }
                    | CompletionStreamEvent::ProviderNativeToolCallCompleted { .. },
                ) => {
                    yield Err(disabled_tool_response_error(
                        provider_id.as_str(),
                        &model,
                        "the backend used a provider-native tool",
                    ));
                    break;
                }
                item => yield item,
            }
        }
    })
}

fn disabled_tool_response_error(provider_id: &str, model: &ModelId, reason: &str) -> AppError {
    ProviderToolModeViolation::disabled_tool_response(provider_id, model, reason).into()
}
