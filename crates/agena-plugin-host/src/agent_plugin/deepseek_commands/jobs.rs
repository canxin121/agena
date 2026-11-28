use std::{
    collections::VecDeque,
    sync::{
        OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use agena_domain::{
    ActivityOutputChannel, ActivityOutputChunk, MAX_ACTIVITY_SPILL_PATHS, valid_activity_spill_path,
};
use agena_plugin_sdk::host_api::{
    CordisJobCancelReply, CordisJobCancellation, CordisJobControl, CordisJobEvent,
    CordisJobOutcome, CordisJobProducer, CordisJobQuery, CordisJobRegistry, CordisJobSpec,
    CordisJobStart, CordisJobStatus, CordisJobUpdate,
};
use rquickjs::Exception;
use tokio::sync::{Notify, oneshot, watch};

use super::{
    super::workflow::{CordisCompletionDelivery, CordisJobsConfig},
    *,
};

const JOBS: &str = include_str!("jobs.js");
const JOBS_GLOBAL: &str = "__agenaCordisJobs";
const OUTPUT_QUEUE: usize = 64;
const EVENT_QUEUE: usize = 128;
const WAIT_LIMIT: usize = 16;
const CANCEL_QUEUE: usize = 32;

pub(super) const TIMERS_PROMISES: &str = r#"
export const setTimeout = globalThis.agenaCordisJobDelay;
export default Object.freeze({ setTimeout });
"#;

pub(super) const TIMERS: &str = r#"
export const setTimeout = globalThis.setTimeout;
export const clearTimeout = globalThis.clearTimeout;
export default Object.freeze({ setTimeout, clearTimeout });
"#;

type StartResult = Result<String, String>;

pub(super) struct Admission {
    spec: CordisJobSpec,
    choice: oneshot::Sender<String>,
    producer: oneshot::Receiver<CordisJobProducer>,
    finish: oneshot::Sender<StartResult>,
}

impl Admission {
    pub(super) async fn answer(
        self,
        host: &dyn HostClient,
        config: &CordisJobsConfig,
        report_completion: bool,
        cancellation: CancellationToken,
    ) {
        let Self {
            spec,
            choice,
            producer,
            finish,
        } = self;
        let start = CordisJobStart {
            spec,
            max_concurrent_per_owner: config.max_concurrent_per_owner,
            report_completion,
            wake_on_completion: matches!(
                config.completion_delivery,
                CordisCompletionDelivery::Wakeup
            ),
            max_consecutive_wakes: config.max_consecutive_wakes,
            inline_output_bytes: config.inline_output_bytes,
            output_retention: config.output_retention,
        };
        let result = host
            .start_cordis_job(
                start,
                Box::new(move |id| {
                    Box::pin(async move {
                        choice
                            .send(id)
                            .map_err(|_| PluginError::internal("Cordis job starter stopped"))?;
                        producer.await.map_err(|_| {
                            PluginError::invalid_params(
                                "Cordis job starter did not return a producer",
                            )
                        })
                    })
                }),
                cancellation,
            )
            .await;
        let _ = finish.send(result.map_err(|error| error.failure.user.fallback.clone()));
    }
}

#[derive(Serialize)]
struct Control {
    id: String,
    action: &'static str,
}

/// The admission channel borrows one current tool call. Control messages
/// belong to the retained module and receive no tool-call authority.
pub(super) struct VmJobs {
    foreground: Mutex<Option<Foreground>>,
    pumping: AtomicBool,
    queued: Mutex<Vec<Control>>,
    cancellations: Mutex<VecDeque<(String, CordisJobCancellation)>>,
    cancellation_ready: Notify,
    events: Mutex<VecDeque<Arc<CordisJobEvent>>>,
    registry: OnceLock<Registry>,
    waits: Mutex<BTreeMap<u32, RegistryWait>>,
    outputs: Mutex<BTreeMap<String, mpsc::WeakSender<CordisJobUpdate>>>,
    pub(super) wake: Notify,
    budget: Arc<Mutex<Option<ScriptBudget>>>,
    controllers: Arc<controllers::JobControllers>,
    controller_count: Arc<AtomicUsize>,
}

struct Registry {
    session: i64,
    owner: String,
    host: Arc<dyn CordisJobRegistry>,
    runtime: tokio::runtime::Handle,
}

struct RegistryWait {
    foreground: Option<Weak<Notify>>,
    cancel: CancellationToken,
    result: Option<RegistryReply>,
}

#[derive(Serialize)]
struct RegistryReply {
    id: u32,
    value: Option<Value>,
    error: Option<String>,
}

pub(super) struct Foreground {
    pub(super) starts: Option<mpsc::Sender<Admission>>,
    pub(super) wake: Arc<Notify>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Schedule {
    pub(super) live: bool,
    pub(super) delay_ms: Option<u64>,
}

impl VmJobs {
    pub(super) fn new(
        budget: Arc<Mutex<Option<ScriptBudget>>>,
        controllers: Arc<controllers::JobControllers>,
    ) -> Arc<Self> {
        Arc::new(Self {
            foreground: Mutex::new(None),
            pumping: AtomicBool::new(false),
            queued: Mutex::new(Vec::new()),
            cancellations: Mutex::new(VecDeque::new()),
            cancellation_ready: Notify::new(),
            events: Mutex::new(VecDeque::new()),
            registry: OnceLock::new(),
            waits: Mutex::new(BTreeMap::new()),
            outputs: Mutex::new(BTreeMap::new()),
            wake: Notify::new(),
            budget,
            controllers,
            controller_count: Arc::default(),
        })
    }

    pub(super) fn install<'js>(
        self: &Arc<Self>,
        ctx: &Ctx<'js>,
        manager: Object<'js>,
        config: &CordisJobsConfig,
    ) -> rquickjs::Result<Object<'js>> {
        let create: Function = ctx.eval(JOBS)?;
        let starter = self.clone();
        let packet_limit = config.output_retention.live_bytes;
        let start_native = Func::from(move |ctx: Ctx<'js>, spec: String, run: Function<'js>| {
            starter.start(&ctx, &spec, &run, packet_limit)
        });
        let notifier = self.clone();
        let wake = Func::from(move || notifier.notify());
        let count = self.controller_count.clone();
        let set_controller_count =
            Func::from(move |size: usize| count.store(size, Ordering::Release));
        let registry = self.clone();
        let query = Func::from(
            move |ctx: Ctx<'js>,
                  wire: String,
                  owner: Option<String>,
                  wait_id: Option<u32>,
                  cancel_ms: Option<u64>| {
                registry.query(&ctx, &wire, owner.as_deref(), wait_id, cancel_ms)
            },
        );
        let cancelling = self.clone();
        let cancel = Func::from(move |id: u32| {
            if let Some(wait) = cancelling
                .waits
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
            {
                wait.cancel.cancel();
            }
        });
        let binding: Object = create.call((
            start_native,
            wake,
            query,
            cancel,
            set_controller_count,
            manager,
            config.pump_poll_ms.clamp(1, 60_000),
        ))?;
        ctx.globals().set(JOBS_GLOBAL, binding.clone())?;
        Ok(binding)
    }

    pub(super) fn bind_registry(
        &self,
        session: i64,
        host: Arc<dyn CordisJobRegistry>,
        lifetime: CancellationToken,
    ) {
        if self
            .registry
            .set(Registry {
                session,
                owner: session.to_string(),
                host,
                runtime: tokio::runtime::Handle::current(),
            })
            .is_ok()
        {
            self.controllers
                .bind(session, &self.controller_count, lifetime);
        }
    }

    pub(super) fn has_controller(&self) -> bool {
        self.controller_count.load(Ordering::Acquire) > 0
    }

    pub(super) fn queue_event(&self, event: Arc<CordisJobEvent>) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if events.len() >= EVENT_QUEUE {
            events.pop_front();
        }
        events.push_back(event);
        drop(events);
        self.notify();
    }

    fn query<'js>(
        self: &Arc<Self>,
        ctx: &Ctx<'js>,
        wire: &str,
        owner: Option<&str>,
        wait_id: Option<u32>,
        cancel_ms: Option<u64>,
    ) -> rquickjs::Result<String> {
        let error = |message: &str| Exception::throw_message(ctx, message);
        let request = serde_json::from_str::<CordisJobQuery>(wire)
            .map_err(|_| error("Invalid Cordis job registry query"))?;
        let registry = self.registry.get();
        let visible = owner.is_some()
            && registry.is_some_and(|registry| owner == Some(registry.owner.as_str()));
        if !visible && matches!(request, CordisJobQuery::List) {
            return Ok("[]".into());
        }
        let registry = registry
            .filter(|_| visible)
            .ok_or_else(|| error("Unknown Cordis job for the requested owner"))?;
        if let (CordisJobQuery::Wait { .. }, Some(id)) = (&request, wait_id) {
            self.start_wait(registry, id, request, cancel_ms)
                .map_err(error)?;
            return Ok("null".into());
        }
        if wait_id.is_some() || matches!(request, CordisJobQuery::Wait { .. }) {
            return Err(error("Invalid Cordis job wait"));
        }
        let cancel = self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|budget| budget.cancellation.clone())
            .unwrap_or_default();
        if matches!(request, CordisJobQuery::Kill { .. }) {
            let reply = self.kill(ctx, registry, request, cancel)?;
            return serde_json::to_string(&reply)
                .map_err(|_| error("Could not serialize the Cordis job cancellation result"));
        }
        let result = self
            .wait_host(|| {
                let outputs = self.output_senders(&request);
                registry.runtime.block_on(async move {
                    for output in outputs {
                        let (reply, result) = oneshot::channel();
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => {
                                return Err(PluginError::from_kind(PluginErrorKind::Cancelled, "Cordis job query cancelled"));
                            }
                            result = async {
                                if output.send(CordisJobUpdate::Flush(reply)).await.is_ok()
                                    && let Ok(result) = result.await
                                {
                                    result?;
                                }
                                Ok::<_, PluginError>(())
                            } => result?,
                        }
                    }
                    registry.host.query(request, cancel).await
                })
            })
            .map_err(|failure| error(&failure.failure.user.fallback))?;
        serde_json::to_string(&result).map_err(|_| error("Could not serialize the Cordis job view"))
    }

    fn output_senders(&self, request: &CordisJobQuery) -> Vec<mpsc::Sender<CordisJobUpdate>> {
        let mut map = self
            .outputs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        map.retain(|_, sender| sender.strong_count() > 0);
        match request {
            CordisJobQuery::Get { id }
            | CordisJobQuery::Read { id }
            | CordisJobQuery::ReadAt { id, .. } => map
                .get(id)
                .and_then(mpsc::WeakSender::upgrade)
                .into_iter()
                .collect(),
            CordisJobQuery::List => map.values().filter_map(mpsc::WeakSender::upgrade).collect(),
            CordisJobQuery::Wait { .. }
            | CordisJobQuery::Kill { .. }
            | CordisJobQuery::Remove { .. } => Vec::new(),
        }
    }

    fn kill(
        &self,
        ctx: &Ctx<'_>,
        registry: &Registry,
        request: CordisJobQuery,
        cancel: CancellationToken,
    ) -> rquickjs::Result<Value> {
        let host = registry.host.clone();
        let mut reply = registry
            .runtime
            .spawn(async move { host.query(request, cancel).await });
        let _abort = agena_async::AbortOnDrop::new(&reply);
        loop {
            let result = self.wait_host(|| {
                registry.runtime.block_on(async {
                    tokio::select! {
                        biased;
                        result = &mut reply => Some(result),
                        _ = self.cancellation_ready.notified() => None,
                    }
                })
            });
            if let Some(result) = result {
                return result
                    .map_err(|_| Exception::throw_message(ctx, "Cordis job cancellation ended"))?
                    .map_err(|error| Exception::throw_message(ctx, &error.failure.user.fallback));
            }
            // A synchronous caller can also be the producer, or be waiting
            // on a module whose callback needs to cancel one of its jobs.
            self.cancel_producers(ctx)?;
        }
    }

    fn cancel_producers(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let binding: Object = ctx.globals().get(JOBS_GLOBAL)?;
        let cancel: Function = binding.get("cancelProducer")?;
        loop {
            let request = self
                .cancellations
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front();
            let Some((id, request)) = request else {
                break;
            };
            let (ready, claimed) = oneshot::channel();
            let (report, result) = oneshot::channel();
            let (applied, committed) = oneshot::channel();
            if request
                .begin
                .send(CordisJobCancelReply {
                    ready,
                    result,
                    applied,
                })
                .is_err()
                || self.wait_host(|| claimed.blocking_recv()).is_err()
            {
                continue;
            }
            let _scope = self.pump_scope();
            let result = cancel
                .call::<_, bool>((id, request.reason))
                .catch(ctx)
                .map_err(|error| error.to_string());
            if report.send(result).is_ok() {
                // Settlement microtasks may already be ready. Let the host
                // record a successful cancel and publish stopping first.
                let _ = self.wait_host(|| committed.blocking_recv());
            }
        }
        Ok(())
    }

    fn start_wait(
        self: &Arc<Self>,
        registry: &Registry,
        id: u32,
        request: CordisJobQuery,
        cancel_ms: Option<u64>,
    ) -> Result<(), &'static str> {
        let foreground = self
            .current_foreground()
            .filter(|_| self.foreground_effects())
            .as_ref()
            .map(Arc::downgrade);
        let cancel = CancellationToken::new();
        {
            let mut waits = self.waits.lock().unwrap_or_else(|e| e.into_inner());
            if waits.len() >= WAIT_LIMIT || waits.contains_key(&id) {
                return Err("Cordis job wait limit reached");
            }
            waits.insert(
                id,
                RegistryWait {
                    foreground,
                    cancel: cancel.clone(),
                    result: None,
                },
            );
        }
        let worker = Arc::downgrade(self);
        let host = registry.host.clone();
        registry.runtime.spawn(async move {
            let query = host.query(request, cancel.clone());
            tokio::pin!(query);
            let result = if let Some(delay) = cancel_ms {
                tokio::select! {
                    result = &mut query => result,
                    _ = tokio::time::sleep(Duration::from_millis(delay)) => {
                        cancel.cancel();
                        query.await
                    }
                }
            } else {
                query.await
            };
            let reply = match result {
                Ok(value) => RegistryReply {
                    id,
                    value: Some(value),
                    error: None,
                },
                Err(error) => RegistryReply {
                    id,
                    value: None,
                    error: Some(error.failure.user.fallback.clone()),
                },
            };
            if let Some(worker) = worker.upgrade() {
                if let Some(wait) = worker
                    .waits
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_mut(&id)
                {
                    wait.result = Some(reply);
                }
                worker.notify();
            }
        });
        Ok(())
    }

    fn current_foreground(&self) -> Option<Arc<Notify>> {
        self.foreground
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|foreground| foreground.wake.clone())
    }

    fn completed_waits(&self) -> (Vec<RegistryReply>, Vec<RegistryReply>) {
        let current = self.current_foreground();
        let mut detached = Vec::new();
        let mut foreground = Vec::new();
        self.waits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, wait| {
                let Some(reply) = wait.result.take() else {
                    return true;
                };
                let origin = wait.foreground.as_ref().and_then(Weak::upgrade);
                if current
                    .as_ref()
                    .zip(origin.as_ref())
                    .is_some_and(|(current, origin)| Arc::ptr_eq(current, origin))
                {
                    foreground.push(reply);
                } else {
                    detached.push(reply);
                }
                false
            });
        (detached, foreground)
    }

    pub(super) fn enter(&self, foreground: Option<Foreground>) -> ForegroundGate<'_> {
        *self.foreground.lock().unwrap_or_else(|e| e.into_inner()) = foreground;
        ForegroundGate(self)
    }

    pub(super) fn foreground_effects(&self) -> bool {
        !self.pumping.load(Ordering::Relaxed)
    }

    fn notify(&self) {
        self.wake.notify_one();
        if let Some(foreground) = self
            .foreground
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            foreground.wake.notify_one();
        }
    }

    fn pump_scope(&self) -> PumpScope<'_> {
        let previous = self.pumping.swap(true, Ordering::Relaxed);
        PumpScope(self, previous)
    }

    fn start<'js>(
        self: &Arc<Self>,
        ctx: &Ctx<'js>,
        spec: &str,
        run: &Function<'js>,
        packet_limit: usize,
    ) -> rquickjs::Result<String> {
        let error = |message: &str| Exception::throw_message(ctx, message);
        if !self.foreground_effects() {
            return Err(error("Cordis job producers require a live project tool or command"));
        }
        let starts = self
            .foreground
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|foreground| foreground.starts.clone())
            .ok_or_else(|| error("Cordis job producers require a live project tool or command"))?;
        if !self
            .registry
            .get()
            .is_some_and(|registry| self.controllers.serves(registry.session))
        {
            return Err(error(
                "Cordis background jobs unavailable: no job controller serves this session",
            ));
        }
        let spec = serde_json::from_str(spec).map_err(|_| error("Invalid Cordis JobSpec"))?;
        let (choice, chosen) = oneshot::channel();
        let (submit, producer) = oneshot::channel();
        let (finish, registration) = oneshot::channel();
        let admission = Admission {
            spec,
            choice,
            producer,
            finish,
        };
        starts
            .try_send(admission)
            .map_err(|_| error("Cordis job admission is unavailable"))?;
        let Ok(id) = self.wait_host(|| chosen.blocking_recv()) else {
            return self.registration(ctx, registration);
        };

        let (publish, output) = mpsc::channel(OUTPUT_QUEUE);
        let sender = publish.downgrade();
        let (complete, outcome) = oneshot::channel();
        let (spill, spill_paths) = watch::channel::<Arc<[String]>>(Arc::from([]));
        let output_end = Arc::new(AtomicU64::new(0));
        let end = output_end.clone();
        let bytes = Arc::new(Semaphore::new(packet_limit * 2));
        let publish = Func::from(move |wire: String| {
            publish_update(&publish, &spill, &end, &bytes, packet_limit, &wire);
        });
        let complete = Mutex::new(Some(complete));
        let settle = Func::from(move |wire: String| {
            if let Some(complete) = complete.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = complete.send(decode_outcome(&wire));
            }
        });
        run.call::<_, ()>((id.as_str(), publish, settle))?;
        let scheduler = Arc::downgrade(self);
        let control = Arc::new(move |action| {
            if let Some(scheduler) = scheduler.upgrade() {
                let action = match action {
                    CordisJobControl::Cancel(request) => {
                        let mut pending = scheduler
                            .cancellations
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        pending.retain(|(_, request)| !request.begin.is_closed());
                        if pending.len() < CANCEL_QUEUE {
                            pending.push_back((id.clone(), request));
                            scheduler.cancellation_ready.notify_one();
                            scheduler.notify();
                        }
                        return;
                    }
                    CordisJobControl::Teardown => "cancel",
                    CordisJobControl::Abandon => "abandon",
                };
                scheduler
                    .queued
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Control {
                        id: id.clone(),
                        action,
                    });
                scheduler.notify();
            }
        });
        let _ = submit.send(CordisJobProducer {
            output,
            output_end,
            spill_paths,
            outcome,
            control,
        });
        let id = self.registration(ctx, registration)?;
        {
            let mut outputs = self
                .outputs
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            outputs.retain(|_, sender| sender.strong_count() > 0);
            outputs.insert(id.clone(), sender);
        }
        // Registration precedes the first pull. This drain still completes
        // inside jobs.start(), and source callbacks carry no caller effects.
        let binding: Object = ctx.globals().get(JOBS_GLOBAL)?;
        let pull: Function = binding.get("pullProducer")?;
        let _scope = self.pump_scope();
        pull.call::<_, ()>((id.as_str(),))?;
        Ok(id)
    }

    fn registration(
        &self,
        ctx: &Ctx<'_>,
        registration: oneshot::Receiver<StartResult>,
    ) -> rquickjs::Result<String> {
        self.wait_host(|| registration.blocking_recv())
            .unwrap_or_else(|_| Err("Cordis job admission ended".into()))
            .map_err(|message| Exception::throw_message(ctx, &message))
    }

    fn wait_host<T>(&self, call: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = call();
        if let Some(budget) = self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            budget.deadline += started.elapsed();
        }
        result
    }

    fn controls(&self) -> Vec<Control> {
        std::mem::take(&mut *self.queued.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

pub(super) struct ForegroundGate<'a>(&'a VmJobs);

impl Drop for ForegroundGate<'_> {
    fn drop(&mut self) {
        self.0
            .foreground
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}

struct PumpScope<'a>(&'a VmJobs, bool);

