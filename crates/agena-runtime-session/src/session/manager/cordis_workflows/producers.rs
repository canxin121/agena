use super::*;
use agena_domain::CommandOutputStream;
use agena_plugin_host::sdk::host_api::{
    CordisJobControl, CordisJobInvocation, CordisJobOutcome, CordisJobProducer, CordisJobSpec, CordisJobStart,
    CordisJobStarter, CordisJobStatus, CordisJobUpdate,
};
use agena_storage::kernel::LogOutput;

const CANCEL_GRACE: Duration = Duration::from_secs(3);
const MAX_LABEL_BYTES: usize = 512;
const MAX_KIND_BYTES: usize = 48;

enum ProducerOrigin {
    Tool(ToolHandle, EntryId),
    Command(PluginCommandActivityOrigin),
}

impl SessionManager {
    pub async fn start_cordis_job(
        &self,
        session: i64,
        invocation: CordisJobInvocation,
        plugin: PluginKey,
        plugin_stop: CancellationToken,
        start: CordisJobStart,
        produce: CordisJobStarter,
        invocation_stop: CancellationToken,
    ) -> agena_plugin_host::sdk::Result<String> {
        validate_spec(&start.spec)?;
        let session = session_id(session).map_err(workflow_error)?;
        let call = match &invocation {
            CordisJobInvocation::Tool(call) => Some(*call),
            CordisJobInvocation::Command(_) => None,
        };
        let (executor, context) = self
            .host_callback_executor(Some(session.get()), call)
            .await
            .map_err(workflow_error)?;
        let Some(context) = context.filter(|context| {
            !context.global
                && context.owner.is_none()
                && context.channel_authority.is_none()
                && executor.plugin_is_enabled(&plugin)
        }) else {
            return Err(PluginError::invalid_params(
                "Cordis job producers require an enabled plugin in a project session",
            ));
        };
        let (origin, call_stop, child_job) = match invocation {
            CordisJobInvocation::Tool(call) => {
                let (tool, control) = self.live_tool(session.get(), call).map_err(workflow_error)?;
                let child = control.cordis_child_jobs.admit(context.relation_kind.is_subagent())
                    .map_err(workflow_error)?;
                let message = self.kernel().reader.execution(session, control.execution)
                    .await.map_err(kernel_error).map_err(workflow_error)?.message_id;
                let stop = child.as_ref().map_or_else(|| control.cancel.clone(), |child| child.cancel.clone());
                (ProducerOrigin::Tool(tool, message), stop, child)
            }
            CordisJobInvocation::Command(command) => (
                ProducerOrigin::Command(PluginCommandActivityOrigin {
                    invocation: command.id,
                    plugin_id: plugin.to_string(),
                    command_id: command.command_id,
                }),
                CancellationToken::new(),
                None,
            ),
        };
        let dispatch = self.session_dispatch(session);
        let admission = async {
            let submission = dispatch.submission.clone().lock_owned().await;
            if matches!(&origin, ProducerOrigin::Command(_)) {
                self.require_writable_bot_history(session.get()).await.map_err(workflow_error)?;
            }
            // Count reservations and registrations under the same session
            // lock used by stop, clear, and delete.
            let owned = self
                .cordis_workflows
                .running
                .lock()
                .values()
                .filter(|job| job.activity.session_id() == session && job.plugin == plugin)
                .count();
            if owned >= start.max_concurrent_per_owner {
                return Err(PluginError::invalid_params(format!(
                    "Cordis background job limit reached for this owner (maximum {})",
                    start.max_concurrent_per_owner
                )));
            }
            let slot = self
                .cordis_workflow_slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| PluginError::invalid_params("Agena Cordis job limit reached"))?;
            let uuid = uuid::Uuid::new_v4().simple().to_string();
            let id = format!("{}-{}", start.spec.kind, &uuid[..16]);
            let producer = produce(id.clone()).await?;
            Ok::<_, PluginError>((submission, slot, id, producer))
        };
        let (submission, slot, id, mut producer) = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => return Err(workflow_error(AppError::Cancelled)),
            _ = call_stop.cancelled() => return Err(workflow_error(AppError::Cancelled)),
            _ = plugin_stop.cancelled() => return Err(workflow_error(AppError::Cancelled)),
            _ = invocation_stop.cancelled() => return Err(workflow_error(AppError::Cancelled)),
            result = admission => result?,
        };
        let specification = BackgroundActivityStart {
            kind: BackgroundActivityKind::Process,
            label: format!(
                "{}{}:{}",
                lookup_prefix(&job_prefix(&plugin), &id),
                start.spec.output_limit_bytes.unwrap_or(0),
                start.spec.label
            ),
        };
        let manager = self.clone();
        let tracking = self.control_tasks.token();
        let (reply, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let _tracking = tracking;
            let _dispatch = dispatch;
            let (origin, activity, output) = match manager
                .open_producer_origin(session, origin, specification, start.output_retention)
                .await
            {
                Ok(opened) => opened,
                Err(error) => {
                    let _ = reply.send(Err(workflow_error(error)));
                    return;
                }
            };
            let (job, _registration) = manager.register_cordis_job(
                activity,
                plugin,
                slot,
                Some(producer.control.clone()),
                child_job,
            );
            let mut run = ProducerJob {
                manager,
                job: job.clone(),
                origin,
                id: id.clone(),
                output,
                log: None,
                text_bytes: 0,
                log_bytes: 0,
                output_total: 0,
                spill_paths: Arc::from([]),
            };
            // The JS starter is blocked on registration here. Commit its
            // staged appends before exposing the initial job projection.
            run.drain_output(&mut producer, false).await;
            run.publish_view(|job| registry::Event::Registered { job })
                .await;
            if call_stop.is_cancelled()
                || job.cancel.is_cancelled()
                || plugin_stop.is_cancelled()
                || invocation_stop.is_cancelled()
            {
                let _ = reply.send(Err(workflow_error(AppError::Cancelled)));
                job.request_stop();
            } else if reply.send(Ok(id.clone())).is_err() {
                job.request_stop();
            }
            drop(submission);
            run.supervise(start, producer, plugin_stop).await;
        });
        receiver
            .await
            .map_err(|_| PluginError::internal("Cordis job launch supervisor stopped"))?
    }

    async fn open_producer_origin(
        &self,
        session: SessionId,
        origin: ProducerOrigin,
        specification: BackgroundActivityStart,
        retention: ActivityOutputRetention,
    ) -> Result<(EntryId, ActivityHandle, TextOutput), AppError> {
        match origin {
            ProducerOrigin::Tool(tool, message) => {
                let (activity, output) = self.open_cordis_activity(&tool, specification, retention).await?;
                Ok((message, activity, output))
            }
            ProducerOrigin::Command(command) => {
                let request = RequestId::new();
                let (message, activity) = persist(|| {
                    self.kernel().sessions.begin_plugin_command_activity(
                        request, session, command.clone(), specification.clone(), retention,
                    )
                }).await.map_err(kernel_error)?.value;
                let output = self.open_cordis_capture(&activity).await?;
                Ok((message, activity, output))
            }
        }
    }
}

