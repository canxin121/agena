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
