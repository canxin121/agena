use super::*;
use agena_plugin_host::{
    PluginScopeKey,
    sdk::{ToolBatchCall, ToolBatchInput},
};

impl SessionManager {
    pub(super) async fn tool_batch_requested_end_turn(
        &self,
        session: SessionId,
        tools: &[ToolId],
    ) -> bool {
        let reader = &self.kernel().reader;
        for id in tools {
            let Ok(tool) = read_with_busy_retry(session, "tool_end_turn", || {
                reader.tool(session, *id)
            })
            .await
            else {
                continue;
            };
            if tool
                .result
                .as_ref()
                .and_then(|result| result.get("metadata"))
                .and_then(|metadata| metadata.get(agena_plugin_host::sdk::TOOL_END_TURN))
                .and_then(serde_json::Value::as_str)
                == Some("true")
            {
                return true;
            }
        }
        false
    }

    pub(super) async fn after_tool_batch(
        &self,
        attempt: &AttemptHandle,
        control: &RunControl,
        state: &ExecutionState,
        executor: &ToolExecutor,
        tools: &[ToolId],
    ) -> Result<(), AppError> {
        let session = control.session;
        let sid = session.get();
        let reader = &self.kernel().reader;
        let host = executor.plugin_manager();
        let scope = PluginScopeKey::session(sid);
        // Keep room for JSON escaping and the dialect's event fields in the
        // imported hook's 1 MiB input. Share the allowance across the batch.
        let allowance = 640 * 1024 / tools.len().max(1);
        let mut calls = Vec::with_capacity(tools.len());
        for id in tools {
            if control.cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let tool = match read_with_busy_retry(session, "tool_batch_result", || {
                reader.tool(session, *id)
            })
            .await
            {
                Ok(tool) => tool,
                Err(error) => {
                    tracing::warn!(%error, tool_id = id.get(), "could not read the committed tool batch");
                    return Ok(());
                }
            };
            let invocation: Option<ToolInvocation> = tool
                .result
                .as_ref()
                .and_then(|result| result.get("invocation"))
                .and_then(|invocation| serde_json::from_value(invocation.clone()).ok());
            let (name, input) = if let Some(invocation) = &invocation {
                (invocation.name.clone(), invocation.input.clone().into())
            } else {
                (tool.call.name.clone(), tool.call.input.clone())
            };
            let key = host
                .lookup_tool_for_name_scope(&name, Some(&scope))
                .map(|tool| tool.tool_key().clone());
            let model_output = match crate::session::kernel::input::tool_batch_response(
                reader,
                session,
                &tool,
                invocation.as_ref(),
                executor,
            )
            .await
            {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(%error, tool_id = id.get(), "could not project a tool result for the batch hook");
                    "[Tool result unavailable.]".into()
                }
            };
            calls.push(bounded_call(
                ToolBatchCall {
                    call_id: id.get(),
                    name,
                    tool: key,
                    input,
                    model_output,
                },
                allowance,
            ));
        }
        let block = host
            .dispatch_tool_batch_after(
                ToolBatchInput {
                    session_id: sid,
                    calls,
                },
                control.cancel.clone(),
                |plugin| executor.plugin_is_enabled(plugin),
            )
            .await;
        if control.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        self.flush_hook_runs(attempt, session, state).await?;
        if let Some(reason) = block {
            return Err(AppError::Config(reason));
        }
        Ok(())
    }
}

fn bounded_call(mut call: ToolBatchCall, allowance: usize) -> ToolBatchCall {
    use super::super::results::bounded_json;

    if bounded_json(&call, allowance).is_ok() {
        return call;
    }
    // Give arguments half the allowance when both fields are large. Keep
    // every call's identity when a single argument or result exceeds it.
    if bounded_json(&call.input, allowance / 2).is_err() {
        call.input = json!({"agena_input_truncated": true});
    }
    if bounded_json(&call, allowance).is_err() {
        const MARKER: &str = "\n[Tool batch result preview limited.]";
        let keep = allowance.saturating_div(12).saturating_sub(MARKER.len());
        call.model_output
            .truncate(call.model_output.floor_char_boundary(keep));
        call.model_output.push_str(MARKER);
    }
    call
}
