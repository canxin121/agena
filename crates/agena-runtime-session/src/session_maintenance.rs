//! Runtime-owned maintenance loops for session-backed processes.
//!
//! The runtime owns the periodic scheduling and shutdown choreography. A
//! concrete composition crate supplies only the interval and the operation
//! that performs its session/cache-specific work.

use std::{future::Future, sync::Arc, time::Duration};

use crate::{PeriodicControl, periodic::run_periodic};

/// Run a session-cache maintenance loop until the composed runtime shuts
/// down. The callbacks deliberately remain generic so the runtime does not
/// depend on a concrete session manager or session aggregate.
pub async fn run_session_maintenance<C, I, F, Fut>(
    task_control: Arc<C>,
    interval: I,
    maintenance: F,
) where
    C: PeriodicControl + ?Sized,
    I: FnMut() -> Duration + Send,
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    run_periodic(task_control, interval, maintenance).await;
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::run_session_maintenance;
    use crate::TaskControl;

    #[tokio::test]
    async fn exits_before_the_first_tick_after_shutdown() {
        let control = Arc::new(TaskControl::default());
        control.shutdown();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_maintenance = Arc::clone(&called);

        run_session_maintenance(
            control,
            || Duration::from_millis(1),
            move || {
                called_maintenance.store(true, std::sync::atomic::Ordering::SeqCst);
                async {}
            },
        )
        .await;

        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
