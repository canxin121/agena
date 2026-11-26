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
    /// The active registration is installed before a claim can create effects,
    /// and removed only after the driver and post-run hooks return. Combining
    /// that exact ownership with the persisted claim epoch proves retirement
    /// without inferring completion from session idleness. A foreign epoch can
    /// never prove normal settlement, including after a graceful restart.
    pub async fn execution_attempt(
        &self,
        id: i64,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> Result<Option<crate::SessionExecutionAttempt>, AppError> {
        let session = session_id(id)?;
        let read_attempt = || {
            read_with_busy_retry(session, "execution_attempt", || async {
                match self
                    .kernel()
                    .reader
                    .execution_attempt(session, execution, generation)
                    .await
                {
                    Ok(view) => Ok(Some((*view).clone())),
                    Err(KernelError::NotFound(_) | KernelError::Ownership) => Ok(None),
                    Err(error) => Err(error),
                }
            })
        };
        let Some(attempt) = read_attempt().await? else {
            return Ok(None);
        };
        let epoch = self.kernel().reader.epoch();
        let live = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&execution)
            .filter(|run| run.session == session)
            .cloned();
        let mut current_generation = live.as_ref().map_or(attempt.current_generation, |run| {
            attempt.current_generation.max(run.generation)
        });
        let mut settlement = if current_generation > generation {
            ExecutionAttemptSettlement::Superseded
        } else if attempt.phase == ExecutionPhase::Interrupted
            || attempt.owner_epoch.is_some_and(|owner| owner != epoch)
        {
            ExecutionAttemptSettlement::Recovered
        } else if !attempt.phase.is_terminal() {
            ExecutionAttemptSettlement::Pending
        } else if live.as_ref().is_some_and(|run| {
            run.generation == generation && !run.settled.load(std::sync::atomic::Ordering::Acquire)
        }) || self.has_pending_agent_settlement(session, execution, generation)
            || self
                .attempt_controls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains_key(&(id, execution, generation))
        {
            ExecutionAttemptSettlement::Terminal
        } else {
            // An unclaimed, cancelled input has no provider effects to drain.
            // Claimed attempts must belong to this live kernel (checked above).
            ExecutionAttemptSettlement::Settled
        };
        if settlement == ExecutionAttemptSettlement::Settled {
            // Generation publication and live ownership are separate reads.
            // Revalidate after inspecting owners so even a rapidly admitted
            // and retired retry cannot make an older registration look done.
            let Some(latest) = read_attempt().await? else {
                return Ok(None);
            };
            current_generation = current_generation.max(latest.current_generation);
            if current_generation > generation {
                settlement = ExecutionAttemptSettlement::Superseded;
            }
        }
        Ok(Some(crate::SessionExecutionAttempt {
            session_id: id,
            execution_id: execution,
            message_id: attempt.message_id,
            attempt_generation: generation,
            current_attempt_generation: current_generation,
            phase: attempt.phase,
            settlement,
            kernel_epoch: epoch,
            owner_epoch: attempt.owner_epoch,
        }))
    }

    /// Managed session artifacts belong to the workspace recorded on the
    /// session, not the server's default executor or a child task's cwd.
    /// Resolve the same durable identity used by the session-media reader.
    pub async fn session_workspace_root(&self, id: i64) -> Result<std::path::PathBuf, AppError> {
        self.session_workspace_root_optional(id)
            .await?
            .ok_or_else(|| {
                AppError::Config(
                    "This session has no project workspace; select an explicit project.".into(),
                )
            })
    }

    pub async fn session_workspace_root_optional(
        &self,
        id: i64,
    ) -> Result<Option<std::path::PathBuf>, AppError> {
        let session = session_id(id)?;
        let metadata = read_with_busy_retry(session, "metadata", || {
            self.kernel().reader.metadata(session)
        })
        .await?;
        match metadata.workspace_id {
            Some(workspace) => self.workspace_root_by_id(workspace).await.map(Some),
            None => Ok(None),
        }
    }

    pub(super) async fn workspace_root_by_id(
        &self,
        workspace_id: i64,
    ) -> Result<std::path::PathBuf, AppError> {
        let source = self
            .acp_workspace_source
            .as_ref()
            .unwrap_or(&self.kernel_runtime);
        let path = source
            .workspaces
            .path_by_id(workspace_id)
            .await
            .map_err(|error| AppError::internal_error(&error))?
            .ok_or_else(|| {
                AppError::Config("The session's owning workspace was not found.".into())
            })?;
        if self.is_acp_scope() {
            self.kernel_runtime
                .register_acp_workspace(workspace_id, &path)
                .await
                .map_err(kernel_error)?;
        }
        Ok(std::path::PathBuf::from(path))
    }

    async fn execution_workspace_root(
        &self,
        workspace_id: Option<i64>,
        configured: Option<&std::path::Path>,
    ) -> Result<Option<std::path::PathBuf>, AppError> {
        match (workspace_id, configured) {
            (None, _) => Ok(None),
            (Some(_), Some(path)) => Ok(Some(path.to_path_buf())),
            // HTTP-created and imported sessions may omit the override. Their
            // tools still belong to their recorded workspace, not the default
            // executor's workspace or whichever session ran most recently.
            (Some(workspace), None) => self.workspace_root_by_id(workspace).await.map(Some),
        }
    }

    /// Whether a live (not deleted) conversation still exists.
    ///
    /// Callers that cached a session id — channel scopes, reminders — use this
    /// instead of matching an error string: a deleted conversation reads as
    /// absent, and any other failure stays a real error.
    pub async fn session_exists(&self, id: i64) -> Result<bool, AppError> {
        match self.kernel().reader.metadata(session_id(id)?).await {
            Ok(_) => Ok(true),
            Err(KernelError::NotFound(_)) => Ok(false),
            Err(error) => Err(kernel_error(error)),
        }
    }

    pub async fn get_session(&self, id: i64) -> Result<Session, AppError> {
        self.get_session_with_state(id, None).await
    }

    pub(super) async fn get_session_with_state(
        &self,
        id: i64,
        state_override: Option<Arc<ExecutionState>>,
    ) -> Result<Session, AppError> {
        let session = session_id(id)?;
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        let metadata = &snapshot.metadata;
        let summary = &snapshot.summary;
        let configuration = &snapshot.configuration;
        let (root_id, depth) = (summary.root_session_id.get(), i64::from(summary.depth));
        let state = state_override.unwrap_or_else(|| {
            if summary.workspace_id.is_none() {
                self.global_state.load_full()
            } else {
                self.state.load_full()
            }
        });
        let profile = self.conversation_profile(configuration, &state).await?;
        let selection = state.config.selection_with_profile(
            configuration,
            &metadata.selection.clone().unwrap_or_default(),
            &profile.definition,
        );
        let permission = state
            .config
            .permission_with_profile(configuration, &profile.definition);
        let permission_ceiling_layers = profile.permission_ceilings(configuration)?;
        let mut context = ToolContext {
            global: metadata.workspace_id.is_none(),
            owner: configuration.owner.clone(),
            scope_session_id: Some(id),
            relation_kind: configuration.relation_kind,
            conversation: configuration.conversation.clone(),
            prompts: configuration.prompts.clone(),
            profile: Some(profile.name),
            profile_is_default: configuration.profile.is_none(),
            profile_from_bot: profile.from_bot,
            profile_prompts: profile.definition.prompts,
            project_instructions_on_demand: profile.definition.context.project_instructions
                == Some(agena_runtime_config::ProjectInstructionsMode::OnDemand),
            preload_skills: profile
                .definition
                .context
                .preload_skills
                .unwrap_or_default(),
            agent_assignment: configuration
                .agent
                .as_ref()
                .map(|agent| agent.assignment.clone())
                .unwrap_or_default(),
            max_turns: profile.definition.max_turns.or(state.config.max_turns),
            selection,
            effective_permission: permission,
            permission_ceiling: configuration.permission_ceiling.clone(),
            permission_ceiling_layers,
            channel_authority: configuration.channel_authority.clone(),
            channel_verifier: self.channel_verifier.get().cloned(),
            capability_denied_tool_names: super::agents::delegation_capability_denials(
                configuration,
            ),
            effective_workspace_root: self
                .execution_workspace_root(
                    metadata.workspace_id,
                    configuration.effective_workspace_root.as_deref(),
                )
                .await?,
        };
        context.channel_authority = self
            .channel_authority_for_execution(
                session,
                self.running_execution(session).await?,
                configuration.channel_authority.as_ref(),
                configuration.relation_kind == agena_domain::SessionRelationKind::Subagent,
            )
            .await?;
        Self::apply_channel_execution_context(
            &mut context,
            configuration.permission_profile.is_some(),
        );
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
            external_agent: configuration.external_agent,
            agent_name: agent.map(|t| t.name.clone()),
            created_at: timestamp(summary.created_at_ms),
            updated_at: timestamp(summary.updated_at_ms),
            message_count: summary.user_message_count,
            workflow: workflow_from_snapshot(&snapshot),
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
        let state = self.state_for_session(id).await?;
        self.execution_context_with_state(id, &state).await
    }

    /// Resolve the ordered system prompt before per-run plugin hooks.
    pub async fn prompt_preview(
        &self,
        id: SessionId,
        model: &agena_domain::ModelRef,
    ) -> Result<agena_prompt_layers::PromptResolution, AppError> {
        let state = self.state_for_session(id).await?;
        let context = self.execution_context_with_state(id, &state).await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let tools_enabled = state
            .providers
            .agena_tool_mode(model)
            .map(|mode| !mode.is_disabled())
            .unwrap_or(true);
        let workspace = context
            .effective_workspace_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bot = self.bot_prompt_layer(&context).await?;
        let inputs = super::session_prompt::PromptInputs {
            profile: &context.profile_prompts,
            project_instructions_on_demand: context.project_instructions_on_demand,
            preload_skills: &context.preload_skills,
            assignment: &context.agent_assignment,
            profile_from_bot: context.profile_from_bot,
            session: &context.prompts,
            bot: bot.as_ref(),
            request_layer: None,
            model,
            session_id: id.get(),
            workspace,
            global: context.global,
            tools_enabled,
        };
        self.session_system_prompt(&executor, &state, &inputs, context.profile.as_deref())
            .await
    }
    /// A shared transcript has one claimed execution. Queued input and a
    /// later speaker must never supply identity to the current tool callback.
    pub(super) async fn running_execution(
        &self,
        session: SessionId,
    ) -> Result<Option<ExecutionId>, AppError> {
        let mut after = None;
        let mut running = None;
        loop {
            let page = read_with_busy_retry(session, "active_executions", || {
                self.kernel().reader.active_executions(session, after, 128)
            })
            .await?;
            for execution in &page.executions {
                if execution.phase == ExecutionPhase::Running {
                    if running.replace(execution.id).is_some() {
                        return Err(AppError::Config(
                            "ambiguous channel execution identity".into(),
                        ));
                    }
                }
            }
            if !page.has_more {
                return Ok(running);
            }
            after = page.next;
            if after.is_none() {
                return Err(AppError::Config(
                    "incomplete active execution snapshot".into(),
                ));
            }
        }
    }

    pub(super) async fn channel_authority_for_execution(
        &self,
        session: SessionId,
        execution: Option<ExecutionId>,
        baseline: Option<&agena_domain::ChannelAuthority>,
        delegated: bool,
    ) -> Result<Option<agena_domain::ChannelAuthority>, AppError> {
        self.channel_authority_for_execution_inner(session, execution, baseline, delegated, 0)
            .await
    }

    async fn channel_authority_for_execution_inner(
        &self,
        session: SessionId,
        execution: Option<ExecutionId>,
        baseline: Option<&agena_domain::ChannelAuthority>,
        delegated: bool,
        depth: usize,
    ) -> Result<Option<agena_domain::ChannelAuthority>, AppError> {
        if depth > 16 {
            return Err(AppError::Config(
                "channel continuation origin is too deep".into(),
            ));
        }
        let Some(baseline) = baseline else {
            return Ok(None);
        };
        let shared = Self::is_shared_channel(baseline, delegated);
        if let Some(execution) = execution {
            let input = self
                .kernel()
                .reader
                .execution(session, execution)
                .await
                .map_err(kernel_error)?;
            let entry = self
                .kernel()
                .reader
                .entry(session, input.message_id)
                .await
                .map_err(kernel_error)?;
            let mut authority = Self::message_channel_authority(&entry)?;
            if shared
                && (input.purpose == ExecutionPurpose::Continuation || input.generation.get() > 1)
            {
                let receipt = self
                    .kernel()
                    .reader
                    .resumed_delivery(session, execution, input.generation)
                    .await
                    .map_err(kernel_error)?;
                if let Some(receipt) = receipt.as_ref()
                    && receipt
                        .payload
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        == Some("agent_result")
                {
                    let child = receipt
                        .payload
                        .get("agent_id")
                        .and_then(serde_json::Value::as_i64)
                        .ok_or_else(|| {
                            AppError::Config("agent continuation has no child identity".into())
                        })?;
                    let child = session_id(child)?;
                    let child_config = self
                        .kernel()
                        .reader
                        .configuration(child)
                        .await
                        .map_err(kernel_error)?;
                    if child_config.parent_session_id != Some(session)
                        || child_config.relation_kind != agena_domain::SessionRelationKind::Subagent
                    {
                        return Err(AppError::Config(
                            "continuation is not from this parent's delegated agent".into(),
                        ));
                    }
                    let origin = child_config
                        .agent
                        .as_ref()
                        .and_then(|agent| agent.origin_message_id)
                        .ok_or_else(|| {
                            AppError::Config(
                                "agent continuation has no persisted launch origin".into(),
                            )
                        })?;
                    if receipt
                        .payload
                        .get("message_id")
                        .and_then(serde_json::Value::as_i64)
                        != Some(origin.get())
                    {
                        return Err(AppError::Config(
                            "agent continuation changed its launch origin".into(),
                        ));
                    }
                    let child_authority =
                        child_config.channel_authority.as_ref().ok_or_else(|| {
                            AppError::Config(
                                "agent continuation has no inherited channel authority".into(),
                            )
                        })?;
                    let origin_entry = self
                        .kernel()
                        .reader
                        .entry(session, origin)
                        .await
                        .map_err(kernel_error)?;
                    let origin_authority = match Self::message_channel_authority(&origin_entry)? {
                        Some(authority) => Some(authority),
                        None if origin_entry.execution_id != execution => {
                            Box::pin(self.channel_authority_for_execution_inner(
                                session,
                                Some(origin_entry.execution_id),
                                Some(baseline),
                                false,
                                depth + 1,
                            ))
                            .await?
                        }
                        None => None,
                    };
                    if origin_authority.as_ref() != Some(child_authority) {
                        return Err(AppError::Config(
                            "agent continuation caller differs from its verified launch input"
                                .into(),
                        ));
                    }
                    authority = Some(child_authority.clone());
                }
            }
            if let Some(authority) = authority {
                Self::validate_channel_execution_scope(baseline, &authority, shared)?;
                return Ok(Some(authority));
            }
            if shared && input.purpose != ExecutionPurpose::Compaction {
                return Err(AppError::Config(
                    "shared channel input has no verified sender authority".into(),
                ));
            }
        }
        let mut authority = baseline.clone();
        if shared {
            // Legacy shared sessions stored their first speaker. Treat that
            // record only as a room boundary while retaining its transcript.
            authority.actor = "@shared".into();
            authority.role = agena_domain::ChannelRole::Member;
            authority.group_ids.clear();
            authority.profile_ids.clear();
            authority.profile_fingerprint.clear();
            authority.permission_overlay = Default::default();
            authority.permission_complete = false;
            authority.expires_at_ms = 0;
        }
        Ok(Some(authority))
    }

    fn message_channel_authority(
        entry: &EntryView,
    ) -> Result<Option<agena_domain::ChannelAuthority>, AppError> {
        if let EntryPayload::User {
            author: Some(author),
            ..
        } = &entry.payload
        {
            return Ok(Self::author_channel_authority(author)?.cloned());
        }
        Ok(None)
    }

    fn author_channel_authority(
        author: &agena_domain::MessageAuthor,
    ) -> Result<Option<&agena_domain::ChannelAuthority>, AppError> {
        if let Some(authority) = &author.channel_authority {
            if author.id != authority.actor
                || author.group_ids != authority.group_ids
                || !matches!(&author.kind, agena_domain::AuthorKind::ChannelMember { role } if *role == authority.role)
            {
                return Err(AppError::Config(
                    "message author differs from its verified execution identity".into(),
                ));
            }
            return Ok(Some(authority));
        }
        Ok(None)
    }

    fn is_shared_channel(baseline: &agena_domain::ChannelAuthority, delegated: bool) -> bool {
        baseline.session_mode == agena_domain::ChannelSessionMode::PerConversation
            && baseline.actor != "@service"
            && !baseline.source_admin
            && !delegated
    }

    fn validate_channel_execution_scope(
        baseline: &agena_domain::ChannelAuthority,
        authority: &agena_domain::ChannelAuthority,
        shared: bool,
    ) -> Result<(), AppError> {
        if authority.binding_id != baseline.binding_id
            || authority.identity != baseline.identity
            || authority.resource != baseline.resource
            || authority.thread != baseline.thread
            || authority.kind != baseline.kind
            || authority.epoch != baseline.epoch
            || authority.session_mode != baseline.session_mode
            || authority.source_admin != baseline.source_admin
            || (!baseline.platform.is_empty() && authority.platform != baseline.platform)
            || (!shared && authority.actor != baseline.actor)
        {
            return Err(AppError::Config(
                "execution identity does not belong to this channel scope".into(),
            ));
        }
        Ok(())
    }

    fn apply_channel_execution_context(context: &mut ToolContext, named_session_permission: bool) {
        let Some(authority) = context.channel_authority.as_ref() else {
            return;
        };
        for fragment in &mut context.prompts.inline {
            if fragment.body == super::session_prompt::LEGACY_CHANNEL_SYSTEM_PROMPT {
                fragment.body = super::session_prompt::CHANNEL_SYSTEM_PROMPT.into();
            }
        }
        if named_session_permission {
            context
                .permission_ceiling_layers
                .push(context.effective_permission.clone());
        }
        context.effective_permission = if authority.permission_complete {
            authority.permission_overlay.clone()
        } else {
            context
                .effective_permission
                .merged_with(&authority.permission_overlay)
        };
        if authority.source_admin || authority.grant.source_workspace {
            if let Some(root) = &authority.source_workspace_root {
                context.effective_workspace_root = Some(root.clone());
            }
        } else {
            context.effective_workspace_root = Some(authority.memory_workspace_root.clone());
        }
    }

    /// Read the persisted sender for one exact execution, including after restart.
    pub async fn channel_execution_authority(
        &self,
        id: i64,
        execution: ExecutionId,
    ) -> Result<Option<agena_domain::ChannelAuthority>, AppError> {
        let session = session_id(id)?;
        let configuration = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        self.channel_authority_for_execution(
            session,
            Some(execution),
            configuration.channel_authority.as_ref(),
            configuration.relation_kind == agena_domain::SessionRelationKind::Subagent,
        )
        .await
    }

    pub async fn verify_channel_execution(
        &self,
        id: i64,
        execution: ExecutionId,
    ) -> Result<Option<agena_domain::ChannelAuthority>, AppError> {
        let authority = self.channel_execution_authority(id, execution).await?;
        if let Some(authority) = &authority {
            self.channel_verifier
                .get()
                .ok_or_else(|| AppError::Config("channel authorization unavailable".into()))?
                .authorize(authority)
                .await
                .map_err(AppError::Config)?;
        }
        Ok(authority)
    }

    pub(super) async fn execution_context_with_state(
        &self,
        id: SessionId,
        state: &ExecutionState,
    ) -> Result<ToolContext, AppError> {
        let execution = self.running_execution(id).await?;
        self.execution_context_for_execution(id, state, execution)
            .await
    }

    pub(super) async fn execution_context_for_execution(
        &self,
        id: SessionId,
        state: &ExecutionState,
        execution: Option<ExecutionId>,
    ) -> Result<ToolContext, AppError> {
        self.build_execution_context(id, state, execution, None)
            .await
    }

    pub(super) async fn execution_context_for_submission(
        &self,
        id: SessionId,
        state: &ExecutionState,
        author: Option<&agena_domain::MessageAuthor>,
    ) -> Result<ToolContext, AppError> {
        if let Some(authority) = author
            .map(Self::author_channel_authority)
            .transpose()?
            .flatten()
        {
            return self
                .build_execution_context(id, state, None, Some(authority))
                .await;
        }
        self.execution_context_with_state(id, state).await
    }

    async fn build_execution_context(
        &self,
        id: SessionId,
        state: &ExecutionState,
        execution: Option<ExecutionId>,
        submitted_authority: Option<&agena_domain::ChannelAuthority>,
    ) -> Result<ToolContext, AppError> {
        let reader = &self.kernel().reader;
        let (metadata, configuration) = tokio::try_join!(
            read_with_busy_retry(id, "metadata", || reader.metadata(id)),
            read_with_busy_retry(id, "configuration", || reader.configuration(id)),
        )?;
        let delegated = configuration.relation_kind.is_subagent();
        let baseline = configuration.channel_authority.as_ref();
        let authority = if let Some(submitted) = submitted_authority {
            let baseline = baseline.ok_or_else(|| {
                AppError::Config("verified channel input requires a channel session".into())
            })?;
            Self::validate_channel_execution_scope(
                baseline,
                submitted,
                Self::is_shared_channel(baseline, delegated),
            )?;
            Some(submitted.clone())
        } else {
            self.channel_authority_for_execution(id, execution, baseline, delegated)
                .await?
        };
        if let Some(worktree) = configuration
            .agent
            .as_ref()
            .and_then(|agent| agent.assignment.worktree.as_ref())
            && (metadata.workspace_id.is_none()
                || authority.is_some()
                || configuration.effective_workspace_root.as_ref() != Some(&worktree.path))
        {
            return Err(AppError::Config(
                "the agent's worktree requires its original project session".into(),
            ));
        }
        if let Some(authority) = &authority {
            self.channel_verifier
                .get()
                .ok_or_else(|| {
                    AppError::Config("channel authorization service is unavailable".into())
                })?
                .authorize(authority)
                .await
                .map_err(AppError::Config)?;
        }
        let profile = self.conversation_profile(&configuration, state).await?;
        let permission = state
            .config
            .permission_with_profile(&configuration, &profile.definition);
        let permission_ceiling_layers = profile.permission_ceilings(&configuration)?;
        let selection = state.config.selection_with_profile(
            &configuration,
            &metadata.selection.clone().unwrap_or_default(),
            &profile.definition,
        );
        let mut context = ToolContext {
            global: metadata.workspace_id.is_none(),
            owner: configuration.owner.clone(),
            scope_session_id: Some(id.get()),
            relation_kind: configuration.relation_kind,
            conversation: configuration.conversation.clone(),
            prompts: configuration.prompts.clone(),
            profile: Some(profile.name),
            profile_is_default: configuration.profile.is_none(),
            profile_from_bot: profile.from_bot,
            profile_prompts: profile.definition.prompts,
            project_instructions_on_demand: profile.definition.context.project_instructions
                == Some(agena_runtime_config::ProjectInstructionsMode::OnDemand),
            preload_skills: profile
                .definition
                .context
                .preload_skills
                .unwrap_or_default(),
            agent_assignment: configuration
                .agent
                .as_ref()
                .map(|agent| agent.assignment.clone())
                .unwrap_or_default(),
            max_turns: profile.definition.max_turns.or(state.config.max_turns),
            selection,
            effective_permission: permission,
            permission_ceiling: configuration.permission_ceiling.clone(),
            permission_ceiling_layers,
            channel_authority: authority,
            channel_verifier: self.channel_verifier.get().cloned(),
            capability_denied_tool_names: super::agents::delegation_capability_denials(
                &configuration,
            ),
            effective_workspace_root: self
                .execution_workspace_root(
                    metadata.workspace_id,
                    configuration.effective_workspace_root.as_deref(),
                )
                .await?,
        };
        Self::apply_channel_execution_context(
            &mut context,
            configuration.permission_profile.is_some(),
        );
        Ok(context)
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
                exclude_bot_owned: true,
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
            result.push(Self::project_summary(summary));
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
        Ok(Self::project_summary(&summary))
    }
    fn project_summary(summary: &SessionSummaryView) -> SessionSummary {
        SessionSummary {
            id: summary.id.get(),
            parent_id: summary.parent_session_id.map(SessionId::get),
            depth: i64::from(summary.depth),
            root_id: summary.root_session_id.get(),
            workspace_id: summary.workspace_id,
            title: summary.title.clone(),
            favorite: summary.favorite,
            pinned: summary.pinned,
            version: summary.metadata_version.get(),
            relation_kind: summary.relation_kind,
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            agent_name: summary.agent_name.clone(),
            agent_status: summary.agent_status,
            created_at: timestamp(summary.created_at_ms),
            updated_at: timestamp(summary.updated_at_ms),
            message_count: summary.user_message_count,
            child_session_count: summary.child_session_count,
            last_message_at: summary.last_message_at_ms.map(timestamp),
        }
    }
    pub async fn list_session_tree(&self, root: i64) -> Result<Vec<SessionSummaryView>, AppError> {
        const MAX_TREE_SESSIONS: usize = 10_000;

        let root = session_id(root)?;
        let root = self
            .kernel()
            .reader
            .summary(root)
            .await
            .map_err(kernel_error)?;
        let workspace = root.workspace_id;
        let mut frontier = std::collections::VecDeque::from([(*root).clone()]);
        drop(root);
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        while let Some(summary) = frontier.pop_front() {
            let id = summary.id;
            if !visited.insert(id) {
                return Err(AppError::Internal(
                    "session lineage contains a cycle".into(),
                ));
            }
            let has_children = summary.child_session_count != 0;
            result.push(summary);
            if !has_children {
                continue;
            }
            let mut before = None;
            loop {
                let page = self
                    .kernel()
                    .reader
                    .query_summaries(SummaryQuery {
                        workspace_id: workspace,
                        include_global: workspace.is_none(),
                        parent_id: Some(id),
                        before,
                        limit: 200,
                        ..Default::default()
                    })
                    .await
                    .map_err(kernel_error)?;
                if result.len() + frontier.len() + page.sessions.len() > MAX_TREE_SESSIONS {
                    return Err(AppError::Config(
                        "session tree is too large; read children in bounded pages".into(),
                    ));
                }
                frontier.extend(page.sessions.iter().cloned());
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
        if session.external_agent.is_some() {
            let id = session_id(session.id)?;
            let context = read_with_busy_retry(id, "external_agent_context", || {
                self.kernel().reader.configuration(id)
            })
            .await?
            .external_agent_state
            .context;
            return Ok(SessionUsage {
                measured_prompt_tokens: None,
                current_tokens: context.map_or(0, |usage| usage.used_tokens),
                projected_tokens: None,
                input_limit_tokens: None,
                limit_tokens: context.map(|usage| u64::from(usage.window_tokens)),
                limit_basis: context.map(|_| agena_domain::SessionUsageLimitBasis::ContextWindow),
                reserved_tokens: None,
                model_context_window_tokens: context.map(|usage| usage.window_tokens),
                model_max_input_tokens: None,
                model_max_output_tokens: None,
                last_cache_read_tokens: None,
                last_cache_input_tokens: None,
                last_cache_hit_rate: None,
                last_performance: None,
            });
        }
        let state = if session.execution.global {
            self.global_state.load_full()
        } else {
            self.state.load_full()
        };
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
        // Performance is useful even when the provider omitted token usage.
        // Read it independently so an unmeasured request cannot erase the last
        // known context size, or reuse an older request's speed.
        let latest_round = self
            .kernel()
            .reader
            .latest_round(session_id(session.id)?)
            .await
            .map_err(kernel_error)?;
        let last_performance = latest_round
            .as_ref()
            .and_then(|round| round.provider_state.as_ref())
            .and_then(KernelProviderState::decode_lossy)
            .and_then(|state| state.usage)
            .and_then(|usage| usage.request_performance);
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
        // Cache efficiency of the last request, using the same definition as the
        // cost report so both surfaces describe one number.
        let cache_usage = usage.as_ref().map(|usage| usage.request_usage_snapshot());
        let cache_input = cache_usage.as_ref().map(|usage| usage.prompt_tokens());
        let cache_read = cache_usage.as_ref().map(|usage| usage.cache_read_tokens);
        let cache_hit_rate = cache_input
            .filter(|input| *input > 0)
            .map(|input| cache_read.unwrap_or_default() as f64 / input as f64);
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
            last_cache_read_tokens: cache_read,
            last_cache_input_tokens: cache_input,
            last_cache_hit_rate: cache_hit_rate,
            last_performance,
        })
    }
    pub fn session_model_status(
        &self,
        session: &Session,
    ) -> Result<agena_domain::ModelSelectionConfig, AppError> {
        let state = if session.execution.global {
            self.global_state.load_full()
        } else {
            self.state.load_full()
        };
        let mut options =
            self.options_for_selection_with_state(&session.execution.selection, &state)?;
        self.apply_model_modes_with_state(&mut options, &state)?;
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
        let mut accumulator = super::super::cost::CostAccumulator::default();
        self.kernel()
            .reader
            .visit_session_usage(id, &mut |stored| {
                if let Some(record) = crate::UsageStatRecord::from_stored(stored) {
                    accumulator.observe(&record.provider_id, &record.model_id, &record.usage);
                }
            })
            .await
            .map_err(kernel_error)?;
        let mut summary = accumulator.finish();
        summary.external_agent_cost = read_with_busy_retry(id, "external_agent_cost", || {
            self.kernel().reader.configuration(id)
        })
        .await?
        .external_agent_state
        .details
        .cost
        .clone();
        Ok(summary)
    }
    pub async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, AppError> {
        let mut accumulator = crate::usage_stats::UsageStatsAccumulator::default();
        self.kernel()
            .reader
            .visit_usage_observations(&query, &mut |stored| {
                if let Some(record) = crate::UsageStatRecord::from_stored(stored) {
                    accumulator.observe(&record, &query);
                }
            })
            .await
            .map_err(kernel_error)?;
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
                    agent_name: config.agent.as_ref().map(|agent| agent.name.clone()),
                    request,
                });
                if result.len() >= 128 {
                    return Ok(result);
                }
            }
            for request in self.external_input_requests(session, config) {
                result.push(PendingInteractiveRequestContext {
                    session_id: id,
                    parent_session_id: config.parent_session_id.map(SessionId::get),
                    agent_name: config.agent.as_ref().map(|agent| agent.name.clone()),
                    request: request.into(),
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
fn workflow_from_snapshot(snapshot: &SessionControlSnapshot) -> WorkflowState {
    if !snapshot.pending_interactions.interactions.is_empty() {
        WorkflowState::AwaitingInteraction
    } else if matches!(
        &snapshot.active_execution,
        Some(agena_domain::ExecutionLifecycle::Active {
            phase: agena_domain::ExecutionPhase::ExecutingTools,
            ..
        })
    ) {
        WorkflowState::ToolPending
    } else {
        WorkflowState::Quiescent
    }
}

fn timestamp(ms: i64) -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or(chrono::DateTime::UNIX_EPOCH)
}
#[cfg(test)]
mod tests;

fn query_error(error: AppError) -> SessionQueryError {
    SessionQueryError::internal_error(&error)
}
#[async_trait::async_trait]
impl SessionQueryService for SessionManager {
    async fn prompt_preview(
        &self,
        id: i64,
    ) -> Result<agena_prompt_layers::PromptResolution, SessionQueryError> {
        let session = super::session_id(id).map_err(query_error)?;
        let context = self
            .execution_context_value(session)
            .await
            .map_err(query_error)?;
        let state = self.state_for_session(session).await.map_err(query_error)?;
        let model = state
            .providers
            .resolve_default_model_selection(&context.selection)
            .ok()
            .flatten()
            .ok_or_else(|| {
                SessionQueryError::internal("no model route is available for this session")
            })?;
        SessionManager::prompt_preview(self, session, &model)
            .await
            .map_err(query_error)
    }
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
    async fn list_session_tree(
        &self,
        id: i64,
    ) -> Result<Vec<SessionSummaryView>, SessionQueryError> {
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
    async fn execution_context(
        &self,
        snapshot: &SessionControlSnapshot,
    ) -> Result<SessionExecutionContext, SessionQueryError> {
        let state = if snapshot.metadata.workspace_id.is_none() {
            self.global_state.load_full()
        } else {
            self.state.load_full()
        };
        let configuration = &snapshot.configuration;
        let selected_selection = snapshot.metadata.selection.clone().unwrap_or_default();
        let profile = self
            .conversation_profile(configuration, &state)
            .await
            .map_err(query_error)?;
        let inherited_selection = state.config.selection_with_profile(
            configuration,
            &agena_domain::ExecutionSelection::default(),
            &profile.definition,
        );
        let selection = inherited_selection.overlay_with_cascade(&selected_selection);
        let agent = configuration.agent.as_ref();
        Ok(SessionExecutionContext {
            profile: Some(profile.name.clone()),
            profile_is_default: configuration.profile.is_none(),
            conversation: configuration.conversation.clone(),
            prompts: configuration.prompts.clone(),
            workflow_state: workflow_from_snapshot(snapshot),
            agent_id: crate::identity::AGENA_AGENT_ID.into(),
            selected_permission: configuration.permission.clone(),
            permission_profile: configuration.permission_profile.clone(),
            inherited_permission: profile
                .definition
                .effective_permission(&state.config.permission),
            effective_permission: state
                .config
                .permission_with_profile(configuration, &profile.definition),
            permission_ceiling: configuration.permission_ceiling.clone(),
            permission_ceiling_layers: profile
                .permission_ceilings(configuration)
                .map_err(query_error)?,
            selected_selection,
            inherited_selection,
            model_provider_id: selection.provider,
            model_adapter_id: selection.adapter,
            model_id: selection.model,
            model_thinking_mode: selection.thinking_mode,
            model_speed_mode: selection.speed_mode,
            model_verbosity: selection.verbosity,
            model_parallel_tool_calls: selection.parallel_tool_calls,
            effective_workspace_root: snapshot.metadata.workspace_id.and(
                configuration
                    .effective_workspace_root
                    .as_ref()
                    .map(|p| p.to_string_lossy().into()),
            ),
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
