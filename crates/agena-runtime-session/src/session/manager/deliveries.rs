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
            let Some(delivery) = pending.first() else {
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
            let options =
                match self.options_for_selection(&metadata.selection.clone().unwrap_or_default()) {
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
            self.supervise(execution, options, false, permit);
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
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 128)
            .await
            .map_err(kernel_error)?;
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
                .contains_key(&execution.id)
            {
                continue;
            }
            let options = self
                .options_for_selection(&snapshot.metadata.selection.clone().unwrap_or_default())?;
            let permit = self.reserve_execution()?;
            self.supervise(execution, options, false, permit);
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
        let user = self
            .kernel()
            .reader
            .entry(session, last.message_id)
            .await
            .map_err(kernel_error)?;
        Ok(
            (user.origin_session_id == session
                && matches!(user.payload, EntryPayload::User { .. }))
            .then_some(user.id),
        )
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
        if entry.origin_session_id != session || !matches!(entry.payload, EntryPayload::User { .. })
        {
            return Err(AppError::Config(
                "background delivery does not own its user message".into(),
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
        for chunk in runs.chunks(12) {
            let mut text = String::from("Plugin hook observations:\n");
            for run in chunk {
                let line = format!(
                    "{} {} {:?}: {}{}\n",
                    run.plugin_id,
                    run.hook,
                    run.status,
                    run.summary,
                    run.detail
                        .as_deref()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                );
                text.push_str(&crate::prompt_budget::truncate_prompt_text(
                    &line,
                    4096,
                    " [hook diagnostic limited]",
                ));
            }
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
fn delivery_text(payload: &Value) -> Result<String, AppError> {
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| payload.get("summary").and_then(Value::as_str));
    let value = match text {
        Some(text) => text.to_owned(),
        None => serde_json::to_string(payload)?,
    };
    Ok(crate::prompt_budget::truncate_prompt_text(
        &value,
        64 * 1024,
        "\n[Notification preview limited; its complete business payload remains recorded.]",
    ))
}