impl Drop for PumpScope<'_> {
    fn drop(&mut self) {
        self.0.pumping.store(self.1, Ordering::Relaxed);
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireJobUpdate {
    Append {
        text: String,
        channel: Option<ActivityOutputChannel>,
        #[serde(default)]
        gap_before: bool,
    },
    Progress {
        line: String,
    },
    SpillPaths {
        paths: Vec<String>,
    },
}

fn publish_update(
    sender: &mpsc::Sender<CordisJobUpdate>,
    spill: &watch::Sender<Arc<[String]>>,
    end: &AtomicU64,
    budget: &Arc<Semaphore>,
    limit: usize,
    wire: &str,
) {
    let Ok(update) = serde_json::from_str::<WireJobUpdate>(wire) else {
        return;
    };
    let update = match update {
        WireJobUpdate::Append {
            mut text,
            channel,
            gap_before,
        } => {
            let at = end.fetch_add(text.len() as u64, Ordering::AcqRel);
            let trim = text.ceil_char_boundary(text.len().saturating_sub(limit));
            if trim == text.len() {
                return;
            }
            if trim > 0 {
                text = text[trim..].to_owned();
            }
            let Ok(admission) = budget.clone().try_acquire_many_owned(text.len() as u32) else {
                return;
            };
            CordisJobUpdate::Append {
                chunk: ActivityOutputChunk {
                    at: at + trim as u64,
                    text,
                    channel,
                    gap_before: gap_before || trim > 0,
                },
                _admission: admission,
            }
        }
        WireJobUpdate::Progress { mut line } => {
            line.truncate(line.floor_char_boundary(line.len().min(1024)));
            CordisJobUpdate::Progress { line }
        }
        WireJobUpdate::SpillPaths { mut paths } => {
            paths.retain(|path| valid_activity_spill_path(path));
            paths.truncate(MAX_ACTIVITY_SPILL_PATHS);
            // Latest source metadata is independent of packet backpressure.
            spill.send_if_modified(|latest| {
                if latest.as_ref() == paths {
                    return false;
                }
                *latest = paths.into();
                true
            });
            return;
        }
    };
    // The byte position was allocated before the send. A later packet or
    // barrier preserves the gap if bounded admission drops this update.
    let _ = sender.try_send(update);
}

fn decode_outcome(wire: &str) -> CordisJobOutcome {
    serde_json::from_str(wire).unwrap_or_else(|_| CordisJobOutcome {
        status: CordisJobStatus::Failed,
        detail: Some("Invalid Cordis job outcome".into()),
        result: None,
    })
}

impl Vm {
    pub(super) fn advance_jobs(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<Duration>, String> {
        self.with_budget(
            Instant::now() + SCRIPT_TIMEOUT,
            Some(cancellation.clone()),
            || {
                self.context.with(|ctx| {
                    self.pump_jobs(&ctx, cancellation)?;
                    Ok(self.job_schedule(&ctx)?.delay_ms.map(Duration::from_millis))
                })
            },
        )
    }

    pub(super) fn job_schedule(&self, ctx: &Ctx<'_>) -> Result<Schedule, String> {
        let error = |error: rquickjs::CaughtError<'_>| error.to_string();
        let binding: Object = ctx.globals().get(JOBS_GLOBAL).catch(ctx).map_err(error)?;
        let schedule: Function = binding.get("schedule").catch(ctx).map_err(error)?;
        decode_js(ctx, schedule.call(()).catch(ctx).map_err(error)?)
    }

    pub(super) fn advance_parked_jobs(
        &self,
        ctx: &Ctx<'_>,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        let previous = self.budget.lock().unwrap_or_else(|e| e.into_inner()).take();
        let started = Instant::now();
        let result = self.with_budget(started + SCRIPT_TIMEOUT, Some(cancellation.clone()), || {
            self.pump_jobs(ctx, cancellation)
        });
        *self.budget.lock().unwrap_or_else(|e| e.into_inner()) = previous.map(|mut budget| {
            budget.deadline += started.elapsed();
            budget
        });
        result
    }

    fn pump_jobs(&self, ctx: &Ctx<'_>, cancellation: &CancellationToken) -> Result<(), String> {
        self.jobs
            .cancel_producers(ctx)
            .catch(ctx)
            .map_err(|error| error.to_string())?;
        let (detached, foreground) = self.jobs.completed_waits();
        let controls = serde_json::to_vec(&self.jobs.controls()).map_err(|e| e.to_string())?;
        let events =
            std::mem::take(&mut *self.jobs.events.lock().unwrap_or_else(|e| e.into_inner()));
        let events = serde_json::to_vec(&events.iter().map(Arc::as_ref).collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
        let error = |error: rquickjs::CaughtError<'_>| error.to_string();
        let binding: Object = ctx.globals().get(JOBS_GLOBAL).catch(ctx).map_err(error)?;
        let tick: Function = binding.get("tick").catch(ctx).map_err(error)?;
        let controls = ctx.json_parse(controls).catch(ctx).map_err(error)?;
        let events = ctx.json_parse(events).catch(ctx).map_err(error)?;
        let resolve: Function = binding.get("resolveWaits").catch(ctx).map_err(error)?;
        let resolve_waits = |replies: Vec<RegistryReply>| {
            if replies.is_empty() {
                return Ok(());
            }
            let replies = serde_json::to_vec(&replies).map_err(|e| e.to_string())?;
            let replies = ctx.json_parse(replies).catch(ctx).map_err(error)?;
            resolve.call::<_, ()>((replies,)).catch(ctx).map_err(error)
        };
        {
            let _scope = self.jobs.pump_scope();
            tick.call::<_, ()>((controls, events))
                .catch(ctx)
                .map_err(error)?;
            resolve_waits(detached)?;
            while !self.out_of_time(cancellation) && ctx.execute_pending_job() {}
            if self.out_of_time(cancellation) {
                return Err("Cordis job JavaScript timed out or was cancelled".into());
            }
            let end: Function = binding.get("endTick").catch(ctx).map_err(error)?;
            end.call::<_, ()>(()).catch(ctx).map_err(error)?;
        }
        resolve_waits(foreground)
    }
}
