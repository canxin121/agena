use std::time::Duration;

use super::builder::AgenaRuntime;

pub(crate) async fn run(runtime: AgenaRuntime) {
    loop {
        let snapshot = runtime.current_snapshot();
        let interval = snapshot.session_gc_interval();
        if wait_for_tick_or_shutdown(&runtime, interval).await {
            break;
        }

        if let Some(manager) = runtime.current_snapshot().session_manager() {
            manager.prune_cache();
        }
    }
}

async fn wait_for_tick_or_shutdown(runtime: &AgenaRuntime, interval: Duration) -> bool {
    if runtime.is_shutdown() {
        return true;
    }

    tokio::select! {
        _ = tokio::time::sleep(interval) => {}
        _ = runtime.task_control().notify().notified() => {}
    }

    runtime.is_shutdown()
}
