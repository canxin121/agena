//! Session-owned jobs launched by imported Cordis tools.

mod output;
mod producers;
mod registry;

use super::{agents::WorkflowOwner, execution::permission_key, host::HOST_POLICY_READS, *};
use agena_domain::{
    ActivityId, ActivityOutputChannel, ActivityOutputChunk, PermissionDecision,
    PermissionResolution, StructuredObject, ToolInvocation,
};
use agena_plugin_host::{
    host::ImportedToolDecision,
    sdk::{
        PluginError, PluginErrorKind, PluginKey, ToolInvokeOutput,
        host_api::{
            AgentSpawnRequest, CordisJobControlSender, CordisWorkflowJobHost,
            CordisWorkflowJobRequest, CordisWorkflowResult, CordisWorkflowRunner,
            CordisWorkflowSettlement, CordisWorkflowStart, CordisWorkflowStarted,
            CordisWorkflowStopReason,
        },
    },
};
use agena_storage::kernel::{
    ActivityOutputHead, ActivityOutputPage, ActivityOutputRetention, ActivityOutputTerminal,
};
use futures_util::FutureExt;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    panic::AssertUnwindSafe,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    sync::{Mutex as AsyncMutex, OwnedMutexGuard, mpsc, oneshot},
    task::JoinSet,
    time::Instant,
};
use tokio_util::task::{TaskTracker, task_tracker::TaskTrackerToken};

const AGENT_TOOL: &str = "agena.agent.spawn";
const MAX_JOBS: usize = 16;
const JOB_LIST_LIMIT: usize = 32;
const MAX_ACTIVITY_READS_PER_CALL: usize = 16;
const MAX_PROGRESS_BYTES: usize = 128 * 1024;
const OUTPUT_READ_BYTES: u64 = 24 * 1024;
const JOB_LABEL_PREFIX: &str = "Cordis workflow ";

#[derive(Clone, Default)]
pub(super) struct CordisWorkflows {
    running: Arc<parking_lot::Mutex<HashMap<ActivityId, Arc<RunningJob>>>>,
    read_gates: Arc<parking_lot::Mutex<HashMap<ActivityId, std::sync::Weak<AsyncMutex<()>>>>>,
    events: registry::JobEvents,
}

impl CordisWorkflows {
    pub(super) fn is_running(&self, id: ActivityId) -> bool {
        self.running.lock().contains_key(&id)
    }

    fn read_gate(&self, activity: ActivityId) -> Arc<AsyncMutex<()>> {
        let mut gates = self.read_gates.lock();
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(&activity).and_then(std::sync::Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(AsyncMutex::new(()));
        gates.insert(activity, Arc::downgrade(&gate));
        gate
    }
}

#[derive(Default)]
pub(super) struct CordisToolReads {
    sealed: bool,
    claims: Vec<CordisReadClaim>,
}

pub(super) struct CordisReadClaim {
    pub(super) read: ActivityOutputRead,
    _gate: OwnedMutexGuard<()>,
}

impl CordisToolReads {
    pub(super) fn seal(&mut self) -> Vec<CordisReadClaim> {
        self.sealed = true;
        std::mem::take(&mut self.claims)
    }
}

struct RunningJob {
    activity: ActivityHandle,
    plugin: PluginKey,
    producer: Option<CordisJobControlSender>,
    cancel: CancellationToken,
    state: parking_lot::Mutex<JobState>,
    done: Notify,
    settled: AtomicBool,
}

/// Detached jobs owned by one attempt of a delegated agent.
#[derive(Default)]
pub(super) struct ChildCordisJobs {
    cancel: CancellationToken,
    tasks: parking_lot::Mutex<TaskTracker>,
}

struct ChildJobLease {
    cancel: CancellationToken,
    _task: TaskTrackerToken,
}

impl ChildCordisJobs {
    pub(super) fn new(cancel: CancellationToken) -> Self {
        Self {
            cancel,
            tasks: Default::default(),
        }
    }

    fn admit(&self, child: bool) -> Result<Option<ChildJobLease>, AppError> {
        if !child {
            return Ok(None);
        }
        let tasks = self.tasks.lock();
        if tasks.is_closed() || self.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        Ok(Some(ChildJobLease {
            cancel: self.cancel.child_token(),
            _task: tasks.token(),
        }))
    }

    pub(super) async fn finish(&self) {
        let tasks = {
            let tasks = self.tasks.lock();
            tasks.close();
            tasks.clone()
        };
        self.cancel.cancel();
        tasks.wait().await;
    }
}

#[derive(Default)]
struct JobState {
    stop: Option<Stop>,
    kill_reason: Option<String>,
    settling: bool,
    progress: Option<String>,
    waiters: usize,
    registry_settlement: Option<(Value, registry::SettlementCause)>,
}

enum Stop {
    Kill,
    Session,
}

impl RunningJob {
    fn request_stop(&self) {
        let mut state = self.state.lock();
        if !state.settling {
            state.stop = Some(Stop::Session);
            self.cancel.cancel();
        }
    }

    fn commit_kill(&self, reason: Option<String>) -> bool {
        let mut state = self.state.lock();
        if state.settling || matches!(state.stop, Some(Stop::Session)) {
            return false;
        }
        state.stop = Some(Stop::Kill);
        if let Some(mut reason) = reason {
            results_text::truncate(&mut reason, 1024);
            state.kill_reason = Some(reason);
        }
        if self.producer.is_none() {
            self.cancel.cancel();
        }
        true
    }

    fn is_stopping(&self) -> bool {
        self.cancel.is_cancelled() || self.state.lock().stop.is_some()
    }

    async fn wait(&self) {
        loop {
            let notified = self.done.notified();
            if self.settled.load(Ordering::Acquire) {
                // Synchronize with the lifecycle broadcasts before a caller
                // can remove the settled record.
                drop(self.state.lock());
                return;
            }
            notified.await;
        }
    }

