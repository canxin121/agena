//! Membership-scoped, bounded transcript projections shared by every transport.
use crate::{
    Application, ApplicationError,
    pagination::{decode_cursor, encode_cursor},
};
use agena_api::{
    live::{RunSummaryResource, SessionPartsResource, ToolDetailSection},
    part::PartResource,
    queries::{ReadPartsParams, ReadRunsParams},
};
use agena_storage::kernel::{
    CaptureState, EntryPayload, EntryView, HistoryCursor, KernelReader, OwnedToolCursor, SessionId,
    ToolPhase,
};

/// One bounded, ascending page of owned tool facts. Its opaque continuation
/// binds the session, live membership, and control version for the whole read.
#[derive(Debug)]
pub struct OwnedToolPartsPage {
    pub parts: Vec<PartResource>,
    pub control_version: i64,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

pub(crate) fn decode_part_cursor(
    session_id: i64,
    cursor: Option<&str>,
) -> Result<Option<HistoryCursor>, ApplicationError> {
    let cursor = cursor.map(decode_cursor::<HistoryCursor>).transpose()?;
    if cursor.is_some_and(|cursor| cursor.session.get() != session_id) {
        return Err(ApplicationError::bad_request(
            "The cursor belongs to a different session.",
        ));
    }
    Ok(cursor)
}
pub(crate) fn encode_part_cursor(
    _session_id: i64,
    cursor: Option<HistoryCursor>,
) -> Result<Option<String>, ApplicationError> {
    cursor.map(|cursor| encode_cursor(&cursor)).transpose()
}

/// Synthesize the user-visible failure part from the failed round that ended an
/// execution. Failure is a read-time projection: legacy transcripts never
/// persisted this part, and no schema change is required for them to render.
pub(crate) async fn append_execution_failures(
    reader: &KernelReader,
    entries: &[EntryView],
    parts: &mut Vec<PartResource>,
) -> Result<(), ApplicationError> {
    use agena_storage::kernel::ExecutionPhase;

    // A bounded history window often contains only the newest entries of a
    // long run. The boundary entry is then absent from `parts`, so recover the
    // failed run identity from every loaded entry of the window.
    let mut candidates = Vec::new();
    let mut seen_runs = std::collections::HashSet::new();
    for entry in entries {
        if entry.execution_phase != ExecutionPhase::Failed {
            continue;
        }
        let Some(run_id) = entry.response_entry_id else {
            continue;
        };
        if !seen_runs.insert(run_id) {
            continue;
        }
        // A forked session reads entries owned by its ancestor. The failed
        // attempt therefore belongs to the entry owner, not the reader.
        candidates.push((
            run_id.get(),
            entry.execution_id,
            entry.generation,
            entry.origin_session_id,
            entry.created_at_ms,
            entry.message_id.get(),
        ));
    }
    let mut failures = Vec::new();
    for (run_id, execution, generation, owner, candidate_created_at_ms, message_id) in candidates {
        if parts.iter().any(|existing| {
            existing.kind == "error"
                && existing.run_id == Some(run_id)
                && existing
                    .content
                    .get("projection_error")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
        }) {
            continue;
        }
        let round = reader
            .failed_round(owner, execution, generation)
            .await
            .map_err(crate::kernel_error)?;
        // A failed attempt does not always leave a failed provider round:
        // manager-level stops (repeat guard, continuation limit, prompt
        // budget) finish after a succeeded round. Keep the run's error part
        // visible even when only the attempt phase records the failure.
        let (content, source_id, created_at_ms) = match &*round {
            Some(round) => match round
                .provider_state
                .as_ref()
                .and_then(|state| state.get("failure"))
                .cloned()
            {
                Some(value) => match serde_json::from_value::<agena_failure::Failure>(value) {
                    Ok(failure) => (
                        failure_content(&failure)?,
                        round.id.get(),
                        // The error follows the content produced by the failed
                        // request, before any later attempt's response.
                        round.finished_at_ms.unwrap_or(round.created_at_ms),
                    ),
                    Err(error) => {
                        tracing::warn!(
                            round_id = round.id.get(),
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "decode provider round failure for transcript projection",
                                &error,
                            ),
                            "failed provider round could not be projected as a transcript error"
                        );
                        (manager_failure_content(), run_id, candidate_created_at_ms)
                    }
                },
                None => (manager_failure_content(), run_id, candidate_created_at_ms),
            },
            None => (manager_failure_content(), run_id, candidate_created_at_ms),
        };
        failures.push((run_id, source_id, created_at_ms, content, owner, message_id));
    }

    let mut used_ids = parts
        .iter()
        .map(|part| part.part_id)
        .collect::<std::collections::HashSet<_>>();
    for (run_id, round_id, created_at_ms, content, owner, message_id) in failures {
        // Synthetic ids stay out of the durable entry range while remaining
        // decimal, which the Web message index requires. A pathological
        // durable-id collision simply advances to the next free value.
        let mut part_id = 2_000_000_000 + round_id;
        while !used_ids.insert(part_id) {
            part_id += 1;
        }
        parts.push(PartResource {
            part_id,
            kind: "error".into(),
            role: "assistant".into(),
            state: "failed".into(),
            run_state: Some("failed".into()),
            content,
            visibility: "both".into(),
            run_id: Some(run_id),
            message_id: Some(message_id),
            origin_session_id: owner.get(),
            revision: 1,
            started_at_ms: created_at_ms,
            finished_at_ms: Some(created_at_ms),
            created_at_ms,
            updated_at_ms: created_at_ms,
            ..PartResource::default()
        });
    }
    // Synthetic ids are identities, not timeline positions. This projection is
    // shared by transports, including clients which preserve server order.
    parts.sort_by_key(|part| (part.created_at_ms, part.run_id, part.part_id));
    Ok(())
}

fn failure_content(
    failure: &agena_failure::Failure,
) -> Result<serde_json::Value, ApplicationError> {
    let problem = agena_failure::UserProblem::from(failure);
    let category = serde_json::to_value(problem.category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned));
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "problem".to_owned(),
        serde_json::to_value(&problem).map_err(|error| ApplicationError::internal_error(&error))?,
    );
    Ok(agena_runtime_contracts::part_content::ErrorContent {
        category,
        message: problem.user.fallback.clone(),
        detail: None,
        retry: None,
        extra,
    }
    .as_value())
}

/// Older manager-level failures predate the durable failure notice, so their
/// exact reason is gone. The attempt phase still proves the run failed; show a
/// visible error instead of only the generic "Response failed" lifecycle row.
fn manager_failure_content() -> serde_json::Value {
    agena_runtime_contracts::part_content::ErrorContent {
        category: Some("internal".into()),
        message: "The response ended unexpectedly before a final answer was produced. \
                  This run was stopped outside the provider stream, so its exact failure \
                  reason was not recorded. Send a new message to continue."
            .into(),
        detail: None,
        retry: None,
        extra: Default::default(),
    }
    .as_value()
}

