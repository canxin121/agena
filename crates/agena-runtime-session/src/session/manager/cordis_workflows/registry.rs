//! Session-scoped registry views and live Cordis job observations.

use super::*;
use agena_plugin_host::sdk::host_api::{
    CordisJobCancelReply, CordisJobCancellation, CordisJobControl, CordisJobQuery,
    CordisJobRegistry,
};
pub(super) use agena_plugin_host::sdk::host_api::{
    CordisJobEvent as Event, CordisJobSettlementCause as SettlementCause,
};
use std::sync::Weak;
use tokio::sync::broadcast;

pub(super) const EVENT_CAPACITY: usize = 128;
const MAX_WAIT_MS: u64 = 600_000;
type EventChannel = broadcast::Sender<Arc<Event>>;

#[derive(Clone, Default)]
pub(super) struct JobEvents(
    Arc<parking_lot::Mutex<HashMap<(SessionId, PluginKey), Weak<EventChannel>>>>,
);

impl JobEvents {
    fn channel(&self, session: SessionId, plugin: &PluginKey) -> Arc<EventChannel> {
        let mut channels = self.0.lock();
        channels.retain(|_, channel| channel.strong_count() > 0);
        let slot = channels.entry((session, plugin.clone())).or_default();
        if let Some(channel) = slot.upgrade() {
            return channel;
        }
        let sender = Arc::new(broadcast::channel(EVENT_CAPACITY).0);
        *slot = Arc::downgrade(&sender);
        sender
    }

    fn active_channel(&self, session: SessionId, plugin: &PluginKey) -> Option<Arc<EventChannel>> {
        self.0
            .lock()
            .get(&(session, plugin.clone()))
            .and_then(Weak::upgrade)
            .filter(|channel| channel.receiver_count() > 0)
    }

    pub(super) fn is_observed(&self, job: &RunningJob) -> bool {
        self.active_channel(job.activity.session_id(), &job.plugin)
            .is_some()
    }

    pub(super) fn publish(&self, session: SessionId, plugin: &PluginKey, event: Event) {
        if let Some(channel) = self.active_channel(session, plugin) {
            let _ = channel.send(Arc::new(event));
        }
    }
}

struct Registry {
    manager: Weak<SessionManager>,
    session: SessionId,
    plugin: PluginKey,
    channel: Arc<EventChannel>,
    plugin_stop: CancellationToken,
    lifetime: CancellationToken,
}

impl SessionManager {
    pub async fn open_cordis_job_registry(
        self: &Arc<Self>,
        session: i64,
        plugin: PluginKey,
        plugin_stop: CancellationToken,
        lifetime: CancellationToken,
    ) -> Result<Arc<dyn CordisJobRegistry>, AppError> {
        let session = session_id(session)?;
        self.authorize_job_registry(session, &plugin).await?;
        let channel = self.cordis_workflows.events.channel(session, &plugin);
        Ok(Arc::new(Registry {
            manager: Arc::downgrade(self),
            session,
            plugin,
            channel,
            plugin_stop,
            lifetime,
        }))
    }

    async fn authorize_job_registry(
        &self,
        session: SessionId,
        plugin: &PluginKey,
    ) -> Result<(), AppError> {
        let state = self.state_for_session(session).await?;
        let context = self.execution_context_with_state(session, &state).await?;
        let executor = state.executor.for_session_resource_context(&context);
        if context.global
            || context.owner.is_some()
            || context.channel_authority.is_some()
            || !executor.plugin_is_enabled(plugin)
        {
            return Err(AppError::Config(
                "Cordis jobs require an enabled plugin in a project session".into(),
            ));
        }
        Ok(())
    }

    async fn registry_view(
        &self,
        activity: &BackgroundActivityView,
        prefix: &str,
    ) -> Result<Value, AppError> {
        let page = self
            .packet_output(activity.origin_session_id, activity.id, None)
            .await?;
        if let Some(page) = page.as_ref() {
            return self.project_registry_job(activity, prefix, page);
        }
        let (page, _) = self.legacy_job_output(activity, None, false).await?;
        self.project_registry_job(activity, prefix, &page)
    }

