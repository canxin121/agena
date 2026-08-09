use super::{
    AppError, BTreeMap, ExecutionStatus, Message, MessagePart, OperationPart, PartContent,
    PendingProviderNativeToolCall, PendingToolCall, SessionProcessor, SessionRunRequest,
    StructuredObject, TimeRange, ToolInvocation, Utc, parse_tool_invocation_lossy,
    placeholder_tool_invocation, provider_native_tool_execution_title,
    tool_api_definition_identity, tool_execution_title, tool_execution_title_for_invocation,
};

impl SessionProcessor {
    pub(crate) async fn ensure_pending_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        pending: &mut PendingToolCall,
    ) -> Result<(), AppError> {
        if pending.part_id.is_none() {
            let part_id = run.part_ids.reserve().await?;
            let call_id = run.next_call_id;
            run.next_call_id += 1;
            let start = Utc::now();
            let invocation = placeholder_tool_invocation(
                pending.name.as_deref(),
                run.completion.tool_api_functions.as_slice(),
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
                tool_api_definition_identity(name, run.completion.tool_api_functions.as_slice())
            }) {
                operation.set_advertised_tool_identity(identity);
            }

            let mut part = MessagePart::from_content(
                part_id,
                assistant.id,
                start,
                ExecutionStatus::Pending,
                PartContent::operation(operation),
            );
            part.part_index = assistant.parts.len() as i32;
            assistant.parts.push(part);
            if assistant.state == ExecutionStatus::Pending {
                assistant
                    .transition_state(ExecutionStatus::InProgress)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }

            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
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
            }
        }

        // Providers can stream a tool name after the placeholder operation was
        // created. Publish that identity immediately so the Activity title is
        // useful while arguments are still arriving; waiting for final
        // argument assembly made running activities look blank or generic.
        if let (Some(part_id), Some(name)) = (
            pending.part_id,
            pending
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty()),
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
            if let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                operation,
            ))) = part.content.as_mut()
                && (operation.invocation.name != name
                    || operation.title != tool_execution_title(Some(name)))
            {
                operation.invocation.name = name.to_owned();
                operation.set_title(tool_execution_title(Some(name)));
            }
        }

        Ok(())
    }

    pub(crate) async fn finalize_pending_tool_calls(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for mut pending in pending_calls.into_values() {
            self.ensure_pending_tool_call_part(run, assistant, &mut pending)
                .await?;

            // A tool call with no name fragment at all is a malformed provider
            // stream, not a nameless-but-executable tool. Fail the run with a
            // clear message instead of substituting a phantom "unknown" name
            // that would surface as a confusing "unknown Tool API function".
            let Some(tool_name) = pending
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                return Err(AppError::Provider(format!(
                    "provider stream ended a tool call in session {} without a function name",
                    run.session_id
                )));
            };
            let invocation = parse_tool_invocation_lossy(
                run.session_id,
                tool_name,
                pending.arguments_json.as_str(),
                run.completion.tool_api_functions.as_slice(),
            )?;
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
                invocation.clone(),
                tool_execution_title_for_invocation(&invocation),
                TimeRange {
                    start_ms: pending.started_at_ms.unwrap_or_default(),
                    end_ms: None,
                },
            );
            if invocation
                .tool_api_call
                .as_ref()
                .is_some_and(|call| call.function != agena_domain::ToolApiFunction::Call)
                && let Some(identity) = tool_api_definition_identity(
                    tool_name,
                    run.completion.tool_api_functions.as_slice(),
                )
            {
                operation.set_advertised_tool_identity(identity);
            }
            part.set_content(PartContent::operation(operation));
        }

        Ok(())
    }

    pub(crate) async fn ensure_provider_native_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        pending: &mut PendingProviderNativeToolCall,
    ) -> Result<(), AppError> {
        let invocation = pending.invocation.clone().unwrap_or_else(|| {
            ToolInvocation::new("provider_native_tool", StructuredObject::default())
        });
        let operation_title = provider_native_tool_execution_title(
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
            operation.set_provider_only(true);
            operation.raw = raw.clone();
            operation.result.raw = raw;

            let mut part = MessagePart::from_content(
                part_id,
                assistant.id,
                start,
                ExecutionStatus::InProgress,
                PartContent::operation(operation),
            );
            part.part_index = assistant.parts.len() as i32;
            part.operation_id = operation_id.clone();
            assistant.parts.push(part);
            if assistant.state == ExecutionStatus::Pending {
                assistant
                    .transition_state(ExecutionStatus::InProgress)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }

            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
        } else if let Some(part_id) = pending.part_id {
            let part = assistant
                .parts
                .iter_mut()
                .find(|part| part.id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "provider tool part missing from assistant snapshot: {part_id}"
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
            operation.set_provider_only(true);
            operation.raw = raw.clone();
            operation.result.raw = raw;
            part.set_content(PartContent::operation(operation));
            if part.status == ExecutionStatus::Pending {
                part.transition_status(ExecutionStatus::InProgress)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
            if part.operation_id != operation_id {
                part.operation_id = operation_id.clone();
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn complete_provider_native_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        mut pending: PendingProviderNativeToolCall,
        id: Option<String>,
        invocation: ToolInvocation,
        title: String,
        summary: String,
        output_text: String,
        blocks: Vec<agena_domain::ViewBlock>,
        details: agena_domain::ToolOutput,
        raw: Option<serde_json::Value>,
    ) -> Result<(), AppError> {
        pending.id = id;
        pending.invocation = Some(invocation.clone());
        pending.title = title.clone();
        pending.raw = raw.clone();
        self.ensure_provider_native_tool_call_part(run, assistant, &mut pending)
            .await?;

        let artifact_key = pending
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("provider-tool-{}", pending.call_id.unwrap_or_default()));
        let blocks = self
            .persist_provider_native_tool_media(run.session_id, artifact_key.as_str(), blocks)
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
                    "provider tool part missing from assistant snapshot: {part_id}"
                ))
            })?;
        let mut operation = OperationPart::completed(
            pending.call_id.unwrap_or_default(),
            invocation.clone(),
            crate::message::OperationCompletion::new(
                title,
                summary,
                output_text,
                blocks,
                Vec::new(),
                details,
            ),
            TimeRange {
                start_ms: pending
                    .started_at_ms
                    .unwrap_or_else(|| Utc::now().timestamp_millis()),
                end_ms: Some(Utc::now().timestamp_millis()),
            },
        );
        operation.set_provider_only(true);
        operation.raw = raw.clone();
        operation.result.raw = raw;
        part.set_content(PartContent::operation(operation));
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

        Ok(())
    }
}