fn requested_sections(sections: &[ToolDetailSection]) -> &[ToolDetailSection] {
    if sections.is_empty() {
        &[ToolDetailSection::Presentation]
    } else {
        sections
    }
}

/// Only presentation decoding is recoverable here. Membership, storage and
/// permission failures must still propagate from their authoritative readers.
#[derive(Debug)]
struct ToolResultProjectionError {
    field: &'static str,
    source: serde_json::Error,
}

fn decode_tool_result_field<T: serde::de::DeserializeOwned>(
    field: &'static str,
    value: Option<serde_json::Value>,
) -> Result<Option<T>, ToolResultProjectionError> {
    value
        .filter(|value| !value.is_null())
        .map(serde_json::from_value)
        .transpose()
        .map_err(|source| ToolResultProjectionError { field, source })
}

fn tool_result_projection_error_content(
    tool_id: i64,
    result: Option<&serde_json::Value>,
    error: &ToolResultProjectionError,
) -> Result<serde_json::Value, ApplicationError> {
    tracing::warn!(
        tool_id,
        field = error.field,
        category = ?error.source.classify(),
        line = error.source.line(),
        column = error.source.column(),
        "stored tool result could not be displayed; preserving the record and other transcript parts"
    );
    let message = format!(
        "This stored tool result cannot be displayed because its {} field is invalid. \
         The original record has been preserved; other messages can still be opened.",
        error.field,
    );
    // A malformed output/metadata field must not erase a valid execution
    // failure. Keep its complete public problem, never the private raw result.
    let failure = result
        .and_then(|value| value.get("failure"))
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value::<agena_failure::Failure>(value.clone()).ok());
    let mut content = if let Some(failure) = failure {
        let mut content = failure_content(&failure)?;
        content["message"] = serde_json::Value::String(format!(
            "{}\n\n{message}",
            content["message"].as_str().unwrap_or_default(),
        ));
        content["detail"] = serde_json::Value::String(message);
        content
    } else {
        agena_runtime_contracts::part_content::ErrorContent {
            category: Some("internal".into()),
            message,
            ..Default::default()
        }
        .as_value()
    };
    content["projection_error"] = serde_json::Value::Bool(true);
    content["tool_call_id"] = serde_json::Value::from(tool_id);
    content["invalid_field"] = serde_json::Value::from(error.field);
    Ok(content)
}

