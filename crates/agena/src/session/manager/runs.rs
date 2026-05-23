use super::*;

impl SessionManager {
    #[tracing::instrument(skip(self, request), fields(session_id = request.session_id))]
    pub async fn submit_user_message(
        &self,
        request: SessionUserMessageRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        let (control, steer_rx) = self.run_registry.register(session_id).await;
        crate::metrics::session_started();
        let manager = self.background_handle();
        let task_control = control.clone();
        let result = tokio::task::spawn(async move {
            manager
                .submit_user_message_inner(request, task_control, steer_rx)
                .await
        })
        .await
        .map_err(|err| AppError::Internal(format!("user run task failed: {err}")))
        .and_then(std::convert::identity);
        crate::metrics::session_finished();
        self.run_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    async fn submit_user_message_inner(
        &self,
        mut request: SessionUserMessageRequest,
        control: Arc<RunControl>,
        steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();

        // Plugin chain: user.prompt.submit. Plugins can rewrite or block the
        // user's prompt before it enters the session.
        let prompt_text = request
            .parts
            .iter()
            .filter_map(|p| p.text_value())
            .collect::<Vec<_>>()
            .join("\n");
        if !prompt_text.is_empty() {
            let input = crate::plugin::UserPromptSubmitInput {
                session_id: request.session_id,
                prompt: prompt_text,
            };
            match state
                .tool_executor
                .plugin_manager()
                .dispatch_user_prompt_submit(input)
                .await
            {
                Ok(updated) => {
                    // Replace text parts with the (potentially rewritten) prompt.
                    let mut replaced = false;
                    for part in &mut request.parts {
                        if part.text_value().is_some() {
                            *part = PartContent::text(updated.prompt.clone());
                            replaced = true;
                            break;
                        }
                    }
                    if !replaced {
                        request.parts.push(PartContent::text(updated.prompt));
                    }
                }
                Err(err) => {
                    return Err(AppError::Internal(format!(
                        "prompt blocked by plugin: {}",
                        err.message
                    )));
                }
            }
        }

        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.options, state.clone())
            .await?;
        let options = self.apply_execution_context_to_run_options(&session, request.options)?;
        self.apply_run_selection_to_session(&mut session, &options);
        let ids = self.store.reserve_message_ids(request.parts.len()).await?;
        let user_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            request.parts,
            MessageMetadata {
                source: MessageSource::User,
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
        session.messages.push(user_message.clone());
        session = self
            .persist_session_changes(
                session,
                vec![user_message.clone()],
                Vec::new(),
                None,
                state.clone(),
            )
            .await?;

        // Append-only history: persist the user-authored message as its own
        // closed run batch so it remains addressable in fork/rewind flows.
        let user_run_id = HistoryRunId::new();
        let user_history_items = vec![
            EventKind::RunStarted(RunStarted {
                run_id: user_run_id,
                source: RunSource::User,
                model_id: options.model.model_id.as_str().into(),
                provider_id: options.model.provider_id.as_str().into(),
                request_digest: None,
            }),
            EventKind::UserMessageAppended(UserMessageAppended {
                message_id: HistoryMessageId(user_message.id),
                run_id: user_run_id,
                created_at: user_message.created_at,
                content: TranscriptContent::from_message_lossy(&user_message),
                parts: user_message.parts.clone(),
                metadata: user_message.metadata.clone(),
                provider_state: user_message.provider_state.clone(),
            }),
            EventKind::RunCompleted(RunCompleted {
                run_id: user_run_id,
                finish_reason: FinishReason::Stop,
            }),
        ];
        session = self
            .store
            .append_history_items(session, user_history_items, state.cache_policy())
            .await?;

        self.run_until_stable(
            session,
            &options,
            false,
            RunSource::User,
            state,
            control,
            steer_rx,
        )
        .await
    }

    pub async fn continue_session(
        &self,
        mut request: SessionContinueRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        let (control, steer_rx) = self.run_registry.register(session_id).await;
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.options, state.clone())
            .await?;
        session = self
            .resume_paused_goal_if_needed(session, state.clone())
            .await?;
        let options = self.apply_execution_context_to_run_options(&session, request.options)?;
        if self.apply_run_selection_to_session(&mut session, &options) {
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;
        }
        let result = self
            .run_until_stable(
                session,
                &options,
                true,
                RunSource::Continue,
                state,
                control.clone(),
                steer_rx,
            )
            .await;
        self.run_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    pub async fn spawn_subtask(
        &self,
        request: SessionSubtaskRequest,
    ) -> Result<SessionSubtaskResponse, AppError> {
        let state = self.execution_state();
        let parent = self
            .store
            .load_session(request.parent_session_id, state.cache_policy())
            .await?;
        let requested_profile_name = request
            .profile_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| request.subagent_type.to_string());
        let resolved_profile = state
            .tool_executor
            .subagent_registry()
            .get(requested_profile_name.as_str());
        let effective_profile_name = resolved_profile
            .as_ref()
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| requested_profile_name.clone());
        if let Some(profile) = resolved_profile.as_ref()
            && !profile.frontmatter.mode.allows_subagent()
        {
            return Err(AppError::Config(format!(
                "agent profile '{}' is not available for subtask sessions",
                profile.name
            )));
        }
        let prompt = resolved_profile
            .as_ref()
            .map(|profile| {
                if request.prompt.trim().is_empty() {
                    profile.prompt.clone()
                } else {
                    format!(
                        "{}\n\nDelegated task:\n{}",
                        profile.prompt.trim(),
                        request.prompt.trim()
                    )
                }
            })
            .unwrap_or_else(|| request.subagent_type.apply_prompt_guidance(&request.prompt));
        let profile_allowed_tools = resolved_profile
            .as_ref()
            .map(|profile| profile.frontmatter.allowed_tools.clone())
            .unwrap_or_default();
        let profile_permission = resolved_profile
            .as_ref()
            .map(|profile| {
                profile
                    .frontmatter
                    .permission
                    .effective_with_defaults(&state.config.permission)
            })
            .unwrap_or_else(|| {
                crate::agent::AgentPermissionConfig::default()
                    .effective_with_defaults(&state.config.permission)
            });
        let profile_mode = resolved_profile
            .as_ref()
            .map(|profile| profile.frontmatter.mode);
        let profile_hidden = resolved_profile
            .as_ref()
            .map(|profile| profile.frontmatter.hidden)
            .unwrap_or(false);
        let profile_color = resolved_profile
            .as_ref()
            .and_then(|profile| profile.frontmatter.color.clone());
        let profile_run = resolved_profile
            .as_ref()
            .map(|profile| crate::agent::AgentRunConfig {
                temperature: profile.frontmatter.temperature,
                max_output_tokens: profile.frontmatter.max_output_tokens,
                steps: profile.frontmatter.steps,
            })
            .unwrap_or_default();
        let profile_default = resolved_profile
            .as_ref()
            .map(|profile| profile.frontmatter.default.clone());
        let requested_model = request.requested_model.clone();

