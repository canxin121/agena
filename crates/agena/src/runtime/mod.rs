mod background_tasks;
mod builder;
mod event_bridge;
pub mod host_client;
mod janitor;
pub mod plugin_slot;
mod reload;
mod snapshot;
mod store;

pub use background_tasks::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskStart,
    RuntimeBackgroundTaskStatus,
};
pub use builder::{AgenaRuntime, AgenaRuntimeConfig, TracingFilterReloadHandle};
pub use event_bridge::spawn_event_bridge;
pub use host_client::{host_client_for, noop_host_client};
pub use reload::{RuntimeReloadCause, RuntimeReloadReport};
pub use snapshot::RuntimeSnapshot;

/// Tool failure recovery and streaming session flows build deep async stacks in
/// the session manager. Production binaries need a larger worker-thread stack
/// than Tokio's default to avoid overflowing on those paths.
pub const APP_RUNTIME_THREAD_STACK_SIZE: usize = 16 * 1024 * 1024;

pub fn build_app_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(APP_RUNTIME_THREAD_STACK_SIZE)
        .build()
}