pub(crate) async fn project_kernel_entry(
    reader: &KernelReader,
    session: SessionId,
    entry: &EntryView,
) -> Result<PartResource, ApplicationError> {
    let run_state = match entry.execution_phase {
        agena_storage::kernel::ExecutionPhase::Queued => "pending",
        agena_storage::kernel::ExecutionPhase::Running => "running",
        agena_storage::kernel::ExecutionPhase::Succeeded => "completed",
        agena_storage::kernel::ExecutionPhase::Failed => "failed",
        agena_storage::kernel::ExecutionPhase::Cancelled => "cancelled",
        agena_storage::kernel::ExecutionPhase::Interrupted => "failed",
    };
    // A running attempt usually has several finished reasoning/text streams.
    // Only an open capture is still producing tokens; treating every part of a
    // running attempt as running leaves completed thoughts spinning forever.
    let content_state = match entry.execution_phase {
        agena_storage::kernel::ExecutionPhase::Queued => "pending",
        agena_storage::kernel::ExecutionPhase::Running
            if entry.content_state == Some(CaptureState::Open) =>
        {
            "running"
        }
        _ => "completed",
    };
    let message_boundary = entry.response_entry_id == Some(entry.id);
    let mut part = PartResource {
        part_id: entry.id.get(),
        role: "assistant".into(),
        state: content_state.into(),
        run_state: message_boundary.then(|| run_state.to_owned()),
        visibility: "both".into(),
        run_id: entry.response_entry_id.map(|id| id.get()),
        message_id: Some(entry.message_id.get()),
        message_boundary,
        origin_session_id: entry.origin_session_id.get(),
        revision: entry.revision.get(),
        created_at_ms: entry.created_at_ms,
        started_at_ms: entry.created_at_ms,
        updated_at_ms: entry.created_at_ms.saturating_add(entry.revision.get()),
        finished_at_ms: entry
            .execution_phase
            .is_terminal()
            .then_some(entry.created_at_ms),
        ..PartResource::default()
    };
    match &entry.payload {
        EntryPayload::User { document } => {
            part.kind = "text".into();
            part.role = "user".into();
            part.run_id = Some(entry.id.get());
            part.message_boundary = true;
            part.run_state = Some(
                match entry.execution_phase {
                    agena_storage::kernel::ExecutionPhase::Queued if !entry.execution_started => {
                        "pending"
                    }
                    agena_storage::kernel::ExecutionPhase::Cancelled
                        if !entry.execution_started =>
                    {
                        "cancelled"
                    }
                    _ => "completed",
                }
                .into(),
            );
            part.content = serde_json::json!({"text":document.text(),"document":document,
                "execution_id":entry.execution_id,"run_kind":"user_send",
                "queue_only":!entry.execution_started});
        }
        EntryPayload::Text { content_id } | EntryPayload::Reasoning { content_id } => {
            let reasoning = matches!(&entry.payload, EntryPayload::Reasoning { .. });
            if let Some(phase) = entry.compaction_phase {
                let (outcome, state) = match phase {
                    agena_storage::kernel::ProviderRoundPhase::Running => ("running", "running"),
                    agena_storage::kernel::ProviderRoundPhase::Succeeded
                        if entry.compaction_committed =>
                    {
                        ("compacted", "completed")
                    }
                    agena_storage::kernel::ProviderRoundPhase::Succeeded => {
                        ("unchanged", "completed")
                    }
                    agena_storage::kernel::ProviderRoundPhase::Failed
                        if entry.execution_phase
                            == agena_storage::kernel::ExecutionPhase::Cancelled =>
                    {
                        ("cancelled", "cancelled")
                    }
                    agena_storage::kernel::ProviderRoundPhase::Failed => ("failed", "failed"),
                    agena_storage::kernel::ProviderRoundPhase::Interrupted => {
                        ("cancelled", "cancelled")
                    }
                };
                part.kind = "compaction".into();
                part.visibility = "user".into();
                part.state = state.into();
                part.content = serde_json::json!({"outcome":outcome,"run_kind":"compaction",
                    "resources":[agena_domain::ContentRef {
                        resource_id:*content_id, kind:agena_domain::ContentKind::Text }]});
                return Ok(part);
            }
            part.kind = if reasoning { "think" } else { "text" }.into();
            part.content = serde_json::json!({"text":"","resources":[agena_domain::ContentRef {
                resource_id:*content_id,kind:agena_domain::ContentKind::Text }],
                "execution_id":entry.execution_id});
        }
        EntryPayload::Compaction { summary, through } => {
            part.kind = "compaction".into();
            part.visibility = "user".into();
            part.content = serde_json::json!({"outcome":"compacted","resources":[agena_domain::ContentRef {
                resource_id:*summary, kind:agena_domain::ContentKind::Text }],"compaction_through":through,
                "run_kind":"compaction"});
        }
        EntryPayload::Notice { text } => {
            match agena_runtime_contracts::part_content::decode_failure_notice(text) {
                Some(Ok(failure)) => {
                    part.kind = "error".into();
                    part.state = "failed".into();
                    part.run_state = Some("failed".into());
                    part.content = failure_content(&failure)?;
                }
                Some(Err(error)) => {
                    tracing::warn!(
                        entry_id = entry.id.get(),
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode execution failure notice for transcript projection",
                            &error,
                        ),
                        "execution failure notice could not be decoded; keeping it as a notice"
                    );
                    part.kind = "text".into();
                    part.content = serde_json::json!({"text":text,"synthetic":true});
                }
                None => {
                    part.kind = "text".into();
                    part.content = serde_json::json!({"text":text,"synthetic":true});
                }
            }
        }
        EntryPayload::Tool { tool_id } => 'tool_projection: {
            let tool = reader
                .tool(session, *tool_id)
                .await
                .map_err(crate::kernel_error)?;
            let state = match tool.phase {
                ToolPhase::Preparing => agena_domain::ToolResultState::Pending,
                ToolPhase::Running => agena_domain::ToolResultState::Running,
                ToolPhase::Succeeded => agena_domain::ToolResultState::Completed,
                ToolPhase::Cancelled => agena_domain::ToolResultState::Cancelled,
                ToolPhase::Failed | ToolPhase::Interrupted => agena_domain::ToolResultState::Failed,
            };
            part.state = match tool.phase {
                ToolPhase::Preparing => "pending",
                ToolPhase::Running => "running",
                ToolPhase::Succeeded => "completed",
                ToolPhase::Cancelled => "cancelled",
                _ => "failed",
            }
            .into();
            part.revision = part.revision.max(tool.revision.get());
            // Tool lifetimes are independent of the enclosing execution. A
            // finished tool can belong to a still-running attempt, and an old
            // terminal record may have no recorded end. Preserve those facts
            // even if its result later fails the presentation projection.
            part.started_at_ms = tool.created_at_ms;
            part.finished_at_ms = match tool.phase {
                ToolPhase::Preparing | ToolPhase::Running => None,
                _ => tool.finished_at_ms,
            };
            part.updated_at_ms = part.finished_at_ms.unwrap_or(tool.created_at_ms);
            // Authoritative reads and interaction validation are deliberately
            // outside the recoverable result-presentation boundary below.
            let mut authorization = agena_domain::OperationAuthorization::default();
            let mut user_input = agena_domain::OperationUserInput::default();
            for id in &tool.interactions {
                let interaction = reader
                    .interaction(session, *id)
                    .await
                    .map_err(crate::kernel_error)?;
                project_interaction(session, &interaction, &mut authorization, &mut user_input)?;
            }
            let result = tool.result.as_ref();
            let decode = |key: &str| {
                result
                    .and_then(|value| value.get(key))
                    .filter(|value| !value.is_null())
                    .cloned()
            };
            let decoded = (|| -> Result<_, ToolResultProjectionError> {
                if result.is_some_and(|value| !value.is_null() && !value.is_object()) {
                    return Err(ToolResultProjectionError {
                        field: "result",
                        source: <serde_json::Error as serde::de::Error>::custom(
                            "stored tool result must be a JSON object",
                        ),
                    });
                }
                let output = if let Some(raw_output) = decode("raw_output") {
                    // Canonical output wins; legacy preview fields may be
                    // unrelated or obsolete and are not consumed in this case.
                    Some(raw_output)
                } else {
                    let response = decode_tool_result_field::<
                        serde_json::Map<String, serde_json::Value>,
                    >(
                        "tool_api_response", decode("tool_api_response")
                    )?;
                    let text = decode_tool_result_field::<String>(
                        "tool_api_response.output_text",
                        response.and_then(|mut response| response.remove("output_text")),
                    )?;
                    text.map(|text| serde_json::json!({"payload": {"text": text}}))
                };
                let output =
                    decode_tool_result_field::<agena_domain::RawOutput>("raw_output", output)?;
                let invocation = decode_tool_result_field::<agena_domain::ToolInvocation>(
                    "invocation",
                    decode("invocation").map(|mut value| {
                        if let Some(object) = value.as_object_mut() {
                            object.remove("tool_api_call");
                        }
                        value
                    }),
                )?;
                let tool_api_call = decode_tool_result_field::<agena_domain::ToolApiCall>(
                    "tool_api_call",
                    decode("tool_api_call"),
                )?;
                let error = decode_tool_result_field::<agena_failure::Failure>(
                    "failure",
                    decode("failure"),
                )?
                .map(|failure| agena_domain::OperationError { failure });
                let metadata = decode_tool_result_field::<
                    std::collections::BTreeMap<String, serde_json::Value>,
                >("metadata", decode("metadata"))?
                .unwrap_or_default();
                Ok((output, invocation, tool_api_call, error, metadata))
            })();
            let (output, invocation, tool_api_call, error, metadata) = match decoded {
                Ok(fields) => fields,
                Err(error) => {
                    // Keep the durable part identity, history ordering and
                    // execution state. This is a visible projection failure,
                    // not a fabricated successful tool or a rewritten record.
                    part.kind = "error".into();
                    part.state = "failed".into();
                    part.content =
                        tool_result_projection_error_content(tool.id.get(), result, &error)?;
                    break 'tool_projection;
                }
            };
            let tool_api_call = tool_api_call.or_else(|| {
                agena_domain::ToolApiFunction::from_function_name(&tool.call.name).and_then(
                    |function| {
                        agena_domain::StructuredObject::try_from(tool.call.input.clone())
                            .ok()
                            .map(|arguments| agena_domain::ToolApiCall {
                                function,
                                arguments,
                            })
                    },
                )
            });
            let (name, plugin, input) = if let Some(invocation) = invocation {
                (
                    invocation.name,
                    invocation.plugin_name,
                    serde_json::Value::from(invocation.input),
                )
            } else if tool_api_call
                .as_ref()
                .is_some_and(|call| call.function == agena_domain::ToolApiFunction::Call)
                && let Some(name) = tool
                    .call
                    .input
                    .get("tool")
                    .and_then(serde_json::Value::as_str)
            {
                let input = tool
                    .call
                    .input
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                // The durable protocol call keeps the model's exact wire input,
                // while every transcript consumer expects execution arguments.
                // Decode accepted strings here just as dispatch does, including
                // before a result has supplied a normalized invocation. Invalid
                // inputs stay intact for failure details instead of becoming {}.
                let input = agena_tool_api::normalize_tool_call_input(name, input.clone())
                    .map(serde_json::Value::Object)
                    .unwrap_or(input);
                (name.to_owned(), None, input)
            } else {
                (tool.call.name.clone(), None, tool.call.input.clone())
            };
            part.summary = result
                .and_then(|value| value.get("summary"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            part.kind = "tool_call".into();
            part.content = agena_runtime_contracts::part_content::ToolCallContent {
                name,
                plugin,
                input,
                tool_api_call,
                call_id: tool.id.get(),
                state,
                output,
                resources: tool
                    .outputs
                    .iter()
                    .map(|source| agena_domain::ContentRef {
                        resource_id: source.id,
                        kind: source.kind,
                    })
                    .collect(),
                authorization,
                user_input,
                error,
                metadata,
                lifecycle: agena_domain::TimeRange {
                    start_ms: part.started_at_ms,
                    end_ms: part.finished_at_ms,
                },
            }
            .as_value();
        }
        EntryPayload::Interaction { interaction_id } => {
            let interaction = reader
                .interaction(session, *interaction_id)
                .await
                .map_err(crate::kernel_error)?;
            part.kind = "interaction".into();
            part.visibility = "user".into();
            part.content = serde_json::to_value(&*interaction)
                .map_err(|error| ApplicationError::internal_error(&error))?;
        }
    }
    if part.kind != "tool_call" && part.kind != "error" {
        part.content["execution_id"] = serde_json::to_value(entry.execution_id)
            .map_err(|error| ApplicationError::internal_error(&error))?;
        part.content["execution_phase"] = serde_json::to_value(entry.execution_phase)
            .map_err(|error| ApplicationError::internal_error(&error))?;
        part.content["generation"] = serde_json::to_value(entry.generation)
            .map_err(|error| ApplicationError::internal_error(&error))?;
        if entry.execution_phase == agena_storage::kernel::ExecutionPhase::Interrupted {
            part.content["abort_reason"] = serde_json::json!("process_restart");
        }
    }
    // A boundary's execution state can advance without editing its content
    // entry. Give those snapshots increasing revisions too, so a refresh or
    // delayed live event cannot leave a claimed input pending in client caches.
    // Phase ranks are monotonic within one attempt; eight slots leave every
    // retry newer than its predecessor's terminal state.
    let phase_revision = match entry.execution_phase {
        agena_storage::kernel::ExecutionPhase::Queued => 0,
        agena_storage::kernel::ExecutionPhase::Running => 1,
        agena_storage::kernel::ExecutionPhase::Succeeded => 2,
        agena_storage::kernel::ExecutionPhase::Failed => 3,
        agena_storage::kernel::ExecutionPhase::Cancelled => 4,
        agena_storage::kernel::ExecutionPhase::Interrupted => 5,
    };
    part.revision = part.revision.saturating_add(
        (entry.execution_generation.get() - 1)
            .saturating_mul(8)
            .saturating_add(phase_revision),
    );
    if !matches!(&entry.payload, EntryPayload::Tool { .. }) {
        part.updated_at_ms = part.created_at_ms.saturating_add(part.revision);
    }
    if part.kind == "text"
        && part.role == "assistant"
        && let Some(text) = part.content.get("text").and_then(serde_json::Value::as_str)
    {
        part.inline_ui = crate::application_inline_ui::resolve_inline_text(reader, session, text)
            .await
            .map_err(crate::kernel_error)?;
    }
    Ok(part)
}

