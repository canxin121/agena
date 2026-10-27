use super::{
    ApplicationError, ApplicationResult, ApplicationService, PageOrder, PaginatedResponse,
    SessionCreateRequest, SessionResource, SessionState, SessionUpdateRequest, build_page,
    decode_cursor, non_empty, normalize_limit, timestamp_millis_to_utc,
};
use agena_storage::kernel::{
    MetadataChange, RequestId, SessionSummaryCursor, SessionSummaryView, SummaryQuery,
};

impl ApplicationService {
    pub async fn list_sessions(
        &self,
        query: crate::dto::SessionListQuery,
    ) -> ApplicationResult<PaginatedResponse<SessionResource>> {
        let limit = normalize_limit(query.pagination.limit());
        let before = query
            .pagination
            .cursor()
            .map(decode_cursor::<SessionSummaryCursor>)
            .transpose()?;
        if before.is_some() && query.offset > 0 {
            return Err(ApplicationError::bad_request(
                "Use either cursor or offset pagination.",
            ));
        }
        use agena_api::queries::SessionListBucket;
        let page = self
            .session_kernel
            .reader
            .query_summaries(SummaryQuery {
                workspace_id: query.workspace_id,
                parent_id: query.parent_id.map(crate::kernel_session_id).transpose()?,
                roots_only: query.roots,
                exclude_subagents: query.exclude_subagents,
                temporary_only: false,
                agents_only: false,
                search: non_empty(query.pagination.search()).map(str::to_owned),
                before,
                offset: query.offset,
                limit: limit as usize,
                favorite: (query.bucket == Some(SessionListBucket::Favorite)).then_some(true),
                pinned: (query.bucket == Some(SessionListBucket::Pinned)).then_some(true),
                states: match query.bucket {
                    Some(SessionListBucket::Running) => vec![
                        agena_domain::SessionStateKind::Running,
                        agena_domain::SessionStateKind::Creating,
                    ],
                    Some(SessionListBucket::Attention) => vec![
                        agena_domain::SessionStateKind::AwaitingInteraction,
                        agena_domain::SessionStateKind::Failed,
                    ],
                    Some(SessionListBucket::Recent) => vec![agena_domain::SessionStateKind::Ready],
                    _ => Vec::new(),
                },
            })
            .await
            .map_err(crate::kernel_error)?;
        let items = if query.count_only {
            Vec::new()
        } else {
            page.sessions
                .iter()
                .map(session_resource_from_kernel_summary)
                .collect::<ApplicationResult<Vec<_>>>()?
        };
        let mut result = build_page(
            items,
            !query.count_only && page.has_more,
            if query.count_only { None } else { page.next },
            PageOrder::Desc,
            limit,
        )?;
        if query.include_total || query.count_only {
            result.total = page.total;
        }
        Ok(result)
    }

    pub async fn get_session(&self, session_id: i64) -> ApplicationResult<Option<SessionResource>> {
        match self
            .session_kernel
            .reader
            .summary(crate::kernel_session_id(session_id)?)
            .await
        {
            Ok(summary) => session_resource_from_kernel_summary(&summary).map(Some),
            Err(agena_storage::kernel::KernelError::NotFound(_)) => Ok(None),
            Err(error) => Err(crate::kernel_error(error)),
        }
    }

    pub async fn create_session(
        &self,
        request: SessionCreateRequest,
    ) -> ApplicationResult<SessionResource> {
        self.ensure_workspace_exists(request.workspace_id).await?;
        let configuration = agena_storage::kernel::SessionConfiguration {
            additional_system_prompt: request.session.additional_system_prompt,
            title_state: if request.session.auto_title {
                agena_storage::kernel::SessionTitleState::automatic()
            } else {
                Default::default()
            },
            parent_session_id: request
                .session
                .parent_id
                .map(crate::kernel_session_id)
                .transpose()?,
            relation_kind: if request.session.parent_id.is_some() {
                agena_domain::SessionRelationKind::Child
            } else {
                agena_domain::SessionRelationKind::Root
            },
            ..Default::default()
        };
        let created = self
            .session_kernel
            .sessions
            .create_configured(
                RequestId::new(),
                request.workspace_id,
                request.session.title,
                configuration,
            )
            .await
            .map_err(crate::kernel_error)?;
        self.get_session(created.value.id.get())
            .await?
            .ok_or_else(|| ApplicationError::not_found("The session was not found."))
    }

