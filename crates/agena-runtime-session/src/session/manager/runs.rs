use super::{
    AppError, Arc, ExecutionControl, ExecutionConversationTarget, ExecutionSource,
    SessionExecutionRequest, SessionManager, SessionSubtaskRequest, SessionSubtaskResponse,
    SessionUserRunRequest, StableRunContext, mpsc,
};
use crate::session::Session;
use crate::session::store::{
    command_ref_from_reference, new_part_from_content, text_content, typed_content_from_value,
    typed_text,
};
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};
use agena_runtime_contracts::part::{CommandReference, CommandReferencePart};
use agena_runtime_contracts::part_content::{
    TypedContent, command_reference_from_command_ref, operation_from_tool_call,
};
use agena_storage::store::{Part, PartRole, PartState};
use sha2::{Digest, Sha256};

static SUBTASK_LOG_PROJECTIONS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

/// The lossy visible text of one run group, derived from its decoded content
/// parts: text and command-reference parts render their content, tool-call parts
/// their best-effort output, and the remaining part kinds fall back to their
/// summary.
fn visible_run_text<'a>(parts: impl Iterator<Item = &'a Part>) -> String {
    parts
        .filter(|part| !part.is_run_marker())
        .filter_map(
            |part| match typed_content_from_value(&part.kind, &part.content) {
                Ok(TypedContent::Text(text)) => Some(text.text),
                Ok(TypedContent::CommandRef(command)) => {
                    Some(command_reference_from_command_ref(&command).summary())
                }
                Ok(TypedContent::ToolCall(tool)) => {
                    // A foreground command inside a delegated task is still
                    // running. Project its independent display tail so the
                    // task log's existing run cursor can update in place.
                    (part.state == PartState::InProgress)
                        .then(|| {
                            tool.live_output()
                                .filter(|text| !text.trim().is_empty())
                                .map(str::to_owned)
                        })
                        .flatten()
                        .or_else(|| tool_visible_text_lossy(&operation_from_tool_call(&tool)))
                }
                Ok(TypedContent::Think(_)) => None,
                _ => part.summary.clone(),
            },
        )
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
pub(crate) fn run_visible_text_lossy(run: &[Part]) -> String {
    visible_run_text(run.iter())
}

fn bounded_run_text(parts: &[&Part], max_bytes: usize) -> String {
    let mut slices = Vec::new();
    let mut remaining = max_bytes;
    for part in parts.iter().rev() {
        if part.is_run_marker() || part.kind == "think" || remaining == 0 {
            continue;
        }
        let body = match part.kind.as_str() {
            "text" | "paste_ref" => part
                .content
                .as_str()
                .or_else(|| part.content.get("text").and_then(serde_json::Value::as_str)),
            "tool_call" => {
                let live = (part.state == PartState::InProgress)
                    .then(|| {
                        part.content
                            .pointer("/metadata/live_output")
                            .and_then(serde_json::Value::as_str)
                    })
                    .flatten();
                live.filter(|text| !text.trim().is_empty())
                    .or_else(|| {
                        part.content
                            .pointer("/output/payload/text")
                            .and_then(serde_json::Value::as_str)
                    })
                    .or_else(|| {
                        part.content
                            .pointer("/output/payload")
                            .and_then(serde_json::Value::as_str)
                    })
                    .filter(|text| !text.trim().is_empty())
                    .or_else(|| {
                        part.content
                            .pointer("/error/failure/user/fallback")
                            .and_then(serde_json::Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                    })
                    .or_else(|| {
                        part.content
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                    })
                    .or(part.summary.as_deref())
            }
            _ => part.summary.as_deref(),
        };
        let command;
        let body = if part.kind == "skill_ref" {
            command = part
                .content
                .get("skills")
                .and_then(serde_json::Value::as_array)
                .map(|commands| {
                    let names: Vec<_> = commands
                        .iter()
                        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
                        .collect();
                    match names.as_slice() {
                        [] => "0 command references".to_owned(),
                        [name] => format!("Command: {name}"),
                        names => format!(
                            "{} commands: {}",
                            names.len(),
                            names.iter().take(3).copied().collect::<Vec<_>>().join(", ")
                        ),
                    }
                })
                .unwrap_or_else(|| "0 command references".to_owned());
            Some(command.as_str())
        } else {
            body
        };
        let Some(body) = body.filter(|body| !body.trim().is_empty()) else {
            continue;
        };
        let separator = usize::from(!slices.is_empty());
        if remaining <= separator {
            continue;
        }
        let mut start = body.len().saturating_sub(remaining - separator);
        while !body.is_char_boundary(start) {
            start += 1;
        }
        let tail = &body[start..];
        if tail.is_empty() {
            continue;
        }
        remaining -= tail.len() + separator;
        slices.push(tail.to_owned());
        if remaining == 0 {
            break;
        }
    }
    slices.reverse();
    slices.join("\n")
}

/// Best-effort textual rendering of a tool-call projection: first non-empty of
/// output text, error message, title, or summary.
fn tool_visible_text_lossy(tool: &agena_runtime_contracts::part::OperationPart) -> Option<String> {
    let candidates = [tool.output_text(), tool.error_message(), tool.title()];
    candidates
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod bounded_log_tests {
    use super::*;
    use agena_storage::store::PartVisibility;
    use serde_json::{Value, json};

    fn part(kind: &str, content: Value) -> Part {
        Part {
            part_id: 2,
            kind: kind.into(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: Some(1),
            origin_session_id: 1,
            revision: 1,
            started_at_ms: 1,
            finished_at_ms: Some(1),
            created_at_ms: 1,
            updated_at_ms: 1,
            provider_state: None,
        }
    }

    #[test]
    fn utf8_tails_respect_budgets_without_empty_slices_or_dangling_separators() {
        let older = part("text", json!({"text": "你"}));
        let latest = part("text", json!({"text": "🙂你"}));
        for (budget, expected) in [
            (0, ""),
            (2, ""),
            (3, "你"),
            (4, "你"),
            (7, "🙂你"),
            (8, "🙂你"),
            (11, "你\n🙂你"),
        ] {
            let text = bounded_run_text(&[&older, &latest], budget);
            assert_eq!(text, expected, "budget={budget}");
            assert!(text.len() <= budget);
        }
    }

    #[test]
    fn tool_logs_keep_live_output_and_terminal_error_and_title_fallbacks() {
        let mut tool = part(
            "tool_call",
            json!({"name": "shell", "output": {"payload": {"text": "completed"}}, "metadata": {"live_output": "still running"}, "error": {"failure": {"user": {"fallback": "failed"}}}}),
        );
        tool.state = PartState::InProgress;
        assert_eq!(bounded_run_text(&[&tool], 100), "still running");
        tool.state = PartState::Completed;
        assert_eq!(bounded_run_text(&[&tool], 100), "completed");
        tool.content["output"] = json!({"payload": "plain output"});
        assert_eq!(bounded_run_text(&[&tool], 100), "plain output");
        tool.content["output"] = Value::Null;
        assert_eq!(bounded_run_text(&[&tool], 100), "failed");
        tool.content["error"] = Value::Null;
        assert_eq!(bounded_run_text(&[&tool], 100), "shell");
    }

    #[test]
    fn logs_skip_reasoning_and_markers_and_preserve_command_reference_summaries() {
        let marker = part("run", json!({"run_kind": "continue"}));
        let mut reasoning = part("think", json!({"text": "private reasoning"}));
        reasoning.summary = Some("reasoning summary".into());
        let command = part("skill_ref", json!({"skills": [{"name": "review"}]}));
        let text = part("text", json!({"text": "done"}));
        assert_eq!(
            bounded_run_text(&[&marker, &reasoning, &command, &text], 100),
            "Command: review\ndone"
        );
    }
}

/// Resolve requested command names into lazy references for a delegated
/// subtask.
///
/// The catalog is the published command catalog — the same one every client
/// renders — so a subtask can only be pointed at a command the workspace
/// actually offers. The body is deliberately omitted; the delegated model
/// reads it on demand through the owning plugin.
fn resolve_subtask_command_references(
    catalog: &[agena_plugin_host::CommandCatalogItem],
    requested: &[String],
) -> Result<Vec<CommandReference>, AppError> {
    requested
        .iter()
        .map(|name| {
            let trimmed = name.trim();
            let entry = catalog
                .iter()
                .find(|entry| command_matches(entry, trimmed))
                .ok_or_else(|| {
                    AppError::Config(format!(
                        "unknown command '{name}' for subtask; the delegated session can list the available commands from the `agena.commands` plugin"
                    ))
                })?;
            Ok(CommandReference {
                name: entry.command.id.clone(),
                description: entry.command.docs.summary.clone().unwrap_or_default(),
                content_hash: command_declaration_hash(entry),
                source: entry.plugin_id.to_string(),
                aliases: entry.command.aliases.clone(),
            })
        })
        .collect()
}

/// Whether `query` names this command, by canonical id, slash spelling or
/// alias — the same three spellings a composer accepts.
fn command_matches(entry: &agena_plugin_host::CommandCatalogItem, query: &str) -> bool {
    let query = query.trim().trim_start_matches('/').to_ascii_lowercase();
    if query.is_empty() {
        return false;
    }
    let slash = entry
        .command
        .slash
        .as_deref()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_ascii_lowercase();
    entry.command.id.to_ascii_lowercase() == query
        || slash == query
        || entry
            .command
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase() == query)
}

/// Stable identity of one catalog declaration. Commands carry no content hash
/// of their own — their body lives behind a plugin — so the reference
/// identifies the declaration the user selected, which is what a stale-catalog
/// comparison needs.
fn command_declaration_hash(entry: &agena_plugin_host::CommandCatalogItem) -> String {
    let mut digest = Sha256::new();
    digest.update(entry.plugin_id.to_string().as_bytes());
    digest.update([0]);
    for field in [
        entry.command.id.as_str(),
        entry.command.title.as_str(),
        entry.command.slash.as_deref().unwrap_or_default(),
    ] {
        digest.update(field.as_bytes());
        digest.update([0]);
    }
    for alias in &entry.command.aliases {
        digest.update(alias.as_bytes());
        digest.update([0]);
    }
    for text in [
        entry.command.docs.summary.as_deref(),
        entry.command.docs.usage.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        digest.update(text.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
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
        self.load_session_with_workspace_root(child_id).await
    }

    pub async fn cancel_subtask(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<i64, AppError> {
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        self.cancel_active_execution_with_outcome(child.id).await?;
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
        self.read_subtask_output_page(parent_session_id, task_id, after_cursor, limit, None)
            .await
    }

    pub async fn read_subtask_output_bounded(
        &self,
        parent_session_id: i64,
        task_id: &str,
        after_cursor: i64,
        limit: u32,
    ) -> Result<crate::SessionSubtaskOutput, AppError> {
        self.read_subtask_output_page(
            parent_session_id,
            task_id,
            after_cursor,
            limit,
            Some(128 * 1024),
        )
        .await
    }

    async fn read_subtask_output_page(
        &self,
        parent_session_id: i64,
        task_id: &str,
        after_cursor: i64,
        limit: u32,
        byte_limit: Option<usize>,
    ) -> Result<crate::SessionSubtaskOutput, AppError> {
        let child_id = self
            .store
            .find_subagent_by_task_id(parent_session_id, task_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::Config(format!(
                    "subtask '{task_id}' does not exist under session {parent_session_id}"
                ))
            })?;
        let page = self
            .store
            .facade
            .load_runs_after(child_id, after_cursor, limit.clamp(1, 500) as usize)
            .await
            .map_err(crate::session::store::store_error)?;
        SUBTASK_LOG_PROJECTIONS
            .run(move || {
                let mut groups = std::collections::BTreeMap::<i64, Vec<&Part>>::new();
                for part in &page.parts {
                    let id = if part.is_run_marker() {
                        Some(part.part_id)
                    } else {
                        part.run_id
                    };
                    if let Some(id) = id {
                        groups.entry(id).or_default().push(part);
                    }
                }
                let mut chunks = Vec::new();
                let mut next_cursor = after_cursor;
                let mut bytes = 0;
                let mut has_more = page.has_more;
                for (_, run) in groups {
                    if byte_limit.is_some_and(|limit| bytes >= limit) {
                        has_more = true;
                        break;
                    }
                    let Some(marker) = run.iter().find(|part| part.is_run_marker()).copied() else {
                        continue;
                    };
                    let text = if let Some(limit) = byte_limit {
                        bounded_run_text(&run, (limit - bytes).min(64 * 1024))
                    } else {
                        // Only this cursor page is materialized; previous runs and
                        // their tool output are never cloned by the log reader.
                        visible_run_text(run.iter().copied())
                    };
                    next_cursor = marker.part_id;
                    if text.trim().is_empty() {
                        continue;
                    }
                    bytes += text.len();
                    chunks.push(crate::SessionSubtaskOutputChunk {
                        cursor: marker.part_id,
                        role: crate::session::store::role_from_part_role(marker.role),
                        text,
                        created_at_ms: marker.created_at_ms,
                    });
                }
                crate::SessionSubtaskOutput {
                    session_id: child_id,
                    chunks,
                    next_cursor,
                    has_more,
                }
            })
            .await
            .map_err(|error| AppError::Internal(format!("subtask log worker failed: {error}")))
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
                prompt: prompt_text.clone(),
            };
            match state
                .tool_executor
                .plugin_manager()
                .dispatch_user_prompt_submit_cancellable(input, Some(control.cancel.clone()))
                .await
            {
                Ok(updated) => {
                    // One user message is an ordered list of parts: body text plus
                    // attachment parts at their inline positions. The hook input joined
                    // every text part, so an unchanged prompt must leave that layout
                    // untouched - rewriting only the first text part would replay part
                    // of the body text twice to the model. A hook that really rewrote
                    // the prompt owns the message text: the rewritten prompt takes the
                    // first text part and the remaining text parts are dropped.
                    if updated.prompt != prompt_text {
                        let mut placed = false;
                        request.parts.retain_mut(|part| {
                            if typed_text(part).is_none() {
                                return true;
                            }
                            if placed {
                                return false;
                            }
                            placed = true;
                            *part = TypedContent::Text(text_content(updated.prompt.clone()));
                            true
                        });
                        if !placed {
                            request
                                .parts
                                .push(TypedContent::Text(text_content(updated.prompt.clone())));
                        }
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

        let mut session = self
            .load_session_with_workspace_root(request.run.session_id)
            .await?;
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
        let input_parts = self
            .materialize_user_media(&session, &options.model, request.parts)
            .await?;
        // The user's message is persisted as a `user_send` run: one run
        // marker plus one `text` content part per submitted payload (the same
        // shape `drain_steer_input` writes). Parts carry no separate activity
        // identity; presentation identities are derived when queried.
        let mut user_parts = input_parts
            .iter()
            .map(|part| new_part_from_content("text", PartRole::User, part, PartState::Completed))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(conversation) = &session.runtime.execution.conversation
            && !session
                .parts()
                .iter()
                .any(|part| part.origin_session_id == session.id && part.role == PartRole::User)
        {
            user_parts.insert(
                0,
                agena_storage::store::NewPart {
                    kind: "text".to_owned(),
                    role: PartRole::User,
                    content: serde_json::json!({ "text": conversation.boundary_notice() }),
                    summary: None,
                    visibility: agena_storage::store::PartVisibility::Ai,
                    parent_part_id: None,
                    state: PartState::Completed,
                },
            );
        }
        let submit_outcome = self
            .store
            .submit_user_run_for_execution(
                session.id,
                user_parts,
                request.idempotency_key.clone(),
                &control.execution_id().to_string(),
            )
            .await?;
        control.set_user_run(submit_outcome.run_id, submit_outcome.created);
        let mut projected = session.parts().to_vec();
        projected.extend(submit_outcome.parts);
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
                    active_model_turn_id: Some(submit_outcome.run_id),
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
        let mut session = self
            .load_session_with_workspace_root(request.session_id)
            .await?;
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

    /// The wake execution driven by `settle_background_operation` when the
    /// session is idle: the settle already appended the notification to its
    /// launch run (or committed a launch-less Runtime ingress), so this
    /// refreshes the session and takes a fresh model turn over that input.
    /// Modeled on `continue_session_inner` minus the Continue identity.
    pub(in crate::session::manager) async fn notification_run_inner(
        &self,
        session_id: i64,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::Receiver<Vec<TypedContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self.load_session_with_workspace_root(session_id).await?;
        self.refresh_execution_policy(&mut session, &state);
        let options = self
            .run_options_from_session_async(&session, state.clone())
            .await?;
        if self.apply_run_selection_to_session(&mut session, &options) {
            session = self.store.persist_execution_config(session).await?;
        }
        self.run_until_stable(
            session,
            &options,
            StableRunContext {
                base_run_source: ExecutionSource::User,
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
        let failure_context = request.run_in_background.then(|| {
            (
                request.parent_session_id,
                request.launch_call_id,
                request.task_id.clone(),
            )
        });
        let result = self.run_subtask_inner(request).await;
        if let Err(error) = &result
            && let Some((parent_id, Some(call_id), task_id)) = failure_context
        {
            match self
                .record_background_task_start_failure(parent_id, call_id, task_id.as_deref(), error)
                .await
            {
                Ok(Some((delivery, notification))) => {
                    let manager = self.background_handle();
                    tokio::spawn(async move {
                        if let Err(error) = manager
                            .dispatch_background_delivery(delivery, notification)
                            .await
                        {
                            tracing::warn!(%error, "failed task-start notification will be retried");
                        }
                    });
                }
                Ok(None) => {}
                Err(record_error) => {
                    tracing::warn!(%record_error, "failed to persist background task-start failure")
                }
            }
        }
        result
    }

    async fn record_background_task_start_failure(
        &self,
        parent_id: i64,
        call_id: i64,
        task_id: Option<&str>,
        error: &AppError,
    ) -> Result<
        Option<(
            agena_storage::store::BackgroundDelivery,
            agena_runtime_contracts::part_content::SystemNotificationContent,
        )>,
        AppError,
    > {
        use agena_runtime_contracts::part_content::SystemNotificationContent;
        use agena_storage::store::{
            BackgroundEventRequest, BackgroundOperationKind, BackgroundOperationPhase,
            NewBackgroundOperation,
        };

        self.session_mutations.run(parent_id, async {
            let parent = self.store.load_session(parent_id).await?;
            let Some((run_id, tool_part_id)) = parent.parts().iter().find_map(|part| {
                let operation = super::replies::operation_from_part(part)?;
                (operation.call_id == call_id && part.role == PartRole::Assistant)
                    .then_some((part.run_id?, part.part_id))
            }) else { return Ok(None); };
            let operations = self.store.facade.active_background_operations_for_session(
                parent_id, Some(BackgroundOperationKind::Task), 64,
            ).await.map_err(crate::session::store::store_error)?;
            let operation = match operations.into_iter().find(|operation| operation.launch_tool_part_id == Some(tool_part_id)) {
                Some(operation) => operation,
                None => self.store.create_background_operation(NewBackgroundOperation {
                    operation_id: super::helpers::background_operation_id(parent_id, tool_part_id),
                    session_id: parent_id,
                    launch_run_id: Some(run_id),
                    launch_tool_part_id: Some(tool_part_id),
                    kind: BackgroundOperationKind::Task,
                }).await?,
            };
            if operation.phase.is_terminal() { return Ok(None); }
            if let Some(child_id) = operation.outcome.as_ref()
                .and_then(|value| value.get("child_session_id"))
                .and_then(serde_json::Value::as_i64)
                && !self.execution_registry.is_active(child_id).await
            {
                let mut child = self.store.load_session(child_id).await?;
                if !child.runtime.subtask.status.is_terminal() {
                    let started = child.runtime.subtask.started_at_ms.or_else(|| {
                        operation.outcome.as_ref()
                            .and_then(|value| value.get("started_at_ms"))
                            .and_then(serde_json::Value::as_i64)
                    }).unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
                    let finished = chrono::Utc::now().timestamp_millis().max(started);
                    let status = if matches!(error, AppError::Cancelled) {
                        agena_domain::SubtaskStatus::Cancelled
                    } else {
                        agena_domain::SubtaskStatus::Failed
                    };
                    child = self.store.update_subtask_state(
                        child, Some(status.as_ref().to_owned()), Some(started), Some(finished),
                        if status == agena_domain::SubtaskStatus::Cancelled { None } else {
                            Some(serde_json::to_value(error.failure()).map_err(|error| AppError::Internal(format!("encode task failure: {error}")))?)
                        },
                    ).await?;
                }
                // A completion write may have failed after a successful run.
                // Preserve the child's committed terminal fact when retrying.
                return self.record_background_task_completion(&operation, &child, task_id.unwrap_or("task")).await;
            }
            let cancelled = matches!(error, AppError::Cancelled);
            let failure = error.failure();
            let notification = SystemNotificationContent {
                operation_id: operation.external_id.clone().unwrap_or_else(|| operation.operation_id.clone()),
                operation_kind: "task".to_owned(),
                status: if cancelled { "cancelled" } else { "failed" }.to_owned(),
                summary: format!("Task {} failed: {}", task_id.unwrap_or("launch"), failure.user.fallback),
                body: failure.user.fallback.clone(),
                ..Default::default()
            };
            let settled = self.store.record_background_event(BackgroundEventRequest {
                operation_id: operation.operation_id,
                event_key: "terminal".to_owned(),
                event_seq: None,
                next_phase: Some(if cancelled { BackgroundOperationPhase::Cancelled } else { BackgroundOperationPhase::Failed }),
                outcome: None,
                failure: Some(serde_json::json!({"id":failure.id.to_string(),"message":failure.user.fallback})),
                notification: new_part_from_content("system_notification", PartRole::Assistant, &TypedContent::SystemNotification(notification.clone()), PartState::Completed)?,
            }).await?;
            Ok(Some((settled.delivery, notification)))
        }).await
    }

    async fn run_subtask_inner(
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
            .commands
            .as_deref()
            .filter(|commands| !commands.is_empty())
        {
            Some(commands) => Some(resolve_subtask_command_references(
                &state.tool_executor.plugin_manager().command_catalog(),
                commands,
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
        let parent = self
            .load_session_with_workspace_root(request.parent_session_id)
            .await?;
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
        let (resumed, child_id, baseline_message_id, baseline_usage, usage_budget, background_id) = self
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
                        self.load_session_with_workspace_root(existing_id).await?
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
                        self.load_session_with_workspace_root(child_id).await?
                    }
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
                // The child owns the same durable workspace id and therefore
                // already has its project base root bound above. Only inherit a
                // real snapshot/worktree override; copying the parent's derived
                // project root into this persisted override would make an
                // ordinary project session look permanently snapshot-scoped.
                child.runtime.execution.effective_workspace_root =
                    parent.runtime.execution.effective_workspace_root.clone();
                if child.runtime.subtask.status.is_terminal()
                    && let Some(external_id) = self
                        .background_task_external_id_for_run(
                            parent.id,
                            &task_id,
                            child.id,
                            child.runtime.subtask.started_at_ms,
                        )
                        .await?
                    && let Some(operation) = self
                        .store
                        .background_operation_by_external_id(
                            agena_storage::store::BackgroundOperationKind::Task,
                            &external_id,
                        )
                        .await?
                    && !operation.phase.is_terminal()
                    && let Some((delivery, notification)) = self
                        .record_background_task_completion(&operation, &child, &task_id)
                        .await?
                {
                    let manager = self.background_handle();
                    tokio::spawn(async move {
                        if let Err(error) = manager
                            .dispatch_background_delivery(delivery, notification)
                            .await
                        {
                            tracing::warn!(%error, "prior task-run completion delivery will be retried");
                        }
                    });
                }
                // The timestamp is also the durable run correlation key.
                // Guarantee uniqueness even for very fast consecutive resumes.
                let started_at_ms = chrono::Utc::now().timestamp_millis().max(
                    child.runtime.subtask.started_at_ms.unwrap_or(0).saturating_add(1),
                );
                let background_id = if request.run_in_background {
                    use agena_storage::store::{
                        BackgroundOperationKind, NewBackgroundOperation,
                    };
                    let initial = self
                        .store
                        .background_operation_by_external_id(BackgroundOperationKind::Task, &task_id)
                        .await?;
                    let launch = request.launch_call_id.and_then(|call_id| {
                        parent.parts().iter().find_map(|part| {
                            let operation = super::replies::operation_from_part(part)?;
                            (operation.call_id == call_id && part.role == PartRole::Assistant)
                                .then_some((part.run_id?, part.part_id))
                        })
                    });
                    let operation = match initial {
                        Some(operation)
                            if operation.session_id == parent.id
                                && !operation.phase.is_terminal()
                                && operation.outcome.is_none() => Some(operation),
                        initial if launch.is_some() => {
                            let (launch_run_id, launch_tool_part_id) = launch.expect("checked launch receipt");
                            let run_id = format!("task_run_{}_{started_at_ms}", child.id);
                            let external_id = if initial.is_none() && !resumed {
                                task_id.clone()
                            } else {
                                run_id.clone()
                            };
                            Some(self.prepare_background_launch(
                                NewBackgroundOperation {
                                    operation_id: run_id,
                                    session_id: parent.id,
                                    launch_run_id: Some(launch_run_id),
                                    launch_tool_part_id: Some(launch_tool_part_id),
                                    kind: BackgroundOperationKind::Task,
                                },
                                external_id,
                            )
                            .await
                            .map_err(|error| match error {
                                super::BackgroundLaunchError::Tool(error) => AppError::Config(error.to_string()),
                                super::BackgroundLaunchError::Session(error) => error,
                            })?)
                        }
                        _ => None,
                    };
                    if let Some(operation) = operation {
                        let operation = self
                            .bind_background_task_run(
                                &operation.operation_id,
                                &task_id,
                                child.id,
                                started_at_ms,
                            )
                            .await?;
                        Some(operation.operation_id)
                    } else {
                        None
                    }
                } else {
                    None
                };
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
                // This live runtime owns the launch. Mark the child reconciled
                // while it is still in Created state, before publishing
                // Running. Otherwise a session-tree/TUI read can land in the
                // few milliseconds before execute_registered installs the
                // child registry entry and misclassify the brand-new task as
                // restart-orphaned, writing Interrupted and notifying the
                // parent even though execution is about to begin. A real
                // process restart constructs a fresh manager with an empty
                // reconciled set, so crash recovery remains intact.
                self.reconciled_sessions.lock().await.insert(child.id);
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
                    background_id,
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
                parts.push(TypedContent::CommandRef(command_ref_from_reference(
                    &CommandReferencePart {
                        commands: skill_references,
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
                Err(timeout_error) => {
                    timed_out = true;
                    tracing::warn!(
                        session_id = child_id,
                        task_id = task_id.as_str(),
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            format!(
                                "subtask execution timed out after {}ms",
                                request.timeout_ms.unwrap_or_default()
                            ),
                            &timeout_error,
                        ),
                        "subtask execution deadline expired"
                    );
                    // `execute_registered` has no suspension point between
                    // registry insertion and lifecycle-owner spawn. If there
                    // is no active execution now, this future timed out before
                    // registration and is safe to drop. Otherwise signal the
                    // supervised owner and give it a bounded cleanup window.
                    if self.execution_registry.is_active(child_id).await {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            self.cancel_active_execution_with_outcome(child_id),
                        )
                        .await
                        {
                            Ok(Ok(_)) => {}
                            Ok(Err(error)) => tracing::error!(
                                session_id = child_id,
                                task_id = task_id.as_str(),
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "failed to request cancellation for a timed-out subtask",
                                    &error,
                                ),
                                "timed-out subtask cancellation request failed"
                            ),
                            Err(error) => tracing::error!(
                                session_id = child_id,
                                task_id = task_id.as_str(),
                                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                    "timed out after 2 seconds while requesting cancellation for a timed-out subtask",
                                    &error,
                                ),
                                "timed-out subtask cancellation request did not complete"
                            ),
                        }
                        match tokio::time::timeout(std::time::Duration::from_secs(5), &mut run)
                            .await
                        {
                            Ok(result) => result,
                            Err(error) => {
                                tracing::error!(
                                    session_id = child_id,
                                    task_id = task_id.as_str(),
                                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                        "timed out after 5 seconds while waiting for a cancelled subtask to finish cleanup",
                                        &error,
                                    ),
                                    "timed-out subtask cleanup did not complete"
                                );
                                Err(AppError::Cancelled)
                            }
                        }
                    } else {
                        Err(AppError::Cancelled)
                    }
                }
            }
        } else {
            run.await
        };

        let timeout_failure = || {
            subtask_timeout_failure(
                task_id.as_str(),
                description,
                request
                    .timeout_ms
                    .expect("timed_out is only set when timeout_ms is present"),
            )
        };
        let (status, failure, budget_exceeded, mut session) = match run_result {
            Ok(session) if timed_out => (
                agena_domain::SubtaskStatus::TimedOut,
                Some(timeout_failure()),
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
                // A timeout deliberately cancels the supervised child, so the
                // returned error is commonly AppError::Cancelled. The timer is
                // the authoritative cause: persist a complete timeout failure
                // regardless of the cleanup future's return value.
                let failure = if timed_out {
                    Some(timeout_failure())
                } else {
                    (!matches!(&error, AppError::Cancelled)).then(|| error.failure())
                };
                let session = self.load_session_with_workspace_root(child_id).await?;
                (status, failure, budget_exceeded, session)
            }
        };
        let finished_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .max(session.runtime.subtask.started_at_ms.unwrap_or(0));
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

        if let Some(background_id) = background_id {
            let completion = self
                .session_mutations
                .run(parent.id, async {
                    let operation = self
                        .store
                        .background_operation(&background_id)
                        .await?
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "background task run {background_id} disappeared"
                            ))
                        })?;
                    self.record_background_task_completion(&operation, &session, &task_id)
                        .await
                })
                .await?;
            if let Some((delivery, notification)) = completion {
                let manager = self.background_handle();
                // A task waiter may still occupy the parent execution. Do not
                // make the child's return wait for its notification ack.
                tokio::spawn(async move {
                    if let Err(error) = manager
                        .dispatch_background_delivery(delivery, notification)
                        .await
                    {
                        tracing::warn!(%error, "task-run completion delivery will be retried");
                    }
                });
            }
        }

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

fn subtask_timeout_failure(task_id: &str, description: &str, timeout_ms: u64) -> Failure {
    let label = if description.trim().is_empty() {
        task_id.to_owned()
    } else {
        format!("\"{}\" ({task_id})", description.trim())
    };
    Failure::new(
        FailureCode::new("subtask.timeout"),
        FailureCategory::Timeout,
        FailureResponsibility::System,
        RetryDirective::UseAlternative,
        RecoveryDirective::ChooseAlternative,
        FailureImpact::OperationFailed,
        UserPresentation::validated(
            "subtask-timeout",
            format!("Task {label} timed out after {timeout_ms} ms."),
        ),
    )
}

pub(in crate::session::manager) fn non_recursive_subtask_capability_denials()
-> std::collections::BTreeSet<String> {
    [
        "task",
        "tasks.run",
        "agena.tasks.run",
        "agena_tasks_run",
        "agena.tasks.followup",
        "agena.tasks.message",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        command_declaration_hash, command_matches, non_recursive_subtask_capability_denials,
        resolve_subtask_command_references,
    };
    use agena_plugin_host::{CommandCatalogItem, PluginKey};
    use agena_plugin_sdk::{CommandDefinition, CommandDocs, CommandTarget, SettingsContract};
    use agena_runtime_contracts::part::CommandReference;

    fn catalog_entry(id: &str, slash: &str, aliases: &[&str], summary: &str) -> CommandCatalogItem {
        CommandCatalogItem {
            plugin_id: PluginKey::new("agena", "commands").expect("plugin key"),
            accepts_empty_input: true,
            default_input: serde_json::json!({}),
            command: CommandDefinition {
                id: id.to_string(),
                title: id.to_string(),
                group: "Commands".to_string(),
                category: Some("Package".to_string()),
                slash: Some(slash.to_string()),
                aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                docs: CommandDocs {
                    summary: Some(summary.to_string()),
                    ..CommandDocs::default()
                },
                input: SettingsContract::empty_object("No input", ""),
                target: CommandTarget::Method {
                    handler: "run".to_string(),
                },
            },
        }
    }

    fn catalog() -> Vec<CommandCatalogItem> {
        vec![
            catalog_entry(
                "verify",
                "/verify",
                &["check"],
                "Validate the current change",
            ),
            catalog_entry(
                "security_review",
                "/security_review",
                &["security-review"],
                "Audit for security regressions",
            ),
        ]
    }

    #[test]
    fn delegated_instances_cannot_recursively_run_tasks() {
        let names = non_recursive_subtask_capability_denials();
        for name in [
            "task",
            "tasks.run",
            "agena.tasks.run",
            "agena_tasks_run",
            "agena.tasks.followup",
            "agena.tasks.message",
        ] {
            assert!(names.contains(name));
        }
    }

    #[test]
    fn subtask_references_resolve_catalog_ids_slashes_and_aliases() {
        let requested = vec![
            "verify".to_string(),
            "/security_review".to_string(),
            "security-review".to_string(),
        ];
        let refs = resolve_subtask_command_references(&catalog(), &requested).expect("resolve");
        let names = refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["verify", "security_review", "security_review"]);
        for reference in &refs {
            assert!(!reference.content_hash.is_empty());
        }
    }

    #[test]
    fn subtask_references_carry_the_declared_summary_and_owner() {
        let requested = vec!["/verify".to_string()];
        let refs = resolve_subtask_command_references(&catalog(), &requested).expect("resolve");
        assert_eq!(refs[0].description, "Validate the current change");
        assert_eq!(refs[0].source, "agena.commands");
        assert_eq!(refs[0].aliases, ["check"]);
    }

    #[test]
    fn subtask_references_reject_unknown_names() {
        let requested = vec!["no-such-command".to_string()];
        let error = resolve_subtask_command_references(&catalog(), &requested).expect_err("reject");
        assert!(
            error
                .to_string()
                .contains("unknown command 'no-such-command'")
        );
    }

    #[test]
    fn reference_identity_tracks_the_declaration_not_the_body() {
        let entry = catalog_entry(
            "verify",
            "/verify",
            &["check"],
            "Validate the current change",
        );
        let baseline = command_declaration_hash(&entry);

        // Renaming the slash spelling changes the identity a user selected.
        let mut renamed = entry.clone();
        renamed.command.slash = Some("/check".to_string());
        assert_ne!(command_declaration_hash(&renamed), baseline);

        // An unrelated command with different copy does not collide.
        let other = catalog_entry("explore", "/explore", &[], "Explore a codebase");
        assert_ne!(command_declaration_hash(&other), baseline);
    }

    #[test]
    fn command_matching_accepts_three_spellings_and_rejects_an_empty_query() {
        let entry = catalog_entry(
            "verify",
            "/verify",
            &["check"],
            "Validate the current change",
        );
        for spelling in ["verify", "VERIFY", " /verify ", "/VERIFY", "check", "Check"] {
            assert!(command_matches(&entry, spelling), "{spelling}");
        }
        assert!(!command_matches(&entry, ""));
        assert!(!command_matches(&entry, "unrelated"));
    }

    #[test]
    fn skill_reference_carries_stable_identity_without_body() {
        let requested = vec!["verify".to_string()];
        let refs = resolve_subtask_command_references(&catalog(), &requested).expect("resolve");
        let first = &refs[0];
        let expected: CommandReference = serde_json::from_value(serde_json::json!({
            "name": first.name,
            "description": first.description,
            "content_hash": first.content_hash,
            "source": first.source,
            "aliases": first.aliases,
        }))
        .expect("serializable lazy reference");
        assert_eq!(&expected, first);
    }
}