    fn project_registry_job(
        &self,
        activity: &BackgroundActivityView,
        prefix: &str,
        output: &ActivityOutputPage,
    ) -> Result<Value, AppError> {
        let session = activity.origin_session_id;
        let label = job_label(activity, prefix).ok_or_else(unknown_job)?;
        let running = self
            .cordis_workflows
            .running
            .lock()
            .get(&activity.id)
            .cloned();
        let mut view = public_job(activity, label, running.as_deref());
        view["owner"] = session.get().to_string().into();
        if label.external_id.is_none()
            && activity.outcome.is_none()
            && let Some(progress) = running
                .as_ref()
                .and_then(|job| job.state.lock().progress.clone())
        {
            view["progress"] = progress.into();
        }
        if activity.outcome.is_some()
            && let Some(detail) = output
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.detail.as_ref())
        {
            view["detail"] = detail.as_str().into();
        }
        view["output"] = json!(output.head);
        if !output.spill_paths.is_empty() {
            view["output"]["spillPaths"] = json!(output.spill_paths);
        }
        Ok(view)
    }

    async fn observed_job_view(&self, job: &RunningJob) -> Option<Value> {
        if !self.cordis_workflows.events.is_observed(job) {
            return None;
        }
        let prefix = job_prefix(&job.plugin);
        let activity = self
            .owned_cordis_activity(job.activity.session_id(), job.activity.id(), &prefix)
            .await
            .ok()?;
        self.registry_view(&activity, &prefix).await.ok()
    }

    pub(super) fn publish_job_event(&self, job: &RunningJob, event: Event) {
        self.cordis_workflows
            .events
            .publish(job.activity.session_id(), &job.plugin, event);
    }

    pub(super) async fn publish_job_view(
        &self,
        job: &RunningJob,
        event: impl FnOnce(Value) -> Event,
    ) {
        if let Some(view) = self.observed_job_view(job).await {
            self.publish_job_event(job, event(view));
        }
    }

    pub(super) async fn remember_job_settlement(&self, job: &RunningJob, cause: SettlementCause) {
        if let Some(view) = self.observed_job_view(job).await {
            job.state.lock().registry_settlement = Some((view, cause));
        }
    }

    pub(super) async fn kill_cordis_job(
        &self,
        session: SessionId,
        prefix: &str,
        id: &str,
        reason: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<(ReadView<BackgroundActivityView>, Option<Arc<RunningJob>>), AppError> {
        let activity = self.find_cordis_activity(session, id, prefix).await?;
        if activity.outcome.is_some() {
            return Ok((activity, None));
        }
        let running = self
            .cordis_workflows
            .running
            .lock()
            .get(&activity.id)
            .cloned();
        if let Some(job) = running {
            let committed = if job.producer.is_some() {
                self.cancel_producer(job.clone(), reason, cancel).await?
            } else {
                job.commit_kill(reason)
            };
            if committed {
                return Ok((activity, Some(job)));
            }
            tokio::select! {
                biased;
                _ = job.wait() => {}
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
            }
        }
        let activity = self
            .owned_cordis_activity(session, activity.id, prefix)
            .await?;
        if activity.outcome.is_none() {
            return Err(AppError::Config("Cordis job has no live supervisor".into()));
        }
        Ok((activity, None))
    }

    async fn cancel_producer(
        &self,
        job: Arc<RunningJob>,
        reason: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        let manager = self.clone();
        let pending = cancel.child_token();
        let _cancel_pending = pending.clone().drop_guard();
        // If the calling module goes away after the producer starts its
        // callback, finish committing the callback's result independently.
        tokio::spawn(async move {
            let (begin, claimed) = oneshot::channel();
            let control = job.producer.as_ref().expect("Cordis producer controller");
            control(CordisJobControl::Cancel(CordisJobCancellation {
                reason: reason.clone(),
                begin,
            }));
            let CordisJobCancelReply { ready, result, applied } = tokio::select! {
                biased;
                _ = job.wait() => return Ok(false),
                _ = pending.cancelled() => return Err(AppError::Cancelled),
                claim = claimed => claim.map_err(|_| AppError::Config("Cordis producer could not accept cancellation".into()))?,
                _ = tokio::time::sleep(Duration::from_secs(3)) => {
                    return Err(AppError::Config("Cordis producer did not respond to cancellation".into()));
                }
            };
            ready.send(()).map_err(|_| AppError::Cancelled)?;
            let accepted = result.await.map_err(|_| AppError::Cancelled)?
                .map_err(AppError::Config)?;
            let committed = accepted && job.commit_kill(reason);
            if committed {
                manager.publish_job_view(&job, |job| Event::Stopping { job }).await;
            }
            let _ = applied.send(());
            Ok(committed)
        })
        .await
        .map_err(|error| AppError::internal_error(&error))?
    }

    async fn remove_cordis_job(
        &self,
        session: SessionId,
        plugin: &PluginKey,
        prefix: &str,
        id: &str,
    ) -> Result<(), AppError> {
        let activity = self.find_cordis_activity(session, id, prefix).await?;
        if activity.outcome.is_none() {
            let running = self
                .cordis_workflows
                .running
                .lock()
                .get(&activity.id)
                .cloned();
            let status = if running.as_deref().is_some_and(RunningJob::is_stopping) {
                "stopping"
            } else {
                "running"
            };
            return Err(AppError::Config(format!(
                "job {id} is still {status}; kill it and wait for settlement before removing it"
            )));
        }
        let running = self
            .cordis_workflows
            .running
            .lock()
            .get(&activity.id)
            .cloned();
        if let Some(running) = running {
            running.wait().await;
        }
        let view = self.registry_view(&activity, prefix).await?;
        let request = RequestId::new();
        let result = persist(|| {
            self.kernel()
                .sessions
                .unlist_settled_activity(request, session, activity.id)
        })
        .await
        .map_err(kernel_error)?;
        if !result.value {
            return Err(unknown_job());
        }
        self.cordis_workflows
            .events
            .publish(session, plugin, Event::Removed { job: view });
        Ok(())
    }
}

