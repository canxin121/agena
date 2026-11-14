//! Reliable business deliveries are consumed at a provider boundary. An idle
//! wake records the notice, consumption and queued execution atomically.
use super::*;
use serde_json::Value;

impl SessionManager {
    pub(super) async fn stop_conditions(
        &self,
        session: SessionId,
        state: &ExecutionState,
    ) -> Result<agena_storage::kernel::StopConditions, AppError> {
        let mut conditions = agena_storage::kernel::StopConditions::default();
        let Some(scheduler) = state.executor.scheduler() else {
            return Ok(conditions);
        };
        let jobs = scheduler
            .pending_jobs_for_session(session.get())
            .await
            .map_err(|error| {
                AppError::Internal(format!(
                    "cannot read session scheduler wake sources: {error}"
                ))
            })?;
        for job in jobs {
            if matches!(job.kind, agena_scheduler::JobKind::Once { .. })
                && let Some(delivery) = job.pending_delivery
            {
                let key = format!("scheduler:{}:{}", job.id, delivery.delivery_key);
                if key.len() > 1024 || conditions.pending_once_delivery_keys.len() == 128 {
                    conditions.scheduler_waiting = true;
                    break;
                }
                conditions.pending_once_delivery_keys.push(key);
            } else {
                conditions.scheduler_waiting = true;
                break;
            }
        }
        Ok(conditions)
    }

    pub async fn deliver_scheduled_job(
        &self,
        id: i64,
        job: String,
        delivery: String,
        prompt: String,
        provenance: Option<agena_scheduler::ScheduledJobLaunchProvenance>,
    ) -> Result<bool, AppError> {
        if prompt.trim().is_empty() {
            return Err(AppError::Config(
                "scheduled prompt must not be empty".into(),
            ));
        }
        let session = session_id(id)?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        self.require_writable_bot_history(id).await?;
        let payload = serde_json::json!({"kind":"scheduler","job_id":job,"delivery_key":delivery,"text":prompt,"launch_provenance":provenance});
        if serde_json::to_vec(&payload)?.len() > 64 * 1024 {
            return Err(AppError::Config(
                "scheduled delivery exceeds its byte budget".into(),
            ));
        }
        let key = format!("scheduler:{job}:{delivery}");
        let request = RequestId::new();
        let receipt = persist(|| {
            self.kernel()
                .executor
                .deliver(request, session, key.clone(), payload.clone())
        })
        .await
        .map_err(kernel_error)?;
        Ok(!receipt.value.consumed)
    }

