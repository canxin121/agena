use super::{
    AppError, BTreeMap, ExecutionStatus, Message, MessagePart, MessageStatus, OperationPart,
    PartContent, PendingNativeToolCall, PendingToolCall, RunBuffer, SessionProcessor,
    SessionRunRequest, StructuredObject, TimeRange, ToolCallId, ToolInvocation, Utc,
    native_tool_execution_title, parse_tool_invocation_lossy, placeholder_tool_invocation,
    tool_definition_identity_from_model_name, tool_execution_title,
};

impl SessionProcessor {
    pub(crate) async fn ensure_pending_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        run_buffer: &mut RunBuffer,
        pending: &mut PendingToolCall,
    ) -> Result<(), AppError> {
        let mut should_emit = false;
        if pending.part_id.is_none() {
            let part_id = run.part_ids.reserve().await?;
            let call_id = run.next_call_id;
            run.next_call_id += 1;
            let start = Utc::now();
            let invocation = placeholder_tool_invocation(
                pending.name.as_deref(),
                run.completion.tools.as_slice(),
            );
            let mut operation = OperationPart::pending(
                call_id,
                invocation,
                tool_execution_title(pending.name.as_deref()),
                TimeRange {
                    start_ms: start.timestamp_millis(),
                    end_ms: None,
                },
            );
            if let Some(identity) = pending.name.as_deref().and_then(|name| {
                tool_definition_identity_from_model_name(name, run.completion.tools.as_slice())
            }) {
                operation.set_advertised_tool_identity(identity);
            }

            let mut part = MessagePart::from_content(
                part_id,
                assistant.id,
                start,
                ExecutionStatus::Pending,
                PartContent::Operation(operation),
            );
            part.part_index = assistant.parts.len() as i32;
            assistant.parts.push(part);
            if assistant.state == MessageStatus::Pending {
                assistant
                    .transition_state(MessageStatus::InProgress)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }

            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
            should_emit = true;

            // Mirror into RunBuffer with a stable history-side call id.
            // Prefer the provider-supplied id when present; otherwise fall
            // back to a synthetic one derived from the integer call_id so it
            // remains stable for the lifetime of this run.
            let history_call_id = match pending.id.as_deref() {
                Some(id) if !id.trim().is_empty() => ToolCallId::new(id),
                _ => ToolCallId::new(format!("call_{call_id}")),
            };
            run_buffer
                .start_tool_call(history_call_id.clone())
                .map_err(|err| AppError::Internal(err.to_string()))?;
            if let Some(name) = pending.name.as_deref() {
                run_buffer
                    .name_tool_call(&history_call_id, name)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
            pending.history_call_id = Some(history_call_id);
        } else if pending.history_call_id.is_some() {
            if let Some(provider_call_id) = pending.id.as_deref().filter(|id| !id.trim().is_empty())
            {
                let next_history_call_id = ToolCallId::new(provider_call_id);
                let should_replace = pending
                    .history_call_id
                    .as_ref()
                    .is_some_and(|history_call_id| history_call_id != &next_history_call_id);
                if should_replace {
                    let current_history_call_id =
                        pending.history_call_id.clone().expect("checked above");
                    run_buffer
                        .replace_tool_call_id(
                            &current_history_call_id,
                            next_history_call_id.clone(),
                        )
                        .map_err(|err| AppError::Internal(err.to_string()))?;
                    pending.history_call_id = Some(next_history_call_id);
                }
            }

            if let Some(history_call_id) = pending.history_call_id.as_ref()
                && let Some(name) = pending.name.as_deref()
            {
                // A second name fragment can arrive after the part already exists.
                // Re-set the name; RunBuffer accepts repeated assignment.
                run_buffer
                    .name_tool_call(history_call_id, name)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
        }

        if let (Some(part_id), Some(operation_id)) = (
            pending.part_id,
            pending
                .id
                .as_ref()
                .filter(|id| !id.trim().is_empty())
                .cloned(),
        ) {
            let part = assistant
                .parts
                .iter_mut()
                .find(|part| part.id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from assistant snapshot: {part_id}"
                    ))
                })?;
            if part.operation_id.as_deref() != Some(operation_id.as_str()) {
                part.operation_id = Some(operation_id);
                should_emit = true;
            }
        }

        if should_emit && let Some(part_id) = pending.part_id {
            self.checkpoint_part(run, assistant, part_id).await?;
        }

        Ok(())
    }

    pub(crate) async fn finalize_pending_tool_calls(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        run_buffer: &mut RunBuffer,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for mut pending in pending_calls.into_values() {
            self.ensure_pending_tool_call_part(run, assistant, run_buffer, &mut pending)
                .await?;

            let tool_name = pending.name.unwrap_or_else(|| "unknown".to_string());
            let invocation = parse_tool_invocation_lossy(
                run.session_id,
                tool_name.as_str(),
                pending.arguments_json.as_str(),
                run.completion.tools.as_slice(),
            );
            let Some(part_id) = pending.part_id else {
                continue;
            };
            let call_id = pending.call_id.unwrap_or(0);

            let part = assistant
                .parts
                .iter_mut()
                .find(|part| part.id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from assistant snapshot: {part_id}"
                    ))
                })?;
            let mut operation = OperationPart::pending(
                call_id,
                invocation,
                tool_execution_title(Some(tool_name.as_str())),
                TimeRange {
                    start_ms: pending.started_at_ms.unwrap_or_default(),
                    end_ms: None,
                },
            );
            if let Some(identity) = tool_definition_identity_from_model_name(
                tool_name.as_str(),
                run.completion.tools.as_slice(),
            ) {
                operation.set_advertised_tool_identity(identity);
            }
            part.set_content(PartContent::Operation(operation));

            // Re-assert name on RunBuffer (final, authoritative). The
            // accumulated `arguments_json` was already streamed in chunks via
            // `append_tool_arguments`; we don't repeat it here.
            if let Some(history_call_id) = pending.history_call_id.as_ref() {
                run_buffer
                    .name_tool_call(history_call_id, tool_name.as_str())
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }

            self.checkpoint_part(run, assistant, part_id).await?;
        }

        Ok(())
    }

    pub(crate) async fn ensure_native_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        pending: &mut PendingNativeToolCall,
    ) -> Result<(), AppError> {
        let invocation = pending
            .invocation
            .clone()
            .unwrap_or_else(|| ToolInvocation::new("native_tool", StructuredObject::default()));
        let operation_title = native_tool_execution_title(
            pending.title.as_str(),
            invocation.name.as_str(),
            &invocation.input,
        );
        let raw = pending.raw.clone();
        let operation_id = pending
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let now = Utc::now();
        let mut should_emit = false;

        if pending.part_id.is_none() {
            let part_id = run.part_ids.reserve().await?;
            let call_id = run.next_call_id;
            run.next_call_id += 1;
            let start = now;
            let mut operation = OperationPart::pending(
                call_id,
                invocation,
                operation_title,
                TimeRange {
                    start_ms: start.timestamp_millis(),
                    end_ms: None,
                },
            );
            operation.set_provider_native_only(true);
            operation.raw = raw.clone();
            operation.result.raw = raw;

            let mut part = MessagePart::from_content(
                part_id,
                assistant.id,
                start,
                ExecutionStatus::InProgress,
                PartContent::Operation(operation),
            );
            part.part_index = assistant.parts.len() as i32;
            part.operation_id = operation_id.clone();
            assistant.parts.push(part);
            if assistant.state == MessageStatus::Pending {
                assistant
                    .transition_state(MessageStatus::InProgress)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }

            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
            should_emit = true;
        } else if let Some(part_id) = pending.part_id {
            let part = assistant
                .parts
                .iter_mut()
                .find(|part| part.id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "native tool part missing from assistant snapshot: {part_id}"
                    ))
                })?;
            let started_at_ms = pending
                .started_at_ms
                .unwrap_or_else(|| now.timestamp_millis());
            let mut operation = OperationPart::pending(
                pending.call_id.unwrap_or_default(),
                invocation,
                operation_title,
                TimeRange {
                    start_ms: started_at_ms,
                    end_ms: None,
                },
            );
            operation.set_provider_native_only(true);
            operation.raw = raw.clone();
            operation.result.raw = raw;
            part.set_content(PartContent::Operation(operation));
            if part.status == ExecutionStatus::Pending {
                part.transition_status(ExecutionStatus::InProgress)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
            if part.operation_id != operation_id {
                part.operation_id = operation_id.clone();
            }
            should_emit = true;
        }

        if should_emit && let Some(part_id) = pending.part_id {
            self.checkpoint_part(run, assistant, part_id).await?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn complete_native_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        mut pending: PendingNativeToolCall,
        id: Option<String>,
        invocation: ToolInvocation,
        title: String,
        output_text: String,
        blocks: Vec<crate::message::OperationBlock>,
        details: crate::message::ToolOutput,
        raw: Option<serde_json::Value>,
    ) -> Result<(), AppError> {
        pending.id = id;
        pending.invocation = Some(invocation.clone());
        pending.title = title.clone();
        pending.raw = raw.clone();
        self.ensure_native_tool_call_part(run, assistant, &mut pending)
            .await?;

        let artifact_key = pending
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("native-tool-{}", pending.call_id.unwrap_or_default()));
        let blocks = self
            .persist_native_tool_media(run.session_id, artifact_key.as_str(), blocks)
            .await;

        let Some(part_id) = pending.part_id else {
            return Ok(());
        };
        let part = assistant
            .parts
            .iter_mut()
            .find(|part| part.id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "native tool part missing from assistant snapshot: {part_id}"
                ))
            })?;
        let mut operation = OperationPart::completed(
            pending.call_id.unwrap_or_default(),
            invocation.clone(),
            output_text,
            blocks,
            Vec::new(),
            details,
            TimeRange {
                start_ms: pending
                    .started_at_ms
                    .unwrap_or_else(|| Utc::now().timestamp_millis()),
                end_ms: Some(Utc::now().timestamp_millis()),
            },
        );
        if !title.trim().is_empty() {
            operation.set_title(title);
        }
        operation.set_provider_native_only(true);
        operation.raw = raw.clone();
        operation.result.raw = raw;
        part.set_content(PartContent::Operation(operation));
        if part.status != ExecutionStatus::Completed {
            part.transition_status(ExecutionStatus::Completed)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        if let Some(operation_id) = pending
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            part.operation_id = Some(operation_id.to_owned());
        }

        self.checkpoint_part(run, assistant, part_id).await?;
        Ok(())
    }
}
