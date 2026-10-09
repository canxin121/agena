#![allow(unused_imports)]

use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
    time::Duration,
};

use agena_plugin_sdk::CommandTarget;
use agena_storage::MemoryStore;
use tokio_util::sync::CancellationToken;

use crate::{
    AppError,
    authorization::ExecutionPrincipal,
    config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment},
    permission::{PermissionPolicy, ToolPermissionPolicy},
    session::SessionManager,
    tool::ToolExecutor,
};

use agena_runtime::{
    ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig, RuntimeBackgroundTask,
    RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin,
    RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskSpec, RuntimeBackgroundTaskStart,
    RuntimeCompositionConfig, RuntimeControlState, RuntimeReloadCause, RuntimeReloadReport,
    TaskControl,
};

use super::{RuntimeSnapshot, reload};

mod auth;
use auth::*;
mod provider_catalog;
use provider_catalog::*;

const PROVIDER_CLIENT_VERSION_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const PROVIDER_CLIENT_VERSION_SETTINGS_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const PROVIDER_CLIENT_VERSION_RETRY_BASE: Duration = Duration::from_secs(5 * 60);
const PROVIDER_CLIENT_VERSION_RETRY_MAX: Duration = Duration::from_secs(6 * 60 * 60);
const MODEL_CATALOG_REFRESH_CHECK_INTERVAL: Duration = Duration::from_secs(30);

fn provider_client_version_retry_delay(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(7);
    PROVIDER_CLIENT_VERSION_RETRY_BASE
        .saturating_mul(1_u32 << exponent)
        .min(PROVIDER_CLIENT_VERSION_RETRY_MAX)
}

fn provider_client_version_wait_duration(enabled: bool, until_refresh: Duration) -> Duration {
    if enabled {
        until_refresh.min(PROVIDER_CLIENT_VERSION_SETTINGS_POLL_INTERVAL)
    } else {
        PROVIDER_CLIENT_VERSION_SETTINGS_POLL_INTERVAL
    }
}

/// Compose the concrete Runtime from a schema-neutral process request and
/// return its stable application capability bundle.
pub async fn bootstrap_application_services(
    request: agena_runtime::RuntimeBootstrapRequest,
) -> Result<agena_runtime::RuntimeBootstrapResult, agena_runtime::RuntimeBootstrapError> {
    agena_runtime::ownership_audit::ensure_runtime_bootstrap_allowed()?;
    agena_runtime::compose_runtime_bootstrap(request, |config| async move {
        let runtime = AgenaRuntime::new(config)
            .await
            .map_err(runtime_bootstrap_error)?;
        Ok(agena_runtime::RuntimeBootstrapComposition::new(
            runtime.application_services(),
            runtime.clone() as Arc<dyn agena_runtime::RuntimeBootstrapLifecycle>,
        ))
    })
    .await
}

impl AgenaRuntime {
    /// Runtime-private composition entrypoint. External consumers use the
    /// stable `bootstrap_application_services` capability result instead.
    pub(crate) async fn new(config: RuntimeCompositionConfig) -> Result<Arc<Self>, AppError> {
        Self::new_with_maintenance(config, true).await
    }

    /// Isolated runtime tests can disable automatic maintenance while keeping
    /// ordinary lifecycle admission active.
    pub(super) async fn new_with_maintenance(
        config: RuntimeCompositionConfig,
        automatic_maintenance: bool,
    ) -> Result<Arc<Self>, AppError> {
        let mut config = config;
        let workspace_root = config.resolve_workspace_root()?;
        let RuntimeCompositionConfig {
            load_request,
            database_connection,
            database_url,
            database_path,
            scheduler_database_connection,
            scheduler_database_url,
            scheduler_database_path,
            initialize_schema,
            tracing_reload_handle,
            bootstrap_preflight,
            workspace_root: _,
        } = config;
        let loader = ConfigLoader::new(ProcessEnvironment);
        let initial_resolution = match &bootstrap_preflight {
            None => {
                let loader = loader.clone();
                let request = load_request.clone();
                Some(
                    crate::blocking::FILE_OPERATIONS
                        .run(move || loader.load(&request))
                        .await
                        .map_err(|error| {
                            AppError::Internal(format!("config load worker failed: {error}"))
                        })??,
                )
            }
            Some(_) => None,
        };
        let tracing = match (&bootstrap_preflight, &initial_resolution) {
            (Some(preflight), _) => &preflight.tracing,
            (None, Some(resolution)) => &resolution.config.tracing,
            (None, None) => {
                return Err(AppError::Internal(
                    "runtime bootstrap tracing preflight was not resolved".to_owned(),
                ));
            }
        };
        let chat_database_url = database_url.clone();
        let chat_database_path = database_path.clone();
        let database =
            agena_runtime::connect_runtime_database(agena_runtime::DatabaseCompositionInputs {
                database_connection,
                database_url,
                database_path,
                initialize_schema,
                tracing,
            })
            .await
            .map_err(runtime_database_error)?;

        // The scheduler owns a dedicated database, separate from the chat
        // database. When the caller (or the scheduler env vars) configure one
        // explicitly it is used as-is. Otherwise, when the chat database is
        // in-memory (tests, ephemeral deployments), the scheduler degrades to
        // its in-memory store instead of creating an unexpected file; a
        // file-backed chat database uses the conventional scheduler default.
        let scheduler_database = {
            let explicitly_configured = scheduler_database_url.is_some()
                || scheduler_database_path.is_some()
                || std::env::var("AGENA_SCHEDULER_DATABASE_URL").is_ok()
                || std::env::var("AGENA_SCHEDULER_DATABASE_PATH").is_ok();
            let chat_url =
                agena_runtime::resolve_runtime_database_url(chat_database_url, chat_database_path)
                    .map_err(agena_runtime::RuntimeDatabaseCompositionError::from)
                    .map_err(runtime_database_error)?;
            if !explicitly_configured && agena_runtime::is_in_memory_database(&chat_url) {
                None
            } else {
                agena_runtime::connect_scheduler_database(
                    scheduler_database_connection,
                    scheduler_database_url,
                    scheduler_database_path,
                    tracing,
                )
                .await
                .map_err(runtime_database_error)?
            }
        };

        // Publish directly to the bounded signal hub. Its synchronous clock
        // observers see every mutation; slow clients receive an explicit lag.
        let live_signals = Arc::new(agena_runtime::LiveSignalHub::new(256));
        let activity_registry =
            crate::activity::ActivityRegistry::with_signals(live_signals.clone());
        // Background-completion bridge: correlates launched-in-background
        // operations (monitored shells, delegated tasks) with their transcript
        // parts. The manager may not exist yet on a control-only bootstrap;
        // the bridge tolerates `None` and simply never terminalizes parts.
        let background_completion = crate::activity::BackgroundCompletionBridge::new(None);
        let monitor_registry = tokio::runtime::Handle::try_current().ok().map(|handle| {
            let registry = crate::MonitorRegistry::from_handle(handle);
            let bridge = crate::activity::MonitorActivityBridge {
                registry: activity_registry.clone(),
                on_finished: std::sync::Arc::new(std::sync::Mutex::new(Some(std::sync::Arc::new(
                    {
                        let completion = background_completion.clone();
                        move |summary: &agena_domain::ProcessSummary| {
                            completion.complete_shell(summary);
                        }
                    },
                )))),
                on_watch_changed: std::sync::Arc::new(std::sync::Mutex::new(Some(
                    std::sync::Arc::new({
                        let completion = background_completion.clone();
                        move |summary: &agena_domain::ProcessSummary| {
                            completion.update_shell_watch(summary);
                        }
                    }),
                ))),
                on_event: std::sync::Arc::new(std::sync::Mutex::new(Some(std::sync::Arc::new({
                    let completion = background_completion.clone();
                    move |event: &agena_domain::ProcessEvent,
                          summary: &agena_domain::ProcessSummary| {
                        completion.settle_monitor_event(event, summary);
                    }
                })))),
            };
            registry.with_monitor_listener(Arc::new(bridge))
        });
        let monitor_service =
            monitor_registry.map(|registry| Arc::new(registry) as Arc<dyn crate::MonitorService>);

        let initial_snapshot = Arc::new(
            RuntimeSnapshot::build(
                1,
                &loader,
                &load_request,
                workspace_root.as_path(),
                super::SnapshotDatabases {
                    chat: database.clone(),
                    scheduler: scheduler_database.clone(),
                },
                None,
                monitor_service.clone(),
            )
            .await?,
        );

        // Late-bind the session manager now that the snapshot exists; the
        // monitor bridge was assembled before it and only holds the bridge.
        background_completion.set_manager(initial_snapshot.session_manager());

        let runtime = AgenaRuntime {
            shared: Arc::new(AgenaRuntimeShared {
                inner: Arc::new(agena_runtime::RuntimeProcessState::new(
                    loader,
                    load_request,
                    workspace_root,
                    database,
                    scheduler_database,
                    RuntimeControlState::new(initial_snapshot.clone(), tracing_reload_handle),
                )),
                activities: Arc::new(crate::activity::ActivityRuntimeState::new(
                    activity_registry.clone(),
                    monitor_service,
                )),
                live_signals,
                subtask_bridge: Arc::new(std::sync::Mutex::new(None)),
                background_completion,
                automatic_maintenance,
            }),
        };

        // Project runtime maintenance tasks (marketplace sync, catalog
        // refresh, reload) into the unified activity registry.
        {
            let bridge = Arc::new(crate::activity::RuntimeTaskActivityBridge {
                registry: activity_registry.clone(),
            });
            runtime
                .inner
                .control_state
                .background_tasks()
                .set_listener(bridge);
        }

        // Drain activity records onto the runtime's ephemeral live-signal
        // stream. There is no persisted event bus: the drain emits observer
        // notifications only, never persisted, never replayed.
        let runtime = Arc::new(runtime);
        // Finish fallible publication preflight before starting maintenance
        // futures, which intentionally retain runtime handles until shutdown.
        let audit_workspace = runtime.inner.workspace_root.clone();
        crate::blocking::FILE_OPERATIONS
            .run(move || agena_runtime::ownership_audit::record_runtime_ownership(&audit_workspace))
            .await
            .map_err(|error| {
                AppError::Internal(format!("ownership audit worker failed: {error}"))
            })??;

        // The candidate client already serves generation configuration during
        // init. Runtime-dependent operations become available at publication.
        {
            let host_handle = initial_snapshot.plugin_manager().host_handle();
            initial_snapshot.host_client.bind_runtime(&runtime);
            super::host_client::install_plugin_host_event_publisher(host_handle, &runtime);
        }

        runtime.apply_tracing_filter(initial_snapshot.tracing_config());
        // Startup recovery of abandoned runs is a precondition, not a
        // background nicety: this process owns the data directory, so a run
        // marker still in flight can only belong to a process that is gone,
        // and every reader — overview, session list, session open — must
        // observe settled state. Await it before the runtime is published
        // (17.4).
        if let Some(manager) = initial_snapshot.session_manager()
            && let Err(error) = manager.reconcile_interrupted_executions().await
        {
            tracing::warn!(
                target: "agena_session",
                %error,
                "startup session-run reconciliation failed"
            );
        }
        if automatic_maintenance {
            runtime.spawn_background_tasks();
        }
        runtime.spawn_subtask_activity_bridge();
        runtime.start_scheduler_activity_bridge();
        runtime.spawn_background_delivery_recovery();
        agena_runtime::install_plugin_host(initial_snapshot.plugin_manager());
        Ok(runtime)
    }

    /// Background recovery for everything a prior process left in flight.
    ///
    /// Abandoned session runs are already reconciled in-line before
    /// publication; what is left is the notification handoffs — the durable
    /// delivery table is authoritative there — which may proceed without
    /// delaying runtime bootstrap.
    fn spawn_background_delivery_recovery(&self) {
        let Some(manager) = self.current_snapshot().session_manager() else {
            return;
        };
        tokio::spawn(async move {
            if let Err(error) = manager.reconcile_background_tasks(64).await {
                tracing::warn!(
                    target: "agena_background",
                    %error,
                    "startup background task reconciliation failed"
                );
            }
            if let Err(error) = manager.reconcile_background_processes(64).await {
                tracing::warn!(
                    target: "agena_background",
                    %error,
                    "startup background process reconciliation failed"
                );
            }
            if let Err(error) = manager.recover_background_deliveries(64).await {
                tracing::warn!(
                    target: "agena_background",
                    %error,
                    "startup background delivery recovery failed"
                );
            }
        });
    }