        if let Some(existing) = self
            .find_child_session_for_task(request.parent_session_id, request.task_id.as_deref())
            .await?
        {
            let mut existing = existing;
            existing.runtime.execution.selection.agent = Some(effective_profile_name.clone());
            existing.runtime.execution.agent_mode = profile_mode;
            existing.runtime.execution.agent_hidden = profile_hidden;
            existing.runtime.execution.agent_color = profile_color.clone();
            existing.runtime.execution.system_prompt_override = Some(prompt.clone());
            existing
                .runtime
                .set_allowed_tools(profile_allowed_tools.clone());
            existing.runtime.execution.agent_permission = profile_permission.clone();
            existing.runtime.execution.agent_run = profile_run.clone();
            existing.runtime.execution.task_id = request.task_id.clone();
            existing = self
                .persist_session_changes(existing, Vec::new(), Vec::new(), None, state.clone())
                .await?;
            let options = self.subtask_run_options(
                &existing,
                &parent,
                &state,
                requested_model.as_deref(),
                profile_default.as_ref(),
            )?;
            let session = Box::pin(self.continue_session(SessionContinueRequest {
                session_id: existing.id,
                options: options.clone(),
            }))
            .await?;
            return Ok(SessionSubtaskResponse {
                profile_name: Some(effective_profile_name),
                model_provider_id: Some(options.model.provider_id.to_string()),
                model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                model_id: Some(options.model.model_id.to_string()),
                session,
            });
        }

        let mut child = self
            .store
            .create_subagent_session(
                request.description.clone(),
                request.parent_session_id,
                state.cache_policy(),
            )
            .await?;
        child.runtime.execution.selection.agent = Some(effective_profile_name.clone());
        child.runtime.execution.agent_mode = profile_mode;
        child.runtime.execution.agent_hidden = profile_hidden;
        child.runtime.execution.agent_color = profile_color;
        child.runtime.execution.system_prompt_override = Some(prompt.clone());
        child.runtime.set_allowed_tools(profile_allowed_tools);
        child.runtime.execution.agent_permission = profile_permission;
        child.runtime.execution.agent_run = profile_run;
        child.runtime.execution.task_id = request.task_id.clone();
        child = self
            .persist_session_changes(child, Vec::new(), Vec::new(), None, state.clone())
            .await?;

        let options = self.subtask_run_options(
            &child,
            &parent,
            &state,
            requested_model.as_deref(),
            profile_default.as_ref(),
        )?;
        let child_id = child.id;
        drop(child);
        drop(parent);
        drop(prompt);
        drop(profile_default);
        drop(requested_model);
        let manager = self.background_handle();
        let run_options = options.clone();
        let session = tokio::task::spawn(async move {
            manager
                .submit_user_message(SessionUserMessageRequest {
                    session_id: child_id,
                    options: run_options,
                    parts: vec![PartContent::text(request.prompt)],
                })
                .await
        })
        .await
        .map_err(|err| AppError::Internal(format!("subtask run task failed: {err}")))??;

        Ok(SessionSubtaskResponse {
            profile_name: Some(effective_profile_name),
            model_provider_id: Some(options.model.provider_id.to_string()),
            model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
            model_id: Some(options.model.model_id.to_string()),
            session,
        })
    }
}
