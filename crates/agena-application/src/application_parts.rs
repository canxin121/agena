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
        ));
    }
    let mut failures = Vec::new();
    for (run_id, execution, generation, owner, candidate_created_at_ms) in candidates {
        if parts
            .iter()
            .any(|existing| existing.kind == "error" && existing.run_id == Some(run_id))
        {
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
                        round.created_at_ms,
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
        failures.push((run_id, source_id, created_at_ms, content, owner));
    }

    let mut used_ids = parts
        .iter()
        .map(|part| part.part_id)
        .collect::<std::collections::HashSet<_>>();
    for (run_id, round_id, created_at_ms, content, owner) in failures {
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
            origin_session_id: owner.get(),
            revision: 1,
            started_at_ms: created_at_ms,
            finished_at_ms: Some(created_at_ms),
            created_at_ms,
            updated_at_ms: created_at_ms,
            ..PartResource::default()
        });
    }
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
            part.content = serde_json::json!({"text":document.text(),"document":document,
                "execution_id":entry.execution_id,"run_kind":"user_send"});
        }
        EntryPayload::Text { content_id } | EntryPayload::Reasoning { content_id } => {
            let reasoning = matches!(&entry.payload, EntryPayload::Reasoning { .. });
            part.kind = if reasoning { "think" } else { "text" }.into();
            part.content = serde_json::json!({"text":"","resources":[agena_domain::ContentRef {
                resource_id:*content_id,kind:agena_domain::ContentKind::Text }],
                "execution_id":entry.execution_id});
        }
        EntryPayload::Compaction { summary, through } => {
            part.kind = "text".into();
            part.visibility = "user".into();
            part.content = serde_json::json!({"text":"","resources":[agena_domain::ContentRef {
                resource_id:*summary, kind:agena_domain::ContentKind::Text }],"compaction_through":through});
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
        EntryPayload::Tool { tool_id } => {
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
            let result = tool.result.as_ref();
            let decode = |key: &str| result.and_then(|value| value.get(key)).cloned();
            let output = decode("raw_output")
                .map(serde_json::from_value::<agena_domain::RawOutput>)
                .transpose()
                .map_err(|error| ApplicationError::internal_error(&error))?;
            let invocation = decode("invocation")
                .map(serde_json::from_value::<agena_domain::ToolInvocation>)
                .transpose()
                .map_err(|error| ApplicationError::internal_error(&error))?;
            let tool_api_call = decode("tool_api_call")
                .filter(|value| !value.is_null())
                .map(serde_json::from_value::<agena_domain::ToolApiCall>)
                .transpose()
                .map_err(|error| ApplicationError::internal_error(&error))?
                .or_else(|| {
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
                (
                    name.to_owned(),
                    None,
                    tool.call
                        .input
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                )
            } else {
                (tool.call.name.clone(), None, tool.call.input.clone())
            };
            let error = decode("failure")
                .map(serde_json::from_value::<agena_failure::Failure>)
                .transpose()
                .map_err(|error| ApplicationError::internal_error(&error))?
                .map(|failure| agena_domain::OperationError { failure });
            let metadata = decode("metadata")
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| ApplicationError::internal_error(&error))?
                .unwrap_or_default();
            let mut authorization = agena_domain::OperationAuthorization::default();
            let mut user_input = agena_domain::OperationUserInput::default();
            for id in &tool.interactions {
                let interaction = reader
                    .interaction(session, *id)
                    .await
                    .map_err(crate::kernel_error)?;
                project_interaction(session, &interaction, &mut authorization, &mut user_input)?;
            }
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
                    start_ms: entry.created_at_ms,
                    end_ms: None,
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
    Ok(part)
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
        let parts = async {
            let mut parts = Vec::with_capacity(page.entries.len());
            for entry in &page.entries {
                parts.push(project_kernel_entry(reader, session, entry).await?);
            }
            Ok::<_, ApplicationError>(parts)
        }
        .await;
        // Tool detail projection performs separate bounded reads. Do not mix
        // versions or cache a partial old/new projection if a writer raced it.
        let version = *reader
            .control_version(session)
            .await
            .map_err(crate::kernel_error)?;
        if version != page.control_version {
            return Err(ApplicationError::conflict(
                "The session changed while loading its file history. Refresh and retry.",
            ));
        }
        Ok(OwnedToolPartsPage {
            parts: parts?,
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
        let summary = reader.summary(session).await.map_err(crate::kernel_error)?;
        Ok(SessionPartsResource {
            session_id: request.session_id,
            version: page.metadata.metadata_version.get(),
            parts,
            runs,
            user_message_count: Some(summary.user_message_count),
            page: agena_api::pagination::PageInfo {
                returned: page.messages.len() as u64,
                has_more: page.has_more,
                next_cursor: encode_part_cursor(request.session_id, page.next)?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{
        ComposerDocument, ComposerNode, PermissionAction, PermissionMode, PermissionScope,
    };
    use agena_storage::kernel::*;

    #[tokio::test]
    async fn native_tool_projection_preserves_provider_output_and_permission_history() {
        let root = tempfile::tempdir().unwrap();
        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let kernel = &runtime.ports;
        let session = kernel
            .sessions
            .create(RequestId::new(), workspace.id, "projection".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = kernel
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "question".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .executor
            .claim(RequestId::new(), session, sent.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![sent.message_id])
            .await
            .unwrap()
            .value;
        let input = serde_json::json!({"tool":"fs.read","input":{"path":"file.txt"}});
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "provider-call".into(),
                    name: "tools_call".into(),
                    input: input.clone(),
                },
            )
            .await
            .unwrap()
            .value;
        let action = PermissionAction::Tool {
            tool_name: "fs.read".into(),
            qualifier: None,
        };
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: serde_json::to_string(&action).unwrap(),
                    description: "read selected file".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let view = kernel.reader.tool(session, tool.id()).await.unwrap();
        let entry = kernel.reader.entry(session, view.entry_id).await.unwrap();
        let pending = project_kernel_entry(&kernel.reader, session, &entry)
            .await
            .unwrap();
        let content =
            agena_runtime_contracts::part_content::ToolCallContent::try_from(&pending.content)
                .unwrap();
        assert_eq!(pending.part_id, view.entry_id.get());
        assert!(pending.message_boundary);
        assert_eq!(pending.run_id, Some(pending.part_id));
        assert_eq!(content.name, "fs.read");
        assert_eq!(content.input, serde_json::json!({"path":"file.txt"}));
        assert_eq!(
            serde_json::Value::from(content.tool_api_call.unwrap().arguments),
            input
        );
        assert_eq!(content.authorization.awaiting().count(), 1);
        assert_eq!(
            content.authorization.permissions[0].request.request_id,
            interaction.get().to_string()
        );
        let expected_reply = agena_domain::PermissionReply {
            request_id: interaction.get().to_string(),
            kind: agena_domain::PermissionReplyKind::AllowAlways,
            reason: Some("approved for this session 中文 🧪".into()),
            scope: Some(PermissionScope::Session),
        };
        kernel
            .sessions
            .reply(
                RequestId::new(),
                session,
                interaction,
                InteractionDecision::Permission {
                    reply: expected_reply.clone(),
                    allowed: true,
                },
                Some(RememberPermission {
                    mode: PermissionMode::Allow,
                    scope: PermissionScope::Session,
                }),
            )
            .await
            .unwrap();
        let invocation = agena_domain::ToolInvocation {
            name: "fs.read".into(),
            plugin_name: Some("fs".into()),
            input: agena_domain::StructuredObject::try_from(serde_json::json!({"path":"file.txt"}))
                .unwrap(),
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: agena_domain::ToolApiFunction::Call,
                arguments: agena_domain::StructuredObject::try_from(input).unwrap(),
            }),
        };
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({
            "raw_output":agena_domain::RawOutput::text(" exact output \n"),"invocation":invocation,
            "metadata":{"source":"fixture"},"summary":"read file"}),
            },
        )
        .await
        .unwrap();
        let entry = kernel.reader.entry(session, view.entry_id).await.unwrap();
        let done = project_kernel_entry(&kernel.reader, session, &entry)
            .await
            .unwrap();
        let content =
            agena_runtime_contracts::part_content::ToolCallContent::try_from(&done.content)
                .unwrap();
        assert_eq!(content.output.unwrap().text_content(), " exact output \n");
        assert_eq!(content.plugin.as_deref(), Some("fs"));
        assert_eq!(content.metadata["source"], "fixture");
        assert_eq!(content.authorization.awaiting().count(), 0);
        assert_eq!(
            content.authorization.permissions[0].reply.as_ref(),
            Some(&expected_reply)
        );
        assert_eq!(
            content.authorization.permissions[0].replied_at_ms,
            kernel
                .reader
                .interaction(session, interaction)
                .await
                .unwrap()
                .decided_at_ms
        );
        assert!(content.authorization.permissions[0].replied_at_ms.is_some());
        assert_eq!(done.summary.as_deref(), Some("read file"));
        let middle = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        middle.append("middle").await.unwrap();
        let tail = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        tail.append("tail").await.unwrap();
        let window = kernel
            .reader
            .message_history(session, None, 2, 1)
            .await
            .unwrap();
        assert!(
            window
                .entries
                .iter()
                .any(|entry| entry.id == tail.entry_id())
        );
        assert!(window.entries.iter().any(|entry| entry.id == view.entry_id));
        let response = window
            .messages
            .iter()
            .find(|message| message.id == view.entry_id)
            .unwrap();
        assert_eq!(response.loaded_count, 2);
        assert_eq!(response.entry_count, 3);
        assert!(response.next.is_some());
    }

    #[tokio::test]
    async fn failure_notice_projects_as_an_error_part_without_a_duplicate() {
        let root = tempfile::tempdir().unwrap();
        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let kernel = &runtime.ports;
        let session = kernel
            .sessions
            .create(RequestId::new(), workspace.id, "failures".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = kernel
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "question".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .executor
            .claim(RequestId::new(), session, sent.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![sent.message_id])
            .await
            .unwrap()
            .value;
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        text.append("answer").await.unwrap();
        text.finish(RequestId::new()).await.unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        let boundary = text.entry_id();
        let failure = agena_failure::Failure::new(
            agena_failure::FailureCode::new("configuration.invalid"),
            agena_failure::FailureCategory::InvalidInput,
            agena_failure::FailureResponsibility::Caller,
            agena_failure::RetryDirective::CorrectInput,
            agena_failure::RecoveryDirective::OpenSettings,
            agena_failure::FailureImpact::RequestRejected,
            agena_failure::UserPresentation::new(
                "configuration.invalid",
                "the model repeated the same tool call without progress",
            ),
        );
        let notice = attempt
            .notice(
                RequestId::new(),
                agena_runtime_contracts::part_content::failure_notice_text(&failure).unwrap(),
            )
            .await
            .unwrap()
            .value;
        attempt
            .finish(RequestId::new(), ExecutionPhase::Failed)
            .await
            .unwrap();

        let notice_entry = kernel.reader.entry(session, notice).await.unwrap();
        let notice_part = project_kernel_entry(&kernel.reader, session, &notice_entry)
            .await
            .unwrap();
        assert_eq!(notice_part.kind, "error");
        assert_eq!(notice_part.state, "failed");
        assert_eq!(notice_part.run_state.as_deref(), Some("failed"));
        assert_eq!(notice_part.run_id, Some(boundary.get()));
        let content =
            agena_runtime_contracts::part_content::ErrorContent::try_from(&notice_part.content)
                .unwrap();
        assert_eq!(
            content.message,
            "the model repeated the same tool call without progress"
        );

        let boundary_entry = kernel.reader.entry(session, boundary).await.unwrap();
        let boundary_part = project_kernel_entry(&kernel.reader, session, &boundary_entry)
            .await
            .unwrap();
        assert_eq!(boundary_part.state, "completed");
        assert_eq!(boundary_part.run_state.as_deref(), Some("failed"));
        let mut parts = vec![boundary_part, notice_part];
        let entries = vec![(*boundary_entry).clone(), (*notice_entry).clone()];
        append_execution_failures(&kernel.reader, &entries, &mut parts)
            .await
            .unwrap();
        assert_eq!(
            parts.iter().filter(|part| part.kind == "error").count(),
            1,
            "the durable failure notice must not be duplicated by the read-time projection"
        );
    }

    #[tokio::test]
    async fn failed_execution_without_a_failed_round_still_projects_an_error() {
        let root = tempfile::tempdir().unwrap();
        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let kernel = &runtime.ports;
        let session = kernel
            .sessions
            .create(RequestId::new(), workspace.id, "manager-failure".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = kernel
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "question".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .executor
            .claim(RequestId::new(), session, sent.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![sent.message_id])
            .await
            .unwrap()
            .value;
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        text.append("answer").await.unwrap();
        text.finish(RequestId::new()).await.unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Failed)
            .await
            .unwrap();

        let boundary_entry = kernel.reader.entry(session, text.entry_id()).await.unwrap();
        let mut tail_only = project_kernel_entry(&kernel.reader, session, &boundary_entry)
            .await
            .unwrap();
        // A bounded window often omits the boundary entry and starts at a
        // later reasoning/tool entry of the same failed run.
        tail_only.message_boundary = false;
        tail_only.run_state = None;
        let mut parts = vec![tail_only];
        let entries = vec![(*boundary_entry).clone()];
        append_execution_failures(&kernel.reader, &entries, &mut parts)
            .await
            .unwrap();
        let errors = parts
            .iter()
            .filter(|part| part.kind == "error")
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].run_state.as_deref(), Some("failed"));
        assert_eq!(errors[0].run_id, Some(text.entry_id().get()));
        let content =
            agena_runtime_contracts::part_content::ErrorContent::try_from(&errors[0].content)
                .unwrap();
        assert!(content.message.contains("ended unexpectedly"));
    }

    #[tokio::test]
    async fn tool_only_tail_projects_the_failed_run_error() {
        let root = tempfile::tempdir().unwrap();
        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let kernel = &runtime.ports;
        let session = kernel
            .sessions
            .create(RequestId::new(), workspace.id, "tool-tail".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = kernel
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "question".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .executor
            .claim(RequestId::new(), session, sent.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![sent.message_id])
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "provider-call".into(),
                    name: "fs.read".into(),
                    input: serde_json::json!({"path":"file.txt"}),
                },
            )
            .await
            .unwrap()
            .value;
        let view = kernel.reader.tool(session, tool.id()).await.unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Failed,
                value: serde_json::json!({"error":{"message":"tool failed"}}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Failed)
            .await
            .unwrap();

        let entry = kernel.reader.entry(session, view.entry_id).await.unwrap();
        let tool_part = project_kernel_entry(&kernel.reader, session, &entry)
            .await
            .unwrap();
        assert_eq!(tool_part.kind, "tool_call");
        let mut parts = vec![tool_part];
        let entries = vec![(*entry).clone()];
        append_execution_failures(&kernel.reader, &entries, &mut parts)
            .await
            .unwrap();
        let errors = parts
            .iter()
            .filter(|part| part.kind == "error")
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].run_id, Some(view.entry_id.get()));
        let content =
            agena_runtime_contracts::part_content::ErrorContent::try_from(&errors[0].content)
                .unwrap();
        assert!(content.message.contains("ended unexpectedly"));
    }

    #[tokio::test]
    async fn completed_streams_in_a_running_attempt_are_not_marked_running() {
        let root = tempfile::tempdir().unwrap();
        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let kernel = &runtime.ports;
        let session = kernel
            .sessions
            .create(RequestId::new(), workspace.id, "streams".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = kernel
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "question".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .executor
            .claim(RequestId::new(), session, sent.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![sent.message_id])
            .await
            .unwrap()
            .value;
        let finished = round
            .begin_text(RequestId::new(), true)
            .await
            .unwrap()
            .value;
        finished.append("thought").await.unwrap();
        finished.finish(RequestId::new()).await.unwrap();
        let streaming = round
            .begin_text(RequestId::new(), true)
            .await
            .unwrap()
            .value;
        streaming.append("still writing").await.unwrap();

        let finished_entry = kernel
            .reader
            .entry(session, finished.entry_id())
            .await
            .unwrap();
        assert_eq!(finished_entry.content_state, Some(CaptureState::Complete));
        let finished_part = project_kernel_entry(&kernel.reader, session, &finished_entry)
            .await
            .unwrap();
        assert_eq!(finished_part.state, "completed");
        assert_eq!(finished_part.run_state.as_deref(), Some("running"));

        let streaming_entry = kernel
            .reader
            .entry(session, streaming.entry_id())
            .await
            .unwrap();
        assert_eq!(streaming_entry.content_state, Some(CaptureState::Open));
        let streaming_part = project_kernel_entry(&kernel.reader, session, &streaming_entry)
            .await
            .unwrap();
        assert_eq!(streaming_part.state, "running");
    }
}
