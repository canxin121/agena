use super::{ExecutionControlError, execution_control_to_app_error};
use crate::{
    AppError,
    message::{Message, MessagePart, PartContent},
    session::{Session, SessionManager},
};
use agena_domain::{ExecutionStatus, Role, SessionSummary};
use agena_runtime::{SessionForkRequest, SessionRewindRequest};

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
            .map_err(|error| agena_runtime::RuntimeEventQueryError::new(error.to_string()))?
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
            .map_err(|error| agena_runtime::RuntimeEventQueryError::new(error.to_string()))?
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
            .map_err(|error| agena_runtime::RuntimeEventQueryError::new(error.to_string()))?
            .iter()
            .map(project_runtime_timeline_event)
            .collect()
    }
}

fn project_runtime_timeline_event(
    event: &crate::event::DomainEvent,
) -> Result<agena_runtime::RuntimeTimelineEvent, agena_runtime::RuntimeEventQueryError> {
    use crate::event::EventKind;

    let (type_key, linked_message_id) = match &event.kind {
        EventKind::ExecutionStarted(_) => ("timeline-type-execution-started", None),
        EventKind::ExecutionFinished(_) => ("timeline-type-execution-failed", None),
        EventKind::SubtaskStatusChanged(_) => ("timeline-type-subtask-status-changed", None),
        EventKind::StreamError(_) => ("timeline-type-stream-error", None),
        EventKind::MessagePartCheckpointed(value) => (
            "timeline-type-message-part-checkpointed",
            Some(value.message_id),
        ),
        EventKind::MessagePartDelta(value) => {
            ("timeline-type-message-part-delta", Some(value.message_id))
        }
        EventKind::CommandBegin(value) => ("timeline-type-command-begin", value.context.message_id),
        EventKind::CommandOutputDelta(value) => (
            "timeline-type-command-output-delta",
            value.context.message_id,
        ),
        EventKind::CommandEnd(value) => ("timeline-type-command-end", value.context.message_id),
        EventKind::PermissionRequested(_) => ("timeline-type-permission-requested", None),
        EventKind::PermissionReplied(_) => ("timeline-type-permission-replied", None),
        EventKind::PermissionRuleCreated(_) => ("timeline-type-permission-rule-created", None),
        EventKind::PermissionRuleUpdated(_) => ("timeline-type-permission-rule-updated", None),
        EventKind::PermissionRuleRevoked(_) => ("timeline-type-permission-rule-revoked", None),
        EventKind::RunStarted(_) => ("timeline-type-run-started", None),
        EventKind::RunCompleted(_) => ("timeline-type-run-completed", None),
        EventKind::RunAborted(_) => ("timeline-type-run-aborted", None),
        EventKind::UserMessageAppended(value) => (
            "timeline-type-user-message-appended",
            Some(value.message_id.into()),
        ),
        EventKind::AssistantMessageFinished(value) => (
            "timeline-type-assistant-message-completed",
            Some(value.message_id.into()),
        ),
        EventKind::ToolCallIssued(_) => ("timeline-type-tool-call-issued", None),
        EventKind::ToolCallCompleted(_) => ("timeline-type-tool-call-completed", None),
        EventKind::SystemNoticeAppended(value) => (
            "timeline-type-system-notice-appended",
            Some(value.message_id.into()),
        ),
        EventKind::PluginEvent(_) | EventKind::PluginToolRegistryChanged(_) => {
            ("timeline-type-plugin-event", None)
        }
    };
    let detail = serde_json::to_string_pretty(&event.kind)
        .map_err(|error| agena_runtime::RuntimeEventQueryError::new(error.to_string()))?;
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
        linked_message_id,
    })
}

