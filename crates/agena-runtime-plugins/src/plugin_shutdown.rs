//! Snapshot-scoped plugin retirement and a runtime-owned final drain.
//!
//! Reload waits for a generation's external leases before closing it. Final
//! shutdown is called only after session supervision has drained, so it may
//! close current hosts even while inspection/service bundles still retain Arcs.

use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agena_plugin_host::{PluginHost, PluginShutdownFailure, PluginShutdownReport};
use tokio::{sync::watch, task::JoinHandle};

const GENERATION_RETIRE_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Default)]
struct GenerationWorker {
    started: bool,
    task: Option<JoinHandle<PluginShutdownReport>>,
}

struct Generation {
    host: Weak<PluginHost>,
    executor: Option<tokio::runtime::Handle>,
    worker: Mutex<GenerationWorker>,
    report: Mutex<Option<PluginShutdownReport>>,
}

#[derive(Default)]
struct Generations {
    closed: bool,
    entries: Vec<Arc<Generation>>,
}

type Completion = watch::Receiver<Option<PluginShutdownReport>>;

#[derive(Default)]
struct CoordinatorInner {
    generations: Mutex<Generations>,
    force: AtomicBool,
    completion: Mutex<Option<Completion>>,
}

/// Shared by every snapshot of one runtime, including unpublished candidates.
/// It holds weak host references so normal reload retirement is not delayed.
#[derive(Clone, Default)]
pub struct PluginShutdownCoordinator {
    inner: Arc<CoordinatorInner>,
}

fn failure(stage: &str, message: impl Into<String>) -> PluginShutdownReport {
    PluginShutdownReport {
        failures: vec![PluginShutdownFailure {
            plugin_id: "<host generation>".into(),
            stage: stage.into(),
            message: message.into(),
        }],
    }
}

impl PluginShutdownCoordinator {
    /// Register successful initialization before any subsequent composition
    /// awaits. Rejection means final draining has begun; the caller must await
    /// the unregistered host's shutdown before abandoning its candidate.
    pub fn register(
        &self,
        plugins: Arc<PluginHost>,
    ) -> Result<Option<Arc<crate::CallbackOnDrop>>, PluginShutdownReport> {
        if plugins.is_empty() {
            return Ok(None);
        }
        let generation = Arc::new(Generation {
            host: Arc::downgrade(&plugins),
            executor: tokio::runtime::Handle::try_current().ok(),
            worker: Mutex::new(GenerationWorker::default()),
            report: Mutex::new(None),
        });
        {
            let mut generations = self
                .inner
                .generations
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if generations.closed {
                return Err(failure(
                    "registration",
                    "runtime plugin shutdown has already begun",
                ));
            }
            // Successful, fully completed retirements need no further work.
            // Retain failures so final shutdown cannot accidentally report success.
            generations.entries.retain(|entry| {
                let successful = entry
                    .report
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .is_some_and(PluginShutdownReport::is_success);
                let finished = entry
                    .worker
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .task
                    .as_ref()
                    .is_some_and(JoinHandle::is_finished);
                !(successful && finished)
            });
            generations.entries.push(generation.clone());
        }
        let coordinator = self.clone();
        Ok(Some(Arc::new(crate::CallbackOnDrop::new(move || {
            coordinator.start_generation(&generation, plugins);
        }))))
    }

    fn start_generation(&self, generation: &Arc<Generation>, plugins: Arc<PluginHost>) {
        let mut worker = generation.worker.lock().unwrap_or_else(|p| p.into_inner());
        if worker.started {
            return;
        }
        worker.started = true;
        let executor = match generation
            .executor
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        {
            Some(executor) => executor,
            None => {
                *generation.report.lock().unwrap_or_else(|p| p.into_inner()) = Some(failure(
                    "retirement",
                    "plugin retirement requires a Tokio executor",
                ));
                return;
            }
        };
        let inner = self.inner.clone();
        let observed = generation.clone();
        worker.task = Some(executor.spawn(async move {
            // Only this retiring worker owns a strong generation reference
            // once every in-flight ToolExecutor lease has gone away.
            while !inner.force.load(Ordering::Acquire) && Arc::strong_count(&plugins) > 1 {
                tokio::time::sleep(GENERATION_RETIRE_POLL_INTERVAL).await;
            }
            let report = plugins.shutdown_report().await;
            drop(plugins);
            *observed.report.lock().unwrap_or_else(|p| p.into_inner()) = Some(report.clone());
            report
        }));
    }

    /// Stop every registered current/retiring host and join all retirement
    /// tasks. Call only after no session or runtime worker may use those hosts.
    /// A cancelled waiter does not cancel the shared drain; every waiter sees
    /// the same transport/effect/panic failures.
    pub async fn shutdown_and_wait(&self) -> Result<(), PluginShutdownReport> {
        let mut receiver = {
            let mut cached = self
                .inner
                .completion
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(receiver) = cached.as_ref() {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                *cached = Some(receiver.clone());
                let entries = {
                    let mut generations = self
                        .inner
                        .generations
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    generations.closed = true;
                    self.inner.force.store(true, Ordering::Release);
                    generations.entries.clone()
                };
                for generation in &entries {
                    if let Some(plugins) = generation.host.upgrade() {
                        self.start_generation(generation, plugins);
                    }
                }
                match tokio::runtime::Handle::try_current() {
                    Ok(executor) => {
                        executor.spawn(async move {
                            let mut report = PluginShutdownReport::default();
                            for generation in entries {
                                let task = generation
                                    .worker
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .task
                                    .take();
                                let outcome = match task {
                                    Some(task) => match task.await {
                                        Ok(report) => report,
                                        Err(error) => failure("retirement_task", error.to_string()),
                                    },
                                    None => generation
                                        .report
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .clone()
                                        .unwrap_or_else(|| {
                                            failure(
                                                "retirement",
                                                "plugin host vanished without a shutdown result",
                                            )
                                        }),
                                };
                                report.failures.extend(outcome.failures);
                            }
                            sender.send_replace(Some(report));
                        });
                    }
                    Err(error) => {
                        sender.send_replace(Some(failure(
                            "drain_task",
                            format!("plugin shutdown requires a Tokio executor: {error}"),
                        )));
                    }
                }
                receiver
            }
        };
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(report) = result {
                return report.into_result();
            }
            if receiver.changed().await.is_err() {
                return Err(failure(
                    "drain_task",
                    "plugin drain owner ended without a result",
                ));
            }
        }
    }
}

