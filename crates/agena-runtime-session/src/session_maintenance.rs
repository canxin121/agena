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
