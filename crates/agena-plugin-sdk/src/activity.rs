//! Activity source adapter — first-class plugin participation in the unified
//! background-activity system.
//!
//! The host keeps one bounded registry of [`BackgroundActivity`] records and
//! lets TUI/Web list, follow, stop, and dismiss them. Plugins that own
//! long-running work (browser sessions, downloads, watchers, ...) publish
//! records with [`HostClient::publish_activity`] and register an
//! [`ActivitySourceAdapter`] so the host can route log reads and stop
//! requests back to the code that owns the work. This replaces the old
//! one-way `plugin_event` bridge with a bidirectional, per-kind contract.

use async_trait::async_trait;

use agena_domain::BackgroundActivityLogRead;

use crate::error::Result;

/// Bidirectional control surface a plugin registers for one activity kind.
///
/// Once registered (usually during `init`), the host dispatches
/// `RuntimeActivityService::activity_logs` and
/// `RuntimeActivityService::stop_activity` requests for records of that
/// kind to this adapter instead of falling back to built-in behavior.
#[async_trait]
pub trait ActivitySourceAdapter: Send + Sync + 'static {
    /// Read incremental activity output with the unified `since_seq` cursor.
    /// `wait_ms` may block for fresh output when nothing new is available yet
    /// (0 disables waiting); adapters without a live stream return what they
    /// have immediately.
    async fn read_logs(
        &self,
        activity_id: &str,
        since_seq: u64,
        limit: Option<u32>,
        wait_ms: u64,
    ) -> Result<BackgroundActivityLogRead>;

    /// Stop the running activity the id refers to. The adapter is responsible
    /// for actually terminating the underlying work and publishing the
    /// terminal record (e.g. via [`HostClient::publish_activity`]) before or
    /// after returning.
    async fn stop(&self, activity_id: &str) -> Result<()>;
}

/// Convenience for adapters that have no log stream: build an empty read.
pub fn empty_log_read(activity_id: &str) -> BackgroundActivityLogRead {
    BackgroundActivityLogRead {
        activity_id: activity_id.to_string(),
        status: agena_domain::BackgroundActivityStatus::Running,
        lines: Vec::new(),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
        completion_reason: None,
    }
}