/// Compatibility guard for consumers with only one independent host. Runtime
/// composition uses a shared PluginShutdownCoordinator for final draining.
pub fn plugin_shutdown_guard(plugins: Arc<PluginHost>) -> Option<Arc<crate::CallbackOnDrop>> {
    PluginShutdownCoordinator::default()
        .register(plugins)
        .expect("a new plugin shutdown coordinator accepts its first host")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::Semaphore;

    #[derive(Clone)]
    struct Probe {
        started: Arc<Semaphore>,
        release: Arc<Semaphore>,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl agena_plugin_sdk::Plugin for Probe {
        fn manifest(&self) -> agena_plugin_sdk::PluginManifest {
            agena_plugin_sdk::PluginManifest::new("test", "shutdown-owner", "test")
        }

        async fn shutdown(&self) -> agena_plugin_sdk::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            if self.fail {
                Err(agena_plugin_sdk::PluginError::not_implemented(
                    "deliberate shutdown failure",
                ))
            } else {
                Ok(())
            }
        }
    }

    async fn host(fail: bool) -> (Arc<PluginHost>, Probe) {
        let probe = Probe {
            started: Arc::new(Semaphore::new(0)),
            release: Arc::new(Semaphore::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
            fail,
        };
        let mut config = agena_plugin_host::PluginsConfig::default();
        config.list.insert(
            "test.shutdown-owner".into(),
            agena_plugin_host::ConfiguredPlugin::static_default(),
        );
        let host = PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
            static_plugins: vec![agena_plugin_host::StaticPluginRegistration::new(
                "test.shutdown-owner".parse().unwrap(),
                probe.clone(),
            )],
            config,
            workspace_root: std::env::temp_dir(),
            agena_version: "test".into(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .unwrap();
        (host, probe)
    }

    #[tokio::test]
    async fn reload_retirement_waits_for_external_generation_leases() {
        let coordinator = PluginShutdownCoordinator::default();
        let (host, probe) = host(false).await;
        let guard = coordinator.register(host.clone()).unwrap();
        drop(guard);
        // The still-held host is an execution generation lease. Yielding lets
        // the retirement worker run, but it must not close the plugin yet.
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        drop(host);
        tokio::time::timeout(Duration::from_secs(2), probe.started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        probe.release.add_permits(1);
        coordinator.shutdown_and_wait().await.unwrap();
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn final_drain_closes_current_host_and_survives_cancelled_waiter() {
        let coordinator = PluginShutdownCoordinator::default();
        let (host, probe) = host(false).await;
        let guard = coordinator.register(host.clone()).unwrap();
        let waiter = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.shutdown_and_wait().await })
        };
        tokio::time::timeout(Duration::from_secs(2), probe.started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        probe.release.add_permits(1);
        coordinator.shutdown_and_wait().await.unwrap();
        host.shutdown_report().await.into_result().unwrap();
        drop(guard);
        coordinator.shutdown_and_wait().await.unwrap();
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_retirement_remains_a_failure_for_later_drains() {
        let coordinator = PluginShutdownCoordinator::default();
        let (host, probe) = host(true).await;
        let guard = coordinator.register(host).unwrap();
        probe.release.add_permits(1);
        drop(guard);
        let first = coordinator.shutdown_and_wait().await.unwrap_err();
        assert!(
            first.to_string().contains("deliberate shutdown failure"),
            "{first}"
        );
        assert_eq!(coordinator.shutdown_and_wait().await.unwrap_err(), first);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn direct_host_waiter_cancellation_does_not_repeat_shutdown() {
        let (host, probe) = host(false).await;
        let first = {
            let host = host.clone();
            tokio::spawn(async move { host.shutdown_report().await })
        };
        tokio::time::timeout(Duration::from_secs(2), probe.started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        first.abort();
        let _ = first.await;
        probe.release.add_permits(1);
        let report = tokio::time::timeout(Duration::from_secs(2), host.shutdown_report())
            .await
            .unwrap();
        assert!(report.is_success(), "{report}");
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn effect_disposal_failure_is_in_the_cached_shutdown_report() {
        let (host, probe) = host(false).await;
        let key = "test.shutdown-owner".parse().unwrap();
        host.host_handle()
            .effect_scope(&key)
            .unwrap()
            .own_sync("test", "failing disposer", || {
                Err("effect refused cleanup".into())
            })
            .unwrap();
        probe.release.add_permits(1);
        let report = host.shutdown_report().await;
        assert!(
            report.failures.iter().any(|failure| {
                failure.stage == "effects" && failure.message.contains("effect refused cleanup")
            }),
            "{report}"
        );
        assert_eq!(host.shutdown_report().await, report);
    }
}