    fn prepare_settlement(
        &self,
        plugin_stop: &CancellationToken,
    ) -> (registry::SettlementCause, Option<String>) {
        let mut state = self.state.lock();
        state.settling = true;
        let cause = match &state.stop {
            _ if plugin_stop.is_cancelled() => registry::SettlementCause::Teardown,
            Some(Stop::Session) => registry::SettlementCause::Teardown,
            Some(Stop::Kill) => registry::SettlementCause::Kill,
            None if self.cancel.is_cancelled() => registry::SettlementCause::Teardown,
            None => registry::SettlementCause::Producer,
        };
        (cause, state.kill_reason.clone())
    }
}

struct JobRegistration {
    jobs: CordisWorkflows,
    job: Arc<RunningJob>,
    // Hold the slot until the activity's terminal transaction finishes.
    _slot: OwnedSemaphorePermit,
    _child: Option<ChildJobLease>,
}

impl Drop for JobRegistration {
    fn drop(&mut self) {
        let mut state = self.job.state.lock();
        self.job.settled.store(true, Ordering::Release);
        self.job.done.notify_waiters();
        if let Some((job, cause)) = state.registry_settlement.take() {
            let output = registry::Event::Output {
                id: job["id"].as_str().unwrap_or_default().to_owned(),
                owner: self.job.activity.session_id().get().to_string(),
                total: job["output"]["total"].as_u64().unwrap_or_default(),
            };
            self.jobs.events.publish(
                self.job.activity.session_id(),
                &self.job.plugin,
                registry::Event::Settled {
                    job,
                    cause,
                    awaited: state.waiters > 0,
                },
            );
            self.jobs
                .events
                .publish(self.job.activity.session_id(), &self.job.plugin, output);
        }
        drop(state);
        self.jobs.running.lock().remove(&self.job.activity.id());
    }
}

type Grants = HashMap<String, PermissionResolution>;

struct JobHost {
    manager: SessionManager,
    plugin: PluginKey,
    owner: WorkflowOwner,
    job: Arc<RunningJob>,
    output: TextOutput,
    progress_capture: parking_lot::Mutex<WorkflowCapture>,
    observations: mpsc::Sender<WorkflowObservation>,
    grants: Grants,
    launches: TaskTracker,
    children: Arc<parking_lot::Mutex<BTreeSet<i64>>>,
}

#[derive(Default)]
struct WorkflowCapture {
    written: usize,
    total: u64,
}

struct WorkflowObservation {
    phase: Option<String>,
    chunk: ActivityOutputChunk,
}

impl SessionManager {
    pub async fn start_cordis_workflow(
        &self,
        session: i64,
        call: i64,
        plugin: PluginKey,
        plugin_stop: CancellationToken,
        start: CordisWorkflowStart,
        run: CordisWorkflowRunner,
        invocation_stop: CancellationToken,
    ) -> Result<CordisWorkflowStarted, AppError> {
        let session = session_id(session)?;
        let (tool, control) = self.live_tool(session.get(), call)?;
        let (executor, context) = self
            .host_callback_executor(Some(session.get()), Some(call))
            .await?;
        let Some(context) = context.filter(|context| {
            !context.global
                && context.owner.is_none()
                && context.channel_authority.is_none()
                && executor.plugin_is_enabled(&plugin)
        }) else {
            return Err(AppError::Config(
                "Cordis workflows require an enabled plugin in a project session".into(),
            ));
        };
        let child_job = control
            .cordis_child_jobs
            .admit(context.relation_kind.is_subagent())?;
        let grants = if child_job.is_some() {
            // The subagent can run the script. Its nested delegation policy
            // still applies to each agent() request.
            Grants::new()
        } else {
            self.cordis_workflow_delegation_grants(
                session,
                &tool,
                &control,
                &executor,
                &start.name,
                &invocation_stop,
            )
            .await?
        };
        let slot = self
            .cordis_workflow_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                AppError::Config(format!(
                    "Agena can run at most {MAX_JOBS} Cordis workflow jobs concurrently"
                ))
            })?;
        let origin = self
            .kernel()
            .reader
            .execution(session, control.execution)
            .await
            .map_err(kernel_error)?
            .message_id;
        let model = control.model.clone();
        let dispatch = self.session_dispatch(session);
        // Session stop and deletion acquire the same lock before looking for
        // live work. Keep the launch visible even if its caller drops the reply.
        let submission = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => return Err(AppError::Cancelled),
            _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
            _ = control.cordis_child_jobs.cancel.cancelled(), if child_job.is_some() => return Err(AppError::Cancelled),
            _ = plugin_stop.cancelled() => return Err(AppError::Cancelled),
            _ = invocation_stop.cancelled() => return Err(AppError::Cancelled),
            guard = dispatch.submission.clone().lock_owned() => guard,
        };
        let manager = self.clone();
        let tracking = self.control_tasks.token();
        let (reply, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let _tracking = tracking;
            // Keep the dispatch itself alive as long as its owned guard.
            let _dispatch = dispatch;
            let specification = BackgroundActivityStart {
                kind: BackgroundActivityKind::Agent,
                label: format!("{}{}:{}", job_prefix(&plugin), start.run_id, start.name),
            };
            let opened = manager
                .open_cordis_activity(&tool, specification, start.output_retention)
                .await;
            let (activity, output) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            };
            let (job, _registration) =
                manager.register_cordis_job(activity, plugin.clone(), slot, None, child_job);
            manager
                .publish_job_view(&job, |job| registry::Event::Registered { job })
                .await;
            let owner = WorkflowOwner {
                id: job.activity.id(),
                origin,
                model,
            };
            let (observations, updates) = mpsc::channel(registry::EVENT_CAPACITY);
            let host = Arc::new(JobHost {
                manager,
                plugin,
                owner,
                job: job.clone(),
                output,
                progress_capture: Default::default(),
                observations,
                grants,
                launches: TaskTracker::new(),
                children: Default::default(),
            });
            if plugin_stop.is_cancelled()
                || invocation_stop.is_cancelled()
                || control.cancel.is_cancelled()
                || job.cancel.is_cancelled()
            {
                let _ = reply.send(Err(AppError::Cancelled));
                job.request_stop();
            } else if reply
                .send(Ok(CordisWorkflowStarted {
                    job_id: job.activity.id(),
                    run_id: start.run_id,
                }))
                .is_err()
            {
                job.request_stop();
            }
            drop(submission);
            drop(control);
            host.run(start, run, plugin_stop, updates).await;
        });
        receiver
            .await
            .map_err(|_| AppError::Internal("Cordis workflow launch supervisor stopped".into()))?
    }

    async fn cordis_workflow_delegation_grants(
        &self,
        session: SessionId,
        tool: &ToolHandle,
        control: &Arc<RunControl>,
        executor: &ToolExecutor,
        name: &str,
        invocation_stop: &CancellationToken,
    ) -> Result<Grants, AppError> {
        let workspace = self
            .kernel()
            .reader
            .metadata(session)
            .await
            .map_err(kernel_error)?
            .workspace_id;
        let invocation = agent_invocation(&AgentSpawnRequest {
            name: "cordis_workflow".into(),
            prompt: format!("Delegate tasks for Cordis workflow {name}."),
            profile: None,
            commands: Vec::new(),
            selection: None,
            timeout_ms: None,
            max_tokens: None,
            max_cost_microusd: None,
            inherit_parent_context: false,
        })?;
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session.get()))
            .await
            .map_err(tool_error)?;
        let mut grants = Grants::new();
        for check in &checks {
            let key = permission_key(&check.action)?;
            let resolution = self
                .resolve_tool_permission(&key, &check.decision, session, workspace)
                .await
                .map_err(tool_error)?;
            if matches!(
                resolution.decision,
                PermissionDecision::Ask { .. } | PermissionDecision::Auto { .. }
            ) {
                grants.insert(key, resolution);
            }
        }
        tokio::select! {
            biased;
            _ = invocation_stop.cancelled() => return Err(AppError::Cancelled),
            result = self.authorize_checks(tool, checks, executor, control, &control.state) => result.map_err(tool_error)?,
        }
        Ok(grants)
    }

    fn register_cordis_job(
        &self,
        activity: ActivityHandle,
        plugin: PluginKey,
        slot: OwnedSemaphorePermit,
        producer: Option<CordisJobControlSender>,
        child: Option<ChildJobLease>,
    ) -> (Arc<RunningJob>, JobRegistration) {
        let job = Arc::new(RunningJob {
            activity,
            plugin,
            producer,
            cancel: child
                .as_ref()
                .map_or_else(|| self.shutdown.child_token(), |lease| lease.cancel.clone()),
            state: Default::default(),
            done: Notify::new(),
            settled: AtomicBool::new(false),
        });
        self.cordis_workflows
            .running
            .lock()
            .insert(job.activity.id(), job.clone());
        let registration = JobRegistration {
            jobs: self.cordis_workflows.clone(),
            job: job.clone(),
            _slot: slot,
            _child: child,
        };
        (job, registration)
    }

    async fn open_cordis_activity(
        &self,
        tool: &ToolHandle,
        specification: BackgroundActivityStart,
        output_retention: ActivityOutputRetention,
    ) -> Result<(ActivityHandle, TextOutput), AppError> {
        let request = RequestId::new();
        let activity = persist(|| {
            tool.begin_activity_with_output(request, specification.clone(), Some(output_retention))
        })
        .await
        .map_err(kernel_error)?
        .value;
        let output = self.open_cordis_capture(&activity).await?;
        Ok((activity, output))
    }

    async fn open_cordis_capture(&self, activity: &ActivityHandle) -> Result<TextOutput, AppError> {
        let request = RequestId::new();
        match persist(|| activity.begin_text(request)).await {
            Ok(output) => Ok(output.value),
            Err(error) => {
                let request = RequestId::new();
                persist(|| {
                    activity.end(
                        request,
                        BackgroundActivityOutcome::Failed {
                            diagnostic: "Could not open the Cordis job output stream".into(),
                        },
                    )
                })
                .await
                .map_err(kernel_error)?;
                Err(kernel_error(error))
            }
        }
    }

    pub async fn cordis_workflow_job(
        &self,
        session: i64,
        call: i64,
        plugin: PluginKey,
        request: CordisWorkflowJobRequest,
        cancel: CancellationToken,
    ) -> Result<Value, AppError> {
        let session = session_id(session)?;
        let (_, control) = self.live_tool(session.get(), call)?;
        let (executor, context) = self
            .host_callback_executor(Some(session.get()), Some(call))
            .await?;
        if context.is_none_or(|context| {
            context.global || context.owner.is_some() || context.channel_authority.is_some()
        }) || !executor.plugin_is_enabled(&plugin)
        {
            return Err(AppError::Config(
                "Cordis jobs require an enabled plugin in a project session".into(),
            ));
        }
        let prefix = job_prefix(&plugin);
        match request {
            CordisWorkflowJobRequest::List => {
                let activities = read_with_busy_retry(session, "cordis_job_list", || {
                    self.kernel().reader.session_activities_by_label(
                        session,
                        &prefix,
                        JOB_LIST_LIMIT,
                    )
                })
                .await?;
                let jobs = activities
                    .iter()
                    .filter_map(|activity| {
                        let label = job_label(activity, &prefix)?;
                        let running = self
                            .cordis_workflows
                            .running
                            .lock()
                            .get(&activity.id)
                            .cloned();
                        Some(public_job(activity, label, running.as_deref()))
                    })
                    .collect::<Vec<_>>();
                Ok(json!(jobs))
            }
            CordisWorkflowJobRequest::Output { job_id, wait_ms } => {
                let activity = self.find_cordis_activity(session, &job_id, &prefix).await?;
                let job_id = activity.id;
                let running = self.cordis_workflows.running.lock().get(&job_id).cloned();
                let steered = if let (None, Some(wait_ms), Some(running)) =
                    (&activity.outcome, wait_ms, running)
                {
                    drop(activity);
                    self.with_agent_wait_permit(
                        session.get(),
                        Some(call),
                        Some(&control),
                        self.wait_for_cordis_job(session, &control, &running, &cancel, wait_ms),
                    )
                    .await?
                } else {
                    false
                };
                self.read_cordis_job(session, call, &control, &cancel, &prefix, job_id, steered)
                    .await
            }
            CordisWorkflowJobRequest::Kill { job_id, reason } => {
                let (activity, running) = tokio::select! {
                    biased;
                    _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                    result = self.kill_cordis_job(session, &prefix, &job_id, reason, &cancel) => result?,
                };
                let label = job_label(&activity, &prefix).expect("owned Cordis activity");
                let outcome = if running.is_some() {
                    "cancellation-requested"
                } else {
                    "already-finished"
                };
                Ok(
                    json!({"outcome": outcome, "job": public_job(&activity, label, running.as_deref())}),
                )
            }
        }
    }

    async fn find_cordis_activity(
        &self,
        session: SessionId,
        id: &str,
        prefix: &str,
    ) -> Result<ReadView<BackgroundActivityView>, AppError> {
        if let Ok(native) = id.parse() {
            let activity = self.owned_cordis_activity(session, native, prefix).await?;
            if job_label(&activity, prefix).is_some_and(|label| label.external_id.is_none()) {
                return Ok(activity);
            }
            return Err(unknown_job());
        }
        producers::kind_from_job_id(id).ok_or_else(unknown_job)?;
        let search_prefix = producers::lookup_prefix(prefix, id);
        let ids = read_with_busy_retry(session, "cordis_job_id", || {
            self.kernel()
                .reader
                .session_activities_by_label(session, &search_prefix, 1)
        })
        .await?;
        let activity = ids.first().ok_or_else(unknown_job)?;
        let activity = self
            .owned_cordis_activity(session, activity.id, prefix)
            .await?;
        if job_label(&activity, prefix).is_some_and(|label| label.external_id == Some(id)) {
            Ok(activity)
        } else {
            Err(unknown_job())
        }
    }

    async fn owned_cordis_activity(
        &self,
        session: SessionId,
        id: ActivityId,
        prefix: &str,
    ) -> Result<ReadView<BackgroundActivityView>, AppError> {
        let activity = read_with_busy_retry(session, "cordis_job", || async {
            match self.kernel().reader.listed_activity(session, id).await {
                Err(KernelError::NotFound(_) | KernelError::Ownership) => Ok(None),
                result => result.map(Some),
            }
        })
        .await?
        .ok_or_else(unknown_job)?;
        if activity.origin_session_id != session || job_label(&activity, prefix).is_none() {
            return Err(unknown_job());
        }
        Ok(activity)
    }

    async fn wait_for_cordis_job(
        &self,
        session: SessionId,
        control: &RunControl,
        job: &RunningJob,
        cancel: &CancellationToken,
        wait_ms: u64,
    ) -> Result<bool, AppError> {
        let Some(_wait) = registry::WaitLease::register(job) else {
            return Ok(false);
        };
        let reader = &self.kernel().reader;
        let mut watch = reader.watch(session, None).map_err(kernel_error)?;
        let deadline = Instant::now() + Duration::from_millis(wait_ms.clamp(1, 600_000));
        loop {
            let input = async {
                let page = read_with_busy_retry(session, "cordis_job_wait_queue", || {
                    reader.active_executions(session, None, 128)
                })
                .await?;
                for execution in page.executions.iter().filter(|execution| {
                    execution.id != control.execution
                        && execution.phase == ExecutionPhase::Queued
                        && execution.purpose == ExecutionPurpose::Message
                }) {
                    let entry = match reader.entry(session, execution.message_id).await {
                        // A queued message can be withdrawn while we inspect it.
                        Ok(entry) => entry,
                        Err(KernelError::NotFound(_)) => continue,
                        Err(error) => return Err(kernel_error(error)),
                    };
                    if matches!(&entry.payload, EntryPayload::User { author, .. }
                        if author.as_ref().is_none_or(|author| author.kind.is_instance_operator()
                            || author.kind.channel_role().is_some()))
                    {
                        return Ok(true);
                    }
                }
                Ok::<_, AppError>(false)
            };
            let steered = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                _ = job.wait() => return Ok(false),
                _ = tokio::time::sleep_until(deadline) => return Ok(false),
                input = input => input?,
            };
            if steered {
                return Ok(true);
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                _ = job.wait() => return Ok(false),
                _ = tokio::time::sleep_until(deadline) => return Ok(false),
                change = watch.recv() => {
                    if change.is_err() {
                        watch = reader.watch(session, None).map_err(kernel_error)?;
                    }
                }
            }
        }
    }

    async fn read_cordis_job(
        &self,
        session: SessionId,
        call: i64,
        control: &RunControl,
        cancel: &CancellationToken,
        prefix: &str,
        id: ActivityId,
        steered: bool,
    ) -> Result<Value, AppError> {
        let reads = self
            .tools
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&(session.get(), call))
            .map(|tool| tool.cordis_reads.clone())
            .ok_or(AppError::Cancelled)?;
        let mut reads = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AppError::Cancelled),
            _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
            guard = reads.lock() => guard,
        };
        if reads.sealed {
            return Err(AppError::Cancelled);
        }
        let existing = reads
            .claims
            .iter()
            .position(|claim| claim.read.activity == id);
        let gate = if existing.is_some() {
            None
        } else {
            if reads.claims.len() >= MAX_ACTIVITY_READS_PER_CALL {
                return Err(AppError::Config(
                    "Cordis tool activity read limit reached".into(),
                ));
            }
            let mutex = self.cordis_workflows.read_gate(id);
            Some(if reads.claims.is_empty() {
                self.with_agent_wait_permit(session.get(), Some(call), Some(control), async {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => Err(AppError::Cancelled),
                        _ = control.cancel.cancelled() => Err(AppError::Cancelled),
                        guard = mutex.lock_owned() => Ok(guard),
                    }
                })
                .await?
            } else {
                // A composite tool can touch multiple jobs. Contention on an
                // additional job cannot block the holder of a different job.
                mutex
                    .try_lock_owned()
                    .map_err(|_| AppError::PersistenceBusy)?
            })
        };
        let activity = self.owned_cordis_activity(session, id, prefix).await?;
        let previous = match existing {
            Some(index) => reads.claims[index].read.position,
            None => self.cordis_output_position(session, id).await?,
        };
        let label = job_label(&activity, prefix).expect("owned Cordis activity");
        let output_budget = label
            .output_limit
            .unwrap_or(OUTPUT_READ_BYTES as usize)
            .min(OUTPUT_READ_BYTES as usize);
        let from = previous.packet_offset.unwrap_or_default();
        let packets = self.packet_output(session, id, Some(from)).await?;
        let (mut text, mut position) = if let Some(packets) = packets.as_ref() {
            let text = output::model_text(packets, from, label.external_id.is_none());
            (
                output::model_tail(text, output_budget),
                ActivityOutputPosition {
                    packet_offset: Some(packets.head.total),
                    ..previous
                },
            )
        } else {
            self.read_cordis_progress(session, &activity, previous, output_budget as u64)
                .await?
        };
        if activity.outcome.is_some() && !previous.outcome_read {
            position.outcome_read = true;
            if let Some(result) = output::terminal_result(&activity, label, packets.as_ref()) {
                let result = if label.external_id.is_some() {
                    bounded(&result, output_budget)
                } else {
                    &result
                };
                if label.external_id.is_some() && !text.is_empty() && !result.is_empty() {
                    results_text::truncate(
                        &mut text,
                        output_budget.saturating_sub(result.len() + 1),
                    );
                }
                if !text.is_empty() && !result.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(result);
            }
        }
        if position != previous {
            if let Some(index) = existing {
                reads.claims[index].read.position = position;
            } else {
                reads.claims.push(CordisReadClaim {
                    read: ActivityOutputRead {
                        activity: id,
                        position,
                    },
                    _gate: gate.expect("new activity read acquired its gate"),
                });
            }
        }
        let running = self.cordis_workflows.running.lock().get(&id).cloned();
        let mut job = public_job(&activity, label, running.as_deref());
        if activity.outcome.is_some()
            && let Some(detail) = packets
                .as_ref()
                .and_then(|packets| packets.terminal.as_ref()?.detail.as_deref())
        {
            job["detail"] = detail.into();
        }
        if steered && activity.outcome.is_none() {
            job["detail"] = match job.get("detail").and_then(Value::as_str) {
                Some(detail) => format!("{detail}, user steered"),
                None => "user steered".into(),
            }
            .into();
        }
        if label.external_id.is_some() {
            results_text::truncate(&mut text, output_budget);
        }
        Ok(json!({"text": text, "job": job}))
    }

    async fn cordis_output_position(
        &self,
        session: SessionId,
        id: ActivityId,
    ) -> Result<ActivityOutputPosition, AppError> {
        read_with_busy_retry(session, "cordis_job_output_position", || {
            self.kernel().reader.activity_output_position(session, id)
        })
        .await
    }

    async fn read_cordis_progress(
        &self,
        session: SessionId,
        activity: &BackgroundActivityView,
        mut position: ActivityOutputPosition,
        max_bytes: u64,
    ) -> Result<(String, ActivityOutputPosition), AppError> {
        let tool = read_with_busy_retry(session, "cordis_job_output", || {
            self.kernel().reader.tool(session, activity.launch_tool)
        })
        .await?;
        let Some(stream) = tool.outputs.iter().find(|stream| {
            stream.activity_id == Some(activity.id)
                && stream.kind == agena_domain::ContentKind::Text
        }) else {
            return Ok((String::new(), position));
        };
        let reader = &self.kernel().reader;
        let watch = read_with_busy_retry(session, "cordis_job_output_head", || {
            reader.watch_content(session, stream.id)
        })
        .await?;
        let head = watch.head().map_err(kernel_error)?;
        // Recovery fences writers by rotating the epoch of open streams.
        // The content identity and its durable logical offsets survive it.
        let (read, seen_loss) = match position
            .cursor
            .as_mut()
            .filter(|cursor| cursor.content == head.id)
        {
            Some(cursor) => {
                cursor.epoch = head.epoch;
                (cursor.offset, position.lost_bytes)
            }
            None => (0, 0),
        };
        let start = read
            .max(head.retained_start)
            .max(head.committed.saturating_sub(max_bytes));
        let lost_bytes = head.capture_loss.committed_bytes;
        let lost = lost_bytes.saturating_sub(seen_loss);
        if read == head.committed && lost == 0 {
            return Ok((String::new(), position));
        }
        let mut text = String::new();
        let omitted = start.saturating_sub(read).saturating_add(lost);
        if omitted > 0 {
            text.push_str(&format!("[{omitted} bytes of earlier progress omitted]\n"));
        }
        let mut cursor = TextCursor {
            content: stream.id,
            epoch: stream.epoch,
            offset: start,
        };
        if start < head.committed {
            let range = read_with_busy_retry(session, "cordis_job_output_range", || {
                reader.text_range(
                    session,
                    stream.id,
                    Some(cursor),
                    (head.committed - start) as usize,
                )
            })
            .await?;
            let first = range
                .bytes
                .iter()
                .position(|byte| byte & 0xc0 != 0x80)
                .unwrap_or(range.bytes.len());
            text.push_str(&String::from_utf8_lossy(&range.bytes[first..]));
            cursor = range.next;
        }
        position.cursor = Some(cursor);
        position.lost_bytes = lost_bytes;
        Ok((text, position))
    }

    pub(super) async fn stop_cordis_workflows(&self, session: SessionId) -> bool {
        let jobs = self
            .cordis_workflows
            .running
            .lock()
            .values()
            .filter(|job| job.activity.session_id() == session)
            .cloned()
            .collect::<Vec<_>>();
        for job in &jobs {
            job.request_stop();
        }
        for job in &jobs {
            job.wait().await;
        }
        !jobs.is_empty()
    }

    pub(super) fn cordis_workflow_slots() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(MAX_JOBS))
    }
}

