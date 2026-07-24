//! Snapshot-scoped plugin-host shutdown orchestration.

use std::sync::Arc;

use agena_plugin_host::PluginHost;

/// Retain a callback guard that asynchronously shuts down a non-empty plugin
/// host when its snapshot is dropped.
pub fn plugin_shutdown_guard(plugins: Arc<PluginHost>) -> Option<Arc<crate::CallbackOnDrop>> {
    if plugins.is_empty() {
        return None;
    }

    let handle = tokio::runtime::Handle::try_current().ok();
    Some(Arc::new(crate::CallbackOnDrop::new(move || match handle {
        Some(handle) => {
            handle.spawn(async move { plugins.shutdown().await });
        }
        None => {
            tracing::debug!(
                target: "agena_plugin_host",
                "no tokio runtime available at snapshot drop; plugins will be cleaned up by their own transports"
            );
        }
    })))
}
