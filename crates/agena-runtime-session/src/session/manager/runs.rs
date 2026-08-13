use super::{
    AppError, Arc, ExecutionControl, ExecutionConversationTarget, ExecutionSource,
    SessionExecutionRequest, SessionManager, SessionSubtaskRequest, SessionSubtaskResponse,
    SessionUserRunRequest, StableRunContext, mpsc,
};
use crate::session::Session;
use crate::session::store::{
    new_part_from_content, skill_ref_from_reference, text_content, typed_content_from_value,
    typed_text,
};
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};
use agena_runtime_contracts::part::{SkillReference, SkillReferencePart};
use agena_runtime_contracts::part_content::{
    TypedContent, operation_from_tool_call, skill_reference_from_skill_ref,
};
use agena_storage::store::{Part, PartRole, PartState};
use std::path::Path;

/// The lossy visible text of one run group, mirroring the v1
/// `Message::visible_text_lossy` over the run's decoded content parts: text
/// and skill-reference parts render their content, operation parts their
/// best-effort output, and the remaining part kinds fall back to their
/// summary.
pub(crate) fn run_visible_text_lossy(run: &[Part]) -> String {
    run.iter()
        .skip(1)
        .filter_map(
            |part| match typed_content_from_value(&part.kind, &part.content) {
                Ok(TypedContent::Text(text)) => Some(text.text.clone()),
                Ok(TypedContent::SkillRef(skill)) => {
                    Some(skill_reference_from_skill_ref(&skill).summary())
                }
                Ok(TypedContent::ToolCall(tool)) => {
                    tool_visible_text_lossy(&operation_from_tool_call(&tool))
                }
                Ok(TypedContent::Think(_)) => None,
                _ => part.summary.clone(),
            },
        )
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Best-effort textual rendering of an operation part, mirroring the v1
/// `Message::tool_text_lossy` (private in contracts): first non-empty of
/// output text, error message, title, or summary.
fn tool_visible_text_lossy(tool: &agena_runtime_contracts::part::OperationPart) -> Option<String> {
    let candidates = [
        tool.output_text(),
        tool.error_message(),
        tool.title(),
        (!tool.summary.trim().is_empty()).then_some(tool.summary.as_str()),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}

/// Resolve requested Skill names/aliases into immutable Skill references
/// for a delegated subtask. The catalog is rebuilt on demand from bundled
/// and filesystem-discovered Skills, so a later catalog change never
/// silently alters the snapshot attached to a child session.
fn resolve_subtask_skill_references(
    workspace_root: &Path,
    requested: &[String],
) -> Result<Vec<SkillReference>, AppError> {
    let mut catalog = std::collections::BTreeMap::new();
    for skill in agena_skills::bundled::all() {
        catalog.insert(skill.frontmatter.name.clone(), skill);
    }
    let roots = agena_skills::discovery::default_roots(Some(workspace_root));
    let discovered = agena_skills::discovery::scan(&roots).map_err(|error| {
        AppError::Internal(format!("failed to scan skills for subtask: {error}"))
    })?;
    for skill in discovered {
        catalog.insert(skill.frontmatter.name.clone(), skill);
    }

    requested
            .iter()
            .map(|name| {
                let trimmed = name.trim();
                let skill = catalog.values().find(|skill| skill.matches(trimmed)).ok_or_else(
                    || {
                        AppError::Config(format!(
                            "unknown skill '{name}' for subtask; use `agena.skills.list` to see available skills"
                        ))
                    },
                )?;
                Ok(SkillReference {
                    name: skill.frontmatter.name.clone(),
                    description: skill.frontmatter.description.clone(),
                    instructions: skill.body.clone(),
                    content_hash: skill.content_hash(),
                    source: skill
                        .source_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "bundled".to_string()),
                    aliases: skill.frontmatter.aliases.clone(),
                })
            })
                        .collect()
}

impl SessionManager {
    async fn require_subtask_session(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Session, AppError> {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return Err(AppError::Config(
                "subtask task_id must not be empty".to_string(),
            ));
        }
        let child_id = self
            .store
            .find_subagent_by_task_id(parent_session_id, task_id)
            .await?
            .ok_or_else(|| {
                AppError::Config(format!(
                    "subtask '{task_id}' does not exist under session {parent_session_id}"
                ))
            })?;
        self.store.load_session(child_id).await
    }

    pub async fn cancel_subtask(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<i64, AppError> {
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        self.cancel_active_execution(child.id).await?;
        Ok(child.id)
    }

    pub async fn message_subtask(
        &self,
        parent_session_id: i64,
        task_id: &str,
        message: String,
    ) -> Result<i64, AppError> {
        if message.trim().is_empty() {
            return Err(AppError::Config(
                "subtask message must not be empty".to_string(),
            ));
        }
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        self.steer_input(child.id, vec![TypedContent::Text(text_content(message))])
            .await?;
        Ok(child.id)
    }

    pub async fn read_subtask_output(
        &self,
        parent_session_id: i64,
        task_id: &str,
        after_cursor: i64,
        limit: u32,
    ) -> Result<crate::SessionSubtaskOutput, AppError> {
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        let limit = limit.clamp(1, 500) as usize;
        let mut messages = crate::session::store::parts_into_runs(child.parts())
            .into_iter()
            .filter_map(|run| {
                let marker_id = run.first().map(|marker| marker.part_id);
                marker_id
                    .filter(|marker_id| *marker_id > after_cursor)
                    .map(|_| run)
            })
            .filter_map(|run| {
                let marker = run.first().expect("run group has a marker");
                let text = run_visible_text_lossy(&run);
                (!text.trim().is_empty()).then_some(crate::SessionSubtaskOutputChunk {
                    cursor: marker.part_id,
                    role: crate::session::store::role_from_part_role(marker.role),
                    text,
                    created_at_ms: marker.created_at_ms,
                })
            })
            .collect::<Vec<_>>();
        messages.sort_by_key(|message| message.cursor);
        let has_more = messages.len() > limit;
        messages.truncate(limit);
        let next_cursor = messages
            .last()
            .map_or(after_cursor, |message| message.cursor);
        Ok(crate::SessionSubtaskOutput {
            session_id: child.id,
            chunks: messages,
            next_cursor,
            has_more,
        })
    }

    pub(in crate::session::manager) async fn submit_user_run_inner(
        &self,
        mut request: SessionUserRunRequest,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::Receiver<Vec<TypedContent>>,
        usage_budget: Option<super::SubtaskUsageBudget>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();

        // Plugin chain: user.prompt.submit. Plugins can rewrite or block the
        // user's prompt before it enters the session.
        let prompt_text = request
            .parts
            .iter()
            .filter_map(|p| typed_text(p))
            .collect::<Vec<_>>()
            .join("\n");
        if !prompt_text.is_empty() {
            let input = agena_plugin_host::UserPromptSubmitInput {
                session_id: request.run.session_id,
                prompt: prompt_text,
            };
            match state
                .tool_executor
                .plugin_manager()
                .dispatch_user_prompt_submit_cancellable(input, Some(control.cancel.clone()))
                .await
            {
                Ok(updated) => {
                    // Replace text parts with the (potentially rewritten) prompt.
                    let mut replaced = false;
                    for part in &mut request.parts {
                        if typed_text(part).is_some() {
                            *part = TypedContent::Text(text_content(updated.prompt.clone()));
                            replaced = true;
                            break;
                        }
                    }
                    if !replaced {
                        request.parts.push(TypedContent::Text(text_content(updated.prompt)));
                    }
                }
                Err(err) => {
                    if control.cancel.is_cancelled() {
                        return Err(AppError::Cancelled);
                    }
                    return Err(AppError::Internal(format!(
                        "prompt blocked by plugin: {}",
                        err.diagnostic_message()
                    )));
                }
            }
        }

        let mut session = self.store.load_session(request.run.session_id).await?;
        self.refresh_execution_policy(&mut session, &state);
        let options = self
            .apply_execution_context_to_run_options_async(&session, request.run.options)
            .await?;
        self.apply_run_selection_to_session(&mut session, &options);
        // Make the run's model selection durable so a reload (post-send
        // refresh, next turn, new session first submit) resolves the same
        // model instead of the default. `persist_session_changes` is a no-op
        // without changed parts, so write the execution config directly.
        session = self.store.persist_execution_config(session).await?;
        let input_parts = request.parts;
        // The user's message is persisted as a `user_send` run: one run
        // marker plus one `text` content part per submitted payload (the same
        // shape `drain_steer_input` writes). v2 parts carry no activity
        // identity, so the v1 `bind_activity` step is dropped here.
        let user_parts = input_parts
            .iter()
            .map(|part| {
                new_part_from_content("text", PartRole::User, part, PartState::Completed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outcome = self
            .store
            .submit_user_run(session.id, user_parts, request.idempotency_key.clone())
            .await?;
        let mut projected = session.parts().to_vec();
        projected.extend(outcome.parts);
        session.install_projected_parts(projected);

        // Record the user.prompt.submit hook runs observed during this
        // submission (plus any unattributed runs claimed by this session)
        // before the model turns begin.
        let hook_runs = state
            .tool_executor
            .plugin_manager()
            .drain_hook_runs(session.id);
        if !hook_runs.is_empty() {
            session = self
                .record_hook_runs(session, hook_runs, state.clone())
                .await?;
        }

        let session_id = session.id;
        let outcome = self
            .run_until_stable(
                session,
                &options,
                StableRunContext {
                    base_run_source: ExecutionSource::User,
                    active_model_turn_id: Some(outcome.run_id),
                    state,
                    control,
                    steer_rx,
                    usage_budget,
                },
            )
            .await;
        match outcome {
            Ok(mut session) => {
                // Drain any hook runs the stable run left behind (for example
                // a stop hook that fired after the final reply) and record them
                // into the returned session so they are not left in the shared
                // queue to be misattributed to the next submission.
                let state = self.execution_state();
                let hook_runs = state
                    .tool_executor
                    .plugin_manager()
                    .drain_hook_runs(session.id);
                if !hook_runs.is_empty() {
                    session = self.record_hook_runs(session, hook_runs, state).await?;
                }
                Ok(session)
            }
            Err(err) => {
                // The run failed; drain anything still queued and record it so
                // the failure records stay attributed to this run instead of
                // leaking into the next one. `record_hook_runs` consumes the
                // session, so reload it from the store first; recording
                // failures are swallowed to keep the original error.
                let state = self.execution_state();
                let hook_runs = state
                    .tool_executor
                    .plugin_manager()
                    .drain_hook_runs(session_id);
                if !hook_runs.is_empty() {
                    match self.store.load_session(session_id).await {
                        Ok(reloaded) => {
                            if let Err(record_err) = self
                                .record_hook_runs(reloaded, hook_runs, state.clone())
                                .await
                            {
                                tracing::warn!(
                                    target: "agena::session::hook_runs",
                                    session_id,
                                    "failed to record hook runs after failed run: {record_err}"
                                );
                            }
                        }
                        Err(load_err) => {
                            tracing::warn!(
                                target: "agena::session::hook_runs",
                                session_id,
                                "failed to reload session to record hook runs: {load_err}"
                            );
                        }
                    }
                }
                Err(err)
            }
        }
    }

    /// Same execution lifecycle as an ordinary user message, with an
    /// optional child-relative budget checked before every model turn. This
    /// stays private to delegated tasks so normal interactive sessions do not
    /// inherit a task's accounting boundary.
    pub(in crate::session::manager) async fn submit_subtask_user_message(
        &self,
        request: SessionUserRunRequest,
        usage_budget: Option<super::SubtaskUsageBudget>,
    ) -> Result<Session, AppError> {
        let session_id = request.run.session_id;
        self.execute_registered(
            session_id,
            ExecutionSource::User,
            ExecutionConversationTarget::NewTurn,
            "subtask execution",
            move |manager, control, steer_rx| async move {
                manager
                    .submit_user_run_inner(request, control, steer_rx, usage_budget)
                    .await
            },
        )
        .await
    }

    pub async fn continue_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        self.execute_registered(
            session_id,
            ExecutionSource::Continue,
            ExecutionConversationTarget::LatestReply,
            "continuation execution",
            move |manager, control, steer_rx| async move {
                manager
                    .continue_session_inner(request, control, steer_rx)
                    .await
            },
        )
        .await
    }

    pub async fn start_continue_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        let session_id = request.session_id;
        self.start_registered(
            session_id,
            ExecutionSource::Continue,
            ExecutionConversationTarget::LatestReply,
            "continuation execution",
            move |manager, control, steer_rx| async move {
                manager
                    .continue_session_inner(request, control, steer_rx)
                    .await
            },
        )
        .await
    }

    async fn continue_session_inner(
        &self,
        request: SessionExecutionRequest,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::Receiver<Vec<TypedContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self.store.load_session(request.session_id).await?;
        self.refresh_execution_policy(&mut session, &state);
        let options = self
            .apply_execution_context_to_run_options_async(&session, request.options)
            .await?;
        if self.apply_run_selection_to_session(&mut session, &options) {
            // Persist the selection durably (`persist_session_changes` with
            // no changed parts is a no-op) so a later reload keeps it.
            session = self.store.persist_execution_config(session).await?;
        }
        self.run_until_stable(
            session,
            &options,
            StableRunContext {
                base_run_source: ExecutionSource::Continue,
                active_model_turn_id: None,
                state,
                control,
                steer_rx,
                usage_budget: None,
            },
        )
        .await
    }

    pub async fn run_subtask(
        &self,
        request: SessionSubtaskRequest,
    ) -> Result<SessionSubtaskResponse, AppError> {
        let task_started = tokio::time::Instant::now();
        let task_timeout = request.timeout_ms.map(std::time::Duration::from_millis);
        let state = self.execution_state();
        let description = request.description.trim();
        if description.is_empty() {
            return Err(AppError::Config(
                "subtask description must not be empty".to_string(),
            ));
        }
        let delegated_prompt = request.prompt.trim();
        if delegated_prompt.is_empty() {
            return Err(AppError::Config(
                "subtask prompt must not be empty".to_string(),
            ));
        }
        let subtask_skill_references = match request
            .skills
            .as_deref()
            .filter(|skills| !skills.is_empty())
        {
            Some(skills) => Some(resolve_subtask_skill_references(
                state.tool_executor.workspace_root(),
                skills,
            )?),
            None => None,
        };
        if request.timeout_ms == Some(0) {
            return Err(AppError::Config(
                "subtask timeout_ms must be greater than zero".to_string(),
            ));
        }
        if request.max_tokens == Some(0) {
            return Err(AppError::Config(
                "subtask max_tokens must be greater than zero".to_string(),
            ));
        }
        if request.max_cost_microusd == Some(0) {
            return Err(AppError::Config(
                "subtask max_cost_microusd must be greater than zero".to_string(),
            ));
        }
        let parent = self.store.load_session(request.parent_session_id).await?;
        if parent.is_subagent() {
            return Err(AppError::Config(
                "delegated subtasks cannot create nested subtasks".to_string(),
            ));
        }
        let options =
            self.subtask_run_options(&parent, &state, &request.requested_model_selection)?;
        let task_id = match request.task_id.as_deref().map(str::trim) {
            Some("") => {
                return Err(AppError::Config(
                    "subtask task_id must not be empty when supplied".to_string(),
                ));
            }
            Some(value) if value.len() > 128 => {
                return Err(AppError::Config(
                    "subtask task_id must not exceed 128 bytes".to_string(),
                ));
            }
            Some(value) => value.to_string(),
            None => format!("task_{}", uuid::Uuid::new_v4().simple()),
        };

        // Serialize only durable subtask preparation for a direct parent. The
        // bounded coordinator rejects nested acquisition and times out queued
        // writers, so this path cannot wait forever or form a lock cycle.
        let (resumed, child_id, baseline_message_id, baseline_usage, usage_budget) = self
            .session_mutations
            .run(parent.id, async {
                let existing = self
                    .store
                    .find_subagent_by_task_id(parent.id, task_id.as_str())
                    .await?;
                let resumed = existing.is_some();
                let mut child = match existing {
                    Some(existing_id) => {
                        if self.execution_registry.is_active(existing_id).await {
                            return Err(AppError::ExecutionAlreadyActive(existing_id));
                        }
                        self.store.load_session(existing_id).await?
                    }
                    None => {
                        let child_id = self
                            .store
                            .create_subagent_session(
                                parent.id,
                                task_id.clone(),
                                description.to_string(),
                            )
                            .await?;
                        self.store.load_session(child_id).await?
                    }
                };

                child.runtime.execution.access =
                    if parent.runtime.execution.access == agena_domain::ExecutionAccess::ReadOnly {
                        agena_domain::ExecutionAccess::ReadOnly
                    } else {
                        request.access
                    };
                let child_permission = self.resolve_effective_session_permission(&child, &state);
                let parent_permission = if parent.runtime.execution.effective_permission.is_empty()
                {
                    self.resolve_effective_session_permission(&parent, &state)
                } else {
                    parent.runtime.execution.effective_permission.clone()
                };
                child.runtime.execution.effective_permission = child_permission;
                child.runtime.execution.permission_ceiling = parent_permission;
                child.runtime.execution.capability_denied_tool_names =
                    non_recursive_subtask_capability_denials();
                child.runtime.execution.effective_workspace_root = Some(
                    parent
                        .runtime
                        .execution
                        .effective_workspace_root
                        .clone()
                        .unwrap_or_else(|| state.tool_executor.workspace_root().to_path_buf()),
                );
                let started_at_ms = chrono::Utc::now().timestamp_millis();
                let baseline_message_id = child
                    .parts()
                    .iter()
                    .filter(|part| part.is_run_marker())
                    .map(|part| part.part_id)
                    .max();
                let baseline_usage = child.aggregate_usage();
                let usage_budget = super::SubtaskUsageBudget::new(
                    baseline_usage.clone(),
                    request.max_tokens,
                    request.max_cost_microusd,
                );
                child.runtime.subtask.status = agena_domain::SubtaskStatus::Running;
                child.runtime.subtask.started_at_ms = Some(started_at_ms);
                child.runtime.subtask.finished_at_ms = None;
                child.runtime.subtask.failure = None;
                self.apply_run_selection_to_session(&mut child, &options);
                child = self
                    .store
                    .update_subtask_state(
                        child,
                        Some(agena_domain::SubtaskStatus::Running.as_ref().to_string()),
                        Some(started_at_ms),
                        None,
                        None,
                    )
                    .await?;
                child = self.store.persist_execution_config(child).await?;
                Ok((
                    resumed,
                    child.id,
                    baseline_message_id,
                    baseline_usage,
                    usage_budget,
                ))
            })
            .await?;

        let manager = self.background_handle();
        let run_options = options.clone();
        let prompt = delegated_prompt.to_string();
        let skill_references = subtask_skill_references;
        let mut run = Box::pin(async move {
            let mut parts = vec![TypedContent::Text(text_content(prompt))];
            if let Some(skill_references) = skill_references {
                parts.push(TypedContent::SkillRef(skill_ref_from_reference(
                    &SkillReferencePart {
                        skills: skill_references,
                    },
                )));
            }
            manager
                .submit_subtask_user_message(
                    SessionUserRunRequest::new(child_id, run_options, parts),
                    usage_budget,
                )
                .await
        });

        let mut timed_out = false;
        let run_result = if let Some(timeout) = task_timeout {
            let remaining = timeout.saturating_sub(task_started.elapsed());
            match tokio::time::timeout(remaining, &mut run).await {
                Ok(result) => result,
                Err(_) => {
                    timed_out = true;
                    // `execute_registered` has no suspension point between
                    // registry insertion and lifecycle-owner spawn. If there
                    // is no active execution now, this future timed out before
                    // registration and is safe to drop. Otherwise signal the
                    // supervised owner and give it a bounded cleanup window.
                    if self.execution_registry.is_active(child_id).await {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            self.cancel_active_execution(child_id),
                        )
                        .await;
                        match tokio::time::timeout(std::time::Duration::from_secs(5), &mut run)
                            .await
                        {
                            Ok(result) => result,
                            Err(_) => Err(AppError::Cancelled),
                        }
                    } else {
                        Err(AppError::Cancelled)
                    }
                }
            }
        } else {
            run.await
        };

        let (status, failure, budget_exceeded, mut session) = match run_result {
            Ok(session) if timed_out => (
                agena_domain::SubtaskStatus::TimedOut,
                Some(Failure::new(
                    FailureCode::new("subtask.timeout"),
                    FailureCategory::Timeout,
                    FailureResponsibility::System,
                    RetryDirective::UseAlternative,
                    RecoveryDirective::ChooseAlternative,
                    FailureImpact::OperationFailed,
                    UserPresentation::new("subtask-timeout", "The subtask timed out."),
                )),
                false,
                session,
            ),
            Ok(session) => (agena_domain::SubtaskStatus::Completed, None, false, session),
            Err(error) => {
                let status = if timed_out {
                    agena_domain::SubtaskStatus::TimedOut
                } else if matches!(&error, AppError::Cancelled) {
                    agena_domain::SubtaskStatus::Cancelled
                } else {
                    agena_domain::SubtaskStatus::Failed
                };
                let budget_exceeded = matches!(&error, AppError::SubtaskBudgetExceeded(_));
                let failure = (!matches!(&error, AppError::Cancelled)).then(|| error.failure());
                let session = self.store.load_session(child_id).await?;
                (status, failure, budget_exceeded, session)
            }
        };
        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        session.runtime.subtask.status = status;
        session.runtime.subtask.finished_at_ms = Some(finished_at_ms);
        session.runtime.subtask.failure = failure.clone();
        let subtask_started_at_ms = session.runtime.subtask.started_at_ms;
        session = self
            .store
            .update_subtask_state(
                session,
                Some(status.as_ref().to_string()),
                subtask_started_at_ms,
                Some(finished_at_ms),
                failure
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|error| {
                        AppError::Internal(format!("serialize subtask failure: {error}"))
                    })?,
            )
            .await?;
        let usage = session.aggregate_usage().saturating_sub(&baseline_usage);

        Ok(SessionSubtaskResponse {
            task_id,
            parent_session_id: parent.id,
            status,
            resumed,
            // Assistant text produced after the subtask's baseline: the
            // aggregate holds only parts created since the baseline marker, so
            // the last assistant text is the child's freshest reply.
            final_text: session
                .parts()
                .iter()
                .rev()
                .filter(|part| part.part_id > baseline_message_id.unwrap_or(0))
                .find_map(|part| {
                    part.content
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                        .or_else(|| part.summary.clone())
                })
                .filter(|text| !text.trim().is_empty()),
            failure,
            usage,
            model_provider_id: Some(options.model.provider_id.to_string()),
            model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
            model_id: Some(options.model.model_id.to_string()),
            budget_exceeded,
            session,
        })
    }
}

pub(in crate::session::manager) fn non_recursive_subtask_capability_denials()
-> std::collections::BTreeSet<String> {
    [
        "task",
        "tasks.run",
        "agena.tasks.run",
        "agena_tasks_run",
        "agena.tasks.create",
        "agena.tasks.followup",
        "agena.tasks.message",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{non_recursive_subtask_capability_denials, resolve_subtask_skill_references};
    use agena_runtime_contracts::part::SkillReference;

    #[test]
    fn delegated_instances_cannot_recursively_run_tasks() {
        let names = non_recursive_subtask_capability_denials();
        for name in [
            "task",
            "tasks.run",
            "agena.tasks.run",
            "agena_tasks_run",
            "agena.tasks.create",
            "agena.tasks.followup",
            "agena.tasks.message",
        ] {
            assert!(names.contains(name));
        }
    }

    #[test]
    fn subtask_skills_resolve_bundled_names_and_aliases() {
        let root = tempfile::tempdir().expect("temp dir");
        let requested = vec!["verify".to_string(), "security-review".to_string()];
        let refs = resolve_subtask_skill_references(root.path(), &requested).expect("resolve");
        assert_eq!(refs.len(), 2);
        let names = refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["verify", "security_review"]);
        for reference in &refs {
            assert!(!reference.instructions.is_empty());
            assert!(!reference.content_hash.is_empty());
        }
    }

    #[test]
    fn subtask_skills_discover_workspace_skills() {
        let root = tempfile::tempdir().expect("temp dir");
        let skill_dir = root.path().join(".agena").join("skills").join("explore");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: explore\ndescription: Explore a codebase\n---\nInvestigate the workspace.\n",
        )
        .expect("write skill");

        let requested = vec!["explore".to_string()];
        let refs = resolve_subtask_skill_references(root.path(), &requested).expect("resolve");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "explore");
        assert!(refs[0].instructions.contains("Investigate the workspace"));
        assert_eq!(
            refs[0].source,
            skill_dir.join("SKILL.md").display().to_string()
        );
    }

    #[test]
    fn subtask_skills_reject_unknown_names() {
        let root = tempfile::tempdir().expect("temp dir");
        let requested = vec!["no-such-skill".to_string()];
        let error = resolve_subtask_skill_references(root.path(), &requested).expect_err("reject");
        assert!(error.to_string().contains("unknown skill 'no-such-skill'"));
    }

    #[test]
    fn skill_reference_snapshot_carries_stable_identity() {
        let root = tempfile::tempdir().expect("temp dir");
        let requested = vec!["verify".to_string()];
        let refs = resolve_subtask_skill_references(root.path(), &requested).expect("resolve");
        let first = &refs[0];
        let expected: SkillReference = serde_json::from_value(serde_json::json!({
            "name": first.name,
            "description": first.description,
            "instructions": first.instructions,
            "content_hash": first.content_hash,
            "source": first.source,
            "aliases": first.aliases,
        }))
        .expect("serializable snapshot");
        assert_eq!(&expected, first);
    }
}
