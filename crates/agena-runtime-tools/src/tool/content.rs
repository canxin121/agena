//! Explicit model reads use membership-authorized kernel sources and bounded pages.

use agena_domain::{ContentKind, ToolInvocation, ToolOutput};
use agena_storage::kernel::{EventCursor, SessionId, TextCursor};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolInvocationExecution};

pub(super) async fn execute(
    executor: &ToolExecutor,
    invocation: &ToolInvocation,
    session_id: i64,
) -> Result<ToolInvocationExecution, ToolError> {
    let input = crate::part::ContentReadToolInput::parse_input(invocation.input.clone().into())
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let content = input
        .resource_id
        .parse::<agena_domain::ContentId>()
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let session =
        SessionId::try_from(session_id).map_err(|error| ToolError::invalid_input_error(&error))?;
    let reader = executor
        .content_reader
        .as_ref()
        .ok_or_else(|| ToolError::plugin("content service is unavailable"))?;
    let watch = reader
        .watch_content(session, content)
        .await
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let head = watch
        .head()
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let epoch = input
        .epoch
        .as_deref()
        .map(str::parse::<agena_domain::ContentEpoch>)
        .transpose()
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    if epoch.is_none() && (input.after != 0 || input.offset != 0) {
        return Err(ToolError::invalid_input(
            "epoch is required to resume a content read",
        ));
    }
    if epoch.is_some_and(|epoch| epoch != head.epoch) {
        return Err(ToolError::invalid_input(
            "content epoch changed; restart from the source head",
        ));
    }
    let budget = input.max_bytes.unwrap_or(4096);
    let payload = if head.kind == ContentKind::Text {
        if input.offset != 0 {
            return Err(ToolError::invalid_input(
                "text source uses the after byte cursor; offset must be zero",
            ));
        }
        let after = epoch.map(|epoch| TextCursor {
            content,
            epoch,
            offset: input.after,
        });
        let page = reader
            .text_range(session, content, after, budget)
            .await
            .map_err(|error| ToolError::invalid_input_error(&error))?;
        let valid_len = match std::str::from_utf8(&page.bytes) {
            Ok(_) => page.bytes.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => {
                return Err(ToolError::invalid_input(
                    "text cursor splits a UTF-8 character",
                ));
            }
        };
        let text = std::str::from_utf8(&page.bytes[..valid_len]).expect("validated text prefix");
        let next = page.start + valid_len as u64;
        serde_json::json!({
            "source": page.stream,
            "text": text,
            "next_position": {"epoch": page.stream.epoch, "after": next, "offset": 0},
            "has_more": page.has_more || valid_len < page.bytes.len(),
        })
    } else {
        // One immutable event is enough to fill a model page. A large log or
        // document event resumes within the same event using a byte offset.
        let after = epoch.map(|epoch| EventCursor {
            content,
            epoch,
            sequence: input.after,
        });
        let page = reader
            .event_page(session, content, after, 1)
            .await
            .map_err(|error| ToolError::invalid_input_error(&error))?;
        if page.records.is_empty() && input.offset != 0 {
            return Err(ToolError::invalid_input(
                "event offset has no successor event",
            ));
        }
        let mut fragments = Vec::new();
        let mut next = input.after;
        let mut offset = 0;
        let mut partial = false;
        if let Some(event) = page.records.first() {
            let text = match &event.payload {
                agena_storage::kernel::EventPayload::Log { text, .. } => text.clone(),
                agena_storage::kernel::EventPayload::Mixed { input, .. } => match &**input {
                    agena_domain::ContentInput::Text { text }
                    | agena_domain::ContentInput::Log { text, .. } => text.clone(),
                    _ => serde_json::to_string(input)
                        .map_err(|error| ToolError::invalid_input_error(&error))?,
                },
                _ => serde_json::to_string(&event.payload)
                    .map_err(|error| ToolError::invalid_input_error(&error))?,
            };
            if input.offset > text.len() || !text.is_char_boundary(input.offset) {
                return Err(ToolError::invalid_input("invalid event text offset"));
            }
            let mut end = input.offset.saturating_add(budget).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            fragments.push(serde_json::json!({
                "sequence": event.sequence,
                "timestamp_ms": event.timestamp_ms,
                "offset": input.offset,
                "text": &text[input.offset..end],
                "encoding": match &event.payload {
                    agena_storage::kernel::EventPayload::Log { .. } => "text",
                    agena_storage::kernel::EventPayload::Mixed { input, .. } if matches!(&**input, agena_domain::ContentInput::Text { .. } | agena_domain::ContentInput::Log { .. }) => "text",
                    _ => "json",
                },
                "channel": match &event.payload {
                    agena_storage::kernel::EventPayload::Log { channel, .. } => Some(channel),
                    agena_storage::kernel::EventPayload::Mixed { input, .. } => match &**input {
                        agena_domain::ContentInput::Log { stream, .. } => Some(stream),
                        _ => None,
                    },
                    _ => None,
                },
            }));
            partial = end < text.len();
            if partial {
                offset = end;
            } else {
                next = event.sequence;
            }
        }
        serde_json::json!({
            "source": page.stream,
            "fragments": fragments,
            "next_position": {"epoch": page.stream.epoch, "after": next, "offset": offset},
            "has_more": partial || page.has_more,
        })
    };
    // Reauthorize after I/O so a concurrently removed membership cannot leak
    // an otherwise valid source through this tool result.
    reader
        .watch_content(session, content)
        .await
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let output = ToolOutput::from_json_payload(Some(&payload)).map_err(ToolError::invalid_input)?;
    Ok(ToolInvocationExecution::new(
        output,
        ToolExecutionView::simple("Read content", "Source page", ""),
    ))
}
