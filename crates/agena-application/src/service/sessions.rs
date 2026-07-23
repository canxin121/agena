use super::{
    ApplicationError, ApplicationResult, ApplicationService, CursorPaginationQuery, EventCursor,
    HashMap, PageOrder, PaginatedResponse, SessionCreateRequest, SessionCursor,
    SessionLifecycleState, SessionRelationKind, SessionResource, SessionUpdateRequest,
    SubtaskStatus, build_page, decode_cursor, non_empty, normalize_limit, timestamp_millis_to_utc,
    trim_page,
};
use agena_storage::SessionSummaryListQuery;

impl ApplicationService {
    pub async fn list_sessions(
        &self,
        query: crate::dto::SessionListQuery,
    ) -> ApplicationResult<PaginatedResponse<SessionResource>> {
        let limit = normalize_limit(query.pagination.limit());
        let cursor = query
            .pagination
            .cursor()
            .map(decode_cursor::<SessionCursor>)
            .transpose()?;
        let rows = self
            .session_summary_repository
            .list(SessionSummaryListQuery {
                workspace_id: query.workspace_id,
                roots_only: query.roots,
                parent_id: query.parent_id,
                search: non_empty(query.pagination.search()).map(ToString::to_string),
                before_updated_at_ms: cursor.map(|value| value.updated_at_ms),
                before_id: cursor.map(|value| value.id),
                offset: 0,
                limit: limit + 1,
                include_subagents: true,
            })
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let next_cursor = slice.last().map(|row| SessionCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });
        let session_ids = slice.iter().map(|row| row.id).collect::<Vec<_>>();
        let message_stats = self
            .session_stats_repository
            .event_stats(&session_ids)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let child_counts = self
            .session_stats_repository
            .child_counts(&session_ids)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let resources = slice
            .iter()
            .map(|summary| {
                session_resource_from_storage_summary(summary, &message_stats, &child_counts)
            })
            .collect::<ApplicationResult<Vec<_>>>()?;

