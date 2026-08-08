use super::{ExecutionControlError, execution_control_to_app_error};
use crate::{
    AppError,
    message::{Message, MessagePart, PartContent},
    session::{Session, SessionManager},
};
use agena_domain::{ExecutionStatus, KindMatcher, Role, SessionSummary};
use agena_runtime::{SessionForkRequest, SessionRewindRequest};
use sea_orm::{ConnectionTrait, Statement};

fn uuid_value<T>(value: String, wrap: impl FnOnce(uuid::Uuid) -> T) -> Result<T, AppError> {
    uuid::Uuid::parse_str(&value)
        .map(wrap)
        .map_err(|error| AppError::Internal(format!("invalid transcript UUID {value}: {error}")))
}

/// Batch-load transcript documents for many owners with a query count that
/// is independent of the number of owners (two queries per chunk of 400
/// owners). `owners` lists `(owner_kind, owner_id)` pairs; the result map is
/// keyed identically and contains every requested key, empty documents
/// included.
async fn transcript_documents_batch(
    db: &sea_orm::DatabaseConnection,
    owners: &[(String, String)],
) -> Result<std::collections::HashMap<(String, String), agena_domain::ContentDocument>, AppError> {
    const MAX_OWNERS_PER_QUERY: usize = 400;

    let mut documents = owners
        .iter()
        .cloned()
        .map(|owner| (owner, agena_domain::ContentDocument::default()))
        .collect::<std::collections::HashMap<_, _>>();

    for chunk in owners.chunks(MAX_OWNERS_PER_QUERY) {
        let tuples = chunk
            .iter()
            .map(|_| "(?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let values = chunk
            .iter()
            .flat_map(|(kind, id)| [kind.clone().into(), id.clone().into()])
            .collect::<Vec<sea_orm::Value>>();

        let node_rows = db
            .query_all(Statement::from_sql_and_values(
                db.get_database_backend(),
                format!(
                    "SELECT owner_kind, owner_id, node_id, node_type, actor, title, payload_json, text, \
                            state, position, revision_seq, started_at_ms, finished_at_ms \
                     FROM agena_content_nodes \
                     WHERE (owner_kind, owner_id) IN (VALUES {tuples}) \
                     ORDER BY position, node_id"
                ),
                values,
            ))
            .await?;
        let mut positioned_nodes = std::collections::HashMap::<_, Vec<_>>::new();
        for row in node_rows {
            let owner_key = (
                row.try_get::<String>("", "owner_kind")?,
                row.try_get::<String>("", "owner_id")?,
            );
            let node_id: String = row.try_get("", "node_id")?;
            let position: i64 = row.try_get("", "position")?;
            let node_type: String = row.try_get("", "node_type")?;
            let node = match node_type.as_str() {
                "text" => {
                    let position = u32::try_from(position).map_err(|_| {
                        AppError::Internal(format!("invalid transcript text position {position}"))
                    })?;
                    agena_domain::ContentNode::text_at_time(
                        uuid_value(node_id.clone(), agena_domain::TextSegmentId)?,
                        row.try_get::<String>("", "text")?,
                        position,
                        row.try_get("", "revision_seq")?,
                        row.try_get::<Option<i64>>("", "started_at_ms")?
                            .unwrap_or(0),
                    )
                }
                "activity" => {
                    let owner = activity_owner_from_parts(&owner_key.0, &owner_key.1)?;
                    agena_domain::ContentNode::activity(parse_activity_row(&row, owner)?)
                }
                other => {
                    return Err(AppError::Internal(format!(
                        "invalid transcript node type {other}"
                    )));
                }
            };
            positioned_nodes.entry(owner_key).or_default().push((
                u32::try_from(position).unwrap_or(u32::MAX),
                node_id,
                node,
            ));
        }

        for (owner_key, mut nodes) in positioned_nodes {
            nodes.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            documents.insert(
                owner_key,
                agena_domain::ContentDocument::new(
                    nodes.into_iter().map(|(_, _, node)| node).collect(),
                ),
            );
        }
    }
    Ok(documents)
}

fn activity_owner_from_parts(
    owner_kind: &str,
    owner_id: &str,
) -> Result<agena_domain::ActivityOwner, AppError> {
    match owner_kind {
        "turn_input" => Ok(agena_domain::ActivityOwner::TurnInput {
            turn_id: uuid_value(owner_id.to_owned(), agena_domain::TurnId)?,
        }),
        "assistant_reply" => Ok(agena_domain::ActivityOwner::AssistantReply {
            reply_id: uuid_value(owner_id.to_owned(), agena_domain::AssistantReplyId)?,
        }),
        "activity" => Ok(agena_domain::ActivityOwner::Activity {
            parent_activity_id: uuid_value(owner_id.to_owned(), agena_domain::ActivityId)?,
        }),
        "session" => Ok(agena_domain::ActivityOwner::Session {
            session_id: owner_id.parse().map_err(|error| {
                AppError::Internal(format!(
                    "invalid transcript session owner id {owner_id}: {error}"
                ))
            })?,
        }),
        other => Err(AppError::Internal(format!(
            "invalid transcript owner kind {other}"
        ))),
    }
}

fn parse_activity_row(
    row: &sea_orm::QueryResult,
    owner: agena_domain::ActivityOwner,
) -> Result<agena_domain::ActivityNode, AppError> {
    let activity_id: String = row.try_get("", "node_id")?;
    let actor: String = row.try_get("", "actor")?;
    let state: String = row.try_get("", "state")?;
    let position: i64 = row.try_get("", "position")?;
    let mut payload: agena_domain::ActivityPayload =
        serde_json::from_value(row.try_get("", "payload_json")?)?;
    // The durable record stores only the compact tool data. Derive the
    // human-facing detail Markdown here, at snapshot load, so terminals get
    // rich output without the detail ever being persisted or transferred
    // while the Activity stays collapsed.
    if let agena_domain::ActivityPayload::Operation(operation) = &mut payload {
        // The title column holds the freshest running title (updated with a
        // tiny column write during streaming); prefer it over the payload.
        let stored_title: Option<String> = row.try_get("", "title").unwrap_or(None);
        if let Some(stored_title) = stored_title.filter(|title| !title.is_empty()) {
            operation.title = stored_title;
        }
        if !operation.data.is_null() && operation.markdown.is_empty() {
            let tool_name = operation.invocation.name.clone();
            let command = operation
                .invocation
                .input
                .get("command")
                .and_then(|value| value.as_text())
                .map(str::to_owned);
            operation.markdown = crate::session::manager::helpers::derive_operation_markdown(
                &tool_name,
                &operation.data,
                command.as_deref(),
            );
        }
    }
    Ok(agena_domain::ActivityNode {
        id: uuid_value(activity_id, agena_domain::ActivityId)?,
        owner,
        actor: match actor.as_str() {
            "user" => agena_domain::ActivityActor::User,
            "assistant" => agena_domain::ActivityActor::Assistant,
            "runtime" => agena_domain::ActivityActor::Runtime,
            "tool" => agena_domain::ActivityActor::Tool,
            "plugin" => agena_domain::ActivityActor::Plugin,
            other => {
                return Err(AppError::Internal(format!(
                    "invalid transcript activity actor {other}"
                )));
            }
        },
        payload,
        state: match state.as_str() {
            "pending" => agena_domain::ActivityState::Pending,
            "in_progress" => agena_domain::ActivityState::InProgress,
            "completed" => agena_domain::ActivityState::Completed,
            "failed" => agena_domain::ActivityState::Failed,
            "cancelled" => agena_domain::ActivityState::Cancelled,
            other => {
                return Err(AppError::Internal(format!(
                    "invalid transcript activity state {other}"
                )));
            }
        },
        position: agena_domain::ContentPosition {
            index: u32::try_from(position).map_err(|_| {
                AppError::Internal(format!("invalid transcript activity position {position}"))
            })?,
        },
        revision_seq: row.try_get("", "revision_seq")?,
        lifecycle: agena_domain::ActivityLifecycle {
            started_at_ms: row.try_get("", "started_at_ms")?,
            finished_at_ms: row.try_get("", "finished_at_ms")?,
        },
        provenance: Default::default(),
    })
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeEventQueryService for SessionManager {
    async fn list_events(
        &self,
        filter: &agena_domain::EventFilter,
        range: agena_runtime::RuntimeEventRange,
    ) -> Result<Vec<agena_runtime::RuntimeEvent>, agena_runtime::RuntimeEventQueryError> {
        self.publisher
            .store()
            .range(
                filter,
                agena_storage::StoreRange {
                    after_seq_global: range.after_seq_global,
                    limit: range.limit,
                },
            )
            .await
            .map_err(|error| agena_runtime::RuntimeEventQueryError::internal(error.to_string()))?
            .iter()
            .map(project_runtime_event)
            .collect()
    }

    async fn list_events_before(
        &self,
        filter: &agena_domain::EventFilter,
        range: agena_runtime::RuntimeReverseEventRange,
    ) -> Result<Vec<agena_runtime::RuntimeEvent>, agena_runtime::RuntimeEventQueryError> {
        self.publisher
            .store()
            .range_before(
                filter,
                agena_storage::ReverseStoreRange {
                    before_seq_global: range.before_seq_global,
                    limit: range.limit,
                },
            )
            .await
            .map_err(|error| agena_runtime::RuntimeEventQueryError::internal(error.to_string()))?
            .iter()
            .map(project_runtime_event)
            .collect()
    }

    async fn list_timeline_events_before(
        &self,
        filter: &agena_domain::EventFilter,
        range: agena_runtime::RuntimeReverseEventRange,
    ) -> Result<Vec<agena_runtime::RuntimeTimelineEvent>, agena_runtime::RuntimeEventQueryError>
    {
        // Forked/rewind sessions share their parent's terminal history by
        // reference (`agena_session_messages`) instead of owning a physical
        // copy, so the child's own event scope only contains the delta.
        // Merge the parent's events up to the fork cutoff with the child's
        // events so the timeline still shows the whole conversation. Paging
        // into the shared prefix (before_seq_global <= cutoff) is served by
        // the plain query below, which already only reads the child's scope.
        if let agena_domain::EventScope::Session { session_id } = &filter.scope
            && let Some((parent_session_id, cutoff_seq)) = self
                .store
                .history
                .fork_source_cutoff(*session_id)
                .await
                .map_err(|error| {
                    agena_runtime::RuntimeEventQueryError::internal(error.to_string())
                })?
        {
            let parent_in_range = range
                .before_seq_global
                .is_none_or(|before| before > cutoff_seq);
            if parent_in_range {
                let mut merged = self
                    .store
                    .history
                    .list_session_events_before(parent_session_id, cutoff_seq, Some(range.limit))
                    .await
                    .map_err(|error| {
                        agena_runtime::RuntimeEventQueryError::internal(error.to_string())
                    })?;
                merged.retain(|event| {
                    filter
                        .kinds
                        .as_ref()
                        .is_none_or(|kinds| kinds.contains(&event.kind.tag()))
                        && filter
                            .since_seq_global
                            .is_none_or(|since| event.meta.seq_global > since)
                });
                merged.reverse(); // oldest-first -> newest-first
                let mut own_events = self
                    .publisher
                    .store()
                    .range_before(
                        filter,
                        agena_storage::ReverseStoreRange {
                            before_seq_global: range.before_seq_global,
                            limit: range.limit,
                        },
                    )
                    .await
                    .map_err(|error| {
                        agena_runtime::RuntimeEventQueryError::internal(error.to_string())
                    })?;
                merged.append(&mut own_events);
                merged.sort_by_key(|b| std::cmp::Reverse(b.meta.seq_global));
                merged.truncate(range.limit);
                return merged.iter().map(project_runtime_timeline_event).collect();
            }
        }
        self.publisher
            .store()
            .range_before(
                filter,
                agena_storage::ReverseStoreRange {
                    before_seq_global: range.before_seq_global,
                    limit: range.limit,
                },
            )
            .await
            .map_err(|error| agena_runtime::RuntimeEventQueryError::internal(error.to_string()))?
            .iter()
            .map(project_runtime_timeline_event)
            .collect()
    }
}

fn project_runtime_timeline_event(
    event: &crate::event::DomainEvent,
) -> Result<agena_runtime::RuntimeTimelineEvent, agena_runtime::RuntimeEventQueryError> {
    use crate::event::EventKind;

    let type_key = match &event.kind {
        EventKind::ExecutionStarted(_) => "timeline-type-execution-started",
        EventKind::ExecutionFinished(event) => match &event.outcome {
            agena_domain::ExecutionOutcome::Completed => "timeline-type-execution-completed",
            agena_domain::ExecutionOutcome::Cancelled => "timeline-type-execution-cancelled",
            agena_domain::ExecutionOutcome::Failed { .. } => "timeline-type-execution-failed",
        },
        EventKind::CompactionCompleted(_) => "timeline-type-compaction-completed",
        EventKind::SubtaskStatusChanged(_) => "timeline-type-subtask-status-changed",
        EventKind::StreamError(_) => "timeline-type-stream-error",
        EventKind::ProviderRetry(_) => "timeline-type-provider-retry",
        EventKind::ProviderRetryResolved(_) => "timeline-type-provider-retry-resolved",
        EventKind::MessagePartCheckpointed(_) => "timeline-type-message-part-checkpointed",
        EventKind::TranscriptPartUpserted(_) => "timeline-type-transcript-part-upserted",
        EventKind::CommandBegin(_) => "timeline-type-command-begin",
        EventKind::CommandOutputDelta(_) => "timeline-type-command-output-delta",
        EventKind::CommandEnd(_) => "timeline-type-command-end",
        EventKind::PermissionRequested(_) => "timeline-type-permission-requested",
        EventKind::PermissionReplied(_) => "timeline-type-permission-replied",
        EventKind::UserInputRequested(_) => "timeline-type-user-input-requested",
        EventKind::PermissionRuleCreated(_) => "timeline-type-permission-rule-created",
        EventKind::PermissionRuleUpdated(_) => "timeline-type-permission-rule-updated",
        EventKind::PermissionRuleRevoked(_) => "timeline-type-permission-rule-revoked",
        EventKind::ToolPolicyDenied(_) => "timeline-type-tool-policy-denied",
        EventKind::ToolUserDeclined(_) => "timeline-type-tool-user-declined",
        EventKind::RunStarted(_) => "timeline-type-run-started",
        EventKind::RunCompleted(_) => "timeline-type-run-completed",
        EventKind::RunAborted(_) => "timeline-type-run-aborted",
        EventKind::UserMessageAppended(_) => "timeline-type-user-message-appended",
        EventKind::AssistantMessageFinished(_) => "timeline-type-assistant-message-completed",
        EventKind::ToolCallIssued(_) => "timeline-type-tool-call-issued",
        EventKind::ToolCallCompleted(_) => "timeline-type-tool-call-completed",
        EventKind::BackgroundActivityChanged(_) => "timeline-type-background-activity-changed",
        EventKind::PluginEvent(_) | EventKind::PluginToolRegistryChanged(_) => {
            "timeline-type-plugin-event"
        }
        EventKind::ActivityV2(_) => "timeline-type-activity-v2",
    };
    let detail = serde_json::to_string_pretty(&event.kind)
        .map_err(|error| agena_runtime::RuntimeEventQueryError::internal(error.to_string()))?;
    let kind = event.kind.tag_str().to_owned();
    let summary = kind.replace('_', " ");
    Ok(agena_runtime::RuntimeTimelineEvent {
        meta: event.meta.clone(),
        kind: kind.clone(),
        type_key: type_key.to_owned(),
        summary: summary.clone(),
        detail_lines: vec![agena_runtime::RuntimeTimelineDetailLine {
            label: "payload".to_owned(),
            value: detail.clone(),
        }],
        search_text: format!("{kind} {summary} {detail}").to_ascii_lowercase(),
    })
}

fn project_runtime_event(
    event: &crate::event::DomainEvent,
) -> Result<agena_runtime::RuntimeEvent, agena_runtime::RuntimeEventQueryError> {
    let value = serde_json::to_value(event)
        .map_err(|error| agena_runtime::RuntimeEventQueryError::internal(error.to_string()))?;
    let mut object = value.as_object().cloned().ok_or_else(|| {
        agena_runtime::RuntimeEventQueryError::internal("event must serialize as an object")
    })?;
    let kind = object
        .remove("kind")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| agena_runtime::RuntimeEventQueryError::internal("event is missing kind"))?;
    let payload = object.remove("payload").unwrap_or(serde_json::Value::Null);
    let meta = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|error| agena_runtime::RuntimeEventQueryError::internal(error.to_string()))?;
    Ok(agena_runtime::RuntimeEvent {
        meta,
        kind,
        payload,
        invalidates_ancestor_projection: event.kind.invalidates_ancestor_projection(),
    })
}