impl JobHost {
    async fn run(
        self: Arc<Self>,
        start: CordisWorkflowStart,
        run: CordisWorkflowRunner,
        plugin_stop: CancellationToken,
        mut observations: mpsc::Receiver<WorkflowObservation>,
    ) {
        let job = &self.job;
        self.progress("workflow/start");
        let result = AssertUnwindSafe(async {
            let execution = run(self.clone(), job.cancel.clone());
            tokio::pin!(execution);
            loop {
                tokio::select! {
                    biased;
                    _ = plugin_stop.cancelled() => {
                        job.request_stop();
                        break Err(PluginError::from_kind(PluginErrorKind::Cancelled, "plugin generation stopped"));
                    }
                    _ = job.cancel.cancelled() => {
                        break Err(PluginError::from_kind(PluginErrorKind::Cancelled, "workflow cancelled"));
                    }
                    Some(update) = observations.recv() => self.publish_observation(update).await,
                    result = &mut execution => break result,
                }
            }
        }).catch_unwind().await.unwrap_or_else(|_| {
            Err(PluginError::internal("Cordis workflow worker stopped unexpectedly"))
        });
        while let Ok(update) = observations.try_recv() {
            self.publish_observation(update).await;
        }
        let stopping = job.cancel.is_cancelled();
        if stopping {
            self.manager
                .publish_job_view(job, |job| registry::Event::Stopping { job })
                .await;
        }

        // An awaiting JS promise can disappear while a durable child launch is
        // preparing. Cancel the observed children only after these launches join.
        self.launches.close();
        self.launches.wait().await;
        let children = std::mem::take(&mut *self.children.lock());
        let mut cleanup = JoinSet::new();
        for child in children {
            let host = self.clone();
            cleanup.spawn(async move { host.stop_owned_child(child).await });
        }
        let mut failed_cleanup = false;
        while let Some(result) = cleanup.join_next().await {
            if let Err(error) = result
                .map_err(|error| AppError::internal_error(&error))
                .and_then(|result| result)
            {
                failed_cleanup = true;
                tracing::error!(%error, job_id = %job.activity.id(), "Cordis workflow child cleanup failed");
            }
        }
        if plugin_stop.is_cancelled() && !job.cancel.is_cancelled() {
            job.request_stop();
        }
        let (cause, reason) = job.prepare_settlement(&plugin_stop);
        let quiet = !matches!(cause, registry::SettlementCause::Producer);
        if quiet && !stopping {
            self.manager
                .publish_job_view(job, |job| registry::Event::Stopping { job })
                .await;
        }
        if let Some(reason) = reason.as_deref() {
            self.progress(&format!("workflow/cancel: {reason}"));
        }
        let outcome = if failed_cleanup {
            BackgroundActivityOutcome::Interrupted {
                diagnostic: "A Cordis workflow child could not be settled".into(),
            }
        } else if job.cancel.is_cancelled() || plugin_stop.is_cancelled() {
            BackgroundActivityOutcome::Stopped
        } else {
            match result {
                Err(error) => BackgroundActivityOutcome::Failed {
                    diagnostic: bounded(error.diagnostic_message(), 4096).to_owned(),
                },
                Ok(result) => {
                    let outcome = BackgroundActivityOutcome::Succeeded {
                        value: json!({
                            "result": result, "maxResultChars": start.max_result_chars,
                        }),
                    };
                    if serde_json::to_vec(&outcome).is_ok_and(|value| value.len() <= 64 * 1024) {
                        outcome
                    } else {
                        BackgroundActivityOutcome::Failed {
                            diagnostic: "Cordis workflow result exceeds the durable output limit"
                                .into(),
                        }
                    }
                }
            }
        };
        let (status, result) = match &outcome {
            BackgroundActivityOutcome::Succeeded { value } => ("completed", stored_result(value)),
            BackgroundActivityOutcome::Stopped => ("killed", None),
            BackgroundActivityOutcome::Interrupted { .. } => ("interrupted", None),
            BackgroundActivityOutcome::Failed { .. } => ("failed", None),
        };
        let id = job.activity.id();
        self.progress(&format!("workflow/end: {status}"));
        while let Ok(update) = observations.try_recv() {
            self.publish_observation(update).await;
        }
        let total = self.progress_capture.lock().total;
        if let Err(error) = self.manager.append_job_output(job, total, &[]).await {
            tracing::warn!(%error, %id, "Cordis workflow final output position could not be saved");
        }
        let report_completion = !quiet && start.report_completion;
        let notice_gate = if !report_completion {
            None
        } else {
            self.manager
                .cordis_workflows
                .read_gate(id)
                .try_lock_owned()
                .ok()
        };
        let notice = if !report_completion {
            None
        } else {
            Some(
                self.completion_notice(&start, status, result.as_ref(), notice_gate.is_some())
                    .await,
            )
        };
        let output_terminal = ActivityOutputTerminal {
            detail: matches!(&outcome, BackgroundActivityOutcome::Stopped)
                .then(|| reason.clone())
                .flatten(),
            result: None,
        };
        let mut settlement = match &outcome {
            BackgroundActivityOutcome::Succeeded { .. } => CordisWorkflowSettlement {
                reason: CordisWorkflowStopReason::Completed,
                error: None,
                agents_started: result.as_ref().map(|value| value.agents_started),
            },
            BackgroundActivityOutcome::Stopped => CordisWorkflowSettlement {
                reason: CordisWorkflowStopReason::Cancelled,
                error: Some(reason.unwrap_or_else(|| "workflow cancelled".into())),
                agents_started: None,
            },
            BackgroundActivityOutcome::Failed { diagnostic }
            | BackgroundActivityOutcome::Interrupted { diagnostic } => CordisWorkflowSettlement {
                reason: CordisWorkflowStopReason::Error,
                error: Some(diagnostic.clone()),
                agents_started: None,
            },
        };
        let request = RequestId::new();
        if let Err(error) = persist(|| {
            job.activity.end_with_output(
                request,
                outcome.clone(),
                notice.clone(),
                Some(output_terminal.clone()),
            )
        })
        .await
        {
            tracing::error!(%error, %id, "Cordis workflow settlement failed");
            settlement.reason = CordisWorkflowStopReason::Error;
            settlement.error = Some("Cordis workflow outcome could not be committed".into());
        } else {
            self.manager.remember_job_settlement(job, cause).await;
        }
        drop(notice_gate);
        (start.on_settled)(settlement);
    }