    pub(super) async fn consume_boundary_deliveries(
        &self,
        session: SessionId,
        execution: ExecutionId,
        message: EntryId,
    ) -> Result<usize, AppError> {
        let mut consumed = 0;
        let mut after = None;
        loop {
            let page = persist(|| self.kernel().reader.pending_deliveries(session, after, 64))
                .await
                .map_err(kernel_error)?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|delivery| delivery.id);
            for delivery in page.iter() {
                if delivery
                    .target_execution
                    .is_some_and(|target| target != execution)
                {
                    continue;
                }
                let target = self
                    .delivery_message(session, &delivery.payload, message)
                    .await?;
                let text = delivery_text(&delivery.payload)?;
                let request = request_key("consume-delivery", &delivery.id.to_string());
                persist(|| {
                    self.kernel().executor.consume_delivery(
                        request,
                        session,
                        delivery.id,
                        target,
                        text.clone(),
                    )
                })
                .await
                .map_err(kernel_error)?;
                consumed += 1;
            }
            if page.len() < 64 {
                break;
            }
        }
        Ok(consumed)
    }
    pub async fn recover_background_deliveries(&self, limit: usize) -> Result<usize, AppError> {
        if self.shutdown.is_cancelled() {
            return Ok(0);
        }
        let before = *self.delivery_scan.lock().unwrap_or_else(|p| p.into_inner());
        let page = self
            .kernel()
            .reader
            .query_summaries(SummaryQuery {
                include_global: true,
                before,
                limit: limit.clamp(1, 128),
                ..Default::default()
            })
            .await
            .map_err(kernel_error)?;
        *self.delivery_scan.lock().unwrap_or_else(|p| p.into_inner()) =
            if page.has_more { page.next } else { None };
        let mut recovered = 0;
        for summary in &page.sessions {
            let dispatch = self.session_dispatch(summary.id);
            let Ok(_submission) = dispatch.submission.try_lock() else {
                continue;
            };
            match self.require_writable_bot_history(summary.id.get()).await {
                Ok(()) => {}
                Err(
                    reason @ (AppError::PersistenceConflict(_)
                    | AppError::BotConflict(_)
                    | AppError::BotLocalHistory { .. }),
                ) => {
                    tracing::debug!(session_id = summary.id.get(), %reason, "read-only Bot history delivery recovery deferred");
                    continue;
                }
                Err(error) => return Err(error),
            }
            let Ok(_guard) = dispatch.execution.try_lock() else {
                continue;
            };
            if self
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .values()
                .any(|run| run.session == summary.id)
            {
                continue;
            }
            if self
                .kernel()
                .reader
                .configuration(summary.id)
                .await
                .map_err(kernel_error)?
                .message_queue_paused
            {
                continue;
            }
            if summary.queued_executions > 0 {
                // A missing route must not starve recovery of other sessions.
                match self.restore_queued_executions(summary.id).await {
                    Ok(count) => recovered += count,
                    Err(error) => {
                        tracing::warn!(session_id = summary.id.get(), %error, "queued input recovery deferred")
                    }
                }
                continue;
            }
            let pending = self
                .kernel()
                .reader
                .pending_deliveries(summary.id, None, 128)
                .await
                .map_err(kernel_error)?;
            let Some(delivery) = pending.iter().find(|delivery| {
                delivery.target_execution.is_none()
                    && delivery.payload.get("kind").and_then(Value::as_str) != Some("agent_message")
            }) else {
                continue;
            };
            // Targeted guidance belongs only to its supervised owner. A
            // terminal transition/recovery retires it without publishing a
            // notice; idle delivery recovery never transfers that authority.
            if delivery.target_execution.is_some() {
                continue;
            }
            let metadata = self
                .kernel()
                .reader
                .metadata(summary.id)
                .await
                .map_err(kernel_error)?;
            let options = match self
                .options_for_session(summary.id, &metadata.selection.clone().unwrap_or_default())
                .await
            {
                Ok(options) => options,
                Err(_) => continue,
            };
            let Some(fallback) = self.latest_origin_message(summary.id).await? else {
                continue;
            };
            let message = self
                .delivery_message(summary.id, &delivery.payload, fallback)
                .await?;
            let text = delivery_text(&delivery.payload)?;
            let live = self
                .kernel()
                .reader
                .active_executions(summary.id, None, 1)
                .await
                .map_err(kernel_error)?;
            if !live.executions.is_empty() {
                continue;
            }
            if delivery.payload.get("wake").and_then(Value::as_bool) == Some(false) {
                let command = request_key("consume-delivery", &delivery.id.to_string());
                persist(|| {
                    self.kernel().executor.consume_delivery(
                        command,
                        summary.id,
                        delivery.id,
                        message,
                        text.clone(),
                    )
                })
                .await
                .map_err(kernel_error)?;
                recovered += 1;
                continue;
            }
            let permit = match self.reserve_execution() {
                Ok(permit) => permit,
                Err(_) => break,
            };
            let command = request_key("resume-delivery", &delivery.id.to_string());
            let execution = persist(|| {
                self.kernel().executor.resume_delivery(
                    command,
                    summary.id,
                    delivery.id,
                    message,
                    text.clone(),
                )
            })
            .await
            .map_err(kernel_error)?
            .value;
            self.supervise(execution, options, false, permit).await?;
            recovered += 1;
        }
        Ok(recovered)
    }

    /// Called under the session submission lock, including before accepting a
    /// new message. Startup recovery may not have scanned this session yet;
    /// existing durable input must be dispatched before any newer submission.
    pub(super) async fn restore_queued_executions(
        &self,
        session: SessionId,
    ) -> Result<usize, AppError> {
        self.require_writable_bot_history(session.get()).await?;
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 128)
            .await
            .map_err(kernel_error)?;
        if snapshot.configuration.message_queue_paused {
            return Ok(0);
        }
        let mut queued = snapshot
            .active_executions
            .executions
            .iter()
            .filter(|execution| execution.phase == ExecutionPhase::Queued)
            .cloned()
            .collect::<Vec<_>>();
        queued.sort_by_key(|execution| execution.message_id);
        let mut recovered = 0;
        for execution in queued {
            if self
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&execution.id)
                .is_some_and(|run| run.generation >= execution.generation)
            {
                continue;
            }
            let options = self
                .options_for_session(
                    session,
                    &snapshot.metadata.selection.clone().unwrap_or_default(),
                )
                .await?;
            let permit = self.reserve_execution()?;
            // A queued maintenance action keeps its purpose across pause,
            // shutdown and recovery. It must not turn into an ordinary reply.
            let compact = execution.purpose == ExecutionPurpose::Compaction;
            self.supervise(execution, options, compact, permit).await?;
            recovered += 1;
        }
        Ok(recovered)
    }
    pub(super) async fn latest_origin_message(
        &self,
        session: SessionId,
    ) -> Result<Option<EntryId>, AppError> {
        let page = self
            .kernel()
            .reader
            .history(session, None, 1)
            .await
            .map_err(kernel_error)?;
        let Some(last) = page.entries.first() else {
            return Ok(None);
        };
        let message = self
            .kernel()
            .reader
            .entry(session, last.message_id)
            .await
            .map_err(kernel_error)?;
        Ok(self
            .owns_message_boundary(session, &message)
            .await?
            .then_some(message.id))
    }

    async fn owns_message_boundary(
        &self,
        session: SessionId,
        entry: &EntryView,
    ) -> Result<bool, AppError> {
        if entry.origin_session_id != session {
            return Ok(false);
        }
        if matches!(entry.payload, EntryPayload::User { .. }) {
            return Ok(true);
        }
        if !matches!(entry.payload, EntryPayload::Notice { .. }) {
            return Ok(false);
        }
        // Only an explicitly typed control execution may use a notice as its
        // local input boundary. Ordinary notices and inherited history never
        // acquire permission to receive execution guidance this way.
        let execution = self
            .kernel()
            .reader
            .execution(session, entry.execution_id)
            .await
            .map_err(kernel_error)?;
        Ok(execution.session_id == session
            && execution.message_id == entry.id
            && execution.purpose != ExecutionPurpose::Message)
    }
    async fn delivery_message(
        &self,
        session: SessionId,
        payload: &Value,
        fallback: EntryId,
    ) -> Result<EntryId, AppError> {
        let message = payload
            .get("message_id")
            .and_then(Value::as_i64)
            .map(entry_id)
            .transpose()?
            .unwrap_or(fallback);
        let entry = self
            .kernel()
            .reader
            .entry(session, message)
            .await
            .map_err(kernel_error)?;
        if !self.owns_message_boundary(session, &entry).await? {
            return Err(AppError::Config(
                "background delivery does not own its input boundary".into(),
            ));
        }
        Ok(message)
    }
    pub(super) async fn flush_hook_runs(
        &self,
        attempt: &AttemptHandle,
        session: SessionId,
        state: &ExecutionState,
    ) -> Result<(), AppError> {
        let runs = state
            .executor
            .plugin_manager()
            .drain_hook_runs(session.get());
        // One hook run is one transcript activity. Encoding each run as its
        // own marker notice keeps the hook identity, status, summary, detail
        // and continuation attached to the same part instead of flattening a
        // batch into user-visible text.
        for run in runs {
            let text = hook_notice_text(&run)?;
            let command = RequestId::new();
            match persist(|| attempt.notice(command, text.clone())).await {
                Ok(_) => {}
                Err(KernelError::StaleAttempt | KernelError::Conflict(_)) => {
                    let execution = match persist(|| {
                        self.kernel()
                            .reader
                            .execution(session, attempt.execution_id())
                    })
                    .await
                    {
                        Ok(execution) => execution,
                        Err(KernelError::NotFound(_)) => continue,
                        Err(error) => return Err(kernel_error(error)),
                    };
                    let delivery = request_key("late-hook-observation", &command.to_string());
                    let text = hook_delivery_text(&run, execution.message_id.get())?;
                    let payload = serde_json::json!({"kind":"hook_observation","message_id":execution.message_id.get(),"text":text,"wake":false});
                    persist(|| {
                        self.kernel().executor.deliver(
                            delivery,
                            session,
                            format!("hook:{command}"),
                            payload.clone(),
                        )
                    })
                    .await
                    .map_err(kernel_error)?;
                }
                Err(error) => return Err(kernel_error(error)),
            }
        }
        Ok(())
    }
}

