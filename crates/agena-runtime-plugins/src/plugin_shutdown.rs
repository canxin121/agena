//! Snapshot-scoped plugin-host shutdown orchestration.

use std::{sync::Arc, time::Duration};

use agena_plugin_host::PluginHost;

const GENERATION_RETIRE_POLL_INTERVAL: Duration = Duration::from_millis(10);

async fn wait_until_generation_released<T>(value: &Arc<T>) {
    // A turn snapshots its ToolExecutor, which owns an Arc<PluginHost>. Runtime
    // reload may retire the snapshot while such a turn is still in flight. The
    // snapshot guard itself is the final owner once every turn-level generation
    // lease has gone away, at which point shutdown can safely close admission.
    while Arc::strong_count(value) > 1 {
        tokio::time::sleep(GENERATION_RETIRE_POLL_INTERVAL).await;
    }
}

/// Retain a callback guard that asynchronously shuts down a non-empty plugin
/// host after the retiring snapshot's last external generation lease is gone.
///
/// In-flight turns retain the exact ToolExecutor/PluginHost generation they
/// started with. Reload must not disconnect that host merely because the
/// runtime installed a newer snapshot; the host is retired only after those
/// Arc leases drain naturally.
pub fn plugin_shutdown_guard(plugins: Arc<PluginHost>) -> Option<Arc<crate::CallbackOnDrop>> {
    if plugins.is_empty() {
        return None;
    }

    let handle = tokio::runtime::Handle::try_current().ok();
    Some(Arc::new(crate::CallbackOnDrop::new(move || match handle {
        Some(handle) => {
            handle.spawn(async move {
                wait_until_generation_released(&plugins).await;
                plugins.shutdown().await;
            });
        }
        None => {
            tracing::debug!(
                target: "agena_plugin_host",
                "no tokio runtime available at snapshot drop; plugins will be cleaned up by their own transports"
            );
        }
    })))
}
