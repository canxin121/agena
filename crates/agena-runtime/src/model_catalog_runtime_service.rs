//! Runtime-facing catalog snapshot and refresh operations.
//!
//! The concrete runtime may compose provider registries, persistence, and
//! refresh tasks internally. Upper layers only need the stable catalog
//! response plus the runtime-owned background-task outcome.

use async_trait::async_trait;

use agena_provider::ModelCatalogResponse;

use crate::{
    RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskStart,
};

/// Stable catalog operations exposed by an already-composed runtime.
#[async_trait]
pub trait ModelCatalogRuntimeService: Send + Sync {
    /// Returns the catalog projection observed by upper-layer presentation.
    fn model_catalog_response(&self) -> ModelCatalogResponse;

    /// Whether a model-catalog refresh task is currently running.
    fn model_catalog_refresh_active(&self) -> bool;

    /// Starts a deduplicated catalog refresh task. Admission failures retain
    /// their kind; failures after acceptance belong to the task's status.
    fn start_model_catalog_refresh(
        &self,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>;
}

#[cfg(test)]
mod tests {
    use super::{ModelCatalogRuntimeService, RuntimeBackgroundTaskControlError};

    struct FakeCatalogRuntime;

    #[async_trait::async_trait]
    impl ModelCatalogRuntimeService for FakeCatalogRuntime {
        fn model_catalog_response(&self) -> agena_provider::ModelCatalogResponse {
            agena_provider::ModelCatalogResponse {
                last_refresh_at: None,
                last_successful_source: None,
                last_failure: None,
                models: Vec::new(),
            }
        }

        fn model_catalog_refresh_active(&self) -> bool {
            false
        }

        fn start_model_catalog_refresh(
            &self,
            _origin: crate::RuntimeBackgroundTaskOrigin,
        ) -> Result<crate::RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError> {
            Err(RuntimeBackgroundTaskControlError::ExecutorUnavailable)
        }
    }

    #[test]
    fn trait_object_only_exposes_stable_catalog_and_task_values() {
        let service: &dyn ModelCatalogRuntimeService = &FakeCatalogRuntime;
        assert!(service.model_catalog_response().models.is_empty());
        assert!(!service.model_catalog_refresh_active());
    }
}