// Direct hook notices may use the full notice budget. The late-delivery path
// applies the smaller delivery-payload budget after wrapping the marker.
const HOOK_NOTICE_MAX_BYTES: usize = 96 * 1024;
const DELIVERY_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

fn hook_notice_text(run: &agena_plugin_host::HookRunRecord) -> Result<String, AppError> {
    hook_notice_text_with_limit(run, HOOK_NOTICE_MAX_BYTES)
}

fn hook_delivery_text(
    run: &agena_plugin_host::HookRunRecord,
    message_id: i64,
) -> Result<String, AppError> {
    let mut content = hook_content(run);
    loop {
        let text = agena_runtime_contracts::part_content::hook_notice_text(&content)?;
        let payload = serde_json::json!({
            "kind": "hook_observation",
            "message_id": message_id,
            "text": text,
            "wake": false,
        });
        if serde_json::to_vec(&payload)?.len() <= DELIVERY_PAYLOAD_MAX_BYTES {
            return Ok(text);
        }
        if !shrink_hook_content(&mut content)? {
            return Err(AppError::Internal(
                "plugin hook activity exceeds the delivery byte limit".into(),
            ));
        }
    }
}

fn hook_notice_text_with_limit(
    run: &agena_plugin_host::HookRunRecord,
    max_bytes: usize,
) -> Result<String, AppError> {
    let mut content = hook_content(run);
    loop {
        let text = agena_runtime_contracts::part_content::hook_notice_text(&content)?;
        if text.len() <= max_bytes {
            return Ok(text);
        }
        if !shrink_hook_content(&mut content)? {
            return Err(AppError::Internal(
                "plugin hook activity exceeds the notice byte limit".into(),
            ));
        }
    }
}

