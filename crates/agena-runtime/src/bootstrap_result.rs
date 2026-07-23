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
pub(crate) trait RuntimeBootstrapLifecycle: Send + Sync {
    fn shutdown(&self);
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
