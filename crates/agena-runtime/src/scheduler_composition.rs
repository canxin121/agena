//! Runtime-owned scheduler composition.
//!
//! A concrete session adapter supplies the delivery sink; Runtime owns the
//! in-process store and reconciliation policy. Publication installs the
//! change observer before starting delivery admission.

use std::sync::Arc;

use agena_scheduler::{JobSink, Scheduler};
use sea_orm::DatabaseConnection;

/// Prepare the process scheduler with the Runtime reconciliation policy.
/// The sink remains generic so this module does not depend on a concrete
/// session manager or tool executor.
pub fn compose_scheduler<S>(
    scheduler_database: Option<Arc<DatabaseConnection>>,
    sink: Arc<S>,
) -> Arc<Scheduler>
where
    S: JobSink + 'static,
{
    agena_scheduler::scheduler::build_persistent(
        scheduler_database,
        sink,
        crate::RuntimeSchedulingPolicy::default().scheduler_reconciliation_interval,
    )
}