#[cfg(test)]
mod tool_input_projection_tests {
    use super::*;
    use agena_domain::{
        ComposerDocument, ComposerNode, StructuredObject, ToolApiCall, ToolApiFunction,
    };
    use agena_runtime_contracts::part_content::ToolCallContent;
    use agena_storage::kernel::{RequestId, ToolHandle, ToolResult, ToolStart};
    use serde_json::{Value, json};

    async fn read_tool_part(
        reader: &KernelReader,
        session: SessionId,
        tool: &ToolHandle,
        arguments: &Value,
    ) -> PartResource {
        let stored = reader.tool(session, tool.id()).await.unwrap();
        assert_eq!(
            &stored.call.input, arguments,
            "projection must not rewrite the protocol call"
        );
        let entry = reader.entry(session, stored.entry_id).await.unwrap();
        project_kernel_entry(reader, session, &entry).await.unwrap()
    }

    fn assert_input_projection(
        part: &PartResource,
        arguments: &Value,
        input: &Value,
        state: &str,
    ) -> ToolCallContent {
        assert_eq!(part.kind, "tool_call");
        assert_eq!(part.state, state);
        let content = ToolCallContent::try_from(&part.content).unwrap();
        assert_eq!(content.name, "shell.exec");
        assert_eq!(&content.input, input);
        assert_eq!(
            Value::from(content.tool_api_call.as_ref().unwrap().arguments.clone()),
            *arguments,
        );
        let details =
            agena_api::live::project_tool_call_content(&part.content, &[ToolDetailSection::Input]);
        assert_eq!(&details["input"], input);
        let presentation = agena_api::live::project_tool_call_content(
            &part.content,
            &[ToolDetailSection::Presentation],
        );
        assert!(presentation.get("input").is_none());
        assert_eq!(presentation["resources"], part.content["resources"]);
        content
    }