    /// Project delegated-task status into the unified background-activity
    /// registry. Subtask state is kept in the `sessions` row; the facade's
    /// [`SessionChange::SessionMetaUpdated`](agena_storage::SessionChange)
    /// notifications (emitted after commit, never persisted) drive this.
    fn spawn_subtask_activity_bridge(&self) {
        if self
            .subtask_bridge
            .lock()
            .expect("subtask bridge lock")
            .is_some()
        {
            return;
        }
        let Some(manager) = self.current_snapshot().session_manager() else {
            return;
        };
        let registry = self.activities.registry.clone();
        let store = manager.session_store();
        let completion_observer = self.background_completion.observer();
        let subscription = store.subscribe_all(Arc::new(move |change| {
            if let agena_storage::store::SessionChange::SessionMetaUpdated { meta, .. } = &change {
                crate::activity::upsert_task_activity_from_meta(&registry, meta);
            } else if match &change {
                agena_storage::store::SessionChange::PartAdded { part, .. }
                | agena_storage::store::SessionChange::PartUpdated { part, .. } => {
                    // Task log rows omit thinking and run-marker bodies.
                    // Their progress does not change an observable log value.
                    !matches!(part.kind.as_str(), "think" | "run")
                }
                agena_storage::store::SessionChange::PartRemoved { .. } => true,
                _ => false,
            } {
                registry.touch_task_logs(change.session_id());
            }
            completion_observer(change);
        }));
        // Retain the handle for the runtime's lifetime; dropping it would
        // unsubscribe (15.5).
        *self.subtask_bridge.lock().expect("subtask bridge lock") = Some(subscription);
    }

    /// Publish just the changed cron descriptor. Install before admission so
    /// startup recovery cannot finish without advancing the resource clocks.
    fn start_scheduler_activity_bridge(&self) {
        let Ok(scheduler) = activity_scheduler(self) else {
            return;
        };
        let registry = self.activities.registry.clone();
        scheduler.set_change_observer(Arc::new(move |change| {
            let (job, removed) = match change {
                agena_scheduler::SchedulerChange::Upsert(job) => (job, false),
                agena_scheduler::SchedulerChange::Removed(job) => (job, true),
            };
            let activity = scheduled_job_activity(job.clone(), cron_source_part_id(job));
            if removed {
                registry.dismiss_projected(activity);
            } else {
                registry.upsert(activity);
            }
        }));
        scheduler.start();
    }
}

#[cfg(test)]
mod provider_client_version_refresh_tests {
    use super::{
        Duration, PROVIDER_CLIENT_VERSION_RETRY_BASE, PROVIDER_CLIENT_VERSION_RETRY_MAX,
        PROVIDER_CLIENT_VERSION_SETTINGS_POLL_INTERVAL, provider_client_version_retry_delay,
        provider_client_version_wait_duration,
    };

    #[test]
    fn registry_failures_use_bounded_exponential_retry_delays() {
        assert_eq!(
            provider_client_version_retry_delay(1),
            PROVIDER_CLIENT_VERSION_RETRY_BASE
        );
        assert_eq!(
            provider_client_version_retry_delay(2),
            PROVIDER_CLIENT_VERSION_RETRY_BASE * 2
        );
        assert_eq!(
            provider_client_version_retry_delay(4),
            PROVIDER_CLIENT_VERSION_RETRY_BASE * 8
        );
        assert_eq!(
            provider_client_version_retry_delay(u32::MAX),
            PROVIDER_CLIENT_VERSION_RETRY_MAX
        );
    }

    #[test]
    fn disabled_auto_update_is_rechecked_before_the_daily_refresh_deadline() {
        assert_eq!(
            provider_client_version_wait_duration(false, Duration::from_secs(24 * 60 * 60)),
            PROVIDER_CLIENT_VERSION_SETTINGS_POLL_INTERVAL
        );
        assert_eq!(
            provider_client_version_wait_duration(true, Duration::from_secs(60)),
            Duration::from_secs(60)
        );
        assert_eq!(
            provider_client_version_wait_duration(true, Duration::ZERO),
            Duration::ZERO
        );
    }
}

#[cfg(test)]
mod durable_activity_projection_tests {
    use std::collections::BTreeMap;

    use agena_domain::{BackgroundActivityKind, BackgroundActivityStatus};
    use agena_storage::store::{
        BackgroundOperation, BackgroundOperationKind, BackgroundOperationPhase,
    };

    use super::{durable_operation_activity, scheduled_job_activity};

    #[test]
    fn monitor_projection_keeps_durable_session_and_source_part_identity() {
        let activity = durable_operation_activity(
            &BTreeMap::new(),
            BackgroundOperation {
                operation_id: "bg_58_42".to_owned(),
                session_id: 58,
                launch_run_id: Some(40),
                launch_tool_part_id: Some(42),
                kind: BackgroundOperationKind::Monitor,
                external_id: Some("proc_58_42".to_owned()),
                phase: BackgroundOperationPhase::Running,
                outcome: None,
                failure: None,
                last_event_seq: 3,
                owner_id: None,
                lease_until_ms: None,
                revision: 2,
                created_at_ms: 100,
                updated_at_ms: 120,
                finished_at_ms: None,
            },
        );
        assert_eq!(activity.kind, BackgroundActivityKind::Monitor);
        assert_eq!(activity.status, BackgroundActivityStatus::Running);
        assert_eq!(activity.session_id, Some(58));
        assert_eq!(activity.source_part_id, Some(42));
        assert_eq!(activity.operation_id.as_deref(), Some("bg_58_42"));
        assert_eq!(activity.last_seq, 3);
    }

    #[test]
    fn paused_cron_projection_remains_manageable_and_exposes_next_wake() {
        let mut job = agena_scheduler::ScheduledJob::new_cron_in_timezone(
            "*/10 * * * * *",
            "wake me",
            7,
            "Asia/Shanghai",
        )
        .expect("cron job");
        job.set_owner(58);
        job.pause();
        let next = job.next_fire_at.expect("next fire").timestamp_millis();
        let activity = scheduled_job_activity(job, Some(77));
        assert_eq!(activity.kind, BackgroundActivityKind::Cron);
        assert_eq!(activity.status, BackgroundActivityStatus::Paused);
        assert_eq!(activity.session_id, Some(58));
        assert_eq!(activity.source_part_id, Some(77));
        assert_eq!(activity.next_event_at_ms, Some(next));
        assert!(activity.is_active());
    }

    #[test]
    fn armed_cron_projection_waits_without_claiming_to_be_running() {
        let mut job = agena_scheduler::ScheduledJob::new_cron_in_timezone(
            "*/10 * * * * *",
            "wake me",
            7,
            "Asia/Shanghai",
        )
        .expect("cron job");
        job.set_owner(58);
        let next = job.next_fire_at.expect("next fire").timestamp_millis();

        let activity = scheduled_job_activity(job, Some(77));

        assert_eq!(activity.status, BackgroundActivityStatus::Waiting);
        assert_eq!(activity.next_event_at_ms, Some(next));
        assert!(activity.is_active());
        assert!(activity.cancellable);
        assert!(!activity.dismissible);
    }

    #[test]
    fn claimed_cron_projection_is_running_while_delivery_is_in_flight() {
        let mut job = agena_scheduler::ScheduledJob::new_cron_in_timezone(
            "*/10 * * * * *",
            "wake me",
            7,
            "Asia/Shanghai",
        )
        .expect("cron job");
        let now = chrono::Utc::now();
        job.next_fire_at = Some(now - chrono::Duration::seconds(1));
        let delivery = job.claim_due_delivery(now).expect("claim due delivery");
        assert!(matches!(
            delivery,
            agena_scheduler::ClaimDueDelivery::Deliver(_)
        ));

        let activity = scheduled_job_activity(job, Some(77));

        assert_eq!(activity.status, BackgroundActivityStatus::Running);
        assert_eq!(activity.next_event_at_ms, None);
    }

    #[test]
    fn cron_retry_backoff_projection_waits_until_retry_deadline() {
        let mut job = agena_scheduler::ScheduledJob::new_cron_in_timezone(
            "*/10 * * * * *",
            "wake me",
            7,
            "Asia/Shanghai",
        )
        .expect("cron job");
        let now = chrono::Utc::now();
        job.next_fire_at = Some(now - chrono::Duration::seconds(1));
        let delivery = job.claim_due_delivery(now).expect("claim due delivery");
        assert!(matches!(
            delivery,
            agena_scheduler::ClaimDueDelivery::Deliver(_)
        ));
        let retry_at = now + chrono::Duration::seconds(15);
        job.retry_at = Some(retry_at);

        let activity = scheduled_job_activity(job, Some(77));

        assert_eq!(activity.status, BackgroundActivityStatus::Waiting);
        assert_eq!(activity.next_event_at_ms, Some(retry_at.timestamp_millis()));
    }
}

fn durable_operation_activity(
    live: &BTreeMap<String, agena_domain::BackgroundActivity>,
    operation: agena_storage::store::BackgroundOperation,
) -> agena_domain::BackgroundActivity {
    use agena_domain::{BackgroundActivity, BackgroundActivityKind, BackgroundActivityStatus};
    use agena_storage::store::{BackgroundOperationKind, BackgroundOperationPhase};

    let external_id = operation
        .external_id
        .clone()
        .unwrap_or_else(|| operation.operation_id.clone());
    let activity_id = match operation.kind {
        BackgroundOperationKind::Task => format!("task_{external_id}"),
        _ => external_id.clone(),
    };
    let kind = match operation.kind {
        BackgroundOperationKind::Shell => BackgroundActivityKind::Shell,
        BackgroundOperationKind::Task => BackgroundActivityKind::Task,
        BackgroundOperationKind::Monitor => BackgroundActivityKind::Monitor,
        BackgroundOperationKind::ScheduledDelivery => BackgroundActivityKind::Cron,
    };
    let status = match operation.phase {
        BackgroundOperationPhase::LaunchRequested | BackgroundOperationPhase::Launching => {
            BackgroundActivityStatus::Pending
        }
        BackgroundOperationPhase::Running => BackgroundActivityStatus::Running,
        BackgroundOperationPhase::Completed => BackgroundActivityStatus::Succeeded,
        BackgroundOperationPhase::Failed | BackgroundOperationPhase::TimedOut => {
            BackgroundActivityStatus::Failed
        }
        BackgroundOperationPhase::Cancelled | BackgroundOperationPhase::Interrupted => {
            BackgroundActivityStatus::Cancelled
        }
    };
    let mut activity = live.get(&activity_id).cloned().unwrap_or_else(|| {
        let title = match kind {
            BackgroundActivityKind::Shell => format!("Background shell · {external_id}"),
            BackgroundActivityKind::Monitor => format!("Monitor · {external_id}"),
            BackgroundActivityKind::Task => format!("Delegated task · {external_id}"),
            BackgroundActivityKind::Cron => format!("Scheduled delivery · {external_id}"),
            BackgroundActivityKind::Runtime | BackgroundActivityKind::Browser => {
                external_id.clone()
            }
        };
        BackgroundActivity {
            id: activity_id.clone(),
            kind,
            status,
            title,
            description: external_id.clone(),
            command: None,
            workdir: None,
            session_id: (kind != BackgroundActivityKind::Task).then_some(operation.session_id),
            parent_session_id: (kind == BackgroundActivityKind::Task)
                .then_some(operation.session_id),
            operation_id: Some(operation.operation_id.clone()),
            source_part_id: operation.launch_tool_part_id,
            created_at_ms: operation.created_at_ms,
            started_at_ms: operation.created_at_ms,
            finished_at_ms: operation.finished_at_ms,
            next_event_at_ms: None,
            exit_code: None,
            message: None,
            failure: None,
            last_seq: operation.last_event_seq,
            has_more: false,
            dropped_lines: 0,
            cancellable: status.is_active(),
            dismissible: status.is_terminal(),
        }
    });
    activity.kind = kind;
    activity.status = status;
    activity.operation_id = Some(operation.operation_id);
    activity.source_part_id = operation.launch_tool_part_id;
    activity.created_at_ms = operation.created_at_ms;
    activity.started_at_ms = operation.created_at_ms;
    activity.finished_at_ms = operation.finished_at_ms;
    activity.last_seq = activity.last_seq.max(operation.last_event_seq);
    activity.cancellable = status.is_active();
    activity.dismissible = status.is_terminal();
    match kind {
        // Durable ownership fixes the process registry's intentionally
        // session-neutral shell record.
        BackgroundActivityKind::Shell | BackgroundActivityKind::Monitor => {
            activity.session_id = Some(operation.session_id);
        }
        // Task registry rows point at the child session; the operation owner
        // is the parent that launched it.
        BackgroundActivityKind::Task => {
            activity.parent_session_id = Some(operation.session_id);
        }
        _ => {}
    }
    if activity.failure.is_none()
        && let Some(failure) = operation.failure
    {
        match serde_json::from_value(failure) {
            Ok(failure) => activity.failure = Some(failure),
            Err(error) => tracing::warn!(
                operation_id = activity.operation_id.as_deref().unwrap_or("unknown"),
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode a persisted background-operation failure",
                    &error,
                ),
                "background activity failure projection is incomplete"
            ),
        }
    }
    activity
}