fn project_runtime_presentation_event(
    event: &crate::event::DomainEvent,
) -> Result<Option<agena_runtime::RuntimePresentationEvent>, agena_runtime::SessionQueryError> {
    use crate::event::EventKind;

    let seq_session = event.meta.seq_session.unwrap_or(event.meta.seq_global);
    let kind = match &event.kind {
        EventKind::MessagePartCheckpointed(update) => {
            if let (Some(turn_id), Some(reply_id)) = (update.turn_id, update.reply_id) {
                transcript_part_patch(
                    seq_session,
                    update.message_role,
                    turn_id,
                    reply_id,
                    &update.part,
                )
                .map(|patch| {
                    agena_runtime::RuntimePresentationEventKind::TranscriptPatch(Box::new(patch))
                })
            } else {
                // Tool-owned parts are checkpointed from the session store,
                // which does not attach the owning turn/reply ids to the
                // event. Without an owner the patch cannot be materialized,
                // so fall back to a refresh: the terminal UI must observe
                // tool output deltas, status changes, and title updates as
                // they are persisted instead of waiting for a later event
                // or a restart.
                Some(agena_runtime::RuntimePresentationEventKind::Refresh {
                    force_refresh: false,
                })
            }
        }
        EventKind::TranscriptPartUpserted(update) => transcript_part_patch(
            seq_session,
            update.message_role,
            update.turn_id,
            update.reply_id,
            &update.part,
        )
        .map(|patch| agena_runtime::RuntimePresentationEventKind::TranscriptPatch(Box::new(patch))),
        EventKind::UserMessageAppended(_) | EventKind::AssistantMessageFinished(_) => {
            Some(agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: false,
            })
        }
        EventKind::ExecutionStarted(_)
        | EventKind::ToolCallCompleted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::CompactionCompleted(_)
        | EventKind::ExecutionFinished(_) => {
            Some(agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: false,
            })
        }
        EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::UserInputRequested(_) => {
            Some(agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: true,
            })
        }
        EventKind::ActivityV2(activity) => Some(
            agena_runtime::RuntimePresentationEventKind::ActivityV2(activity.clone()),
        ),
        EventKind::CommandOutputDelta(_) => {
            // Live streaming detail is carried by EventKind::ActivityV2
            // (DetailDelta) so TUI and Web render the same ViewBlock stream;
            // the legacy OperationDetailDelta projection is removed.
            None
        }
        EventKind::ProviderRetry(update) => {
            // A retryable provider failure observed mid-stream. Project a
            // live retry-progress node for the reply immediately (in memory
            // only, never persisted). The node shares the stable error id with
            // the durable Error activity written by a final failure, so a
            // later refresh replaces it instead of duplicating it; on success
            // the resolved event removes it entirely.
            let reply_id = update.reply_id;
            let node_id = agena_domain::ActivityId::for_reply_error(reply_id);
            let title = format!(
                "Provider request error · retrying ({}/{})",
                update.attempt, update.max_retries
            );
            let node = agena_domain::ContentNode::activity(agena_domain::ActivityNode {
                id: node_id,
                owner: agena_domain::ActivityOwner::AssistantReply { reply_id },
                actor: agena_domain::ActivityActor::Runtime,
                payload: agena_domain::ActivityPayload::Notice(agena_domain::NoticeActivity {
                    kind: "provider_retry".to_owned(),
                    summary: title,
                    detail: Some(update.message.clone()),
                    occurred_at_ms: Some(update.ts_ms),
                    title: None,
                }),
                state: agena_domain::ActivityState::InProgress,
                position: agena_domain::ContentPosition { index: 0 },
                revision_seq: seq_session,
                lifecycle: agena_domain::ActivityLifecycle {
                    started_at_ms: update.ts_ms,
                    finished_at_ms: None,
                },
                provenance: Default::default(),
            });
            Some(
                agena_runtime::RuntimePresentationEventKind::TranscriptPatch(Box::new(
                    agena_domain::TranscriptPatch::ContentUpserted {
                        seq_session,
                        owner: agena_domain::ActivityOwner::AssistantReply { reply_id },
                        node,
                    },
                )),
            )
        }
        EventKind::ProviderRetryResolved(update) if update.succeeded => {
            // The retry sequence ended with a successful reply. The live node
            // was never persisted, so removing it here (and on the follow-up
            // refresh) is what keeps a recovered reply free of the error.
            Some(
                agena_runtime::RuntimePresentationEventKind::TranscriptPatch(Box::new(
                    agena_domain::TranscriptPatch::ContentRemoved {
                        seq_session,
                        owner: agena_domain::ActivityOwner::AssistantReply {
                            reply_id: update.reply_id,
                        },
                        node_id: agena_domain::ActivityId::for_reply_error(update.reply_id),
                    },
                )),
            )
        }
        EventKind::ProviderRetryResolved(_) => None,
        EventKind::BackgroundActivityChanged(event) => Some(
            agena_runtime::RuntimePresentationEventKind::ActivityChanged {
                activity: Box::new(event.activity.clone()),
                reason: event.reason,
            },
        ),
        _ => None,
    };
    Ok(kind.map(|kind| agena_runtime::RuntimePresentationEvent {
        meta: event.meta.clone(),
        invalidates_ancestor_projection: event.kind.invalidates_ancestor_projection(),
        // Live-only kinds consume a global sequence number but are never
        // written to the durable log; consumers reconcile their local
        // high-water mark against the durable watermark, so the flag must
        // mirror `EventKind::is_persistent` exactly.
        durable: event.kind.is_persistent(),
        kind,
    }))
}

