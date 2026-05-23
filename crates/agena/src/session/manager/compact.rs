use super::*;

const COMPACTION_AGENT: &str = "compaction";
const DEFAULT_TAIL_USER_MESSAGES: usize = 2;

struct CompactionRuntimeInstall {
    summary: String,
    tail_start_message_id: Option<i64>,
    compacted_at_message_id: Option<i64>,
    compacted_by_message_id: Option<i64>,
    strategy: PromptCompactionStrategy,
    touched_messages: Vec<Message>,
}

impl SessionManager {
    #[tracing::instrument(skip(self, request), fields(session_id = request.session_id))]
    pub async fn compact_session(
        &self,
        request: SessionCompactRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        let (control, _steer_rx) = self.run_registry.register(session_id).await;
        crate::metrics::session_started();
        let manager = self.background_handle();
        let task_control = control.clone();
        let result = tokio::task::spawn(async move {
            manager
                .compact_session_inner(request, task_control.clone())
                .await
        })
        .await
        .map_err(|err| AppError::Internal(format!("compact task failed: {err}")))
        .and_then(std::convert::identity);
        crate::metrics::session_finished();
        self.run_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    async fn compact_session_inner(
        &self,
        mut request: SessionCompactRequest,
        control: Arc<RunControl>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.options, state.clone())
            .await?;
        if session.messages.is_empty() {
            return Ok(session);
        }

        let original_execution = session.runtime.execution.clone();
        let original_options =
            self.apply_execution_context_to_run_options(&session, request.options)?;
        let compacted_at = session.messages.last().map(|message| message.id);
        let tail_start = select_tail_start_message_id(session.messages.as_slice());

        match self
            .try_remote_compact(&session, &original_options, state.clone())
            .await
        {
            Ok(Some(summary)) => {
                let session = self
                    .install_compaction_runtime(
                        session,
                        original_execution,
                        CompactionRuntimeInstall {
                            summary,
                            tail_start_message_id: tail_start,
                            compacted_at_message_id: compacted_at,
                            compacted_by_message_id: compacted_at,
                            strategy: PromptCompactionStrategy::Remote,
                            touched_messages: Vec::new(),
                        },
                        state,
                    )
                    .await?;
                return Ok(session);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    target: "agena::session::compact",
                    session_id = request.session_id,
                    error = %err,
                    "remote compaction failed; falling back to local compaction agent"
                );
            }
        }