fn scheduled_job_activity(
    job: agena_scheduler::ScheduledJob,
    source_part_id: Option<i64>,
) -> agena_domain::BackgroundActivity {
    use agena_domain::{BackgroundActivity, BackgroundActivityKind, BackgroundActivityStatus};

    let (title, description) = match &job.kind {
        agena_scheduler::JobKind::Cron { expression, .. } => (
            format!("Cron · {expression} · {}", job.cron_timezone()),
            job.prompt.clone(),
        ),
        agena_scheduler::JobKind::Once { at } => (
            format!("Scheduled wake · {}", at.to_rfc3339()),
            job.prompt.clone(),
        ),
    };
    let status = if job.completed {
        match job.last_run.as_ref().map(|run| run.status) {
            Some(agena_scheduler::JobRunStatus::Failed) => BackgroundActivityStatus::Failed,
            _ => BackgroundActivityStatus::Succeeded,
        }
    } else if job.paused {
        BackgroundActivityStatus::Paused
    } else if job.pending_delivery.is_some() && job.retry_at.is_none() {
        BackgroundActivityStatus::Running
    } else {
        BackgroundActivityStatus::Waiting
    };
    let failure = job
        .last_run
        .as_ref()
        .and_then(|run| run.failure.as_ref())
        .map(Into::into);
    BackgroundActivity {
        id: format!("cron_{}", job.id),
        kind: BackgroundActivityKind::Cron,
        status,
        title,
        description,
        command: None,
        workdir: None,
        session_id: job.owner_session_id,
        parent_session_id: None,
        operation_id: None,
        source_part_id,
        created_at_ms: job.created_at.timestamp_millis(),
        started_at_ms: job.created_at.timestamp_millis(),
        finished_at_ms: job.completed.then(|| {
            job.last_run.as_ref().map_or_else(
                || job.created_at.timestamp_millis(),
                |run| run.finished_at.timestamp_millis(),
            )
        }),
        next_event_at_ms: if status == BackgroundActivityStatus::Running {
            None
        } else {
            job.retry_at
                .or(job.next_fire_at)
                .map(|time| time.timestamp_millis())
        },
        exit_code: None,
        message: job.paused.then_some("Paused".to_owned()),
        failure,
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: status.is_active(),
        dismissible: status.is_terminal(),
    }
}

fn cron_source_part_id(job: &agena_scheduler::ScheduledJob) -> Option<i64> {
    job.launch_provenance
        .map(|provenance| provenance.tool_part_id)
}

fn cron_activity_job_id(
    activity_id: &str,
) -> Result<uuid::Uuid, agena_runtime::ActivityControlError> {
    activity_id
        .strip_prefix("cron_")
        .ok_or_else(|| agena_runtime::ActivityControlError::not_stoppable(activity_id))
        .and_then(|id| {
            uuid::Uuid::parse_str(id)
                .map_err(|_| agena_runtime::ActivityControlError::not_found(activity_id))
        })
}

