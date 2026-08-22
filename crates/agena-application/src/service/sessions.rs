use super::{
    ApplicationError, ApplicationResult, ApplicationService, PageOrder, PaginatedResponse,
    SessionCreateRequest, SessionCursor, SessionLifecycleState, SessionRelationKind,
    SessionResource, SessionState, SessionUpdateRequest, SubtaskStatus, build_page, decode_cursor,
    execution_access_from_domain, non_empty, normalize_limit, timestamp_millis_to_utc, trim_page,
};
use agena_storage::store::SessionListQuery;

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
            .transpose()?
            .map(|cursor| agena_storage::store::SessionCursor {
                updated_at_ms: cursor.updated_at_ms,
                id: cursor.id,
            });
        let fetch_limit = i64::try_from(limit.saturating_add(1)).map_err(|_| {
            ApplicationError::internal("page limit cannot be represented in storage")
        })?;
        let rows = self
            .session_store
            .list_session_summaries(SessionListQuery {
                workspace_id: query.workspace_id,
                parent_id: query.parent_id,
                roots_only: query.roots,
                exclude_subagents: query.exclude_subagents,
                search: non_empty(query.pagination.search()).map(ToString::to_string),
                limit: Some(fetch_limit),
                before: cursor,
            })
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let next_cursor = slice.last().map(|row| SessionCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });
        let session_ids = slice.iter().map(|summary| summary.id).collect::<Vec<_>>();
        let states = self
            .session_store
            .session_states(session_ids.as_slice())
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let mut resources = Vec::with_capacity(slice.len());
        for summary in slice {
            let state = states.get(&summary.id).copied().ok_or_else(|| {
                ApplicationError::internal(format!(
                    "session {} disappeared while listing processing states",
                    summary.id
                ))
            })?;
            resources.push(session_resource_from_storage_summary(&summary, state)?);
        }

        build_page(resources, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_session(&self, session_id: i64) -> ApplicationResult<Option<SessionResource>> {
        let Some(summary) = self
            .session_store
            .get_session_summary(session_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
        else {
            return Ok(None);
        };
        if summary.lifecycle_state != agena_domain::SessionLifecycleState::Ready {
            return Ok(None);
        }
        let state = self
            .session_store
            .session_state(session_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
            .state;
        Ok(Some(session_resource_from_storage_summary(
            &summary, state,
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

        let relation_kind = if request.session.parent_id.is_some() {
            agena_domain::SessionRelationKind::Child
        } else {
            agena_domain::SessionRelationKind::Root
        };
        let created = self
            .session_store
            .create_session(agena_storage::store::NewSession {
                workspace_id: request.workspace_id,
                parent_id: request.session.parent_id,
                relation_kind,
                cutoff_part_id: None,
                title: request.session.title,
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        session_resource_from_storage_meta(&created, agena_storage::store::SessionState::Ready)
    }

    pub async fn replace_session(
        &self,
        session_id: i64,
        request: SessionUpdateRequest,
    ) -> ApplicationResult<SessionResource> {
        self.ensure_session_model(session_id).await?;

        if request.title.is_none() && request.favorite.is_none() && request.pinned.is_none() {
            return Err(ApplicationError::bad_request(
                "session metadata update must include title, favorite, or pinned",
            ));
        }
        if request
            .title
            .as_deref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(ApplicationError::bad_request(
                "session title cannot be empty",
            ));
        }

        let updated = self
            .session_store
            .update_metadata(
                session_id,
                agena_storage::store::SessionMetadataPatch {
                    title: request.title,
                    favorite: request.favorite,
                    pinned: request.pinned,
                },
            )
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let state = self
            .session_store
            .session_state(session_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
            .state;
        session_resource_from_storage_meta(&updated, state)
    }

    pub async fn delete_session(&self, session_id: i64) -> ApplicationResult<SessionResource> {
        let summary = self.ensure_session_model(session_id).await?;
        let state = self
            .session_store
            .session_state(session_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
            .state;
        let resource = session_resource_from_storage_summary(&summary, state)?;
        self.session_store
            .delete(session_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        Ok(resource)
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
        favorite: summary.favorite,
        pinned: summary.pinned,
        version: summary.version,
        relation_kind: session_relation_kind_from_domain(summary.relation_kind),
        lifecycle_state: session_lifecycle_state_from_domain(summary.lifecycle_state),
        state: session_state_from_lifecycle(summary.lifecycle_state),
        source_cutoff_seq_global: summary.source_cutoff_seq_global,
        source_message_id: summary.source_message_id,
        is_subagent: summary.relation_kind.is_subagent(),
        task_id: summary.task_id,
        subtask_access: summary.subtask_access.map(execution_access_from_domain),
        subtask_status: summary.subtask_status.map(subtask_status_from_domain),
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        message_count: summary.message_count,
        child_session_count: summary.child_session_count,
        last_message_at: summary.last_message_at,
    }
}

fn session_resource_from_storage_summary(
    summary: &agena_storage::store::SessionSummary,
    state: agena_storage::store::SessionState,
) -> ApplicationResult<SessionResource> {
    let subtask_status = if summary.relation_kind.is_subagent() {
        summary
            .subtask_status
            .as_deref()
            .and_then(agena_domain::SubtaskStatus::parse)
            .map(subtask_status_from_domain)
            .or(Some(SubtaskStatus::default()))
    } else {
        None
    };
    Ok(SessionResource {
        id: summary.id,
        parent_id: summary.parent_id,
        depth: summary.depth,
        root_id: summary.root_id,
        workspace_id: summary.workspace_id,
        title: summary.title.clone(),
        favorite: summary.favorite,
        pinned: summary.pinned,
        version: summary.version,
        relation_kind: session_relation_kind_from_domain(summary.relation_kind),
        lifecycle_state: session_lifecycle_state_from_domain(summary.lifecycle_state),
        state: session_state_from_storage(state),
        source_cutoff_seq_global: None,
        source_message_id: None,
        is_subagent: summary.relation_kind.is_subagent(),
        task_id: summary.task_id.clone(),
        // v2 dissolved per-summary `subtask_access` (13.2); wire keeps it None.
        subtask_access: None,
        subtask_status,
        created_at: timestamp_millis_to_utc(summary.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(summary.updated_at_ms)?,
        message_count: u64::try_from(summary.message_count).map_err(|_| {
            ApplicationError::internal(format!(
                "invalid negative message count for session {}",
                summary.id
            ))
        })?,
        child_session_count: u64::try_from(summary.child_session_count).map_err(|_| {
            ApplicationError::internal(format!(
                "invalid negative child session count for session {}",
                summary.id
            ))
        })?,
        last_message_at: summary
            .last_message_at_ms
            .map(timestamp_millis_to_utc)
            .transpose()?,
    })
}

/// Project a v2 `SessionMeta` (returned by create/rename) into the public
/// resource. Counts are zero — a freshly created/renamed session has no parts
/// yet, and callers re-fetch via `get_session` when they need full stats.
fn session_resource_from_storage_meta(
    meta: &agena_storage::store::SessionMeta,
    state: agena_storage::store::SessionState,
) -> ApplicationResult<SessionResource> {
    let subtask_status = if meta.relation_kind.is_subagent() {
        meta.subtask_status
            .as_deref()
            .and_then(agena_domain::SubtaskStatus::parse)
            .map(subtask_status_from_domain)
            .or(Some(SubtaskStatus::default()))
    } else {
        None
    };
    Ok(SessionResource {
        id: meta.id,
        parent_id: meta.parent_id,
        depth: meta.depth,
        root_id: meta.root_id,
        workspace_id: meta.workspace_id,
        title: meta.title.clone(),
        favorite: meta.favorite,
        pinned: meta.pinned,
        version: meta.version,
        relation_kind: session_relation_kind_from_domain(meta.relation_kind),
        lifecycle_state: session_lifecycle_state_from_domain(meta.lifecycle_state),
        state: session_state_from_storage(state),
        source_cutoff_seq_global: None,
        source_message_id: None,
        is_subagent: meta.relation_kind.is_subagent(),
        task_id: meta.task_id.clone(),
        subtask_access: None,
        subtask_status,
        created_at: timestamp_millis_to_utc(meta.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(meta.updated_at_ms)?,
        message_count: 0,
        child_session_count: 0,
        last_message_at: None,
    })
}

fn session_state_from_lifecycle(value: agena_domain::SessionLifecycleState) -> SessionState {
    match value {
        agena_domain::SessionLifecycleState::Creating => SessionState::Creating,
        agena_domain::SessionLifecycleState::Ready => SessionState::Ready { last_failure: None },
        agena_domain::SessionLifecycleState::Failed => SessionState::Failed { failure: None },
    }
}

pub(crate) fn session_state_from_storage(
    value: agena_storage::store::SessionState,
) -> SessionState {
    match value {
        agena_storage::store::SessionState::Creating => SessionState::Creating,
        agena_storage::store::SessionState::Ready => SessionState::Ready { last_failure: None },
        agena_storage::store::SessionState::Running => SessionState::Running {
            execution: None,
            workflow: agena_api::resource::WorkflowState::Quiescent,
            requests: Vec::new(),
        },
        agena_storage::store::SessionState::AwaitingInteraction => {
            SessionState::AwaitingInteraction {
                run_id: None,
                execution: None,
                requests: Vec::new(),
            }
        }
        agena_storage::store::SessionState::Interrupted => SessionState::Interrupted {
            run_id: None,
            reason: Some("lease_lost".to_owned()),
            last_failure: None,
        },
        agena_storage::store::SessionState::Failed => SessionState::Failed { failure: None },
    }
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
    use std::sync::Arc;

    use agena_domain::SessionRelationKind;
    use agena_storage::WorkspaceRepository;
    use agena_storage::store::{NewPart, NewSession, PartRole, SessionFacade, SessionStore};
    use sea_orm::Database;
    use serde_json::json;

    use super::*;

    async fn test_service() -> (ApplicationService, Arc<dyn SessionStore>, i64) {
        let db = Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("open test database"),
        );
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("initialize test schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::clone(&db))
            .ensure_id("/test/workspace")
            .await
            .expect("create test workspace");
        let engine = agena_storage_sqlite::SqliteEngine::new(Arc::clone(&db));
        let facade: Arc<dyn SessionStore> =
            Arc::new(SessionFacade::new(engine.clone(), "application-test", 64));
        let service = ApplicationService::new(
            "/test/workspace",
            Arc::new(agena_storage::MemoryStore::for_workspace(
                std::path::Path::new("/test/workspace"),
            )),
            Arc::new(agena_storage_sqlite::SeaWorkspaceRepository::new(
                Arc::clone(&db),
            )),
            Arc::new(agena_storage_sqlite::SeaPermissionRuleRepository::new(
                Arc::clone(&db),
            )),
            Arc::clone(&facade),
        );
        (service, facade, workspace_id)
    }

    /// One user content part. `submit_user_run` creates the D9 run marker.
    fn marker_part() -> NewPart {
        NewPart::pending("text", PartRole::User, json!({ "text": "hello" }))
    }

    #[tokio::test]
    async fn session_list_materializes_message_and_child_counts() {
        let (service, facade, workspace_id) = test_service().await;
        let session = facade
            .create_session(NewSession {
                workspace_id,
                parent_id: None,
                relation_kind: SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "Counted session".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create test session");
        facade
            .create_session(NewSession {
                workspace_id,
                parent_id: Some(session.id),
                relation_kind: SessionRelationKind::Child,
                cutoff_part_id: None,
                title: "Child session".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create child session");
        // Two runs over the parent session → message_count 2, last message at
        // the second run's timestamp.
        facade
            .submit_user_run(session.id, "application-test", vec![marker_part()], None)
            .await
            .expect("first run");
        let first_run = facade
            .load(session.id)
            .await
            .expect("load first run")
            .parts
            .into_iter()
            .find(|part| part.kind == "run")
            .expect("first marker")
            .part_id;
        facade
            .complete_run(
                session.id,
                "application-test",
                first_run,
                agena_storage::store::RunOutcome {
                    status: agena_storage::store::PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
            )
            .await
            .expect("complete first run");
        facade
            .submit_user_run(session.id, "application-test", vec![marker_part()], None)
            .await
            .expect("second run");

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
        assert!(resource.last_message_at.is_some());
        assert!(resource.state.is_running());
    }

    #[tokio::test]
    async fn list_exclude_subagents_hides_only_task_children() {
        let (service, facade, workspace_id) = test_service().await;
        let parent = facade
            .create_session(NewSession {
                workspace_id,
                parent_id: None,
                relation_kind: agena_domain::SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "Parent".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create parent");
        // A task child via the store's dedicated subagent path.
        facade
            .create_subagent_session(parent.id, "task-1".to_owned(), "Sub task".to_owned())
            .await
            .expect("create subagent");

        let all = service
            .list_sessions(crate::dto::SessionListQuery {
                workspace_id: Some(workspace_id),
                ..Default::default()
            })
            .await
            .expect("list all");
        assert!(
            all.items.iter().any(|item| item.is_subagent),
            "without the filter the task child is listed"
        );

        let parents_only = service
            .list_sessions(crate::dto::SessionListQuery {
                workspace_id: Some(workspace_id),
                exclude_subagents: true,
                ..Default::default()
            })
            .await
            .expect("list excluding subagents");
        assert!(
            parents_only.items.iter().all(|item| !item.is_subagent),
            "task child must be hidden by exclude_subagents"
        );
        assert_eq!(parents_only.items.len(), 1);
        assert_eq!(parents_only.items[0].id, parent.id);
    }

    #[tokio::test]
    async fn get_create_rename_delete_round_trip_on_the_facade() {
        let (service, _facade, workspace_id) = test_service().await;
        let created = service
            .create_session(crate::dto::SessionCreateRequest {
                workspace_id,
                session: crate::dto::SessionHierarchyRequest {
                    parent_id: None,
                    title: "New session".to_owned(),
                },
            })
            .await
            .expect("create session");
        assert_eq!(created.title, "New session");
        assert_eq!(created.lifecycle_state, SessionLifecycleState::Ready);

        let renamed = service
            .replace_session(
                created.id,
                SessionUpdateRequest {
                    title: Some("Renamed".to_owned()),
                    favorite: Some(true),
                    pinned: Some(true),
                },
            )
            .await
            .expect("rename session");
        assert_eq!(renamed.title, "Renamed");
        assert!(renamed.favorite);
        assert!(renamed.pinned);
        assert_eq!(renamed.version, created.version + 1);

        let unpinned = service
            .replace_session(
                created.id,
                SessionUpdateRequest {
                    title: None,
                    favorite: None,
                    pinned: Some(false),
                },
            )
            .await
            .expect("update only pinned state");
        assert_eq!(unpinned.title, "Renamed");
        assert!(unpinned.favorite);
        assert!(!unpinned.pinned);

        let fetched = service
            .get_session(created.id)
            .await
            .expect("get session")
            .expect("session exists");
        assert_eq!(fetched.title, "Renamed");
        assert!(fetched.favorite);
        assert!(!fetched.pinned);

        let deleted = service
            .delete_session(created.id)
            .await
            .expect("delete session");
        assert_eq!(deleted.title, "Renamed");
        assert!(
            service
                .get_session(created.id)
                .await
                .expect("get session")
                .is_none()
        );
    }
}