    #[tokio::test]
    async fn tool_timing_projection_preserves_stored_lifetimes_and_membership() {
        use agena_storage::kernel::ExecutionPhase;

        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .ensure_id("/tool-timing-projection-tests")
            .await
            .unwrap();
        let session = runtime
            .ports
            .sessions
            .create_configured(RequestId::new(), workspace, "A".into(), Default::default())
            .await
            .unwrap()
            .value
            .id;
        let foreign = runtime
            .ports
            .sessions
            .create_configured(RequestId::new(), workspace, "B".into(), Default::default())
            .await
            .unwrap()
            .value
            .id;
        let execution = runtime
            .ports
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "Check tool timing".into(),
                }]),
            )
            .await
            .unwrap()
            .value
            .id;
        let attempt = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let reader = &runtime.ports.reader;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "timed-tool".into(),
                    name: "shell.exec".into(),
                    input: json!({"command":"true"}),
                },
            )
            .await
            .unwrap()
            .value;
        let running = reader.tool(session, tool.id()).await.unwrap();
        assert!(running.created_at_ms > 0);
        assert_eq!(running.finished_at_ms, None);
        let running_entry = reader.entry(session, running.entry_id).await.unwrap();
        let live = project_kernel_entry(reader, session, &running_entry)
            .await
            .unwrap();
        assert_eq!(live.started_at_ms, running.created_at_ms);
        assert_eq!(live.finished_at_ms, None);
        assert_eq!(live.updated_at_ms, running.created_at_ms);
        let lifecycle = ToolCallContent::try_from(&live.content).unwrap().lifecycle;
        assert_eq!(lifecycle.start_ms, running.created_at_ms);
        assert_eq!(lifecycle.end_ms, None);

        // A real SQLite completion records a measurable interval, with no
        // fabricated result timestamps or production database mutation.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({}),
            },
        )
        .await
        .unwrap();
        let completed = reader.tool(session, tool.id()).await.unwrap();
        let end_ms = completed.finished_at_ms.unwrap();
        assert!(end_ms > completed.created_at_ms);
        let entry = reader.entry(session, completed.entry_id).await.unwrap();
        assert_eq!(entry.execution_phase, ExecutionPhase::Running);
        let projected = project_kernel_entry(reader, session, &entry).await.unwrap();
        assert_eq!(projected.started_at_ms, completed.created_at_ms);
        assert_eq!(projected.finished_at_ms, Some(end_ms));
        assert_eq!(projected.updated_at_ms, end_ms);
        assert_eq!(projected.created_at_ms, entry.created_at_ms);
        assert!(projected.revision > live.revision);
        for sections in [
            vec![],
            vec![ToolDetailSection::Input],
            vec![ToolDetailSection::Output],
            vec![ToolDetailSection::Metadata],
            vec![ToolDetailSection::Presentation],
        ] {
            let content = agena_api::live::project_tool_call_content(&projected.content, &sections);
            assert_eq!(content["lifecycle"]["start_ms"], completed.created_at_ms);
            assert_eq!(content["lifecycle"]["end_ms"], end_ms);
        }

        let broken = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "timed-projection-error".into(),
                    name: "shell.exec".into(),
                    input: json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        broken
            .complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: json!({"raw_output":17}),
                },
            )
            .await
            .unwrap();
        let broken_stored = reader.tool(session, broken.id()).await.unwrap();
        let broken_entry = reader.entry(session, broken_stored.entry_id).await.unwrap();
        let broken_part = project_kernel_entry(reader, session, &broken_entry)
            .await
            .unwrap();
        assert_eq!(broken_part.kind, "error");
        assert_eq!(broken_part.started_at_ms, broken_stored.created_at_ms);
        assert_eq!(broken_part.finished_at_ms, broken_stored.finished_at_ms);
        assert_eq!(
            broken_part.updated_at_ms,
            broken_stored.finished_at_ms.unwrap()
        );

        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let history = reader.history_window(session, None, &[], 64).await.unwrap();
        let history_entry = history
            .entries
            .iter()
            .find(|entry| entry.id == completed.entry_id)
            .unwrap();
        let historical = project_kernel_entry(reader, session, history_entry)
            .await
            .unwrap();
        assert_eq!(historical.started_at_ms, completed.created_at_ms);
        assert_eq!(historical.finished_at_ms, Some(end_ms));
        assert_eq!(historical.updated_at_ms, end_ms);
        assert!(historical.revision > projected.revision);
        let child = runtime
            .ports
            .sessions
            .fork(RequestId::new(), session, broken_stored.entry_id)
            .await
            .unwrap()
            .value
            .id;
        let inherited_entry = reader.entry(child, completed.entry_id).await.unwrap();
        let inherited = project_kernel_entry(reader, child, &inherited_entry)
            .await
            .unwrap();
        assert_eq!(inherited.started_at_ms, historical.started_at_ms);
        assert_eq!(inherited.finished_at_ms, historical.finished_at_ms);
        assert_eq!(inherited.origin_session_id, session.get());
        assert!(reader.tool(foreign, tool.id()).await.is_err());
        assert!(
            project_kernel_entry(reader, foreign, history_entry)
                .await
                .is_err()
        );

        // Older serialized views remain readable after adding timestamps.
        let mut legacy = serde_json::to_value(&*completed).unwrap();
        legacy.as_object_mut().unwrap().remove("created_at_ms");
        legacy.as_object_mut().unwrap().remove("finished_at_ms");
        let legacy: agena_storage::kernel::ToolView = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.created_at_ms, 0);
        assert_eq!(legacy.finished_at_ms, None);
    }

    #[tokio::test]
    async fn tool_input_projection_preserves_commands_across_live_completion_and_history_reads() {
        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .ensure_id("/tool-input-projection-tests")
            .await
            .unwrap();
        let session = runtime
            .ports
            .sessions
            .create_configured(RequestId::new(), workspace, "A".into(), Default::default())
            .await
            .unwrap()
            .value
            .id;
        let execution = runtime
            .ports
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "Run a command".into(),
                }]),
            )
            .await
            .unwrap()
            .value
            .id;
        let attempt = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let reader = &runtime.ports.reader;
        let input = json!({
            "command": "printf '%s\\n' '\"quoted\"'\nprintf '%s' '你好'",
            "workdir": "/tmp/a b",
        });
        for (encoding, wire_input) in [input.clone(), json!(input.to_string())]
            .into_iter()
            .enumerate()
        {
            for with_invocation in [false, true] {
                let arguments = json!({"tool":"shell.exec", "input":wire_input});
                let draft = round
                    .begin_tool_draft(
                        RequestId::new(),
                        format!("command-{encoding}-{with_invocation}"),
                        "tools_call".into(),
                    )
                    .await
                    .unwrap()
                    .value;
                let pending = reader.tool(session, draft.id()).await.unwrap();
                assert_eq!(pending.phase, ToolPhase::Preparing);
                drop(pending);
                let tool = draft
                    .finalize(RequestId::new(), arguments.clone())
                    .await
                    .unwrap()
                    .value;
                let log = tool.begin_log(RequestId::new()).await.unwrap().value;
                let live = read_tool_part(reader, session, &tool, &arguments).await;
                let content = assert_input_projection(&live, &arguments, &input, "running");
                assert_eq!(content.resources[0].resource_id, log.id());
                let rendered_input = StructuredObject::try_from(content.input).unwrap();
                assert_eq!(
                    rendered_input.get("command").unwrap().as_text(),
                    input["command"].as_str()
                );

                let mut result = json!({"raw_output":{"payload":{"exit_code":0}}});
                if with_invocation {
                    // Successful tool serializers can retain an explicit null.
                    // It must project like an omitted failure, also from history.
                    result["failure"] = Value::Null;
                    result["invocation"] = serde_json::to_value(agena_domain::ToolInvocation::new(
                        "shell.exec".to_owned(),
                        StructuredObject::try_from(input.clone()).unwrap(),
                    ))
                    .unwrap();
                }
                tool.complete(
                    RequestId::new(),
                    ToolResult {
                        phase: ToolPhase::Succeeded,
                        value: result,
                    },
                )
                .await
                .unwrap();
                let completed = read_tool_part(reader, session, &tool, &arguments).await;
                let content = assert_input_projection(&completed, &arguments, &input, "completed");
                assert!(content.error.is_none());
                assert_eq!(content.resources[0].resource_id, log.id());

                // Reconstruct from the durable history query, including old
                // completed records that have no persisted invocation object.
                let history = reader.history_window(session, None, &[], 64).await.unwrap();
                let entry = history.entries.iter().find(|entry| {
                    matches!(&entry.payload, EntryPayload::Tool { tool_id } if *tool_id == tool.id())
                }).unwrap();
                let rehydrated = project_kernel_entry(reader, session, entry).await.unwrap();
                assert_input_projection(&rehydrated, &arguments, &input, "completed");
            }
        }

        // Rejected calls must keep their actual input and failure details. A
        // malformed string, non-object JSON or missing input is never {}.
        for (index, raw_input) in [
            Some(json!("{")),
            Some(json!("[]")),
            Some(json!(r#""{}""#)),
            Some(Value::Null),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let mut arguments = json!({"tool":"shell.exec"});
            if let Some(input) = &raw_input {
                arguments["input"] = input.clone();
            }
            let call = ToolApiCall {
                function: ToolApiFunction::Call,
                arguments: StructuredObject::try_from(arguments.clone()).unwrap(),
            };
            let failure = agena_tool_api::ToolApiCatalog::default()
                .dispatch(&call)
                .unwrap_err()
                .failure();
            let tool = round
                .begin_tool(
                    RequestId::new(),
                    ToolStart {
                        call_id: format!("invalid-{index}"),
                        name: "tools_call".into(),
                        input: arguments.clone(),
                    },
                )
                .await
                .unwrap()
                .value;
            tool.complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Failed,
                    value: json!({"failure":failure}),
                },
            )
            .await
            .unwrap();
            let failed = read_tool_part(reader, session, &tool, &arguments).await;
            let content = assert_input_projection(
                &failed,
                &arguments,
                &raw_input.unwrap_or(Value::Null),
                "failed",
            );
            let document =
                crate::application_tools::tool_document_with_failure(&content, None).unwrap();
            assert_eq!(
                document.blocks[0].text_value(),
                Some(failure.user.fallback.as_str())
            );
            for section in [
                ToolDetailSection::Presentation,
                ToolDetailSection::Input,
                ToolDetailSection::Output,
                ToolDetailSection::Metadata,
            ] {
                let projected =
                    agena_api::live::project_tool_call_content(&failed.content, &[section]);
                let error: agena_domain::OperationError =
                    serde_json::from_value(projected["error"].clone()).unwrap();
                assert_eq!(
                    agena_failure::UserProblem::from(error.failure),
                    agena_failure::UserProblem::from(&failure),
                    "loading {section:?} must retain the failure even without output",
                );
            }
            assert_eq!(
                agena_failure::UserProblem::from(content.error.unwrap().failure),
                agena_failure::UserProblem::from(failure),
            );
        }

        // A malformed result is visibly isolated at its original entry. The
        // original facts remain available for repair; neighboring history must
        // not disappear because one optional field has the wrong shape.
        let invalid_fields = [
            ("result", json!([])),
            ("failure", json!("private-invalid-value")),
            ("failure", json!({})),
            ("failure", json!({"id":null})),
            ("raw_output", json!(17)),
            ("raw_output", json!({"attachments":[null]})),
            ("tool_api_response", json!(17)),
            ("tool_api_response.output_text", json!(17)),
            ("invocation", json!([])),
            ("tool_api_call", json!(false)),
            ("metadata", json!([])),
        ];
        for (index, (field, value)) in invalid_fields.iter().enumerate() {
            let tool = round
                .begin_tool(
                    RequestId::new(),
                    ToolStart {
                        call_id: format!("invalid-projection-{index}"),
                        name: "shell.exec".into(),
                        input: input.clone(),
                    },
                )
                .await
                .unwrap()
                .value;
            let result = match *field {
                "result" => value.clone(),
                "tool_api_response.output_text" => json!({
                    "raw_output":null,
                    "tool_api_response":{"output_text":value},
                }),
                _ => {
                    let mut result = json!({});
                    result[*field] = value.clone();
                    result
                }
            };
            tool.complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: result.clone(),
                },
            )
            .await
            .unwrap();
            let stored = reader.tool(session, tool.id()).await.unwrap();
            let entry = reader.entry(session, stored.entry_id).await.unwrap();
            let projected = project_kernel_entry(reader, session, &entry).await.unwrap();
            assert_eq!(projected.part_id, entry.id.get());
            assert_eq!(projected.kind, "error");
            assert_eq!(projected.state, "failed");
            assert_eq!(projected.content["projection_error"], true);
            assert_eq!(projected.content["invalid_field"], *field);
            assert!(
                !projected
                    .content
                    .to_string()
                    .contains("private-invalid-value")
            );
            assert_eq!(stored.phase, ToolPhase::Succeeded);
            assert_eq!(stored.result.as_ref(), Some(&result));
        }

        let history = reader.history_window(session, None, &[], 64).await.unwrap();
        let mut projected = Vec::new();
        for entry in &history.entries {
            projected.push(project_kernel_entry(reader, session, entry).await.unwrap());
        }
        assert_eq!(
            projected.iter().filter(|part| part.kind == "error").count(),
            invalid_fields.len(),
        );
        assert!(
            projected
                .iter()
                .any(|part| { part.kind == "tool_call" && part.state == "completed" })
        );
        assert!(
            projected
                .iter()
                .any(|part| { part.kind == "tool_call" && part.state == "failed" })
        );
    }

    #[tokio::test]
    async fn tool_result_projection_preserves_media_nulls_and_real_failures() {
        use agena_storage::kernel::ExecutionPhase;

        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .ensure_id("/tool-result-projection-tests")
            .await
            .unwrap();
        let session = runtime
            .ports
            .sessions
            .create_configured(RequestId::new(), workspace, "A".into(), Default::default())
            .await
            .unwrap()
            .value
            .id;
        let foreign = runtime
            .ports
            .sessions
            .create_configured(RequestId::new(), workspace, "B".into(), Default::default())
            .await
            .unwrap()
            .value
            .id;
        let execution = runtime
            .ports
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "Create an image".into(),
                }]),
            )
            .await
            .unwrap()
            .value
            .id;
        let attempt = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let reader = &runtime.ports.reader;
        let input = json!({"prompt":"An image"});
        let attachment = json!({
            "kind":"image", "mime":"image/png",
            "source":{"source":"local_path", "path":"/managed/session/image.png"},
            "filename":"image.png", "size_bytes":1024, "sha256":"abc123",
        });
        for explicit_nulls in [false, true] {
            for with_attachment in [false, true] {
                let tool = round
                    .begin_tool(
                        RequestId::new(),
                        ToolStart {
                            call_id: format!("media-{explicit_nulls}-{with_attachment}"),
                            name: "image_generation".into(),
                            input: input.clone(),
                        },
                    )
                    .await
                    .unwrap()
                    .value;
                let mut result = json!({
                    "tool_api_response":{"output_text":"Legacy output preview"},
                });
                if explicit_nulls {
                    for field in [
                        "raw_output",
                        "invocation",
                        "tool_api_call",
                        "failure",
                        "metadata",
                    ] {
                        result[field] = Value::Null;
                    }
                }
                if with_attachment {
                    result["raw_output"] = json!({
                        "payload":{"text":"Generated image"},
                        "attachments":[attachment.clone()],
                        "truncated":false,
                    });
                    // An unused malformed legacy preview must not override
                    // or invalidate valid canonical image output.
                    result["tool_api_response"] = if explicit_nulls {
                        json!(17)
                    } else {
                        json!({"output_text":17})
                    };
                }
                tool.complete(
                    RequestId::new(),
                    ToolResult {
                        phase: ToolPhase::Succeeded,
                        value: result.clone(),
                    },
                )
                .await
                .unwrap();
                let stored = reader.tool(session, tool.id()).await.unwrap();
                assert_eq!(stored.result.as_ref(), Some(&result));
                let entry = reader.entry(session, stored.entry_id).await.unwrap();
                let projected = project_kernel_entry(reader, session, &entry).await.unwrap();
                let content = ToolCallContent::try_from(&projected.content).unwrap();
                assert_eq!(content.name, "image_generation");
                assert_eq!(content.input, input);
                assert!(content.error.is_none());
                assert!(content.metadata.is_empty());
                let output = content.output.unwrap();
                if with_attachment {
                    assert_eq!(output.attachments.len(), 1);
                    assert_eq!(
                        serde_json::to_value(&output.attachments[0]).unwrap(),
                        attachment
                    );
                    assert_eq!(output.text_content(), "Generated image");
                } else {
                    assert!(output.attachments.is_empty());
                    assert_eq!(output.text_content(), "Legacy output preview");
                }
            }
        }

        for (index, response) in [Value::Null, json!({"output_text":null}), json!({})]
            .into_iter()
            .enumerate()
        {
            let tool = round
                .begin_tool(
                    RequestId::new(),
                    ToolStart {
                        call_id: format!("empty-legacy-output-{index}"),
                        name: "image_generation".into(),
                        input: input.clone(),
                    },
                )
                .await
                .unwrap()
                .value;
            let result = json!({"raw_output":null,"tool_api_response":response});
            tool.complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: result.clone(),
                },
            )
            .await
            .unwrap();
            let stored = reader.tool(session, tool.id()).await.unwrap();
            let entry = reader.entry(session, stored.entry_id).await.unwrap();
            let projected = project_kernel_entry(reader, session, &entry).await.unwrap();
            let content = ToolCallContent::try_from(&projected.content).unwrap();
            assert!(content.output.is_none());
            assert!(content.error.is_none());
            assert_eq!(stored.result.as_ref(), Some(&result));
        }

        let failure = agena_tool_api::ToolApiCatalog::default()
            .dispatch(&ToolApiCall {
                function: ToolApiFunction::Call,
                arguments: StructuredObject::try_from(json!({
                    "tool":"shell.exec", "input":null,
                }))
                .unwrap(),
            })
            .unwrap_err()
            .failure();
        let problem = serde_json::to_value(agena_failure::UserProblem::from(&failure)).unwrap();
        let broken = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "bad-output-with-real-failure".into(),
                    name: "shell.exec".into(),
                    input: json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let broken_result = json!({
            "raw_output":"private-corrupt-output",
            "failure":failure,
        });
        broken
            .complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Failed,
                    value: broken_result.clone(),
                },
            )
            .await
            .unwrap();
        let stored = reader.tool(session, broken.id()).await.unwrap();
        let entry = reader.entry(session, stored.entry_id).await.unwrap();
        assert!(project_kernel_entry(reader, foreign, &entry).await.is_err());
        let projected = project_kernel_entry(reader, session, &entry).await.unwrap();
        assert_eq!(projected.part_id, entry.id.get());
        assert_eq!(projected.kind, "error");
        assert_eq!(projected.content["problem"], problem);
        assert_eq!(projected.content["invalid_field"], "raw_output");
        assert!(
            !projected
                .content
                .to_string()
                .contains("private-corrupt-output")
        );
        assert_eq!(stored.result.as_ref(), Some(&broken_result));

        round
            .finish(RequestId::new(), Some(json!({"failure":failure})), false)
            .await
            .unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Failed)
            .await
            .unwrap();
        let history = reader.history_window(session, None, &[], 64).await.unwrap();
        let mut parts = Vec::new();
        for entry in &history.entries {
            parts.push(project_kernel_entry(reader, session, entry).await.unwrap());
        }
        append_execution_failures(reader, &history.entries, &mut parts)
            .await
            .unwrap();
        let errors = parts
            .iter()
            .filter(|part| part.kind == "error")
            .collect::<Vec<_>>();
        assert_eq!(
            errors.len(),
            2,
            "projection errors must not suppress the real round failure"
        );
        assert!(
            errors
                .iter()
                .any(|part| part.content["projection_error"] == true)
        );
        assert!(errors.iter().any(|part| {
            part.content.get("projection_error").is_none() && part.content["problem"] == problem
        }));
        assert_eq!(
            parts.iter().filter(|part| part.kind == "tool_call").count(),
            7
        );
    }
}

