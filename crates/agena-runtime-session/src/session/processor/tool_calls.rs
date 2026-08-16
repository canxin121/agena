use super::{
    AppError, BTreeMap, OperationPart, PendingProviderNativeToolCall, PendingToolCall,
    SessionProcessor, SessionRunRequest, StructuredObject, TimeRange, ToolInvocation, Utc,
    merge_provider_native_tool_invocation, parse_tool_invocation_lossy,
    placeholder_tool_invocation, provider_native_tool_execution_title,
    tool_api_definition_identity, tool_execution_title, tool_execution_title_for_invocation,
};
use crate::session::store::{
    OPERATION_ID_METADATA_KEY, tool_call_from_operation, typed_content_from_value,
    typed_content_to_value,
};
use agena_provider::merge_provider_metadata;
use agena_runtime_contracts::part_content::{TypedContent, operation_from_tool_call};
use agena_storage::store::{Part, PartRole, PartState, PartVisibility};

impl SessionProcessor {
    pub(crate) async fn ensure_pending_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
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
            let content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))?;
            parts.push(placeholder_part(
                part_id,
                run_id,
                start.timestamp_millis(),
                content,
            ));
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
            let part = parts
                .iter_mut()
                .find(|part| part.part_id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from turn accumulator: {part_id}"
                    ))
                })?;
            let mut content = typed_content_from_value(&part.kind, &part.content)?;
            if let TypedContent::ToolCall(tool_call) = &mut content {
                let mut operation = operation_from_tool_call(tool_call);
                if operation
                    .metadata
                    .get(OPERATION_ID_METADATA_KEY)
                    .and_then(serde_json::Value::as_str)
                    != Some(operation_id.as_str())
                {
                    operation.metadata.insert(
                        OPERATION_ID_METADATA_KEY.to_owned(),
                        serde_json::Value::String(operation_id.clone()),
                    );
                    *tool_call = tool_call_from_operation(&operation);
                    part.content = typed_content_to_value(&content)?;
                }
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
            let part = parts
                .iter_mut()
                .find(|part| part.part_id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from turn accumulator: {part_id}"
                    ))
                })?;
            let mut content = typed_content_from_value(&part.kind, &part.content)?;
            if let TypedContent::ToolCall(tool_call) = &mut content {
                let mut operation = operation_from_tool_call(tool_call);
                if operation.invocation.name != name
                    || operation.title != tool_execution_title(Some(name))
                {
                    operation.invocation.name = name.to_owned();
                    operation.set_title(tool_execution_title(Some(name)));
                    *tool_call = tool_call_from_operation(&operation);
                    part.content = typed_content_to_value(&content)?;
                }
            }
        }

        Ok(())
    }

    pub(crate) async fn finalize_pending_tool_calls(
        &self,
        run: &mut SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for mut pending in pending_calls.into_values() {
            self.ensure_pending_tool_call_part(run, run_id, parts, &mut pending)
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

            let part = parts
                .iter_mut()
                .find(|part| part.part_id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from turn accumulator: {part_id}"
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
            // `ensure_pending_tool_call_part` records the provider's model-side
            // call id as soon as the stream exposes it. Rebuilding the final
            // invocation must carry that identity forward: Responses replays
            // correlate `function_call` and `function_call_output` by this
            // exact id. Dropping it here made persisted calls fall back to an
            // unrelated local sequence number on every follow-up request.
            if let Some(operation_id) = pending
                .id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                operation.metadata.insert(
                    OPERATION_ID_METADATA_KEY.to_owned(),
                    serde_json::Value::String(operation_id.to_owned()),
                );
            }
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
            part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))?;
        }

        Ok(())
    }

    pub(crate) async fn ensure_provider_native_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
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
            if let Some(operation_id) = &operation_id {
                operation.metadata.insert(
                    OPERATION_ID_METADATA_KEY.to_owned(),
                    serde_json::Value::String(operation_id.clone()),
                );
            }
            let content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))?;
            parts.push(placeholder_part(
                part_id,
                run_id,
                start.timestamp_millis(),
                content,
            ));
            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
        } else if let Some(part_id) = pending.part_id {
            let part = parts
                .iter_mut()
                .find(|part| part.part_id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "provider tool part missing from turn accumulator: {part_id}"
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
            if let Some(operation_id) = &operation_id {
                operation.metadata.insert(
                    OPERATION_ID_METADATA_KEY.to_owned(),
                    serde_json::Value::String(operation_id.clone()),
                );
            }
            part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))?;
            if part.state == PartState::Pending {
                part.state = PartState::InProgress;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn complete_provider_native_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
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
        if let Some(id) = id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            pending.id = Some(id);
        }
        let invocation =
            merge_provider_native_tool_invocation(pending.invocation.as_ref(), invocation);
        pending.invocation = Some(invocation.clone());
        let title = if title.trim().is_empty() && !pending.title.trim().is_empty() {
            pending.title.clone()
        } else {
            title
        };
        pending.title = title.clone();
        let raw = merge_provider_metadata(pending.raw.take(), raw);
        pending.raw = raw.clone();
        self.ensure_provider_native_tool_call_part(run, run_id, parts, &mut pending)
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
        let part = parts
            .iter_mut()
            .find(|part| part.part_id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "provider tool part missing from turn accumulator: {part_id}"
                ))
            })?;
        let mut operation = OperationPart::completed(
            pending.call_id.unwrap_or_default(),
            invocation.clone(),
            crate::part::OperationCompletion::new(
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
        if let Some(operation_id) = pending
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            operation.metadata.insert(
                OPERATION_ID_METADATA_KEY.to_owned(),
                serde_json::Value::String(operation_id.to_owned()),
            );
        }
        part.content = typed_content_to_value(&TypedContent::ToolCall(tool_call_from_operation(
            &operation,
        )))?;
        if part.state != PartState::Completed {
            part.state = PartState::Completed;
        }

        Ok(())
    }
}

/// Build an in-memory placeholder part for a tool call streamed during the
/// run. The id is negative until the call-side part is persisted (deferred
/// tool parts), at which point the accumulator remaps it onto the engine id.
fn placeholder_part(
    part_id: i64,
    run_id: i64,
    started_at_ms: i64,
    content: serde_json::Value,
) -> Part {
    Part {
        part_id,
        kind: "tool_call".to_owned(),
        role: PartRole::Assistant,
        state: PartState::InProgress,
        content,
        summary: None,
        visibility: PartVisibility::Both,
        rendered_markdown: None,
        parent_part_id: None,
        run_id: Some(run_id),
        origin_session_id: 0,
        revision: 0,
        started_at_ms,
        finished_at_ms: None,
        created_at_ms: started_at_ms,
        updated_at_ms: started_at_ms,
        provider_state: None,
    }
}