        build_page(resources, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_session(&self, session_id: i64) -> ApplicationResult<Option<SessionResource>> {
        let Some(summary) = self
            .session_summary_repository
            .get(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Ok(None);
        };
        if summary.lifecycle_state != agena_domain::SessionLifecycleState::Ready {
            return Ok(None);
        }
        let message_stats = self
            .session_stats_repository
            .event_stats(&[session_id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let child_counts = self
            .session_stats_repository
            .child_counts(&[session_id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        Ok(Some(session_resource_from_storage_summary(
            &summary,
            &message_stats,
            &child_counts,
        )?))
    }

    pub async fn create_session(
        &self,
        request: SessionCreateRequest,
    ) -> ApplicationResult<SessionResource> {
        self.ensure_workspace_exists(request.workspace_id).await?;
        if let Some(parent_id) = request.session.parent_id {
            let parent = self.ensure_session_model(parent_id).await?;
            if parent.workspace_id != request.workspace_id {
                return Err(ApplicationError::bad_request(
                    "parent session must belong to the same workspace",
                ));
            }
        }

        let created = self
            .session_mutation_repository
            .create(
                request.workspace_id,
                request.session.parent_id,
                request.session.title,
            )
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let message_stats = self
            .session_stats_repository
            .event_stats(&[created.id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let child_counts = self
            .session_stats_repository
            .child_counts(&[created.id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        session_resource_from_storage_summary(&created, &message_stats, &child_counts)
    }

    pub async fn replace_session(
        &self,
        session_id: i64,
        request: SessionUpdateRequest,
    ) -> ApplicationResult<SessionResource> {
        self.ensure_session_model(session_id).await?;

        let updated = self
            .session_mutation_repository
            .rename(session_id, request.title)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .ok_or_else(|| {
                ApplicationError::not_found(format!("session not found: {session_id}"))
            })?;
        let message_stats = self
            .session_stats_repository
            .event_stats(&[session_id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let child_counts = self
            .session_stats_repository
            .child_counts(&[session_id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        session_resource_from_storage_summary(&updated, &message_stats, &child_counts)
    }

    pub async fn delete_session(&self, session_id: i64) -> ApplicationResult<SessionResource> {
        self.ensure_session_model(session_id).await?;
        let existing = self
            .session_summary_repository
            .get(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .ok_or_else(|| {
                ApplicationError::not_found(format!("session not found: {session_id}"))
            })?;
        let message_stats = self
            .session_stats_repository
            .event_stats(&[session_id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let child_counts = self
            .session_stats_repository
            .child_counts(&[session_id])
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let resource =
            session_resource_from_storage_summary(&existing, &message_stats, &child_counts)?;
        self.session_mutation_repository
            .delete(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        Ok(resource)
    }

    pub async fn list_session_events(
        &self,
        events: &dyn agena_runtime::RuntimeEventQueryService,
        session_id: i64,
        query: CursorPaginationQuery,
    ) -> ApplicationResult<PaginatedResponse<agena_runtime::RuntimeEvent>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<EventCursor>)
            .transpose()?;

        let fetch_limit = usize::try_from(limit)
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        let newest_first = events
            .list_events_before(
                &agena_domain::EventFilter {
                    scope: agena_domain::EventScope::Session { session_id },
                    kinds: None,
                    since_seq_global: None,
                },
                agena_runtime::RuntimeReverseEventRange {
                    before_seq_global: cursor.map(|cursor| cursor.seq),
                    limit: fetch_limit,
                },
            )
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;

        // Storage returns newest-first; reverse after truncation so clients
        // can append each page in ascending event order.
        let has_more = newest_first.len() > limit as usize;
        let mut slice: Vec<_> = newest_first.into_iter().take(limit as usize).collect();
        let next_cursor = slice.last().map(|e| EventCursor {
            seq: e.meta.seq_global,
            id: e.meta.seq_global,
        });
        slice.reverse();

        build_page(slice, has_more, next_cursor, PageOrder::Asc, limit)
    }
}

pub(crate) fn session_resource_from_summary(
    summary: agena_domain::SessionSummary,
) -> SessionResource {
    SessionResource {
        id: summary.id,
        parent_id: summary.parent_id,
        depth: summary.depth,
        root_id: summary.root_id,
        workspace_id: summary.workspace_id,
        title: summary.title,
        version: summary.version,
        relation_kind: session_relation_kind_from_domain(summary.relation_kind),
        lifecycle_state: session_lifecycle_state_from_domain(summary.lifecycle_state),
        source_cutoff_seq_global: summary.source_cutoff_seq_global,
        source_message_id: summary.source_message_id,
        is_subagent: summary.relation_kind.is_subagent(),
        task_id: summary.task_id,
        subtask_profile: summary.subtask_profile,
        subtask_status: summary.subtask_status.map(subtask_status_from_domain),
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        message_count: summary.message_count,
        child_session_count: summary.child_session_count,
        last_message_at: summary.last_message_at,
    }
}

fn session_resource_from_storage_summary(
    summary: &agena_storage::SessionSummaryRecord,
    message_stats: &HashMap<i64, agena_storage::SessionEventStats>,
    child_counts: &HashMap<i64, i64>,
) -> ApplicationResult<SessionResource> {
    let subtask_status = if summary.relation_kind.is_subagent() {
        summary
            .subtask_status
            .map(subtask_status_from_domain)
            .or_else(|| Some(SubtaskStatus::default()))
    } else {
        None
    };
    let stats = message_stats.get(&summary.id).copied();
    let message_count = stats
        .map(|item| u64::try_from(item.message_count))
        .transpose()
        .map_err(|_| ApplicationError::internal("invalid negative message count"))?
        .unwrap_or_default();
    let child_session_count = child_counts
        .get(&summary.id)
        .copied()
        .map(u64::try_from)
        .transpose()
        .map_err(|_| ApplicationError::internal("invalid negative child session count"))?
        .unwrap_or_default();
    Ok(SessionResource {
        id: summary.id,
        parent_id: summary.parent_id,
        depth: summary.depth,
        root_id: summary.root_id,
        workspace_id: summary.workspace_id,
        title: summary.title.clone(),
        version: summary.version,
        relation_kind: session_relation_kind_from_domain(summary.relation_kind),
        lifecycle_state: session_lifecycle_state_from_domain(summary.lifecycle_state),
        source_cutoff_seq_global: summary.source_cutoff_seq_global,
        source_message_id: summary.source_message_id,
        is_subagent: summary.relation_kind.is_subagent(),
        task_id: summary.task_id.clone(),
        subtask_profile: summary.subtask_profile.clone(),
        subtask_status,
        created_at: timestamp_millis_to_utc(summary.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(summary.updated_at_ms)?,
        message_count,
        child_session_count,
        last_message_at: stats
            .and_then(|item| item.last_message_at_ms)
            .map(timestamp_millis_to_utc)
            .transpose()?,
    })
}

pub(crate) const fn session_relation_kind_from_domain(
    value: agena_domain::SessionRelationKind,
) -> SessionRelationKind {
    match value {
        agena_domain::SessionRelationKind::Root => SessionRelationKind::Root,
        agena_domain::SessionRelationKind::Child => SessionRelationKind::Child,
        agena_domain::SessionRelationKind::Fork => SessionRelationKind::Fork,
        agena_domain::SessionRelationKind::Rewind => SessionRelationKind::Rewind,
        agena_domain::SessionRelationKind::Subagent => SessionRelationKind::Subagent,
    }
}

pub(crate) const fn session_lifecycle_state_from_domain(
    value: agena_domain::SessionLifecycleState,
) -> SessionLifecycleState {
    match value {
        agena_domain::SessionLifecycleState::Creating => SessionLifecycleState::Creating,
        agena_domain::SessionLifecycleState::Ready => SessionLifecycleState::Ready,
        agena_domain::SessionLifecycleState::Failed => SessionLifecycleState::Failed,
    }
}

pub(crate) const fn subtask_status_from_domain(
    value: agena_domain::SubtaskStatus,
) -> SubtaskStatus {
    match value {
        agena_domain::SubtaskStatus::Created => SubtaskStatus::Created,
        agena_domain::SubtaskStatus::Running => SubtaskStatus::Running,
        agena_domain::SubtaskStatus::Completed => SubtaskStatus::Completed,
        agena_domain::SubtaskStatus::Failed => SubtaskStatus::Failed,
        agena_domain::SubtaskStatus::Cancelled => SubtaskStatus::Cancelled,
        agena_domain::SubtaskStatus::TimedOut => SubtaskStatus::TimedOut,
        agena_domain::SubtaskStatus::Interrupted => SubtaskStatus::Interrupted,
    }
}

#[cfg(test)]
mod tests {
    use agena_domain::{
        EVENT_ENVELOPE_SCHEMA_VERSION, EventEnvelope, EventKindTag, EventMeta, KindMatcher,
    };
    use agena_storage::{EventStore, SessionMutationRepository, WorkspaceRepository};
    use chrono::{TimeZone, Utc};
    use sea_orm::Database;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    enum SessionListFixtureEvent {
        UserMessageAppended,
        AssistantMessageFinished,
    }

    impl KindMatcher for SessionListFixtureEvent {
        fn tag(&self) -> EventKindTag {
            match self {
                Self::UserMessageAppended => "user_message_appended".into(),
                Self::AssistantMessageFinished => "assistant_message_finished".into(),
            }
        }
    }

    fn fixture_event(
        session_id: i64,
        workspace_id: i64,
        seq: i64,
        kind: SessionListFixtureEvent,
        created_at_ms: i64,
    ) -> EventEnvelope<SessionListFixtureEvent> {
        EventEnvelope {
            meta: EventMeta {
                id: uuid::Uuid::from_u128(seq as u128),
                seq_global: seq,
                seq_session: Some(seq),
                session_id: Some(session_id),
                workspace_id: Some(workspace_id),
                created_at: Utc
                    .timestamp_millis_opt(created_at_ms)
                    .single()
                    .expect("fixture event timestamp"),
                causation_id: None,
                correlation_id: None,
                envelope_schema: EVENT_ENVELOPE_SCHEMA_VERSION,
            },
            kind,
        }
    }

    #[tokio::test]
    async fn session_list_materializes_message_and_child_counts() {
        let db = std::sync::Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("open test database"),
        );
        agena_storage_sqlite::initialize_schema(db.as_ref())
            .await
            .expect("initialize test schema");
        let workspace_id =
            agena_storage_sqlite::SeaWorkspaceRepository::new(std::sync::Arc::clone(&db))
                .ensure_id("/test/workspace")
                .await
                .expect("create test workspace");
        let sessions =
            agena_storage_sqlite::SeaSessionSummaryRepository::new(std::sync::Arc::clone(&db));
        let session = sessions
            .create(workspace_id, None, "Counted session".to_owned())
            .await
            .expect("create test session");
        sessions
            .create(workspace_id, Some(session.id), "Child session".to_owned())
            .await
            .expect("create child session");
        let events = agena_storage_sqlite::SeaEventStore::<SessionListFixtureEvent>::new(
            std::sync::Arc::clone(&db),
        );
        events
            .append_batch(&[
                fixture_event(
                    session.id,
                    workspace_id,
                    1,
                    SessionListFixtureEvent::UserMessageAppended,
                    1_000,
                ),
                fixture_event(
                    session.id,
                    workspace_id,
                    2,
                    SessionListFixtureEvent::AssistantMessageFinished,
                    2_000,
                ),
            ])
            .await
            .expect("append test events");

        let service = ApplicationService::new(
            "/test/workspace",
            None,
            std::sync::Arc::new(agena_storage::MemoryStore::for_workspace(
                std::path::Path::new("/test/workspace"),
            )),
            std::sync::Arc::new(agena_storage_sqlite::SeaWorkspaceRepository::new(
                std::sync::Arc::clone(&db),
            )),
            std::sync::Arc::new(agena_storage_sqlite::SeaPermissionRuleRepository::new(
                std::sync::Arc::clone(&db),
            )),
            std::sync::Arc::new(agena_storage_sqlite::SeaSessionStatsRepository::new(
                std::sync::Arc::clone(&db),
            )),
            std::sync::Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                std::sync::Arc::clone(&db),
            )),
            std::sync::Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                std::sync::Arc::clone(&db),
            )),
        );
        let page = service
            .list_sessions(crate::dto::SessionListQuery {
                workspace_id: Some(workspace_id),
                ..Default::default()
            })
            .await
            .expect("list test sessions");
        let resource = page
            .items
            .iter()
            .find(|resource| resource.id == session.id)
            .expect("listed parent session");

        assert_eq!(resource.message_count, 2);
        assert_eq!(resource.child_session_count, 1);
        assert_eq!(
            resource
                .last_message_at
                .map(|value| value.timestamp_millis()),
            Some(2_000)
        );
    }
}