fn project_interaction(
    session: SessionId,
    interaction: &agena_storage::kernel::InteractionView,
    authorization: &mut agena_domain::OperationAuthorization,
    user_input: &mut agena_domain::OperationUserInput,
) -> Result<(), ApplicationError> {
    use agena_storage::kernel::{InteractionDecision, InteractionRequest};
    let request_id = interaction.id.get().to_string();
    match &interaction.request {
        InteractionRequest::Permission {
            action_key,
            description,
        } => {
            let request = agena_domain::PermissionRequest {
                request_id: request_id.clone(),
                session_id: Some(session.get()),
                action: serde_json::from_str(action_key)
                    .map_err(|error| ApplicationError::internal_error(&error))?,
                related_actions: Vec::new(),
                requested_actions: Vec::new(),
                reason: description.clone(),
                explanation: String::new(),
                source: Some("kernel".into()),
                scope: None,
                operator: None,
                trace: Vec::new(),
                created_at: chrono::DateTime::from_timestamp_millis(interaction.created_at_ms)
                    .unwrap_or(chrono::DateTime::UNIX_EPOCH),
            };
            let reply = match &interaction.decision {
                Some(InteractionDecision::Permission { reply, .. }) => Some(reply.clone()),
                Some(InteractionDecision::Cancel) => Some(agena_domain::PermissionReply {
                    request_id,
                    kind: agena_domain::PermissionReplyKind::DenyOnce,
                    reason: None,
                    scope: None,
                }),
                Some(_) => {
                    return Err(ApplicationError::internal(
                        "A permission interaction has an invalid decision.",
                    ));
                }
                None => None,
            };
            authorization
                .permissions
                .push(agena_domain::OperationPermission {
                    request,
                    reply,
                    replied_at_ms: interaction.decided_at_ms,
                });
        }
        InteractionRequest::Input { request } => {
            let mut request = request.clone();
            request.request_id = request_id.clone();
            request.session_id = Some(session.get());
            let reply = match &interaction.decision {
                Some(InteractionDecision::Input(value)) => {
                    let mut reply: agena_domain::UserInputReply = serde_json::from_str(value)
                        .map_err(|error| ApplicationError::internal_error(&error))?;
                    reply.request_id = request_id;
                    Some(reply)
                }
                Some(_) => Some(agena_domain::UserInputReply {
                    request_id,
                    kind: agena_domain::UserInputReplyKind::Cancel,
                    answers: Default::default(),
                    reason: None,
                }),
                None => None,
            };
            user_input
                .requests
                .push(agena_domain::OperationUserInputRecord {
                    request,
                    reply,
                    replied_at_ms: interaction.decided_at_ms,
                });
        }
    }
    Ok(())
}