#[async_trait::async_trait]
impl CordisJobRegistry for Registry {
    fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.channel.subscribe()
    }

    async fn query(
        &self,
        request: CordisJobQuery,
        cancel: CancellationToken,
    ) -> agena_plugin_host::sdk::Result<Value> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| workflow_error(AppError::Cancelled))?;
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(AppError::Cancelled),
            _ = self.plugin_stop.cancelled() => Err(AppError::Cancelled),
            _ = manager.shutdown.cancelled() => Err(AppError::Cancelled),
            result = self.read(&manager, request, &cancel) => result,
        }
        .map_err(workflow_error)
    }
}

impl Registry {
    async fn read(
        &self,
        manager: &SessionManager,
        query: CordisJobQuery,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        manager
            .authorize_job_registry(self.session, &self.plugin)
            .await?;
        let prefix = job_prefix(&self.plugin);
        match query {
            CordisJobQuery::List => {
                let activities = read_with_busy_retry(self.session, "cordis_registry_list", || {
                    manager.kernel().reader.session_activities_by_label(
                        self.session,
                        &prefix,
                        JOB_LIST_LIMIT,
                    )
                })
                .await?;
                let mut ordered: Vec<_> = activities.iter().collect();
                ordered.sort_by_key(|activity| {
                    (activity.created_at_ms, activity.launch_entry, activity.id)
                });
                let mut views = Vec::with_capacity(ordered.len());
                for activity in ordered {
                    if cancel.is_cancelled() {
                        return Err(AppError::Cancelled);
                    }
                    if job_label(activity, &prefix).is_some() {
                        views.push(manager.registry_view(activity, &prefix).await?);
                    }
                }
                Ok(Value::Array(views))
            }
            CordisJobQuery::Get { id } => {
                let activity = manager
                    .find_cordis_activity(self.session, &id, &prefix)
                    .await?;
                manager.registry_view(&activity, &prefix).await
            }
            CordisJobQuery::ReadAt { id, from } => {
                let activity = manager
                    .find_cordis_activity(self.session, &id, &prefix)
                    .await?;
                let page = manager
                    .packet_output(self.session, activity.id, Some(from))
                    .await?;
                if let Some(page) = page.as_ref() {
                    return Ok(project_range(page, from));
                }
                let (page, _) = manager
                    .legacy_job_output(&activity, Some(from), false)
                    .await?;
                Ok(project_range(&page, from))
            }
            CordisJobQuery::Read { id } => self.consume_output(manager, &id, &prefix, cancel).await,
            CordisJobQuery::Kill { id, reason } => {
                let (_, running) = manager
                    .kill_cordis_job(self.session, &prefix, &id, reason, cancel)
                    .await?;
                Ok(if running.is_some() {
                    "requested"
                } else {
                    "already-finished"
                }
                .into())
            }
            CordisJobQuery::Remove { id } => {
                let manager = manager.clone();
                let session = self.session;
                let plugin = self.plugin.clone();
                tokio::spawn(async move {
                    manager
                        .remove_cordis_job(session, &plugin, &prefix, &id)
                        .await
                })
                .await
                .map_err(|error| AppError::internal_error(&error))??;
                Ok(Value::Null)
            }
            CordisJobQuery::Wait {
                id,
                timeout_ms,
                pre_aborted,
            } => {
                self.wait(manager, &id, &prefix, timeout_ms, pre_aborted, cancel)
                    .await
            }
        }
    }