fn project_runtime_event(
    event: &crate::event::DomainEvent,
) -> Result<agena_runtime::RuntimeEvent, agena_runtime::RuntimeEventQueryError> {
    let value = serde_json::to_value(event)
        .map_err(|error| agena_runtime::RuntimeEventQueryError::new(error.to_string()))?;
    let mut object = value.as_object().cloned().ok_or_else(|| {
        agena_runtime::RuntimeEventQueryError::new("event must serialize as an object")
    })?;
    let kind = object
        .remove("kind")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| agena_runtime::RuntimeEventQueryError::new("event is missing kind"))?;
    let payload = object.remove("payload").unwrap_or(serde_json::Value::Null);
    let meta = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|error| agena_runtime::RuntimeEventQueryError::new(error.to_string()))?;
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

    let kind = match &event.kind {
        EventKind::MessagePartCheckpointed(update) => {
            let metadata = &update.message_metadata;
            let part = project_message_part(update.part.clone())?;
            Some(
                agena_runtime::RuntimePresentationEventKind::MessagePartCheckpointed(Box::new(
                    agena_runtime::RuntimeMessagePartCheckpoint {
                        session_id: update.session_id,
                        execution_id: update.execution_id,
                        run_id: update.run_id,
                        message_id: update.message_id,
                        message_role: update.message_role,
                        message_state: update.message_state,
                        message_created_at: update.message_created_at,
                        message_metadata: agena_runtime::RuntimeMessageMetadata {
                            source: metadata.source,
                            idempotency_key: metadata.idempotency_key.clone(),
                            turn_id: metadata.turn_id,
                            parent_message_id: metadata.parent_message_id,
                            generated_by_call_id: metadata.generated_by_call_id,
                            model_provider_id: metadata.model_provider_id.clone(),
                            model_adapter_id: metadata.model_adapter_id.clone(),
                            model_id: metadata.model_id.clone(),
                            model_thinking_mode: metadata.model_thinking_mode.clone(),
                            model_speed_mode: metadata.model_speed_mode.clone(),
                        },
                        part,
                        ts_ms: update.ts_ms,
                    },
                )),
            )
        }
        EventKind::MessagePartDelta(delta) => {
            Some(agena_runtime::RuntimePresentationEventKind::MessagePartDelta(delta.clone()))
        }
        EventKind::UserMessageAppended(message) => Some(
            agena_runtime::RuntimePresentationEventKind::UserMessageAppended {
                message_id: message.message_id.raw(),
            },
        ),
        EventKind::AssistantMessageFinished(_) => {
            Some(agena_runtime::RuntimePresentationEventKind::AssistantMessageFinished)
        }
        EventKind::ToolCallCompleted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::SystemNoticeAppended(_)
        | EventKind::ExecutionFinished(_) => {
            Some(agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: false,
            })
        }
        EventKind::PermissionRequested(_) | EventKind::PermissionReplied(_) => {
            Some(agena_runtime::RuntimePresentationEventKind::Refresh {
                force_refresh: true,
            })
        }
        _ => None,
    };
    Ok(kind.map(|kind| agena_runtime::RuntimePresentationEvent {
        meta: event.meta.clone(),
        invalidates_ancestor_projection: event.kind.invalidates_ancestor_projection(),
        kind,
    }))
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
            .map_err(|error| agena_runtime::RuntimeEventPublishError::new(error.to_string()))
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
        let root_result = self.execution_registry.cancel(session_id).await;
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
            let result = self.execution_registry.cancel(target_id).await;
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
        if !is_completed_user_rewind_target(
            source
                .messages
                .iter()
                .find(|message| message.id == request.message_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "message not found in session {}: {}",
                        source.id, request.message_id
                    ))
                })?,
        ) {
            return Err(AppError::Internal(format!(
                "rewind target must be a completed user message: {}",
                request.message_id
            )));
        }
        let title = format!("Rewind of {}", source.title);
        self.store
            .fork_session_before_message(source, request.message_id, title, state.cache_policy())
            .await
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
}