fn hook_content(
    run: &agena_plugin_host::HookRunRecord,
) -> agena_runtime_contracts::part_content::HookContent {
    let status = match run.status {
        agena_plugin_host::HookRunStatus::Applied => "applied",
        agena_plugin_host::HookRunStatus::Skipped => "skipped",
        agena_plugin_host::HookRunStatus::Failed => "failed",
        agena_plugin_host::HookRunStatus::TimedOut => "timed_out",
    };
    let mut content = agena_runtime_contracts::part_content::HookContent {
        hook: run.hook.clone(),
        plugin_id: Some(run.plugin_id.clone()),
        summary: crate::prompt_budget::truncate_prompt_text(
            &run.summary,
            4096,
            " [hook summary limited]",
        ),
        detail: run.detail.as_deref().map(|detail| {
            crate::prompt_budget::truncate_prompt_text(detail, 64 * 1024, " [hook detail limited]")
        }),
        message: run.message.as_deref().map(|message| {
            crate::prompt_budget::truncate_prompt_text(
                message,
                192 * 1024,
                " [hook continuation limited]",
            )
        }),
        extra: Default::default(),
    };
    content
        .extra
        .insert("status".into(), serde_json::Value::String(status.into()));
    content
}

fn shrink_hook_content(
    content: &mut agena_runtime_contracts::part_content::HookContent,
) -> Result<bool, AppError> {
    // JSON escaping can grow a large continuation beyond the notice limit.
    // Shrink the content fields until the encoded marker fits; never cut the
    // serialized JSON itself, which would make the marker unreadable.
    if shrink_hook_field(&mut content.message) {
        return Ok(true);
    }
    if shrink_hook_field(&mut content.detail) {
        return Ok(true);
    }
    if content.summary.len() > 1024 {
        content.summary = crate::prompt_budget::truncate_prompt_text(
            &content.summary,
            1024,
            " [hook summary limited]",
        );
        return Ok(true);
    }
    Ok(false)
}

