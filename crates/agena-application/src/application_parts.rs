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
    part.updated_at_ms = part.created_at_ms.saturating_add(part.revision);
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