#[async_trait::async_trait]
impl agena_runtime::SessionQueryService for SessionManager {
    async fn find_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        SessionManager::find_session_id_for_message(self, message_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn find_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        SessionManager::find_session_id_for_part(self, part_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn list_session_summaries(
        &self,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<agena_domain::SessionSummary>, agena_runtime::SessionQueryError> {
        self.list_session_summaries(request)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn session_presentation(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionPresentation, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        let workflow_state = session.runtime().workflow.state;
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

    async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_runtime::SessionProjectedMessageHeader>, agena_runtime::SessionQueryError>
    {
        SessionManager::list_projected_message_headers(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?
            .into_iter()
            .map(|header| {
                Ok(agena_runtime::SessionProjectedMessageHeader {
                    id: header.id,
                    role: header.role,
                    state: header.state,
                    created_at: header.created_at,
                    metadata: serde_json::to_value(header.metadata).map_err(|error| {
                        agena_runtime::SessionQueryError::new(error.to_string())
                    })?,
                    usage: header
                        .usage
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            agena_runtime::SessionQueryError::new(error.to_string())
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
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?
            .into_iter()
            .map(|message| {
                Ok(agena_runtime::SessionProjectedMessage {
                    id: message.id,
                    role: message.role,
                    state: message.state,
                    created_at: message.created_at,
                    metadata: serde_json::to_value(message.metadata).map_err(|error| {
                        agena_runtime::SessionQueryError::new(error.to_string())
                    })?,
                    usage: message
                        .usage
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            agena_runtime::SessionQueryError::new(error.to_string())
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
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn export_session_jsonl(
        &self,
        session_id: i64,
    ) -> Result<String, agena_runtime::SessionQueryError> {
        SessionManager::export_session_jsonl(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn latest_event_seq(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        let events = SessionManager::list_session_events(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        Ok(events.iter().map(|event| event.meta.seq_global).max())
    }

    async fn session_usage(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionUsage, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        SessionManager::session_usage(self, &session)
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionCostSummary, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        Ok(crate::session::cost::summarize(&session.messages))
    }

    async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, agena_runtime::SessionQueryError> {
        SessionManager::usage_stats(self, query)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))
    }

    async fn pending_interactive_requests(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_domain::PendingInteractiveRequestContext>, agena_runtime::SessionQueryError>
    {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        let tree = SessionManager::list_session_tree(self, session.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
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
                    .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?,
            );
        }

        Ok(sessions
            .into_iter()
            .flat_map(|pending_session| {
                let session_id = pending_session.id;
                let parent_session_id = pending_session.parent_id;
                let task_id = pending_session.task_id.clone();
                let profile = pending_session.runtime().execution.selection.agent.clone();
                pending_session
                    .pending_interactive_requests()
                    .into_iter()
                    .map(
                        move |request| agena_domain::PendingInteractiveRequestContext {
                            session_id,
                            parent_session_id,
                            task_id: task_id.clone(),
                            profile: profile.clone(),
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
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        let runtime = session.runtime();
        Ok(agena_runtime::SessionExecutionContext {
            workflow_state: session.workflow_state(),
            agent_profile: runtime.execution.selection.agent.clone(),
            active_skill_name: runtime.execution.active_skill_name.clone(),
            agent_system_prompt: runtime.execution.agent_system_prompt.clone(),
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
            subtask_error: runtime.subtask.error.clone(),
        })
    }

    async fn is_descendant_session(
        &self,
        descendant_id: i64,
        ancestor_id: i64,
    ) -> Result<bool, agena_runtime::SessionQueryError> {
        let descendant = SessionManager::get_session(self, descendant_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
        let tree = SessionManager::list_session_tree(self, descendant.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?;
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
        operation_id: part.operation_id,
        created_at: part.created_at,
        detail: part.content.as_ref().map(project_part_detail),
        content: part
            .content
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| agena_runtime::SessionQueryError::new(error.to_string()))?,
    })
}

fn project_part_detail(content: &PartContent) -> agena_runtime::SessionProjectedPartDetail {
    match content {
        PartContent::Text(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.text.clone(),
            synthetic: value.synthetic,
            ignored: value.ignored,
        },
        PartContent::Reasoning(value) => agena_runtime::SessionProjectedPartDetail::Reasoning {
            summary: value.summary.clone(),
            raw_content: value.raw_content.clone(),
            encrypted_content: value.encrypted_content.clone(),
        },
        PartContent::Error(value) => agena_runtime::SessionProjectedPartDetail::Error {
            code: value.code.clone(),
            message: value.message.clone(),
        },
        PartContent::Attachment(value) => {
            agena_runtime::SessionProjectedPartDetail::Attachment(value.clone())
        }
        PartContent::Request(crate::message::RequestPart::Permission(value)) => {
            agena_runtime::SessionProjectedPartDetail::PermissionRequest {
                request: value.request.clone(),
                reply: value.reply.clone(),
            }
        }
        PartContent::Request(crate::message::RequestPart::UserInput(value)) => {
            agena_runtime::SessionProjectedPartDetail::UserInputRequest {
                request: value.request.clone(),
                reply: value.reply.clone(),
            }
        }
        PartContent::Operation(value) => agena_runtime::SessionProjectedPartDetail::Operation(
            Box::new(project_operation_part(value)),
        ),
    }
}

fn project_operation_part(
    value: &crate::message::OperationPart,
) -> agena_runtime::SessionProjectedOperationPart {
    agena_runtime::SessionProjectedOperationPart {
        call_id: value.call_id,
        invocation: value.invocation.clone(),
        title: value.title.clone(),
        summary: value.summary.clone(),
        model_output: project_model_visible_output(&value.model_output),
        blocks: value.blocks.iter().map(project_operation_block).collect(),
        artifacts: value.artifacts.clone(),
        attachments: value.attachments.clone(),
        details: value.details.clone(),
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
        structured: value.structured.clone(),
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
    value: &crate::message::OperationBlock,
) -> agena_runtime::SessionProjectedOperationBlock {
    use agena_runtime::SessionProjectedOperationBlock as Projected;
    match value {
        crate::message::OperationBlock::Text { text } => Projected::Text { text: text.clone() },
        crate::message::OperationBlock::Markdown { text } => {
            Projected::Markdown { text: text.clone() }
        }
        crate::message::OperationBlock::Json { value } => Projected::Json {
            value: value.clone(),
        },
        crate::message::OperationBlock::Table { columns, rows } => Projected::Table {
            columns: columns.clone(),
            rows: rows.clone(),
        },
        crate::message::OperationBlock::Log { stream, text } => Projected::Log {
            stream: stream.clone(),
            text: text.clone(),
        },
        crate::message::OperationBlock::Command {
            command,
            cwd,
            exit_code,
            stdout,
            stderr,
        } => Projected::Command {
            command: command.clone(),
            cwd: cwd.clone(),
            exit_code: *exit_code,
            stdout: stdout.clone(),
            stderr: stderr.clone(),
        },
        crate::message::OperationBlock::Diff { diff, language } => Projected::Diff {
            diff: diff.clone(),
            language: language.clone(),
        },
        crate::message::OperationBlock::FileChanges { changes } => Projected::FileChanges {
            changes: changes.clone(),
        },
        crate::message::OperationBlock::SearchResults { query, results } => {
            Projected::SearchResults {
                query: query.clone(),
                results: results.clone(),
            }
        }
        crate::message::OperationBlock::Citation {
            uri,
            title,
            snippet,
        } => Projected::Citation {
            uri: uri.clone(),
            title: title.clone(),
            snippet: snippet.clone(),
        },
        crate::message::OperationBlock::Image { mime, url } => Projected::Image {
            mime: mime.clone(),
            url: url.clone(),
        },
        crate::message::OperationBlock::Audio { mime, url } => Projected::Audio {
            mime: mime.clone(),
            url: url.clone(),
        },
        crate::message::OperationBlock::ResourceLink {
            uri,
            title,
            mime_type,
        } => Projected::ResourceLink {
            uri: uri.clone(),
            title: title.clone(),
            mime_type: mime_type.clone(),
        },
        crate::message::OperationBlock::EmbeddedResource {
            uri,
            mime,
            text,
            base64,
        } => Projected::EmbeddedResource {
            uri: uri.clone(),
            mime: mime.clone(),
            text: text.clone(),
            base64: base64.clone(),
        },
        crate::message::OperationBlock::File {
            url,
            filename,
            mime,
        } => Projected::File {
            url: url.clone(),
            filename: filename.clone(),
            mime: mime.clone(),
        },
        crate::message::OperationBlock::Media {
            mime_type,
            artifact,
        } => Projected::Media {
            mime_type: mime_type.clone(),
            artifact: artifact.clone(),
        },
        crate::message::OperationBlock::Checklist { items } => Projected::Checklist {
            items: items.clone(),
        },
        crate::message::OperationBlock::NestedTask {
            task_id,
            title,
            status,
        } => Projected::NestedTask {
            task_id: task_id.clone(),
            title: title.clone(),
            status: *status,
        },
        crate::message::OperationBlock::Progress { message, percent } => Projected::Progress {
            message: message.clone(),
            percent: *percent,
        },
        crate::message::OperationBlock::Custom { schema, value } => Projected::Custom {
            schema: schema.clone(),
            value: value.clone(),
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
            subtask_profile: None,
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