fn shrink_hook_field(field: &mut Option<String>) -> bool {
    let Some(value) = field.as_mut() else {
        return false;
    };
    if value.is_empty() {
        *field = None;
        return true;
    }
    *value = crate::prompt_budget::truncate_prompt_text(
        value,
        value.len() / 2,
        " [hook activity limited]",
    );
    true
}

pub(super) fn delivery_text(payload: &Value) -> Result<String, AppError> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let structured = match kind {
        "shell" | "monitor" => Some(delivery_system_notification(
            payload,
            kind,
            "process_id",
            "text",
            "completed",
        )?),
        "scheduler" => Some(delivery_system_notification(
            payload,
            "scheduled_delivery",
            "delivery_key",
            "text",
            "ready",
        )?),
        "bot_instruction" => Some(delivery_system_notification(
            payload,
            "operator_bot",
            "source_session_id",
            "text",
            "instruction",
        )?),
        "integration_instruction" => Some(delivery_system_notification(
            payload,
            "integration",
            "binding_id",
            "text",
            "instruction",
        )?),
        "assistant_notice" => Some(delivery_system_notification(
            payload,
            "assistant",
            "notice_id",
            "text",
            "updated",
        )?),
        "integration_notice" => Some(delivery_system_notification(
            payload,
            "integration",
            "notice_id",
            "text",
            "updated",
        )?),
        "agent_result" => Some(delivery_system_notification(
            payload,
            "task",
            "agent_id",
            "text",
            "completed",
        )?),
        _ => None,
    };
    if let Some(text) = structured {
        return Ok(text);
    }
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| payload.get("summary").and_then(Value::as_str));
    let value = match text {
        Some(text) => text.to_owned(),
        None => serde_json::to_string(payload)?,
    };
    if is_structured_notice(&value) {
        return Ok(value);
    }
    Ok(crate::prompt_budget::truncate_prompt_text(
        &value,
        64 * 1024,
        "\n[Notification preview limited; its complete business payload remains recorded.]",
    ))
}

const DELIVERY_NOTICE_MAX_BYTES: usize = 96 * 1024;

fn is_structured_notice(text: &str) -> bool {
    !matches!(
        agena_runtime_contracts::part_content::decode_notice(text),
        agena_runtime_contracts::part_content::DecodedNotice::Plain(_)
    )
}

fn delivery_system_notification(
    payload: &Value,
    operation_kind: &str,
    operation_id_key: &str,
    body_key: &str,
    default_status: &str,
) -> Result<String, AppError> {
    let body = payload
        .get(body_key)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let summary = payload
        .get("summary")
        .and_then(Value::as_str)
        .filter(|summary| !summary.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| first_delivery_line(body));
    let mut notification = agena_runtime_contracts::part_content::SystemNotificationContent {
        operation_id: delivery_operation_id(payload.get(operation_id_key)).unwrap_or_default(),
        operation_kind: operation_kind.to_owned(),
        status: payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(default_status)
            .to_owned(),
        summary,
        detail: payload
            .get("detail")
            .and_then(Value::as_str)
            .map(str::to_owned),
        body: body.to_owned(),
        event_seq: payload.get("event_seq").and_then(Value::as_u64),
        ..Default::default()
    };
    if let Some(fields) = payload.as_object() {
        for (key, value) in fields {
            if matches!(
                key.as_str(),
                "kind" | "text" | "summary" | "detail" | "status" | "event_seq" | "message_id"
            ) || key == operation_id_key
            {
                continue;
            }
            notification.extra.insert(key.clone(), value.clone());
        }
    }
    encode_delivery_notification(notification)
}

fn delivery_operation_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(id) => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

fn first_delivery_line(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    crate::prompt_budget::truncate_prompt_text(line, 4096, " [notification summary limited]")
}

