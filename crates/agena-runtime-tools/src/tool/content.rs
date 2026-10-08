//! Explicit model reads use the same canonical resources and bounded pages.

use agena_domain::{ContentCursor, ContentTextPosition, ToolInvocation, ToolOutput};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolInvocationExecution};

pub(super) async fn execute(
    executor: &ToolExecutor,
    invocation: &ToolInvocation,
    session_id: i64,
) -> Result<ToolInvocationExecution, ToolError> {
    let input = crate::part::ContentReadToolInput::parse_input(invocation.input.clone().into())
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let id = input
        .resource_id
        .parse::<agena_domain::ContentId>()
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let store = executor
        .content_store
        .as_ref()
        .ok_or_else(|| ToolError::plugin("content service is unavailable"))?;
    let resource = store
        .contents()
        .describe(id)
        .await
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    let view = store
        .load_part_ids(session_id, &[resource.part_id])
        .await
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    if !view
        .parts
        .iter()
        .any(|part| part.visibility.visible_to_ai() && part.references_content(&resource))
    {
        return Err(ToolError::invalid_input(
            "content resource is not available in this session",
        ));
    }
    let position = match input.epoch {
        Some(epoch) => Some(ContentTextPosition {
            after: ContentCursor {
                epoch: epoch
                    .parse()
                    .map_err(|error| ToolError::invalid_input_error(&error))?,
                sequence: input.after,
            },
            offset: input.offset,
        }),
        None if input.after == 0 && input.offset == 0 => None,
        None => {
            return Err(ToolError::invalid_input(
                "epoch is required to resume a content read",
            ));
        }
    };
    let page = store
        .contents()
        .read_text_page(id, position, input.max_bytes.unwrap_or(4096))
        .await
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    // Reads may await file I/O. A removal during that interval must not expose
    // content whose session membership has already been revoked.
    let view = store
        .load_part_ids(session_id, &[resource.part_id])
        .await
        .map_err(|error| ToolError::invalid_input_error(&error))?;
    if !view
        .parts
        .iter()
        .any(|part| part.visibility.visible_to_ai() && part.references_content(&resource))
    {
        return Err(ToolError::invalid_input(
            "content resource is not available in this session",
        ));
    }
    let payload =
        serde_json::to_value(page).map_err(|error| ToolError::invalid_input_error(&error))?;
    let output = ToolOutput::from_json_payload(Some(&payload)).map_err(ToolError::invalid_input)?;
    Ok(ToolInvocationExecution::new(
        output,
        ToolExecutionView::simple("Read content", "Resource page", ""),
    ))
}
