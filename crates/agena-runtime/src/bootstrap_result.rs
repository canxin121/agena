//! Runtime-owned output of concrete bootstrap composition.
//!
//! Application consumers retain this stable capability bundle rather than a
//! concrete runtime handle. Runtime owns the concrete builder and the result
//! keeps that implementation detail out of upper layers.

use std::sync::Arc;

use crate::{
    RuntimeApplicationServices, RuntimeBootstrapError, RuntimeBootstrapRequest,
    RuntimeCompositionConfig,
};

/// Minimal lifecycle control retained by a bootstrap result without exposing a
/// concrete runtime implementation to the application layer.
#[async_trait::async_trait]
pub(crate) trait RuntimeBootstrapLifecycle: Send + Sync {
    fn shutdown(&self);
    async fn shutdown_and_wait(&self) -> Result<(), crate::RuntimeControlServiceError>;
}

/// Concrete capability/lifecycle values returned by Runtime composition.
/// The result envelope exposes only application contracts and lifecycle
/// control, never an implementation handle.
pub(crate) struct RuntimeBootstrapComposition {
    pub application_services: RuntimeApplicationServices,
    pub lifecycle: Arc<dyn RuntimeBootstrapLifecycle>,
}

impl RuntimeBootstrapComposition {
    pub(crate) fn new(
        application_services: RuntimeApplicationServices,
        lifecycle: Arc<dyn RuntimeBootstrapLifecycle>,
    ) -> Self {
        Self {
            application_services,
            lifecycle,
        }
    }
}

/// Capabilities made available after a runtime has been composed.
///
/// The service objects retain the concrete adapter instances that implement
/// their ports, so this result is sufficient to keep an already-composed
/// runtime alive for application consumers.
#[derive(Clone)]
pub struct RuntimeBootstrapResult {
    application_services: RuntimeApplicationServices,
    lifecycle: Arc<dyn RuntimeBootstrapLifecycle>,
}

impl RuntimeBootstrapResult {
    pub(crate) fn new(
        application_services: RuntimeApplicationServices,
        lifecycle: Arc<dyn RuntimeBootstrapLifecycle>,
    ) -> Self {
        Self {
            application_services,
            lifecycle,
        }
    }

    /// Return the application-facing Runtime capability bundle.
    pub fn application_services(&self) -> RuntimeApplicationServices {
        self.application_services.clone()
    }

    /// Consume the result when only the capability bundle is required.
    pub fn into_application_services(self) -> RuntimeApplicationServices {
        self.application_services
    }

    /// Request orderly shutdown of the composed runtime.
    pub fn shutdown(&self) {
        self.lifecycle.shutdown();
    }

    /// Request shutdown and wait for owned workers, session supervision and
    /// the session kernel to drain. All clones/callers observe the same result.
    /// Cancelling a waiter does not cancel the runtime-owned drain task.
    pub async fn shutdown_and_wait(&self) -> Result<(), crate::RuntimeControlServiceError> {
        self.lifecycle.shutdown_and_wait().await
    }
}

/// Normalize a process bootstrap request and let the concrete composition
/// adapter provide the implementation-specific runtime values. Runtime owns
/// both request conversion and the stable result envelope, so consumers never
/// need to know which crate currently builds the concrete snapshot.
pub(crate) async fn compose_runtime_bootstrap<F, Fut>(
    request: RuntimeBootstrapRequest,
    compose: F,
) -> Result<RuntimeBootstrapResult, RuntimeBootstrapError>
where
    F: FnOnce(RuntimeCompositionConfig) -> Fut,
    Fut: std::future::Future<Output = Result<RuntimeBootstrapComposition, RuntimeBootstrapError>>,
{
    let config = request.into_composition_config()?;
    let composition = compose(config).await?;
    Ok(RuntimeBootstrapResult::new(
        composition.application_services,
        composition.lifecycle,
    ))
}

type ShutdownResult = Result<(), crate::RuntimeControlServiceError>;
type ShutdownReceiver = tokio::sync::watch::Receiver<Option<ShutdownResult>>;

/// One owned drain, independent of any observer's future. The result is kept
/// for repeated/concurrent callers, including callers after a failed drain.
#[derive(Default)]
pub(crate) struct RuntimeShutdownCompletion {
    receiver: std::sync::Mutex<Option<ShutdownReceiver>>,
}

impl RuntimeShutdownCompletion {
    pub(crate) fn start<F, Fut>(&self, prepare: F) -> ShutdownResult
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ShutdownResult> + Send + 'static,
    {
        use futures_util::FutureExt;
        let mut receiver = self.receiver.lock().unwrap_or_else(|p| p.into_inner());
        if receiver.is_some() {
            return Ok(());
        }
        let executor = tokio::runtime::Handle::try_current()
            .map_err(|error| crate::RuntimeControlServiceError::from_error(&error))?;
        let (sender, observed) = tokio::sync::watch::channel(None);
        *receiver = Some(observed);
        // Publish ownership before executing user/resource Drop code. A
        // reentrant shutdown call observes the same result without deadlocking.
        drop(receiver);
        let work = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(prepare)) {
            Ok(work) => work,
            Err(_) => {
                let error =
                    crate::RuntimeControlServiceError::new("runtime shutdown preparation panicked");
                sender.send_replace(Some(Err(error.clone())));
                return Err(error);
            }
        };
        executor.spawn(async move {
            let result = std::panic::AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(crate::RuntimeControlServiceError::new(
                        "runtime shutdown task panicked",
                    ))
                });
            if let Err(error) = &result {
                tracing::error!(%error, "runtime shutdown failed");
            }
            sender.send_replace(Some(result));
        });
        Ok(())
    }

    pub(crate) async fn wait(&self) -> ShutdownResult {
        let mut receiver = self
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| {
                crate::RuntimeControlServiceError::new("runtime shutdown requires a Tokio executor")
            })?;
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return result;
            }
            receiver.changed().await.map_err(|_| {
                crate::RuntimeControlServiceError::new(
                    "runtime shutdown task ended without a drain result",
                )
            })?;
        }
    }
}

#[cfg(test)]
mod shutdown_completion_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn cancelled_waiter_does_not_cancel_shared_shutdown_or_repeat_it() {
        let completion = Arc::new(RuntimeShutdownCompletion::default());
        let starts = Arc::new(AtomicUsize::new(0));
        let (release, released) = tokio::sync::oneshot::channel();
        let counted = starts.clone();
        completion
            .start(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async move {
                    released.await.unwrap();
                    Ok(())
                }
            })
            .unwrap();
        let first = {
            let completion = completion.clone();
            tokio::spawn(async move { completion.wait().await })
        };
        first.abort();
        let _ = first.await;
        completion
            .start(|| {
                starts.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .unwrap();
        release.send(()).unwrap();
        let (second, third) = tokio::join!(completion.wait(), completion.wait());
        second.unwrap();
        third.unwrap();
        completion.wait().await.unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_shutdown_is_retained_for_every_waiter() {
        let completion = RuntimeShutdownCompletion::default();
        let expected = crate::RuntimeControlServiceError::new("kernel drain failed");
        let failure = expected.clone();
        completion.start(|| async move { Err(failure) }).unwrap();
        assert_eq!(completion.wait().await, Err(expected.clone()));
        completion
            .start(|| async { panic!("must not launch a second drain") })
            .unwrap();
        assert_eq!(completion.wait().await, Err(expected));
    }
}
