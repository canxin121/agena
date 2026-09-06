//! Application lifecycle for a stdio transport attached to a PluginHost.

use super::*;
use crate::effect_scope::PluginEffectScope;
use crate::host::HostHandle;
use crate::transport::initialization::PluginInitialization;

pub(super) struct HostedState {
    host: std::sync::Weak<HostHandle>,
    logical_scope: std::sync::Weak<PluginEffectScope>,
    process_scope: Option<Arc<PluginEffectScope>>,
    generation: u64,
    pub initialization: Option<PluginInitialization>,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::sdk::host_api::NoopHostClient;

    async fn bound_transport() -> (Arc<StdioTransport>, Arc<HostHandle>, Arc<PluginEffectScope>) {
        let host = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        let scope = host.begin_plugin_instance("test.handoff".parse().unwrap());
        let transport = Arc::new(
            StdioTransport::spawn_with_policy(
                "/usr/bin/python3",
                &["-c".into(), "import time; time.sleep(60)".into()],
                &HashMap::new(),
                None,
                None,
                RestartPolicy {
                    policy: RestartMode::Never,
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
        );
        transport
            .bind_host(host.clone(), scope.clone())
            .await
            .unwrap();
        (transport, host, scope)
    }

    async fn wait_for_quiescence(process: &PluginEffectScope) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while process.is_accepting() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn handoff_suspends_admission_until_resource_transfer_finishes() {
        let (transport, _host, _scope) = bound_transport().await;
        let process = transport
            .inner
            .hosted
            .lock()
            .await
            .as_ref()
            .unwrap()
            .process_scope
            .clone()
            .unwrap();
        let lease = process.lease().unwrap();
        let next = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        let scope = next.begin_plugin_instance("test.handoff".parse().unwrap());
        let task = tokio::spawn({
            let transport = transport.clone();
            async move { transport.bind_host(next, scope).await }
        });
        wait_for_quiescence(&process).await;
        let notification = transport.notify("test/late", serde_json::json!({})).await;
        let request = tokio::time::timeout(
            Duration::from_millis(100),
            transport.dispatch("test/late", serde_json::json!({})),
        )
        .await;
        drop(lease);
        task.await.unwrap().unwrap();
        transport.close().await.unwrap();
        assert!(matches!(notification, Err(TransportError::Disconnected(_))));
        assert!(matches!(request, Ok(Err(TransportError::Disconnected(_)))));
    }

    #[tokio::test]
    async fn cancelled_handoff_terminates_the_partially_bound_process() {
        let (transport, _host, _scope) = bound_transport().await;
        let process = transport
            .inner
            .hosted
            .lock()
            .await
            .as_ref()
            .unwrap()
            .process_scope
            .clone()
            .unwrap();
        let lease = process.lease().unwrap();
        let finished = transport
            .inner
            .handles
            .lock()
            .await
            .as_ref()
            .unwrap()
            .finished
            .clone();
        let next = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        let scope = next.begin_plugin_instance("test.handoff".parse().unwrap());
        let task = tokio::spawn({
            let transport = transport.clone();
            async move { transport.bind_host(next, scope).await }
        });
        wait_for_quiescence(&process).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(lease);
        let cleanup = tokio::time::timeout(Duration::from_secs(1), finished).await;
        let notification = transport.notify("test/late", serde_json::json!({})).await;
        transport.close().await.unwrap();
        cleanup
            .expect("cancelled handoff must finish child cleanup")
            .unwrap();
        assert!(matches!(notification, Err(TransportError::Disconnected(_))));
        assert_eq!(
            process.state(),
            crate::effect_scope::PluginEffectScopeState::Disposed
        );
    }

    #[tokio::test]
    async fn close_during_handoff_cannot_publish_a_successful_binding() {
        let (transport, _host, logical) = bound_transport().await;
        assert!(transport.can_reuse(&logical).await);
        let process = transport
            .inner
            .hosted
            .lock()
            .await
            .as_ref()
            .unwrap()
            .process_scope
            .clone()
            .unwrap();
        let lease = process.lease().unwrap();
        let next = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        let scope = next.begin_plugin_instance("test.handoff".parse().unwrap());
        let initialization = PluginInitialization {
            host: Arc::downgrade(&next),
            plugin_id: "test.handoff".parse().unwrap(),
            manifest: crate::sdk::PluginManifest::new("test", "handoff", "1.0.0"),
            agena_version: "test".into(),
            workspace_root: PathBuf::from("/tmp"),
            settings: serde_json::json!({}),
        };
        let handoff = tokio::spawn({
            let transport = transport.clone();
            async move {
                transport
                    .try_rebind_host(next, scope, logical, initialization)
                    .await
            }
        });
        wait_for_quiescence(&process).await;
        let close = tokio::spawn({
            let transport = transport.clone();
            async move { transport.close().await }
        });
        transport.inner.shutdown.cancelled().await;
        drop(lease);
        let result = handoff.await.unwrap();
        close.await.unwrap().unwrap();
        assert!(
            result.is_err(),
            "closing during resource handoff must not publish success"
        );
    }

    #[tokio::test]
    async fn cancelled_initialization_terminates_the_unready_process() {
        let (transport, host, _scope) = bound_transport().await;
        let finished = transport
            .inner
            .handles
            .lock()
            .await
            .as_ref()
            .unwrap()
            .finished
            .clone();
        let task = tokio::spawn({
            let transport = transport.clone();
            async move {
                transport
                    .initialize(PluginInitialization {
                        host: Arc::downgrade(&host),
                        plugin_id: "test.handoff".parse().unwrap(),
                        manifest: crate::sdk::PluginManifest::new("test", "handoff", "1.0.0"),
                        agena_version: "test".into(),
                        workspace_root: PathBuf::from("/tmp"),
                        settings: serde_json::json!({}),
                    })
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while transport.inner.pending.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let cleanup = tokio::time::timeout(Duration::from_secs(1), finished).await;
        transport.close().await.unwrap();
        cleanup
            .expect("cancelled initialization must finish child cleanup")
            .unwrap();
    }
}

pub(super) struct GenerationTransport {
    pub inner: Arc<Inner>,
    pub generation: u64,
}

#[async_trait]
impl PluginTransport for GenerationTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        self.inner
            .dispatch_to_generation(Some(self.generation), method, params)
            .await
    }
}

fn new_process_scope(
    parent: &Arc<PluginEffectScope>,
) -> Result<Arc<PluginEffectScope>, TransportError> {
    let process = PluginEffectScope::new(parent.plugin_id().clone());
    parent
        .own_child(Arc::clone(&process))
        .map_err(|error| TransportError::Io(error.to_string()))?;
    Ok(process)
}

async fn dispose(scope: &Arc<PluginEffectScope>) -> Result<(), String> {
    let report = scope.dispose().await;
    if report.errors.is_empty() {
        Ok(())
    } else {
        Err(report.errors.join("; "))
    }
}

struct BindingGuard {
    inner: Arc<Inner>,
    process: Arc<PluginEffectScope>,
    committed: bool,
}

impl Drop for BindingGuard {
    fn drop(&mut self) {
        if !self.committed {
            // Quiescence cannot be undone: some old callbacks may already
            // have been cancelled. A failed/cancelled handoff must therefore
            // stop the process instead of admitting work into a partial host.
            self.inner.closed.store(true, Ordering::SeqCst);
            self.inner.shutdown.cancel();
            self.inner
                .fail_pending("stdio host handoff did not complete");
            self.inner
                .record_spawn_failure("stdio host handoff did not complete");
            let inner = Arc::clone(&self.inner);
            let process = Arc::clone(&self.process);
            tokio::spawn(async move {
                if let Err(error) = dispose(&process).await {
                    inner.record_spawn_failure(&error);
                }
            });
        }
    }
}

impl Inner {
    pub(super) fn can_reuse_hosted(
        &self,
        hosted: Option<&HostedState>,
        handles: Option<&ChildHandles>,
        owner: &Arc<PluginEffectScope>,
    ) -> bool {
        !self.closed.load(Ordering::SeqCst)
            && owner.is_accepting()
            && hosted
                .and_then(|hosted| hosted.logical_scope.upgrade())
                .is_some_and(|scope| Arc::ptr_eq(&scope, owner))
            && handles.is_some_and(|handles| handles.ready && !handles.stop.is_cancelled())
    }

    pub(super) async fn bind_host_inner(
        self: &Arc<Self>,
        host: Arc<HostHandle>,
        scope: Arc<PluginEffectScope>,
        previous_scope: Option<Arc<PluginEffectScope>>,
        restart_initialization: Option<PluginInitialization>,
    ) -> Result<bool, TransportError> {
        let _spawn_guard = self.spawn_lock.lock().await;
        let mut hosted = self.hosted.lock().await;
        if let Some(owner) = previous_scope.as_ref() {
            let handles = self.handles.lock().await;
            if !self.can_reuse_hosted(hosted.as_ref(), handles.as_ref(), owner) {
                return Ok(false);
            }
        }
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::disconnected(
                "cannot attach a closed stdio transport",
            ));
        }
        let process = new_process_scope(&scope)?;
        let mut binding = BindingGuard {
            inner: Arc::clone(self),
            process: Arc::clone(&process),
            committed: false,
        };
        let was_ready = {
            let mut handles = self.handles.lock().await;
            handles
                .as_mut()
                .is_some_and(|handles| std::mem::replace(&mut handles.ready, false))
        };
        let mut initialization = restart_initialization;
        if let Some(previous) = hosted.as_mut() {
            if let Some(previous_process) = previous.process_scope.as_ref() {
                previous_process.quiesce().await;
                let contributions = previous
                    .host
                    .upgrade()
                    .map(|host| host.process_contributions(previous_process));
                dispose(previous_process)
                    .await
                    .map_err(TransportError::Io)?;
                if let Some(contributions) = contributions {
                    host.import_process_contributions(Arc::clone(&process), contributions)
                        .await
                        .map_err(TransportError::Plugin)?;
                }
            }
            initialization = initialization.or_else(|| previous.initialization.clone());
        }
        if let Some(initialization) = initialization.as_mut() {
            initialization.host = Arc::downgrade(&host);
        }
        // Acquire every async lock before publication so cancellation cannot
        // split the callback, status, restart recipe and admission commit.
        let mut handler = self.host_handler.lock().await;
        let mut handles = self.handles.lock().await;
        if self.closed.load(Ordering::SeqCst)
            || handles
                .as_ref()
                .is_none_or(|handles| handles.stop.is_cancelled())
        {
            return Err(TransportError::disconnected(
                "stdio child stopped during host handoff",
            ));
        }
        *handler = Some(host.host_handler_for_process(Arc::clone(&scope), Arc::clone(&process)));
        let successor = host.status_registry();
        {
            let mut binding = self
                .status_sink
                .write()
                .unwrap_or_else(|error| error.into_inner());
            if let (Some(previous), Some(plugin_id)) = (binding.as_ref(), self.plugin_id.as_ref())
                && let Some(status) = previous.get(plugin_id)
            {
                successor.set(status);
            }
            *binding = Some(successor);
        }
        *hosted = Some(HostedState {
            host: Arc::downgrade(&host),
            logical_scope: Arc::downgrade(&scope),
            process_scope: Some(process),
            generation: self.child_generation.load(Ordering::SeqCst),
            initialization,
        });
        if was_ready && let Some(handles) = handles.as_mut() {
            handles.ready = !handles.stop.is_cancelled();
        }
        binding.committed = true;
        Ok(true)
    }

    pub(super) async fn initialize_hosted(
        self: &Arc<Self>,
        initialization: PluginInitialization,
    ) -> Result<crate::sdk::InitOutcome, TransportError> {
        let _spawn_guard = self.spawn_lock.lock().await;
        {
            let mut hosted = self.hosted.lock().await;
            let hosted = hosted.as_mut().ok_or_else(|| {
                TransportError::Io("stdio initialization requires a bound host".into())
            })?;
            hosted.initialization = Some(initialization.clone());
        }
        let (generation, stop) = {
            let mut handles = self.handles.lock().await;
            let handles = handles.as_mut().ok_or_else(|| {
                TransportError::disconnected("stdio plugin exited before initialization")
            })?;
            handles.ready = false;
            (handles.generation, handles.stop.clone())
        };
        let cancellation_guard = stop.drop_guard();
        let result = initialization
            .initialize(&GenerationTransport {
                inner: Arc::clone(self),
                generation,
            })
            .await;
        match result {
            Ok(outcome) => {
                self.mark_generation_ready(generation).await?;
                cancellation_guard.disarm();
                Ok(outcome)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn prepare_hosted_generation(
        &self,
        generation: u64,
    ) -> Result<(), TransportError> {
        let mut hosted = self.hosted.lock().await;
        let hosted = hosted
            .as_mut()
            .ok_or_else(|| TransportError::Io("stdio restart has no host binding".into()))?;
        let host = hosted
            .host
            .upgrade()
            .ok_or_else(|| TransportError::disconnected("stdio restart host has been dropped"))?;
        let logical_scope = hosted
            .logical_scope
            .upgrade()
            .ok_or_else(|| TransportError::disconnected("stdio plugin owner has been dropped"))?;
        let process = new_process_scope(&logical_scope)?;
        *self.host_handler.lock().await =
            Some(host.host_handler_for_process(logical_scope, Arc::clone(&process)));
        if let Some(initialization) = hosted.initialization.as_ref() {
            host.restore_manifest_tools(&initialization.plugin_id, &initialization.manifest)
                .map_err(TransportError::Plugin)?;
        }
        hosted.process_scope = Some(process);
        hosted.generation = generation;
        Ok(())
    }

    pub(super) async fn dispose_hosted_generation(&self, generation: u64) -> Result<(), String> {
        let mut hosted = self.hosted.lock().await;
        if let Some(hosted) = hosted.as_mut()
            && hosted.generation == generation
            && let Some(process) = hosted.process_scope.take()
        {
            dispose(&process).await?;
        }
        Ok(())
    }

    pub(super) async fn mark_generation_ready(
        &self,
        generation: u64,
    ) -> Result<(), TransportError> {
        let mut handles = self.handles.lock().await;
        let handles = handles
            .as_mut()
            .filter(|handles| handles.generation == generation && !handles.stop.is_cancelled())
            .ok_or_else(|| {
                TransportError::disconnected("stdio child exited during initialization")
            })?;
        handles.ready = true;
        Ok(())
    }
}