fn transcript_part_patch(
    seq_session: i64,
    role: Role,
    turn_id: agena_domain::TurnId,
    reply_id: agena_domain::AssistantReplyId,
    part: &MessagePart,
) -> Option<agena_domain::TranscriptPatch> {
    let (owner, actor) = match role {
        Role::User => (
            agena_domain::ActivityOwner::TurnInput { turn_id },
            agena_domain::ActivityActor::User,
        ),
        Role::Assistant => (
            agena_domain::ActivityOwner::AssistantReply { reply_id },
            agena_domain::ActivityActor::Assistant,
        ),
        Role::Tool => (
            agena_domain::ActivityOwner::AssistantReply { reply_id },
            agena_domain::ActivityActor::Tool,
        ),
        Role::System => (
            agena_domain::ActivityOwner::AssistantReply { reply_id },
            agena_domain::ActivityActor::Runtime,
        ),
    };
    let position = u32::try_from(part.part_index).unwrap_or_default();
    let node = if let Some(activity_id) = part.activity_id {
        let mut payload = crate::session::history::activity_payload(part, role)?;
        // Live transcript patches carry the derived detail Markdown so the
        // terminal renders the human view immediately. The durable projection
        // (activity_payload → content_nodes) deliberately leaves it empty.
        if let agena_domain::ActivityPayload::Operation(operation) = &mut payload
            && operation.markdown.is_empty()
            && !operation.data.is_null()
        {
            let command = operation
                .invocation
                .input
                .get("command")
                .and_then(|value| value.as_text())
                .map(str::to_owned);
            operation.markdown = crate::session::manager::helpers::derive_operation_markdown(
                &operation.invocation.name,
                &operation.data,
                command.as_deref(),
            );
        }
        let state = match part.status {
            ExecutionStatus::Pending => agena_domain::ActivityState::Pending,
            ExecutionStatus::InProgress => agena_domain::ActivityState::InProgress,
            ExecutionStatus::Completed => agena_domain::ActivityState::Completed,
            // ActivityState is intentionally coarse; the transcript part and
            // tool-result envelope preserve the precise non-execution reason.
            ExecutionStatus::PolicyDenied
            | ExecutionStatus::UserDeclined
            | ExecutionStatus::CapabilityUnavailable
            | ExecutionStatus::ToolUnavailable => agena_domain::ActivityState::Completed,
            ExecutionStatus::Failed => agena_domain::ActivityState::Failed,
            ExecutionStatus::Cancelled => agena_domain::ActivityState::Cancelled,
        };
        let finished_at_ms = state
            .is_terminal()
            .then_some(part.created_at.timestamp_millis());
        agena_domain::ContentNode::activity(agena_domain::ActivityNode {
            id: activity_id,
            owner,
            actor,
            payload,
            state,
            position: agena_domain::ContentPosition { index: position },
            revision_seq: seq_session,
            lifecycle: agena_domain::ActivityLifecycle {
                started_at_ms: part.created_at.timestamp_millis(),
                finished_at_ms,
            },
            provenance: Default::default(),
        })
    } else {
        let segment_id = part.segment_id?;
        let PartContent::Text(text) = part.content.as_ref()? else {
            return None;
        };
        agena_domain::ContentNode::text_at(segment_id, text.text.clone(), position, seq_session)
    };
    Some(agena_domain::TranscriptPatch::ContentUpserted {
        seq_session,
        owner,
        node,
    })
}

