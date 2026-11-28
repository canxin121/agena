//! Read durable facts for a session lifecycle callback with bounded previews.

use agena_domain::{AuthorKind, ToolApiFunction};
use agena_plugin_host::sdk::{PluginKey, host_api::SessionHistoryEvent};

use super::*;

const MAX_PREVIEW_BYTES: usize = 16 * 1024;

impl SessionManager {
    pub async fn visit_ending_session_history(
        &self,
        id: i64,
        plugin: &PluginKey,
        visit: &mut (dyn FnMut(SessionHistoryEvent) + Send),
    ) -> Result<(), AppError> {
        let session = session_id(id)?;
        let state = self.state_for_session(session).await?;
        let context = self.execution_context_with_state(session, &state).await?;
        let executor = state.executor.for_session_resource_context(&context);
        if context.global
            || context.owner.is_some()
            || context.channel_authority.is_some()
            || context.relation_kind.is_subagent()
            || context
                .conversation
                .as_ref()
                .is_some_and(|view| view.is_read_only())
            || !executor.plugin_is_enabled(plugin)
        {
            return Ok(());
        }
        let reader = &self.kernel().reader;
        let snapshot = reader
            .history_snapshot(session)
            .await
            .map_err(kernel_error)?;
        let mut cursor = None;
        loop {
            let page = snapshot.page(cursor, 128).await.map_err(kernel_error)?;
            for record in &page.entries {
                let entry = &record.entry;
                if entry.origin_session_id != session {
                    continue;
                }
                let event = match &entry.payload {
                    EntryPayload::ContextCleared { .. } => SessionHistoryEvent::ContextCleared,
                    EntryPayload::User {
                        document, author, ..
                    } if entry.execution_started
                        && author.as_ref().is_none_or(|author| {
                            matches!(author.kind, AuthorKind::InstanceOperator)
                        }) =>
                    {
                        let mut text = document.text();
                        super::results_text::truncate(&mut text, MAX_PREVIEW_BYTES);
                        SessionHistoryEvent::User(text)
                    }
                    EntryPayload::Text { content_id, .. }
                        if entry.compaction_phase.is_none()
                            && entry.content_state == Some(CaptureState::Complete) =>
                    {
                        let text = self
                            .assistant_text(session, *content_id, MAX_PREVIEW_BYTES)
                            .await?;
                        if text.trim().is_empty() {
                            continue;
                        }
                        SessionHistoryEvent::Assistant(text)
                    }
                    EntryPayload::Tool { tool_id } => {
                        let tool = reader.tool(session, *tool_id).await.map_err(kernel_error)?;
                        let Some(event) = completed_tool(&tool) else {
                            continue;
                        };
                        event
                    }
                    _ => continue,
                };
                visit(event);
            }
            if !page.has_more {
                break;
            }
            cursor = page.next;
        }
        Ok(())
    }
}

fn completed_tool(tool: &ToolView) -> Option<SessionHistoryEvent> {
    if !matches!(tool.phase, ToolPhase::Succeeded | ToolPhase::Failed) {
        return None;
    }
    let result = tool.result.as_ref()?;
    let name = result
        .pointer("/invocation/name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            if tool.call.name == ToolApiFunction::Call.function_name() {
                tool.call
                    .input
                    .get("tool")
                    .and_then(serde_json::Value::as_str)
            } else {
                None
            }
        })
        .unwrap_or(&tool.call.name);
    let output = [
        "/hook_model_output",
        "/tool_api_response/output_text",
        "/error/user/fallback",
        "/raw_output/payload/output",
        "/raw_output/payload/text",
        "/summary",
    ]
    .into_iter()
    .find_map(|field| result.pointer(field).and_then(serde_json::Value::as_str))
    .unwrap_or_default();
    let output =
        output[..output.floor_char_boundary(output.len().min(MAX_PREVIEW_BYTES))].to_owned();
    Some(SessionHistoryEvent::ToolCompleted {
        name: name.to_owned(),
        output,
        failed: tool.phase == ToolPhase::Failed,
        user_declined: result
            .pointer("/failure/code")
            .and_then(serde_json::Value::as_str)
            == Some("permission.user_declined"),
    })
}
