use super::{builder::AgenaRuntime, wait_for_tick_or_shutdown};

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