struct RuntimeLiveEventSubscriptionAdapter {
    inner: crate::event::Subscription<crate::event::EventKind>,
}

struct RuntimeLivePresentationSubscriptionAdapter {
    inner: crate::event::Subscription<crate::event::EventKind>,
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeLiveEventSubscription for RuntimeLiveEventSubscriptionAdapter {
    async fn recv(&mut self) -> Option<agena_runtime::RuntimeLiveEventSubscriptionItem> {
        use crate::event::bus::SubscriptionItem;
        match self.inner.recv().await {
            Some(SubscriptionItem::Event(event)) => project_runtime_event(event.as_ref())
                .map(agena_runtime::RuntimeLiveEventSubscriptionItem::Event)
                .ok(),
            Some(SubscriptionItem::Lagged(skipped)) => Some(
                agena_runtime::RuntimeLiveEventSubscriptionItem::Lagged(skipped),
            ),
            None => None,
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeLivePresentationSubscription
    for RuntimeLivePresentationSubscriptionAdapter
{
    async fn recv(&mut self) -> Option<agena_runtime::RuntimeLivePresentationSubscriptionItem> {
        use crate::event::bus::SubscriptionItem;
        loop {
            match self.inner.recv().await {
                Some(SubscriptionItem::Event(event)) => {
                    match project_runtime_presentation_event(event.as_ref()) {
                        Ok(Some(event)) => {
                            return Some(
                                agena_runtime::RuntimeLivePresentationSubscriptionItem::Event(
                                    Box::new(event),
                                ),
                            );
                        }
                        Ok(None) => continue,
                        Err(_) => return None,
                    }
                }
                Some(SubscriptionItem::Lagged(skipped)) => {
                    return Some(
                        agena_runtime::RuntimeLivePresentationSubscriptionItem::Lagged(skipped),
                    );
                }
                None => return None,
            }
        }
    }
}

impl agena_runtime::RuntimeEventStreamService for SessionManager {
    fn subscribe_events(
        &self,
        filter: agena_domain::EventFilter,
    ) -> Box<dyn agena_runtime::RuntimeLiveEventSubscription> {
        Box::new(RuntimeLiveEventSubscriptionAdapter {
            inner: self.bus.subscribe(filter),
        })
    }

    fn subscribe_presentation_events(
        &self,
        filter: agena_domain::EventFilter,
    ) -> Option<Box<dyn agena_runtime::RuntimeLivePresentationSubscription>> {
        Some(Box::new(RuntimeLivePresentationSubscriptionAdapter {
            inner: self.bus.subscribe(filter),
        }))
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeEventPublishService for SessionManager {
    async fn publish_event(
        &self,
        request: agena_runtime::RuntimeEventPublishRequest,
    ) -> Result<(), agena_runtime::RuntimeEventPublishError> {
        let kind = match request {
            agena_runtime::RuntimeEventPublishRequest::PermissionRuleCreated(event) => {
                crate::event::EventKind::PermissionRuleCreated(event)
            }
            agena_runtime::RuntimeEventPublishRequest::PermissionRuleUpdated(event) => {
                crate::event::EventKind::PermissionRuleUpdated(event)
            }
            agena_runtime::RuntimeEventPublishRequest::PermissionRuleRevoked(event) => {
                crate::event::EventKind::PermissionRuleRevoked(event)
            }
            agena_runtime::RuntimeEventPublishRequest::PluginEvent {
                plugin_id,
                kind_label,
                payload,
            } => crate::event::EventKind::PluginEvent(crate::event::PluginEventPayload {
                plugin_id,
                kind_label,
                payload,
            }),
        };
        self.publisher
            .publish(crate::event::PublishContext::default(), kind)
            .await
            .map(|_| ())
            .map_err(|error| agena_runtime::RuntimeEventPublishError::internal(error.to_string()))
    }
}

impl SessionManager {
    pub async fn fork_session(&self, request: SessionForkRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let source = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let title = request
            .title
            .unwrap_or_else(|| format!("Fork of {}", source.title));
        self.store
            .fork_session(source, request.at_message_id, title, state.cache_policy())
            .await
    }

    /// External entry: cancel the active execution for `session_id` and every
    /// active descendant. Descendants are cancelled deepest-first so a parent
    /// waiting on a delegated tool cannot keep its child alive.
    ///
    /// Cancellation is idempotent: a task can complete between the UI
    /// deciding to cancel and this call reaching the manager, so the absence
    /// of a control is a successful no-op rather than an error.
    pub async fn cancel_active_execution(&self, session_id: i64) -> Result<(), AppError> {
        let state = self.execution_state();
        // Signal the requested execution before any database traversal. This
        // keeps Ctrl+C latency independent of session-tree size and storage
        // contention; descendant discovery continues after the active model
        // stream or tool has already received cancellation.
        let root_result = self.execution_registry.cancel_current(session_id).await;
        self.cancel_host_interactive_waiters(session_id).await;
        let cancellation_order = match self
            .store
            .load_session(session_id, state.cache_policy())
            .await
        {
            Ok(session) => {
                let tree = self.store.list_session_tree(session.root_id).await?;
                descendant_cancellation_order(session_id, tree.as_slice())
            }
            Err(_) => vec![session_id],
        };

        let mut first_error = cancel_active_execution_result(root_result).err();
        for target_id in cancellation_order
            .into_iter()
            .filter(|target_id| *target_id != session_id)
        {
            let result = self.execution_registry.cancel_current(target_id).await;
            // A plugin-hosted tool can be suspended in a host permission or
            // user-input callback. A cancellation token is only observed
            // between run-loop iterations, so release those one-shot waiters
            // as well; otherwise Ctrl+C leaves the executor blocked forever.
            self.cancel_host_interactive_waiters(target_id).await;
            if let Err(error) = cancel_active_execution_result(result)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Exact external cancellation. Only after the observed root execution is
    /// matched do we cascade to its active descendants.
    pub async fn cancel_execution(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult, AppError> {
        let result = self
            .execution_registry
            .cancel_exact(session_id, execution_id)
            .await
            .map_err(execution_control_to_app_error)?;
        if result != agena_domain::CancellationResult::CancellationRequested {
            return Ok(result);
        }
        self.cancel_host_interactive_waiters(session_id).await;

        let state = self.execution_state();
        if let Ok(session) = self
            .store
            .load_session(session_id, state.cache_policy())
            .await
        {
            let tree = self.store.list_session_tree(session.root_id).await?;
            for target_id in descendant_cancellation_order(session_id, tree.as_slice())
                .into_iter()
                .filter(|target_id| *target_id != session_id)
            {
                let _ = self.execution_registry.cancel_current(target_id).await;
                self.cancel_host_interactive_waiters(target_id).await;
            }
        }
        Ok(result)
    }

    /// External entry: inject `parts` as a steer message into the active
    /// execution for `session_id`. Returns `Err` if no execution is active or the
    /// channel was closed.
    pub async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), AppError> {
        self.execution_registry
            .steer(session_id, parts)
            .await
            .map_err(execution_control_to_app_error)
    }

    pub async fn rewind_session(&self, request: SessionRewindRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let source = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let message_id = self
            .model_user_message_id_for_turn(request.session_id, request.turn_id)
            .await?;
        if !is_completed_user_rewind_target(
            source
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "canonical turn {} has no projected user message in session {}",
                        request.turn_id, source.id
                    ))
                })?,
        ) {
            return Err(AppError::Internal(format!(
                "rewind target must be a completed canonical user turn: {}",
                request.turn_id
            )));
        }
        let title = format!("Rewind of {}", source.title);
        self.store
            .fork_session_before_message(source, message_id, title, state.cache_policy())
            .await
    }

    async fn model_user_message_id_for_turn(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
    ) -> Result<i64, AppError> {
        let row = self
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                self.store.db.get_database_backend(),
                "SELECT m.message_id \
                 FROM agena_turns t \
                 JOIN agena_assistant_replies r ON r.turn_id = t.turn_id \
                 JOIN agena_reply_executions e ON e.reply_id = r.reply_id AND e.source = 'user' \
                 JOIN agena_model_messages m ON m.execution_id = e.execution_id \
                 WHERE t.session_id = ? AND t.turn_id = ? AND m.role = 1 \
                 ORDER BY m.created_at_ms, m.message_id",
                [session_id.into(), turn_id.to_string().into()],
            ))
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "canonical turn not found in session {session_id}: {turn_id}"
                ))
            })?;
        Ok(row.try_get("", "message_id")?)
    }

    /// Serialise `session_id` as a JSONL bundle. The first line is the
    /// session header (id, parent, depth, runtime); subsequent lines are
    /// persistent event payloads in `seq_global` order.
    pub async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        self.store.export_session_jsonl(session_id).await
    }

    /// Replay a JSONL bundle produced by [`Self::export_session_jsonl`] into
    /// this manager's workspace as a fresh session.
    pub async fn import_session_jsonl(&self, bundle: &str) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .import_session_jsonl(bundle, state.cache_policy())
            .await
    }

    /// Return every session that shares the same `root_id`, ordered by
    /// `(depth, id)`. Useful for tree visualisation and bulk export.
    pub async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, AppError> {
        self.store.list_session_tree(root_id).await
    }

    pub async fn transcript_snapshot(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::TranscriptSnapshot, AppError> {
        let rows = self
            .store
            .db
            .query_all(Statement::from_sql_and_values(
                self.store.db.get_database_backend(),
                "SELECT t.turn_id, t.turn_seq, t.created_at_ms AS turn_created_at_ms, \
                        r.reply_id, r.status, r.revision_seq, \
                        r.created_at_ms AS reply_created_at_ms, r.finished_at_ms, \
                        r.failure_json \
                 FROM agena_turns t \
                 JOIN agena_assistant_replies r ON r.turn_id = t.turn_id \
                 WHERE t.session_id = ? ORDER BY t.turn_seq, r.created_at_ms",
                [session_id.into()],
            ))
            .await?;
        let owners = rows
            .iter()
            .flat_map(|row| {
                let turn_id = row.try_get::<String>("", "turn_id").unwrap_or_default();
                let reply_id = row.try_get::<String>("", "reply_id").unwrap_or_default();
                [
                    ("turn_input".to_owned(), turn_id),
                    ("assistant_reply".to_owned(), reply_id),
                ]
            })
            .collect::<Vec<_>>();
        let documents = transcript_documents_batch(&self.store.db, &owners).await?;
        let mut turns = Vec::with_capacity(rows.len());
        for row in rows {
            let turn_id = uuid_value(row.try_get("", "turn_id")?, agena_domain::TurnId)?;
            let reply_id =
                uuid_value(row.try_get("", "reply_id")?, agena_domain::AssistantReplyId)?;
            let status_text: String = row.try_get("", "status")?;
            let status = match status_text.as_str() {
                "pending" => agena_domain::AssistantReplyStatus::Pending,
                "in_progress" => agena_domain::AssistantReplyStatus::InProgress,
                "completed" => agena_domain::AssistantReplyStatus::Completed,
                "failed" => agena_domain::AssistantReplyStatus::Failed,
                "cancelled" => agena_domain::AssistantReplyStatus::Cancelled,
                value => {
                    return Err(AppError::Internal(format!(
                        "invalid assistant reply status {value}"
                    )));
                }
            };
            let input = documents
                .get(&("turn_input".to_owned(), turn_id.to_string()))
                .cloned()
                .unwrap_or_default();
            let reply_content = documents
                .get(&("assistant_reply".to_owned(), reply_id.to_string()))
                .cloned()
                .unwrap_or_default();
            turns.push(agena_domain::TurnSnapshot {
                id: turn_id,
                session_id,
                sequence: row.try_get("", "turn_seq")?,
                input,
                reply: agena_domain::AssistantReplySnapshot {
                    id: reply_id,
                    turn_id,
                    status,
                    content: reply_content,
                    revision_seq: row.try_get("", "revision_seq")?,
                    created_at_ms: row.try_get("", "reply_created_at_ms")?,
                    finished_at_ms: row.try_get("", "finished_at_ms")?,
                    failure: row
                        .try_get::<Option<serde_json::Value>>("", "failure_json")?
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|error| {
                            AppError::Internal(format!(
                                "invalid assistant reply failure projection: {error}"
                            ))
                        })?,
                },
                created_at_ms: row.try_get("", "turn_created_at_ms")?,
            });
        }
        let seq_session = self
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                self.store.db.get_database_backend(),
                "SELECT COALESCE(MAX(seq_session), 0) AS seq_session FROM agena_events WHERE session_id = ?",
                [session_id.into()],
            ))
            .await?
            .map(|row| row.try_get("", "seq_session"))
            .transpose()?
            .unwrap_or_default();
        let session_activities = self
            .store
            .db
            .query_all(Statement::from_sql_and_values(
                self.store.db.get_database_backend(),
                                "SELECT node_id, actor, payload_json, state, position, revision_seq, started_at_ms, finished_at_ms \
                 FROM agena_content_nodes WHERE owner_kind = 'session' AND owner_id = ? \
                 ORDER BY position, node_id",
                [session_id.to_string().into()],
            ))
            .await?
            .into_iter()
            .map(|row| {
                parse_activity_row(
                    &row,
                    agena_domain::ActivityOwner::Session { session_id },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(agena_domain::TranscriptSnapshot {
            session_id,
            seq_session,
            turns,
            session_activities,
        })
    }
}

#[async_trait::async_trait]
impl agena_runtime::SessionQueryService for SessionManager {
    async fn find_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        SessionManager::find_session_id_for_message(self, message_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn find_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        SessionManager::find_session_id_for_part(self, part_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn list_session_summaries(
        &self,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<agena_domain::SessionSummary>, agena_runtime::SessionQueryError> {
        self.list_session_summaries(request)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn session_presentation(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionPresentation, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let workflow_state = session.workflow_state();
        Ok(agena_runtime::SessionPresentation {
            id: session.id,
            parent_id: session.parent_id,
            workspace_id: session.workspace_id,
            title: session.title,
            version: session.version,
            created_at: session.created_at,
            updated_at: session.updated_at,
            message_count: session.messages.len(),
            workflow_state,
        })
    }

    async fn transcript_snapshot(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::TranscriptSnapshot, agena_runtime::SessionQueryError> {
        SessionManager::transcript_snapshot(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn operation_detail(
        &self,
        session_id: i64,
        activity_id: agena_domain::ActivityId,
    ) -> Result<Option<agena_runtime::OperationDetail>, agena_runtime::SessionQueryError> {
        // Load the snapshot (which derives detail Markdown into Operation
        // Activities on load) and locate the requested Activity.
        let snapshot = SessionManager::transcript_snapshot(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let mut found: Option<agena_domain::ActivityNode> = None;
        for turn in &snapshot.turns {
            for node in turn.input.nodes() {
                if let agena_domain::ContentNode::Activity { activity } = node
                    && activity.id == activity_id
                {
                    found = Some(activity.as_ref().clone());
                }
            }
            for node in turn.reply.content.nodes() {
                if let agena_domain::ContentNode::Activity { activity } = node
                    && activity.id == activity_id
                {
                    found = Some(activity.as_ref().clone());
                }
            }
        }
        for activity in &snapshot.session_activities {
            if activity.id == activity_id {
                found = Some(activity.clone());
            }
        }
        let activity = match found {
            Some(activity) => activity,
            None => return Ok(None),
        };
        let agena_domain::ActivityPayload::Operation(operation) = &activity.payload else {
            return Ok(None);
        };
        // If the snapshot did not pre-derive the detail (e.g. a live in-memory
        // node), derive it now from the compact data.
        let markdown = if !operation.markdown.is_empty() {
            operation.markdown.clone()
        } else if !operation.data.is_null() {
            let command = operation
                .invocation
                .input
                .get("command")
                .and_then(|value| value.as_text())
                .map(str::to_owned);
            crate::session::manager::helpers::derive_operation_markdown(
                &operation.invocation.name,
                &operation.data,
                command.as_deref(),
            )
        } else {
            String::new()
        };
        let streaming = activity.state == agena_domain::ActivityState::InProgress;
        Ok(Some(agena_runtime::OperationDetail {
            activity_id,
            markdown,
            streaming,
        }))
    }

    async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_runtime::SessionProjectedMessageHeader>, agena_runtime::SessionQueryError>
    {
        SessionManager::list_projected_message_headers(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?
            .into_iter()
            .map(|header| {
                Ok(agena_runtime::SessionProjectedMessageHeader {
                    id: header.id,
                    role: header.role,
                    state: header.state,
                    created_at: header.created_at,
                    metadata: serde_json::to_value(header.metadata).map_err(|error| {
                        agena_runtime::SessionQueryError::internal(error.to_string())
                    })?,
                    usage: header
                        .usage
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            agena_runtime::SessionQueryError::internal(error.to_string())
                        })?,
                    part_count: header.part_count,
                })
            })
            .collect()
    }

    async fn list_projected_messages(
        &self,
        session_id: i64,
        include_content: bool,
    ) -> Result<Vec<agena_runtime::SessionProjectedMessage>, agena_runtime::SessionQueryError> {
        SessionManager::list_projected_messages(self, session_id, include_content)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?
            .into_iter()
            .map(|message| {
                Ok(agena_runtime::SessionProjectedMessage {
                    id: message.id,
                    role: message.role,
                    state: message.state,
                    created_at: message.created_at,
                    metadata: serde_json::to_value(message.metadata).map_err(|error| {
                        agena_runtime::SessionQueryError::internal(error.to_string())
                    })?,
                    usage: message
                        .usage
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            agena_runtime::SessionQueryError::internal(error.to_string())
                        })?,
                    parts: message
                        .parts
                        .into_iter()
                        .map(project_message_part)
                        .collect::<Result<Vec<_>, agena_runtime::SessionQueryError>>()?,
                })
            })
            .collect()
    }

    async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, agena_runtime::SessionQueryError> {
        SessionManager::list_session_tree(self, root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn export_session_jsonl(
        &self,
        session_id: i64,
    ) -> Result<String, agena_runtime::SessionQueryError> {
        SessionManager::export_session_jsonl(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn latest_event_seq(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        let events = SessionManager::list_session_events(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        Ok(events.iter().map(|event| event.meta.seq_global).max())
    }

    async fn session_usage(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionUsage, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        SessionManager::session_usage(self, &session)
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionCostSummary, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        Ok(crate::session::cost::summarize(&session.messages))
    }

    async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, agena_runtime::SessionQueryError> {
        SessionManager::usage_stats(self, query)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn pending_interactive_requests(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_domain::PendingInteractiveRequestContext>, agena_runtime::SessionQueryError>
    {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let tree = SessionManager::list_session_tree(self, session.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let mut descendants = std::collections::HashSet::from([session.id]);
        loop {
            let previous_len = descendants.len();
            for summary in &tree {
                if summary
                    .parent_id
                    .is_some_and(|parent_id| descendants.contains(&parent_id))
                {
                    descendants.insert(summary.id);
                }
            }
            if descendants.len() == previous_len {
                break;
            }
        }

        let mut sessions = vec![session];
        for summary in tree {
            if summary.id == session_id
                || !descendants.contains(&summary.id)
                || SessionManager::active_execution(self, summary.id)
                    .await
                    .is_none()
            {
                continue;
            }
            sessions.push(
                SessionManager::get_session(self, summary.id)
                    .await
                    .map_err(|error| {
                        agena_runtime::SessionQueryError::internal(error.to_string())
                    })?,
            );
        }

        Ok(sessions
            .into_iter()
            .flat_map(|pending_session| {
                let session_id = pending_session.id;
                let parent_session_id = pending_session.parent_id;
                let task_id = pending_session.task_id.clone();
                pending_session
                    .pending_interactive_requests()
                    .into_iter()
                    .map(
                        move |request| agena_domain::PendingInteractiveRequestContext {
                            session_id,
                            parent_session_id,
                            task_id: task_id.clone(),
                            request,
                        },
                    )
            })
            .collect())
    }

    async fn execution_context(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionExecutionContext, agena_runtime::SessionQueryError> {
        let mut session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let state = self.execution_state();
        self.refresh_execution_policy(&mut session, state.as_ref());
        let runtime = session.runtime();
        Ok(agena_runtime::SessionExecutionContext {
            workflow_state: session.workflow_state(),
            agent_id: crate::identity::AGENA_AGENT_ID.to_string(),
            execution_access: runtime.execution.access,
            selected_permission: runtime.execution.selection.permission.clone(),
            effective_permission: runtime.execution.effective_permission.clone(),
            permission_ceiling: runtime.execution.permission_ceiling.clone(),
            model_provider_id: runtime.execution.selection.provider.clone(),
            model_adapter_id: runtime.execution.selection.adapter.clone(),
            model_id: runtime.execution.selection.model.clone(),
            model_thinking_mode: runtime.execution.selection.thinking_mode.clone(),
            model_speed_mode: runtime.execution.selection.speed_mode.clone(),
            model_verbosity: runtime.execution.selection.verbosity.clone(),
            model_parallel_tool_calls: runtime.execution.selection.parallel_tool_calls,
            effective_workspace_root: runtime
                .effective_workspace_root()
                .map(|path| path.display().to_string()),
            task_id: session.task_id.clone(),
            subtask_status: session.is_subagent().then_some(runtime.subtask.status),
            subtask_started_at: runtime
                .subtask
                .started_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis),
            subtask_finished_at: runtime
                .subtask
                .finished_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis),
            subtask_failure: runtime.subtask.failure.clone(),
        })
    }

    async fn is_descendant_session(
        &self,
        descendant_id: i64,
        ancestor_id: i64,
    ) -> Result<bool, agena_runtime::SessionQueryError> {
        let descendant = SessionManager::get_session(self, descendant_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let tree = SessionManager::list_session_tree(self, descendant.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let parents = tree
            .into_iter()
            .map(|summary| (summary.id, summary.parent_id))
            .collect::<std::collections::HashMap<_, _>>();
        let mut cursor = parents.get(&descendant_id).copied().flatten();
        let mut visited = std::collections::HashSet::new();
        while let Some(session_id) = cursor {
            if !visited.insert(session_id) {
                return Ok(false);
            }
            if session_id == ancestor_id {
                return Ok(true);
            }
            cursor = parents.get(&session_id).copied().flatten();
        }
        Ok(false)
    }
}

/// Projects one private Runtime message part into the stable transcript value.
///
/// Live event consumers use this adapter before crossing into API/TUI
/// presentation mappings; they never need an Application → Runtime-internal
/// dependency.
fn project_message_part(
    part: MessagePart,
) -> Result<agena_runtime::SessionProjectedMessagePart, agena_runtime::SessionQueryError> {
    Ok(agena_runtime::SessionProjectedMessagePart {
        id: part.id,
        message_id: part.message_id,
        part_index: part.part_index,
        status: part.status,
        kind: part.kind,
        name: part.name,
        summary: part.summary,
        has_detail: part.has_detail,
        activity_id: part.activity_id,
        segment_id: part.segment_id,
        operation_id: part.operation_id,
        created_at: part.created_at,
        detail: part.content.as_ref().map(project_part_detail),
        content: part
            .content
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?,
    })
}

fn project_part_detail(content: &PartContent) -> agena_runtime::SessionProjectedPartDetail {
    match content {
        PartContent::Text(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.text.clone(),
            synthetic: value.synthetic,
        },
        PartContent::Activity(crate::message::RuntimeActivity::Reasoning(value)) => {
            agena_runtime::SessionProjectedPartDetail::Reasoning {
                summary: value.summary.clone(),
                raw_content: value.raw_content.clone(),
                encrypted_content: value.encrypted_content.clone(),
            }
        }
        PartContent::Activity(crate::message::RuntimeActivity::Error(value)) => {
            agena_runtime::SessionProjectedPartDetail::Error {
                problem: value.problem.clone(),
            }
        }
        PartContent::Activity(crate::message::RuntimeActivity::Resource(value)) => {
            agena_runtime::SessionProjectedPartDetail::Attachment(value.clone())
        }
        PartContent::Activity(crate::message::RuntimeActivity::SkillReference(value)) => {
            agena_runtime::SessionProjectedPartDetail::SkillReference(value.clone())
        }
        PartContent::Activity(crate::message::RuntimeActivity::Interaction(
            crate::message::RequestPart::UserInput(value),
        )) => agena_runtime::SessionProjectedPartDetail::UserInputRequest {
            request: value.request.clone(),
            reply: value.reply.clone(),
        },
        PartContent::Activity(crate::message::RuntimeActivity::Operation(value)) => {
            agena_runtime::SessionProjectedPartDetail::Operation(Box::new(project_operation_part(
                value,
            )))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Hook(value)) => {
            agena_runtime::SessionProjectedPartDetail::Hook(Box::new(
                agena_runtime::SessionProjectedHookPart {
                    hook: value.hook.clone(),
                    plugin_id: value.plugin_id.clone(),
                    summary: value.summary.clone(),
                    detail: value.detail.clone(),
                },
            ))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Notice(value)) => {
            agena_runtime::SessionProjectedPartDetail::Notice {
                summary: value.summary.clone(),
                detail: value.detail.clone(),
            }
        }
    }
}

fn project_operation_part(
    value: &crate::message::OperationPart,
) -> agena_runtime::SessionProjectedOperationPart {
    let details = value.details.clone();
    agena_runtime::SessionProjectedOperationPart {
        call_id: value.call_id,
        invocation: value.invocation.clone(),
        authorization: value.authorization.clone(),
        title: value.title.clone(),
        summary: value.summary.clone(),
        model_output: project_model_visible_output(&value.result.model_preview),
        blocks: value
            .result
            .content
            .iter()
            .map(project_operation_block)
            .collect(),
        artifacts: value.artifacts.clone(),
        attachments: value.result.attachments.clone(),
        details,
        result: agena_runtime::SessionProjectedToolResult {
            state: value.result.state,
            structured: value.result.structured.clone(),
            content: value
                .result
                .content
                .iter()
                .map(project_operation_block)
                .collect(),
            model_preview: project_model_visible_output(&value.result.model_preview),
            managed_outputs: value.result.managed_outputs.clone(),
            display: value.result.display.clone(),
            attachments: value.result.attachments.clone(),
            error: value.result.error.clone(),
            metadata: value.result.metadata.clone(),
            raw: value.result.raw.clone(),
        },
        structured: value.result.structured.clone(),
        metadata: value.metadata.clone(),
        error: value.error.clone(),
        raw: value.raw.clone(),
        lifecycle: value.lifecycle.clone(),
    }
}

fn project_model_visible_output(
    value: &crate::message::ModelVisibleOutput,
) -> agena_runtime::SessionProjectedModelVisibleOutput {
    agena_runtime::SessionProjectedModelVisibleOutput {
        text: value.text.clone(),
        attachments: value.attachments.clone(),
        truncated: value.truncated,
    }
}

fn project_operation_block(
    value: &agena_domain::ViewBlock,
) -> agena_runtime::SessionProjectedOperationBlock {
    use agena_runtime::SessionProjectedOperationBlock as Projected;
    match value {
        agena_domain::ViewBlock::Text { text, .. } => Projected::Text { text: text.clone() },
        agena_domain::ViewBlock::Markdown { text, .. } => {
            Projected::Markdown { text: text.clone() }
        }
        agena_domain::ViewBlock::Json { value, .. } => Projected::Json {
            value: value.clone(),
        },
        agena_domain::ViewBlock::Table { columns, rows, .. } => Projected::Table {
            columns: columns
                .iter()
                .map(|label| agena_domain::TableColumn {
                    key: label.clone(),
                    label: Some(label.clone()),
                })
                .collect(),
            rows: rows.clone(),
        },
        agena_domain::ViewBlock::Log { stream, text, .. } => Projected::Log {
            stream: Some(match stream {
                agena_domain::CommandOutputStream::Stdout => "stdout".to_string(),
                agena_domain::CommandOutputStream::Stderr => "stderr".to_string(),
            }),
            text: text.clone(),
        },
        agena_domain::ViewBlock::Command {
            command,
            cwd,
            exit_code,
            stdout,
            stderr,
            ..
        } => Projected::Command {
            command: command.clone(),
            cwd: cwd.clone(),
            exit_code: *exit_code,
            stdout: Some(stdout.clone()),
            stderr: Some(stderr.clone()),
        },
        agena_domain::ViewBlock::Diff { diff, language, .. } => Projected::Diff {
            diff: diff.clone(),
            language: language.clone(),
        },
        agena_domain::ViewBlock::FileChanges { changes, .. } => Projected::FileChanges {
            changes: changes.clone(),
        },
        agena_domain::ViewBlock::SearchResults { items, .. } => Projected::SearchResults {
            query: None,
            results: items
                .iter()
                .map(|item| agena_domain::SearchResultItem {
                    title: item.title.clone(),
                    uri: item.url.clone(),
                    snippet: item.snippet.clone(),
                    score: None,
                })
                .collect(),
        },
        agena_domain::ViewBlock::Media { artifact, .. } => Projected::Media {
            mime_type: artifact.mime.clone(),
            artifact: artifact.clone(),
        },
        agena_domain::ViewBlock::Custom {
            kind,
            schema,
            presentation,
            ..
        } => Projected::Custom {
            schema: if schema.is_null() {
                None
            } else {
                Some(schema.to_string())
            },
            value: serde_json::json!({ "kind": kind, "presentation": presentation }),
        },
    }
}

fn descendant_cancellation_order(session_id: i64, tree: &[SessionSummary]) -> Vec<i64> {
    let mut included = std::collections::HashSet::from([session_id]);
    loop {
        let previous_len = included.len();
        for summary in tree {
            if summary
                .parent_id
                .is_some_and(|parent_id| included.contains(&parent_id))
            {
                included.insert(summary.id);
            }
        }
        if included.len() == previous_len {
            break;
        }
    }

    let mut descendants = tree
        .iter()
        .filter(|summary| included.contains(&summary.id))
        .map(|summary| (summary.depth, summary.id))
        .collect::<Vec<_>>();
    if !descendants.iter().any(|(_, id)| *id == session_id) {
        descendants.push((i64::MIN, session_id));
    }
    descendants.sort_by(|left, right| right.cmp(left));
    descendants.into_iter().map(|(_, id)| id).collect()
}

fn cancel_active_execution_result(
    result: Result<(), ExecutionControlError>,
) -> Result<(), AppError> {
    match result {
        Ok(()) | Err(ExecutionControlError::NoActiveExecution(_)) => Ok(()),
        Err(error) => Err(execution_control_to_app_error(error)),
    }
}

fn is_completed_user_rewind_target(message: &Message) -> bool {
    message.role == Role::User && message.state == ExecutionStatus::Completed
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionControlError, Message, Role, cancel_active_execution_result,
        descendant_cancellation_order, is_completed_user_rewind_target,
        project_runtime_presentation_event,
    };
    use agena_domain::SessionSummary;
    use agena_domain::{ExecutionStatus, SubtaskStatus};

    #[test]
    fn cancelling_a_completed_run_is_a_successful_no_op() {
        assert!(
            cancel_active_execution_result(Err(ExecutionControlError::NoActiveExecution(42)))
                .is_ok()
        );
        assert!(cancel_active_execution_result(Err(ExecutionControlError::SteerClosed)).is_err());
    }

    #[test]
    fn execution_start_immediately_invalidates_the_transcript_projection() {
        let session_id = 42;
        let event = crate::event::DomainEvent {
            meta: agena_domain::EventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: 1,
                seq_session: Some(1),
                session_id: Some(session_id),
                workspace_id: Some(1),
                created_at: chrono::Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: agena_domain::EVENT_ENVELOPE_SCHEMA_VERSION,
            },
            kind: crate::event::EventKind::ExecutionStarted(agena_domain::ExecutionStartedEvent {
                session_id,
                execution_id: agena_domain::ExecutionId::new(),
                turn_id: agena_domain::TurnId::new(),
                reply_id: agena_domain::AssistantReplyId::new(),
                source: agena_domain::ExecutionSource::Compaction,
                ts_ms: 1,
            }),
        };

        let projected = project_runtime_presentation_event(&event)
            .expect("presentation projection")
            .expect("execution start must be visible");
        assert!(matches!(
            projected.kind,
            agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: false
            }
        ));
    }

    #[test]
    fn user_input_request_forces_a_presentation_refresh() {
        // Regression guard: a durable UserInputRequested event must project to
        // a force refresh so every UI re-reads the execution snapshot and pops
        // the pending approval/question modal. The part checkpoint alone only
        // projects to a TranscriptPatch, which terminals apply without
        // refreshing the execution (and therefore without seeing the request).
        let session_id = 42;
        let event = crate::event::DomainEvent {
            meta: agena_domain::EventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: 1,
                seq_session: Some(1),
                session_id: Some(session_id),
                workspace_id: Some(1),
                created_at: chrono::Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: agena_domain::EVENT_ENVELOPE_SCHEMA_VERSION,
            },
            kind: crate::event::EventKind::UserInputRequested(
                agena_domain::UserInputRequestedEvent {
                    session_id,
                    operation_id: "op-1".to_string(),
                    call_id: 7,
                    request_id: "req-1".to_string(),
                    ts_ms: 1,
                },
            ),
        };

        let projected = project_runtime_presentation_event(&event)
            .expect("presentation projection")
            .expect("user input request must be visible");
        assert!(matches!(
            projected.kind,
            agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: true
            }
        ));
    }

    #[test]
    fn presentation_events_carry_the_durable_flag_of_the_source_kind() {
        // The TUI reconciles its local high-water mark against the server's
        // durable `latest_event_seq`. Live-only kinds (ActivityV2, streamed
        // text upserts) consume a global sequence number but are never
        // persisted, so the projection must mark them non-durable or the TUI
        // watermark races ahead of the durable log and drops the terminal
        // execution.
        let event = crate::event::DomainEvent {
            meta: agena_domain::EventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: 1,
                seq_session: Some(1),
                session_id: Some(42),
                workspace_id: Some(1),
                created_at: chrono::Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: agena_domain::EVENT_ENVELOPE_SCHEMA_VERSION,
            },
            kind: crate::event::EventKind::ExecutionStarted(agena_domain::ExecutionStartedEvent {
                session_id: 42,
                execution_id: agena_domain::ExecutionId::new(),
                turn_id: agena_domain::TurnId::new(),
                reply_id: agena_domain::AssistantReplyId::new(),
                source: agena_domain::ExecutionSource::User,
                ts_ms: 1,
            }),
        };
        let durable = project_runtime_presentation_event(&event)
            .expect("projection")
            .expect("execution start projects");
        assert!(durable.durable, "ExecutionStarted is durable");

        let live = crate::event::DomainEvent {
            meta: agena_domain::EventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: 2,
                seq_session: Some(2),
                session_id: Some(42),
                workspace_id: Some(1),
                created_at: chrono::Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: agena_domain::EVENT_ENVELOPE_SCHEMA_VERSION,
            },
            kind: crate::event::EventKind::ActivityV2(Box::new(
                crate::activity::ActivityLiveEvent::StateChanged {
                    activity_id: agena_domain::ActivityId::new(),
                    state: agena_domain::ActivityState::Completed,
                },
            )),
        };
        let live_projected = project_runtime_presentation_event(&live)
            .expect("projection")
            .expect("activity v2 projects");
        assert!(
            !live_projected.durable,
            "ActivityV2 is live-only and must be marked non-durable"
        );
    }

    #[test]
    fn rewind_accepts_only_completed_user_messages() {
        let mut user = Message::prompt_text(Role::User, "undo this");
        user.state = ExecutionStatus::Completed;
        let mut assistant = Message::prompt_text(Role::Assistant, "response");
        assistant.state = ExecutionStatus::Completed;
        let mut pending_user = Message::prompt_text(Role::User, "pending");
        pending_user.state = ExecutionStatus::Pending;

        assert!(is_completed_user_rewind_target(&user));
        assert!(!is_completed_user_rewind_target(&assistant));
        assert!(!is_completed_user_rewind_target(&pending_user));
    }

    #[test]
    fn cancellation_orders_descendants_deepest_first() {
        let now = chrono::Utc::now();
        let summary = |id, parent_id, depth| SessionSummary {
            id,
            parent_id,
            depth,
            root_id: 1,
            workspace_id: 1,
            title: id.to_string(),
            version: 1,
            relation_kind: if parent_id.is_some() {
                agena_domain::SessionRelationKind::Subagent
            } else {
                agena_domain::SessionRelationKind::Root
            },
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            task_id: None,
            subtask_access: None,
            subtask_status: parent_id.map(|_| SubtaskStatus::Running),
            created_at: now,
            updated_at: now,
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
        };
        let tree = vec![
            summary(1, None, 0),
            summary(2, Some(1), 1),
            summary(3, Some(2), 2),
            summary(4, Some(1), 1),
        ];

        assert_eq!(descendant_cancellation_order(2, &tree), vec![3, 2]);
        assert_eq!(descendant_cancellation_order(1, &tree), vec![3, 4, 2, 1]);
    }
}