fn encode_delivery_notification(
    mut notification: agena_runtime_contracts::part_content::SystemNotificationContent,
) -> Result<String, AppError> {
    loop {
        let text =
            agena_runtime_contracts::part_content::system_notification_notice_text(&notification)?;
        if text.len() <= DELIVERY_NOTICE_MAX_BYTES {
            return Ok(text);
        }
        // JSON escaping can make a large body outgrow its source bytes. Shrink
        // fields until the complete marker fits; never truncate serialized JSON,
        // which would make the typed part unreadable after it is persisted.
        if notification.body.len() > 256 {
            notification.body = crate::prompt_budget::truncate_prompt_text(
                &notification.body,
                notification.body.len() / 2,
                " [notification body limited]",
            );
            continue;
        }
        if let Some(detail) = notification.detail.as_mut() {
            if detail.len() > 256 {
                *detail = crate::prompt_budget::truncate_prompt_text(
                    detail,
                    detail.len() / 2,
                    " [notification detail limited]",
                );
                continue;
            }
            notification.detail = None;
            continue;
        }
        if notification.summary.len() > 256 {
            notification.summary = crate::prompt_budget::truncate_prompt_text(
                &notification.summary,
                256,
                " [notification summary limited]",
            );
            continue;
        }
        if remove_largest_delivery_extra(&mut notification.extra) {
            continue;
        }
        return Err(AppError::Internal(
            "background notification exceeds the notice byte limit".into(),
        ));
    }
}

fn remove_largest_delivery_extra(extra: &mut std::collections::BTreeMap<String, Value>) -> bool {
    let Some(key) = extra
        .iter()
        .max_by_key(|(_, value)| serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()))
        .map(|(key, _)| key.clone())
    else {
        return false;
    };
    extra.remove(&key);
    true
}

#[cfg(test)]
mod delivery_text_tests {
    use super::*;
    use agena_runtime_contracts::part_content::{DecodedNotice, decode_system_notification_notice};

    fn decode(text: &str) -> agena_runtime_contracts::part_content::SystemNotificationContent {
        decode_system_notification_notice(text)
            .expect("delivery must be a typed notification")
            .expect("delivery marker must be valid JSON")
    }

    #[test]
    fn shell_and_monitor_completion_keep_their_operation_identity() {
        for kind in ["shell", "monitor"] {
            let text = delivery_text(&serde_json::json!({
                "kind": kind,
                "text": "Command finished (exit code 0)\nOutput resource: res_1",
                "process_id": "proc_1",
                "status": "completed",
                "event_seq": 7,
                "message_id": 42,
            }))
            .unwrap();
            let notification = decode(&text);
            assert_eq!(notification.operation_kind, kind);
            assert_eq!(notification.operation_id, "proc_1");
            assert_eq!(notification.status, "completed");
            assert_eq!(notification.event_seq, Some(7));
            assert_eq!(notification.summary, "Command finished (exit code 0)");
            assert!(notification.body.contains("Output resource: res_1"));
            assert!(!notification.extra.contains_key("message_id"));
        }
    }

    #[test]
    fn scheduler_and_agent_results_use_typed_operation_kinds() {
        let scheduler = delivery_text(&serde_json::json!({
            "kind": "scheduler",
            "job_id": "job_1",
            "delivery_key": "once_1",
            "text": "Continue the plan.",
        }))
        .unwrap();
        let scheduler = decode(&scheduler);
        assert_eq!(scheduler.operation_kind, "scheduled_delivery");
        assert_eq!(scheduler.operation_id, "once_1");
        assert_eq!(scheduler.body, "Continue the plan.");
        assert_eq!(scheduler.extra["job_id"], "job_1");

        let agent = delivery_text(&serde_json::json!({
            "kind": "agent_result",
            "agent_id": 91,
            "name": "explore",
            "run_id": "run_1",
            "status": "failed",
            "text": "Agent explore (91) failed:\nboom",
            "failure": {"code": "provider.failed"},
        }))
        .unwrap();
        let agent = decode(&agent);
        assert_eq!(agent.operation_kind, "task");
        assert_eq!(agent.operation_id, "91");
        assert_eq!(agent.status, "failed");
        assert_eq!(agent.extra["run_id"], "run_1");
        assert_eq!(agent.extra["failure"]["code"], "provider.failed");
    }