        Box::pin(self.local_compact_with_agent(
            session,
            original_options,
            original_execution,
            tail_start,
            compacted_at,
            state,
            control,
        ))
        .await
    }

    pub(super) async fn auto_compact_session(
        &self,
        session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
        control: Arc<RunControl>,
    ) -> Result<Session, AppError> {
        if session.messages.is_empty() {
            return Ok(session);
        }

        let original_execution = session.runtime.execution.clone();
        let original_options = options.clone();
        let compacted_at = session.messages.last().map(|message| message.id);
        let tail_start = select_tail_start_message_id(session.messages.as_slice());

        match self
            .try_remote_compact(&session, &original_options, state.clone())
            .await
        {
            Ok(Some(summary)) => {
                let session = self
                    .install_compaction_runtime(
                        session,
                        original_execution,
                        CompactionRuntimeInstall {
                            summary,
                            tail_start_message_id: tail_start,
                            compacted_at_message_id: compacted_at,
                            compacted_by_message_id: compacted_at,
                            strategy: PromptCompactionStrategy::Remote,
                            touched_messages: Vec::new(),
                        },
                        state,
                    )
                    .await?;
                return Ok(session);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    target: "agena::session::compact",
                    session_id = session.id,
                    error = %err,
                    "automatic remote compaction failed; falling back to local compaction agent"
                );
            }
        }

        Box::pin(self.local_compact_with_agent(
            session,
            original_options,
            original_execution,
            tail_start,
            compacted_at,
            state,
            control,
        ))
        .await
    }

    async fn try_remote_compact(
        &self,
        session: &Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Option<String>, AppError> {
        let active_messages = prompt_window::active_prompt_messages(session);
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        let tools = scoped_executor.available_tools_for_messages_and_loaded(
            active_messages.as_slice(),
            session.runtime.loaded_deferred_tools(),
        );
        let request = options.completion_request(
            options.system.clone(),
            active_messages,
            tools,
            Some(prompt_window::prompt_cache_key_for_session(session)),
            None,
            Some(session.runtime.prompt_window.generation),
        );
        state
            .processor
            .provider_registry()
            .compact_conversation(&options.model, request)
            .await
            .map(|summary| summary.filter(|value| !value.trim().is_empty()))
    }

    async fn local_compact_with_agent(
        &self,
        mut session: Session,
        mut options: SessionRunOptions,
        original_execution: SessionExecutionContext,
        tail_start: Option<i64>,
        compacted_at: Option<i64>,
        state: Arc<SessionManagerState>,
        control: Arc<RunControl>,
    ) -> Result<Session, AppError> {
        options.agent_profile = Some(COMPACTION_AGENT.to_string());
        options.max_run_loops = Some(2);
        session = self
            .apply_requested_agent_profile(session, &mut options, state.clone())
            .await?;
        options = self.apply_execution_context_to_run_options(&session, options)?;

        let ids = self.store.reserve_message_ids(1).await?;
        let request_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text(compaction_prompt())],
            MessageMetadata {
                source: MessageSource::System,
                parent_message_id: session
                    .last_conversation_message()
                    .map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: options.model.provider_id.to_string(),
                model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                model_id: options.model.model_id.to_string(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
            },
        );
        session.messages.push(request_message.clone());
        session = self
            .persist_session_changes(
                session,
                vec![request_message.clone()],
                Vec::new(),
                None,
                state.clone(),
            )
            .await?;

        let session_id = session.id;
        let run_result = Box::pin(self.run_model_turn(
            session,
            &options,
            RunSource::Compaction,
            state.clone(),
            control,
        ))
        .await;
        let mut session = match run_result {
            Ok(session) => session,
            Err(err) => {
                self.restore_execution_after_compaction_error(
                    session_id,
                    original_execution,
                    state,
                )
                .await?;
                return Err(err);
            }
        };

        let assistant_index = session
            .messages
            .iter()
            .rposition(|message| message.role == Role::Assistant && message.id > request_message.id)
            .ok_or_else(|| {
                AppError::Internal(
                    "local compaction did not produce an assistant summary".to_string(),
                )
            })?;
        let assistant_id = session.messages[assistant_index].id;
        let summary = session.messages[assistant_index].as_text_lossy();
        if summary.trim().is_empty() {
            session.runtime.execution = original_execution;
            let _ = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state)
                .await?;
            return Err(AppError::Provider(
                "local compaction returned an empty summary".to_string(),
            ));
        }
        let touched = vec![session.messages[assistant_index].clone()];
        self.install_compaction_runtime(
            session,
            original_execution,
            CompactionRuntimeInstall {
                summary,
                tail_start_message_id: tail_start,
                compacted_at_message_id: compacted_at,
                compacted_by_message_id: Some(assistant_id),
                strategy: PromptCompactionStrategy::LocalAgent,
                touched_messages: touched,
            },
            state,
        )
        .await
    }

    async fn restore_execution_after_compaction_error(
        &self,
        session_id: i64,
        original_execution: SessionExecutionContext,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        session.runtime.execution = original_execution;
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await
    }

    async fn install_compaction_runtime(
        &self,
        mut session: Session,
        original_execution: SessionExecutionContext,
        install: CompactionRuntimeInstall,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let CompactionRuntimeInstall {
            summary,
            tail_start_message_id,
            compacted_at_message_id,
            compacted_by_message_id,
            strategy,
            touched_messages,
        } = install;
        session.runtime.execution = original_execution;
        session.runtime.prompt_window.generation =
            session.runtime.prompt_window.generation.saturating_add(1);
        session.runtime.prompt_window.compaction = Some(PromptCompactionRuntime {
            summary: summary.trim().to_string(),
            tail_start_message_id,
            compacted_at_message_id,
            compacted_by_message_id,
            strategy,
            created_at_ms: Utc::now().timestamp_millis(),
        });
        session.runtime.clear_provider_anchors();
        session.runtime.clear_prompt_tokens();
        self.persist_session_changes(session, touched_messages, Vec::new(), None, state)
            .await
    }
}

fn select_tail_start_message_id(messages: &[Message]) -> Option<i64> {
    let mut user_messages = 0usize;
    for message in messages.iter().rev() {
        if message.role == Role::User && message.metadata.source == MessageSource::User {
            user_messages += 1;
            if user_messages == DEFAULT_TAIL_USER_MESSAGES {
                return Some(message.id);
            }
        }
    }
    messages.last().map(|message| message.id)
}

fn compaction_prompt() -> String {
    [
        "Summarize the conversation so far for future continuation.",
        "Preserve the current objective, explicit constraints, key decisions, important files, commands and results, tool state, pending work, blockers, and open questions.",
        "The newest messages will be kept verbatim after this summary, so keep the summary dense and avoid repeating short recent exchanges unless they are necessary context.",
        "Do not call tools. Do not mention compaction. Return only the summary.",
    ]
    .join("\n")
}
