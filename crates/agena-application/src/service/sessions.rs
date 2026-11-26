use super::{
    ApplicationError, ApplicationResult, ApplicationService, PageOrder, PaginatedResponse,
    SessionCreateRequest, SessionResource, SessionState, SessionUpdateRequest, build_page,
    decode_cursor, non_empty, normalize_resource_limit, timestamp_millis_to_utc,
};
use agena_prompt_layers::{PromptError, PromptLayer, PromptScope};
use agena_storage::kernel::{
    MetadataChange, RequestId, SessionSummaryCursor, SessionSummaryView, SummaryQuery,
    SummarySelection,
};

impl ApplicationService {
    pub async fn external_agent_workspace(
        &self,
        workspace_id: i64,
        action: agena_domain::ExternalAgentWorkspaceAction,
    ) -> ApplicationResult<agena_domain::ExternalAgentWorkspaceResult> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        self.ensure_workspace_exists(workspace_id).await?;
        commands
            .external_agent_workspace(workspace_id, action)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn external_agent_session(
        &self,
        session_id: i64,
        action: agena_domain::ExternalAgentSessionAction,
    ) -> ApplicationResult<agena_domain::ExternalAgentSessionResult> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        commands
            .external_agent_session(session_id, action)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn external_agent_options(
        &self,
        session_id: i64,
        request: agena_api::commands::ExternalAgentOptionsParams,
    ) -> ApplicationResult<agena_domain::ExternalAgentOptions> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        commands
            .external_agent_options(session_id, request.change)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn external_agent_auth(
        &self,
        session_id: i64,
        action: agena_domain::ExternalAgentAuthAction,
    ) -> ApplicationResult<agena_domain::ExternalAgentAuth> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        commands
            .external_agent_auth(session_id, action)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn external_agent_live_auth(
        &self,
        session_id: i64,
    ) -> ApplicationResult<Option<agena_domain::ExternalAgentAuthStatus>> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        commands
            .external_agent_live_auth(session_id)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn external_agent_providers(
        &self,
        session_id: i64,
    ) -> ApplicationResult<agena_domain::ExternalAgentProviders> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        commands
            .external_agent_providers(session_id)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn external_agent_work(
        &self,
        session_id: i64,
        cancel_id: Option<uuid::Uuid>,
    ) -> ApplicationResult<Vec<agena_domain::ExternalAgentWorkItem>> {
        let commands = self.session_commands.as_ref().ok_or_else(|| {
            ApplicationError::internal("Session execution commands are unavailable.")
        })?;
        commands
            .external_agent_work(session_id, cancel_id)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))
    }

    pub async fn list_sessions(
        &self,
        query: crate::dto::SessionListQuery,
    ) -> ApplicationResult<PaginatedResponse<SessionResource>> {
        let limit = normalize_resource_limit(query.pagination.limit());
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
                // Ownership decides sidebar membership independently of the
                // working directory. Explicit Bot queries and unscoped child
                // queries can still read global and workspace-backed Bots.
                include_global: query.parent_id.is_some()
                    || query.owner_bot_id.is_some()
                    || query.owner_surface.is_some(),
                exclude_bot_owned: query.owner_bot_id.is_none()
                    && query.owner_surface.is_none()
                    && (query.parent_id.is_none() || query.workspace_id.is_some()),
                exclude_external_agents: !self.is_acp_scope(),
                owner_bot_id: query.owner_bot_id.map(|id| id.to_string()),
                owner_surface: query.owner_surface.map(|s| s.as_str().to_owned()),
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
                selection: if query.count_only {
                    SummarySelection::Total
                } else if query.include_total {
                    SummarySelection::PageWithTotal
                } else {
                    SummarySelection::Page
                },
            })
            .await
            .map_err(crate::kernel_error)?;
        let items = page
            .sessions
            .iter()
            .map(session_resource_from_kernel_summary)
            .collect::<ApplicationResult<Vec<_>>>()?;
        let mut result = build_page(items, page.has_more, page.next, PageOrder::Desc, limit)?;
        result.total = page.total;
        Ok(result)
    }

    /// Replace the session prompt layer and return the updated session.
    pub async fn set_session_prompts(
        &self,
        session_id: i64,
        prompts: Option<agena_prompt_layers::PromptLayer>,
    ) -> ApplicationResult<SessionResource> {
        if let Some(layer) = &prompts {
            validate_session_prompt_layer(layer)?;
        }
        self.session_kernel
            .sessions
            .update_metadata(
                RequestId::new(),
                crate::kernel_session_id(session_id)?,
                MetadataChange {
                    prompts,
                    ..Default::default()
                },
            )
            .await
            .map_err(crate::kernel_error)?;
        self.get_session(session_id)
            .await?
            .ok_or_else(|| ApplicationError::bad_request("session not found"))
    }

    pub async fn get_session(&self, session_id: i64) -> ApplicationResult<Option<SessionResource>> {
        match self
            .session_kernel
            .reader
            .summary(crate::kernel_session_id(session_id)?)
            .await
        {
            Ok(summary) if self.is_acp_scope() == summary.external_agent.is_some() => {
                session_resource_from_kernel_summary(&summary).map(Some)
            }
            Ok(_) => Ok(None),
            Err(agena_storage::kernel::KernelError::NotFound(_)) => Ok(None),
            Err(error) => Err(crate::kernel_error(error)),
        }
    }

    pub async fn create_session(
        &self,
        request: SessionCreateRequest,
    ) -> ApplicationResult<SessionResource> {
        if self.is_acp_scope() != request.session.external_agent.is_some()
            || (self.is_acp_scope() && request.session.parent_id.is_some())
        {
            return Err(ApplicationError::bad_request(
                "Choose the ACP or Agena session entry point for this conversation.",
            ));
        }
        self.ensure_workspace_exists(request.workspace_id).await?;
        if let Some(layer) = &request.session.prompts {
            validate_session_prompt_layer(layer)?;
        }
        let mut configuration = agena_storage::kernel::SessionConfiguration {
            prompts: request.session.prompts.unwrap_or_default(),
            external_agent: request.session.external_agent,
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
        if let Some(parent) = configuration.parent_session_id {
            let inherited = self
                .session_kernel
                .reader
                .configuration(parent)
                .await
                .map_err(crate::kernel_error)?;
            configuration.profile = inherited.profile.clone();
            configuration.selection_defaults = inherited.selection_defaults.clone();
            configuration.permission = inherited.permission.clone();
            configuration.permission_ceiling = inherited.permission_ceiling.clone();
            configuration.permission_ceiling_layers = inherited.permission_ceiling_layers.clone();
        }
        configuration = self
            .prepare_session_profile(
                Some(request.workspace_id),
                configuration,
                request.session.profile,
                true,
            )
            .await?;
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
            && request.prompts.is_none()
            && request.profile.is_none()
        {
            return Err(ApplicationError::bad_request(
                "Include title, favorite, pinned, auto_title, prompts, or profile in a session update.",
            ));
        }
        if let Some(layer) = &request.prompts {
            validate_session_prompt_layer(layer)?;
        }
        // Profile validation and the eventual write use one metadata revision;
        // a concurrent update cannot overwrite the policy we just validated.
        let expected_version = if request.profile.is_some() {
            let session = crate::kernel_session_id(session_id)?;
            let snapshot = self
                .session_kernel
                .reader
                .control_snapshot(session, 1)
                .await
                .map_err(crate::kernel_error)?;
            self.prepare_session_profile(
                snapshot.metadata.workspace_id,
                snapshot.configuration.clone(),
                request.profile.clone(),
                false,
            )
            .await?;
            Some(snapshot.metadata.metadata_version.get())
        } else {
            None
        };
        self.session_kernel
            .sessions
            .update_metadata(
                RequestId::new(),
                crate::kernel_session_id(session_id)?,
                MetadataChange {
                    expected_version,
                    profile: request.profile,
                    title: request.title,
                    favorite: request.favorite,
                    pinned: request.pinned,
                    auto_title: request.auto_title,
                    prompts: request.prompts,
                    ..Default::default()
                },
            )
            .await
            .map_err(crate::kernel_error)?;
        self.get_session(session_id)
            .await?
            .ok_or_else(|| ApplicationError::not_found("The session was not found."))
    }

    /// Start a conversation over without deleting its stored history.
    ///
    /// `context_only` keeps the transcript readable and restarts only the
    /// model's prompt window; otherwise the transcript is cleared as well.
    /// Both modes leave every entry, content block and segment in place.
    pub async fn clear_session(
        &self,
        session_id: i64,
        context_only: bool,
    ) -> ApplicationResult<SessionResource> {
        let resource = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| ApplicationError::not_found("The session was not found."))?;
        let mode = if context_only {
            agena_storage::kernel::ClearMode::Context
        } else {
            agena_storage::kernel::ClearMode::History
        };
        self.session_kernel
            .sessions
            .clear(
                RequestId::new(),
                crate::kernel_session_id(session_id)?,
                mode,
            )
            .await
            .map_err(crate::kernel_error)?;
        Ok(resource)
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

fn validate_session_prompt_layer(layer: &PromptLayer) -> ApplicationResult<()> {
    layer
        .validate()
        .map_err(|error| ApplicationError::bad_request_error(&error))?;
    if let Some(id) = layer
        .order
        .iter()
        .find(|id| id.is_builtin() && !agena_runtime_contracts::identity::is_builtin_prompt(id))
    {
        return Err(ApplicationError::bad_request_error(
            &PromptError::UnknownFragment {
                scope: PromptScope::Session,
                id: id.to_string(),
            },
        ));
    }
    Ok(())
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
        external_agent: summary.external_agent,
        owner_bot_id: summary.owner_bot_id.clone(),
        owner_surface: summary.owner_surface.clone(),
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
        external_agent: None,
        owner_bot_id: None,
        owner_surface: None,
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