    async fn consume_output(
        &self,
        manager: &SessionManager,
        id: &str,
        prefix: &str,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let mut activity = manager
            .find_cordis_activity(self.session, id, prefix)
            .await?;
        // A JS tool may already own the gate through a staged model read. A
        // synchronous registry command must not wait for its own tool commit.
        let _gate = manager
            .cordis_workflows
            .read_gate(activity.id)
            .try_lock_owned()
            .map_err(|_| AppError::PersistenceBusy)?;
        loop {
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let label = job_label(&activity, prefix).ok_or_else(unknown_job)?;
            let previous = manager
                .cordis_output_position(self.session, activity.id)
                .await?;
            let from = previous
                .packet_offset
                .or_else(|| previous.cursor.map(|cursor| cursor.offset))
                .unwrap_or_default();
            let packets = manager
                .packet_output(self.session, activity.id, Some(from))
                .await?;
            let legacy;
            let (output, mut position) = if let Some(packets) = packets.as_ref() {
                (
                    packets,
                    ActivityOutputPosition {
                        packet_offset: Some(packets.head.total),
                        ..Default::default()
                    },
                )
            } else {
                let (output, position) = manager
                    .legacy_job_output(&activity, Some(from), true)
                    .await?;
                legacy = output;
                (&legacy, position)
            };
            if activity.outcome.is_none() && output.terminal.is_some() {
                activity = manager
                    .owned_cordis_activity(self.session, activity.id, prefix)
                    .await?;
                continue;
            }
            position.outcome_read = previous.outcome_read || activity.outcome.is_some();
            let result = if !previous.outcome_read && activity.outcome.is_some() {
                output::terminal_result(&activity, label, Some(output))
            } else {
                None
            };
            if position != previous {
                let request = RequestId::new();
                let read = ActivityOutputRead {
                    activity: activity.id,
                    position,
                };
                let committed = persist(|| {
                    manager.kernel().sessions.acknowledge_activity_output(
                        request,
                        self.session,
                        read,
                        previous,
                    )
                })
                .await
                .map_err(kernel_error)?;
                if !committed.value {
                    activity = manager
                        .owned_cordis_activity(self.session, activity.id, prefix)
                        .await?;
                    continue;
                }
            }
            let mut reply = json!({
                "chunks": output.chunks,
                "lossy": output::is_lossy(output, from),
                "job": manager.registry_view(&activity, prefix).await?,
            });
            if let Some(result) = result {
                reply["result"] = result.into();
            }
            return Ok(reply);
        }
    }

    async fn wait(
        &self,
        manager: &SessionManager,
        id: &str,
        prefix: &str,
        timeout_ms: u64,
        pre_aborted: bool,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        if timeout_ms == 0 || timeout_ms > MAX_WAIT_MS {
            return Err(AppError::Config(
                "Cordis job wait requires a timeout from 1 to 600000 ms".into(),
            ));
        }
        let activity = manager
            .find_cordis_activity(self.session, id, prefix)
            .await?;
        let running = if activity.outcome.is_none() {
            manager
                .cordis_workflows
                .running
                .lock()
                .get(&activity.id)
                .cloned()
        } else {
            None
        };
        let Some(running) = running else {
            return manager.registry_view(&activity, prefix).await;
        };
        let activity_id = activity.id;
        drop(activity);
        let lease = WaitLease::register(&running);
        if pre_aborted && lease.is_some() {
            return Err(AppError::Cancelled);
        }
        if lease.is_some() {
            tokio::select! {
                biased;
                _ = running.wait() => {}
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {}
            }
        }
        drop(lease);
        // This wait attached while the record was visible. A concurrent
        // remove can hide it from new callers after releasing the waiter.
        let activity = read_with_busy_retry(self.session, "cordis_job_wait_result", || {
            manager.kernel().reader.activity(self.session, activity_id)
        })
        .await?;
        manager.registry_view(&activity, prefix).await
    }
}

fn project_range(output: &ActivityOutputPage, from: u64) -> Value {
    json!({"chunks": output.chunks, "next": output.head.total, "lossy": output::is_lossy(output, from)})
}

pub(super) struct WaitLease<'a>(&'a RunningJob);

impl<'a> WaitLease<'a> {
    pub(super) fn register(job: &'a RunningJob) -> Option<Self> {
        let mut state = job.state.lock();
        if job.settled.load(Ordering::Acquire) {
            return None;
        }
        state.waiters += 1;
        Some(Self(job))
    }
}

impl Drop for WaitLease<'_> {
    fn drop(&mut self) {
        self.0.state.lock().waiters -= 1;
    }
}