    pub async fn replace_session(
        &self,
        session_id: i64,
        request: SessionUpdateRequest,
    ) -> ApplicationResult<SessionResource> {
        if request.title.is_none()
            && request.favorite.is_none()
            && request.pinned.is_none()
            && request.auto_title.is_none()
            && request.additional_system_prompt.is_none()
        {
            return Err(ApplicationError::bad_request(
                "Include title, favorite, pinned, auto_title, or additional_system_prompt in a session update.",
            ));
        }
        self.session_kernel
            .sessions
            .update_metadata(
                RequestId::new(),
                crate::kernel_session_id(session_id)?,
                MetadataChange {
                    title: request.title,
                    favorite: request.favorite,
                    pinned: request.pinned,
                    auto_title: request.auto_title,
                    additional_system_prompt: request.additional_system_prompt,
                    ..Default::default()
                },
            )
            .await
            .map_err(crate::kernel_error)?;
        self.get_session(session_id)
            .await?
            .ok_or_else(|| ApplicationError::not_found("The session was not found."))
    }

    pub async fn delete_session(&self, session_id: i64) -> ApplicationResult<SessionResource> {
        let resource = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| ApplicationError::not_found("The session was not found."))?;
        self.session_kernel
            .sessions
            .delete(RequestId::new(), crate::kernel_session_id(session_id)?)
            .await
            .map_err(crate::kernel_error)?;
        Ok(resource)
    }
}

pub(crate) fn session_resource_from_kernel_summary(
    summary: &SessionSummaryView,
) -> ApplicationResult<SessionResource> {
    Ok(SessionResource {
        id: summary.id.get(),
        parent_id: summary.parent_session_id.map(|id| id.get()),
        depth: i64::from(summary.depth),
        root_id: summary.root_session_id.get(),
        workspace_id: summary.workspace_id,
        title: summary.title.clone(),
        favorite: summary.favorite,
        pinned: summary.pinned,
        version: summary.metadata_version.get(),
        relation_kind: summary.relation_kind,
        lifecycle_state: agena_domain::SessionLifecycleState::Ready,
        state: session_state_from_storage(summary.state),
        source_cutoff_seq_global: None,
        source_message_id: None,
        is_subagent: summary.relation_kind.is_subagent(),
        agent_name: summary.agent_name.clone(),
        agent_status: summary.agent_status,
        created_at: timestamp_millis_to_utc(summary.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(summary.updated_at_ms)?,
        message_count: summary.user_message_count,
        child_session_count: summary.child_session_count,
        last_message_at: summary
            .last_message_at_ms
            .map(timestamp_millis_to_utc)
            .transpose()?,
    })
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
        relation_kind: summary.relation_kind,
        lifecycle_state: summary.lifecycle_state,
        state: match summary.lifecycle_state {
            agena_domain::SessionLifecycleState::Creating => SessionState::Creating,
            agena_domain::SessionLifecycleState::Failed => SessionState::Failed { failure: None },
            _ => SessionState::Ready { last_failure: None },
        },
        source_cutoff_seq_global: summary.source_cutoff_seq_global,
        source_message_id: summary.source_message_id,
        is_subagent: summary.relation_kind.is_subagent(),
        agent_name: summary.agent_name,
        agent_status: summary.agent_status,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        message_count: summary.message_count,
        child_session_count: summary.child_session_count,
        last_message_at: summary.last_message_at,
    }
}

pub(crate) fn session_state_from_storage(value: agena_domain::SessionStateKind) -> SessionState {
    match value {
        agena_domain::SessionStateKind::Creating => SessionState::Creating,
        agena_domain::SessionStateKind::Ready => SessionState::Ready { last_failure: None },
        agena_domain::SessionStateKind::Running => SessionState::Running {
            execution: None,
            workflow: agena_api::resource::WorkflowState::Quiescent,
            requests: Vec::new(),
        },
        agena_domain::SessionStateKind::AwaitingInteraction => SessionState::AwaitingInteraction {
            run_id: None,
            execution: None,
            requests: Vec::new(),
        },
        agena_domain::SessionStateKind::Failed => SessionState::Failed { failure: None },
    }
}
