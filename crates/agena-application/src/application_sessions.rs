//! Session/workspace operations, migrated from
//! `agena-tui-backend/src/backend_session.rs` and `backend_workspace.rs`
//! (session-facing subset) plus the shared helpers from
//! `backend_plugins.rs` (`resolve_workspace_resource`, `current_workspace_id`,
//! `list_sessions_query`, `resolve_session_root`).

use std::collections::HashSet;

use agena_api::queries::ListSessionsParams;
use agena_api::resource::{
    PermissionReply, RunOptions, SessionExecutionResource, SessionOverviewResource,
    SessionResource, UserInputReply, WorkspaceResource,
};
use agena_domain::{CancellationResult, ComposerDocument, ExecutionId, PermissionConfig, TurnId};
use agena_runtime::{SessionForkRequest, SessionRewindRequest};
use agena_storage::store::SessionPartView;

use crate::dto::{
    CursorPaginationQuery, SearchPaginationQuery, SessionCreateRequest, SessionHierarchyRequest,
    SessionListQuery, WorkspacePathRequest, WorkspaceResolveRequest,
};
use crate::{Application, ApplicationError};

impl Application {
    /// Build the default server home view: every session needing attention,
    /// every running session, then a bounded recent tail.
    pub async fn session_overview(
        &self,
        workspace_id: Option<i64>,
        recent_limit: u64,
    ) -> Result<SessionOverviewResource, ApplicationError> {
        let workspace_id = match workspace_id {
            Some(workspace_id) => workspace_id,
            None => self.current_workspace_id().await?,
        };
        let sessions = self
            .list_sessions_query(ListSessionsParams {
                cursor: None,
                limit: Some(200),
                workspace_id: Some(workspace_id),
                parent_id: None,
                roots: false,
                exclude_subagents: true,
                search: None,
            })
            .await?;
        let mut attention = Vec::new();
        let mut running = Vec::new();
        let mut recent = Vec::new();
        for session in sessions {
            if session.state.is_attention() {
                attention.push(session);
            } else if session.state.is_running()
                || matches!(session.state, agena_api::resource::SessionState::Creating)
            {
                running.push(session);
            } else {
                recent.push(session);
            }
        }
        let sort_recent = |left: &SessionResource, right: &SessionResource| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        };
        attention.sort_by(|left, right| {
            session_attention_priority(&right.state)
                .cmp(&session_attention_priority(&left.state))
                .then_with(|| sort_recent(left, right))
        });
        running.sort_by(sort_recent);
        recent.sort_by(sort_recent);
        recent.truncate(usize::try_from(recent_limit.clamp(1, 200)).unwrap_or(200));
        Ok(SessionOverviewResource {
            attention,
            running,
            recent,
            generated_at: chrono::Utc::now(),
        })
    }

    /// Project runtime summaries into public resources and enrich every row
    /// with the authoritative store-derived processing state.
    pub async fn session_resources_from_summaries(
        &self,
        summaries: Vec<agena_domain::SessionSummary>,
    ) -> Result<Vec<SessionResource>, ApplicationError> {
        let session_ids = summaries
            .iter()
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        let states = self
            .session_store_facade()?
            .session_states(session_ids.as_slice())
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let mut resources = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let mut resource = crate::session::session_resource_from_summary(summary);
            let state = states.get(&resource.id).copied().ok_or_else(|| {
                ApplicationError::internal(format!(
                    "session {} disappeared while projecting processing state",
                    resource.id
                ))
            })?;
            resource.state = session_state_resource(state);
            resources.push(resource);
        }
        Ok(resources)
    }

    /// Return the store-derived processing state without materializing a full
    /// transcript. Session lists in non-HTTP transports use this same source
    /// of truth as `SessionResource.state`.
    pub async fn session_processing_state(
        &self,
        session_id: i64,
    ) -> Result<agena_api::resource::SessionState, ApplicationError> {
        let state = self
            .session_store_facade()?
            .session_state(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .state;
        Ok(session_state_resource(state))
    }

    pub async fn list_child_sessions(
        &self,
        parent_id: i64,
    ) -> Result<Vec<SessionResource>, ApplicationError> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(ListSessionsParams {
            cursor: None,
            limit: Some(200),
            workspace_id: Some(workspace_id),
            parent_id: Some(parent_id),
            roots: false,
            exclude_subagents: false,
            search: None,
        })
        .await
        .map_err(|error| {
            ApplicationError::internal(format!("failed to list child sessions: {error}"))
        })
    }

    async fn get_session(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionResource>, ApplicationError> {
        self.service()
            .get_session(session_id)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("failed to fetch session: {error}"))
            })
    }

    pub async fn list_session_subtree(
        &self,
        session_id: i64,
    ) -> Result<Vec<SessionResource>, ApplicationError> {
        let root = self.resolve_session_root(session_id).await?;
        let mut items = vec![root.clone()];
        let mut seen = HashSet::from([root.id]);
        let mut stack = vec![root.id];

        while let Some(parent_id) = stack.pop() {
            let children = self
                .list_sessions_query(ListSessionsParams {
                    cursor: None,
                    limit: Some(200),
                    workspace_id: Some(root.workspace_id),
                    parent_id: Some(parent_id),
                    roots: false,
                    exclude_subagents: false,
                    search: None,
                })
                .await
                .map_err(|error| {
                    ApplicationError::internal(format!(
                        "failed to list subtree children for session {parent_id}: {error}"
                    ))
                })?;
            for child in children {
                if seen.insert(child.id) {
                    stack.push(child.id);
                    items.push(child);
                }
            }
        }

        Ok(items)
    }

    pub async fn list_session_timeline_parts(
        &self,
        session_id: i64,
        limit: u64,
    ) -> Result<Vec<SessionPartView>, ApplicationError> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let view = self
            .session_store_facade()?
            .load(session_id)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "failed to load session timeline parts: {error}"
                ))
            })?;
        let visible = view
            .parts
            .into_iter()
            .filter(|part| part.visibility.visible_to_user())
            .collect::<Vec<_>>();
        let skip = visible.len().saturating_sub(limit);
        Ok(visible
            .into_iter()
            .skip(skip)
            .map(|part| SessionPartView {
                part_id: part.part_id,
                kind: part.kind,
                role: part.role,
                state: part.state,
                summary: part.summary,
                content: part.content,
                rendered_markdown: part.rendered_markdown,
                parent_part_id: part.parent_part_id,
                run_id: part.run_id,
                revision: part.revision,
                created_at_ms: part.created_at_ms,
                updated_at_ms: part.updated_at_ms,
            })
            .collect())
    }

    /// Shared read-back: project a session's execution resource after a
    /// command or for a state query. Every session command method reads back
    /// through this single helper instead of re-assembling the runtime
    /// services per call (this absorbed the old
    /// `session::session_execution_resource` free-function passthrough).
    pub async fn session_execution_resource(
        &self,
        session_id: i64,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let session_services = self.session_execution_services()?;
        let mut resource = self
            .service()
            .session_execution_resource(
                session_services.execution_control.as_ref(),
                session_services.queries.as_ref(),
                session_id,
            )
            .await?;
        if let Ok(activities) = self.runtime_activities() {
            let filter = agena_domain::BackgroundActivityFilter {
                session_id: Some(session_id),
                active_only: true,
                ..Default::default()
            };
            resource.background_activities = activities
                .list_activities(&filter)
                .await
                .map_err(|error| {
                    ApplicationError::internal(format!(
                        "failed to project background activities for session {session_id}: {error}"
                    ))
                })?
                .iter()
                .map(agena_api::resource::BackgroundActivityResource::from)
                .collect();
        }
        Ok(resource)
    }

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: PermissionConfig,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        self.session_execution_services()?
            .commands
            .set_session_permission(session_id, permission)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "failed to set permission for session {session_id}: {error}"
                ))
            })?;
        self.session_execution_resource(session_id).await
    }

    pub async fn rewind_session_to_turn(
        &self,
        session_id: i64,
        turn_id: TurnId,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let expected_version = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| ApplicationError::internal(format!("session not found: {session_id}")))?
            .version;
        let session_services = self.session_execution_services()?;
        let outcome = session_services
            .commands
            .rewind_session(SessionRewindRequest {
                session_id,
                turn_id,
                expected_version: Some(expected_version),
            })
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    // ── Unified session command family ────────────────────────────────────
    // The terminal, JSON-RPC dispatch, and REST handlers all drive these
    // Application methods. Each one is the same "assemble request → call the
    // runtime command trait → read back the execution projection" sequence, so
    // transports keep only their own wire adaptation (request/response shape
    // and error mapping) and never re-assemble the runtime services.

    /// Submit a user document (composer message) as a run and read back the
    /// resulting execution state.
    pub async fn submit_user_run(
        &self,
        session_id: i64,
        document: ComposerDocument,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let request =
            crate::session::session_user_run_request(self, session_id, options, document).await?;
        let outcome = self
            .session_execution_services()?
            .commands
            .submit_user_run(request)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Continue an existing session with the given run options.
    pub async fn continue_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let request = crate::session::session_execution_request(self, session_id, options).await?;
        let outcome = self
            .session_execution_services()?
            .commands
            .continue_session(request)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Compact an existing session with the given run options.
    pub async fn compact_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let request = crate::session::session_execution_request(self, session_id, options).await?;
        let outcome = self
            .session_execution_services()?
            .commands
            .compact_session(request)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Reply to a pending permission request.
    pub async fn reply_permission(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: PermissionReply,
        source: Option<String>,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let request = crate::session::session_permission_reply_request(
            self, session_id, options, reply, source,
        )
        .await?;
        let outcome = self
            .session_execution_services()?
            .commands
            .reply_permission(request)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Reply to a pending interactive user-input request.
    pub async fn reply_user_input(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: UserInputReply,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let request =
            crate::session::session_user_input_reply_request(self, session_id, options, reply)
                .await?;
        let outcome = self
            .session_execution_services()?
            .commands
            .reply_user_input(request)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Clone a session's full history into a new child session.
    pub async fn fork_session(
        &self,
        session_id: i64,
        at_message_id: Option<i64>,
        title: Option<String>,
        expected_version: Option<i64>,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let outcome = self
            .session_execution_services()?
            .commands
            .fork_session(SessionForkRequest {
                session_id,
                at_message_id,
                title,
                expected_version,
            })
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Cancel the active run of `session_id`.
    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: ExecutionId,
    ) -> Result<CancellationResult, ApplicationError> {
        self.session_execution_services()?
            .execution_control
            .cancel_execution(session_id, execution_id)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    /// Durable, idempotent acknowledgement that an interactive user-input
    /// request has been shown to the user.
    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let outcome = self
            .session_execution_services()?
            .commands
            .mark_interactive_request_presented(session_id, request_id)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    /// Update the session's selected model/options without starting a run.
    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let options =
            crate::session::resolve_session_run_options(self, session_id, options).await?;
        let outcome = self
            .session_execution_services()?
            .commands
            .update_session_selection(session_id, options)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        self.session_execution_resource(outcome.session_id).await
    }

    pub async fn list_workspace_sessions(
        &self,
        roots_only: bool,
    ) -> Result<Vec<SessionResource>, ApplicationError> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(ListSessionsParams {
            cursor: None,
            limit: Some(200),
            workspace_id: Some(workspace_id),
            parent_id: None,
            roots: roots_only,
            exclude_subagents: true,
            search: None,
        })
        .await
        .map_err(|error| {
            ApplicationError::internal(format!("failed to list workspace sessions: {error}"))
        })
    }

    pub async fn create_session(
        &self,
        title: String,
        parent_id: Option<i64>,
    ) -> Result<SessionResource, ApplicationError> {
        let workspace = self
            .resolve_workspace_resource(true)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "failed to resolve workspace for terminal UI: {error}"
                ))
            })?;

        self.service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                session: SessionHierarchyRequest { title, parent_id },
            })
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("failed to create session: {error}"))
            })
    }

    pub async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<SessionResource, ApplicationError> {
        let existing = self
            .get_session(session_id)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("failed to load session before rename: {error}"))
            })?
            .ok_or_else(|| {
                ApplicationError::internal(format!("session not found: {session_id}"))
            })?;

        self.service()
            .assert_session_version(session_id, existing.version)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "failed to assert session version before rename: {error}"
                ))
            })?;

        self.service()
            .replace_session(
                session_id,
                crate::dto::SessionUpdateRequest {
                    title: Some(title),
                    favorite: None,
                    pinned: None,
                },
            )
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("failed to rename session: {error}"))
            })
    }

    async fn resolve_workspace_resource(
        &self,
        create_if_missing: bool,
    ) -> Result<WorkspaceResource, ApplicationError> {
        self.service()
            .resolve_workspace(WorkspaceResolveRequest {
                workspace: WorkspacePathRequest {
                    path: self.workspace_root().to_string_lossy().to_string(),
                },
                create_if_missing,
            })
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("failed to resolve workspace: {error}"))
            })
    }

    async fn current_workspace_id(&self) -> Result<i64, ApplicationError> {
        Ok(self
            .resolve_workspace_resource(true)
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("failed to resolve current workspace: {error}"))
            })?
            .id)
    }

    async fn list_sessions_query(
        &self,
        query: ListSessionsParams,
    ) -> Result<Vec<SessionResource>, ApplicationError> {
        let mut cursor = query.cursor.clone();
        let limit = query.limit.unwrap_or(200);
        let mut items = Vec::new();

        loop {
            let page = self
                .service()
                .list_sessions(SessionListQuery {
                    pagination: SearchPaginationQuery {
                        pagination: CursorPaginationQuery {
                            cursor: cursor.clone(),
                            limit: Some(limit),
                        },
                        search: query.search.clone(),
                    },
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                    exclude_subagents: query.exclude_subagents,
                })
                .await?;
            cursor = page.page.next_cursor.clone();
            items.extend(page.items);
            if !page.page.has_more || cursor.is_none() {
                break;
            }
        }

        Ok(items)
    }

    async fn resolve_session_root(
        &self,
        session_id: i64,
    ) -> Result<SessionResource, ApplicationError> {
        let mut current = self.get_session(session_id).await?.ok_or_else(|| {
            ApplicationError::internal(format!("session not found: {session_id}"))
        })?;
        while let Some(parent_id) = current.parent_id {
            current = self.get_session(parent_id).await?.ok_or_else(|| {
                ApplicationError::internal(format!("session not found: {parent_id}"))
            })?;
        }
        Ok(current)
    }
}

fn session_state_resource(
    state: agena_storage::store::SessionState,
) -> agena_api::resource::SessionState {
    crate::service::sessions::session_state_from_storage(state)
}

fn session_attention_priority(state: &agena_api::resource::SessionState) -> u8 {
    match state {
        agena_api::resource::SessionState::AwaitingInteraction { .. } => 2,
        agena_api::resource::SessionState::Interrupted { .. } => 1,
        agena_api::resource::SessionState::Running { requests, .. } if !requests.is_empty() => 3,
        _ => 0,
    }
}
