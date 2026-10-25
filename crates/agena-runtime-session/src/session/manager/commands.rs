use super::*;
use crate::session::usage as context_usage;
use crate::{
    SessionExecutionCommandError, SessionExecutionCommandOutcome, SessionExecutionCommandService,
};
use agena_domain::{
    CancellationOutcome, CancellationResult, PermissionMode, PermissionReplyKind, PermissionScope,
    UserInputReplyKind,
};

impl SessionManager {
    pub async fn create_session(&self, request: SessionCreateRequest) -> Result<Session, AppError> {
        let manager = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = manager.create_session_inner(request).await;
            let _ = tx.send(result);
        });
        rx.await
            .map_err(|_| AppError::Internal("session creation supervisor stopped".into()))?
    }
    async fn create_session_inner(
        &self,
        request: SessionCreateRequest,
    ) -> Result<Session, AppError> {
        let workspace = self.current_workspace_id().await?;
        let state = self.state.load_full();
        let parent = request.parent_session_id.map(session_id).transpose()?;
        let mut configuration = SessionConfiguration {
            title_state: if request.auto_title {
                SessionTitleState::automatic()
            } else {
                Default::default()
            },
            parent_session_id: parent,
            relation_kind: if parent.is_some() {
                agena_domain::SessionRelationKind::Child
            } else {
                agena_domain::SessionRelationKind::Root
            },
            effective_workspace_root: Some(state.executor.workspace_root().to_path_buf()),
            ..Default::default()
        };
        if let Some(parent) = parent {
            let inherited = self
                .kernel()
                .reader
                .configuration(parent)
                .await
                .map_err(kernel_error)?;
            configuration.permission = inherited.permission.clone();
            configuration.permission_ceiling = inherited.permission_ceiling.clone();
        }
        let command = RequestId::new();
        let metadata = persist(|| {
            self.kernel().sessions.create_configured(
                command,
                workspace,
                request.title.clone(),
                configuration.clone(),
            )
        })
        .await
        .map_err(kernel_error)?
        .value;
        if let Some(model) = state
            .providers
            .resolve_default_model_selection(&state.config.default_selection)?
        {
            self.kernel()
                .sessions
                .update_selection(
                    RequestId::new(),
                    metadata.id,
                    SelectionChange::Model(Some(model)),
                )
                .await
                .map_err(kernel_error)?;
        }
        let patch = state
            .executor
            .plugin_manager()
            .dispatch_session_start(agena_plugin_host::SessionStartInput {
                session_id: metadata.id.get(),
                source: agena_plugin_host::SessionStartSource::Startup,
                workspace_root: state.executor.workspace_root().to_string_lossy().into(),
                model: None,
            })
            .await;
        if let Ok(patch) = patch {
            if let Some(context) = patch.additional_context {
                self.kernel()
                    .executor
                    .deliver(
                        RequestId::new(),
                        metadata.id,
                        format!("session-start-context:{}", metadata.id.get()),
                        serde_json::json!({"text":context}),
                    )
                    .await
                    .map_err(kernel_error)?;
            }
            if let Some(message) = patch.initial_user_message {
                if let Ok(options) = self.options_for_selection(&state.config.default_selection) {
                    self.submit(crate::SessionUserRunRequest::new(
                        metadata.id.get(),
                        options,
                        ComposerDocument(vec![ComposerNode::Text { text: message }]),
                    ))
                    .await?;
                } else {
                    let command = RequestId::new();
                    persist(|| {
                        self.kernel().executor.deliver(
                            command,
                            metadata.id,
                            format!("session-start-message:{}", metadata.id.get()),
                            serde_json::json!({"text":message,"wake":false}),
                        )
                    })
                    .await
                    .map_err(kernel_error)?;
                }
            }
        }
        self.get_session(metadata.id.get()).await
    }
    pub async fn rename_session(&self, id: i64, title: String) -> Result<Session, AppError> {
        self.kernel()
            .sessions
            .update_metadata(
                RequestId::new(),
                session_id(id)?,
                MetadataChange {
                    title: Some(title),
                    ..Default::default()
                },
            )
            .await
            .map_err(kernel_error)?;
        self.get_session(id).await
    }
    /// Persist a new model/option selection without starting a run.
    ///
    /// This is the explicit switch path: it refuses a target model whose prompt
    /// window cannot hold the conversation, so the user is told to compact
    /// before the selection is committed. A run that carries a smaller model
    /// instead goes through [`Self::update_session_selection`], which does not
    /// gate — the run loop compacts automatically there.
    pub async fn switch_session_selection(
        &self,
        id: i64,
        options: SessionRunOptions,
    ) -> Result<(), AppError> {
        self.ensure_model_fits_session(id, &options.model).await?;
        self.update_session_selection(id, options).await
    }
    pub async fn update_session_selection(
        &self,
        id: i64,
        options: SessionRunOptions,
    ) -> Result<(), AppError> {
        let session = session_id(id)?;
        let commands = &self.kernel().sessions;
        let changes = [
            SelectionChange::Model(Some(options.model)),
            SelectionChange::ThinkingMode(options.thinking_mode),
            SelectionChange::SpeedMode(options.speed_mode),
            SelectionChange::Verbosity(options.verbosity),
            SelectionChange::ParallelToolCalls(options.model_supports_parallel_tool_calls),
        ];
        // Admit every field command before waiting so the writer can group commit.
        // Await every receipt, including when another field command fails.
        let results = futures_util::future::join_all(
            changes
                .into_iter()
                .map(|change| commands.update_selection(RequestId::new(), session, change)),
        )
        .await;
        for result in results {
            result.map_err(kernel_error)?;
        }
        Ok(())
    }
    /// Refuse a switch the conversation provably cannot fit.
    ///
    /// Every other difference between routes degrades gracefully — history
    /// keeps its text and loses only what the new route cannot deliver. Size is
    /// the one condition that cannot be repaired silently: a model with a
    /// smaller prompt window would reject the request itself. Switching is
    /// therefore blocked with an actionable [`AppError::PromptBudgetExceeded`]
    /// until the conversation is compacted. Sending a message is *not* gated:
    /// the run loop still compacts automatically there, so a session that is
    /// already on the selected model is left alone.
    async fn ensure_model_fits_session(&self, id: i64, model: &ModelRef) -> Result<(), AppError> {
        let state = self.state.load_full();
        let selection = self
            .kernel()
            .reader
            .metadata(session_id(id)?)
            .await
            .map_err(kernel_error)?
            .selection
            .clone()
            .unwrap_or_default();
        let current_model = state
            .providers
            .resolve_default_model_selection(&selection)
            .map_err(|error| AppError::Config(error.to_string()))?;
        if current_model.as_ref() == Some(model) {
            return Ok(());
        }
        let metadata = state.providers.model_metadata(model).unwrap_or_default();
        let output = crate::session_output_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_output_tokens,
            None,
        );
        let Some(limit) = crate::prompt_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_input_tokens,
            Some(output),
        ) else {
            return Ok(());
        };
        let latest = self
            .kernel()
            .reader
            .latest_usage_round(session_id(id)?)
            .await
            .map_err(kernel_error)?;
        let current = latest
            .as_ref()
            .and_then(|round| round.provider_state.as_ref())
            .and_then(|value| {
                serde_json::from_value::<super::super::kernel::KernelProviderState>(value.clone())
                    .ok()
            })
            .and_then(|state| state.usage)
            .map(|usage| context_usage::context_tokens(&usage));
        let Some(current) = current else {
            return Ok(());
        };
        if current > u64::from(limit) {
            return Err(AppError::PromptBudgetExceeded {
                session_id: id,
                estimated_tokens: current,
                limit_tokens: u64::from(limit),
            });
        }
        Ok(())
    }
    pub async fn set_session_model_override(
        &self,
        id: i64,
        model: ModelRef,
    ) -> Result<Session, AppError> {
        self.ensure_model_fits_session(id, &model).await?;
        self.kernel()
            .sessions
            .update_selection(
                RequestId::new(),
                session_id(id)?,
                SelectionChange::Model(Some(model)),
            )
            .await
            .map_err(kernel_error)?;
        self.get_session(id).await
    }
    pub async fn set_session_permission(
        &self,
        id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> Result<(), AppError> {
        self.kernel()
            .sessions
            .configure(
                RequestId::new(),
                session_id(id)?,
                SessionConfigurationChange {
                    permission: Some(permission),
                    ..Default::default()
                },
            )
            .await
            .map_err(kernel_error)?;
        Ok(())
    }
    pub async fn fork_session(
        &self,
        request: crate::SessionForkRequest,
    ) -> Result<Session, AppError> {
        let session = session_id(request.session_id)?;
        self.check_version(session, request.expected_version)
            .await?;
        let through = match request.at_message_id {
            Some(id) => entry_id(id)?,
            None => {
                self.kernel()
                    .reader
                    .history(session, None, 1)
                    .await
                    .map_err(kernel_error)?
                    .entries
                    .first()
                    .ok_or_else(|| AppError::Config("cannot fork an empty session".into()))?
                    .id
            }
        };
        let commands = &self.kernel().sessions;
        let fork = match request.conversation_mode {
            Some(mode) => {
                commands
                    .fork_conversation(RequestId::new(), session, through, mode)
                    .await
            }
            None => commands.fork(RequestId::new(), session, through).await,
        }
        .map_err(kernel_error)?
        .value;
        if let Some(title) = request.title {
            self.kernel()
                .sessions
                .update_metadata(
                    RequestId::new(),
                    fork.id,
                    MetadataChange {
                        title: Some(title),
                        ..Default::default()
                    },
                )
                .await
                .map_err(kernel_error)?;
        }
        self.get_session(fork.id.get()).await
    }
    pub async fn rewind_session(
        &self,
        request: crate::SessionRewindRequest,
    ) -> Result<(Session, ComposerDocument), AppError> {
        let session = session_id(request.session_id)?;
        self.check_version(session, request.expected_version)
            .await?;
        let result = self
            .kernel()
            .sessions
            .rewind(RequestId::new(), session, entry_id(request.at_message_id)?)
            .await
            .map_err(kernel_error)?
            .value;
        Ok((
            self.get_session(result.session.id.get()).await?,
            result.document,
        ))
    }
    pub async fn delete_session(&self, id: i64) -> Result<(), AppError> {
        self.cancel_active_execution_with_outcome(id).await?;
        if let Some(monitors) = self.tool_executor().monitor_registry() {
            monitors.stop_session(id);
        }
        let command = RequestId::new();
        persist(|| {
            self.kernel()
                .sessions
                .delete(command, session_id(id).expect("validated session"))
        })
        .await
        .map_err(kernel_error)?;
        Ok(())
    }
    pub async fn withdraw_user_message(&self, id: i64, message: i64) -> Result<Session, AppError> {
        self.kernel()
            .sessions
            .withdraw(RequestId::new(), session_id(id)?, entry_id(message)?)
            .await
            .map_err(kernel_error)?;
        self.get_session(id).await
    }
    async fn check_version(
        &self,
        session: SessionId,
        expected: Option<i64>,
    ) -> Result<(), AppError> {
        if let Some(expected) = expected {
            let metadata = self
                .kernel()
                .reader
                .metadata(session)
                .await
                .map_err(kernel_error)?;
            if expected != metadata.metadata_version.get() {
                return Err(AppError::Conflict {
                    session_id: session.get(),
                    expected,
                    current: metadata.metadata_version.get(),
                });
            }
        }
        Ok(())
    }
    pub async fn active_execution(&self, id: i64) -> Option<agena_domain::ExecutionLifecycle> {
        self.kernel()
            .reader
            .control_snapshot(session_id(id).ok()?, 128)
            .await
            .ok()?
            .active_execution
            .clone()
    }
    pub async fn is_run_active(&self, id: i64) -> bool {
        self.active_execution(id).await.is_some()
    }
    pub async fn cancel_active_execution_with_outcome(
        &self,
        id: i64,
    ) -> Result<CancellationOutcome, AppError> {
        self.cancel_session_tree(id, false).await
    }

    pub async fn set_message_queue_paused(&self, id: i64, paused: bool) -> Result<(), AppError> {
        let session = session_id(id)?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        self.persist_message_queue_pause(session, paused).await?;
        if !paused {
            self.restore_queued_executions(session).await?;
        }
        Ok(())
    }

    async fn persist_message_queue_pause(
        &self,
        session: SessionId,
        paused: bool,
    ) -> Result<(), AppError> {
        let request = RequestId::new();
        persist(|| {
            self.kernel().sessions.configure(
                request,
                session,
                SessionConfigurationChange {
                    message_queue_paused: Some(paused),
                    ..Default::default()
                },
            )
        })
        .await
        .map_err(kernel_error)?;
        Ok(())
    }

    async fn cancel_session_tree(
        &self,
        id: i64,
        preserve_queue: bool,
    ) -> Result<CancellationOutcome, AppError> {
        let root = session_id(id)?;
        let mut sessions = std::collections::VecDeque::from([root]);
        let mut visited = std::collections::BTreeSet::new();
        let mut found = false;
        while let Some(session) = sessions.pop_front() {
            if !visited.insert(session) {
                continue;
            }
            if visited.len() > 10_000 {
                return Err(AppError::Config(
                    "cancellation task tree exceeds its bound".into(),
                ));
            }
            // Fence all future claims before signalling a running task. Its
            // completion/cancellation can otherwise release the next input.
            let dispatch = self.session_dispatch(session);
            let _submission = dispatch.submission.lock().await;
            self.persist_message_queue_pause(session, true).await?;
            let mut after = None;
            loop {
                let page = self
                    .kernel()
                    .reader
                    .active_executions(session, after, 128)
                    .await
                    .map_err(kernel_error)?;
                for execution in &page.executions {
                    found = true;
                    if preserve_queue && execution.phase == ExecutionPhase::Queued {
                        continue;
                    }
                    self.cancel_execution_with_outcome(session.get(), execution.id)
                        .await?;
                }
                if !page.has_more {
                    break;
                }
                after = page.next;
            }
            if let Some(monitors) = self.tool_executor().monitor_registry() {
                monitors.stop_session(session.get());
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
        Ok(if found {
            CancellationResult::CancellationRequested
        } else {
            CancellationResult::NotFound
        }
        .into())
    }

    pub async fn cancel_execution_with_outcome(
        &self,
        id: i64,
        execution: ExecutionId,
    ) -> Result<CancellationOutcome, AppError> {
        let session = session_id(id)?;
        let before = match self.kernel().reader.execution(session, execution).await {
            Ok(view) => view,
            Err(KernelError::NotFound(_)) => return Ok(CancellationResult::NotFound.into()),
            Err(error) => return Err(kernel_error(error)),
        };
        if before.phase.is_terminal() {
            return Ok(CancellationResult::AlreadyTerminal.into());
        }
        // Signal external cancellation promptly; the returned acknowledgement
        // still waits for the durable cancellation fence below.
        if let Some(run) = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&execution)
        {
            run.phase(agena_domain::ExecutionPhase::Cancelling);
            run.cancel.cancel();
        }
        let request = RequestId::new();
        persist(|| self.kernel().sessions.cancel(request, session, execution))
            .await
            .map_err(kernel_error)?;
        self.tool_executor()
            .plugin_manager()
            .dispatch_agent_cancel(agena_plugin_host::AgentCancelInput {
                session_id: id,
                execution_id: execution.to_string(),
            })
            .await;
        Ok(CancellationResult::CancellationRequested.into())
    }
    pub async fn steer_input(&self, id: i64, document: ComposerDocument) -> Result<(), AppError> {
        // Steering is a committed input execution queued behind the current
        // owner. It has a stable identity and cannot mutate frozen input.
        let session = self.get_session(id).await?;
        let options = self.options_for_selection(&session.execution.selection)?;
        self.submit(crate::SessionUserRunRequest::new(id, options, document))
            .await?;
        Ok(())
    }
    pub async fn reply_permission(
        &self,
        request: crate::SessionPermissionReplyRequest,
    ) -> Result<(), AppError> {
        let id = request.request.session_id;
        let session = session_id(id)?;
        let reply = request.request.reply;
        let interaction = InteractionId::try_from(
            reply
                .request_id
                .parse::<i64>()
                .map_err(|e| AppError::Config(e.to_string()))?,
        )
        .map_err(|e| AppError::Config(e.to_string()))?;
        let allow = match reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => true,
            PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => false,
            PermissionReplyKind::AutoApprove => {
                let view = persist(|| self.kernel().reader.interaction(session, interaction))
                    .await
                    .map_err(|error| interaction_error(error, "permission", interaction))?;
                if let Some(decision) = &view.decision {
                    return match decision {
                        InteractionDecision::Permission { reply: stored, .. }
                            if stored == &reply =>
                        {
                            Ok(())
                        }
                        _ => Err(interaction_not_pending("permission", interaction)),
                    };
                }
                let InteractionRequest::Permission { action_key, .. } = &view.request else {
                    return Err(AppError::Config("this is not a permission request".into()));
                };
                let action = serde_json::from_str(action_key)?;
                let state = self.state.load_full();
                self.classify_permission(
                    &action,
                    &agena_domain::PermissionDecision::Auto {
                        reason: "user requested automatic review".into(),
                    },
                    session,
                    &state,
                    &state.executor,
                )
                .await
                .ok_or_else(|| {
                    AppError::Config("automatic approval could not decide this request".into())
                })?
            }
        };
        let remember = match reply.kind {
            PermissionReplyKind::AllowAlways | PermissionReplyKind::DenyAlways => {
                Some(RememberPermission {
                    mode: if allow {
                        PermissionMode::Allow
                    } else {
                        PermissionMode::Deny
                    },
                    scope: reply.scope.unwrap_or(PermissionScope::Session),
                })
            }
            _ => None,
        };
        let command = RequestId::new();
        let decision = InteractionDecision::Permission {
            reply,
            allowed: allow,
        };
        let result = persist(|| {
            self.kernel().sessions.reply(
                command,
                session,
                interaction,
                decision.clone(),
                remember.clone(),
            )
        })
        .await;
        if let Err(error) = result {
            // Concurrent automatic reviews can finish with different model
            // verdicts. The complete user reply identifies its durable winner;
            // a replay must reuse that verdict rather than replace it.
            if matches!(error, KernelError::Conflict(_))
                && matches!(
                    &decision,
                    InteractionDecision::Permission { reply, .. }
                        if reply.kind == PermissionReplyKind::AutoApprove
                )
            {
                let stored = persist(|| self.kernel().reader.interaction(session, interaction))
                    .await
                    .map_err(|error| interaction_error(error, "permission", interaction))?;
                if matches!(
                    (&stored.decision, &decision),
                    (
                        Some(InteractionDecision::Permission { reply: winner, .. }),
                        InteractionDecision::Permission { reply, .. }
                    ) if winner == reply
                ) {
                    return Ok(());
                }
            }
            return Err(interaction_error(error, "permission", interaction));
        }
        // The committed decision is the acknowledgement. Its execution owner
        // can now advance independently; a later read is not part of deciding
        // whether this reply succeeded.
        Ok(())
    }
    pub async fn reply_user_input(
        &self,
        request: crate::SessionExecutionReplyRequest<agena_domain::UserInputReply>,
    ) -> Result<(), AppError> {
        let session = session_id(request.session_id)?;
        let reply = request.reply;
        let interaction = InteractionId::try_from(
            reply
                .request_id
                .parse::<i64>()
                .map_err(|e| AppError::Config(e.to_string()))?,
        )
        .map_err(|e| AppError::Config(e.to_string()))?;
        let view = self
            .kernel()
            .reader
            .interaction(session, interaction)
            .await
            .map_err(|error| interaction_error(error, "user input", interaction))?;
        let InteractionRequest::Input { request: input } = &view.request else {
            return Err(AppError::Config("this is not a user input request".into()));
        };
        validate_answers(input, &reply)?;
        let decision = match reply.kind {
            UserInputReplyKind::Submit => {
                InteractionDecision::Input(serde_json::to_string(&reply)?)
            }
            UserInputReplyKind::Cancel | UserInputReplyKind::Timeout => InteractionDecision::Cancel,
        };
        self.kernel()
            .sessions
            .reply(RequestId::new(), session, interaction, decision, None)
            .await
            .map_err(|error| interaction_error(error, "user input", interaction))?;
        Ok(())
    }
    pub async fn mark_interactive_request_presented(
        &self,
        id: i64,
        request: String,
    ) -> Result<(), AppError> {
        let interaction = InteractionId::try_from(
            request
                .parse::<i64>()
                .map_err(|e| AppError::Config(e.to_string()))?,
        )
        .map_err(|e| AppError::Config(e.to_string()))?;
        self.kernel()
            .sessions
            .present_interaction(RequestId::new(), session_id(id)?, interaction)
            .await
            .map_err(|error| interaction_error(error, "interactive", interaction))?;
        Ok(())
    }
}
fn interaction_not_pending(kind: &str, interaction: InteractionId) -> AppError {
    AppError::InteractiveRequestNotPending {
        request_kind: kind.into(),
        request_id: interaction.get().to_string(),
    }
}
fn interaction_error(error: KernelError, kind: &str, interaction: InteractionId) -> AppError {
    match error {
        KernelError::NotFound(_) | KernelError::Ownership | KernelError::Conflict(_) => {
            interaction_not_pending(kind, interaction)
        }
        other => kernel_error(other),
    }
}
fn command_error(error: AppError) -> SessionExecutionCommandError {
    SessionExecutionCommandError::from_failure(error.failure())
}
#[async_trait::async_trait]
impl SessionExecutionCommandService for SessionManager {
    async fn edit_queued_message(
        &self,
        request: crate::SessionQueuedMessageEditRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        let session = session_id(request.session_id).map_err(command_error)?;
        let revision = agena_storage::kernel::EntryRevision::try_from(request.expected_revision)
            .map_err(|error| command_error(AppError::Config(error.to_string())))?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        // Later submissions can select another model. Edit attachments using
        // the route the queued supervisor will actually run, rather than the
        // session's most recently selected model or reloaded provider config.
        let supervised = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&request.execution_id)
            .filter(|run| run.session == session)
            .map(|run| (run.model.clone(), run.state.clone()));
        let (model, state) = match supervised {
            Some(value) => value,
            None => {
                let metadata = self
                    .kernel()
                    .reader
                    .metadata(session)
                    .await
                    .map_err(|error| command_error(kernel_error(error)))?;
                let options = self
                    .options_for_selection(&metadata.selection.clone().unwrap_or_default())
                    .map_err(command_error)?;
                (options.model, self.state.load_full())
            }
        };
        let (document, blobs) = self
            .prepare_user_media_with_state(
                session,
                &model,
                request.document,
                request.request_id,
                state,
            )
            .await
            .map_err(command_error)?;
        persist(|| {
            self.kernel().sessions.edit_queued_with_blobs(
                request.request_id,
                session,
                request.execution_id,
                revision,
                document.clone(),
                &blobs,
            )
        })
        .await
        .map_err(|error| {
            command_error(match error {
                KernelError::Conflict(message) => AppError::PersistenceConflict(message),
                other => kernel_error(other),
            })
        })?;
        Ok(SessionExecutionCommandOutcome::completed(
            request.session_id,
        ))
    }

    async fn create_session(
        &self,
        r: SessionCreateRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        Ok(SessionExecutionCommandOutcome::completed(
            SessionManager::create_session(self, r)
                .await
                .map_err(command_error)?
                .id,
        ))
    }
    async fn submit_user_run(
        &self,
        r: crate::SessionUserRunRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        self.submit(r).await.map_err(command_error)
    }
    async fn continue_session(
        &self,
        r: SessionExecutionRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        self.start_continue_session(r).await.map_err(command_error)
    }
    async fn compact_session(
        &self,
        r: SessionExecutionRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        self.start_compact_session(r).await.map_err(command_error)
    }
    async fn fork_session(
        &self,
        r: crate::SessionForkRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        Ok(SessionExecutionCommandOutcome::completed(
            SessionManager::fork_session(self, r)
                .await
                .map_err(command_error)?
                .id,
        ))
    }
    async fn rewind_session(
        &self,
        r: crate::SessionRewindRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        Ok(SessionExecutionCommandOutcome::completed(
            SessionManager::rewind_session(self, r)
                .await
                .map_err(command_error)?
                .0
                .id,
        ))
    }
    async fn import_session_file(
        &self,
        request_id: agena_domain::CommandId,
        path: &std::path::Path,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        let workspace = self.current_workspace_id().await.map_err(command_error)?;
        // Parsing stays in this future until a self-owned disk staging
        // database exists. Detached writer work never needs the caller's file.
        let imported = persist(|| {
            self.kernel()
                .sessions
                .import_archive(request_id, workspace, path)
        })
        .await
        .map_err(|error| SessionExecutionCommandError::from_failure(archive_failure(error)))?;
        Ok(SessionExecutionCommandOutcome::completed(
            imported.value.id.get(),
        ))
    }
    async fn steer_input(
        &self,
        id: i64,
        document: ComposerDocument,
    ) -> Result<(), SessionExecutionCommandError> {
        SessionManager::steer_input(self, id, document)
            .await
            .map_err(command_error)
    }
    async fn reply_permission(
        &self,
        r: crate::SessionPermissionReplyRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        let id = r.request.session_id;
        SessionManager::reply_permission(self, r)
            .await
            .map_err(command_error)?;
        Ok(SessionExecutionCommandOutcome::completed(id))
    }
    async fn reply_user_input(
        &self,
        r: crate::SessionExecutionReplyRequest<agena_domain::UserInputReply>,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        let id = r.session_id;
        SessionManager::reply_user_input(self, r)
            .await
            .map_err(command_error)?;
        Ok(SessionExecutionCommandOutcome::completed(id))
    }
    async fn mark_interactive_request_presented(
        &self,
        id: i64,
        r: String,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        SessionManager::mark_interactive_request_presented(self, id, r)
            .await
            .map_err(command_error)?;
        Ok(SessionExecutionCommandOutcome::completed(id))
    }
    async fn update_session_selection(
        &self,
        id: i64,
        r: SessionRunOptions,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        SessionManager::switch_session_selection(self, id, r)
            .await
            .map_err(command_error)?;
        Ok(SessionExecutionCommandOutcome::completed(id))
    }
    async fn set_session_permission(
        &self,
        id: i64,
        r: agena_domain::PermissionConfig,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
        SessionManager::set_session_permission(self, id, r)
            .await
            .map_err(command_error)?;
        Ok(SessionExecutionCommandOutcome::completed(id))
    }
}
#[async_trait::async_trait]
impl crate::SessionExecutionControl for SessionManager {
    async fn set_message_queue_paused(
        &self,
        id: i64,
        paused: bool,
    ) -> Result<(), crate::SessionExecutionControlError> {
        SessionManager::set_message_queue_paused(self, id, paused)
            .await
            .map_err(|e| crate::SessionExecutionControlError::internal_error(&e))
    }
    async fn active_execution(&self, id: i64) -> Option<agena_domain::ExecutionLifecycle> {
        SessionManager::active_execution(self, id).await
    }
    async fn cancel_execution_with_outcome(
        &self,
        id: i64,
        e: ExecutionId,
    ) -> Result<CancellationOutcome, crate::SessionExecutionControlError> {
        self.set_message_queue_paused(id, true)
            .await
            .map_err(|e| crate::SessionExecutionControlError::internal_error(&e))?;
        SessionManager::cancel_execution_with_outcome(self, id, e)
            .await
            .map_err(|e| crate::SessionExecutionControlError::internal_error(&e))
    }
    async fn cancel_session_with_outcome(
        &self,
        id: i64,
    ) -> Result<CancellationOutcome, crate::SessionExecutionControlError> {
        self.cancel_session_tree(id, true)
            .await
            .map_err(|e| crate::SessionExecutionControlError::internal_error(&e))
    }
    async fn list_scheduled_jobs(
        &self,
    ) -> Result<Vec<agena_scheduler::ScheduledJob>, crate::SessionExecutionControlError> {
        match self.tool_executor().scheduler() {
            Some(scheduler) => scheduler
                .list()
                .await
                .map_err(|e| crate::SessionExecutionControlError::internal_error(&e)),
            None => Ok(Vec::new()),
        }
    }
    fn scheduler_available(&self) -> bool {
        self.tool_executor().scheduler().is_some()
    }
    async fn selected_model(
        &self,
        id: i64,
    ) -> Result<Option<ModelRef>, crate::SessionExecutionControlError> {
        let session = self
            .get_session(id)
            .await
            .map_err(|e| crate::SessionExecutionControlError::internal_error(&e))?;
        self.state
            .load()
            .providers
            .resolve_default_model_selection(&session.execution.selection)
            .map_err(|e| crate::SessionExecutionControlError::internal_error(&e))
    }
}
pub(super) fn validate_answers(
    input: &agena_domain::UserInputRequest,
    reply: &agena_domain::UserInputReply,
) -> Result<(), AppError> {
    if reply.kind != UserInputReplyKind::Submit {
        return Ok(());
    }
    if reply.answers.len() != input.questions.len() {
        return Err(AppError::Config(
            "answer every question before submitting".into(),
        ));
    }
    for (key, answers) in &reply.answers {
        let index = key
            .parse::<usize>()
            .map_err(|_| AppError::Config("answer references an unknown question".into()))?;
        let question = input
            .questions
            .get(index)
            .ok_or_else(|| AppError::Config("answer references an unknown question".into()))?;
        if answers.is_empty() || answers.iter().any(|a| a.trim().is_empty()) {
            return Err(AppError::Config(
                "an answer must include a selected option or text".into(),
            ));
        }
        if !question.multiple && answers.len() != 1 {
            return Err(AppError::Config(format!(
                "question {index} accepts exactly one answer"
            )));
        }
        if question.custom_option_index().is_none()
            && answers.iter().any(|a| {
                !question
                    .options
                    .iter()
                    .filter(|o| !o.custom)
                    .any(|o| o.label.trim() == a.trim())
            })
        {
            return Err(AppError::Config("answer includes an unknown option".into()));
        }
    }
    Ok(())
}