    async fn publish_observation(&self, update: WorkflowObservation) {
        let WorkflowObservation { phase, chunk } = update;
        let total = chunk.at + chunk.text.len() as u64;
        let appended = match self
            .manager
            .append_job_output(&self.job, total, &[chunk])
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, job_id = %self.job.activity.id(), "Cordis workflow output could not be saved");
                false
            }
        };
        if let Some(phase) = phase {
            self.manager
                .publish_job_view(&self.job, |mut job| {
                    job["detail"] = phase.clone().into();
                    job["progress"] = phase.into();
                    registry::Event::Progress { job }
                })
                .await;
        }
        if appended {
            self.manager.publish_job_event(
                &self.job,
                registry::Event::Output {
                    id: self.job.activity.id().to_string(),
                    owner: self.job.activity.session_id().get().to_string(),
                    total,
                },
            );
        }
    }

    async fn completion_notice(
        &self,
        start: &CordisWorkflowStart,
        status: &str,
        result: Option<&CordisWorkflowResult>,
        inline: bool,
    ) -> BackgroundActivityNotice {
        let id = self.job.activity.id();
        let mut text = format!("Cordis workflow {} {status} (job {id}).", start.name);
        let limit = start.inline_output_bytes.min(8192);
        let mut output_position = None;
        if let Some(result) = result.filter(|_| inline) {
            let rendered = render_result(&start.name, start.max_result_chars, result);
            let reserved = text.len() + 2 + rendered.len();
            if let Some(room) = limit.checked_sub(reserved)
                && let Some((progress, position)) =
                    self.manager.inline_job_output(&self.job, room, true).await
            {
                text.push('\n');
                text.push_str(&progress);
                text.push_str(&rendered);
                output_position = Some(position);
            }
        }
        if output_position.is_none() {
            text.push_str(&format!(" Read the result with job_output for {id}."));
        }
        BackgroundActivityNotice {
            text,
            origin_message_id: self.owner.origin,
            wake: start.wake_on_completion,
            wake_budget: start
                .max_consecutive_wakes
                .map(|limit| BackgroundActivityWakeBudget {
                    source: self.plugin.to_string(),
                    limit,
                }),
            output_position,
        }
    }

    async fn authorized_child(
        &self,
        request: AgentSpawnRequest,
    ) -> Result<AgentSpawnRequest, AppError> {
        let session = self.job.activity.session_id();
        let (executor, context) = self
            .manager
            .host_callback_executor(Some(session.get()), None)
            .await?;
        let Some(context) = context.filter(|context| {
            !context.global
                && context.owner.is_none()
                && context.channel_authority.is_none()
                && executor.plugin_is_enabled(&self.plugin)
        }) else {
            return Err(AppError::Config(
                "Cordis workflow plugin is unavailable in its owning session".into(),
            ));
        };
        if context.relation_kind.is_subagent() {
            return Err(AppError::Config(
                "delegated agents cannot create nested agents".into(),
            ));
        }
        let executor = executor.with_cancellation_token(Some(self.job.cancel.clone()));
        let invocation = agent_invocation(&request)?;
        let prepared = executor
            .prepare_invocation(&invocation, session.get(), -1)
            .await
            .map_err(tool_error)?;
        if let Some(reason) = prepared.permission_ask {
            return Err(AppError::Config(format!(
                "Cordis workflow child requires interactive approval: {reason}"
            )));
        }
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(
                &prepared.invocation,
                Some(session.get()),
            )
            .await
            .map_err(tool_error)?;
        {
            let _read = HOST_POLICY_READS
                .acquire()
                .await
                .expect("host policy admission is never closed");
            let workspace = self
                .manager
                .kernel()
                .reader
                .metadata(session)
                .await
                .map_err(kernel_error)?
                .workspace_id;
            for check in checks {
                let key = permission_key(&check.action)?;
                let resolution = self
                    .manager
                    .resolve_tool_permission(&key, &check.decision, session, workspace)
                    .await
                    .map_err(tool_error)?;
                match &resolution.decision {
                    PermissionDecision::Allow => {}
                    PermissionDecision::Ask { .. } | PermissionDecision::Auto { .. }
                        if self.grants.get(&key) == Some(&resolution) => {}
                    PermissionDecision::Deny { reason } => {
                        return Err(AppError::Config(format!(
                            "Cordis child delegation denied: {reason}"
                        )));
                    }
                    PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
                        return Err(AppError::Config(format!(
                            "Cordis child delegation requires interactive approval: {reason}"
                        )));
                    }
                }
            }
        }
        match executor
            .run_imported_tool_check(
                &prepared.invocation,
                session.get(),
                -1,
                PermissionDecision::Allow,
            )
            .await
        {
            Some(ImportedToolDecision::Deny { reason } | ImportedToolDecision::Ask { reason }) => {
                return Err(AppError::Config(reason.unwrap_or_else(|| {
                    "A plugin blocked detached agent delegation".into()
                })));
            }
            _ => {}
        }
        serde_json::from_value(Value::from(prepared.invocation.input)).map_err(AppError::from)
    }

    async fn stop_owned_child(&self, child: i64) -> Result<(), AppError> {
        // A finished child may have been reused by the user. Check the
        // persisted ownership marker under the same fence as agent.followup.
        let _preparation = self.manager.agent_preparation.lock().await;
        let session = session_id(child)?;
        let configuration = read_with_busy_retry(session, "cordis_child_stop", || {
            self.manager.kernel().reader.configuration(session)
        })
        .await?;
        if configuration.agent.as_ref().is_some_and(|agent| {
            agent.workflow == Some(self.owner.id) && !agent.status.is_terminal()
        }) {
            self.manager
                .cancel_active_execution_with_outcome(child)
                .await?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl CordisWorkflowJobHost for JobHost {
    async fn spawn_child(&self, request: AgentSpawnRequest) -> agena_plugin_host::sdk::Result<i64> {
        let session = self.job.activity.session_id();
        let request = self
            .authorized_child(request)
            .await
            .map_err(workflow_error)?;
        if self.job.cancel.is_cancelled() {
            return Err(workflow_error(AppError::Cancelled));
        }
        let manager = self.manager.clone();
        let owner = self.owner.clone();
        // A tracked completion records the child even if QuickJS drops the
        // awaiting future on timeout or cancellation.
        let children = self.children.clone();
        self.launches
            .spawn(async move {
                let child = manager
                    .spawn_agent_with_workflow(session.get(), None, request, Some(owner))
                    .await
                    .map_err(workflow_error)?
                    .agent_id;
                children.lock().insert(child);
                Ok(child)
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?
    }

    async fn wait_child(
        &self,
        child: i64,
        timeout_ms: u64,
    ) -> agena_plugin_host::sdk::Result<ToolInvokeOutput> {
        if !self.children.lock().contains(&child) {
            return Err(PluginError::invalid_params(
                "The child does not belong to this Cordis workflow job",
            ));
        }
        let result = self
            .manager
            .wait_imported_child(
                self.job.activity.session_id().get(),
                child,
                timeout_ms,
                self.job.cancel.clone(),
            )
            .await
            .map_err(workflow_error);
        if result.is_ok() {
            self.children.lock().remove(&child);
        }
        result
    }

    fn progress(&self, message: &str) {
        let mut capture = self.progress_capture.lock();
        let phase = message.strip_prefix("phase: ").and_then(|phase| {
            let phase: String = bounded(phase, 256)
                .chars()
                .filter(|character| !character.is_control())
                .collect();
            let mut state = self.job.state.lock();
            if state.progress.as_ref() == Some(&phase) {
                return None;
            }
            state.progress = Some(phase.clone());
            Some(phase)
        });
        let mut line = bounded(message, 4095).to_owned();
        line.retain(|character| !character.is_control() || matches!(character, '\n' | '\t'));
        line.push('\n');
        if capture.written < MAX_PROGRESS_BYTES {
            capture.written += line.len();
            let text = if capture.written > MAX_PROGRESS_BYTES {
                "[remaining progress omitted]\n"
            } else {
                &line
            };
            if self.output.try_append(text).is_err() {
                let _ = self.output.record_loss(
                    text.len() as u64,
                    "Cordis progress exceeded output admission",
                );
            }
        }
        let at = capture.total;
        capture.total += line.len() as u64;
        let _ = self.observations.try_send(WorkflowObservation {
            phase,
            chunk: ActivityOutputChunk {
                at,
                text: line,
                channel: Some(ActivityOutputChannel::Log),
                gap_before: false,
            },
        });
    }
}

pub(super) fn agent_invocation(request: &AgentSpawnRequest) -> Result<ToolInvocation, AppError> {
    let value = serde_json::to_value(request)?;
    let input = StructuredObject::try_from(value).map_err(AppError::Config)?;
    Ok(ToolInvocation::new(AGENT_TOOL, input))
}

fn job_prefix(plugin: &PluginKey) -> String {
    format!("{JOB_LABEL_PREFIX}{plugin}:")
}

fn unknown_job() -> AppError {
    AppError::Config("Unknown Cordis job in this session".into())
}

#[derive(Clone, Copy)]
struct JobLabel<'a> {
    external_id: Option<&'a str>,
    kind: &'a str,
    text: &'a str,
    output_limit: Option<usize>,
}

fn job_label<'a>(activity: &'a BackgroundActivityView, prefix: &str) -> Option<JobLabel<'a>> {
    let (first, rest) = activity.start.label.strip_prefix(prefix)?.split_once(':')?;
    match activity.start.kind {
        BackgroundActivityKind::Agent => {
            uuid::Uuid::parse_str(first).ok()?;
            Some(JobLabel {
                external_id: None,
                kind: "workflow",
                text: rest,
                output_limit: None,
            })
        }
        BackgroundActivityKind::Process => {
            let (id, rest) = rest.split_at_checked(first.parse().ok()?)?;
            let kind = producers::kind_from_job_id(id)?;
            let (limit, text) = rest.strip_prefix(':')?.split_once(':')?;
            let output_limit = (limit != "0")
                .then(|| limit.parse::<usize>())
                .transpose()
                .ok()?;
            Some(JobLabel {
                external_id: Some(id),
                kind,
                text,
                output_limit,
            })
        }
        _ => None,
    }
}

/// Generic job captures are read by job_output and completion delivery.
/// The origin tool's provider projection only needs the returned job ID.
pub(in crate::session) fn cordis_job_capture_is_explicit(
    activity: &BackgroundActivityView,
) -> bool {
    let Some((plugin, _)) = activity
        .start
        .label
        .strip_prefix(JOB_LABEL_PREFIX)
        .and_then(|label| label.split_once(':'))
    else {
        return false;
    };
    let prefix = &activity.start.label[..JOB_LABEL_PREFIX.len() + plugin.len() + 1];
    job_label(activity, prefix).is_some_and(|label| label.external_id.is_some())
}

fn public_job(
    activity: &BackgroundActivityView,
    label: JobLabel<'_>,
    live: Option<&RunningJob>,
) -> Value {
    let native = label.external_id.is_none();
    let progress = live.and_then(|job| job.state.lock().progress.clone());
    let (status, detail) = match &activity.outcome {
        None => {
            let status = if live.is_some_and(RunningJob::is_stopping) {
                "stopping"
            } else {
                "running"
            };
            (status, progress.clone().filter(|_| native))
        }
        Some(BackgroundActivityOutcome::Succeeded { value }) => (
            "completed",
            value
                .pointer("/result/agentsStarted")
                .and_then(Value::as_u64)
                .map(|n| format!("{n} agent{}", if n == 1 { "" } else { "s" }))
                .or_else(|| {
                    value
                        .get("detail")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
        ),
        Some(BackgroundActivityOutcome::Stopped) => ("killed", None),
        Some(
            BackgroundActivityOutcome::Failed { diagnostic }
            | BackgroundActivityOutcome::Interrupted { diagnostic },
        ) => {
            let diagnostic = bounded(diagnostic, 1024)
                .chars()
                .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
                .collect::<String>();
            ("failed", Some(diagnostic))
        }
    };
    let mut job = json!({
        "id": label.external_id.map_or_else(|| activity.id.to_string(), str::to_owned),
        "kind": label.kind, "label": label.text,
        "status": status, "startedAt": activity.created_at_ms,
    });
    if !native {
        job["owner"] = activity.origin_session_id.get().to_string().into();
        if let Some(limit) = label.output_limit {
            job["outputLimitBytes"] = limit.into();
        }
        if let Some(progress) = progress.filter(|_| activity.outcome.is_none()) {
            job["progress"] = progress.into();
        }
    }
    if let Some(detail) = detail {
        job["detail"] = detail.into();
    }
    if let Some(ended) = activity.finished_at_ms {
        job["finishedAt"] = ended.into();
    }
    job
}

fn stored_result(value: &Value) -> Option<CordisWorkflowResult> {
    serde_json::from_value(value.get("result")?.clone()).ok()
}

fn render_result(name: &str, max_chars: usize, result: &CordisWorkflowResult) -> String {
    let text = serde_json::to_string_pretty(&result.value).unwrap_or_else(|_| "null".into());
    let end = text
        .char_indices()
        .nth(max_chars.min(24 * 1024))
        .map_or(text.len(), |(index, _)| index);
    let end = end.min(text.floor_char_boundary(24 * 1024));
    let mut rendered = format!(
        "workflow \"{}\" completed ({} agents).\nReturn value:\n{}",
        name,
        result.agents_started,
        &text[..end]
    );
    if end < text.len() {
        rendered.push_str("\n… [truncated]");
    }
    rendered
}

fn bounded(text: &str, max: usize) -> &str {
    &text[..text.floor_char_boundary(max.min(text.len()))]
}

fn workflow_error(error: AppError) -> PluginError {
    match error {
        AppError::Cancelled => {
            PluginError::from_kind(PluginErrorKind::Cancelled, "Cordis workflow cancelled")
        }
        AppError::Config(message) => PluginError::invalid_params(message),
        error => PluginError::internal_error(&error),
    }
}