fn activity_scheduler(
    runtime: &AgenaRuntime,
) -> Result<Arc<agena_scheduler::Scheduler>, agena_runtime::ActivityControlError> {
    let manager = runtime
        .current_snapshot()
        .session_manager()
        .ok_or_else(|| {
            agena_runtime::ActivityControlError::internal("session runtime unavailable")
        })?;
    manager
        .tool_executor()
        .scheduler()
        .cloned()
        .ok_or_else(|| agena_runtime::ActivityControlError::internal("scheduler unavailable"))
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeActivityService for AgenaRuntime {
    async fn list_activities(
        &self,
        filter: &agena_domain::BackgroundActivityFilter,
    ) -> Result<Vec<agena_domain::BackgroundActivity>, agena_runtime::ActivityControlError> {
        let mut projected = self
            .activities
            .registry
            .list(filter)
            .into_iter()
            .map(|activity| (activity.id.clone(), activity))
            .collect::<BTreeMap<_, _>>();

        if let Some(manager) = self.current_snapshot().session_manager() {
            let session_store = manager.session_store();
            // A dropped terminal observer must not leave an in-memory Running
            // row visible after the durable aggregate has already settled.
            // Resolve every registry-backed durable kind first, including
            // terminal operations, then add active aggregates that have no
            // process-local detail (for example after restart).
            let include_durable = filter.kinds.is_empty()
                || filter.kinds.iter().any(|kind| {
                    matches!(
                        kind,
                        agena_domain::BackgroundActivityKind::Shell
                            | agena_domain::BackgroundActivityKind::Monitor
                            | agena_domain::BackgroundActivityKind::Task
                    )
                });
            if include_durable {
                for live_activity in projected.values().cloned().collect::<Vec<_>>() {
                    if let Some(operation) =
                        durable_operation_for_live_activity(session_store.as_ref(), &live_activity)
                            .await?
                    {
                        let activity = durable_operation_activity(&projected, operation);
                        projected.insert(activity.id.clone(), activity);
                    }
                }
                let operations = if let Some(session_id) = filter.session_id {
                    session_store
                        .active_background_operations_for_session(session_id, None, 4_096)
                        .await
                } else {
                    session_store
                        .active_background_operations(None, 4_096)
                        .await
                }
                .map_err(|error| {
                    agena_runtime::ActivityControlError::internal(format!(
                        "load durable background operations: {error}"
                    ))
                })?;
                for operation in operations {
                    let activity = durable_operation_activity(&projected, operation);
                    projected.insert(activity.id.clone(), activity);
                }
            }

            if (filter.kinds.is_empty()
                || filter
                    .kinds
                    .contains(&agena_domain::BackgroundActivityKind::Cron))
                && let Some(scheduler) = manager.tool_executor().scheduler().cloned()
            {
                // Persistent schedules are authoritative even after another
                // process removed a row that remains in this live cache.
                projected.retain(|_, activity| {
                    activity.kind != agena_domain::BackgroundActivityKind::Cron
                });
                for job in scheduler
                    .list_filtered(filter.session_id, filter.active_only)
                    .await
                    .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?
                {
                    if filter.active_only && job.completed {
                        continue;
                    }
                    if filter
                        .session_id
                        .is_some_and(|session_id| job.owner_session_id != Some(session_id))
                    {
                        continue;
                    }
                    let source_part_id = cron_source_part_id(&job);
                    let activity = scheduled_job_activity(job, source_part_id);
                    projected.insert(activity.id.clone(), activity);
                }
            }
        }

        let mut activities = projected
            .into_values()
            .filter(|activity| filter.matches(activity))
            .collect::<Vec<_>>();
        activities.sort_by(|left, right| {
            right
                .is_active()
                .cmp(&left.is_active())
                .then_with(|| right.started_at_ms.cmp(&left.started_at_ms))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(activities)
    }

    async fn get_activity(
        &self,
        activity_id: &str,
    ) -> Result<agena_domain::BackgroundActivity, agena_runtime::ActivityControlError> {
        let live = self.activities.registry.get(activity_id);
        let snapshot = self.current_snapshot();
        if let Some(manager) = snapshot.session_manager() {
            if let Some(job_id) = activity_id
                .strip_prefix("cron_")
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                && let Some(scheduler) = manager.tool_executor().scheduler()
            {
                let job = scheduler
                    .get(job_id)
                    .await
                    .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?
                    .ok_or_else(|| agena_runtime::ActivityControlError::not_found(activity_id))?;
                let source = cron_source_part_id(&job);
                return Ok(scheduled_job_activity(job, source));
            }
            let store = manager.session_store();
            if let Some(activity) = &live {
                if let Some(operation) =
                    durable_operation_for_live_activity(store.as_ref(), activity).await?
                {
                    return Ok(durable_operation_activity(
                        &BTreeMap::from([(activity.id.clone(), activity.clone())]),
                        operation,
                    ));
                }
            } else {
                use agena_storage::store::BackgroundOperationKind;
                let (external_id, kinds): (&str, &[BackgroundOperationKind]) =
                    if let Some(task) = activity_id.strip_prefix("task_") {
                        (task, &[BackgroundOperationKind::Task])
                    } else {
                        (
                            activity_id,
                            &[
                                BackgroundOperationKind::Shell,
                                BackgroundOperationKind::Monitor,
                                BackgroundOperationKind::ScheduledDelivery,
                            ],
                        )
                    };
                for kind in kinds {
                    if let Some(operation) = store
                        .background_operation_by_external_id(*kind, external_id)
                        .await
                        .map_err(|error| {
                            agena_runtime::ActivityControlError::internal_error(&error)
                        })?
                    {
                        return Ok(durable_operation_activity(&BTreeMap::new(), operation));
                    }
                }
            }
        }
        live.ok_or_else(|| agena_runtime::ActivityControlError::not_found(activity_id))
    }

    async fn activity_logs(
        &self,
        activity_id: &str,
        since_seq: u64,
        limit: Option<u32>,
        wait_ms: u64,
    ) -> Result<agena_domain::BackgroundActivityLogRead, agena_runtime::ActivityControlError> {
        let activity = self.get_activity(activity_id).await?;
        // Plugin-registered activity sources own their log stream; dispatch
        // to them before any built-in behavior.
        if let Some(adapter) = self.activities.source_for(activity.kind) {
            return adapter
                .read_logs(activity_id, since_seq, limit, wait_ms)
                .await
                .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error));
        }
        match activity.kind {
            agena_domain::BackgroundActivityKind::Shell
            | agena_domain::BackgroundActivityKind::Monitor => {
                let Some(monitor) = self.activities.monitor.as_ref() else {
                    return Err(agena_runtime::ActivityControlError::no_log_source(
                        activity_id,
                    ));
                };
                let monitor = Arc::clone(monitor);
                let activity_id = activity_id.to_string();
                crate::blocking::PROCESS_LOG_READS
                    .run(move || {
                        crate::activity::read_shell_logs(
                            monitor.as_ref(),
                            activity_id.as_str(),
                            since_seq,
                            limit,
                            wait_ms,
                        )
                    })
                    .await
                    .map_err(|error| {
                        agena_runtime::ActivityControlError::internal(format!(
                            "shell log worker failed: {error}"
                        ))
                    })?
                    .map_err(|err| agena_runtime::ActivityControlError::internal_error(&err))
            }
            agena_domain::BackgroundActivityKind::Task => {
                let (parent_session_id, task_id) = activity
                    .parent_session_id
                    .zip(activity_id.strip_prefix("task_"))
                    .ok_or_else(|| {
                        agena_runtime::ActivityControlError::no_log_source(activity_id)
                    })?;
                let mut logs = crate::activity::read_task_logs(
                    self.current_snapshot().session_manager().as_ref(),
                    parent_session_id,
                    task_id,
                    since_seq as i64,
                )
                .await;
                logs.status = activity.status;
                Ok(logs)
            }
            // Kinds without a registered source adapter (runtime maintenance
            // tasks today, browser sessions before the web plugin registers
            // its adapter) have no incremental log stream.
            agena_domain::BackgroundActivityKind::Cron
            | agena_domain::BackgroundActivityKind::Runtime
            | agena_domain::BackgroundActivityKind::Browser => {
                Ok(agena_domain::BackgroundActivityLogRead {
                    activity_id: activity_id.to_string(),
                    status: activity.status,
                    lines: Vec::new(),
                    last_seq: 0,
                    has_more: false,
                    dropped_lines: 0,
                    exit_code: activity.exit_code,
                    completion_reason: activity.message.clone(),
                })
            }
        }
    }

    async fn stop_activity(
        &self,
        activity_id: &str,
    ) -> Result<agena_domain::BackgroundActivity, agena_runtime::ActivityControlError> {
        let activity = self.get_activity(activity_id).await?;
        if activity.kind == agena_domain::BackgroundActivityKind::Cron {
            return self.pause_activity(activity_id).await;
        }
        if !activity.is_active() {
            return Err(agena_runtime::ActivityControlError::not_running(
                activity_id,
            ));
        }
        if !activity.cancellable {
            return Err(agena_runtime::ActivityControlError::not_stoppable(
                activity_id,
            ));
        }
        // Plugin-registered activity sources implement stop themselves (e.g.
        // the web plugin closes the matching browser session); the adapter
        // publishes the terminal record so the refreshed activity reflects it.
        if let Some(adapter) = self.activities.source_for(activity.kind) {
            adapter
                .stop(activity_id)
                .await
                .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?;
            return self.get_activity(activity_id).await;
        }
        match activity.kind {
            agena_domain::BackgroundActivityKind::Shell
            | agena_domain::BackgroundActivityKind::Monitor => {
                let Some(monitor) = self.activities.monitor.as_ref() else {
                    return Err(agena_runtime::ActivityControlError::not_stoppable(
                        activity_id,
                    ));
                };
                monitor
                    .stop(activity_id)
                    .map_err(|err| agena_runtime::ActivityControlError::internal_error(&err))?;
            }
            agena_domain::BackgroundActivityKind::Runtime => {
                self.inner
                    .control_state
                    .background_tasks()
                    .cancel(activity_id)
                    .map_err(|err| agena_runtime::ActivityControlError::internal_error(&err))?;
            }
            agena_domain::BackgroundActivityKind::Task => {
                let Some((parent_session_id, task_id)) = activity
                    .parent_session_id
                    .zip(activity_id.strip_prefix("task_"))
                else {
                    return Err(agena_runtime::ActivityControlError::not_stoppable(
                        activity_id,
                    ));
                };
                let Some(manager) = self.current_snapshot().session_manager() else {
                    return Err(agena_runtime::ActivityControlError::not_stoppable(
                        activity_id,
                    ));
                };
                manager
                    .cancel_subtask(parent_session_id, task_id)
                    .await
                    .map_err(|err| agena_runtime::ActivityControlError::internal_error(&err))?;
            }
            // Without a registered source adapter there is nothing to stop;
            // the record's `cancellable` flag is the source's responsibility.
            agena_domain::BackgroundActivityKind::Cron
            | agena_domain::BackgroundActivityKind::Browser => {
                return Err(agena_runtime::ActivityControlError::not_stoppable(
                    activity_id,
                ));
            }
        }
        self.get_activity(activity_id).await
    }

    async fn pause_activity(
        &self,
        activity_id: &str,
    ) -> Result<agena_domain::BackgroundActivity, agena_runtime::ActivityControlError> {
        let scheduler = activity_scheduler(self)?;
        let job_id = cron_activity_job_id(activity_id)?;
        let job = scheduler
            .pause(job_id)
            .await
            .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?
            .ok_or_else(|| agena_runtime::ActivityControlError::not_found(activity_id))?;
        let source_part_id = cron_source_part_id(&job);
        let activity = scheduled_job_activity(job, source_part_id);
        Ok(activity)
    }

    async fn resume_activity(
        &self,
        activity_id: &str,
    ) -> Result<agena_domain::BackgroundActivity, agena_runtime::ActivityControlError> {
        let scheduler = activity_scheduler(self)?;
        let job_id = cron_activity_job_id(activity_id)?;
        let job = scheduler
            .resume(job_id)
            .await
            .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?
            .ok_or_else(|| agena_runtime::ActivityControlError::not_found(activity_id))?;
        let source_part_id = cron_source_part_id(&job);
        let activity = scheduled_job_activity(job, source_part_id);
        Ok(activity)
    }

    async fn delete_activity(
        &self,
        activity_id: &str,
    ) -> Result<agena_domain::BackgroundActivity, agena_runtime::ActivityControlError> {
        let mut activity = self.get_activity(activity_id).await?;
        let scheduler = activity_scheduler(self)?;
        let job_id = cron_activity_job_id(activity_id)?;
        if !scheduler
            .remove(job_id)
            .await
            .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?
        {
            return Err(agena_runtime::ActivityControlError::not_found(activity_id));
        }
        activity.status = agena_domain::BackgroundActivityStatus::Stopped;
        activity.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
        activity.next_event_at_ms = None;
        activity.cancellable = false;
        activity.dismissible = true;
        activity.message = Some("Deleted".to_owned());
        Ok(activity)
    }

    fn dismiss_activity(
        &self,
        activity_id: &str,
    ) -> Result<agena_domain::BackgroundActivity, agena_runtime::ActivityControlError> {
        self.activities
            .registry
            .dismiss(activity_id)
            .ok_or_else(|| agena_runtime::ActivityControlError::not_found(activity_id))
    }

    async fn clear_finished(&self) -> Result<usize, agena_runtime::ActivityControlError> {
        let mut removed = 0;
        if let Ok(scheduler) = activity_scheduler(self) {
            for job in scheduler
                .list()
                .await
                .map_err(|error| agena_runtime::ActivityControlError::internal_error(&error))?
            {
                if job.completed
                    && scheduler.remove(job.id).await.map_err(|error| {
                        agena_runtime::ActivityControlError::internal_error(&error)
                    })?
                {
                    removed += 1;
                }
            }
        }
        removed += self.activities.registry.clear_finished().len();
        Ok(removed)
    }
}

async fn durable_operation_for_live_activity(
    store: &dyn agena_storage::store::SessionStore,
    activity: &agena_domain::BackgroundActivity,
) -> Result<Option<agena_storage::store::BackgroundOperation>, agena_runtime::ActivityControlError>
{
    use agena_domain::BackgroundActivityKind;
    use agena_storage::store::BackgroundOperationKind;

    let query = |kind, external_id: &str| {
        let external_id = external_id.to_owned();
        async move {
            store
                .background_operation_by_external_id(kind, &external_id)
                .await
                .map_err(|error| {
                    agena_runtime::ActivityControlError::internal(format!(
                        "resolve durable background activity {}: {error}",
                        activity.id
                    ))
                })
        }
    };
    match activity.kind {
        BackgroundActivityKind::Shell => {
            if let Some(operation) = query(BackgroundOperationKind::Shell, &activity.id).await? {
                return Ok(Some(operation));
            }
            query(BackgroundOperationKind::Monitor, &activity.id).await
        }
        BackgroundActivityKind::Monitor => {
            query(BackgroundOperationKind::Monitor, &activity.id).await
        }
        BackgroundActivityKind::Task => {
            let Some(task_id) = activity.id.strip_prefix("task_") else {
                return Ok(None);
            };
            query(BackgroundOperationKind::Task, task_id).await
        }
        BackgroundActivityKind::Cron
        | BackgroundActivityKind::Runtime
        | BackgroundActivityKind::Browser => Ok(None),
    }
}

impl agena_runtime::ModelCatalogRuntimeService for AgenaRuntime {
    fn model_catalog_response(&self) -> agena_provider::ModelCatalogResponse {
        self.current_snapshot().model_catalog_response()
    }

    fn model_catalog_refresh_active(&self) -> bool {
        AgenaRuntime::model_catalog_refresh_active(self)
    }

    fn start_model_catalog_refresh(
        &self,
        origin: agena_runtime::RuntimeBackgroundTaskOrigin,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTaskStart,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::start_model_catalog_refresh(self, origin)
    }
}

#[async_trait::async_trait]
impl agena_runtime::PluginRuntimeService for AgenaRuntime {
    fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus> {
        self.current_snapshot().plugin_manager().plugin_statuses()
    }

    fn plugin_status(&self, plugin_id: &str) -> Option<agena_plugin_host::status::PluginStatus> {
        self.current_snapshot()
            .plugin_manager()
            .plugin_status(plugin_id)
    }

    fn plugin_surface_catalog(&self) -> agena_plugin_host::PluginSurfaceCatalog {
        self.current_snapshot().plugin_manager().surface_catalog()
    }

    fn plugin_architecture_catalog(&self) -> agena_plugin_host::PluginArchitectureCatalog {
        self.current_snapshot()
            .plugin_manager()
            .architecture_catalog()
    }

    fn permission_tool_catalog(&self) -> Vec<agena_runtime::RuntimePluginToolCatalogItem> {
        let mut tools = self
            .current_snapshot()
            .plugin_manager()
            .registered_tools()
            .into_iter()
            .map(|tool| {
                let mut tags = tool
                    .effective_tags()
                    .into_iter()
                    .map(|tag| tag.as_ref().to_string())
                    .collect::<Vec<_>>();
                tags.sort();
                tags.dedup();
                agena_runtime::RuntimePluginToolCatalogItem {
                    name: tool.canonical_name(),
                    summary: tool.summary_text().unwrap_or_default().to_string(),
                    tags,
                }
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools.dedup_by(|left, right| left.name == right.name);
        tools
    }

    fn display_contributions(&self) -> Vec<agena_plugin_host::HostDisplayContribution> {
        self.current_snapshot()
            .plugin_manager()
            .display_contributions()
    }

    fn host_notifications(&self) -> Vec<agena_plugin_host::HostNotification> {
        self.current_snapshot()
            .plugin_manager()
            .host_notifications()
    }

    fn theme_palettes(&self) -> Vec<agena_plugin_host::HostThemePalette> {
        self.current_snapshot().plugin_manager().theme_palettes()
    }

    fn command_catalog(&self) -> Vec<agena_plugin_host::CommandCatalogItem> {
        self.current_snapshot().plugin_manager().command_catalog()
    }

    fn tool_registry_generation(&self) -> u64 {
        self.current_snapshot()
            .plugin_manager()
            .tool_registry_generation()
    }

    fn projection_generation(&self) -> (u64, u64) {
        let snapshot = self.current_snapshot();
        (
            snapshot.generation(),
            snapshot.plugin_manager().tool_registry_generation(),
        )
    }

    fn tool_registry_events_since(
        &self,
        after_generation: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent> {
        self.current_snapshot()
            .plugin_manager()
            .tool_registry_events_since(after_generation, limit)
    }

    fn plugin_inspect(&self, plugin_id: &str) -> Option<agena_plugin_host::PluginInspect> {
        self.current_snapshot()
            .plugin_manager()
            .plugin_inspect(plugin_id)
    }

    fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::PluginLogRecord> {
        self.current_snapshot()
            .plugin_manager()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    fn resolve_plugin_tool(
        &self,
        plugin_id: Option<&str>,
        tool_name: &str,
    ) -> Option<agena_runtime::PluginToolDescriptor> {
        let host = self.current_snapshot().plugin_manager();
        let entry = match plugin_id {
            Some(plugin_id) => host.resolve_registered_tool_for_plugin_tool(plugin_id, tool_name),
            None => host.lookup_tool_for_name(tool_name),
        }?;
        Some(agena_runtime::PluginToolDescriptor {
            canonical_name: entry.canonical_name(),
            plugin_full_name: entry.plugin_full_name(),
            plugin_id: entry.plugin_key().clone(),
        })
    }

    async fn invoke_plugin_command(
        &self,
        plugin_id: &str,
        input: agena_plugin_host::sdk::CommandInvokeInput,
    ) -> Result<agena_plugin_host::sdk::CommandResult, String> {
        let host = self.current_snapshot().plugin_manager();
        host.invoke_plugin_command_async(plugin_id, input)
            .await
            .map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("plugin operation `{plugin_id}` failed"),
                    &error,
                )
            })
    }

    async fn plugin_rpc(
        &self,
        plugin_id: &str,
        callback_token: Option<String>,
        request: agena_plugin_host::sdk::rpc::Request,
    ) -> Result<agena_plugin_host::sdk::rpc::Response, agena_runtime::PluginRuntimeRpcError> {
        agena_runtime::dispatch_plugin_rpc(
            self.current_snapshot().plugin_manager(),
            plugin_id,
            callback_token,
            request,
        )
        .await
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeToolExecutionService for AgenaRuntime {
    async fn available_runtime_tools(&self) -> Vec<agena_runtime::RuntimeToolDescriptor> {
        let tools = self
            .runtime_tool_executor()
            .available_execution_tools_async()
            .await;
        let names = crate::tool::execution_tool_names(&tools);
        tools
            .into_iter()
            .zip(names)
            .map(|(tool, name)| {
                let tags = tool.effective_tags();
                let interactive = agena_plugin_host::sdk::ToolTag::is_interactive(&tags);
                let read_only = agena_plugin_host::sdk::ToolTag::is_read_only(&tags);
                let open_world = tags.iter().any(|tag| {
                    matches!(
                        tag,
                        agena_plugin_host::sdk::ToolTag::Network
                            | agena_plugin_host::sdk::ToolTag::Fetch
                    )
                });
                let output_schema = tool.output_schema();
                let output_schema = (!output_schema.is_null()).then_some(output_schema);
                agena_runtime::RuntimeToolDescriptor {
                    name,
                    summary: tool.summary_text().map(ToOwned::to_owned),
                    before_help: tool.before_help_text().map(ToOwned::to_owned),
                    after_help: tool.after_help_text().map(ToOwned::to_owned),
                    input_schema: tool.input_schema(),
                    output_schema,
                    interactive,
                    read_only,
                    destructive: tags.iter().any(|tag| {
                        matches!(
                            tag,
                            agena_plugin_host::sdk::ToolTag::Shell
                                | agena_plugin_host::sdk::ToolTag::Mutate
                                | agena_plugin_host::sdk::ToolTag::Execute
                        )
                    }),
                    open_world,
                    task: agena_plugin_host::sdk::ToolTag::is_task(&tags),
                    plugin_id: tool.plugin_full_name(),
                }
            })
            .collect()
    }

    async fn available_tool_api_definitions(&self) -> Vec<agena_provider::ToolApiDefinition> {
        self.runtime_tool_executor()
            .available_tool_api_bindings_async()
            .await
            .into_iter()
            .map(|binding| binding.definition())
            .collect()
    }

    async fn execute_runtime_tool(
        &self,
        invocation: &agena_domain::ToolInvocation,
        call_id: i64,
    ) -> Result<agena_runtime::SessionToolExecutionOutcome, agena_runtime::RuntimeToolExecutionError>
    {
        let manager = self.current_snapshot().session_manager().ok_or_else(|| {
            agena_runtime::RuntimeToolExecutionError::new(
                "session manager is unavailable for authorization",
            )
        })?;
        manager
            .execute_unscoped_tool(invocation.clone(), call_id)
            .await
            .map_err(|error| agena_runtime::RuntimeToolExecutionError::from_error(&error))
    }

    async fn render_tool_result(
        &self,
        invocation: &agena_domain::ToolInvocation,
        output: &agena_domain::RawOutput,
    ) -> agena_runtime::RuntimeToolResultProjection {
        let rendered = self
            .runtime_tool_executor()
            .render_tool_result(invocation, output)
            .await;
        let human = rendered.human.unwrap_or_default();
        agena_runtime::RuntimeToolResultProjection {
            model: rendered.model.unwrap_or_default(),
            human: agena_runtime::RuntimeToolHumanPresentation {
                title: human.title,
                summary: human.summary,
                blocks: human.blocks,
            },
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeAuthenticationService for AgenaRuntime {
    fn auth_providers(
        &self,
    ) -> Result<Vec<agena_runtime::RuntimeAuthProvider>, agena_runtime::RuntimeAuthenticationError>
    {
        self.current_snapshot()
            .provider_configs()
            .iter()
            .filter(|(_, resolved)| auth_provider_is_configured(resolved))
            .map(|(provider_id, resolved)| auth_provider_projection(provider_id, resolved))
            .collect()
    }

    fn auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<agena_runtime::RuntimeAuthProvider, agena_runtime::RuntimeAuthenticationError> {
        let resolved = auth_resolved_provider(self, provider_id)?;
        auth_provider_projection(provider_id, &resolved)
    }

    fn set_auth_api_key(
        &self,
        provider_id: &str,
        api_key: String,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        let resolved = auth_resolved_provider(self, provider_id)?;
        if !crate::config::provider_supports_api_key_write(&resolved) {
            return Err(auth_bad_request(format!(
                "{provider_id} does not support api key login"
            )));
        }
        auth_manager_for_runtime(self)
            .set_api_key(provider_id, api_key)
            .map_err(auth_internal)
    }

    async fn start_auth_browser(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        redirect_uri: String,
    ) -> Result<agena_runtime::RuntimeAuthBrowserStart, agena_runtime::RuntimeAuthenticationError>
    {
        let target = auth_oauth_target(self, provider_id, AuthOAuthPurpose::BrowserLogin)?;
        let manager = auth_manager_for_runtime(self);
        match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderOAuthTarget::OpenAi,
            ) => {
                let start = manager
                    .start_openai_browser_login(redirect_uri)
                    .map_err(auth_internal)?;
                Ok(agena_runtime::RuntimeAuthBrowserStart {
                    instance_url: None,
                    authorize_url: start.authorize_url,
                    state: start.state,
                    pkce_verifier: start.pkce_verifier,
                })
            }
            (
                agena_runtime::RuntimeAuthLoginKind::Gitlab,
                crate::config::ProviderOAuthTarget::Gitlab { instance_url },
            ) => {
                let start = manager
                    .start_gitlab_login(instance_url.clone(), redirect_uri)
                    .map_err(auth_internal)?;
                Ok(agena_runtime::RuntimeAuthBrowserStart {
                    instance_url: Some(instance_url),
                    authorize_url: start.authorize_url,
                    state: start.state,
                    pkce_verifier: start.pkce_verifier,
                })
            }
            (kind, _) => Err(auth_bad_request(format!(
                "{provider_id} does not support {} browser login",
                auth_kind_name(kind)
            ))),
        }
    }

    async fn wait_auth_browser_callback(
        &self,
        port: u16,
        expected_state: &str,
        timeout: std::time::Duration,
    ) -> Result<agena_provider::OAuthCallback, agena_runtime::RuntimeAuthenticationError> {
        agena_runtime::wait_for_oauth_callback_async(port, expected_state, timeout)
            .await
            .map_err(|error| {
                agena_runtime::RuntimeAuthenticationError::bad_request(error.to_string())
            })
    }

    async fn finish_auth_browser(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        let target = auth_oauth_target(self, provider_id, AuthOAuthPurpose::BrowserLogin)?;
        let manager = auth_manager_for_runtime(self);
        match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderOAuthTarget::OpenAi,
            ) => manager
                .finish_openai_browser_login(provider_id, code, pkce_verifier, redirect_uri)
                .await
                .map(|_| ())
                .map_err(auth_internal),
            (
                agena_runtime::RuntimeAuthLoginKind::Gitlab,
                crate::config::ProviderOAuthTarget::Gitlab { instance_url },
            ) => manager
                .finish_gitlab_login(provider_id, instance_url, code, pkce_verifier, redirect_uri)
                .await
                .map(|_| ())
                .map_err(auth_internal),
            (kind, _) => Err(auth_bad_request(format!(
                "{provider_id} does not support {} browser login",
                auth_kind_name(kind)
            ))),
        }
    }

    async fn start_auth_device(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        enterprise_domain: Option<String>,
    ) -> Result<agena_runtime::RuntimeAuthDeviceStart, agena_runtime::RuntimeAuthenticationError>
    {
        let target = auth_device_target(self, provider_id)?;
        let manager = auth_manager_for_runtime(self);
        let start = match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderDeviceAuthTarget::OpenAi,
            ) => manager
                .start_openai_headless_login()
                .await
                .map_err(auth_internal)?,
            (
                agena_runtime::RuntimeAuthLoginKind::GithubCopilot,
                crate::config::ProviderDeviceAuthTarget::Copilot,
            ) => manager
                .start_copilot_login(auth_copilot_deployment(enterprise_domain))
                .await
                .map_err(auth_internal)?,
            (kind, _) => {
                return Err(auth_bad_request(format!(
                    "{provider_id} does not support {} device login",
                    auth_kind_name(kind)
                )));
            }
        };
        Ok(agena_runtime::RuntimeAuthDeviceStart {
            verification_url: start.verification_url,
            user_code: start.user_code,
            device_code: start.device_code,
            interval_seconds: start.interval_seconds,
        })
    }

    async fn poll_auth_device(
        &self,
        provider_id: &str,
        kind: agena_runtime::RuntimeAuthLoginKind,
        device_code: String,
        user_code: Option<String>,
        enterprise_domain: Option<String>,
    ) -> Result<bool, agena_runtime::RuntimeAuthenticationError> {
        let target = auth_device_target(self, provider_id)?;
        let manager = auth_manager_for_runtime(self);
        match (kind, target) {
            (
                agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
                crate::config::ProviderDeviceAuthTarget::OpenAi,
            ) => manager
                .poll_openai_headless_login(provider_id, device_code, user_code.unwrap_or_default())
                .await
                .map(|result| result.is_some())
                .map_err(auth_internal),
            (
                agena_runtime::RuntimeAuthLoginKind::GithubCopilot,
                crate::config::ProviderDeviceAuthTarget::Copilot,
            ) => manager
                .poll_copilot_login(
                    provider_id,
                    device_code,
                    auth_copilot_deployment(enterprise_domain),
                )
                .await
                .map(|result| result.is_some())
                .map_err(auth_internal),
            (kind, _) => Err(auth_bad_request(format!(
                "{provider_id} does not support {} device login",
                auth_kind_name(kind)
            ))),
        }
    }

    fn remove_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        auth_resolved_provider(self, provider_id)?;
        auth_manager_for_runtime(self)
            .remove(provider_id)
            .map_err(auth_internal)
    }

    async fn refresh_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<(), agena_runtime::RuntimeAuthenticationError> {
        let target = auth_oauth_target(self, provider_id, AuthOAuthPurpose::CredentialRefresh)?;
        let manager = auth_manager_for_runtime(self);
        match target {
            crate::config::ProviderOAuthTarget::OpenAi => manager
                .refresh_openai_login(provider_id)
                .await
                .map(|_| ())
                .map_err(auth_internal),
            crate::config::ProviderOAuthTarget::Gitlab { instance_url } => manager
                .refresh_gitlab_login(provider_id, instance_url)
                .await
                .map(|_| ())
                .map_err(auth_internal),
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeDraftAuthenticationService for AgenaRuntime {
    fn start_draft_auth_browser(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        instance_url: Option<String>,
        redirect_uri: String,
    ) -> Result<
        agena_runtime::RuntimeDraftAuthBrowserStart,
        agena_runtime::RuntimeAuthenticationError,
    > {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                agena_runtime::start_openai_draft_auth_browser(redirect_uri.as_str())
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => {
                let instance_url = instance_url
                    .ok_or_else(|| auth_bad_request("GitLab instance URL is required"))?;
                agena_runtime::start_gitlab_draft_auth_browser(
                    instance_url.as_str(),
                    redirect_uri.as_str(),
                )
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => Err(auth_bad_request(
                "GitHub Copilot does not support browser draft login",
            )),
        }
    }

    async fn finish_draft_auth_browser(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        instance_url: Option<String>,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<agena_runtime::RuntimeDraftAuthToken, agena_runtime::RuntimeAuthenticationError>
    {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                let user_agent = self.current_snapshot().client_identity().codex_user_agent();
                agena_runtime::finish_openai_draft_auth_browser(
                    user_agent.as_str(),
                    code.as_str(),
                    pkce_verifier.as_str(),
                    redirect_uri.as_str(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => {
                let instance_url = instance_url
                    .ok_or_else(|| auth_bad_request("GitLab instance URL is required"))?;
                agena_runtime::finish_gitlab_draft_auth_browser(
                    instance_url.as_str(),
                    code.as_str(),
                    pkce_verifier.as_str(),
                    redirect_uri.as_str(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => Err(auth_bad_request(
                "GitHub Copilot does not support browser draft login",
            )),
        }
    }

    async fn start_draft_auth_device(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
    ) -> Result<agena_runtime::RuntimeDraftAuthDeviceStart, agena_runtime::RuntimeAuthenticationError>
    {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                let user_agent = self.current_snapshot().client_identity().codex_user_agent();
                agena_runtime::start_openai_draft_auth_device(user_agent.as_str()).await
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => {
                agena_runtime::start_copilot_draft_auth_device(
                    enterprise_domain.as_deref().unwrap_or("github.com"),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => Err(auth_bad_request(
                "GitLab does not support device draft login",
            )),
        }
    }

    async fn poll_draft_auth_device(
        &self,
        kind: agena_runtime::RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
        device_code: String,
        user_code: Option<String>,
    ) -> Result<
        Option<agena_runtime::RuntimeDraftAuthToken>,
        agena_runtime::RuntimeAuthenticationError,
    > {
        match kind {
            agena_runtime::RuntimeDraftAuthKind::OpenaiChatgpt => {
                let user_agent = self.current_snapshot().client_identity().codex_user_agent();
                agena_runtime::poll_openai_draft_auth_device(
                    user_agent.as_str(),
                    device_code.as_str(),
                    user_code.as_deref().unwrap_or_default(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::GithubCopilot => {
                agena_runtime::poll_copilot_draft_auth_device(
                    enterprise_domain.as_deref().unwrap_or("github.com"),
                    device_code.as_str(),
                )
                .await
            }
            agena_runtime::RuntimeDraftAuthKind::Gitlab => Err(auth_bad_request(
                "GitLab does not support device draft login",
            )),
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeConfigurationService for AgenaRuntime {
    fn runtime_configuration(
        &self,
    ) -> Result<agena_runtime::RuntimeConfigurationSnapshot, agena_runtime::RuntimeConfigurationError>
    {
        let snapshot = self.current_snapshot();
        let effective_config = snapshot
            .resolved_config_value()
            .map_err(|error| agena_runtime::RuntimeConfigurationError::from_error(&error))?;
        let effective_config = serde_json::to_value(effective_config)
            .map_err(|error| agena_runtime::RuntimeConfigurationError::from_error(&error))?;
        let configuration_document = snapshot
            .config_value()
            .map_err(|error| agena_runtime::RuntimeConfigurationError::from_error(&error))?;
        Ok(agena_runtime::RuntimeConfigurationSnapshot {
            config_path: snapshot.config_path().to_path_buf(),
            config_found: snapshot.config_found(),
            project_config_path: snapshot.project_config_path().to_path_buf(),
            project_config_found: snapshot.project_config_found(),
            applied_layers: snapshot.applied_layer_descriptions(),
            effective_config,
            configuration_document,
        })
    }
}

impl agena_runtime::RuntimeConfigSettingsService for AgenaRuntime {
    fn read_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsGetInput,
    ) -> Result<agena_runtime::ConfigSettingsReadResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::read_runtime_file_setting(
            self.current_snapshot().config_path().to_path_buf(),
            input,
        )
    }

    fn read_project_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsGetInput,
    ) -> Result<agena_runtime::ConfigSettingsReadResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::read_runtime_file_setting(
            self.current_snapshot().project_config_path().to_path_buf(),
            input,
        )
    }

    fn list_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsListInput,
    ) -> Result<agena_runtime::ConfigSettingsListResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        agena_runtime::list_runtime_file_settings(
            self.current_snapshot().config_path().to_path_buf(),
            input,
        )
    }

    fn set_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsSetInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        let snapshot = self.current_snapshot();
        let validator = runtime_settings_schema_validator(&snapshot);
        agena_runtime::set_runtime_file_setting(
            snapshot.config_path().to_path_buf(),
            input,
            Some(&validator),
        )
    }

    fn set_project_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsSetInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        let snapshot = self.current_snapshot();
        let validator = runtime_settings_schema_validator(&snapshot);
        agena_runtime::set_runtime_file_setting(
            snapshot.project_config_path().to_path_buf(),
            input,
            Some(&validator),
        )
    }

    fn patch_file_settings(
        &self,
        input: agena_runtime::ConfigSettingsPatchInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        let snapshot = self.current_snapshot();
        let validator = runtime_settings_schema_validator(&snapshot);
        agena_runtime::patch_runtime_file_settings(
            snapshot.config_path().to_path_buf(),
            input,
            Some(&validator),
        )
    }

    fn delete_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsDeleteInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        let snapshot = self.current_snapshot();
        let validator = runtime_settings_schema_validator(&snapshot);
        agena_runtime::delete_runtime_file_setting(
            snapshot.config_path().to_path_buf(),
            input,
            Some(&validator),
        )
    }

    fn delete_project_file_setting(
        &self,
        input: agena_runtime::ConfigSettingsDeleteInput,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, agena_runtime::RuntimeConfigSettingsError>
    {
        let snapshot = self.current_snapshot();
        let validator = runtime_settings_schema_validator(&snapshot);
        agena_runtime::delete_runtime_file_setting(
            snapshot.project_config_path().to_path_buf(),
            input,
            Some(&validator),
        )
    }

    fn validate_file_settings(
        &self,
        _input: agena_runtime::ConfigSettingsValidateInput,
    ) -> Result<
        agena_runtime::ConfigSettingsValidateResponse,
        agena_runtime::RuntimeConfigSettingsError,
    > {
        let snapshot = self.current_snapshot();
        let validator = runtime_settings_schema_validator(&snapshot);
        agena_runtime::validate_runtime_file_settings(
            snapshot.config_path().to_path_buf(),
            &validator,
        )
    }
}

/// Build the schema validator used by settings edits.
///
/// A layer document is validated against the *composed* global + workspace
/// configuration rather than on its own, so a workspace override may reference
/// entities that only the global layer declares (for example the provider used
/// by `providers.default_selection`). Validating the workspace document in
/// isolation would reject every such override even though the merged runtime
/// configuration is perfectly valid.
fn runtime_settings_schema_validator(
    snapshot: &Arc<RuntimeSnapshot>,
) -> impl Fn(&std::path::Path, &str) -> Result<(), agena_runtime::RuntimeConfigSettingsError> + 'static
{
    let global_path = snapshot.config_path().to_path_buf();
    let workspace_path = snapshot.project_config_path().to_path_buf();
    move |config_path: &std::path::Path, text: &str| {
        let edited_layer = if config_path == global_path {
            agena_runtime_config::ConfigSettingsLayer::Global
        } else {
            agena_runtime_config::ConfigSettingsLayer::Workspace
        };
        crate::config::validate_layered_config_text(
            global_path.as_path(),
            workspace_path.as_path(),
            edited_layer,
            text,
            &crate::config::ProcessEnvironment,
        )
        .map_err(agena_runtime::config_error_to_settings_error)
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeControlService for AgenaRuntime {
    async fn reload(
        &self,
    ) -> Result<agena_runtime::RuntimeReloadReport, agena_runtime::RuntimeControlServiceError> {
        AgenaRuntime::reload(self)
            .await
            .map_err(|error| match error {
                AppError::Cancelled => agena_runtime::RuntimeControlServiceError::Shutdown,
                error => agena_runtime::RuntimeControlServiceError::from_error(&error),
            })
    }

    async fn fetch_provider_client_versions(
        &self,
    ) -> Result<agena_provider::ProviderClientVersions, agena_runtime::RuntimeControlServiceError>
    {
        agena_runtime::fetch_latest_provider_client_versions()
            .await
            .map_err(|error| agena_runtime::RuntimeControlServiceError::from_error(&error))
    }

    fn runtime_metrics(&self) -> agena_runtime::RuntimeMetricsSnapshot {
        agena_runtime::runtime_metrics_snapshot()
    }

    fn start_runtime_reload_task(
        &self,
        cause: agena_runtime::RuntimeReloadCause,
        origin: agena_runtime::RuntimeBackgroundTaskOrigin,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTaskStart,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::start_runtime_reload_task(self, cause, origin)
    }

    fn start_background_task(
        &self,
        kind: agena_runtime::RuntimeBackgroundTaskKind,
        origin: agena_runtime::RuntimeBackgroundTaskOrigin,
        title: String,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: agena_runtime::RuntimeBackgroundTaskWork,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTaskStart,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::spawn_background_task(
            self,
            kind,
            origin,
            title,
            dedupe_key,
            cancellable,
            move |cancel| async move {
                work(cancel).await.map_err(|error| match error {
                    agena_runtime::RuntimeControlServiceError::Shutdown => AppError::Cancelled,
                    error => AppError::config_error(&error),
                })
            },
        )
    }

    fn background_tasks(&self) -> Vec<agena_runtime::RuntimeBackgroundTask> {
        AgenaRuntime::background_tasks(self)
    }

    fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<
        agena_runtime::RuntimeBackgroundTask,
        agena_runtime::RuntimeBackgroundTaskControlError,
    > {
        AgenaRuntime::cancel_background_task(self, task_id)
    }
}

#[async_trait::async_trait]
impl agena_runtime::RuntimeStatusService for AgenaRuntime {
    async fn runtime_status(&self) -> agena_runtime::RuntimeStatusSnapshot {
        let snapshot = self.current_snapshot();
        let mut provider_ids = snapshot.provider_registry().provider_ids();
        provider_ids.sort();

        let mcp = if let Some(manager) = snapshot.mcp_manager() {
            agena_runtime::RuntimeMcpStatus {
                servers: manager
                    .statuses()
                    .await
                    .into_iter()
                    .map(|status| agena_runtime::RuntimeMcpServerStatus {
                        name: status.name,
                        connected: status.connected,
                        tool_count: status.tool_count,
                        network_target: status.network_target,
                        last_failure: status.last_failure.map(Into::into),
                        instructions_present: status.instructions.is_some(),
                        tool_generation: status.tool_generation,
                        resource_generation: status.resource_generation,
                        prompt_generation: status.prompt_generation,
                        last_refresh_failure: status.last_refresh_failure.map(Into::into),
                        reconnect_supervisor_running: status.reconnect_supervisor_running,
                        auth_mode: status.auth_mode.as_str().to_owned(),
                        oauth_health: status.oauth_health.map(|health| {
                            agena_runtime::RuntimeMcpOAuthHealth {
                                credential_state: health.credential_state.as_str().to_owned(),
                                expiry_state: health
                                    .expiry_state
                                    .map(|state| state.as_str().to_owned()),
                                refresh_available: health.refresh_available,
                            }
                        }),
                        credential_conflict: status.credential_conflict.map(|conflict| {
                            agena_runtime::RuntimeMcpCredentialConflict {
                                state: conflict.as_str().to_owned(),
                                recommendation: conflict.recommendation().to_owned(),
                            }
                        }),
                    })
                    .collect(),
            }
        } else {
            agena_runtime::RuntimeMcpStatus::default()
        };

        let lsp = if let Some(registry) = snapshot.lsp_registry() {
            let mut servers = registry.server_specs().await;
            servers.sort_by(|left, right| left.name.cmp(&right.name));
            let diagnostics = registry.collect_diagnostics().await;
            agena_runtime::RuntimeLspStatus {
                diagnostics_count: diagnostics.iter().map(|(_, entries)| entries.len()).sum(),
                files_with_diagnostics: diagnostics.len(),
                servers: servers
                    .into_iter()
                    .map(|server| agena_runtime::RuntimeLspServerStatus {
                        name: server.name,
                        command: server.command,
                        file_extensions: server.file_extensions,
                        root_markers: server.root_markers,
                    })
                    .collect(),
            }
        } else {
            agena_runtime::RuntimeLspStatus::default()
        };

        // One command surface: what the catalog publishes is what every client
        // renders, so the status projection reads that catalog rather than
        // re-deriving a second list from tool tags.
        let commands = {
            let mut commands = snapshot
                .plugin_manager()
                .command_catalog()
                .into_iter()
                .map(|entry| agena_runtime::RuntimeCommandStatus {
                    name: entry.command.id,
                    slash: entry.command.slash,
                    description: entry.command.docs.summary.unwrap_or_default(),
                    aliases: entry.command.aliases,
                    group: entry.command.group,
                    category: entry.command.category,
                    plugin_id: entry.plugin_id.to_string(),
                    target: match entry.command.target {
                        CommandTarget::Client { .. } => "client",
                        CommandTarget::Method { .. } => "method",
                        CommandTarget::Tool { .. } => "tool",
                    }
                    .to_string(),
                })
                .collect::<Vec<_>>();
            commands.sort_by(|left, right| left.name.cmp(&right.name));
            agena_runtime::RuntimeCommandsStatus { commands }
        };

        let session_manager = snapshot.session_manager();
        let session_runtime_available = session_manager.is_some();
        let (automation_available, scheduled_jobs) = match session_manager {
            Some(manager) => (
                agena_runtime::SessionExecutionControl::scheduler_available(manager.as_ref()),
                agena_runtime::SessionExecutionControl::list_scheduled_jobs(manager.as_ref()).await,
            ),
            None => (false, Ok(Vec::new())),
        };
        let plugin_manager = snapshot.plugin_manager();
        agena_runtime::RuntimeStatusSnapshot {
            generation: snapshot.generation(),
            loaded_at: snapshot.loaded_at(),
            workspace_root: self.workspace_root().to_path_buf(),
            config_path: snapshot.config_path().to_path_buf(),
            config_found: snapshot.config_found(),
            provider_ids,
            plugin_count: plugin_manager.plugins().len(),
            session_runtime_available,
            watch_paths: snapshot.watch_paths().to_vec(),
            reload_enabled: snapshot.reload_enabled(),
            reload_interval_secs: snapshot.reload_poll_interval().as_secs(),
            session_gc_enabled: snapshot.session_gc_enabled(),
            session_gc_interval_secs: snapshot.session_gc_interval().as_secs(),
            model_catalog: snapshot.model_catalog_response(),
            model_catalog_refreshing: self.model_catalog_refresh_active(),
            background_tasks: self.background_tasks(),
            automation_available,
            scheduled_jobs,
            mcp,
            lsp,
            commands,
            agent_id: agena_runtime_contracts::identity::AGENA_AGENT_ID.to_string(),
            plugin_surface_catalog: plugin_manager.surface_catalog(),
            tool_registry_generation: plugin_manager.tool_registry_generation(),
            tool_registry_last_event: plugin_manager
                .tool_registry_events_since(None, 1)
                .into_iter()
                .next(),
        }
    }
}

#[async_trait::async_trait]
impl agena_provider::ProviderModelSource for AgenaRuntime {
    fn provider_ids(&self) -> Vec<agena_domain::ProviderId> {
        self.current_snapshot()
            .catalog_source_provider_registry()
            .provider_ids()
            .into_iter()
            .map(agena_domain::ProviderId::new)
            .collect()
    }

    async fn list_models(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Result<Vec<agena_domain::Model>, agena_provider::ProviderCatalogError> {
        self.current_snapshot()
            .catalog_source_provider_registry()
            .list_models(provider_id.as_ref())
            .await
            .map_err(|error| agena_provider::ProviderCatalogError::operation_error(&error))
    }
}

impl AgenaRuntime {
    async fn list_adapter_models_target(
        &self,
        target: crate::config::ProviderAdapterModelsTarget,
        snapshot: Arc<RuntimeSnapshot>,
    ) -> Result<agena_provider::ProviderAdapterModelsListing, agena_provider::ProviderCatalogError>
    {
        let network = snapshot
            .provider_configs()
            .get(target.provider_id.as_str())
            .map(|provider| provider.network)
            .unwrap_or_default();
        let client = crate::provider::ProviderRegistry::build_http_client(
            agena_provider::ProviderHttpClientConfig {
                timeout: std::time::Duration::from_secs(network.request_timeout_secs),
                connect_timeout: std::time::Duration::from_secs(network.connect_timeout_secs),
            },
        )
        .map_err(|error| agena_provider::ProviderCatalogError::operation_error(&error))?;
        let catalog = snapshot.model_catalog().snapshot();
        let adapters = agena_runtime_provider_adapters::config_support::registry::list_provider_adapter_models(
            target.provider_id.as_str(),
            &target.auth,
            &target.adapters,
            Some(&catalog),
            client,
            &crate::config::ProcessEnvironment,
            snapshot.client_identity(),
        )
        .await;
        Ok(agena_provider::ProviderAdapterModelsListing {
            provider_id: target.provider_id,
            adapters: adapters
                .into_iter()
                .map(|adapter| agena_provider::ProviderAdapterModelsEntry {
                    adapter_id: adapter.adapter_id,
                    enabled: adapter.enabled,
                    resolved_base_url: adapter.resolved_base_url,
                    models: adapter.models,
                    failure: adapter.failure,
                })
                .collect(),
        })
    }
}

fn runtime_bootstrap_error(error: AppError) -> agena_runtime::RuntimeBootstrapError {
    match &error {
        AppError::Config(_) | AppError::ConfigErr(_) => {
            agena_runtime::RuntimeBootstrapError::configuration_error(&error)
        }
        AppError::Database(_) | AppError::StorageConfig(_) => {
            agena_runtime::RuntimeBootstrapError::database_error(&error)
        }
        AppError::Io(_) => agena_runtime::RuntimeBootstrapError::io_error(&error),
        _ => agena_runtime::RuntimeBootstrapError::internal_error(&error),
    }
}

fn runtime_database_error(error: agena_runtime::RuntimeDatabaseCompositionError) -> AppError {
    match error {
        agena_runtime::RuntimeDatabaseCompositionError::StorageConfig(error) => {
            AppError::StorageConfig(error)
        }
        agena_runtime::RuntimeDatabaseCompositionError::Database(error) => {
            AppError::Database(error)
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgenaRuntime {
    // All clones refer to one shared owner, including clones wrapped in a new
    // Arc by capability adapters. Weak callbacks must target this owner.
    shared: Arc<AgenaRuntimeShared>,
}

impl std::ops::Deref for AgenaRuntime {
    type Target = AgenaRuntimeShared;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

#[derive(Clone)]
pub(super) struct WeakAgenaRuntime(std::sync::Weak<AgenaRuntimeShared>);

impl WeakAgenaRuntime {
    pub(super) fn upgrade(&self) -> Option<AgenaRuntime> {
        self.0.upgrade().map(|shared| AgenaRuntime { shared })
    }
}

impl AgenaRuntime {
    pub(super) fn downgrade(&self) -> WeakAgenaRuntime {
        WeakAgenaRuntime(Arc::downgrade(&self.shared))
    }
}

pub(crate) struct AgenaRuntimeShared {
    pub(crate) inner: Arc<AgenaRuntimeInner>,
    pub(crate) activities: Arc<crate::activity::ActivityRuntimeState>,
    pub(crate) live_signals: Arc<crate::live_signal::LiveSignalHub>,
    /// Holds the subtask→activity facade subscription for the runtime's
    /// lifetime. The handle must not be dropped or it unsubscribes (15.5).
    subtask_bridge: Arc<std::sync::Mutex<Option<agena_storage::store::GlobalSubscription>>>,
    /// Correlates launched-in-background operations with their transcript
    /// parts. The observer subscription it creates is held by
    /// `subtask_bridge`; the completion signals reach it through the monitor
    /// `on_finished` callback and the facade `SessionMetaUpdated` events.
    background_completion: crate::activity::BackgroundCompletionBridge,
    automatic_maintenance: bool,
}

pub(crate) type AgenaRuntimeInner = agena_runtime::RuntimeProcessState<
    ConfigLoader<ProcessEnvironment>,
    LoadConfigRequest,
    RuntimeSnapshot,
    AppError,
>;

impl agena_runtime::RuntimeBootstrapLifecycle for AgenaRuntime {
    fn shutdown(&self) {
        AgenaRuntime::shutdown(self);
    }
}

impl AgenaRuntime {
    pub(crate) fn current_snapshot(&self) -> Arc<RuntimeSnapshot> {
        self.inner.control_state.current_snapshot()
    }

    pub(crate) fn session_manager(&self) -> Option<Arc<SessionManager>> {
        self.current_snapshot().session_manager()
    }

    /// Assemble application-facing runtime capabilities once at the concrete
    /// composition boundary. Application consumers receive the runtime-owned
    /// result rather than converting this concrete handle independently.
    pub fn application_services(&self) -> agena_runtime::RuntimeApplicationServices {
        self.application_services_with_manager_option(self.session_manager())
    }

    fn application_services_with_manager_option(
        &self,
        session_manager: Option<Arc<SessionManager>>,
    ) -> agena_runtime::RuntimeApplicationServices {
        let repositories = self.inner.database.as_ref().map(|database| {
            agena_runtime::RuntimeApplicationRepositories {
                memory: Arc::new(MemoryStore::for_workspace(self.workspace_root())),
                workspace: Arc::new(agena_storage_sqlite::SeaWorkspaceRepository::new(
                    Arc::clone(database),
                )),
                permission_rules: Arc::new(agena_storage_sqlite::SeaPermissionRuleRepository::new(
                    Arc::clone(database),
                )),
            }
        });
        let session_store = session_manager
            .as_ref()
            .map(|manager| manager.session_store());
        let session_queries = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionQueryService>);
        let execution_control = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionExecutionControl>);
        let execution_commands = session_manager.as_ref().map(|manager| {
            manager.clone() as Arc<dyn agena_runtime::SessionExecutionCommandService>
        });
        let tool_execution = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionToolExecutionService>);
        let plugin_commands = session_manager
            .as_ref()
            .map(|manager| manager.clone() as Arc<dyn agena_runtime::SessionPluginCommandService>);
        agena_runtime::compose_runtime_application_services(
            agena_runtime::RuntimeApplicationServiceCompositionInputs {
                workspace_root: self.workspace_root().to_path_buf(),
                repositories,
                provider_catalog: Arc::new(self.clone()),
                model_catalog: Arc::new(self.clone()),
                plugins: Arc::new(self.clone()),
                configuration: Arc::new(self.clone()),
                config_settings: Arc::new(self.clone()),
                control: Arc::new(self.clone()),
                authentication: Arc::new(self.clone()),
                draft_authentication: Arc::new(self.clone()),
                status: Arc::new(self.clone()),
                tools: Arc::new(self.clone()),
                activities: Some(
                    Arc::new(self.clone()) as Arc<dyn agena_runtime::RuntimeActivityService>
                ),
                live_signals: {
                    struct Adapter(Arc<agena_runtime::LiveSignalHub>);
                    impl agena_runtime::RuntimeLiveSignalService for Adapter {
                        fn observe(
                            &self,
                            observer: agena_runtime::RuntimeLiveSignalObserver,
                        ) -> Option<agena_runtime::RuntimeLiveSignalObservation>
                        {
                            Some(self.0.observe(observer))
                        }
                        fn subscribe(
                            &self,
                        ) -> Box<dyn agena_runtime::RuntimeLiveSignalSubscription>
                        {
                            self.0
                                .subscribe()
                                .expect("live signal subscription requires a tokio runtime")
                        }
                    }
                    Some(Arc::new(Adapter(Arc::clone(&self.live_signals)))
                        as Arc<dyn agena_runtime::RuntimeLiveSignalService>)
                },
                session_store,
                session_queries,
                execution_control,
                execution_commands,
                tool_execution,
                plugin_commands,
            },
        )
    }

    fn runtime_tool_executor(&self) -> ToolExecutor {
        if let Some(manager) = self.session_manager() {
            return manager.tool_executor();
        }

        let snapshot = self.current_snapshot();
        ToolExecutor::new(
            self.workspace_root().to_path_buf(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            snapshot.plugin_manager(),
            None,
            None,
        )
    }

    pub fn workspace_root(&self) -> &Path {
        &self.inner.workspace_root
    }

    pub fn shutdown(&self) {
        // Close admission and order shutdown after any admitted publication
        // before selecting the session notification host. Concurrent/repeated
        // calls share the same first-transition decision.
        if !self.inner.control_state.shutdown() {
            return;
        }
        if let Some(service) = self.activities.monitor.as_ref() {
            service.shutdown();
        }
        if let Some(session_manager) = self.session_manager() {
            if let Some(scheduler) = session_manager.tool_executor().scheduler() {
                scheduler.stop();
            }
            match tokio::runtime::Handle::try_current() {
                Ok(_handle) => {
                    let broadcast = session_manager
                        .broadcast_active_session_end(agena_plugin_host::SessionEndReason::Other);
                    agena_runtime::spawn_detached(broadcast);
                }
                Err(error) => {
                    tracing::warn!(
                        target: "agena_plugin_host::session_end",
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "session.end could not be broadcast during runtime shutdown because no Tokio runtime is available",
                            &error,
                        ),
                        "no tokio runtime available during shutdown; skipping session.end broadcast"
                    );
                }
            }
        }
    }

    pub async fn reload(&self) -> Result<RuntimeReloadReport, AppError> {
        self.reload_with_cause(RuntimeReloadCause::Manual).await
    }

    pub(crate) async fn reload_with_cause(
        &self,
        cause: RuntimeReloadCause,
    ) -> Result<RuntimeReloadReport, AppError> {
        let control = self.inner.control_state.task_control();
        let _guard = tokio::select! {
            biased;
            _ = control.cancelled() => return Err(AppError::Cancelled),
            guard = self.inner.control_state.reload_gate().acquire() => guard,
        };
        let previous = self.current_snapshot();
        let next = Arc::new(tokio::select! {
            biased;
            _ = control.cancelled() => return Err(AppError::Cancelled),
            result = RuntimeSnapshot::build_with_previous(
                previous.generation() + 1,
                &self.inner.loader,
                &self.inner.load_request,
                self.inner.workspace_root.as_path(),
                super::SnapshotDatabases {
                    chat: self.inner.database.clone(),
                    scheduler: self.inner.scheduler_database.clone(),
                },
                previous.session_manager(),
                Arc::clone(&previous),
                self.activities.monitor.clone(),
            ) => result?,
        });
        self.publish_reload_candidate(previous, next, cause)
    }

    /// Commit a completed candidate while the caller owns the reload gate.
    /// The lifecycle guard orders all synchronous publication against shutdown.
    pub(super) fn publish_reload_candidate(
        &self,
        previous: Arc<RuntimeSnapshot>,
        next: Arc<RuntimeSnapshot>,
        cause: RuntimeReloadCause,
    ) -> Result<RuntimeReloadReport, AppError> {
        let previous_generation = previous.generation();
        {
            let _publication = self
                .inner
                .control_state
                .task_control()
                .running_guard()
                .ok_or(AppError::Cancelled)?;
            self.apply_tracing_filter(next.tracing_config());
            // Bind before publication; the client's generation check becomes
            // ready with the snapshot swap. Shutdown cannot overtake this
            // synchronous publication after the final lifecycle check.
            next.host_client.bind_runtime(self);
            next.publish_session_configuration();
            let _ = self.inner.control_state.swap_snapshot(next.clone());
            self.start_scheduler_activity_bridge();
            let host_handle = next.plugin_manager().host_handle();
            super::host_client::install_plugin_host_event_publisher(host_handle, self);
            agena_runtime::install_plugin_host(next.plugin_manager());
        }
        if let Err(error) =
            self.start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System)
        {
            // Publication is complete. Failure to enqueue this independent
            // maintenance operation belongs to the catalog status, and must
            // not turn an already-published reload into a failed task.
            next.model_catalog().record_refresh_failure(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to start model catalog refresh after runtime reload",
                    &error,
                ),
            );
        }

        Ok(RuntimeReloadReport {
            cause,
            previous_generation,
            generation: next.generation(),
            loaded_at: next.loaded_at(),
        })
    }

    pub(crate) fn task_control_handle(&self) -> Arc<TaskControl> {
        self.inner.control_state.task_control_handle()
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.inner.control_state.task_control().is_shutdown()
    }

    pub fn background_tasks(&self) -> Vec<RuntimeBackgroundTask> {
        self.inner.control_state.background_tasks().list()
    }

    pub fn model_catalog_refresh_active(&self) -> bool {
        self.inner
            .control_state
            .background_tasks()
            .is_kind_running(RuntimeBackgroundTaskKind::ModelCatalogRefresh)
    }

    pub fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<RuntimeBackgroundTask, RuntimeBackgroundTaskControlError> {
        self.inner.control_state.background_tasks().cancel(task_id)
    }

    pub fn spawn_background_task<F, Fut>(
        &self,
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
        title: impl Into<String>,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: F,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<RuntimeBackgroundTaskOutcome, AppError>>
            + Send
            + 'static,
    {
        if self.is_shutdown() {
            return Err(RuntimeBackgroundTaskControlError::Shutdown);
        }

        let spec = RuntimeBackgroundTaskSpec::new(kind, origin, title, dedupe_key, cancellable);
        self.inner
            .control_state
            .background_tasks()
            .spawn(spec, move |cancel| async move {
                // Cancellation can arrive after the registry has polled its
                // token in this turn. Preserve an explicit cancellation from
                // the work branch instead of recording it as a failed task.
                match work(cancel).await {
                    Err(AppError::Cancelled) => Ok(RuntimeBackgroundTaskOutcome::cancelled(
                        "Runtime operation cancelled.",
                    )),
                    result => result,
                }
            })
    }

    pub fn start_runtime_reload_task(
        &self,
        cause: RuntimeReloadCause,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let dedupe_key = Some("runtime_reload".to_owned());
        let title = match &cause {
            RuntimeReloadCause::Manual => "Reload runtime".to_owned(),
            RuntimeReloadCause::WatchedPathsChanged { paths } => {
                format!(
                    "Reload runtime after {} watched path change(s)",
                    paths.len()
                )
            }
        };
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::RuntimeReload,
            origin,
            title,
            dedupe_key,
            false,
            move |_| async move {
                let report = runtime.reload_with_cause(cause.clone()).await?;
                let message = match &cause {
                    RuntimeReloadCause::Manual => {
                        format!("Runtime reloaded to generation {}.", report.generation)
                    }
                    RuntimeReloadCause::WatchedPathsChanged { paths } => format!(
                        "Runtime reloaded to generation {} after changes in {} watched path(s).",
                        report.generation,
                        paths.len()
                    ),
                };
                Ok(RuntimeBackgroundTaskOutcome::succeeded(message))
            },
        )
    }

    /// A plugin cannot await its own retirement. Keep the accepted control
    /// task outside its callback context and defer mutation until the entire
    /// originating call has returned (including nested calls/streams).
    pub(crate) fn start_plugin_reload_task(
        &self,
        after: agena_plugin_host::PluginCallCompletion,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::RuntimeReload,
            RuntimeBackgroundTaskOrigin::System,
            "Reload runtime after plugin call",
            Some(format!("plugin_reload:{}", after.id())),
            false,
            move |_| async move {
                after.wait().await;
                let report = runtime
                    .reload_with_cause(RuntimeReloadCause::Manual)
                    .await?;
                Ok(RuntimeBackgroundTaskOutcome::succeeded(format!(
                    "Runtime reloaded to generation {}.",
                    report.generation
                )))
            },
        )
    }

    pub fn start_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        self.spawn_model_catalog_refresh(origin)
    }

    fn start_model_catalog_refresh_if_needed(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<Option<RuntimeBackgroundTaskStart>, RuntimeBackgroundTaskControlError> {
        if !self.automatic_maintenance || self.is_shutdown() {
            return Ok(None);
        }

        if !self
            .current_snapshot()
            .model_catalog()
            .needs_startup_refresh()
        {
            return Ok(None);
        }

        self.spawn_model_catalog_refresh(origin).map(Some)
    }

    fn spawn_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
        let runtime = self.clone();
        self.spawn_background_task(
            RuntimeBackgroundTaskKind::ModelCatalogRefresh,
            origin,
            "Refresh model catalog",
            Some("model_catalog_refresh".to_owned()),
            true,
            move |cancel| async move {
                let result: Result<RuntimeBackgroundTaskOutcome, AppError> = async {
                    let snapshot = runtime.current_snapshot();
                    let model_catalog = snapshot.model_catalog();
                    let provider_priorities =
                        agena_runtime::provider_model_catalog_priorities(snapshot.provider_configs());
                    let refreshed = agena_runtime::run_cancellable_refresh(
                        cancel.clone(),
                        || runtime.is_shutdown(),
                        || async {
                            model_catalog
                                .refresh_from_source(
                                    &runtime,
                                    Some(&provider_priorities),
                                )
                                .await
                                .map_err(|error| AppError::config_error(&error))
                        },
                        || async { runtime.reload().await.map(|_| ()) },
                    )
                    .await?;

                    let Some(refreshed) = refreshed else {
                        return Ok(RuntimeBackgroundTaskOutcome::cancelled(
                            "Cancelled before applying the refreshed catalog to the runtime snapshot.",
                        ));
                    };

                    let message = if refreshed.last_failure.is_some() {
                        "Refreshed the model catalog, but some sources were unavailable.".to_owned()
                    } else {
                        "Refreshed model catalog.".to_owned()
                    };

                    Ok(RuntimeBackgroundTaskOutcome::succeeded(message))
                }
                .await;

                if let Err(error) = &result
                    && !matches!(error, AppError::Cancelled)
                {
                    runtime
                        .current_snapshot()
                        .model_catalog()
                        .record_refresh_failure(error.to_string());
                    tracing::warn!(
                        error = %error,
                        origin = ?origin,
                        "background model catalog refresh failed"
                    );
                }

                result
            },
        )
    }

    fn spawn_background_tasks(&self) {
        let janitor_runtime = self.clone();
        let reload_runtime = self.clone();
        let client_versions_runtime = self.clone();
        let model_catalog_runtime = self.clone();
        agena_runtime::spawn_runtime_maintenance_loops(
            self.inner.control_state.task_control(),
            async move {
                let control = janitor_runtime.task_control_handle();
                let interval_runtime = janitor_runtime.clone();
                let tick_runtime = janitor_runtime;
                agena_runtime::run_session_maintenance(
                    control,
                    move || interval_runtime.current_snapshot().session_gc_interval(),
                    move || {
                        let runtime = tick_runtime.clone();
                        async move {
                            if let Some(manager) = runtime.current_snapshot().session_manager() {
                                if let Err(error) = manager.maintenance_tick().await {
                                    tracing::warn!(
                                        error = %error,
                                        "session maintenance failed"
                                    );
                                }
                                let delivery_manager = Arc::clone(&manager);
                                // Await the bounded round so maintenance never overlaps itself.
                                {
                                    if let Err(error) = delivery_manager
                                        .renew_background_operation_leases(128)
                                        .await
                                    {
                                        tracing::warn!(
                                            target: "agena_background",
                                            %error,
                                            "background operation lease renewal failed"
                                        );
                                    }
                                    if let Err(error) =
                                        delivery_manager.reconcile_background_tasks(64).await
                                    {
                                        tracing::warn!(
                                            target: "agena_background",
                                            %error,
                                            "periodic background task reconciliation failed"
                                        );
                                    }
                                    if let Err(error) =
                                        delivery_manager.reconcile_background_processes(64).await
                                    {
                                        tracing::warn!(
                                            target: "agena_background",
                                            %error,
                                            "periodic background process reconciliation failed"
                                        );
                                    }
                                    if let Err(error) =
                                        delivery_manager.recover_background_deliveries(64).await
                                    {
                                        tracing::warn!(
                                            target: "agena_background",
                                            %error,
                                            "periodic background delivery recovery failed"
                                        );
                                    }
                                }
                            }
                        }
                    },
                )
                .await;
            },
            async move { reload::run(reload_runtime).await },
            async move {
                let control = client_versions_runtime.task_control_handle();
                let mut next_refresh_at = tokio::time::Instant::now();
                let mut failures = 0_u32;
                loop {
                    let enabled = client_versions_runtime
                        .current_snapshot()
                        .provider_client_versions_auto_update();
                    if !enabled {
                        failures = 0;
                        // Config reloads do not own this long-lived maintenance
                        // task, so poll briefly while disabled. Re-enabling the
                        // setting can then take effect without waiting a day.
                        next_refresh_at = tokio::time::Instant::now();
                    }

                    let remaining =
                        next_refresh_at.saturating_duration_since(tokio::time::Instant::now());
                    let wait = provider_client_version_wait_duration(enabled, remaining);
                    if !wait.is_zero() {
                        tokio::select! {
                            biased;
                            _ = control.cancelled() => break,
                            _ = tokio::time::sleep(wait) => {}
                        }
                        continue;
                    }
                    match client_versions_runtime
                        .refresh_provider_client_versions_if_enabled()
                        .await
                    {
                        Ok(true) => {
                            failures = 0;
                            next_refresh_at = tokio::time::Instant::now()
                                + PROVIDER_CLIENT_VERSION_REFRESH_INTERVAL;
                        }
                        Ok(false) => {
                            // The file settings can disable auto-update before
                            // the active runtime snapshot has reloaded.
                            failures = 0;
                            next_refresh_at = tokio::time::Instant::now()
                                + PROVIDER_CLIENT_VERSION_SETTINGS_POLL_INTERVAL;
                        }
                        Err(error) => {
                            failures = failures.saturating_add(1);
                            next_refresh_at = tokio::time::Instant::now()
                                + provider_client_version_retry_delay(failures);
                            tracing::warn!(
                                error = %error,
                                "automatic provider client-version refresh failed"
                            );
                        }
                    }
                }
            },
            async move {
                let control = model_catalog_runtime.task_control_handle();
                loop {
                    tokio::select! {
                        biased;
                        _ = control.cancelled() => break,
                        _ = tokio::time::sleep(MODEL_CATALOG_REFRESH_CHECK_INTERVAL) => {}
                    }
                    if model_catalog_runtime.is_shutdown() {
                        break;
                    }
                    if let Err(error) = model_catalog_runtime
                        .start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System)
                    {
                        model_catalog_runtime
                            .current_snapshot()
                            .model_catalog()
                            .record_refresh_failure(error.to_string());
                        tracing::warn!(
                            error = %error,
                            "failed to enqueue stale model catalog refresh"
                        );
                    }
                }
            },
        );

        if let Err(error) =
            self.start_model_catalog_refresh_if_needed(RuntimeBackgroundTaskOrigin::System)
        {
            self.current_snapshot()
                .model_catalog()
                .record_refresh_failure(error.to_string());
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "failed to start the initial background model catalog refresh"
            );
        }
    }

    async fn refresh_provider_client_versions_if_enabled(&self) -> Result<bool, AppError> {
        if self.is_shutdown() {
            return Ok(false);
        }
        let snapshot = self.current_snapshot();
        if !snapshot.provider_client_versions_auto_update() {
            return Ok(false);
        }
        let file_settings =
            <Self as agena_runtime::RuntimeConfigSettingsService>::read_file_settings(
                self,
                agena_runtime::ConfigSettingsGetInput {
                    target: agena_runtime::ConfigSettingsPathInput {
                        path: Some("runtime.providers.client_versions".to_owned()),
                    },
                    source: agena_runtime::ConfigSettingsSource::File,
                },
            )
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let client_versions_settings = file_settings.value.as_object();
        if client_versions_settings
            .and_then(|settings| settings.get("auto_update"))
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            return Ok(false);
        }
        let expected_revision = file_settings.revision;
        let latest = agena_runtime::fetch_latest_provider_client_versions()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let current = self.current_snapshot();
        if !current.provider_client_versions_auto_update()
            || current.generation() != snapshot.generation()
        {
            return Ok(false);
        }
        if current.client_identity().versions() == latest {
            return Ok(true);
        }

        let response = <Self as agena_runtime::RuntimeConfigSettingsService>::patch_file_settings(
            self,
            agena_runtime::ConfigSettingsPatchInput {
                target: agena_runtime::ConfigSettingsPathInput {
                    path: Some("runtime.providers.client_versions".to_owned()),
                },
                changes: serde_json::json!({
                    "codex": latest.codex,
                    "claude": latest.claude,
                    "gemini": latest.gemini,
                    "auto_update": true,
                }),
                options: agena_runtime::ConfigSettingsEditOptions {
                    expected_revision,
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if response.reload_required {
            self.reload().await?;
        }
        tracing::info!(
            generation = self.current_snapshot().generation(),
            "provider client identities refreshed from npm"
        );
        Ok(true)
    }

    fn apply_tracing_filter(&self, tracing: &agena_runtime::RuntimeTracingConfiguration) {
        match agena_runtime::apply_runtime_tracing_filter(&self.inner.control_state, tracing) {
            Ok(false) => {
                tracing::debug!("tracing filter reload skipped or rejected");
            }
            Ok(true) => {}
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    filter = tracing.filter,
                    database = tracing.database,
                    "invalid tracing filter in runtime config"
                );
            }
        }
    }
}
