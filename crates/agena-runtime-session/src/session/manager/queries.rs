use super::super::kernel::KernelProviderState;
use super::super::model::SessionExecutionContext as ToolContext;
use super::*;
use crate::session::usage as context_usage;
use crate::{SessionExecutionContext, SessionPresentation, SessionQueryError, SessionQueryService};
use agena_domain::{
    PendingInteractiveRequestContext, PermissionAction, PermissionRequest, SessionListRequest,
    SessionSummary, SessionUsage, WorkflowState,
};

impl SessionManager {
    pub async fn get_session(&self, id: i64) -> Result<Session, AppError> {
        let session = session_id(id)?;
        let (metadata, summary, configuration, pending) = tokio::try_join!(
            self.kernel().reader.metadata(session),
            self.kernel().reader.summary(session),
            self.kernel().reader.configuration(session),
            self.kernel().reader.pending_interactions(session, None, 1)
        )
        .map_err(kernel_error)?;
        let (root_id, depth) = (summary.root_session_id.get(), i64::from(summary.depth));
        let state = self.state.load_full();
        let selection = state
            .config
            .default_selection
            .overlay_with_cascade(&metadata.selection.clone().unwrap_or_default());
        let context = ToolContext {
            scope_session_id: Some(id),
            conversation: configuration.conversation.clone(),
            selection,
            effective_permission: state
                .config
                .permission
                .merged_with(&configuration.permission),
            permission_ceiling: configuration.permission_ceiling.clone(),
            capability_denied_tool_names: configuration.capability_denied_tool_names.clone(),
            effective_workspace_root: configuration.effective_workspace_root.clone(),
        };
        let agent = configuration.agent.as_ref();
        Ok(Session {
            id,
            parent_id: configuration.parent_session_id.map(SessionId::get),
            root_id,
            depth,
            workspace_id: metadata.workspace_id,
            title: metadata.title.clone(),
            favorite: metadata.favorite,
            pinned: metadata.pinned,
            version: metadata.metadata_version.get(),
            relation_kind: configuration.relation_kind,
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            agent_name: agent.map(|t| t.name.clone()),
            created_at: timestamp(summary.created_at_ms),
            updated_at: timestamp(summary.updated_at_ms),
            message_count: summary.user_message_count,
            workflow: if !pending.interactions.is_empty() {
                WorkflowState::AwaitingInteraction
            } else {
                WorkflowState::Quiescent
            },
            execution: context,
            agent_status: agent.map(|t| t.status),
            agent_started_at_ms: agent.and_then(|t| t.started_at_ms),
            agent_finished_at_ms: agent.and_then(|t| t.finished_at_ms),
            agent_failure: agent.and_then(|t| t.failure.clone()),
        })
    }
    pub(super) async fn execution_context_value(
        &self,
        id: SessionId,
    ) -> Result<ToolContext, AppError> {
        self.execution_context_with_state(id, &self.state.load_full())
            .await
    }
    pub(super) async fn execution_context_with_state(
        &self,
        id: SessionId,
        state: &ExecutionState,
    ) -> Result<ToolContext, AppError> {
        let (metadata, configuration) = tokio::try_join!(
            self.kernel().reader.metadata(id),
            self.kernel().reader.configuration(id)
        )
        .map_err(kernel_error)?;
        Ok(ToolContext {
            scope_session_id: Some(id.get()),
            conversation: configuration.conversation.clone(),
            selection: state
                .config
                .default_selection
                .overlay_with_cascade(&metadata.selection.clone().unwrap_or_default()),
            effective_permission: state
                .config
                .permission
                .merged_with(&configuration.permission),
            permission_ceiling: configuration.permission_ceiling.clone(),
            capability_denied_tool_names: configuration.capability_denied_tool_names.clone(),
            effective_workspace_root: configuration.effective_workspace_root.clone(),
        })
    }
    pub async fn list_session_summaries(
        &self,
        request: SessionListRequest,
    ) -> Result<Vec<SessionSummary>, AppError> {
        let page = self
            .kernel()
            .reader
            .query_summaries(SummaryQuery {
                workspace_id: Some(self.current_workspace_id().await?),
                parent_id: request.parent_id.map(session_id).transpose()?,
                roots_only: request.roots_only,
                exclude_subagents: !request.include_subagents,
                search: request.search,
                offset: request.offset,
                limit: request.limit.unwrap_or(200).clamp(1, 256) as usize,
                ..Default::default()
            })
            .await
            .map_err(kernel_error)?;
        let mut result = Vec::with_capacity(page.sessions.len());
        for summary in &page.sessions {
            result.push(self.project_summary(summary).await?);
        }
        Ok(result)
    }
    pub async fn get_session_summary(&self, id: i64) -> Result<SessionSummary, AppError> {
        let summary = self
            .kernel()
            .reader
            .summary(session_id(id)?)
            .await
            .map_err(kernel_error)?;
        self.project_summary(&summary).await
    }
    async fn project_summary(
        &self,
        summary: &SessionSummaryView,
    ) -> Result<SessionSummary, AppError> {
        let configuration = self
            .kernel()
            .reader
            .configuration(summary.id)
            .await
            .map_err(kernel_error)?;
        let (root_id, depth) = (summary.root_session_id.get(), i64::from(summary.depth));
        Ok(SessionSummary {
            id: summary.id.get(),
            parent_id: configuration.parent_session_id.map(SessionId::get),
            depth,
            root_id,
            workspace_id: summary.workspace_id,
            title: summary.title.clone(),
            favorite: summary.favorite,
            pinned: summary.pinned,
            version: summary.metadata_version.get(),
            relation_kind: configuration.relation_kind,
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            agent_name: configuration.agent.as_ref().map(|t| t.name.clone()),
            agent_status: configuration.agent.as_ref().map(|t| t.status),
            created_at: timestamp(summary.created_at_ms),
            updated_at: timestamp(summary.updated_at_ms),
            message_count: summary.user_message_count,
            child_session_count: summary.child_session_count,
            last_message_at: summary.last_message_at_ms.map(timestamp),
        })
    }
    pub async fn list_session_tree(&self, root: i64) -> Result<Vec<SessionSummary>, AppError> {
        let root = session_id(root)?;
        let mut frontier = std::collections::VecDeque::from([root]);
        let mut result = Vec::new();
        let mut visited = std::collections::BTreeSet::new();
        while let Some(id) = frontier.pop_front() {
            if !visited.insert(id) {
                return Err(AppError::Internal(
                    "session lineage contains a cycle".into(),
                ));
            }
            if result.len() >= 10_000 {
                return Err(AppError::Config(
                    "session tree is too large; read children in bounded pages".into(),
                ));
            }
            result.push(self.get_session_summary(id.get()).await?);
            let mut before = None;
            loop {
                let page = self
                    .kernel()
                    .reader
                    .query_summaries(SummaryQuery {
                        workspace_id: Some(self.current_workspace_id().await?),
                        parent_id: Some(id),
                        before,
                        limit: 200,
                        ..Default::default()
                    })
                    .await
                    .map_err(kernel_error)?;
                frontier.extend(page.sessions.iter().map(|s| s.id));
                if !page.has_more {
                    break;
                }
                before = page.next;
            }
        }
        Ok(result)
    }
    pub async fn is_descendant_session(
        &self,
        descendant: i64,
        ancestor: i64,
    ) -> Result<bool, AppError> {
        let mut id = session_id(descendant)?;
        let ancestor = session_id(ancestor)?;
        for _ in 0..64 {
            let cfg = self
                .kernel()
                .reader
                .configuration(id)
                .await
                .map_err(kernel_error)?;
            let Some(parent) = cfg.parent_session_id else {
                return Ok(false);
            };
            if parent == ancestor {
                return Ok(true);
            }
            id = parent;
        }
        Err(AppError::Internal(
            "session lineage exceeds maximum depth".into(),
        ))
    }
    pub async fn session_usage_async(&self, session: &Session) -> Result<SessionUsage, AppError> {
        let state = self.state.load_full();
        let model = state
            .providers
            .resolve_default_model_selection(&session.execution.selection)?;
        let metadata = model
            .as_ref()
            .and_then(|m| state.providers.model_metadata(m).ok())
            .unwrap_or_default();
        let latest = self
            .kernel()
            .reader
            .latest_usage_round(session_id(session.id)?)
            .await
            .map_err(kernel_error)?;
        let usage = latest
            .as_ref()
            .and_then(|r| r.provider_state.as_ref())
            .and_then(|v| serde_json::from_value::<KernelProviderState>(v.clone()).ok())
            .and_then(|s| s.usage);
        let measured = usage.as_ref().map(context_usage::prompt_tokens);
        let current = usage.as_ref().map_or(0, context_usage::context_tokens);
        let output = crate::session_output_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_output_tokens,
            None,
        );
        let limit = crate::prompt_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_input_tokens,
            Some(output),
        );
        Ok(SessionUsage {
            measured_prompt_tokens: measured,
            current_tokens: current,
            projected_tokens: None,
            input_limit_tokens: limit.map(u64::from),
            limit_tokens: limit.map(u64::from),
            limit_basis: None,
            reserved_tokens: Some(output),
            model_context_window_tokens: metadata.limits.context_window_tokens,
            model_max_input_tokens: metadata.limits.max_input_tokens,
            model_max_output_tokens: metadata.limits.max_output_tokens,
        })
    }
    pub fn session_model_status(
        &self,
        session: &Session,
    ) -> Result<agena_domain::ModelSelectionConfig, AppError> {
        let mut options = self.options_for_selection(&session.execution.selection)?;
        self.apply_model_mode_requests(&mut options)?;
        Ok(agena_domain::ModelSelectionConfig {
            provider: Some(options.model.provider_id.to_string()),
            adapter: options.model.adapter_id.map(|a| a.to_string()),
            model: Some(options.model.model_id.to_string()),
            thinking_mode: options.thinking_mode,
            speed_mode: options.speed_mode,
            verbosity: options.verbosity,
            parallel_tool_calls: options.model_supports_parallel_tool_calls,
        })
    }
    async fn cost_summary(&self, id: i64) -> Result<agena_domain::SessionCostSummary, AppError> {
        let id = session_id(id)?;
        let mut after = None;
        let mut accumulator = super::super::cost::CostAccumulator::default();
        loop {
            let page = self
                .kernel()
                .reader
                .rounds(id, after, 200)
                .await
                .map_err(kernel_error)?;
            if page.is_empty() {
                break;
            }
            for round in page.iter() {
                if let Some(value) = &round.provider_state {
                    let state: KernelProviderState = serde_json::from_value(value.clone())?;
                    if let Some(usage) = state.usage {
                        accumulator.observe(
                            state.model.provider_id.as_ref(),
                            state.model.model_id.as_ref(),
                            &usage,
                        );
                    }
                }
            }
            after = page.last().map(|r| r.id);
        }
        Ok(accumulator.finish())
    }
    pub async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, AppError> {
        let mut accumulator = crate::usage_stats::UsageStatsAccumulator::default();
        let workspace = self.current_workspace_id().await?;
        let mut before = None;
        loop {
            let sessions = self
                .kernel()
                .reader
                .summaries(workspace, before, 200)
                .await
                .map_err(kernel_error)?;
            for session in &sessions.sessions {
                let config = self
                    .kernel()
                    .reader
                    .configuration(session.id)
                    .await
                    .map_err(kernel_error)?;
                let mut after = None;
                loop {
                    let rounds = self
                        .kernel()
                        .reader
                        .rounds(session.id, after, 200)
                        .await
                        .map_err(kernel_error)?;
                    if rounds.is_empty() {
                        break;
                    }
                    for round in rounds.iter() {
                        if let Some(value) = &round.provider_state {
                            let state: KernelProviderState = serde_json::from_value(value.clone())?;
                            if let Some(usage) = state.usage {
                                accumulator.observe(
                                    &crate::UsageStatRecord {
                                        session_id: session.id.get(),
                                        session_title: session.title.clone(),
                                        is_subagent: config.relation_kind.is_subagent(),
                                        created_at: timestamp(round.created_at_ms),
                                        provider_id: state.model.provider_id.to_string(),
                                        model_id: state.model.model_id.to_string(),
                                        usage,
                                    },
                                    &query,
                                );
                            }
                        }
                    }
                    after = rounds.last().map(|r| r.id);
                }
            }
            if !sessions.has_more {
                break;
            }
            before = sessions.next;
        }
        Ok(accumulator.finish(&query, Utc::now()))
    }
    pub async fn pending_interactive_requests(
        &self,
        root: &SessionControlSnapshot,
    ) -> Result<Vec<PendingInteractiveRequestContext>, AppError> {
        let mut sessions = std::collections::VecDeque::from([root.metadata.id]);
        let mut visited = std::collections::BTreeSet::new();
        let mut result = Vec::new();
        while let Some(session) = sessions.pop_front() {
            if !visited.insert(session) {
                continue;
            }
            if visited.len() > 10_000 {
                return Err(AppError::Config(
                    "pending interaction agent tree exceeds its bound".into(),
                ));
            }
            // Root attention belongs to the same cut as its execution state.
            // Each descendant has its own short control snapshot; an agent tree
            // is not one globally atomic projection.
            let descendant;
            let snapshot = if session == root.metadata.id {
                root
            } else {
                descendant = self
                    .kernel()
                    .reader
                    .control_snapshot(session, 128 - result.len())
                    .await
                    .map_err(kernel_error)?;
                &descendant
            };
            let config = &snapshot.configuration;
            let id = session.get();
            let page = &snapshot.pending_interactions;
            for view in &page.interactions {
                let request = match &view.request {
                    InteractionRequest::Permission {
                        action_key,
                        description,
                    } => PermissionRequest {
                        request_id: view.id.get().to_string(),
                        session_id: Some(id),
                        action: serde_json::from_str::<PermissionAction>(action_key)?,
                        related_actions: Vec::new(),
                        requested_actions: Vec::new(),
                        reason: description.clone(),
                        explanation: String::new(),
                        source: Some("kernel".into()),
                        scope: None,
                        operator: None,
                        trace: Vec::new(),
                        created_at: timestamp(view.created_at_ms),
                    }
                    .into(),
                    InteractionRequest::Input { request } => {
                        let mut request = request.clone();
                        request.request_id = view.id.get().to_string();
                        request.session_id = Some(id);
                        request.into()
                    }
                };
                result.push(PendingInteractiveRequestContext {
                    session_id: id,
                    parent_session_id: config.parent_session_id.map(SessionId::get),
                    agent_name: config.agent.as_ref().map(|t| t.name.clone()),
                    request,
                });
                if result.len() >= 128 {
                    return Ok(result);
                }
            }
            let mut before = None;
            loop {
                let page = self
                    .kernel()
                    .reader
                    .query_summaries(SummaryQuery {
                        parent_id: Some(session),
                        before,
                        limit: 200,
                        ..Default::default()
                    })
                    .await
                    .map_err(kernel_error)?;
                sessions.extend(
                    page.sessions
                        .iter()
                        .filter(|s| s.relation_kind.is_subagent())
                        .map(|s| s.id),
                );
                if !page.has_more {
                    break;
                }
                before = page.next;
            }
        }
        Ok(result)
    }
}
fn timestamp(ms: i64) -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or(chrono::DateTime::UNIX_EPOCH)
}
fn query_error(error: AppError) -> SessionQueryError {
    SessionQueryError::internal_error(&error)
}
#[async_trait::async_trait]
impl SessionQueryService for SessionManager {
    async fn list_session_summaries(
        &self,
        r: SessionListRequest,
    ) -> Result<Vec<SessionSummary>, SessionQueryError> {
        SessionManager::list_session_summaries(self, r)
            .await
            .map_err(query_error)
    }
    async fn session_presentation(
        &self,
        id: i64,
    ) -> Result<SessionPresentation, SessionQueryError> {
        let session = self.get_session(id).await.map_err(query_error)?;
        Ok(SessionPresentation {
            id,
            parent_id: session.parent_id,
            workspace_id: session.workspace_id,
            title: session.title,
            version: session.version,
            created_at: session.created_at,
            updated_at: session.updated_at,
            message_count: session.message_count as usize,
            workflow_state: session.workflow,
        })
    }
    async fn list_session_tree(&self, id: i64) -> Result<Vec<SessionSummary>, SessionQueryError> {
        SessionManager::list_session_tree(self, id)
            .await
            .map_err(query_error)
    }
    async fn export_session_to(
        &self,
        id: i64,
        mut sink: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> Result<(), SessionQueryError> {
        let session = SessionId::try_from(id)
            .map_err(|error| KernelError::Invalid(error.to_string()))
            .map_err(|error| SessionQueryError {
                failure: Box::new(archive_failure(error)),
            })?;
        self.kernel()
            .reader
            .export_jsonl(session, &mut sink)
            .await
            .map(|_| ())
            .map_err(|error| SessionQueryError {
                failure: Box::new(archive_failure(error)),
            })
    }
    async fn latest_event_seq(&self, id: i64) -> Result<Option<i64>, SessionQueryError> {
        Ok(Some(
            *self
                .kernel()
                .reader
                .control_version(session_id(id).map_err(query_error)?)
                .await
                .map_err(|e| query_error(kernel_error(e)))?,
        ))
    }
    async fn session_usage(&self, id: i64) -> Result<SessionUsage, SessionQueryError> {
        let session = self.get_session(id).await.map_err(query_error)?;
        self.session_usage_async(&session)
            .await
            .map_err(query_error)
    }
    async fn session_cost_summary(
        &self,
        id: i64,
    ) -> Result<agena_domain::SessionCostSummary, SessionQueryError> {
        self.cost_summary(id).await.map_err(query_error)
    }
    async fn usage_stats(
        &self,
        q: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, SessionQueryError> {
        SessionManager::usage_stats(self, q)
            .await
            .map_err(query_error)
    }
    async fn pending_interactive_requests(
        &self,
        snapshot: &SessionControlSnapshot,
    ) -> Result<Vec<PendingInteractiveRequestContext>, SessionQueryError> {
        SessionManager::pending_interactive_requests(self, snapshot)
            .await
            .map_err(query_error)
    }
    fn execution_context(
        &self,
        snapshot: &SessionControlSnapshot,
    ) -> Result<SessionExecutionContext, SessionQueryError> {
        let state = self.state.load_full();
        let configuration = &snapshot.configuration;
        let selection = state
            .config
            .default_selection
            .overlay_with_cascade(&snapshot.metadata.selection.clone().unwrap_or_default());
        let agent = configuration.agent.as_ref();
        Ok(SessionExecutionContext {
            conversation: configuration.conversation.clone(),
            workflow_state: if snapshot.pending_interactions.interactions.is_empty() {
                WorkflowState::Quiescent
            } else {
                WorkflowState::AwaitingInteraction
            },
            agent_id: crate::identity::AGENA_AGENT_ID.into(),
            selected_permission: configuration.permission.clone(),
            effective_permission: state
                .config
                .permission
                .merged_with(&configuration.permission),
            permission_ceiling: configuration.permission_ceiling.clone(),
            model_provider_id: selection.provider,
            model_adapter_id: selection.adapter,
            model_id: selection.model,
            model_thinking_mode: selection.thinking_mode,
            model_speed_mode: selection.speed_mode,
            model_verbosity: selection.verbosity,
            model_parallel_tool_calls: selection.parallel_tool_calls,
            effective_workspace_root: configuration
                .effective_workspace_root
                .as_ref()
                .map(|p| p.to_string_lossy().into()),
            agent_name: agent.map(|agent| agent.name.clone()),
            agent_status: agent.map(|agent| agent.status),
            agent_started_at: agent.and_then(|agent| agent.started_at_ms).map(timestamp),
            agent_finished_at: agent.and_then(|agent| agent.finished_at_ms).map(timestamp),
            agent_failure: agent.and_then(|agent| agent.failure.clone()),
        })
    }
    async fn is_descendant_session(&self, d: i64, a: i64) -> Result<bool, SessionQueryError> {
        SessionManager::is_descendant_session(self, d, a)
            .await
            .map_err(query_error)
    }
}
