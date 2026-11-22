//! Runtime-owned scheduler composition.
//!
//! A concrete session adapter supplies the delivery sink; Runtime owns the
//! in-process store and reconciliation policy. Publication installs the
//! change observer before starting delivery admission.

use std::sync::Arc;

use agena_scheduler::{JobSink, Scheduler};
use sea_orm::DatabaseConnection;

pub(crate) type SchedulerDatabase = (Arc<DatabaseConnection>, Arc<DatabaseConnection>);

/// Prepare the process scheduler with the Runtime reconciliation policy.
/// The sink remains generic so this module does not depend on a concrete
/// session manager or tool executor.
pub fn compose_scheduler<S>(
    scheduler_database: Option<SchedulerDatabase>,
    sink: Arc<S>,
) -> Arc<Scheduler>
where
    S: JobSink + 'static,
{
    let tick = crate::RuntimeSchedulingPolicy::default().scheduler_reconciliation_interval;
    match scheduler_database {
        Some((writer, reader)) => Scheduler::new(
            Arc::new(agena_scheduler::SqliteJobStore::with_reader(
                writer.as_ref().clone(),
                reader.as_ref().clone(),
            )),
            sink,
            tick,
        ),
        None => agena_scheduler::scheduler::build_in_memory(sink, tick),
    }
}