    #[test]
    fn assistant_and_integration_notices_keep_their_typed_identities() {
        for (kind, operation) in [
            ("assistant_notice", "assistant"),
            ("integration_notice", "integration"),
        ] {
            let text = delivery_text(&serde_json::json!({
                "kind": kind, "notice_id":"contact-42", "origin_message_id":12,
                "text":"Scheduled contact data"
            }))
            .unwrap();
            let notification = decode(&text);
            assert_eq!(notification.operation_kind, operation);
            assert_eq!(notification.operation_id, "contact-42");
            assert_eq!(notification.body, "Scheduled contact data");
            assert_eq!(notification.extra["origin_message_id"], 12);
        }
    }

    #[test]
    fn ordinary_messages_stay_plain_and_existing_markers_pass_through() {
        let plain = delivery_text(&serde_json::json!({
            "kind": "agent_message",
            "text": "Message from parent (1):\nhello",
        }))
        .unwrap();
        assert_eq!(plain, "Message from parent (1):\nhello");
        assert!(!matches!(
            agena_runtime_contracts::part_content::decode_notice(&plain),
            DecodedNotice::SystemNotification(_)
        ));

        let marker = agena_runtime_contracts::part_content::system_notification_notice_text(
            &agena_runtime_contracts::part_content::SystemNotificationContent {
                operation_id: "proc_1".into(),
                operation_kind: "shell".into(),
                status: "event".into(),
                summary: "line".into(),
                body: "line".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            delivery_text(&serde_json::json!({
                "kind": "hook_observation",
                "text": marker,
                "wake": false,
            }))
            .unwrap(),
            marker
        );
    }

    #[test]
    fn explicit_background_kind_wins_over_marker_shaped_output() {
        let hook_marker = agena_runtime_contracts::part_content::hook_notice_text(
            &agena_runtime_contracts::part_content::HookContent {
                hook: "agent.stop".into(),
                summary: "inside command output".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let text = delivery_text(&serde_json::json!({
            "kind": "shell",
            "text": hook_marker,
            "process_id": "proc_1",
        }))
        .unwrap();
        let notification = decode(&text);
        assert_eq!(notification.operation_kind, "shell");
        assert_eq!(notification.body, hook_marker);
    }

    #[test]
    fn late_hook_delivery_fits_the_storage_payload_budget() {
        let run = agena_plugin_host::HookRunRecord::new(
            "agent.stop",
            "test.stop-hook-probe",
            Some(7),
            agena_plugin_host::HookRunStatus::Applied,
            "agent.stop hook blocked stop",
            None,
        )
        .with_message(Some("\"quoted\" step\n".repeat(20_000)));
        let message_id = 42;
        let text = hook_delivery_text(&run, message_id).unwrap();
        let payload = serde_json::json!({
            "kind": "hook_observation",
            "message_id": message_id,
            "text": text,
            "wake": false,
        });
        assert!(serde_json::to_vec(&payload).unwrap().len() <= DELIVERY_PAYLOAD_MAX_BYTES);
        let hook = agena_runtime_contracts::part_content::decode_hook_notice(
            payload["text"].as_str().unwrap(),
        )
        .expect("late delivery must carry a hook marker")
        .expect("late hook marker must decode");
        assert_eq!(hook.hook, "agent.stop");
        assert!(
            hook.message
                .as_deref()
                .is_some_and(|message| { message.contains("[hook activity limited]") })
        );
    }

    #[test]
    fn oversized_notification_is_shrunk_without_corrupting_the_marker() {
        let body = "\"quoted\"\n".repeat(20_000);
        let text = delivery_text(&serde_json::json!({
            "kind": "monitor",
            "text": body,
            "process_id": "proc_1",
            "status": "event",
        }))
        .unwrap();
        assert!(text.len() <= DELIVERY_NOTICE_MAX_BYTES);
        let notification = decode(&text);
        assert_eq!(notification.operation_id, "proc_1");
        assert!(notification.body.contains("[notification body limited]"));
    }
}
