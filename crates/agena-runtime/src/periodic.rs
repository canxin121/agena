use std::time::Duration;
use std::{future::Future, sync::Arc};

use crate::TaskControl;

/// Wait for one scheduler tick or a runtime shutdown notification.
pub async fn wait_for_tick_or_shutdown(control: &TaskControl, interval: Duration) -> bool {
    if control.is_shutdown() {
        return true;
    }

    tokio::select! {
        _ = tokio::time::sleep(interval) => {}
        _ = control.notify().notified() => {}
    }

    control.is_shutdown()
}

/// Run a runtime-owned periodic loop while invoking a caller-provided tick.
#[allow(dead_code)]
pub async fn run_periodic<I, F, Fut>(control: Arc<TaskControl>, mut interval: I, mut tick: F)
where
    I: FnMut() -> Duration + Send,
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    loop {
        if wait_for_tick_or_shutdown(&control, interval()).await {
            break;
        }
        tick().await;
    }
}