fn valid_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= MAX_KIND_BYTES
        && !kind.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub(super) fn lookup_prefix(prefix: &str, id: &str) -> String {
    format!("{prefix}{}:{id}:", id.len())
}

pub(super) fn kind_from_job_id(id: &str) -> Option<&str> {
    let (kind, suffix) = id.rsplit_once('-')?;
    (valid_kind(kind) && suffix.len() == 16 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(kind)
}

fn validate_spec(spec: &CordisJobSpec) -> agena_plugin_host::sdk::Result<()> {
    if !valid_kind(&spec.kind)
        || spec.label.trim().is_empty()
        || spec.label.len() > MAX_LABEL_BYTES
        || spec.label.chars().any(char::is_control)
        || spec.output_limit_bytes == Some(0)
    {
        return Err(PluginError::invalid_params(
            "Invalid Cordis JobSpec kind, label or output limit",
        ));
    }
    Ok(())
}

struct ProducerJob {
    manager: SessionManager,
    job: Arc<RunningJob>,
    origin: EntryId,
    id: String,
    output: TextOutput,
    log: Option<LogOutput>,
    text_bytes: usize,
    log_bytes: usize,
    output_total: u64,
    spill_paths: Arc<[String]>,
}

impl ProducerJob {
    async fn supervise(
        mut self,
        start: CordisJobStart,
        mut producer: CordisJobProducer,
        plugin_stop: CancellationToken,
    ) {
        let mut tearing_down = false;
        let mut output_open = true;
        let mut spill_paths_open = true;
        let grace = tokio::time::sleep(CANCEL_GRACE);
        tokio::pin!(grace);
        let outcome = loop {
            tokio::select! {
                biased;
                _ = async {
                    tokio::select! {
                        _ = plugin_stop.cancelled() => {}
                        _ = self.job.cancel.cancelled() => {}
                    }
                }, if !tearing_down => {
                    self.job.request_stop();
                    tearing_down = true;
                    grace.as_mut().reset(Instant::now() + CANCEL_GRACE);
                    (producer.control)(CordisJobControl::Teardown);
                    self.publish_view(|job| registry::Event::Stopping { job }).await;
                }
                _ = &mut grace, if tearing_down => {
                    (producer.control)(CordisJobControl::Abandon);
                    break Err("Cordis job producer did not settle after cancellation");
                }
                result = &mut producer.outcome => {
                    break result.map_err(|_| "Cordis job producer was released before settlement");
                }
                changed = producer.spill_paths.changed(), if spill_paths_open => {
                    if changed.is_err() {
                        spill_paths_open = false;
                    } else {
                        self.update_spill_paths(&mut producer).await;
                    }
                }
                update = producer.output.recv(), if output_open => {
                    match update {
                        Some(update) => {
                            self.update(update, &mut producer, true).await;
                        }
                        None => output_open = false,
                    }
                }
            }
        };
        // The terminal channel is independent of output backpressure. Its
        // sender ran after the last accepted append.
        self.drain_output(&mut producer, true).await;
        let (cause, reason) = self.job.prepare_settlement(&plugin_stop);
        let report_completion =
            start.report_completion && matches!(cause, registry::SettlementCause::Producer);
        let (status, outcome, terminal) = self.terminal(outcome, reason).await;
        let notice_gate = report_completion
            .then(|| {
                self.manager
                    .cordis_workflows
                    .read_gate(self.job.activity.id())
                    .try_lock_owned()
                    .ok()
            })
            .flatten();
        let notice = if !report_completion {
            None
        } else {
            Some(
                self.notice(&start, &terminal, status, notice_gate.is_some())
                    .await,
            )
        };
        let request = RequestId::new();
        if let Err(error) = persist(|| {
            self.job.activity.end_with_output(
                request,
                outcome.clone(),
                notice.clone(),
                Some(terminal.clone()),
            )
        })
        .await
        {
            tracing::error!(%error, job_id = %self.id, "Cordis job settlement failed");
        } else {
            self.manager.remember_job_settlement(&self.job, cause).await;
        }
        drop(notice_gate);
    }

    async fn drain_output(&mut self, producer: &mut CordisJobProducer, announce: bool) {
        while let Ok(update) = producer.output.try_recv() {
            self.update(update, producer, announce).await;
        }
        if let Err(error) = self.flush_output(producer, announce).await {
            tracing::warn!(%error, job_id = %self.id, "Cordis producer output position could not be saved");
        }
    }

    fn publish(&self, event: registry::Event) {
        self.manager.publish_job_event(&self.job, event);
    }

    async fn publish_view(&self, event: impl FnOnce(Value) -> registry::Event) {
        self.manager.publish_job_view(&self.job, event).await;
    }

    async fn update(
        &mut self,
        update: CordisJobUpdate,
        producer: &mut CordisJobProducer,
        announce: bool,
    ) {
        match update {
            CordisJobUpdate::Progress { line } => {
                let line = clean(&line, 256);
                let line = (!line.is_empty()).then_some(line);
                let changed = {
                    let mut state = self.job.state.lock();
                    let changed = state.progress != line;
                    state.progress = line;
                    changed
                };
                if changed && announce {
                    self.publish_view(|job| registry::Event::Progress { job })
                        .await;
                }
            }
            CordisJobUpdate::Append {
                mut chunk,
                _admission,
            } => {
                self.update_spill_paths(producer).await;
                let total = chunk.at + chunk.text.len() as u64;
                if let Err(error) = self
                    .manager
                    .append_job_output(&self.job, total, std::slice::from_ref(&chunk))
                    .await
                {
                    tracing::warn!(%error, job_id = %self.id, "Cordis producer output packet could not be saved");
                } else {
                    self.output_total = total;
                    if announce {
                        self.publish_output(total);
                    }
                }
                if chunk.gap_before {
                    chunk.text.insert_str(0, output::SOURCE_GAP);
                }
                self.capture_transcript(chunk).await;
            }
            CordisJobUpdate::Flush(reply) => {
                let result = self.flush_output(producer, announce).await;
                let _ = reply.send(result.map_err(workflow_error));
            }
        }
    }

    async fn update_spill_paths(&mut self, producer: &mut CordisJobProducer) {
        if let Err(error) = self.save_spill_paths(producer).await {
            tracing::warn!(%error, job_id = %self.id, "Cordis producer spill paths could not be saved");
        }
    }

    async fn save_spill_paths(&mut self, producer: &mut CordisJobProducer) -> Result<(), AppError> {
        let paths = producer.spill_paths.borrow_and_update().clone();
        if paths == self.spill_paths {
            return Ok(());
        }
        let request = RequestId::new();
        persist(|| self.job.activity.set_output_spill_paths(request, &paths))
            .await
            .map_err(kernel_error)?;
        self.spill_paths = paths;
        Ok(())
    }

    async fn flush_output(
        &mut self,
        producer: &mut CordisJobProducer,
        announce: bool,
    ) -> Result<(), AppError> {
        // Snapshot the queue's byte watermark before any storage await.
        let total = producer.output_end.load(Ordering::Acquire);
        self.save_spill_paths(producer).await?;
        if total > self.output_total {
            self.manager
                .append_job_output(&self.job, total, &[])
                .await?;
            let _ = self.output.record_loss(
                total - self.output_total,
                "Cordis producer output was limited",
            );
            self.output_total = total;
            if announce {
                self.publish_output(total);
            }
        }
        Ok(())
    }

    fn publish_output(&self, total: u64) {
        self.publish(registry::Event::Output {
            id: self.id.clone(),
            owner: self.job.activity.session_id().get().to_string(),
            total,
        });
    }

    async fn capture_transcript(&mut self, chunk: ActivityOutputChunk) {
        let text = &chunk.text;
        if chunk.channel == Some(ActivityOutputChannel::Log) {
            let request = RequestId::new();
            if self.log.is_none() {
                self.log = self
                    .job
                    .activity
                    .begin_log(request)
                    .await
                    .ok()
                    .map(|r| r.value);
            }
            if let Some(log) = &self.log {
                let remaining = MAX_PROGRESS_BYTES.saturating_sub(self.log_bytes);
                if text.len() <= remaining
                    && log.try_append(CommandOutputStream::Stdout, text).is_ok()
                {
                    self.log_bytes += text.len();
                } else {
                    let _ = log.record_loss(
                        text.len() as u64,
                        "Cordis job log exceeded capture admission",
                    );
                }
            }
        } else {
            let remaining = MAX_PROGRESS_BYTES.saturating_sub(self.text_bytes);
            if text.len() <= remaining && self.output.append(text).await.is_ok() {
                self.text_bytes += text.len();
            } else {
                let _ = self.output.record_loss(
                    text.len() as u64,
                    "Cordis job output exceeded capture admission",
                );
            }
        }
    }

    async fn terminal(
        &self,
        outcome: Result<CordisJobOutcome, &str>,
        reason: Option<String>,
    ) -> (
        &'static str,
        BackgroundActivityOutcome,
        ActivityOutputTerminal,
    ) {
        let Ok(outcome) = outcome else {
            return (
                "interrupted",
                BackgroundActivityOutcome::Interrupted {
                    diagnostic: outcome.err().unwrap_or_default().to_owned(),
                },
                ActivityOutputTerminal {
                    detail: None,
                    result: None,
                },
            );
        };
        let mut detail = outcome.detail.as_deref().map(|s| clean(s, 1024));
        let result = outcome
            .result
            .as_deref()
            .map(|text| bounded(text, 64 * 1024).to_owned());
        let preview = result
            .as_deref()
            .map(|text| clean(text, OUTPUT_READ_BYTES as usize));
        let (status, outcome) = match outcome.status {
            CordisJobStatus::Completed => (
                "completed",
                BackgroundActivityOutcome::Succeeded {
                    value: json!({ "detail": detail, "producer_result": preview }),
                },
            ),
            CordisJobStatus::Failed | CordisJobStatus::Killed => {
                let mut final_output = preview.unwrap_or_default();
                if matches!(outcome.status, CordisJobStatus::Killed) {
                    for item in detail.iter().chain(reason.iter()) {
                        if !final_output.is_empty() {
                            final_output.push('\n');
                        }
                        final_output.push_str(item);
                    }
                }
                if !final_output.is_empty() && self.output.append(&final_output).await.is_err() {
                    let _ = self.output.record_loss(
                        final_output.len() as u64,
                        "Cordis job final output could not be captured",
                    );
                }
                if matches!(outcome.status, CordisJobStatus::Killed) {
                    if let Some(reason) = reason {
                        detail = Some(match detail {
                            Some(detail) => format!("{detail}; {reason}"),
                            None => reason,
                        });
                    }
                    ("killed", BackgroundActivityOutcome::Stopped)
                } else {
                    (
                        "failed",
                        BackgroundActivityOutcome::Failed {
                            diagnostic: detail
                                .clone()
                                .unwrap_or_else(|| "Cordis job producer failed".into()),
                        },
                    )
                }
            }
        };
        (status, outcome, ActivityOutputTerminal { detail, result })
    }

    async fn notice(
        &self,
        start: &CordisJobStart,
        terminal: &ActivityOutputTerminal,
        status: &str,
        inline: bool,
    ) -> BackgroundActivityNotice {
        let mut text = format!("Cordis job {} {status} ({}).", start.spec.label, self.id);
        let limit = start
            .inline_output_bytes
            .min(8192)
            .min(start.spec.output_limit_bytes.unwrap_or(usize::MAX));
        let result = terminal.result.as_deref().unwrap_or_default();
        let mut output_position = None;
        if inline
            && let Some(room) = limit.checked_sub(text.len().saturating_add(result.len() + 2))
            && let Some((progress, position)) =
                self.manager.inline_job_output(&self.job, room, false).await
        {
            if !progress.is_empty() {
                text.push('\n');
                text.push_str(&progress);
            }
            if !result.is_empty() {
                text.push('\n');
                text.push_str(result);
            }
            output_position = Some(position);
        }
        if output_position.is_none() {
            text.push_str(&format!(
                " Read the result with job_output for {}.",
                self.id
            ));
            // The notice must still identify the job when the inline budget is
            // too small to carry both its identity and its output.
            if text.len() > limit {
                text = format!("Cordis job {} {status}.", self.id);
            }
        }
        BackgroundActivityNotice {
            text,
            origin_message_id: self.origin,
            wake: start.wake_on_completion,
            wake_budget: start
                .max_consecutive_wakes
                .map(|limit| BackgroundActivityWakeBudget {
                    source: self.job.plugin.to_string(),
                    limit,
                }),
            output_position,
        }
    }
}

pub(super) fn producer_result(outcome: &BackgroundActivityOutcome) -> Option<&str> {
    match outcome {
        BackgroundActivityOutcome::Succeeded { value } => value.get("producer_result")?.as_str(),
        _ => None,
    }
}

fn clean(text: &str, limit: usize) -> String {
    bounded(text, limit)
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
