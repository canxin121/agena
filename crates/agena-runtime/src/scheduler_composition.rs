//! Runtime-owned scheduler composition.
//!
//! A concrete session adapter supplies the delivery sink; Runtime owns the
//! in-process store, polling policy, and scheduler task startup.

use std::sync::Arc;

use agena_scheduler::{JobSink, Scheduler};

/// Compose and start the process scheduler with the Runtime polling policy.
/// The sink remains generic so this module does not depend on a concrete
/// session manager or tool executor.
pub fn compose_scheduler<S>(sink: Arc<S>) -> Arc<Scheduler>
where
    S: JobSink + 'static,
{
    let scheduler = agena_scheduler::scheduler::build_in_memory(
        sink,
        crate::RuntimeSchedulingPolicy::default().scheduler_poll_interval,
    );
    scheduler.start();
    scheduler
}