impl Application {
    /// Read only live tool entries owned by this session. Non-tool history and
    /// inherited tools are excluded by the indexed kernel query before bodies
    /// are loaded. File evidence uses raw facts, so no presentation is rendered.
    pub async fn read_owned_tool_parts(
        &self,
        session_id: i64,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<OwnedToolPartsPage, ApplicationError> {
        let session = crate::kernel_session_id(session_id)?;
        let cursor = cursor.map(decode_cursor::<OwnedToolCursor>).transpose()?;
        if cursor.is_some_and(|cursor| cursor.session != session) {
            return Err(ApplicationError::bad_request(
                "The cursor belongs to a different session.",
            ));
        }
        let reader = &self.session_kernel()?.reader;
        let page = reader
            .owned_tool_entries(session, cursor, limit.clamp(1, 128))
            .await
            .map_err(crate::kernel_error)?;
        let mut parts = Vec::with_capacity(page.entries.len());
        for entry in &page.entries {
            parts.push(project_kernel_entry(reader, session, entry).await?);
        }
        // Each tool detail is an authoritative read. Session control changes
        // (titles, rounds, text, etc.) do not invalidate durable tool evidence.
        // Transports retain the resource token captured before this read, so
        // concurrent edits get a trailing validation rather than a false 409.
        Ok(OwnedToolPartsPage {
            parts,
            control_version: page.control_version,
            next_cursor: page.next.map(|cursor| encode_cursor(&cursor)).transpose()?,
            has_more: page.has_more,
        })
    }

    pub async fn project_entry(
        &self,
        session_id: i64,
        entry: &EntryView,
        sections: &[ToolDetailSection],
    ) -> Result<PartResource, ApplicationError> {
        let mut part = project_kernel_entry(
            &self.session_kernel()?.reader,
            crate::kernel_session_id(session_id)?,
            entry,
        )
        .await?;
        self.project_part_resource(&mut part, sections).await;
        Ok(part)
    }

    pub async fn read_parts(
        &self,
        request: ReadPartsParams,
    ) -> Result<SessionPartsResource, ApplicationError> {
        if request.ids.len() > 256
            || request.run_ids.len() > 32
            || request
                .ids
                .iter()
                .chain(&request.run_ids)
                .any(|id| *id <= 0)
        {
            return Err(ApplicationError::bad_request(
                "The entry selection exceeds its limit or contains an invalid identity.",
            ));
        }
        let session = crate::kernel_session_id(request.session_id)?;
        let reader = &self.session_kernel()?.reader;
        let (metadata, entries, next, has_more) = if request.ids.is_empty() {
            let page = reader
                .history_window(
                    session,
                    decode_part_cursor(request.session_id, request.cursor.as_deref())?,
                    &request.run_ids,
                    request.limit.unwrap_or(64).clamp(1, 256) as usize,
                )
                .await
                .map_err(crate::kernel_error)?;
            (
                page.metadata.clone(),
                page.entries.clone(),
                page.next,
                page.has_more,
            )
        } else {
            if !request.run_ids.is_empty() || request.cursor.is_some() || request.limit.is_some() {
                return Err(ApplicationError::bad_request(
                    "Explicit entry identities cannot be combined with a history range.",
                ));
            }
            let metadata = reader
                .metadata(session)
                .await
                .map_err(crate::kernel_error)?;
            let mut entries = Vec::with_capacity(request.ids.len());
            for id in request
                .ids
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
            {
                let id = id
                    .try_into()
                    .map_err(|error| ApplicationError::bad_request_error(&error))?;
                match reader.entry(session, id).await {
                    Ok(entry) => entries.push((*entry).clone()),
                    Err(agena_storage::kernel::KernelError::NotFound(_)) => {}
                    Err(error) => return Err(crate::kernel_error(error)),
                }
            }
            ((*metadata).clone(), entries, None, false)
        };
        let mut parts = Vec::with_capacity(entries.len());
        for entry in &entries {
            if !matches!(entry.payload, EntryPayload::Interaction { .. }) {
                parts.push(
                    self.project_entry(
                        request.session_id,
                        entry,
                        requested_sections(&request.sections),
                    )
                    .await?,
                );
            }
        }
        append_execution_failures(reader, &entries, &mut parts).await?;
        let summary = reader.summary(session).await.map_err(crate::kernel_error)?;
        Ok(SessionPartsResource {
            session_id: request.session_id,
            version: metadata.metadata_version.get(),
            page: agena_api::pagination::PageInfo {
                returned: parts.len() as u64,
                has_more,
                next_cursor: encode_part_cursor(request.session_id, next)?,
            },
            parts,
            runs: Vec::new(),
            user_message_count: Some(summary.user_message_count),
        })
    }

    pub async fn read_runs(
        &self,
        request: ReadRunsParams,
    ) -> Result<SessionPartsResource, ApplicationError> {
        let session = crate::kernel_session_id(request.session_id)?;
        let reader = &self.session_kernel()?.reader;
        let page = reader
            .message_history(
                session,
                decode_part_cursor(request.session_id, request.cursor.as_deref())?,
                request.limit.unwrap_or(8).clamp(1, 32) as usize,
                request.part_limit.unwrap_or(32).clamp(1, 128) as usize,
            )
            .await
            .map_err(crate::kernel_error)?;
        let mut parts = Vec::with_capacity(page.entries.len());
        for entry in &page.entries {
            parts.push(
                self.project_entry(
                    request.session_id,
                    entry,
                    requested_sections(&request.sections),
                )
                .await?,
            );
        }
        append_execution_failures(reader, &page.entries, &mut parts).await?;
        let runs = page
            .messages
            .iter()
            .map(|message| {
                Ok(RunSummaryResource {
                    run_id: message.id.get(),
                    part_count: message.entry_count,
                    page: agena_api::pagination::PageInfo {
                        returned: message.loaded_count,
                        has_more: message.loaded_count < message.entry_count,
                        next_cursor: encode_part_cursor(request.session_id, message.next)?,
                    },
                })
            })
            .collect::<Result<Vec<_>, ApplicationError>>()?;
        Ok(SessionPartsResource {
            session_id: request.session_id,
            version: page.metadata.metadata_version.get(),
            parts,
            runs,
            user_message_count: Some(page.user_message_count),
            page: agena_api::pagination::PageInfo {
                returned: page.messages.len() as u64,
                has_more: page.has_more,
                next_cursor: encode_part_cursor(request.session_id, page.next)?,
            },
        })
    }
}
