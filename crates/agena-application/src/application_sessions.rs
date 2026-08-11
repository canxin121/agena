//! Session/workspace operations, migrated from
//! `agena-tui-backend/src/backend_session.rs` and `backend_workspace.rs`
//! (session-facing subset) plus the shared helpers from
//! `backend_plugins.rs` (`resolve_workspace_resource`, `current_workspace_id`,
//! `list_sessions_query`, `resolve_session_root`).

use std::collections::HashSet;

use agena_api::queries::ListSessionsParams;
use agena_api::resource::{SessionExecutionResource, SessionResource, WorkspaceResource};
use agena_domain::{PermissionConfig, TurnId};
use agena_runtime::SessionRewindRequest;
use agena_storage::store::SessionPartView;

use crate::dto::{
    CursorPaginationQuery, SearchPaginationQuery, SessionCreateRequest, SessionHierarchyRequest,
    SessionListQuery, WorkspacePathRequest, WorkspaceResolveRequest,
};
use crate::{Application, ApplicationError};

impl Application {
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

    async fn get_session_state(
        &self,
        session_id: i64,
    ) -> Result<SessionExecutionResource, ApplicationError> {
        let session_services = self.session_execution_services()?;
        crate::session::session_execution_resource(
            self,
            session_services.execution_control.as_ref(),
            session_services.queries.as_ref(),
            session_id,
        )
        .await
        .map_err(|error| {
            ApplicationError::internal(format!("failed to load session state: {error}"))
        })
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
        self.get_session_state(session_id).await
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
        crate::session::session_execution_resource(
            self,
            session_services.execution_control.as_ref(),
            session_services.queries.as_ref(),
            outcome.session_id,
        )
        .await
        .map_err(|error| {
            ApplicationError::internal(format!("failed to rewind session to turn: {error}"))
        })
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
            .replace_session(session_id, crate::dto::SessionUpdateRequest { title })
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
